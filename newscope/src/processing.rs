use anyhow::{Context, Result};
use sqlx::{SqlitePool, Row};
use tracing::{info, warn, error};
use std::sync::Arc;

use crate::llm::{LlmProvider, summarizer};
use crate::storage;

/// Process multiple articles in batch with rate limiting
pub async fn batch_process_articles(
    pool: &SqlitePool,
    article_ids: &[i64],
    provider: Arc<dyn LlmProvider>,
    model: &str,
) -> Result<usize> {
    if article_ids.is_empty() {
        return Ok(0);
    }
    
    info!("Processing {} articles with LLM", article_ids.len());
    let mut processed_count = 0;
    
    // Process in batches of 5 to avoid overwhelming the LLM API
    const BATCH_SIZE: usize = 5;
    
    for chunk in article_ids.chunks(BATCH_SIZE) {
        for &article_id in chunk {
            match process_single_article(pool, article_id, provider.clone(), model).await {
                Ok(_) => {
                    processed_count += 1;
                }
                Err(e) => {
                    error!("Failed to process article {}: {}", article_id, e);
                    // Continue processing other articles despite error
                }
            }
        }
        
        // Rate limit: wait 2 seconds between batches
        if article_ids.len() > BATCH_SIZE {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }
    }
    
    info!("Processed {}/{} articles successfully", processed_count, article_ids.len());
    Ok(processed_count)
}

/// Process a single article: fetch content, summarize, store summary
async fn process_single_article(
    pool: &SqlitePool,
    article_id: i64,
    llm_provider: Arc<dyn LlmProvider>,
    model: &str,
) -> Result<()> {
    // Fetch article content from database
    let row = sqlx::query(
        "SELECT content, canonical_url FROM articles WHERE id = ?"
    )
    .bind(article_id)
    .fetch_optional(pool)
    .await
    .context("Failed to fetch article")?;

    let Some(row) = row else {
        warn!("Article {} not found, skipping", article_id);
        return Ok(());
    };

    let content: String = row.get("content");
    let url: String = row.get("canonical_url");
    
    // If content is too short (< 100 chars), try scraping the full article
    let final_content = if content.len() < 100 {
        info!("Article {} has short content ({}), attempting to scrape from {}", 
              article_id, content.len(), url);
        
        match crate::scraping::scrape_article_content(&url, 10).await {
            Ok(scraped) => {
                info!("Successfully scraped article {}, got {} chars", article_id, scraped.len());
                scraped
            }
            Err(e) => {
                warn!("Failed to scrape article {}: {}, using original content", article_id, e);
                content
            }
        }
    } else {
        content
    };

    // Skip if still too short after scraping attempt
    if final_content.len() < 50 {
        info!("Article {} content too short even after scraping ({}), skipping summarization", 
              article_id, final_content.len());
        return Ok(());
    }
    // Convert HTML to Markdown for cleaner LLM input
    // We use a width of 80 chars for wrapping, but the LLM doesn't care much about wrapping.
    let markdown_content = html2text::from_read(final_content.as_bytes(), 80)
        .context("Failed to convert HTML to Markdown")?;
    
    // Generate summary
    info!("Summarizing article {} with {} chars of content (converted to {} chars markdown)", 
          article_id, final_content.len(), markdown_content.len());
    
    let summary = crate::llm::summarizer::summarize_article(
        llm_provider.as_ref(),
        &markdown_content,
        500,
    )
    .await;

    info!("Summary generated for article {}: headline='{}'", article_id, summary.headline);

    // Store summary
    crate::storage::store_article_summary(
        pool,
        article_id,
        &summary,
        model,
    )
    .await
    .context("Failed to store article summary")?;

    // Mark article as processed
    sqlx::query(
        "UPDATE articles SET processing_status = 'completed', processed_at = ? WHERE id = ?"
    )
    .bind(chrono::Utc::now())
    .bind(article_id)
    .execute(pool)
    .await
    .context("Failed to update article processing status")?;

    info!(
        "Processed article {}: headline='{}', bullets={}, tokens={}",
        article_id,
        summary.headline,
        summary.bullets.len(),
        summary.usage.total_tokens
    );

    Ok(())
}

/// Process all pending articles (those with processing_status = 'pending')
pub async fn process_pending_articles(
    pool: &SqlitePool,
    provider: Arc<dyn LlmProvider>,
    model: &str,
    limit: Option<usize>,
) -> Result<usize> {
    // Find pending articles
    let limit_clause = limit.map(|l| format!("LIMIT {}", l)).unwrap_or_default();
    let query = format!(
        "SELECT id FROM articles WHERE processing_status = 'pending' ORDER BY first_seen_at DESC {}",
        limit_clause
    );
    
    let rows = sqlx::query(&query)
        .fetch_all(pool)
        .await
        .context("Failed to fetch pending articles")?;
    
    let article_ids: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
    
    if article_ids.is_empty() {
        info!("No pending articles to process");
        return Ok(0);
    }
    
    info!("Found {} pending articles to process", article_ids.len());
    batch_process_articles(pool, &article_ids, provider, model).await
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_batch_chunking() {
        let ids: Vec<i64> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let chunks: Vec<_> = ids.chunks(5).collect();
        
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 5);
        assert_eq!(chunks[1].len(), 5);
        assert_eq!(chunks[2].len(), 2);
    }
}
