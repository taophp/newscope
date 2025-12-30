use anyhow::{Context, Result};
use reqwest::Client;
use std::time::Duration;
use tracing::{info, warn};
use std::io::Cursor;

/// Scrapes the content of an article from the given URL.
/// Returns the extracted text content.
pub async fn scrape_article_content(url: &str, timeout_secs: u64) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent("Newscope/0.1.0")
        .build()
        .context("failed to build reqwest client")?;

    let response = client.get(url).send().await.context("failed to fetch article page")?;

    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("article fetch failed with status: {}", status));
    }

    // Readability requires a Reader, so we fetch bytes
    let bytes = response.bytes().await.context("failed to read response body")?;
    let mut reader = Cursor::new(bytes);

    // Use readability to extract the main content
    // We construct a Url object for readability to resolve relative links
    let url_obj = url::Url::parse(url).context("failed to parse article URL")?;

    match readability::extractor::extract(&mut reader, &url_obj) {
        Ok(product) => {
            // product.content contains the HTML of the main article content
            let html = product.content;
            
            // Convert HTML to Markdown for cleaner LLM input
            // We use a width of 80 for wrapping
            match html2text::from_read(html.as_bytes(), 80) {
                Ok(markdown) => {
                    info!("scraping: readability extracted {} chars markdown from {}", markdown.len(), url);
                    Ok(markdown)
                },
                Err(e) => {
                    warn!("scraping: failed to convert extracted HTML to markdown: {}", e);
                    // Fallback: return the HTML title + text content if markdown conversion fails
                    // readability also provides .text, but it might be less structured than markdown
                    Ok(product.text)
                }
            }
        },
        Err(e) => {
            warn!("scraping: readability failed for {}: {}", url, e);
            // Return empty string as per previous behavior on failure, or could error out
            Ok(String::new())
        }
    }
}
