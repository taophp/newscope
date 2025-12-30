use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;
use std::time::Duration;

/// Integration test that ensures only articles coming from feeds the user is subscribed to
/// are selected by the same SQL used by the websocket session logic.
///
/// The test creates a minimal in-memory SQLite schema, inserts:
/// - two feeds (A and B)
/// - a single user subscribed only to feed A
/// - two articles (one occurring in feed A, one in feed B)
/// - personalized summaries for both articles for the user (both marked relevant)
///
/// Then it runs the selection query used in the websocket code and asserts that only the
/// article coming from the subscribed feed A is returned.
#[tokio::test]
async fn test_subscription_filter_only_subscribed_feeds() {
    // Create in-memory SQLite pool
    let pool = SqlitePoolOptions::new()
        .connect_timeout(Duration::from_secs(5))
        .connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory sqlite pool");

    // For deterministic tests, disable foreign keys (simpler table creation without ordering)
    let _ = sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await;

    // Create minimal schema needed for the test
    // users, feeds, subscriptions, articles, article_occurrences, user_article_summaries, user_article_views
    sqlx::query(
        r#"
        CREATE TABLE users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL UNIQUE
        );
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        CREATE TABLE feeds (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT NOT NULL,
            title TEXT
        );
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        CREATE TABLE subscriptions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            feed_id INTEGER NOT NULL,
            title TEXT,
            weight INTEGER DEFAULT 0
        );
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        CREATE TABLE articles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            canonical_url TEXT NOT NULL,
            first_seen_at TEXT
        );
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        CREATE TABLE article_occurrences (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            article_id INTEGER NOT NULL,
            feed_id INTEGER NOT NULL
        );
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        CREATE TABLE user_article_summaries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            article_id INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            personalized_headline TEXT,
            personalized_bullets TEXT,
            personalized_details TEXT,
            relevance_score REAL,
            is_relevant INTEGER DEFAULT 0
        );
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        CREATE TABLE user_article_views (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            article_id INTEGER NOT NULL,
            session_id INTEGER,
            viewed_at TEXT,
            rating INTEGER
        );
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    // Insert a user
    sqlx::query("INSERT INTO users (username) VALUES (?)")
        .bind("alice")
        .execute(&pool)
        .await
        .unwrap();

    // Insert two feeds: A (id=1) and B (id=2)
    sqlx::query("INSERT INTO feeds (url, title) VALUES (?, ?)")
        .bind("http://feed-a.example/rss")
        .bind("Feed A")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO feeds (url, title) VALUES (?, ?)")
        .bind("http://feed-b.example/rss")
        .bind("Feed B")
        .execute(&pool)
        .await
        .unwrap();

    // Subscribe user (id = 1) only to feed A (feed_id = 1)
    sqlx::query("INSERT INTO subscriptions (user_id, feed_id, title) VALUES (?, ?, ?)")
        .bind(1_i64)
        .bind(1_i64)
        .bind("Alice's subscription to A")
        .execute(&pool)
        .await
        .unwrap();

    // Insert two articles: article 1 belongs to feed A, article 2 belongs to feed B
    sqlx::query("INSERT INTO articles (canonical_url, first_seen_at) VALUES (?, ?)")
        .bind("http://article-a.example/1")
        .bind("2025-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO articles (canonical_url, first_seen_at) VALUES (?, ?)")
        .bind("http://article-b.example/1")
        .bind("2025-01-02T00:00:00Z")
        .execute(&pool)
        .await
        .unwrap();

    // Link occurrences
    sqlx::query("INSERT INTO article_occurrences (article_id, feed_id) VALUES (?, ?)")
        .bind(1_i64) // article 1 -> feed 1 (A)
        .bind(1_i64)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO article_occurrences (article_id, feed_id) VALUES (?, ?)")
        .bind(2_i64) // article 2 -> feed 2 (B)
        .bind(2_i64)
        .execute(&pool)
        .await
        .unwrap();

    // Insert user_article_summaries for both articles for user 1 (both relevant)
    sqlx::query(
        r#"
        INSERT INTO user_article_summaries
            (article_id, user_id, personalized_headline, personalized_bullets, personalized_details, relevance_score, is_relevant)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(1_i64)
    .bind(1_i64)
    .bind("Headline A")
    .bind("[\"bullet1\"]")
    .bind(Some("Details A"))
    .bind(0.95_f64)
    .bind(1_i64)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        INSERT INTO user_article_summaries
            (article_id, user_id, personalized_headline, personalized_bullets, personalized_details, relevance_score, is_relevant)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(2_i64)
    .bind(1_i64)
    .bind("Headline B")
    .bind("[\"bulletB\"]")
    .bind(Some("Details B"))
    .bind(0.85_f64)
    .bind(1_i64)
    .execute(&pool)
    .await
    .unwrap();

    // No entries in user_article_views (uav IS NULL)

    // Now run the same selection query used in websocket.rs but bound to user_id = 1 and session_id = 999
    let estimated_limit = 10_i64;
    let rows = sqlx::query(
        r#"
        SELECT
            uas.article_id,
            uas.personalized_headline,
            uas.personalized_bullets,
            uas.personalized_details,
            uas.relevance_score,
            a.canonical_url,
            f.title as feed_title
         FROM user_article_summaries uas
         JOIN articles a ON uas.article_id = a.id
         JOIN article_occurrences ao ON a.id = ao.article_id
         JOIN subscriptions s ON s.feed_id = ao.feed_id AND s.user_id = ?
         JOIN feeds f ON ao.feed_id = f.id
         LEFT JOIN user_article_views uav ON uas.user_id = uav.user_id AND uas.article_id = uav.article_id AND uav.session_id = ?
         WHERE uas.user_id = ?
           AND uas.is_relevant = 1
           AND uav.id IS NULL
         GROUP BY uas.article_id
         ORDER BY uas.relevance_score DESC, a.first_seen_at DESC
         LIMIT ?
         "#,
    )
    .bind(1_i64) // s.user_id
    .bind(999_i64) // uav.session_id
    .bind(1_i64) // uas.user_id
    .bind(estimated_limit)
    .fetch_all(&pool)
    .await
    .expect("Query failed");

    // We expect only one row (the article from Feed A)
    assert_eq!(
        rows.len(),
        1,
        "Expected exactly one article from subscribed feed"
    );

    let row = &rows[0];
    let article_id: i64 = row.get("article_id");
    let feed_title: String = row.get("feed_title");

    assert_eq!(
        article_id, 1,
        "Returned article should be article 1 (from feed A)"
    );
    assert_eq!(
        feed_title, "Feed A",
        "Returned article should come from Feed A"
    );
}
