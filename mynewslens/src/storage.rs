use anyhow::{Context, Result};
use chrono::Utc;
use feed_rs::model::Entry;
use sqlx::SqlitePool;
use tracing::{info, debug};

use crate::scraping;

/// Stores a list of feed entries into the database.
/// Returns the IDs of newly inserted articles.
pub async fn store_feed_items(
    pool: &SqlitePool,
    feed_id: i64,
    entries: &[Entry],
) -> Result<Vec<i64>> {
    let mut new_article_ids = Vec::new();

    for entry in entries {
        // 1. Extract basic info
        let title = entry.title.as_ref().map(|t| t.content.clone()).unwrap_or_default();
        // Use the first link as the URL
        let url = entry.links.first().map(|l| l.href.clone()).unwrap_or_default();
        
        if url.is_empty() {
            debug!("Skipping entry without URL: {:?}", title);
            continue;
        }

        let published = entry.published.map(|d| d).unwrap_or_else(Utc::now);
        let mut content = entry.content.as_ref().map(|c| c.body.clone().unwrap_or_default())
            .or_else(|| entry.summary.as_ref().map(|s| s.content.clone()))
            .unwrap_or_default();

        // SCRAPING FALLBACK
        // If content is very short (likely just a summary or empty), try to scrape the page.
        // Threshold: 500 chars is arbitrary but reasonable for a "full article".
        if content.len() < 500 {
            info!("Content short ({}), attempting to scrape: {}", content.len(), url);
            // We use a default timeout of 10s for scraping for now
            match scraping::scrape_article_content(&url, 10).await {
                Ok(scraped) => {
                    if scraped.len() > content.len() {
                        info!("Scraping successful, replaced content ({} -> {} chars)", content.len(), scraped.len());
                        content = scraped;
                    } else {
                        info!("Scraping returned less content, keeping original");
                    }
                }
                Err(e) => {
                    // Log but don't fail the whole process
                    tracing::warn!("Failed to scrape {}: {}", url, e);
                }
            }
        }

        // 2. Check if article already exists (deduplication by URL)
        // In a real app, we might also check by title+date or hash if URL varies.
        // For now, simple URL check.
        let existing_id = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM articles WHERE canonical_url = ?"
        )
        .bind(&url)
        .fetch_optional(pool)
        .await
        .context("failed to check existing article")?;

        let article_id = if let Some(id) = existing_id {
            id
        } else {
            // Insert new article
            let id = sqlx::query_scalar::<_, i64>(
                r#"
                INSERT INTO articles (canonical_url, title, content, published_at, first_seen_at)
                VALUES (?, ?, ?, ?, ?)
                RETURNING id
                "#
            )
            .bind(&url)
            .bind(&title)
            .bind(&content)
            .bind(published)
            .bind(Utc::now())
            .fetch_one(pool)
            .await
            .context("failed to insert article")?;
            
            new_article_ids.push(id);
            id
        };

        // 3. Record occurrence for this feed
        // We use INSERT OR IGNORE to avoid duplicates if we re-fetch the same feed item
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO article_occurrences (article_id, feed_id, discovered_at)
            VALUES (?, ?, ?)
            "#
        )
        .bind(article_id)
        .bind(feed_id)
        .bind(Utc::now())
        .execute(pool)
        .await
        .context("failed to insert occurrence")?;
    }

    Ok(new_article_ids)
}

/// Store an article summary in the database
pub async fn store_article_summary(
    pool: &SqlitePool,
    article_id: i64,
    summary: &crate::llm::Summary,
    model: &str,
) -> Result<()> {
    let bullets_json = serde_json::to_string(&summary.bullets)
        .context("failed to serialize bullets")?;
    
    sqlx::query(
        r#"
        INSERT OR REPLACE INTO article_summaries 
        (article_id, headline, bullets_json, details, model, prompt_tokens, completion_tokens)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#
    )
    .bind(article_id)
    .bind(&summary.headline)
    .bind(&bullets_json)
    .bind(&summary.details)
    .bind(model)
    .bind(summary.usage.prompt_tokens as i32)
    .bind(summary.usage.completion_tokens as i32)
    .execute(pool)
    .await
    .context("failed to insert article summary")?;
    
    info!("Stored summary for article {}", article_id);
    Ok(())
}
