use mynewslens::sessions::{create_session, get_messages, get_session, list_sessions, store_message};
use sqlx::sqlite::SqlitePoolOptions;

async fn setup_test_db() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("Failed to create test pool");

    // Disable foreign key constraints for easier testing
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .unwrap();

    // Create minimal schema
    sqlx::query(
        r#"
        CREATE TABLE users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            display_name TEXT
        );
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        CREATE TABLE sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            start_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            duration_requested_seconds INTEGER,
            digest_summary_id INTEGER
        );
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        CREATE TABLE chat_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id INTEGER NOT NULL,
            author TEXT NOT NULL,
            message TEXT,
            created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    pool
}

#[tokio::test]
async fn test_session_crud() {
    let pool = setup_test_db().await;

    // Insert test user
    sqlx::query("INSERT INTO users (username, display_name) VALUES (?, ?)")
        .bind("test_user")
        .bind("Test User")
        .execute(&pool)
        .await
        .unwrap();

    let user_id = 1;

    // Test 1: Create session
    let session = create_session(&pool, user_id, Some(1200))
        .await
        .expect("Failed to create session");

    assert_eq!(session.user_id, user_id);
    assert_eq!(session.duration_requested_seconds, Some(1200));

    // Test 2: Get session
    let retrieved = get_session(&pool, session.id)
        .await
        .expect("Failed to get session");

    assert_eq!(retrieved.id, session.id);
    assert_eq!(retrieved.user_id, user_id);

    // Test 3: List sessions
    let sessions = list_sessions(&pool, user_id)
        .await
        .expect("Failed to list sessions");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, session.id);

    // Test 4: Store messages
    let msg1 = store_message(&pool, session.id, "user", "Hello!")
        .await
        .expect("Failed to store user message");

    assert_eq!(msg1.author, "user");
    assert_eq!(msg1.message, "Hello!");

    let msg2 = store_message(&pool, session.id, "assistant", "Hi there!")
        .await
        .expect("Failed to store assistant message");

    assert_eq!(msg2.author, "assistant");

    // Test 5: Get messages
    let messages = get_messages(&pool, session.id)
        .await
        .expect("Failed to get messages");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].author, "user");
    assert_eq!(messages[1].author, "assistant");
}

#[tokio::test]
async fn test_multiple_sessions() {
    let pool = setup_test_db().await;

    // Insert test users
    sqlx::query("INSERT INTO users (id, username, display_name) VALUES (1, 'user1', 'User 1'), (2, 'user2', 'User 2')")
        .execute(&pool)
        .await
        .unwrap();

    // Create sessions for different users
    create_session(&pool, 1, Some(600))
        .await
        .expect("Failed to create session for user 1");
    create_session(&pool, 1, Some(900))
        .await
        .expect("Failed to create second session for user 1");
    create_session(&pool, 2, Some(1200))
        .await
        .expect("Failed to create session for user 2");

    // List sessions per user
    let user1_sessions = list_sessions(&pool, 1)
        .await
        .expect("Failed to list user 1 sessions");
    let user2_sessions = list_sessions(&pool, 2)
        .await
        .expect("Failed to list user 2 sessions");

    assert_eq!(user1_sessions.len(), 2);
    assert_eq!(user2_sessions.len(), 1);
}

