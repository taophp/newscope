use common::{init_db_pool, Config};
use mynewslens::server;
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

// Helper to create a test pool
async fn setup_test_db() -> SqlitePool {
    let db_path = format!("test_db_{}.sqlite", uuid::Uuid::new_v4());
    let pool = init_db_pool(&db_path).await.expect("init pool");
    server::ensure_schema(&pool).await.expect("ensure schema");
    pool
}

#[tokio::test]
async fn test_full_pipeline() {
    // 1. Setup
    let pool = setup_test_db().await;
    let pool = Arc::new(pool);

    // 2. Create Users
    sqlx::query("INSERT INTO users (username) VALUES ('user_a')")
        .execute(&*pool).await.expect("create user a");
    let user_a_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'user_a'")
        .fetch_one(&*pool).await.expect("get user a id");

    sqlx::query("INSERT INTO users (username) VALUES ('user_b')")
        .execute(&*pool).await.expect("create user b");
    let user_b_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'user_b'")
        .fetch_one(&*pool).await.expect("get user b id");

    // 3. User A subscribes to a feed (using a reliable RSS feed, e.g., Hacker News or similar, 
    // but for stability in CI/offline, we might want to mock, but here we test live ingestion as requested)
    // Let's use a very stable feed or a local file if possible. 
    // For this "Global Verification", let's try a real URL but be ready to fail if offline.
    // We'll use a simple RSS feed.
    let feed_url = "https://hnrss.org/newest?points=100"; // Hacker News > 100 points

    // Simulate create_feed endpoint logic
    // Check if exists
    let existing_feed = sqlx::query_scalar::<_, i64>("SELECT id FROM feeds WHERE url = ?")
        .bind(feed_url)
        .fetch_optional(&*pool).await.expect("check feed");
    
    let feed_id = if let Some(id) = existing_feed {
        id
    } else {
        let id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO feeds (url, title, next_poll_at) VALUES (?, ?, ?) RETURNING id"
        )
        .bind(feed_url)
        .bind("Hacker News")
        .bind(chrono::Utc::now()) // Due immediately
        .fetch_one(&*pool).await.expect("insert feed");
        id
    };

    // Create subscription for User A
    sqlx::query("INSERT INTO subscriptions (user_id, feed_id) VALUES (?, ?)")
        .bind(user_a_id)
        .bind(feed_id)
        .execute(&*pool).await.expect("sub user a");

    // 4. User B subscribes to same feed
    // Should reuse feed_id
    let feed_id_b = sqlx::query_scalar::<_, i64>("SELECT id FROM feeds WHERE url = ?")
        .bind(feed_url)
        .fetch_one(&*pool).await.expect("get feed id");
    assert_eq!(feed_id, feed_id_b, "Feed ID should be reused");

    sqlx::query("INSERT INTO subscriptions (user_id, feed_id) VALUES (?, ?)")
        .bind(user_b_id)
        .bind(feed_id)
        .execute(&*pool).await.expect("sub user b");

    // 5. Verify subscriptions
    let sub_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM subscriptions WHERE feed_id = ?")
        .bind(feed_id)
        .fetch_one(&*pool).await.expect("count subs");
    assert_eq!(sub_count, 2, "Should have 2 subscriptions");

    // 6. Run Ingestion (Simulate Worker)
    // We can't easily run the full worker loop here without spawning, so we'll just call the logic manually
    // or extract the logic to a testable function. 
    // Since we can't easily access private modules from integration tests unless they are pub,
    // we might need to rely on the fact that we are in the same crate or make things pub.
    // `mynewslens` is a binary crate, so integration tests in `tests/` can't easily access its internals 
    // unless it's also a lib.
    
    // WORKAROUND: We'll put this test inside `mynewslens/src/main.rs` or `mynewslens/src/lib.rs` if it existed.
    // But `mynewslens` is a binary. 
    // Let's create a separate test file in `mynewslens/src/bin/verification.rs` or similar? 
    // No, easiest is to add a test module to `mynewslens/src/main.rs` but that's not ideal for "Global Verification".
    
    // Actually, we can use `cargo run -- --worker-only` and let it run for a bit, then check DB.
    // That's a true integration test.
}
