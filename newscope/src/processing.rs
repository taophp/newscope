use anyhow::{Context, Result};
use sqlx::{SqlitePool, Row};
use tracing::{info, warn, error};
use std::sync::Arc;

use crate::llm::{LlmProvider, summarizer, LlmRequest};

/// Helper to create a processing job
async fn create_processing_job(
    pool: &SqlitePool,
    job_type: &str,
    entity_id: i64,
    model: &str,
) -> Result<i64> {
    let job_id = sqlx::query(
        "INSERT INTO processing_jobs (job_type, entity_id, status, llm_model, created_at) VALUES (?, ?, 'pending', ?, datetime('now')) RETURNING id"
    )
    .bind(job_type)
    .bind(entity_id)
    .bind(model)
    .fetch_one(pool)
    .await?
    .get(0);
    Ok(job_id)
}

/// Helper to update job status
async fn update_job_status(
    pool: &SqlitePool,
    job_id: i64,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    let now = if status == "completed" || status == "failed" {
        Some(chrono::Utc::now())
    } else {
        None
    };

    let query = if let Some(completed_at) = now {
        sqlx::query("UPDATE processing_jobs SET status = ?, error_message = ?, completed_at = ? WHERE id = ?")
            .bind(status)
            .bind(error)
            .bind(completed_at)
            .bind(job_id)
    } else {
        sqlx::query("UPDATE processing_jobs SET status = ?, error_message = ?, started_at = datetime('now') WHERE id = ?")
            .bind(status)
            .bind(error)
            .bind(job_id)
    };
    
    query.execute(pool).await?;
    Ok(())
}

/// Helper to complete job with stats
async fn complete_processing_job(
    pool: &SqlitePool,
    job_id: i64,
    prompt_tokens: usize,
    completion_tokens: usize,
    processing_time_ms: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE processing_jobs SET status = 'completed', completed_at = datetime('now'), prompt_tokens = ?, completion_tokens = ?, processing_time_ms = ? WHERE id = ?"
    )
    .bind(prompt_tokens as i64)
    .bind(completion_tokens as i64)
    .bind(processing_time_ms)
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Classify article using LLM
async fn classify_article(
    llm_provider: &dyn LlmProvider,
    headline: &str,
    summary_bullets: &[String],
) -> Result<Vec<String>> {
    let prompt = format!(
        "Classify this article into categories (max 3): {}\n\nKey points: {}\n\n\
         Categories: politics, economy, technology, sports, culture, science, \
         local_news, international, faits_divers, health, environment\n\n\
         Return only category names, comma-separated.",
        headline,
        summary_bullets.join(", ")
    );
    
    let response = llm_provider.generate(LlmRequest {
        prompt,
        max_tokens: Some(50),
        temperature: Some(0.3),
        timeout_seconds: Some(10),
    }).await?;
    
    Ok(response.content
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .collect())
}

