use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};
use std::sync::Arc;
use tracing::{info, warn};

use crate::llm::{LlmProvider, Summary};
use crate::personalization::{
    evaluate_article_relevance, generate_personalized_summary, get_user_profile,
};

/// Personalize article for all active users after generic summary generated
pub async fn personalize_for_users(
    pool: &SqlitePool,
    article_id: i64,
    generic_summary: &Summary,
    llm_provider: Arc<dyn LlmProvider>,
    model: &str,
) -> Result<usize> {
    // Get all active users (include users without explicit preferences)
    info!(
        "Fetching users for article personalization (including users without explicit preferences)"
    );
    let users = sqlx::query("SELECT DISTINCT u.id FROM users u")
        .fetch_all(pool)
        .await
        .context("Failed to fetch active users")?;

    info!(
        "Found {} users with preferences for personalization",
        users.len()
    );

    if users.is_empty() {
        info!("No active users to personalize for");
        return Ok(0);
    }

    let total_users = users.len();
    let mut personalized_count = 0;

    for user_row in users {
        let user_id: i64 = user_row.get("id");

        // Fetch user profile
        let user_profile = match get_user_profile(pool, user_id).await {
            Ok(profile) => profile,
            Err(e) => {
                warn!("Failed to fetch profile for user {}: {}", user_id, e);
                continue;
            }
        };

        // 1. Evaluate relevance
        let relevance =
            match evaluate_article_relevance(llm_provider.as_ref(), generic_summary, &user_profile)
                .await
            {
                Ok(eval) => eval,
                Err(e) => {
                    warn!(
                        "Failed to evaluate relevance for user {} article {}: {}",
                        user_id, article_id, e
                    );
                    continue;
                }
            };

        // Skip if not relevant (score < 0.3)
        if relevance.score < 0.3 {
            info!(
                "Article {} not relevant for user {} (score: {})",
                article_id, user_id, relevance.score
            );
            continue;
        }

        // 2. Generate personalized summary
        let personalized = match generate_personalized_summary(
            llm_provider.as_ref(),
            generic_summary,
            &user_profile,
            relevance.score,
        )
        .await
        {
            Ok(summary) => summary,
            Err(e) => {
                warn!(
                    "Failed to personalize for user {} article {}: {}",
                    user_id, article_id, e
                );
                continue;
            }
        };

        // 3. Store in database
        let relevance_reasons_json = serde_json::to_string(&relevance.reasons)?;
        let bullets_json = serde_json::to_string(&personalized.bullets)?;

        match sqlx::query(
            "INSERT OR REPLACE INTO user_article_summaries
             (user_id, article_id, relevance_score, relevance_reasons, is_relevant,
              personalized_headline, personalized_bullets, personalized_details,
              language, complexity_level, summary_length, llm_model,
              prompt_tokens, completion_tokens)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(article_id)
        .bind(relevance.score)
        .bind(relevance_reasons_json)
        .bind(true)
        .bind(&personalized.headline)
        .bind(bullets_json)
        .bind(&personalized.details)
        .bind(&user_profile.language)
        .bind(&user_profile.complexity_level)
        .bind(&personalized.length)
        .bind(model)
        .bind(personalized.usage.prompt_tokens as i64)
        .bind(personalized.usage.completion_tokens as i64)
        .execute(pool)
        .await
        {
            Ok(_) => {
                info!(
                    "Personalized article {} for user {} (relevance: {:.2})",
                    article_id, user_id, relevance.score
                );
                personalized_count += 1;
            }
            Err(e) => {
                warn!(
                    "Failed to store personalized summary for user {} article {}: {}",
                    user_id, article_id, e
                );
            }
        }
    }

    info!(
        "Personalized article {} for {}/{} active users",
        article_id, personalized_count, total_users
    );

    Ok(personalized_count)
}
