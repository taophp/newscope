use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{SqlitePool, Row};
use std::sync::Arc;
use tracing::info;

use crate::llm::LlmProvider;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ScoredArticle {
    pub id: i64,
    pub headline: String,
    pub bullets: Vec<String>,
    pub feed_title: String,
    pub article_title: String,
    pub url: String,
    pub score: f64,
    pub categories: Vec<String>,
    pub published_at: DateTime<Utc>,
}

/// Fetch and score articles based on user preferences
/// Returns ALL articles with summaries, regardless of publication date
pub async fn fetch_and_score_articles(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Vec<ScoredArticle>> {
    // 1. Fetch user preferences
    let prefs = sqlx::query(
        "SELECT preference_type, preference_key, preference_value FROM user_preferences WHERE user_id = ?"
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut category_weights = std::collections::HashMap::new();
    
    for row in prefs {
        let p_type: String = row.get("preference_type");
        let key: String = row.get("preference_key");
        let val: f64 = row.get("preference_value");
        
        if p_type == "category_filter" {
            category_weights.insert(key.to_lowercase(), val);
        }
    }

    // 2. Fetch articles with summaries (all articles, not just recent ones)
    // This ensures we don't miss articles from newly added feeds or failed processing
    // BUT we exclude articles the user has already seen
    let rows = sqlx::query(
        r#"
        SELECT 
            a.id,
            s.headline, 
            s.bullets_json, 
            s.categories,
            f.title as feed_title,
            a.title as article_title,
            a.canonical_url,
            a.first_seen_at
        FROM article_summaries s
        JOIN articles a ON s.article_id = a.id
        JOIN article_occurrences ao ON a.id = ao.article_id
        JOIN subscriptions sub ON ao.feed_id = sub.feed_id
        JOIN feeds f ON sub.feed_id = f.id
        LEFT JOIN user_article_views uav ON uav.user_id = ? AND uav.article_id = a.id
        WHERE sub.user_id = ?
        AND uav.id IS NULL  -- Exclude articles already viewed
        "#
    )
    .bind(user_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("Failed to fetch article summaries")?;

    let mut scored_articles = Vec::new();

    for row in rows {
        let id: i64 = row.get("id");
        let headline: String = row.get("headline");
        let bullets_json: String = row.get("bullets_json");
        let categories_json: Option<String> = row.get("categories");
        let feed_title: String = row.get("feed_title");
        let article_title: String = row.get("article_title");
        let url: String = row.get("canonical_url");
        let published_at: DateTime<Utc> = row.get("first_seen_at");

        let bullets: Vec<String> = serde_json::from_str(&bullets_json).unwrap_or_default();
        let categories: Vec<String> = categories_json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();

        // Scoring logic
        let mut score = 1.0;
        
        // Recency boost (newer is better)
        let age_hours = (Utc::now() - published_at).num_hours() as f64;
        score += (24.0 - age_hours).max(0.0) * 0.05; // Up to +1.2 for very new

        // Category weights
        for cat in &categories {
            if let Some(weight) = category_weights.get(cat) {
                if *weight < 0.0 {
                    score = -1.0; // Blocked
                    break;
                }
                score += weight;
            }
        }

        if score > 0.0 {
            scored_articles.push(ScoredArticle {
                id,
                headline,
                bullets,
                feed_title,
                article_title,
                url,
                score,
                categories,
                published_at,
            });
        }
    }

    // Sort by score descending
    scored_articles.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    Ok(scored_articles)
}

/// Generate a personalized press review for a user (Advanced Half-Life Selection)
pub async fn generate_press_review(
    pool: &SqlitePool,
    user_id: i64,
    _llm_provider: Arc<dyn LlmProvider>,
    _model: &str,
    duration_seconds: i64,
) -> Result<String> {
    // 1. Fetch user profile
    let user = crate::personalization::get_user_profile(pool, user_id).await?;
    let reading_speed = user.reading_speed as f64; // wpm
    
    info!("Generating half-life press review for user {} (speed: {} wpm, budget: {}s)", 
          user_id, reading_speed, duration_seconds);

    // 2. Calculate average publication interval per feed (Frequency Analysis)
    // We look at the last 20 articles per feed to determine their "cadence"
    let feed_stats_rows = sqlx::query(
        r#"
        WITH lead_times AS (
            SELECT 
                ao.feed_id, 
                a.first_seen_at,
                LAG(a.first_seen_at) OVER (PARTITION BY ao.feed_id ORDER BY a.first_seen_at) as prev_seen
            FROM article_occurrences ao
            JOIN articles a ON ao.article_id = a.id
            WHERE ao.feed_id IN (SELECT feed_id FROM subscriptions WHERE user_id = ?)
        )
        SELECT 
            feed_id,
            AVG(unixepoch(first_seen_at) - unixepoch(prev_seen)) as avg_interval_seconds
        FROM lead_times
        WHERE prev_seen IS NOT NULL
        GROUP BY feed_id
        "#
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("Failed to calculate feed publication frequencies")?;

    let mut feed_half_lives = std::collections::HashMap::new();
    for row in feed_stats_rows {
        let feed_id: i64 = row.get("feed_id");
        let avg_interval: f64 = row.get::<Option<f64>, _>("avg_interval_seconds").unwrap_or(86400.0); // Default 1 day
        
        // Half-life concept: T1/2 = 10 * average interval
        // An article loses 50% value after 10 "expected" publication intervals
        let half_life_secs = (avg_interval * 10.0).clamp(3600.0, 31536000.0); // 1h to 1 year
        feed_half_lives.insert(feed_id, half_life_secs);
    }

    // 3. Fetch candidate articles: last 30 unread articles per feed (relative window)
    let rows = sqlx::query(
        r#"
        WITH ranked_articles AS (
            SELECT 
                uas.id as summary_id,
                uas.user_id,
                uas.article_id,
                uas.relevance_score,
                uas.relevance_reasons,
                uas.is_relevant,
                uas.personalized_headline,
                uas.personalized_bullets,
                uas.personalized_details,
                uas.language,
                uas.complexity_level,
                uas.summary_length,
                uas.created_at,
                uas.llm_model,
                uas.prompt_tokens,
                uas.completion_tokens,
                ao.feed_id,
                ROW_NUMBER() OVER (PARTITION BY ao.feed_id ORDER BY a.first_seen_at DESC) as rank
            FROM user_article_summaries uas
            JOIN articles a ON uas.article_id = a.id
            JOIN article_occurrences ao ON a.id = ao.article_id
            JOIN subscriptions sub ON ao.feed_id = sub.feed_id AND sub.user_id = uas.user_id
            LEFT JOIN user_article_views uav ON uav.user_id = uas.user_id AND uav.article_id = uas.article_id
            WHERE uas.user_id = ?
            AND uav.id IS NULL
        )
        SELECT * FROM ranked_articles WHERE rank <= 30
        "#
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("Failed to fetch top articles per feed")?;

    if rows.is_empty() {
        return Ok(if user.language == "fr" { 
            "Pas de nouveaux articles trouvés.".to_string() 
        } else { 
            "No new articles found.".to_string() 
        });
    }

    // 4. Calculate Final Score with Semantic Similarity & Exponential Half-Life Decay
    // Fetch user vector
    let user_vector = crate::personalization::get_user_vector(pool, user_id).await.unwrap_or(None);
    
    let mut scored_articles = Vec::new();
    for row in rows {
        let feed_id: i64 = row.get("feed_id");
        let article_id: i64 = row.get("article_id");
        let relevance_score: f64 = row.get("relevance_score");
        let created_at_str: String = row.get("created_at");
        
        // Semantic Boost
        let mut semantic_similarity = 0.5; // Default neutral if no vectors
        if let Some(uv) = &user_vector {
             // Fetch article vector
             let article_vec_row = sqlx::query(
                 "SELECT vec_distance_cosine(v.embedding, ?) as distance 
                  FROM vec_articles v 
                  WHERE v.article_id = ?"
             )
             .bind(f32_vec_to_bytes(uv))
             .bind(article_id)
             .fetch_optional(pool)
             .await
             .unwrap_or(None);
             
             if let Some(avr) = article_vec_row {
                  let distance: f64 = avr.get("distance");
                  semantic_similarity = (1.0 - distance).max(0.0);
             }
        }

        let created_at = match DateTime::parse_from_rfc3339(&created_at_str) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(_) => {
                // Try SQLite format: YYYY-MM-DD HH:MM:SS
                chrono::NaiveDateTime::parse_from_str(&created_at_str, "%Y-%m-%d %H:%M:%S")
                    .map(|ndt| ndt.and_utc())
                    .unwrap_or_else(|_| Utc::now())
            }
        };
        
        let age_secs = (Utc::now() - created_at).num_seconds() as f64;
        let t_half = feed_half_lives.get(&feed_id).cloned().unwrap_or(864000.0); // Default 10 days
        
        let decay_exponent = age_secs / t_half;
        let freshness_boost = 2.0_f64.powf(-decay_exponent);
        
        // Final Score blend: (LLM Relevance * 0.4) + (Semantic Similarity * 0.6)
        // Then apply freshness decay
        let blended_score = (relevance_score * 0.4) + (semantic_similarity * 0.6);
        let final_score = blended_score * freshness_boost;
        
        let headline: String = row.get("personalized_headline");
        let bullets_json: String = row.get("personalized_bullets");
        
        scored_articles.push((final_score, article_id, headline, bullets_json));
    }

    // Sort by final score
    scored_articles.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // 5. Budgeting & Formatting
    let target_words = ((duration_seconds as f64 / 60.0) / 2.0 * reading_speed) as usize;
    let target_words = target_words.clamp(100, 3000);

    let mut digest = String::new();
    if user.language == "fr" {
        digest.push_str("# Revue de Presse : Sélection Dynamique\n\n");
    } else {
        digest.push_str("# Press Review: Dynamic Selection\n\n");
    }

    let mut current_words = 0;
    let mut article_count = 0;

    for (_score, article_id, headline, bullets_json) in scored_articles {
        if current_words >= target_words && article_count >= 3 {
             break;
        }

        let (feed_title, url): (String, String) = sqlx::query_as(
            "SELECT f.title, a.canonical_url FROM articles a JOIN article_occurrences ao ON a.id = ao.article_id JOIN feeds f ON ao.feed_id = f.id WHERE a.id = ? LIMIT 1"
        )
        .bind(article_id)
        .fetch_one(pool)
        .await
        .unwrap_or(("Source".to_string(), "#".to_string()));

        let bullets: Vec<String> = serde_json::from_str(&bullets_json).unwrap_or_default();
        let article_text = format!(
            "## {}\n{}\n\n*Source: {} • [Lire l'article]({})*\n\n", 
            headline,
            bullets.iter().map(|b| format!("- {}", b)).collect::<Vec<_>>().join("\n"),
            feed_title,
            url
        );

        let word_count = article_text.split_whitespace().count();
        if current_words + word_count > target_words + 200 && article_count >= 3 {
            break;
        }

        digest.push_str(&article_text);
        current_words += word_count;
        article_count += 1;
    }

    info!("Digest generated: {} articles, ~{} words", article_count, current_words);
    Ok(digest)
}

/// Helper to convert Vec<f32> to bytes for BLOB storage (bind parameter)
fn f32_vec_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}
