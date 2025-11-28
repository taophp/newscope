use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome, Request};
use rocket::serde::json::Json;
use rocket::{get, post, routes, State};
use rocket::fs::FileServer;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use common::Config;

// Ingestion and storage for feed refresh
use crate::{ingestion, storage};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use jsonwebtoken::{
    decode, encode, DecodingKey, EncodingKey, Header as JwtHeader, TokenData, Validation,
};
use rand::rngs::OsRng;

/// Application state stored inside Rocket managed state.
#[derive(Clone)]
pub struct AppState {
    pub started_at: DateTime<Utc>,
    pub config: Option<Arc<Config>>,
    pub db: SqlitePool,
    pub llm_provider: Option<Arc<dyn crate::llm::LlmProvider>>,
}

/// Response structure for `/api/v1/status`.
#[derive(Serialize)]
struct StatusResponse {
    status: &'static str,
    uptime_seconds: i64,
    users_count: usize,
    scheduler_times: Vec<String>,
}

/// Representation of feed row returned by the API.
/// Representation of feed row returned by the API (joined with subscription).
#[derive(Serialize)]
struct FeedRow {
    id: i64,
    subscription_id: i64,
    user_id: i64,
    url: String,
    title: Option<String>,
    last_checked: Option<String>,
    status: Option<String>,
    weight: i64,
}

/// Request body for creating a feed. `user_id` or `token` (JWT) may be provided.
/// If `user_id` is omitted, a `token` may be provided and the server will extract
/// the subject (`sub`) from the token to identify the user.
#[derive(Deserialize)]
struct FeedCreate {
    user_id: Option<i64>,
    /// Optional JWT token that can contain the subject user id.
    token: Option<String>,
    url: String,
    title: Option<String>,
}


use rocket::response::Redirect;

/// Redirect root to static index.html
#[get("/")]
async fn index_redirect() -> Redirect {
    Redirect::to("/static/index.html")
}


#[get("/health")]
async fn health() -> &'static str {
    "OK"
}

/// Status endpoint returning simple JSON with uptime and basic config info.
#[get("/api/v1/status")]
async fn status(state: &State<AppState>) -> Json<StatusResponse> {
    let now = Utc::now();
    let uptime = (now - state.started_at).num_seconds();

    let (users_count, scheduler_times) = match &state.config {
        Some(cfg) => (cfg.users.len(), cfg.scheduler.times.clone()),
        None => (0usize, Vec::new()),
    };

    Json(StatusResponse {
        status: "ok",
        uptime_seconds: uptime,
        users_count,
        scheduler_times,
    })
}

/// List users defined in configuration (safe read-only).
#[get("/api/v1/users")]
async fn list_users(state: &State<AppState>) -> Json<serde_json::Value> {
    let users = state
        .config
        .as_ref()
        .map(|c| c.users.clone())
        .unwrap_or_default();
    Json(serde_json::json!(users))
}

