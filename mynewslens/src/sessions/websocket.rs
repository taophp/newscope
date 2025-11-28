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

                            match crate::press_review::generate_press_review(&pool, user_id, llm_provider, &model, duration_seconds).await {
                                Ok(review) => {
                                    // Store message
                                    let _ = crate::sessions::store_message(&pool, session_id, "assistant", &review).await;
                                    // Send to client
                                    let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                        "type": "message",
                                        "content": review
                                    })).unwrap())).await;
                                }
                                Err(e) => {
                                    error!("Newscope: Failed to generate press review: {}", e);
                                    let msg = "I couldn't generate the press review at this time. How can I help you?";
                                    let _ = crate::sessions::store_message(&pool, session_id, "assistant", msg).await;
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
        "You are a helpful news assistant for MyNewsLens. \
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
