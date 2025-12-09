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
                            
                            let greeting = match language.as_str() {
                                "fr" => "👋 Bonjour ! Je prépare votre revue de presse personnalisée. Je vous enverrai une notification quand elle sera prête...",
                                "es" => "👋 ¡Hola! Estoy preparando su resumen de prensa personalizado. Le enviaré una notificación cuando esté listo...",
                                "de" => "👋 Hallo! Ich bereite Ihren persönlichen Pressespiegel vor. Ich sende Ihnen eine Benachrichtigung, wenn er fertig ist...",
                                "it" => "👋 Ciao! Sto preparando la tua rassegna stampa personalizzata. Ti invierò una notifica quando sarà pronta...",
                                _ => "👋 Hello! I'm preparing your personalized press review. I'll send you a notification when it's ready..."
                            };

                            let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                "type": "message",
                                "content": greeting
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
                                    // Fetch user profile to get preferred language
                                    let mut language = language.clone();
                                    if let Ok(profile) = crate::personalization::get_user_profile(&pool, user_id).await {
                                        language = profile.language;
                                    }

                                    // Hide progress indicator
                                    let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                        "type": "progress_hide"
                                    })).unwrap())).await;

                                    // Extract article data from rows
                                    use sqlx::Row;
                                    let article_data: Vec<(i64, String, String, Option<String>, f64, String, Option<String>)> = articles.iter()
                                        .map(|row| {
                                            let article_id: i64 = row.get("article_id");
                                            let headline: String = row.get("personalized_headline");
                                            let bullets: String = row.get("personalized_bullets");
                                            let details: Option<String> = row.try_get("personalized_details").ok();
                                            let relevance: f64 = row.get("relevance_score");
                                            let url: String = row.get("canonical_url");
                                            let feed_title: Option<String> = row.try_get("feed_title").ok();
                                            (article_id, headline, bullets, details, relevance, url, feed_title)
                                        })
                                        .collect();

                                    // STREAMING MODE: Send articles as individual cards
                                    for (article_id, headline, bullets_json, details, relevance, url, feed_title) in article_data {
                                        // Construct summary: prefer details (paragraph), fallback to bullets
                                        let summary = if let Some(d) = details {
                                            d
                                        } else {
                                            let bullets: Vec<String> = serde_json::from_str(&bullets_json).unwrap_or_default();
                                            bullets.join(" ")
                                        };

                                        let theme = feed_title.clone().unwrap_or_else(|| "Actualité".to_string());
                                        let source_name = feed_title.unwrap_or_else(|| "Unknown".to_string());

                                        // Send News Card
                                        let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                            "type": "news_item",
                                            "article": {
                                                "id": article_id,
                                                "title": headline,
                                                "theme": theme,
                                                "summary": summary,
                                                "sources": [{
                                                    "name": source_name,
                                                    "url": url
                                                }]
                                            }
                                        })).unwrap())).await;

                                        // Mark as viewed immediately
                                        let _ = sqlx::query(
                                            "INSERT OR IGNORE INTO user_article_views (user_id, article_id) VALUES (?, ?)"
                                        )
                                        .bind(user_id)
                                        .bind(article_id)
                                        .execute(&pool)
                                        .await;
                                        
                                        // Small delay for progressive effect (optional, but nice)
                                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                    }

                                    // Final message
                                    let completion_msg = match language.as_str() {
                                        "fr" => "Voilà pour l'essentiel de l'actualité. Souhaitez-vous approfondir un sujet ?",
                                        "es" => "Eso es todo por ahora. ¿Desea profundizar en algún tema?",
                                        "de" => "Das war das Wichtigste. Möchten Sie ein Thema vertiefen?",
                                        "it" => "Questo è tutto per ora. Vuoi approfondire un argomento?",
                                        _ => "That's the main news. Would you like to explore any topic further?"
                                    };

                                    let _ = crate::sessions::store_message(&pool, session_id, "assistant", completion_msg).await;
                                    let _ = stream.send(Message::Text(serde_json::to_string(&json!({
                                        "type": "message",
                                        "content": completion_msg
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

    // Get session to find user_id
    let session = crate::sessions::get_session(pool, session_id).await?;
    
    // Get user profile for language
    let mut language = "English".to_string();
    if let Ok(profile) = crate::personalization::get_user_profile(pool, session.user_id).await {
        language = match profile.language.as_str() {
            "fr" => "French",
            "es" => "Spanish",
            "de" => "German",
            "it" => "Italian",
            _ => "English"
        }.to_string();
    }

    // Build conversation context
    let mut context = format!(
        "You are a helpful news assistant for Newscope. \
         The user is exploring their personalized news feed. \
         Answer questions concisely and help them understand the news. \
         IMPORTANT: You MUST answer in {}.\n\n",
         language
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
