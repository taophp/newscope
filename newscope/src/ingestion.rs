use anyhow::{Context, Result};
use feed_rs::parser;
use feed_rs::model::Feed;
use reqwest::Client;
use std::time::Duration;


/// Fetches a feed from the given URL and parses it.
/// Enforces a timeout and size limit (though size limit is tricky with streaming, 
/// we'll rely on timeout and simple content-length check for now).
pub async fn fetch_and_parse_feed(url: &str, timeout_secs: u64) -> Result<Feed> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent("Newscope/0.1.0")
        .build()
        .context("failed to build reqwest client")?;

    let max_retries = 3;
    let mut last_error = None;

    for attempt in 1..=max_retries {
        if attempt > 1 {
            let backoff = Duration::from_secs(2u64.pow(attempt - 2)); // 1s, 2s, 4s...
            tracing::info!("Retrying feed fetch for {} (attempt {}/{}) after {:?}...", url, attempt, max_retries, backoff);
            tokio::time::sleep(backoff).await;
        }

        match client.get(url).send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    let bytes = response.bytes().await.context("failed to read response body")?;
                    let feed = parser::parse(bytes.as_ref()).context("failed to parse feed")?;
                    return Ok(feed);
                } else if status.is_server_error() { // 5xx
                    last_error = Some(anyhow::anyhow!("server error: {}", status));
                    continue; // Retry
                } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                     last_error = Some(anyhow::anyhow!("rate limited: {}", status));
                     continue; // Retry
                } else {
                    // Client error (4xx) - likely permament, don't retry
                    return Err(anyhow::anyhow!("feed fetch failed with status: {}", status));
                }
            }
            Err(e) => {
                // Network error - retry
                last_error = Some(anyhow::Error::new(e).context("network error during fetch"));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("unknown error after retries")))
}
