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

/// Initialize user vectors from their interest keywords if they don't have one
pub async fn initialize_user_vectors(
    pool: &SqlitePool,
    llm_provider: Arc<dyn LlmProvider>,
) -> Result<usize> {
    let users = sqlx::query(
        "SELECT u.id FROM users u LEFT JOIN vec_users vu ON u.id = vu.user_id WHERE vu.user_id IS NULL"
    )
    .fetch_all(pool)
    .await?;

    let mut count = 0;
    for user_row in users {
        let user_id: i64 = user_row.get("id");
        let profile = get_user_profile(pool, user_id).await?;
        
        if profile.interests.is_empty() {
            continue;
        }

        let interests_text = profile.interests.join(" ");
        match llm_provider.embed(&interests_text).await {
            Ok(embedding) => {
                crate::personalization::update_user_vector(pool, user_id, &embedding).await?;
                count += 1;
                info!("Initialized vector for user {} from interest keywords", user_id);
            }
            Err(e) => {
                warn!("Failed to initialize vector for user {}: {}", user_id, e);
            }
        }
    }

    Ok(count)
}

/// Update user vector based on an interaction with an article
pub async fn update_user_vector_from_interaction(
    pool: &SqlitePool,
    user_id: i64,
    article_id: i64,
    weight: f32, // 1.0 for view, 2.0 for rate/chat, etc.
) -> Result<()> {
    // 1. Get user vector
    let user_vec = crate::personalization::get_user_vector(pool, user_id).await?;
    
    // 2. Get article vector
    let article_vec_row = sqlx::query(
        "SELECT embedding FROM vec_articles WHERE article_id = ?"
    )
    .bind(article_id)
    .fetch_optional(pool)
    .await?;

    let Some(article_vec_row) = article_vec_row else {
        return Ok(()); // Article not vectorized yet
    };

    let article_bytes: Vec<u8> = article_vec_row.get("embedding");
    let article_vec: Vec<f32> = article_bytes.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();

    // 3. Update logic
    let new_vec = if let Some(uv) = user_vec {
        // new = (old * 0.9) + (article * 0.1 * weight)
        // This is a simple EMA (Exponential Moving Average) to allow interest drift
        let alpha = 0.1 * weight;
        uv.iter().zip(article_vec.iter())
            .map(|(u, a)| u * (1.0 - alpha) + a * alpha)
            .collect()
    } else {
        article_vec
    };

    crate::personalization::update_user_vector(pool, user_id, &new_vec).await?;
    info!("Updated vector for user {} based on interaction with article {}", user_id, article_id);
    
    Ok(())
}
