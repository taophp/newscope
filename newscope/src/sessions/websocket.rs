use anyhow::Result;
use rocket::request::{FromRequest, Outcome, Request};
use rocket::{State, get};
use rocket::futures::{SinkExt, StreamExt};
use rocket_ws::{Channel, Message, WebSocket};
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing::{error, info};

use crate::llm::{LlmProvider, LlmRequest};
use super::{get_messages, store_message};

use serde_json::json;

/// Request guard for Accept-Language header
pub struct AcceptLanguage(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AcceptLanguage {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let lang = req.headers()
            .get_one("Accept-Language")
            .and_then(|s| s.split(',').next()) // Get first language preference
            .and_then(|s| s.split('-').next()) // Get primary tag (e.g. "fr" from "fr-FR")
            .unwrap_or("en")
            .to_string();
        Outcome::Success(AcceptLanguage(lang))
    }
}

/// WebSocket chat endpoint
#[get("/chat?<session_id>")]
pub fn chat_websocket(
    ws: WebSocket,
    session_id: i64,
    accept_lang: AcceptLanguage,
    state: &State<crate::server::AppState>,
) -> Channel<'static> {
    let pool = state.db.clone();
    let llm = state.llm_provider.clone();
    let config = state.config.clone();
    let language = accept_lang.0;

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
                                "message": "Fetching your personalized articles..."
                            })).unwrap())).await;

                            // PHASE 3: Fetch PRE-COMPUTED personalized summaries
                            let reading_minutes = duration_seconds / 120; // 50% for reading (divide by 2, then convert to minutes)
                            let estimated_articles = (reading_minutes * 3).min(50); // ~3 articles/min reading, max 50

                            match sqlx::query(
                                "SELECT 
                                    uas.article_id,
                                    uas.personalized_headline,
                                    uas.personalized_bullets,
                                    uas.relevance_score,
                                    a.canonical_url,
                                    f.title as feed_title
                                 FROM user_article_summaries uas
                                 JOIN articles a ON uas.article_id = a.id
                                 LEFT JOIN article_occurrences ao ON a.id = ao.article_id
                                 LEFT JOIN feeds f ON ao.feed_id = f.id
                                 LEFT JOIN user_article_views uav ON uas.user_id = uav.user_id AND uas.article_id = uav.article_id
                                 WHERE uas.user_id = ?
                                   AND uas.is_relevant = 1
                                   AND uav.id IS NULL
                                 GROUP BY uas.article_id
                                 ORDER BY uas.relevance_score DESC, a.first_seen_at DESC
                                 LIMIT ?"
                            )
                            .bind(user_id)
                            .bind(estimated_articles)
                            .fetch_all(&pool)
                            .await
                            {
                                Ok(articles) => {
                                    if articles.is_empty() {
                                        let msg = "Welcome back! Your personalized articles are still being processed. Please check back in a few minutes.";
                                        let _ = crate::sessions::store_message(&pool, session_id, "assistant", msg).await;
                                        let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                            "type": "message",
                                            "content": msg
                                        })).unwrap())).await;
                                    } else {
                                        // Hide progress indicator
                                        let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                            "type": "progress_hide"
                                        })).unwrap())).await;

                                        // Extract article data from rows
                                        use sqlx::Row;
                                        let article_data: Vec<(i64, String, String, f64, String, Option<String>)> = articles.iter()
                                            .map(|row| {
                                                let article_id: i64 = row.get("article_id");
                                                let headline: String = row.get("personalized_headline");
                                                let bullets: String = row.get("personalized_bullets");
                                                let relevance: f64 = row.get("relevance_score");
                                                let url: String = row.get("canonical_url");
                                                let feed_title: Option<String> = row.try_get("feed_title").ok();
                                                (article_id, headline, bullets, relevance, url, feed_title)
                                            })
                                            .collect();

                                        // Build context from pre-computed summaries
                                        let articles_context: Vec<String> = article_data.iter()
                                            .map(|(_, headline, bullets_json, relevance, _, feed_title)| {
                                                let bullets: Vec<String> = serde_json::from_str(bullets_json).unwrap_or_default();
                                                format!(
                                                    "**{}**\nSource: {}\nRelevance: {:.2}\nPoints:\n- {}",
                                                    headline,
                                                    feed_title.as_deref().unwrap_or("Unknown"),
                                                    relevance,
                                                    bullets.join("\n- ")
                                                )
                                            })
                                            .collect();

                                        let context_text = articles_context.join("\n\n---\n\n");

                                        // LIGHTWEIGHT LLM TASK: Create narrative synthesis
                                        let synthesis_prompt = format!(
                                            "You are creating a personalized news briefing for a {} minute session.

The user has {} minutes for reading. Create a cohesive narrative synthesis highlighting the most important themes and stories from these {} pre-selected articles:

{}

Instructions:
1. Respond in {} (important!)
2. Identify 2-3 major themes connecting these stories
3. Create a compelling introduction highlighting what's most important
4. Group related stories together with smooth transitions
5. Keep the synthesis engaging and conversational
6. Total length should fit ~{} minutes of reading

Create a well-structured, engaging briefing.",
                                            duration_seconds / 60,
                                            reading_minutes,
                                            article_data.len(),
                                            context_text,
                                            match language.as_str() {
                                                "fr" => "French",
                                                "es" => "Spanish",
                                                "de" => "German",
                                                "it" => "Italian",
                                                _ => "English"
                                            },
                                            reading_minutes
                                        );

                                        // Single focused LLM call for synthesis (much lighter!)
                                        match llm_provider.generate(crate::llm::LlmRequest {
                                            prompt: synthesis_prompt,
                                            max_tokens: Some((reading_minutes * 150) as usize),
                                            temperature: Some(0.7),
                                            timeout_seconds: Some(120),
                                        }).await {
                                            Ok(response) => {
                                                let _ = crate::sessions::store_message(&pool, session_id, "assistant", &response.content).await;
                                                let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                                    "type": "message",
                                                    "content": response.content
                                                })).unwrap())).await;

                                                // Include source links
                                                let sources: Vec<serde_json::Value> = article_data.iter()
                                                    .map(|(_, headline, _, relevance, url, _)| json!({
                                                        "title": headline,
                                                        "url": url,
                                                        "score": relevance
                                                    }))
                                                    .collect();

                                                let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                                    "type": "sources",
                                                    "sources": sources
                                                })).unwrap())).await;

                                                // Mark articles as viewed ONLY if synthesis succeeded
                                                for (article_id, _, _, _, _, _) in &article_data {
                                                    let _ = sqlx::query(
                                                        "INSERT OR IGNORE INTO user_article_views (user_id, article_id) VALUES (?, ?)"
                                                    )
                                                    .bind(user_id)
                                                    .bind(article_id)
                                                    .execute(&pool)
                                                    .await;
                                                }
                                            }
                                            Err(e) => {
                                                error!("Failed to generate synthesis: {}", e);
                                                let error_msg = "I apologize, but I encountered an error while generating the review. Please check the server logs or try again later.";
                                                let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                                    "type": "error",
                                                    "content": error_msg
                                                })).unwrap())).await;
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
                                    error!("Failed to fetch personalized articles for user {}: {:?}", user_id, e);
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
    
    // Note: We don't easily have language here without passing it through store_message or similar,
    // but the system prompt could be updated if we stored language in session.
    // For now, we rely on the LLM adapting to the user's language in the conversation history.

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