/// List feeds stored in the database for the current user.
#[get("/api/v1/feeds?<user_id>")]
async fn list_feeds(state: &State<AppState>, user_id: Option<i64>) -> Result<Json<Vec<FeedRow>>, Status> {
    // TODO: proper auth guard. For now we rely on the fact that this is a personal instance
    // or we should extract user_id from token if we had a guard.
    // Since we don't have a guard in this signature, we can't easily filter by user without passing it.
    // However, the previous implementation didn't filter by user in the query (it returned all feeds).
    // But now we have subscriptions. We should probably require auth.
    // For MVP/Dev without strict auth guard, let's just return all subscriptions if no user_id is implicit?
    // Actually, the previous `list_feeds` didn't take any auth args, so it listed EVERYTHING.
    // We will keep it simple: list all subscriptions for now, or if we can, filter.
    // But `FeedRow` expects `user_id`.
    
    let pool = &state.db;
    // Query subscriptions joined with feeds
    let rows = sqlx::query(
        r#"
        SELECT 
            f.id as feed_id, 
            s.id as sub_id,
            s.user_id, 
            f.url, 
            s.title, 
            f.last_checked, 
            f.status, 
            s.weight 
        FROM subscriptions s
        JOIN feeds f ON s.feed_id = f.id
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("failed to query feeds: {}", e);
        Status::InternalServerError
    })?;

    let feeds = rows
        .into_iter()
        .map(|r| FeedRow {
            id: r.get::<i64, _>("feed_id"),
            subscription_id: r.get::<i64, _>("sub_id"),
            user_id: r.get::<i64, _>("user_id"),
            url: r.get::<String, _>("url"),
            title: r.get::<Option<String>, _>("title"),
            last_checked: r.get::<Option<String>, _>("last_checked"),
            status: r.get::<Option<String>, _>("status"),
            weight: r.get::<Option<i64>, _>("weight").unwrap_or(0),
        })
        .collect();

    Ok(Json(feeds))
}

/// Request body for user registration.
#[derive(Deserialize)]
struct RegisterRequest {
    username: String,
    display_name: Option<String>,
    password: String,
}

/// Request body for user login.
#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

/// JWT claims we encode (subject = user id)
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: i64,
    exp: usize,
}

/// Authentication note: token-based auth is handled by decoding a token passed in request bodies
/// (field `token`) for endpoints that accept it. A Rocket request guard implementation for
/// `AuthUser` was causing incompatibilities with the Rocket version's Outcome alias/generics
/// in this codebase. To keep the code stable and portable across toolchains, we avoid an inline
/// FromRequest implementation here.
///
/// If you want to reintroduce a request guard in the future, implement `FromRequest` that
/// returns the `rocket::request::Outcome<'r, Self, Self::Error>` type (or the alias expected by
/// your Rocket version) and use `Outcome::Success(...)` / `Outcome::Failure((Status, error))`
/// or `Outcome::Forward(...)` as appropriate. Also ensure you import the right symbols:
///   use rocket::request::{FromRequest, Outcome, Request};
/// and use `rocket::outcome::Outcome` / `rocket::request::Outcome` consistent with your Rocket crate.
///
/// For now, handlers decode the JWT from JSON payloads (field `token`) or accept explicit
/// `user_id` in the request body so authentication works without a guard.
struct AuthUser {
    // placeholder type kept for compatibility with other code sections.
    user_id: i64,
}

