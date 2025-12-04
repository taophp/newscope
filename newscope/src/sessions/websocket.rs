use anyhow::Result;
use rocket::{State, get};
use rocket::futures::{SinkExt, StreamExt};
use rocket_ws::{Channel, Message, WebSocket};
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing::{error, info};

use crate::llm::{LlmProvider, LlmRequest};
use super::{get_messages, store_message};

use serde_json::json;

/// WebSocket chat endpoint
#[get("/chat?<session_id>")]
pub fn chat_websocket(
    ws: WebSocket,
    session_id: i64,
    state: &State<crate::server::AppState>,
) -> Channel<'static> {
    let pool = state.db.clone();
    let llm = state.llm_provider.clone();
    let config = state.config.clone();

    ws.channel(move |mut stream| {
        Box::pin(async move {
            info!("WebSocket connected for session {}", session_id);

            // Send chat history on connection
            // Send initial greeting or history
            match crate::sessions::get_session_with_messages(&pool, session_id).await {
                Ok((session, messages)) => {
                    let user_id = session.user_id;
                    let duration_seconds = session.duration_requested_seconds.unwrap_or(1200) as i64;
                    
                    if messages.is_empty() {
                        // New session: generate press review
                        if let Some(llm_provider) = llm.clone() {
                            let pool = pool.clone();
                            let model = config.as_ref()
                            .and_then(|c| c.llm.as_ref())
                            .and_then(|l| l.remote.as_ref())
                            .and_then(|r| r.model.as_deref())
                            .unwrap_or("unknown")
                            .to_string();
                            
                            let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                "type": "message",
                                "content": "👋 Hello! I'm preparing your personalized press review based on new articles..."
                            })).unwrap())).await;

                            // Send initial progress
                            let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                "type": "progress",
                                "message": "Fetching your articles..."
                            })).unwrap())).await;

                            // Progressive generation - fetch ALL articles with summaries
                            match crate::press_review::fetch_and_score_articles(&pool, user_id).await {
                                Ok(articles) => {
                                    if articles.is_empty() {
                                        let msg = "Welcome back! I haven't found any new articles since your last visit.";
                                        let _ = crate::sessions::store_message(&pool, session_id, "assistant", msg).await;
                                        let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                            "type": "message",
                                            "content": msg
                                        })).unwrap())).await;
                                    } else {
                                        let intro = format!("👋 Hello! I found {} relevant articles for you. Let's go through them.", articles.len());
                                        let _ = crate::sessions::store_message(&pool, session_id, "assistant", &intro).await;
                                        let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                            "type": "message",
                                            "content": intro
                                        })).unwrap())).await;

                                        let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                            "type": "progress",
                                            "message": format!("Analyzing {} articles...", articles.len())
                                        })).unwrap())).await;

                                        // Process in chunks of 5
                                        let total_chunks = (articles.len() + 4) / 5;
                                        for (chunk_idx, chunk) in articles.chunks(5).enumerate() {
                                            // Send progress for this chunk
                                            let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                                "type": "progress",
                                                "message": format!("Generating review ({}/{})", chunk_idx + 1, total_chunks)
                                            })).unwrap())).await;
                                            let mut prompt = String::new();
                                            prompt.push_str("You are a personal news editor. Summarize these articles for a quick press review.\n");
                                            prompt.push_str("Group by topic if possible. Be concise and engaging.\n\n");
                                            
                                            for article in chunk {
                                                prompt.push_str(&format!("- **{}** (Source: {})\n", article.headline, article.feed_title));
                                                for bullet in article.bullets.iter().take(2) {
                                                    prompt.push_str(&format!("  * {}\n", bullet));
                                                }
                                                prompt.push_str(&format!("  [Read more]({})\n\n", article.url));
                                            }
                                            
                                            prompt.push_str("\nGenerate a short review section for these:");

                                            match llm_provider.generate(LlmRequest {
                                                prompt,
                                                max_tokens: Some(300),
                                                temperature: Some(0.7),
                                                timeout_seconds: Some(30),
                                            }).await {
                                                Ok(response) => {
                                                    let content = response.content;
                                                    
                                                    // Prepare sources for this chunk
                                                    let sources: Vec<serde_json::Value> = chunk.iter().map(|article| {
                                                        json!({
                                                            "url": article.url,
                                                            "title": article.article_title,
                                                            "feed_title": article.feed_title,
                                                            "score": article.score
                                                        })
                                                    }).collect();
                                                    
                                                    let _ = crate::sessions::store_message(&pool, session_id, "assistant", &content).await;
                                                    let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                                        "type": "message",
                                                        "content": content,
                                                        "sources": sources
                                                    })).unwrap())).await;

                                                    // Mark articles in this chunk as viewed ONLY if generation succeeded
                                                    for article in chunk {
                                                        let _ = sqlx::query(
                                                            "INSERT OR IGNORE INTO user_article_views (user_id, article_id, session_id) VALUES (?, ?, ?)"
                                                        )
                                                        .bind(user_id)
                                                        .bind(article.id)
                                                        .bind(session_id)
                                                        .execute(&pool)
                                                        .await;
                                                    }
                                                }
                                                Err(e) => {
                                                    error!("Failed to generate chunk review: {}", e);
                                                    // Do NOT mark as viewed so they can be retried later
                                                }
                                            }
                                        }
                                        

                                        
                                        let outro = "That's all for now! Let me know if you want to explore any topic in depth.";
                                        let _ = crate::sessions::store_message(&pool, session_id, "assistant", outro).await;
                                        let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                            "type": "message",
                                            "content": outro
                                        })).unwrap())).await;
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to fetch articles: {}", e);
                                    let msg = "I'm having trouble accessing the latest news. Please try again later.";
                                    let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                        "type": "message",
                                        "content": msg
                                    })).unwrap())).await;
                                }
                            }
                        } else {
                             let msg = "Hello! I'm ready to discuss the news with you.";
                             let _ = crate::sessions::store_message(&pool, session_id, "assistant", msg).await;
                             let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                 "type": "message",
                                 "content": msg
                             })).unwrap())).await;
                        }
                    } else {
                        // Existing session: replay history
                        for msg in messages {
                            let role = if msg.author == "user" { "user" } else { "assistant" };
                            let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                "type": "history",
                                "role": role,
                                "content": msg.message
                            })).unwrap())).await;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to load chat history: {}", e);
                }
            }

            // Handle incoming messages
            while let Some(message) = stream.next().await {
                match message {
                    Ok(Message::Text(text)) => {
                        info!("Received message for session {}: {}", session_id, text);

                        // Parse user message
                        let user_message = match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(json) => json["message"].as_str().unwrap_or(&text).to_string(),
                            Err(_) => text,
                        };

                        // Store user message
                        if let Err(e) = store_message(&pool, session_id, "user", &user_message).await {
                            error!("Failed to store user message: {}", e);
                            continue;
                        }

                        // Generate LLM response
                        let response = if let Some(ref provider) = llm {
                            match handle_chat_message(&pool, provider, session_id, &user_message).await {
                                Ok(resp) => resp,
                                Err(e) => {
                                    error!("LLM error: {}", e);
                                    "Sorry, I encountered an error processing your message.".to_string()
                                }
                            }
                        } else {
                            "LLM provider not configured.".to_string()
                        };

                        // Store assistant response
                        if let Err(e) = store_message(&pool, session_id, "assistant", &response).await {
                            error!("Failed to store assistant message: {}", e);
                        }

                        // Send response to client
                        let json = serde_json::json!({
                            "type": "message",
                            "author": "assistant",
                            "message": response,
                        });
                        if let Err(e) = stream.send(Message::Text(json.to_string())).await {
                            error!("Failed to send response: {}", e);
                            break;
                        }
                    }
                    Ok(Message::Close(_)) => {
                        info!("WebSocket closed for session {}", session_id);
                        break;
                    }
                    Err(e) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }

            Ok(())
        })
    })
}

/// Handle chat message with LLM
async fn handle_chat_message(
    pool: &SqlitePool,
    llm_provider: &Arc<dyn LlmProvider>,
    session_id: i64,
    user_message: &str,
) -> Result<String> {
    // Get conversation history
    let messages = get_messages(pool, session_id).await?;

    // Build conversation context
    let mut context = String::from(
        "You are a helpful news assistant for Newscope. \\
         The user is exploring their personalized news feed. \
         Answer questions concisely and help them understand the news.\n\n"
    );

    for msg in messages.iter().rev().take(10).rev() {
        context.push_str(&format!("{}: {}\n", msg.author, msg.message));
    }
    context.push_str(&format!("user: {}\nassistant:", user_message));

    // Generate LLM response
    let request = LlmRequest {
        prompt: context,
        max_tokens: Some(300),
        temperature: Some(0.7),
        timeout_seconds: Some(30),
    };

    let response = llm_provider.generate(request).await?;

    Ok(response.content)
}
