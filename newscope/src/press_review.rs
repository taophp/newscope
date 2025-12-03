use anyhow::{Context, Result};
use chrono::{DateTime, Utc, Duration};
use sqlx::{SqlitePool, Row};
use std::sync::Arc;
use tracing::{info, warn};

use crate::llm::LlmProvider;

/// Generate a personalized press review for a user
pub async fn generate_press_review(
    pool: &SqlitePool,
    user_id: i64,
    llm_provider: Arc<dyn LlmProvider>,
    model: &str,
    duration_seconds: i64,
) -> Result<String> {
    // 1. Get user's last login time (or default to 24h ago)
    let last_login: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT last_login FROM users WHERE id = ?"
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("Failed to fetch user last_login")?
    .flatten(); // Flatten Option<Option<DateTime>> to Option<DateTime> if column is nullable

    let since = last_login.unwrap_or_else(|| Utc::now() - Duration::hours(24));
    
    info!("Generating press review for user {} since {}", user_id, since);

    // 2. Fetch relevant article summaries
    // We limit to 20 articles to fit in context
    let rows = sqlx::query(
        r#"
        SELECT 
            s.headline, 
            s.bullets_json, 
            f.title as feed_title,
            a.title as article_title,
            a.canonical_url,
            a.first_seen_at
        FROM article_summaries s
        JOIN articles a ON s.article_id = a.id
        JOIN article_occurrences ao ON a.id = ao.article_id
        JOIN subscriptions sub ON ao.feed_id = sub.feed_id
        JOIN feeds f ON sub.feed_id = f.id
        WHERE sub.user_id = ?
        AND a.first_seen_at > ?
        ORDER BY a.first_seen_at DESC
        LIMIT 20
        "#
    )
    .bind(user_id)
    .bind(since)
    .fetch_all(pool)
    .await
    .context("Failed to fetch article summaries")?;

    if rows.is_empty() {
        return Ok("Welcome back! I haven't found any new articles since your last visit.".to_string());
    }

    // Calculate target length: half the session duration, assuming 200 wpm
    // duration_seconds / 60 (mins) / 2 * 200 = duration_seconds * 1.66
    let target_words = (duration_seconds as f64 * 1.6) as usize;
    let target_words = target_words.clamp(100, 2000); // Reasonable limits

    // 3. Construct prompt
    let mut prompt = String::new();
    prompt.push_str("You are a personal news editor. Generate a personalized press review based on the following article summaries.\n");
    prompt.push_str("Group the news by topic/feed. Highlight the most important information.\n");
    prompt.push_str(&format!("The user has allocated {} minutes for this session. Aim for a reading time of about {} minutes (approx. {} words).\n", duration_seconds / 60, duration_seconds / 60 / 2, target_words));
    prompt.push_str("Use Markdown formatting to make it readable (bold, lists, headers).\n");
    prompt.push_str("Keep it conversational and engaging.\n\n");

    let mut current_feed = String::new();
    
    for row in rows {
        let feed_title: String = row.get("feed_title");
        let headline: String = row.get("headline");
        let bullets_json: String = row.get("bullets_json");
        let url: String = row.get("canonical_url");
        
        if feed_title != current_feed {
            prompt.push_str(&format!("\n## Source: {}\n", feed_title));
            current_feed = feed_title;
        }
        
        prompt.push_str(&format!("- **{}**\n", headline));
        
        if let Ok(bullets) = serde_json::from_str::<Vec<String>>(&bullets_json) {
            for bullet in bullets.iter().take(2) {
                prompt.push_str(&format!("  * {}\n", bullet));
            }
        }
        prompt.push_str(&format!("  [Read more]({})\n", url));
    }

    prompt.push_str("\n\nPress Review:");

    // 4. Call LLM
    let request = crate::llm::LlmRequest {
        prompt,
        max_tokens: Some(1000),
        temperature: Some(0.7),
        timeout_seconds: Some(60),
    };

    let response = llm_provider.generate(request).await
        .context("LLM generation failed")?;
    
    let summary = response.content;

    Ok(summary)
}
