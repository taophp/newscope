use anyhow::Result;
use rocket::futures::{SinkExt, StreamExt};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::{get, State};
use rocket_ws::{Channel, Message, WebSocket};
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing::{error, info};

use super::{get_messages, store_message};
use crate::llm::{LlmProvider, LlmRequest};

use serde_json::json;

/// Request guard for Accept-Language header
pub struct AcceptLanguage(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AcceptLanguage {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let lang = req
            .headers()
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

    ws.channel(move |stream| {
        Box::pin(async move {
            info!("WebSocket connected for session {}", session_id);

            // Split stream into sink and stream
            let (mut ws_sink, mut ws_stream) = stream.split();

            // Create MPSC channel for sending messages to the websocket
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

            // Spawn task to forward messages from channel to websocket
            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    if let Err(e) = ws_sink.send(msg).await {
                        error!("Failed to send message to websocket: {}", e);
                        break;
                    }
                }
            });

            // Helper to send JSON message
            let send_json = |tx: &tokio::sync::mpsc::UnboundedSender<Message>, json: serde_json::Value| {
                let _ = tx.send(Message::Text(json.to_string()));
            };

            // Fetch session info first
            let (user_id, messages, duration_seconds) = match crate::sessions::get_session_with_messages(&pool, session_id).await {
                Ok((session, msgs)) => (
                    session.user_id,
                    msgs,
                    session.duration_requested_seconds.unwrap_or(1200) as i64
                ),
                Err(e) => {
                    error!("Failed to fetch session {}: {}", session_id, e);
                    return Ok(());
                }
            };

            // Shared state for article context (empty for now, populated if new session)
            let article_context = Arc::new(std::sync::Mutex::new(Vec::<ArticleContext>::new()));
            let article_context_bg = article_context.clone();
            let article_context_chat = article_context.clone();

            if messages.is_empty() {
                // New session: generate press review
                if let Some(llm_provider) = llm.clone() {
                    let pool = pool.clone();
                    let _model = config.as_ref()
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

                    send_json(&tx, json!({
                        "type": "message",
                        "content": greeting
                    }));

                    // Spawn background task for heavy lifting
                    let tx_clone = tx.clone(); // Clone sender for background task
                    let language_clone = language.clone();
                    // Initialize user_profile_lang from Accept-Language; it may be updated after fetching profile

                    tokio::spawn(async move {
                        // Notify when ready
                        let _ = tx_clone.send(Message::Text(serde_json::to_string(&json!({
                            "type": "notification",
                            "title": "Newscope",
                            "body": "Votre revue de presse est prête !"
                        })).unwrap()));

                        // PHASE 3: Fetch PRE-COMPUTED personalized summaries
                        let duration = duration_seconds as u64;
                        let reading_minutes = (duration as f64 / 60.0).ceil();

                        // Fetch user profile for reading speed and preferred language
                        let mut reading_speed = 250;
                        // Initialize from Accept-Language header (language_clone is moved into the spawn)
                        let mut user_profile_lang = language_clone.clone(); // default to Accept-Language header

                        let user_profile_opt = match crate::personalization::get_user_profile(&pool, user_id).await {
                            Ok(profile) => {
                                reading_speed = profile.reading_speed;
                                user_profile_lang = profile.language.clone();
                                Some(profile)
                            }
                            Err(_) => None,
                        };

                        // Calculate number of articles
                        let total_words_budget = (reading_minutes / 2.0) * reading_speed as f64;
                        let estimated_articles = (total_words_budget / 150.0).ceil() as i64;
                        // Ensure at least 3 articles, max 15
                        let estimated_articles = estimated_articles.max(3).min(15);

                        info!("Session {}: duration {}s ({}m), speed {}wpm -> budget {} words -> {} articles",
                            session_id, duration, reading_minutes, reading_speed, total_words_budget, estimated_articles);

                        match sqlx::query(
                            "SELECT
                                uas.article_id,
                                uas.personalized_headline,
                                uas.personalized_bullets,
                                uas.personalized_details,
                                uas.language,
                                uas.relevance_score,
                                a.canonical_url,
                                f.title as feed_title
                             FROM user_article_summaries uas
                             JOIN articles a ON uas.article_id = a.id
                             -- Require that the article appears in at least one feed the user is subscribed to.
                             JOIN article_occurrences ao ON a.id = ao.article_id
                             JOIN subscriptions s ON s.feed_id = ao.feed_id AND s.user_id = ?
                             LEFT JOIN feeds f ON ao.feed_id = f.id
                             -- Exclude articles already viewed by the user in ANY session
                             LEFT JOIN user_article_views uav ON uas.user_id = uav.user_id AND uas.article_id = uav.article_id
                             WHERE uas.user_id = ?
                               AND uas.is_relevant = 1
                               AND uav.id IS NULL
                             GROUP BY uas.article_id
                             ORDER BY uas.relevance_score DESC, a.first_seen_at DESC
                             LIMIT ?"
                        )
                        // Bind order corresponds to the ? placeholders above:
                        // 1: s.user_id, 2: uas.user_id, 3: LIMIT
                        .bind(user_id)
                        .bind(user_id)
                        .bind(estimated_articles)
                        .fetch_all(&pool)
                        .await
                        {
                            Ok(articles) => {
                                if articles.is_empty() {
                                    let msg = "I couldn't find any new relevant articles for you right now. Please check back later!";
                                    let _ = tx_clone.send(Message::Text(serde_json::to_string(&json!({
                                        "type": "message",
                                        "content": msg
                                    })).unwrap()));
                                } else {
                                    // Hide progress indicator
                                    let _ = tx_clone.send(Message::Text(serde_json::to_string(&json!({
                                        "type": "progress_hide"
                                    })).unwrap()));

                                    // Extract article data from rows (include stored summary language)
                                    use sqlx::Row;
                                    let article_data: Vec<(i64, String, String, Option<String>, String, f64, String, Option<String>)> = articles.iter()
                                        .map(|row| {
                                            let article_id: i64 = row.get("article_id");
                                            let headline: String = row.get("personalized_headline");
                                            let bullets: String = row.get("personalized_bullets");
                                            let details: Option<String> = row.try_get("personalized_details").ok();
                                            let article_lang: String = row.get("language");
                                            let relevance: f64 = row.get("relevance_score");
                                            let url: String = row.get("canonical_url");
                                            let feed_title: Option<String> = row.try_get("feed_title").ok();
                                            (article_id, headline, bullets, details, article_lang, relevance, url, feed_title)
                                        })
                                        .collect();

                                    // STREAMING MODE: Send articles as individual cards
                                    for (article_id, headline, bullets_json, details, article_lang, _relevance, url, feed_title) in article_data {
                                        // Construct raw summary
                                        // Borrow the inner string to avoid moving `details` so it can still be used later.
                                        let raw_summary = if let Some(ref d) = details {
                                            d.clone()
                                        } else {
                                            let bullets: Vec<String> = serde_json::from_str(&bullets_json).unwrap_or_default();
                                            bullets.join(" ")
                                        };

                                        let theme = feed_title.clone().unwrap_or_else(|| "Actualité".to_string());
                                        let source_name = feed_title.unwrap_or_else(|| "Unknown".to_string());

                                        // JIT REFINEMENT: Translate & Fix Truncation & Remove Markdown
                                        // We call the LLM to ensure the content is in the user's language and properly formatted.
                                        
                                        // Truncate input to avoid context limits and reduce noise (e.g. footers/links)
                                        let input_text = if raw_summary.len() > 2000 {
                                            format!("{}...", &raw_summary[..2000])
                                        } else {
                                            raw_summary.clone()
                                        };

                                        let refine_prompt = format!(
                                            "Task: Translate and refine this news item for a {} speaker.
                                    
                                    Original Headline: {}
                                    Content Snippet: {}

                                    Requirements:
                                    1. Language: {} ONLY (for the content).
                                    2. No truncation: Keep the content complete.
                                    3. No Markdown: Output PLAIN TEXT only.
                                    4. Format: Use the exact format below. DO NOT translate the keywords TITLE and SUMMARY.
                                    TITLE: <title>
                                    SUMMARY: <summary>
                                    5. No chatter: Do NOT add intro/outro text. Do NOT add notes like '(Note: ...)'.
                                    6. STRICT: Return ONLY the TITLE and SUMMARY sections.
                                    ",
                                            match user_profile_lang.as_str() {
                                                "fr" => "French",
                                                "es" => "Spanish",
                                                "de" => "German",
                                                "it" => "Italian",
                                                _ => "English"
                                            },
                                            headline,
                                            input_text,
                                            match user_profile_lang.as_str() {
                                                "fr" => "French",
                                                "es" => "Spanish",
                                                "de" => "German",
                                                "it" => "Italian",
                                                _ => "English"
                                            }
                                        );

                                        let (final_title, final_summary, final_lang) = match llm_provider.generate(crate::llm::LlmRequest {
                                            prompt: refine_prompt,
                                            max_tokens: Some(600),
                                            temperature: Some(0.3),
                                            timeout_seconds: Some(45),
                                        }).await {
                                            Ok(resp) => {
                                                // Robust parsing of TITLE: ... SUMMARY: ...
                                                // We accept French variants as fallback if the model disobeys instructions
                                                let content = resp.content.trim();
                                                
                                                let find_marker = |text: &str, markers: &[&str]| -> Option<(usize, usize)> {
                                                    for m in markers {
                                                        if let Some(idx) = text.find(m) {
                                                            return Some((idx, m.len()));
                                                        }
                                                    }
                                                    None
                                                };

                                                let title_marker = find_marker(content, &["TITLE:", "TITRE:", "Title:", "Titre:"]);
                                                let summary_marker = find_marker(content, &["SUMMARY:", "RESUME:", "RÉSUMÉ:", "Summary:", "Resume:", "Résumé:"]);
                                                
                                                if let (Some((t_idx, t_len)), Some((s_idx, s_len))) = (title_marker, summary_marker) {
                                                    if t_idx < s_idx {
                                                        let title_part = content[t_idx + t_len..s_idx].trim().to_string();
                                                        let mut summary_part = content[s_idx + s_len..].trim().to_string();

                                                        // Heuristic to strip common trailing notes if the model ignores checking
                                                        // e.g. "(Note: ...)" "\nNote: ..."
                                                        // We look for the last occurrence of such patterns if they are near the end
                                                        if let Some(note_idx) = summary_part.rfind("(Note:") {
                                                            if note_idx > 10 { summary_part.truncate(note_idx); }
                                                        } else if let Some(note_idx) = summary_part.rfind("(Nota:") {
                                                            if note_idx > 10 { summary_part.truncate(note_idx); }
                                                        } else if let Some(note_idx) = summary_part.rfind("\nNote:") {
                                                            if note_idx > 10 { summary_part.truncate(note_idx); }
                                                        }

                                                        let summary_clean = summary_part.trim().to_string();
                                                        
                                                        if !title_part.is_empty() && !summary_clean.is_empty() {
                                                             (title_part, summary_clean, user_profile_lang.clone())
                                                        } else {
                                                            error!("JIT Refinement: parsed empty fields");
                                                            (headline.clone(), raw_summary.clone(), article_lang.clone())
                                                        }
                                                    } else {
                                                        error!("JIT Refinement: markers out of order");
                                                        (headline.clone(), raw_summary.clone(), article_lang.clone())
                                                    }
                                                } else {
                                                    // Markers not found. If the response is non-empty, use it as summary
                                                    // This handles cases where the model forgets "SUMMARY:" but produces good text.
                                                    if !content.is_empty() && content.len() > 20 {
                                                        // Assume the whole text is the summary, keep original title
                                                        info!("JIT Refinement: markers missing, using full content as summary");
                                                        (headline.clone(), content.to_string(), user_profile_lang.clone())
                                                    } else {
                                                        error!("JIT Refinement: response too short or invalid");
                                                        (headline.clone(), raw_summary.clone(), article_lang.clone())
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                error!("JIT refinement failed: {}", e);
                                                (headline.clone(), raw_summary.clone(), article_lang.clone())
                                            }
                                        };

                                        // Update shared context
                                        if let Ok(mut ctx) = article_context_bg.lock() {
                                            ctx.push(ArticleContext {
                                                title: final_title.clone(),
                                                summary: final_summary.clone(),
                                                content: details.clone(), // Use details as content snippet if available
                                            });
                                        }

                                        // Send card (set lang to the content language)
                                        let card = json!({
                                            "type": "news_card",
                                            "article": {
                                                "id": article_id,
                                                "title": final_title,
                                                "summary": final_summary,
                                                "source": { "name": source_name },
                                                "url": url,
                                                "theme": theme,
                                                "lang": final_lang
                                            }
                                        });
                                        let _ = tx_clone.send(Message::Text(serde_json::to_string(&card).unwrap()));

                                        // Mark as viewed immediately
                                        let _ = sqlx::query(
                                            "INSERT OR IGNORE INTO user_article_views (user_id, article_id, session_id) VALUES (?, ?, ?)"
                                        )
                                        .bind(user_id)
                                        .bind(article_id)
                                        .bind(session_id)
                                        .execute(&pool)
                                        .await;

                                        // Small delay for progressive effect
                                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                                    }

                                    // Final message
                                    let completion_msg = match language_clone.as_str() {
                                        "fr" => "Voilà pour l'essentiel de l'actualité. Souhaitez-vous approfondir un sujet ?",
                                        "es" => "Eso es todo por ahora. ¿Desea profundizar en algún tema?",
                                        "de" => "Das war das Wichtigste. Möchten Sie ein Thema vertiefen?",
                                        "it" => "Questo è tutto per ora. Vuoi approfondire un argomento?",
                                        _ => "That's the main news. Would you like to explore any topic further?"
                                    };

                                    let _ = crate::sessions::store_message(&pool, session_id, "assistant", completion_msg).await;
                                    let _ = tx_clone.send(Message::Text(serde_json::to_string(&json!({
                                        "type": "message",
                                        "content": completion_msg
                                    })).unwrap()));
                                }
                            }
                            Err(e) => {
                                error!("Failed to fetch personalized articles for user {}: {:?}", user_id, e);
                                let msg = "I'm having trouble accessing the latest news. Please try again later.";
                                let _ = tx_clone.send(Message::Text(serde_json::to_string(&json!({
                                    "type": "message",
                                    "content": msg
                                })).unwrap()));
                            }
                        }
                    });
                }
            } else {
                // Existing session: replay history
                for msg in messages {
                    let role = if msg.author == "user" { "user" } else { "assistant" };
                    send_json(&tx, json!({
                        "type": "history",
                        "role": role,
                        "content": msg.message
                    }));
                }
            }

            // Handle incoming messages
            while let Some(message) = ws_stream.next().await {
                match message {
                    Ok(Message::Text(text)) => {
                        info!("Received message for session {}: {}", session_id, text);

                        // Parse user message
                        let json_msg: serde_json::Value = serde_json::from_str(&text).unwrap_or(json!({"type": "message", "message": text}));

                        if json_msg["type"] == "rate" {
                            // Handle Rating
                            if let (Some(article_id), Some(rating)) = (json_msg["article_id"].as_i64(), json_msg["rating"].as_i64()) {
                                info!("User {} rated article {} with {} stars", user_id, article_id, rating);
                                let _ = sqlx::query(
                                    "UPDATE user_article_views SET rating = ? WHERE user_id = ? AND article_id = ?"
                                )
                                .bind(rating)
                                .bind(user_id)
                                .bind(article_id)
                                .execute(&pool)
                                .await;
                            }
                            continue;
                        }

                        let user_message = if json_msg["type"] == "message" {
                            json_msg["message"].as_str().unwrap_or(&text).to_string()
                        } else {
                            text
                        };

                        // Store user message
                        if let Err(e) = store_message(&pool, session_id, "user", &user_message).await {
                            error!("Failed to store user message: {}", e);
                            continue;
                        }

                        // Generate LLM response
                        let response = if let Some(ref provider) = llm {
                            // Get current articles context
                            let current_articles = article_context_chat.lock()
                                .map(|guard| guard.clone())
                                .unwrap_or_default();

                            match handle_chat_message(&pool, provider, session_id, &user_message, &current_articles).await {
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
                        send_json(&tx, json!({
                            "type": "message",
                            "author": "assistant",
                            "message": response,
                        }));
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

/// Context for an article to be used in chat
#[derive(Clone, Debug)]
pub struct ArticleContext {
    pub title: String,
    pub summary: String,
    pub content: Option<String>,
}

/// Handle chat message with LLM
async fn handle_chat_message(
    pool: &SqlitePool,
    llm_provider: &Arc<dyn LlmProvider>,
    session_id: i64,
    user_message: &str,
    articles: &[ArticleContext],
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
            _ => "English",
        }
        .to_string();
    }

    // Build conversation context
    let mut context = format!(
        "You are a helpful news assistant for Newscope. \
         The user is exploring their personalized news feed. \
         Answer questions concisely and help them understand the news. \
         IMPORTANT: You MUST answer in {}.\n\n",
        language
    );

    // Add article context if available
    if !articles.is_empty() {
        context.push_str("Here are the articles in the user's current session:\n\n");
        for (i, article) in articles.iter().enumerate() {
            context.push_str(&format!(
                "Article {}:\nTitle: {}\nSummary: {}\n",
                i + 1,
                article.title,
                article.summary
            ));
            if let Some(content) = &article.content {
                // Truncate content to avoid token limit issues, e.g. 500 chars
                let truncated = if content.len() > 500 {
                    format!("{}...", &content[0..500])
                } else {
                    content.clone()
                };
                context.push_str(&format!("Content Snippet: {}\n", truncated));
            }
            context.push_str("\n");
        }
        context.push_str("Use the above articles to answer the user's questions if relevant.\n\n");
    }

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
