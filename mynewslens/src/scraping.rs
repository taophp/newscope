use anyhow::{Context, Result};
use reqwest::Client;
use scraper::{Html, Selector};
use std::time::Duration;
use tracing::{info, warn};

/// Scrapes the content of an article from the given URL.
/// Returns the extracted text content.
pub async fn scrape_article_content(url: &str, timeout_secs: u64) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent("MyNewsLens/0.1.0")
        .build()
        .context("failed to build reqwest client")?;

    let response = client.get(url).send().await.context("failed to fetch article page")?;

    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("article fetch failed with status: {}", status));
    }

    let html_content = response.text().await.context("failed to read response body")?;
    let document = Html::parse_document(&html_content);

    // Heuristic: try to find the main content
    // 1. <article> tag
    // 2. <main> tag
    // 3. Fallback to body (maybe too noisy?)
    
    let selectors = ["article", "main", ".post-content", ".entry-content", "#content"];
    
    for selector_str in selectors {
        if let Ok(selector) = Selector::parse(selector_str) {
            if let Some(element) = document.select(&selector).next() {
                // Found a matching element, extract HTML and convert to Markdown
                let html = element.html();
                let markdown = html2text::from_read(html.as_bytes(), 80);
                
                // html2text::from_read returns a Result, handle it
                if let Ok(markdown) = markdown {
                    if !markdown.is_empty() {
                        info!("scraping: found content using selector '{}', converted to {} chars markdown", 
                              selector_str, markdown.len());
                        return Ok(markdown);
                    }
                }
            }
        }
    }

    // Fallback: just get all paragraphs
    if let Ok(p_selector) = Selector::parse("p") {
        let mut full_html = String::new();
        for element in document.select(&p_selector) {
            full_html.push_str(&element.html());
            full_html.push_str("\n");
        }
            
        if !full_html.is_empty() {
             let markdown = html2text::from_read(full_html.as_bytes(), 80);
             if let Ok(markdown) = markdown {
                 info!("scraping: fallback to all <p> tags, converted to {} chars markdown", markdown.len());
                 return Ok(markdown);
             }
        }
    }

    warn!("scraping: could not extract content for {}", url);
    Ok(String::new())
}