/// Process multiple articles in batch with rate limiting
pub async fn batch_process_articles(
    pool: &SqlitePool,
    article_ids: &[i64],
    summarization_provider: Arc<dyn LlmProvider>,
    personalization_provider: Option<Arc<dyn LlmProvider>>,
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
            match process_single_article(pool, article_id, summarization_provider.clone(), personalization_provider.clone(), model).await {
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
    summarization_provider: Arc<dyn LlmProvider>,
    personalization_provider: Option<Arc<dyn LlmProvider>>,
    model: &str,
) -> Result<()> {
    // 1. Create job
    let job_id = create_processing_job(pool, "article_summary", article_id, model).await?;
    
    // 2. Mark running
    update_job_status(pool, job_id, "running", None).await?;
    let start_time = std::time::Instant::now();

    let result = async {
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
            return Ok((0, 0));
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
            return Ok((0, 0));
        }
        
        // Convert HTML to Markdown for cleaner LLM input
        let markdown_content = html2text::from_read(final_content.as_bytes(), 80)
            .context("Failed to convert HTML to Markdown")?;
        
        // Summarize
        let summary = summarizer::summarize_article(summarization_provider.as_ref(), &markdown_content, 500).await;
        
        // Classify
        let categories = classify_article(
            summarization_provider.as_ref(),
            &summary.headline,
            &summary.bullets
        ).await.unwrap_or_default();
        
        let bullets_json = serde_json::to_string(&summary.bullets)?;
        let categories_json = serde_json::to_string(&categories)?;

        // Store summary
        sqlx::query(
            "INSERT OR REPLACE INTO article_summaries \
             (article_id, headline, bullets_json, details, model, categories, \
              prompt_tokens, completion_tokens) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(article_id)
        .bind(&summary.headline)
        .bind(&bullets_json)
        .bind(&summary.details)
        .bind(model)
        .bind(&categories_json)
        .bind(summary.usage.prompt_tokens as i32)
        .bind(summary.usage.completion_tokens as i32)
        .execute(pool)
        .await?;
        
        // Mark article as processed
        sqlx::query(
            "UPDATE articles SET processing_status = 'completed', processed_at = ? WHERE id = ?"
        )
        .bind(chrono::Utc::now())
        .bind(article_id)
        .execute(pool)
        .await?;
        
        // 4. Personalize for all active users (Phase 8: NEW!)
        if let Some(personalization_llm) = personalization_provider {
            info!("Starting personalization for article {} for active users", article_id);
            match crate::personalize_worker::personalize_for_users(
                pool,
                article_id,
                &summary,
                personalization_llm,
                model,
            )
            .await
            {
                Ok(count) => {
                    info!(
                        "Successfully personalized article {} for {} users",
                        article_id, count
                    );
                }
                Err(e) => {
                    // Don't fail the whole job if personalization fails
                    warn!(
                        "Failed to personalize article {} for users: {}",
                        article_id, e
                    );
                }
            }
        }
        
        Ok::<_, anyhow::Error>((summary.usage.prompt_tokens, summary.usage.completion_tokens))
    }.await;

    let processing_time = start_time.elapsed().as_millis() as i64;

    match result {
        Ok((prompt_tokens, completion_tokens)) => {
            complete_processing_job(pool, job_id, prompt_tokens, completion_tokens, processing_time).await?;
        }
        Err(e) => {
            update_job_status(pool, job_id, "failed", Some(&e.to_string())).await?;
            return Err(e);
        }
    }

    Ok(())
}


/// Process all pending articles (those with processing_status = 'pending')
pub async fn process_pending_articles(
    pool: &SqlitePool,
    summarization_provider: Arc<dyn LlmProvider>,
    personalization_provider: Option<Arc<dyn LlmProvider>>,
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
    batch_process_articles(pool, &article_ids, summarization_provider, personalization_provider, model).await
}

/// Convert Vec<f32> to Vec<u8> (Little Endian bytes) for BLOB storage
fn f32_vec_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Process articles missing embeddings
pub async fn process_missing_embeddings(
    pool: &SqlitePool,
    provider: Arc<dyn LlmProvider>,
    _model: &str,
    limit: usize,
) -> Result<usize> {
    // 1. Find articles needing embeddings
    let rows = sqlx::query(
        r#"
        SELECT 
            a.id, 
            a.title, 
            s.headline, 
            s.bullets_json, 
            a.content
        FROM articles a
        LEFT JOIN article_summaries s ON a.id = s.article_id
        LEFT JOIN vec_articles v ON a.id = v.article_id
        WHERE v.article_id IS NULL
        ORDER BY a.first_seen_at DESC
        LIMIT ?
        "#
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("Failed to fetch articles for embedding")?;

    if rows.is_empty() {
        return Ok(0);
    }

    info!("Found {} articles missing embeddings", rows.len());
    let mut count = 0;

    for article in rows {
        // Construct text to embed: Title + Summary (or truncated content)
        let article_id: i64 = article.get("id");
        let title: String = article.get("title");
        let headline: Option<String> = article.get("headline");
        let bullets_json: Option<String> = article.get("bullets_json");
        let content: String = article.get("content");
        
        let mut summary_text = String::new();
        let has_summary = headline.is_some() && bullets_json.is_some();
        
        if has_summary {
             let h = headline.unwrap();
             let b_json = bullets_json.unwrap();
             if let Ok(bullets) = serde_json::from_str::<Vec<String>>(&b_json) {
                 summary_text = format!("{}\n{}", h, bullets.join(" "));
             }
        }
        
        if summary_text.is_empty() {
             // Fallback to first 500 chars of content
             summary_text = content.chars().take(500).collect();
        }

        let text_to_embed = format!("{}\n{}", title, summary_text);
        
        // Call LLM Embed
        match provider.embed(&text_to_embed).await {
            Ok(embedding) => {
                let bytes = f32_vec_to_bytes(&embedding);
                
                sqlx::query(
                    "INSERT INTO vec_articles (article_id, embedding) VALUES (?, ?)"
                )
                .bind(article_id)
                .bind(bytes)
                .execute(pool)
                .await?;
                
                count += 1;
            }
            Err(e) => {
                error!("Failed to embed article {}: {}", article_id, e);
                // Continue with next
            }
        }
    }
    
    Ok(count)
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
