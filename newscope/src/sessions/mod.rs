use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Session represents a user's reading session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: i64,
    pub user_id: i64,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub start_at: DateTime<Utc>,
    pub duration_requested_seconds: Option<i32>,
    pub digest_summary_id: Option<i64>,
    pub title: Option<String>,
}

/// ChatMessage represents a single message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: i64,
    pub session_id: i64,
    pub author: String, // "user" or "assistant"
    pub message: String,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created_at: DateTime<Utc>,
}

/// Create a new session
pub async fn create_session(
    pool: &SqlitePool,
    user_id: i64,
    duration_seconds: Option<i32>,
) -> Result<Session> {
    // Create session
    let result = sqlx::query(
        r#"
        INSERT INTO sessions (user_id, duration_requested_seconds)
        VALUES (?, ?)
        "#,
    )
    .bind(user_id)
    .bind(duration_seconds)
    .execute(pool)
    .await
    .context("Failed to insert session")?;

    let session_id = result.last_insert_rowid();

    // Fetch the created session
    get_session(pool, session_id).await
}

/// Get a single session by ID
pub async fn get_session(pool: &SqlitePool, session_id: i64) -> Result<Session> {
    let session = sqlx::query_as::<_, SessionRow>(
        r#"
        SELECT id, user_id, start_at, duration_requested_seconds, digest_summary_id, title
        FROM sessions
        WHERE id = ?
        "#,
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
    .context("Failed to fetch session")?;

    Ok(Session {
        id: session.id,
        user_id: session.user_id,
        start_at: DateTime::parse_from_rfc3339(&session.start_at)
            .context("Failed to parse start_at")?
            .with_timezone(&Utc),
        duration_requested_seconds: session.duration_requested_seconds,
        digest_summary_id: session.digest_summary_id,
        title: session.title,
    })
}

/// List all sessions for a user
pub async fn list_sessions(pool: &SqlitePool, user_id: i64) -> Result<Vec<Session>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        r#"
        SELECT id, user_id, start_at, duration_requested_seconds, digest_summary_id, title
        FROM sessions
        WHERE user_id = ?
        ORDER BY start_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("Failed to list sessions")?;

    rows.into_iter()
        .map(|row| {
            Ok(Session {
                id: row.id,
                user_id: row.user_id,
                start_at: DateTime::parse_from_rfc3339(&row.start_at)
                    .context("Failed to parse start_at")?
                    .with_timezone(&Utc),
                duration_requested_seconds: row.duration_requested_seconds,
                digest_summary_id: row.digest_summary_id,
                title: row.title,
            })
        })
        .collect()
}

/// Update session title
pub async fn update_session_title(
    pool: &SqlitePool,
    session_id: i64,
    title: &str,
) -> Result<()> {
    sqlx::query("UPDATE sessions SET title = ? WHERE id = ?")
        .bind(title)
        .bind(session_id)
        .execute(pool)
        .await
        .context("Failed to update session title")?;
    Ok(())
}

/// Get session with full chat message history
pub async fn get_session_with_messages(
    pool: &SqlitePool,
    session_id: i64,
) -> Result<(Session, Vec<ChatMessage>)> {
    let session = get_session(pool, session_id).await?;
    let messages = get_messages(pool, session_id).await?;
    Ok((session, messages))
}

/// Get all messages for a session
pub async fn get_messages(pool: &SqlitePool, session_id: i64) -> Result<Vec<ChatMessage>> {
    let rows = sqlx::query_as::<_, ChatMessageRow>(
        r#"
        SELECT id, session_id, author, message, created_at
        FROM chat_messages
        WHERE session_id = ?
        ORDER BY created_at ASC
        "#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .context("Failed to fetch messages")?;

    rows.into_iter()
        .map(|row| {
            Ok(ChatMessage {
                id: row.id,
                session_id: row.session_id,
                author: row.author,
                message: row.message,
                created_at: DateTime::parse_from_rfc3339(&row.created_at)
                    .context("Failed to parse created_at")?
                    .with_timezone(&Utc),
            })
        })
        .collect()
}

/// Store a chat message
pub async fn store_message(
    pool: &SqlitePool,
    session_id: i64,
    author: &str,
    message: &str,
) -> Result<ChatMessage> {
    let result = sqlx::query(
        r#"
        INSERT INTO chat_messages (session_id, author, message)
        VALUES (?, ?, ?)
        "#,
    )
    .bind(session_id)
    .bind(author)
    .bind(message)
    .execute(pool)
    .await
    .context("Failed to insert message")?;

    let message_id = result.last_insert_rowid();

    // Fetch the created message
    let row = sqlx::query_as::<_, ChatMessageRow>(
        r#"
        SELECT id, session_id, author, message, created_at
        FROM chat_messages
        WHERE id = ?
        "#,
    )
    .bind(message_id)
    .fetch_one(pool)
    .await
    .context("Failed to fetch inserted message")?;

    Ok(ChatMessage {
        id: row.id,
        session_id: row.session_id,
        author: row.author,
        message: row.message,
        created_at: DateTime::parse_from_rfc3339(&row.created_at)
            .context("Failed to parse created_at")?
            .with_timezone(&Utc),
    })
}

// Internal row types for SQLx mapping
#[derive(sqlx::FromRow)]
struct SessionRow {
    id: i64,
    user_id: i64,
    start_at: String,
    duration_requested_seconds: Option<i32>,
    digest_summary_id: Option<i64>,
    title: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ChatMessageRow {
    id: i64,
    session_id: i64,
    author: String,
    message: String,
    created_at: String,
}

pub mod websocket;