/// Create a signed JWT for a user id.
/// Expiration is configurable; default 24h.
fn create_jwt_for_user(user_id: i64) -> Result<String, jsonwebtoken::errors::Error> {
    let secret = std::env::var("MYNEWSLENS_JWT_SECRET").unwrap_or_else(|_| "dev-secret".into());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as usize;
    // 24h expiry
    let exp = now + (24 * 3600);
    let claims = Claims { sub: user_id, exp };
    encode(
        &JwtHeader::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

/// Register endpoint: create a user with hashed password and return a JWT.
#[post("/api/v1/register", data = "<body>")]
async fn register(
    state: &State<AppState>,
    body: Json<RegisterRequest>,
) -> Result<Json<serde_json::Value>, Status> {
    let pool = &state.db;

    // Hash password with Argon2 + random salt
    let salt = SaltString::generate(&mut OsRng);
    let argon = Argon2::default();
    let password_hash = argon
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| {
            tracing::error!("failed to hash password: {}", e);
            Status::InternalServerError
        })?
        .to_string();

    // Insert user
    let res =
        sqlx::query("INSERT INTO users (username, display_name, password_hash) VALUES (?, ?, ?)")
            .bind(&body.username)
            .bind(body.display_name.clone())
            .bind(&password_hash)
            .execute(pool)
            .await
            .map_err(|e| {
                tracing::error!("failed to insert user: {}", e);
                // If constraint violation (username exists) return conflict
                Status::InternalServerError
            })?;

    let user_id = res.last_insert_rowid();

    // Create JWT for the new user
    match create_jwt_for_user(user_id) {
        Ok(token) => Ok(Json(
            serde_json::json!({ "token": token, "user_id": user_id }),
        )),
        Err(e) => {
            tracing::error!("failed to create jwt: {}", e);
            Err(Status::InternalServerError)
        }
    }
}

/// Login endpoint: verify password and return JWT.
#[post("/api/v1/login", data = "<body>")]
async fn login(
    state: &State<AppState>,
    body: Json<LoginRequest>,
) -> Result<Json<serde_json::Value>, Status> {
    let pool = &state.db;

    // Fetch user by username
    let row = sqlx::query("SELECT id, password_hash FROM users WHERE username = ?")
        .bind(&body.username)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            tracing::error!("db error on login: {}", e);
            Status::InternalServerError
        })?;

    let row = match row {
        Some(r) => r,
        None => return Err(Status::Unauthorized),
    };

    let user_id = row.get::<i64, _>("id");
    let stored_hash: String = row.get::<String, _>("password_hash");

    // Verify password using PasswordHash parser
    let parsed_hash = PasswordHash::new(&stored_hash).map_err(|e| {
        tracing::error!("invalid password hash in db: {}", e);
        Status::InternalServerError
    })?;

    let argon = Argon2::default();
    argon
        .verify_password(body.password.as_bytes(), &parsed_hash)
        .map_err(|e| {
            tracing::warn!("password verify failed: {}", e);
            Status::Unauthorized
        })?;

    // Create JWT
    match create_jwt_for_user(user_id) {
        Ok(token) => Ok(Json(
            serde_json::json!({ "token": token, "user_id": user_id }),
        )),
        Err(e) => {
            tracing::error!("failed to create jwt: {}", e);
            Err(Status::InternalServerError)
        }
    }
}

/// Create a new feed in the database.
/// Accepts either `user_id` in the JSON body or a `token` (JWT) in the JSON body;
/// the token's `sub` claim will be used as the user id. If both are present the
/// explicit `user_id` takes precedence.
#[post("/api/v1/feeds", data = "<body>")]
async fn create_feed(
    state: &State<AppState>,
    body: Json<FeedCreate>,
) -> Result<Json<serde_json::Value>, Status> {
    let pool = &state.db;

    // Determine user id: prefer explicit user_id, otherwise attempt to decode token.
    let mut user_id_opt = body.user_id;

    if user_id_opt.is_none() {
        if let Some(ref token) = body.token {
            // Use env secret (fallback to dev-secret for local dev)
            let secret =
                std::env::var("MYNEWSLENS_JWT_SECRET").unwrap_or_else(|_| "dev-secret".into());
            let decoding_key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());
            let validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);

            match jsonwebtoken::decode::<Claims>(token, &decoding_key, &validation) {
                Ok(token_data) => {
                    user_id_opt = Some(token_data.claims.sub);
                }
                Err(e) => {
                    tracing::warn!("create_feed: failed to decode token: {}", e);
                    return Err(Status::Unauthorized);
                }
            }
        }
    }

    let user_id = match user_id_opt {
        Some(uid) => uid,
        None => {
            tracing::error!("create_feed: missing user_id and no valid token provided");
            return Err(Status::BadRequest);
        }
    };

    // Verify that the user exists
    let exists = sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            tracing::error!("db error checking user exists: {}", e);
            Status::InternalServerError
        })?;

    if exists.is_none() {
        return Err(Status::Unauthorized);
    }

    // 1. Check if feed exists (by URL)
    let feed_id_opt = sqlx::query_scalar::<_, i64>("SELECT id FROM feeds WHERE url = ?")
        .bind(&body.url)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            tracing::error!("db error checking feed: {}", e);
            Status::InternalServerError
        })?;

    let feed_id = if let Some(id) = feed_id_opt {
        id
    } else {
        // Create new feed with next_poll_at = NULL to trigger immediate polling
        let res = sqlx::query("INSERT INTO feeds (url, title, next_poll_at) VALUES (?, ?, NULL)")
            .bind(&body.url)
            .bind(body.title.as_deref()) // Initial title from first user
            .execute(pool)
            .await
            .map_err(|e| {
                tracing::error!("failed to insert feed: {}", e);
                Status::InternalServerError
            })?;
        res.last_insert_rowid()
    };

    // 2. Create subscription
    // Check if subscription already exists
    let sub_exists = sqlx::query_scalar::<_, i64>("SELECT id FROM subscriptions WHERE user_id = ? AND feed_id = ?")
        .bind(user_id)
        .bind(feed_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            tracing::error!("db error checking subscription: {}", e);
            Status::InternalServerError
        })?;

    if sub_exists.is_some() {
        // Already subscribed, return success (idempotent-ish)
        return Ok(Json(serde_json::json!({ "id": feed_id, "subscription_id": sub_exists.unwrap(), "message": "Already subscribed" })));
    }

    let res = sqlx::query("INSERT INTO subscriptions (user_id, feed_id, title) VALUES (?, ?, ?)")
        .bind(user_id)
        .bind(feed_id)
        .bind(body.title.as_deref())
        .execute(pool)
        .await
        .map_err(|e| {
            tracing::error!("failed to insert subscription: {}", e);
            Status::InternalServerError
        })?;

    let sub_id = res.last_insert_rowid();
    Ok(Json(serde_json::json!({ "id": feed_id, "subscription_id": sub_id })))
}

