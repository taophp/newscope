#[path = "../ingestion.rs"]
mod ingestion;

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Test feeds
    let feeds = vec![
        "http://rss.cnn.com/rss/edition.rss",
        "https://hnrss.org/newest?points=100",
        "https://feeds.arstechnica.com/arstechnica/index",
    ];

    for url in feeds {
        println!("\n{}", "=".repeat(60));
        println!("Testing: {}", url);
        println!("{}", "=".repeat(60));
        
        match ingestion::fetch_and_parse_feed(url, 10).await {
            Ok(feed) => {
                println!("✓ Success!");
                println!("  Title: {:?}", feed.title.as_ref().map(|t| &t.content));
                println!("  Entries: {}", feed.entries.len());
                
                if !feed.entries.is_empty() {
                    println!("\n  First 3 entries:");
                    for (i, entry) in feed.entries.iter().take(3).enumerate() {
                        println!("    {}. {:?}", i+1, entry.title.as_ref().map(|t| &t.content));
                        println!("       URL: {}", entry.links.first().map(|l| l.href.as_str()).unwrap_or("none"));
                        let content_len = entry.content.as_ref()
                            .and_then(|c| c.body.as_ref())
                            .map(|b| b.len())
                            .unwrap_or(0);
                        let summary_len = entry.summary.as_ref()
                            .map(|s| s.content.len())
                            .unwrap_or(0);
                        println!("       Content: {} chars, Summary: {} chars", content_len, summary_len);
                    }
                }
            }
            Err(e) => {
                println!("✗ Failed: {}", e);
            }
        }
    }
}
