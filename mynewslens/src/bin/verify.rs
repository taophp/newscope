/*
 * End-to-end verification binary for MyNewsLens MVP
 * 
 * Replaces verify.py with native Rust implementation
 * 
 * Steps:
 * 1. Clean/create verification database
 * 2. Bootstrap schema
 * 3. Create test user
 * 4. Add test feed (CNN RSS)
 * 5. Wait for worker to poll and ingest articles
 * 6. Verify articles in database
 * 7. Check summaries (if LLM configured)
 */

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;

const VERIFICATION_DB: &str = "data/verification.db";
const SERVER_URL: &str = "http://localhost:8000";
const TEST_FEED_URL: &str = "http://rss.cnn.com/rss/edition.rss";

#[tokio::main]
async fn main() -> Result<()> {
    println!("\n{}", "=".repeat(60));
    println!("MyNewsLens End-to-End Verification");
    println!("{}\n", "=".repeat(60));

    // Step 1: Clean database
    println!("[1/7] Cleaning verification database...");
    if Path::new(VERIFICATION_DB).exists() {
        std::fs::remove_file(VERIFICATION_DB)?;
    }
    std::fs::create_dir_all("data")?;

    // Step 2: Create database pool and schema
    println!("[2/7] Creating database and schema...");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&format!("sqlite://{}", VERIFICATION_DB))
        .await
        .context("Failed to connect to verification database")?;

    // Bootstrap schema (using common::init_db_pool would be ideal but we're in a binary)
    // For now, we'll use HTTP API which triggers schema creation
    
    // Step 3: Create test user via API
    println!("[3/7] Creating test user...");
    let client = Client::new();
    
    let register_response = client
        .post(format!("{}/api/v1/register", SERVER_URL))
        .json(&json!({
            "username": "test_user",
            "display_name": "Test User",
            "password": "test123"
        }))
        .send()
        .await
        .context("Failed to register user")?;

    if !register_response.status().is_success() {
        anyhow::bail!("User registration failed: {}", register_response.status());
    }

    let user_data: serde_json::Value = register_response.json().await?;
    let user_id = user_data["user_id"].as_i64().context("No user_id in response")?;
    println!("   ✓ User created with ID: {}", user_id);

    // Step 4: Add test feed
    println!("[4/7] Adding test feed ({})...", TEST_FEED_URL);
    let feed_response = client
        .post(format!("{}/api/v1/feeds", SERVER_URL))
        .json(&json!({
            "user_id": user_id,
            "url": TEST_FEED_URL,
            "title": "CNN RSS (Test)"
        }))
        .send()
        .await
        .context("Failed to create feed")?;

    if !feed_response.status().is_success() {
        anyhow::bail!("Feed creation failed: {}", feed_response.status());
    }

    let feed_data: serde_json::Value = feed_response.json().await?;
    let feed_id = feed_data["id"].as_i64().context("No feed id in response")?;
    println!("   ✓ Feed created with ID: {}", feed_id);

    // Step 5: Wait for worker to poll
    println!("[5/7] Waiting for worker to poll feed (max 30s)...");
    let mut attempts = 0;
    let max_attempts = 15; // 15 * 2s = 30s
    
    loop {
        sleep(Duration::from_secs(2)).await;
        attempts += 1;

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM articles")
            .fetch_one(&pool)
            .await?;

        println!("   Attempt {}/{}: {} articles found", attempts, max_attempts, count.0);

        if count.0 > 0 {
            println!("   ✓ Articles ingested!");
            break;
        }

        if attempts >= max_attempts {
            anyhow::bail!("No articles ingested after {}s", attempts * 2);
        }
    }

    // Step 6: Verify articles
    println!("[6/7] Verifying article data...");
    let articles: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, title, canonical_url FROM articles LIMIT 5"
    )
    .fetch_all(&pool)
    .await?;

    for (id, title, url) in &articles {
        println!("   Article {}: {} ({})", id, title.chars().take(50).collect::<String>(), url);
    }
    println!("   ✓ {} articles verified", articles.len());

    // Step 7: Check LLM summaries (if configured)
    println!("[7/7] Checking LLM summaries...");
    let summary_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM llm_usage_log")
        .fetch_one(&pool)
        .await?;

    if summary_count.0 > 0 {
        println!("   ✓ LLM usage logged: {} operations", summary_count.0);
    } else {
        println!("   ⚠ No LLM usage found (LLM may not be configured)");
    }

    println!("\n{}", "=".repeat(60));
    println!("✅ VERIFICATION COMPLETE");
    println!("{}\n", "=".repeat(60));

    Ok(())
}