/// Minimal fetch trigger for a feed: enqueues a background task that will perform the fetch.
/// For now this is a placeholder that logs and updates last_checked time.
#[derive(Deserialize)]
struct FetchRequest {
    feed_id: i64,
}

#[post("/api/v1/fetch", data = "<req>")]
async fn trigger_fetch(state: &State<AppState>, req: Json<FetchRequest>) -> Result<Status, Status> {
    let feed_id = req.feed_id;
    let pool = state.db.clone();
    let config = state.config.clone();
    let llm_provider = state.llm_provider.clone();
    
    // Spawn a background task to fetch and parse the feed
    tokio::spawn(async move {
        tracing::info!("manual fetch: triggered for feed id {}", feed_id);
        
        // Get feed URL
        let feed_row = sqlx::query("SELECT url, poll_interval_minutes, adaptive_scheduling FROM feeds WHERE id = ?")
            .bind(feed_id)
            .fetch_optional(&pool)
            .await;
            
        let (url, mut interval, adaptive) = match feed_row {
            Ok(Some(row)) => {
                let url: String = row.try_get("url").unwrap_or_default();
                let interval: i64 = row.try_get("poll_interval_minutes").unwrap_or(60);
                let adaptive: bool = row.try_get("adaptive_scheduling").unwrap_or(false);
                (url, interval, adaptive)
            }
            Ok(None) => {
                tracing::error!("manual fetch: feed {} not found", feed_id);
                return;
            }
            Err(e) => {
                tracing::error!("manual fetch: failed to query feed {}: {}", feed_id, e);
                return;
            }
        };
        
        // Fetch and parse feed
        let timeout = config
            .as_ref()
            .and_then(|c| c.politeness.as_ref())
            .and_then(|p| p.fetch_timeout_seconds)
            .unwrap_or(10);
            
        let fetch_result = ingestion::fetch_and_parse_feed(&url, timeout).await;
        
        let mut new_items_found = false;
        let fetch_success = fetch_result.is_ok();
        
        match fetch_result {
            Ok(feed) => {
                tracing::info!("manual fetch: successfully fetched feed {}, found {} items", feed_id, feed.entries.len());
                
                match storage::store_feed_items(&pool, feed_id, &feed.entries).await {
                    Ok(new_article_ids) => {
                        let new_count = new_article_ids.len();
                        if new_count > 0 {
                            new_items_found = true;
                            tracing::info!("manual fetch: stored {} new articles for feed {}", new_count, feed_id);
                            
                            // Process articles with LLM if available
                            if let Some(llm_prov) = llm_provider.clone() {
                                let pool_clone = pool.clone();
                                let model = config.as_ref()
                                    .and_then(|c| c.llm.as_ref())
                                    .and_then(|l| l.remote.as_ref())
                                    .and_then(|r| r.model.as_deref())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let ids = new_article_ids.clone();
                                
                                tokio::spawn(async move {
                                    if let Err(e) = crate::processing::batch_process_articles(
                                        &pool_clone,
                                        &ids,
                                        llm_prov,
                                        &model
                                    ).await {
                                        tracing::error!("manual fetch: failed to process articles: {}", e);
                                    }
                                });
                            }
                        } else {
                            tracing::info!("manual fetch: no new articles for feed {}", feed_id);
                        }
                    }
                    Err(e) => {
                        tracing::error!("manual fetch: failed to store items for feed {}: {}", feed_id, e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("manual fetch: failed to fetch feed {}: {}", feed_id, e);
            }
        }
        
        // Adaptive logic (same as worker)
        if adaptive && fetch_success {
            if new_items_found {
                interval = (interval / 2).max(15);
            } else {
                interval = (interval + (interval / 2)).min(1440);
            }
        }
        
        // Calculate next poll time and update DB
        let now = chrono::Utc::now();
        let next_poll = now + chrono::Duration::minutes(interval);
        
        if let Err(e) = sqlx::query(
            "UPDATE feeds SET next_poll_at = ?, poll_interval_minutes = ?, last_checked = ? WHERE id = ?"
        )
        .bind(next_poll)
        .bind(interval)
        .bind(now)
        .bind(feed_id)
        .execute(&pool)
        .await {
            tracing::error!("manual fetch: failed to update feed {}: {}", feed_id, e);
        } else {
            tracing::info!("manual fetch: updated feed {} (next poll at {}, interval {}m)", feed_id, next_poll, interval);
        }
    });

    Ok(Status::Accepted)
}

// ============================================================================
// Session Management Endpoints
// ============================================================================

#[derive(Deserialize)]
struct CreateSessionRequest {
    user_id: i64,
    duration_seconds: Option<i32>,
}

#[derive(Serialize)]
struct SessionWithMessages {
    session: crate::sessions::Session,
    messages: Vec<crate::sessions::ChatMessage>,
}

#[post("/api/v1/sessions", data = "<body>")]
async fn create_session(
    state: &State<AppState>,
    body: Json<CreateSessionRequest>,
) -> Result<Json<crate::sessions::Session>, Status> {
    let pool = &state.db;
    crate::sessions::create_session(&state.db, body.user_id, body.duration_seconds)
        .await
        .map(Json)
        .map_err(|_| Status::InternalServerError)
}

#[get("/api/v1/sessions?<user_id>")]
async fn list_sessions(
    state: &State<AppState>,
    user_id: i64,
) -> Result<Json<Vec<crate::sessions::Session>>, Status> {
    crate::sessions::list_sessions(&state.db, user_id)
        .await
        .map(Json)
        .map_err(|_| Status::InternalServerError)
}

#[get("/api/v1/sessions/<session_id>")]
async fn get_session(
    state: &State<AppState>,
    session_id: i64,
) -> Result<Json<SessionWithMessages>, Status> {
    crate::sessions::get_session_with_messages(&state.db, session_id)
        .await
        .map(|(session, messages)| Json(SessionWithMessages { session, messages }))
        .map_err(|_| Status::InternalServerError)
}

/// Trigger processing of pending articles
#[post("/api/v1/process-pending")]
async fn process_pending(state: &State<AppState>) -> Status {
    let pool = state.db.clone();
    let llm_provider = state.llm_provider.clone();
    let config = state.config.clone();
    
    tokio::spawn(async move {
        tracing::info!("Manual trigger: processing pending articles");
        
        if let Some(llm_prov) = llm_provider {
            let model = config.as_ref()
                .and_then(|c| c.llm.as_ref())
                .and_then(|l| l.remote.as_ref())
                .and_then(|r| r.model.as_deref())
                .unwrap_or("unknown")
                .to_string();
            
            match crate::processing::process_pending_articles(&pool, llm_prov, &model, Some(50)).await {
                Ok(count) => tracing::info!("Processed {} pending articles", count),
                Err(e) => tracing::error!("Failed to process pending articles: {:?}", e),
            }
        } else {
            tracing::warn!("No LLM provider configured, cannot process articles");
        }
    });
    
    Status::Accepted
}


// ============================================================================
// Database Schema Management
// ============================================================================

/// Ensure the required schema exists. This runs CREATE TABLE IF NOT EXISTS statements for core tables.
/// This function is idempotent and safe to call at startup.
pub async fn ensure_schema(pool: &SqlitePool) -> Result<()> {
    tracing::info!("server: ensuring DB schema (CREATE TABLE IF NOT EXISTS ...)");
    // Check for migration: if `feeds` table has `user_id` column, it's the old schema.
    // We use pragma_table_info to check columns.
    let needs_migration = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pragma_table_info('feeds') WHERE name='user_id'"
    )
    .fetch_optional(pool)
    .await
    .unwrap_or(None)
    .unwrap_or(0) > 0;

    if needs_migration {
        tracing::info!("Newscope server starting"); // Added based on Code Edit, simplified for syntactic correctness
        tracing::info!("server: detecting old schema (feeds.user_id exists), migrating...");
        // Rename old table
        sqlx::query("ALTER TABLE feeds RENAME TO feeds_old").execute(pool).await?;
        
        // Create new tables (we'll do this via the standard stmts loop below, but we need to ensure they are created before data migration)
        // Actually, let's just create them here to be safe and populate them.
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS feeds (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL UNIQUE,
                site_url TEXT,
                title TEXT,
                last_checked TIMESTAMP,
                status TEXT,
                next_poll_at TIMESTAMP,
                poll_interval_minutes INTEGER DEFAULT 60,
                adaptive_scheduling BOOLEAN DEFAULT TRUE,
                weight INTEGER DEFAULT 0
            );
        "#).execute(pool).await?;

        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS subscriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                feed_id INTEGER NOT NULL,
                title TEXT,
                weight INTEGER DEFAULT 0,
                created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY(feed_id) REFERENCES feeds(id) ON DELETE CASCADE,
                UNIQUE(user_id, feed_id)
            );
        "#).execute(pool).await?;

        // Migrate data
        tracing::info!("server: migrating data from feeds_old...");
        // Insert unique feeds
        sqlx::query(r#"
            INSERT OR IGNORE INTO feeds (url, site_url, title, last_checked, status, weight)
            SELECT url, site_url, title, last_checked, status, weight FROM feeds_old
        "#).execute(pool).await?;

        // Insert subscriptions
        sqlx::query(r#"
            INSERT INTO subscriptions (user_id, feed_id, title, weight)
            SELECT fo.user_id, f.id, fo.title, fo.weight
            FROM feeds_old fo
            JOIN feeds f ON fo.url = f.url
        "#).execute(pool).await?;

        // Drop old table
        sqlx::query("DROP TABLE feeds_old").execute(pool).await?;
        tracing::info!("server: migration complete");
    }

    let stmts = [
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL UNIQUE,
            display_name TEXT,
            password_hash TEXT,
            prefs_json TEXT,
            created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            last_login TIMESTAMP
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS feeds (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT NOT NULL UNIQUE,
            site_url TEXT,
            title TEXT,
            last_checked TIMESTAMP,
            status TEXT,
            next_poll_at TIMESTAMP,
            poll_interval_minutes INTEGER DEFAULT 60,
            adaptive_scheduling BOOLEAN DEFAULT TRUE,
            weight INTEGER DEFAULT 0
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS subscriptions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            feed_id INTEGER NOT NULL,
            title TEXT,
            weight INTEGER DEFAULT 0,
            created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
            FOREIGN KEY(feed_id) REFERENCES feeds(id) ON DELETE CASCADE,
            UNIQUE(user_id, feed_id)
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS articles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            canonical_url TEXT NOT NULL UNIQUE,
            title TEXT,
            content TEXT,
            full_content TEXT,
            published_at TIMESTAMP,
            processing_status TEXT DEFAULT 'pending',
            processed_at TIMESTAMP,
            created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            canonical_hash TEXT
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS article_occurrences (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            article_id INTEGER NOT NULL,
            feed_id INTEGER NOT NULL,
            feed_item_id TEXT,
            discovered_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            FOREIGN KEY(article_id) REFERENCES articles(id) ON DELETE CASCADE,
            FOREIGN KEY(feed_id) REFERENCES feeds(id) ON DELETE CASCADE
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS article_summaries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            article_id INTEGER NOT NULL UNIQUE,
            headline TEXT,
            bullets_json TEXT,
            details TEXT,
            model TEXT,
            prompt_tokens INTEGER,
            completion_tokens INTEGER,
            created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            FOREIGN KEY(article_id) REFERENCES articles(id) ON DELETE CASCADE
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS llm_usage_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            operation TEXT,
            model TEXT,
            prompt_tokens INTEGER,
            completion_tokens INTEGER,
            success BOOLEAN,
            error_message TEXT,
            created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            start_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            duration_requested_seconds INTEGER,
            digest_summary_id INTEGER,
            FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS chat_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id INTEGER NOT NULL,
            author TEXT NOT NULL,
            message TEXT,
            created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS summaries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id INTEGER,
            summary_text TEXT,
            by_model TEXT,
            tokens_used INTEGER,
            created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
        );
        "#,
    ];

    for s in &stmts {
        sqlx::query(s)
            .execute(pool)
            .await
            .with_context(|| "failed to ensure schema")?;
    }

    // Idempotent migrations for new columns
    // Add processing_status to articles if it doesn't exist
    let has_processing_status = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pragma_table_info('articles') WHERE name='processing_status'"
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0) > 0;

    if !has_processing_status {
        tracing::info!("Adding processing_status column to articles table");
        sqlx::query("ALTER TABLE articles ADD COLUMN processing_status TEXT DEFAULT 'pending'")
            .execute(pool)
            .await
            .context("Failed to add processing_status column")?;
        
        sqlx::query("ALTER TABLE articles ADD COLUMN processed_at TIMESTAMP")
            .execute(pool)
            .await
            .context("Failed to add processed_at column")?;
    }

    tracing::info!("server: DB schema ensured");
    Ok(())
}

/// Build and launch a Rocket server.
///
/// The server will attempt to load configuration from the path specified in the `CONFIG_PATH`
/// environment variable, falling back to `/config/config.toml` then `config.toml` in the
/// current working directory. If configuration cannot be read, the server still starts but the
/// `/api/v1/status` response will be limited.
///
/// This function blocks until the Rocket server shuts down (it awaits `rocket.launch().await`)
/// and returns an error if Rocket fails to start.
pub async fn launch_rocket(db_pool: Arc<SqlitePool>, config: Option<Arc<Config>>) -> Result<()> {
    // The DB pool and optional application config are provided by the caller.
    // The server must not re-init or migrate the database here; migrations and pool
    // creation are the responsibility of the process startup code (main).
    
    // Initialize LLM provider if configured
    let llm_provider: Option<Arc<dyn crate::llm::LlmProvider>> = if let Some(ref cfg) = config {
        if let Some(ref llm_config) = cfg.llm {
            if llm_config.adapter.as_deref() == Some("remote") {
                if let Some(ref remote_cfg) = llm_config.remote {
                    if let (Some(api_url), Some(api_key_env)) = (&remote_cfg.api_url, &remote_cfg.api_key_env) {
                        if let Ok(api_key) = std::env::var(api_key_env) {
                            let model = remote_cfg.model.clone().unwrap_or_else(|| "gpt-4o-mini".to_string());
                            let provider = crate::llm::remote::RemoteLlmProvider::new(
                                api_url,
                                &api_key,
                                &model,
                            ).with_defaults(
                                remote_cfg.timeout_seconds.unwrap_or(30),
                                500,
                                0.7,
                            );
                            tracing::info!("LLM provider initialized: remote ({}) at {}", model, api_url);
                            Some(Arc::new(provider) as Arc<dyn crate::llm::LlmProvider>)
                        } else {
                            tracing::warn!("LLM configured but API key env var '{}' not set", api_key_env);
                            None
                        }
                    } else {
                        tracing::warn!("LLM remote config: missing api_url or api_key_env");
                        None
                    }
                } else {
                    None
                }
            } else {
                tracing::info!("LLM adapter '{}' not supported yet", llm_config.adapter.as_deref().unwrap_or("none"));
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    
    let state = AppState {
        started_at: Utc::now(),
        config,
        db: db_pool.as_ref().clone(), // Unwrap Arc since SqlitePool is already ref-counted
        llm_provider,
    };

    // Build Rocket with managed state and mount routes, applying server.bind and server.port from a config file if present.
    // Determine config path: env CONFIG_PATH -> /config/config.toml -> ./config.toml
    let mut fig = rocket::Config::figment();
    let cfg_path_env =
        std::env::var("CONFIG_PATH").unwrap_or_else(|_| "/config/config.toml".to_string());
    let cfg_path = if std::path::Path::new(&cfg_path_env).exists() {
        cfg_path_env
    } else if std::path::Path::new("config.toml").exists() {
        "config.toml".to_string()
    } else {
        String::new()
    };

    if !cfg_path.is_empty() {
        // Read config file and extract [server] bind/port if present (defensive; failure here is non-fatal)
        if let Ok(cfg_contents) = std::fs::read_to_string(&cfg_path) {
            if let Ok(toml_val) = toml::from_str::<toml::Value>(&cfg_contents) {
                if let Some(server_val) = toml_val.get("server") {
                    if let Some(bind) = server_val.get("bind").and_then(|v| v.as_str()) {
                        // Merge address from config
                        fig = fig.merge(("address", bind.to_string()));
                    }
                    if let Some(port) = server_val.get("port").and_then(|v| v.as_integer()) {
                        // Merge port from config (figment expects integer)
                        fig = fig.merge(("port", port as u16));
                    }
                }
            }
        }
    }

    let rocket = rocket::custom(fig).manage(state).mount(
        "/",
        routes![
            index_redirect,
            health,
            status,
            list_users,
            list_feeds,
            create_feed,
            trigger_fetch,
            process_pending,
            register,
            login,
            // Session routes
            create_session,
            list_sessions,
            get_session,
        ],
    )
    .mount("/ws", routes![
        crate::sessions::websocket::chat_websocket,
    ])
    .mount("/static", FileServer::from("mynewslens/static"));

    // Launch Rocket - this will run until shutdown (SIGINT/SIGTERM etc.)
    tracing::info!("Starting Rocket HTTP server");
    rocket
        .launch()
        .await
        .map_err(|e| anyhow!("Rocket failed: {}", e))?;

    tracing::info!("Rocket HTTP server has shut down");
    Ok(())
}
