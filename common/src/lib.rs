/*!
common/src/lib.rs

Shared configuration types and DB helper functions for MyNewsLens.

This file provides:
- Config data structures (deserialized from TOML)
- An async loader for a TOML config file
- Helpers to initialize and migrate an SQLite database
*/

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

/// Database configuration section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Path to the sqlite database file (e.g. "data/mynewslens.db")
    pub path: String,
}

/// Scheduler (ingestion times) configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// List of wall-clock times in "HH:MM" 24h format when ingestion should run
    pub times: Vec<String>,
}

/// Politeness / fetching configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolitenessConfig {
    pub delay_seconds: Option<u64>,
    pub concurrency_per_domain: Option<u32>,
    pub max_response_bytes: Option<u64>,
    pub fetch_timeout_seconds: Option<u64>,
    pub respect_robots_txt: Option<bool>,
}

/// Local LLM config (used if `llm.adapter = "local"`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalLlmConfig {
    pub model_path: Option<String>,
    pub max_threads: Option<u32>,
}

/// Remote LLM config (used if `llm.adapter = "remote"`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteLlmConfig {
    pub api_url: Option<String>,
    pub api_key_env: Option<String>,
    pub model: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub max_tokens: Option<usize>,
}

/// LLM top-level config grouping local/remote specifics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub adapter: Option<String>, // "local", "remote", "none"
    pub local: Option<LocalLlmConfig>,
    // Fallback: single remote config
    pub remote: Option<RemoteLlmConfig>,
    // Task-specific configs
    pub summarization: Option<RemoteLlmConfig>,
    pub personalization: Option<RemoteLlmConfig>,
    pub embedding: Option<RemoteLlmConfig>,
    pub interaction: Option<RemoteLlmConfig>,
    // Compatibility redirects
    pub background: Option<RemoteLlmConfig>,
    pub interactive: Option<RemoteLlmConfig>,
}

/// Simple feed descriptor used in per-user initial feed lists
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedConfig {
    pub url: String,
    pub title: Option<String>,
}

/// Per-user configuration (users are defined in the global config file)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub username: String,
    pub display_name: Option<String>,
    pub preferred_language: Option<String>,
    pub password_hash: Option<String>,
    #[serde(default)]
    pub feeds: Vec<FeedConfig>,
}

/// Scoring defaults group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringConfig {
    pub w_pref: Option<f64>,
    pub w_red: Option<f64>,
    pub w_recency: Option<f64>,
    pub w_src: Option<f64>,
    pub w_novel: Option<f64>,
    pub serendipity: Option<f64>,
}

/// Admin / maintenance config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminConfig {
    pub auto_migrate: Option<bool>,
    pub diagnostics_dir: Option<String>,
}

/// Top-level application configuration (deserialized from config.toml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub database: DatabaseConfig,
    pub scheduler: SchedulerConfig,
    pub politeness: Option<PolitenessConfig>,
    pub llm: Option<LlmConfig>,
    #[serde(default)]
    pub users: Vec<UserConfig>,
    pub scoring: Option<ScoringConfig>,
    pub admin: Option<AdminConfig>,
}

impl Config {
    /// Load configuration from a TOML file asynchronously.
    ///
    /// Example:
    ///   let cfg = Config::from_file("config.toml").await?;
    pub async fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = tokio::fs::read_to_string(path.as_ref())
            .await
            .with_context(|| format!("Failed to read config file: {}", path.as_ref().display()))?;
        let cfg: Config = toml::from_str(&data).context("Failed to parse TOML configuration")?;
        Ok(cfg)
    }

    /// Load configuration with an optional default file and an optional override file.
    /// If both are present, they are merged (override takes precedence).
    pub async fn load_with_defaults(default_path: Option<&Path>, override_path: Option<&Path>) -> Result<Self> {
        let mut config_value = toml::Value::Table(toml::map::Map::new());

        if let Some(path) = default_path {
            if path.exists() {
                let data = tokio::fs::read_to_string(path).await
                    .with_context(|| format!("Failed to read default config: {}", path.display()))?;
                let val: toml::Value = toml::from_str(&data)
                    .context("Failed to parse default configuration")?;
                merge_toml(&mut config_value, val);
            }
        }

        if let Some(path) = override_path {
            if path.exists() {
                let data = tokio::fs::read_to_string(path).await
                    .with_context(|| format!("Failed to read override config: {}", path.display()))?;
                let val: toml::Value = toml::from_str(&data)
                    .context("Failed to parse override configuration")?;
                merge_toml(&mut config_value, val);
            }
        }
        
        let cfg: Config = config_value.try_into().context("Failed to parse merged configuration")?;
        Ok(cfg)
    }
}

fn merge_toml(a: &mut toml::Value, b: toml::Value) {
    match (a, b) {
        (toml::Value::Table(a_map), toml::Value::Table(b_map)) => {
            for (k, v) in b_map {
                if let Some(a_val) = a_map.get_mut(&k) {
                    merge_toml(a_val, v);
                } else {
                    a_map.insert(k, v);
                }
            }
        }
        (a_val, b_val) => *a_val = b_val,
    }
}

