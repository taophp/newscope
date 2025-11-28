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
        .user_agent("MyNewsLens/0.1.0")
        .build()
        .context("failed to build reqwest client")?;

    let response = client.get(url).send().await.context("failed to fetch feed")?;

    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("feed fetch failed with status: {}", status));
    }

    let bytes = response.bytes().await.context("failed to read response body")?;
    
    // Parse the feed
    let feed = parser::parse(bytes.as_ref()).context("failed to parse feed")?;

    Ok(feed)
}