/// Run SQL migrations using sqlx's migration macro.
/// This expects a `migrations` directory at the project root (or packaged alongside the binary)
/// containing SQL migration files. The caller provides an async `SqlitePool` and the migrator
/// is executed against the provided pool.
///
/// Note: this function intentionally accepts a pool rather than working with rusqlite or raw
/// files so the migration process integrates with the `sqlx` async stack cleanly.
pub async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    // Use sqlx migration macro to locate and run migrations.
    // When built inside the workspace, migrations are located at ../migrations relative to this crate.
    // Packaging may require adapting this path or embedding migrations.
    sqlx::migrate!("../migrations")
        .run(pool)
        .await
        .context("Failed to run sqlx migrations")?;

    Ok(())
}

/// Initialize an SQLite connection pool.
///
/// This function will create the parent directory if necessary, ensure the DB file exists
/// (attempting to create it if missing), and return a configured `SqlitePool`. Defaults are
/// conservative for resource-constrained platforms:
/// - max_connections: 5
/// - connection timeout default provided by `sqlx`
///
/// Example:
///   let pool = init_db_pool("data/mynewslens.db").await?;
pub async fn init_db_pool(path: &str) -> Result<SqlitePool> {
    // Ensure parent directory exists
    if let Some(parent) = Path::new(path).parent() {
        tokio::fs::create_dir_all(parent).await.with_context(|| {
            format!("Failed to create DB parent directory: {}", parent.display())
        })?;
    }

    // Try to create the DB file if it does not already exist. This gives a clearer error
    // earlier (filesystem permission or path issues) instead of only surfacing it via the
    // SQLite connection attempt.
    tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(path)
        .await
        .with_context(|| format!("Failed to create or open DB file: {}", path))?;

    // Migrations are intended to be executed explicitly by the caller (for example, from `main`)
    // using `run_migrations(pool)` once a `SqlitePool` is available.

    // Use a modest pool size for RPI and similar devices. Provide more context on connect errors.
    let mut options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path))?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

    // Load sqlite-vec extension
    // We use the loadable extension (vec0.so) which should be in the root directory.
    options = options.extension_with_entrypoint("./vec0", "sqlite3_vec_init");

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .with_context(|| format!("Failed to connect to sqlite database at path: {}", path))?;

    Ok(pool)
}

/// Convenience: sleep helper used by implementations (kept public for tests)
pub async fn sleep_millis(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// Ensure that users defined in the in-memory configuration are present in the `users` table.
/// This function will:
///  - INSERT OR IGNORE a row for each configured user (so it is safe to call multiple times)
///  - UPDATE the `display_name` and `password_hash` if those fields are provided in the config
/// Usage: call this once after running migrations so the `users` table contains the configured users.
pub async fn sync_users(config: &Config, pool: &SqlitePool) -> Result<()> {
    for u in &config.users {
        // Insert if missing (prefs_json left NULL for now)
        sqlx::query(
            "INSERT OR IGNORE INTO users (username, display_name, password_hash, prefs_json) VALUES (?, ?, ?, ?)"
        )
        .bind(&u.username)
        .bind(u.display_name.clone())
        .bind(u.password_hash.clone())
        .bind(None::<String>)
        .execute(pool)
        .await
        .with_context(|| format!("failed to insert or ignore user {}", u.username))?;

        // Update fields if provided in config (COALESCE keeps existing values if None provided)
        sqlx::query(
            "UPDATE users SET display_name = COALESCE(?, display_name), password_hash = COALESCE(?, password_hash) WHERE username = ?"
        )
        .bind(u.display_name.clone())
        .bind(u.password_hash.clone())
        .bind(&u.username)
        .execute(pool)
        .await
        .with_context(|| format!("failed to update user {}", u.username))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::SystemTime;

    #[tokio::test]
    async fn config_from_string_and_db_pool() {
        // Minimal TOML to test parsing
        let toml = r#"
            [database]
            path = "data/test.db"

            [scheduler]
            times = ["05:00", "11:00"]

            [[users]]
            username = "alice"
            display_name = "Alice"
        "#;

        // Parse from string using toml crate directly for test
        let cfg: Config = toml::from_str(toml).expect("parse config");
        assert_eq!(cfg.scheduler.times.len(), 2);
        assert_eq!(cfg.users.len(), 1);
        assert_eq!(cfg.users[0].username, "alice");

        // Test DB pool initialization in a temporary directory under the OS temp dir
        let now = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_millis();
        let dir = std::env::temp_dir().join(format!("mynews_test_{}", now));
        let _ = fs::create_dir_all(&dir);
        let db_path = dir.join("mynews.db");
        let db_path_str = db_path.to_string_lossy().to_string();

        let pool = init_db_pool(&db_path_str).await.expect("init pool");
        // Simple sanity: acquire a connection
        let conn = pool.acquire().await.expect("acquire conn");
        drop(conn);
    }
}
