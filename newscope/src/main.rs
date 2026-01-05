/*
newscope - single-binary main.rs
This binary starts the Rocket HTTP server and runs the background worker inside the same process.
*/

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use common::Config;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::select;
use tokio::sync::Notify;
use tokio::time::Duration;
use tracing::{error, info};
use tracing_subscriber::{fmt, EnvFilter};

use common::init_db_pool;
use sqlx::Row;

// Import modules from the lib
use newscope::server;
use newscope::llm;
use newscope::ingestion;
use newscope::storage;
use newscope::processing;
use newscope::personalization;
use server::launch_rocket;

#[derive(Parser, Debug)]
#[command(name = "newscope", about = "Newscope single-binary server + worker")]
struct Args {
    /// Path to config.toml
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Disable background worker (run server only)
    #[arg(long)]
    no_worker: bool,

    /// Run worker only (do not bind HTTP server)
    #[arg(long)]
    worker_only: bool,

    /// Override log level (info, debug, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse CLI args
    let args = Args::parse();

    // Initialize logging
    let filter = EnvFilter::try_new(&args.log_level).unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();

    // Resolve config paths
    let default_path = PathBuf::from("config.default.toml");
    
    let override_path = if let Some(p) = args.config {
        if !p.exists() {
             error!(path = ?p, "specified config file not found");
             return Err(anyhow::anyhow!("Config file not found: {}", p.display()));
        }
        Some(p)
    } else {
        let p = PathBuf::from("config.toml");
        if p.exists() { Some(p) } else { None }
    };

    // Load configuration with defaults
    let config = match Config::load_with_defaults(
        if default_path.exists() { Some(&default_path) } else { None },
        override_path.as_deref()
    ).await {
        Ok(cfg) => cfg,
        Err(e) => {
            error!(%e, "failed to load configuration");
            return Err(e.into());
        }
    };
    info!(default = ?default_path, override = ?override_path, "configuration loaded");

    // Initialize DB pool - resolve and log the absolute DB path before connecting
    let db_path_abs = match tokio::fs::canonicalize(&config.database.path).await {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => config.database.path.clone(),
    };
    info!(db_path = %db_path_abs, "resolved DB path");

    let db_pool = match init_db_pool(&db_path_abs).await {
        Ok(p) => p,
        Err(e) => {
            error!(%e, db_path = %db_path_abs, "failed to initialize database pool");
            return Err(e.into());
        }
    };
    let db_pool = Arc::new(db_pool);

    // Prepare a shutdown notifier to signal worker tasks
    let shutdown_notify = Arc::new(Notify::new());

    // Initialize LLM providers (dual mode: background + interactive)
    let background_llm: Option<Arc<dyn newscope::llm::LlmProvider>> = if let Some(ref llm_config) = config.llm {
        match create_llm_provider(llm_config, LlmMode::Background) {
            Ok(provider) => {
                info!("Background LLM provider initialized: {:?}", llm_config.background.as_ref()
                    .or(llm_config.remote.as_ref())
                    .and_then(|c| c.model.as_deref())
                    .unwrap_or("unknown"));
                Some(Arc::from(provider))
            }
            Err(e) => {
                error!("Failed to initialize background LLM provider: {}", e);
                None
            }
        }
    } else {
        None
    };

    let interactive_llm: Option<Arc<dyn newscope::llm::LlmProvider>> = if let Some(ref llm_config) = config.llm {
        match create_llm_provider(llm_config, LlmMode::Interactive) {
            Ok(provider) => {
                info!("Interactive LLM provider initialized: {:?}", llm_config.interactive.as_ref()
                    .or(llm_config.remote.as_ref())
                    .and_then(|c| c.model.as_deref())
                    .unwrap_or("unknown"));
                Some(Arc::from(provider))
            }
            Err(e) => {
                error!("Failed to initialize interactive LLM provider: {}", e);
                None
            }
        }
    } else {
        None
    };

    // If worker_only, run the worker tasks (without HTTP) and exit when shutdown requested
    if args.worker_only {
        info!("Starting in worker-only mode");
        let worker = run_worker(db_pool.clone(), config.clone(), shutdown_notify.clone(), background_llm.clone());

        // Wait for CTRL-C or worker completion (worker runs until notified)
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("ctrl-c received, notifying worker to shutdown");
                shutdown_notify.notify_waiters();
                // give worker a small grace period
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            res = worker => {
                if let Err(e) = res {
                    error!(%e, "worker encountered an error");
                }
            }
        }
        info!("worker-only run finished");
        return Ok(());
    }

    // Otherwise, start worker (unless disabled) and then start HTTP server.
    let mut worker_handle = None;
    if !args.no_worker {
        info!("Spawning background worker task");
        let w_db = db_pool.clone();
        let w_cfg = config.clone();
        let w_shutdown = shutdown_notify.clone();
        let w_llm = background_llm.clone();
        worker_handle = Some(tokio::spawn(async move {
            if let Err(e) = run_worker(w_db, w_cfg, w_shutdown, w_llm).await {
                error!(%e, "background worker failed");
                Err(e)
            } else {
                Ok(())
            }
        }));
    } else {
        info!("Background worker disabled via CLI (--no-worker)");
    }

    // Before launching the HTTP server, optionally run automatic DB migrations
    // if the administrator enabled `admin.auto_migrate = true` in config.
    // Also ensure the DB file/directory exists (init_db_pool already creates parent dir).
    if config
        .admin
        .as_ref()
        .and_then(|a| a.auto_migrate)
        .unwrap_or(false)
    {
        info!("Auto-migrate enabled: running DB migrations");
        // Run sqlx migrations against the existing pool. The migrations directory may be empty
        // in some packaging scenarios; to be robust we also ensure the core schema exists.
        sqlx::migrate!("../migrations").run(&*db_pool).await?;
        info!("DB migrations completed");
        // Ensure core schema exists even if migrations didn't create tables (defensive).
        server::ensure_schema(&*db_pool).await?;
    // Start worker loop
    info!("Newscope worker starting...");
    
    // Initial fetch
    info!("Performing initial feed fetch...");
        // Ensure users defined in config are present in the DB users table
        common::sync_users(&config, &*db_pool).await?;
        info!("Configuration users synchronized into database");
    }

    // Launch the Rocket server (blocking until Rocket shuts down)
    // The server is implemented in the `server` module and should return when it stops.
    info!("Launching Rocket HTTP server");
    if let Err(e) = launch_rocket(db_pool.clone(), Some(Arc::new(config.clone()))).await {
        error!(%e, "Rocket server failed");
        // Signal worker to stop if running
        shutdown_notify.notify_waiters();
    }

    // When the server shuts down, notify worker and wait a bit for graceful termination.
    info!("HTTP server stopped; notifying worker to shutdown");
    shutdown_notify.notify_waiters();

    // Optionally wait for the worker to finish or timeout
    if let Some(handle) = worker_handle {
        match tokio::time::timeout(Duration::from_secs(20), handle).await {
            Ok(join_res) => match join_res {
                Ok(Ok(_)) => info!("worker exited cleanly"),
                Ok(Err(e)) => error!(%e, "worker task returned an error"),
                Err(join_err) => error!(%join_err, "worker task panicked"),
            },
            Err(_) => {
                info!("Timed out waiting for worker to exit; continuing shutdown");
            }
        }
    }

    info!("Shutdown complete");
    Ok(())
}

/// LLM mode for selecting appropriate configuration
#[derive(Debug, Clone, Copy)]
enum LlmMode {
    Background,   // For article processing (slow, powerful)
    Interactive,  // For press review & chat (fast, lightweight)
}

/// Create an LLM provider based on configuration and mode
fn create_llm_provider(llm_config: &common::LlmConfig, mode: LlmMode) -> anyhow::Result<Box<dyn newscope::llm::LlmProvider>> {
    let adapter = llm_config.adapter.as_deref().unwrap_or("none");
    match adapter {
        "local" => {
            // Placeholder for local provider
            anyhow::bail!("Local LLM adapter not yet implemented in main.rs factory")
        }
        "remote" => {
            // Choose config based on mode
            let endpoint_config = match mode {
                LlmMode::Background => llm_config.background.as_ref()
                    .or(llm_config.remote.as_ref()),
                LlmMode::Interactive => llm_config.interactive.as_ref()
                    .or(llm_config.remote.as_ref()),
            };

            if let Some(remote_config) = endpoint_config {
                // Fetch API key from env var
                let api_key_env = remote_config.api_key_env.as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Missing api_key_env in remote config"))?;
                
                let api_key = std::env::var(api_key_env)
                    .with_context(|| format!("LLM API key env var '{}' not set", api_key_env))?;
                
                let model = remote_config.model.clone().unwrap_or_else(|| "gpt-4o-mini".to_string());
                let api_url = remote_config.api_url.clone().unwrap_or_else(|| "http://localhost:11434/v1/chat/completions".to_string());
                let timeout_secs = remote_config.timeout_seconds.unwrap_or(30);
                let max_tokens = remote_config.max_tokens.unwrap_or(500);

                let provider = newscope::llm::remote::RemoteLlmProvider::new(
                    api_url,
                    api_key,
                    model,
                ).with_defaults(
                    timeout_secs,
                    max_tokens,
                    0.7,
                );
                Ok(Box::new(provider))
            } else {
                anyhow::bail!("Remote adapter selected but no LLM config found for mode {:?}", mode)
            }
        }
        "none" => {
             anyhow::bail!("LLM adapter 'none' not supported in this factory yet")
        }
        _ => anyhow::bail!("Unknown LLM adapter type: {}", adapter),
    }
}

/// run_worker is the top-level background worker entrypoint. It runs until `shutdown_notify`
/// is signalled. The function encapsulates scheduling logic, politeness and ingestion loops.
/// For now it runs a placeholder schedule loop. Replace the TODO sections with the real logic.
async fn run_worker(
    _db_pool: Arc<sqlx::SqlitePool>,
    config: Config,
    shutdown_notify: Arc<Notify>,
    background_llm: Option<Arc<dyn newscope::llm::LlmProvider>>,
) -> anyhow::Result<()> {
    info!(
        "worker: initializing scheduler with times: {:?}",
        config.scheduler.times
    );

    // Example: convert times to a vector for scheduling; real implementation should parse times
    // and schedule ingestion windows precisely at wall-clock times.
    // Placeholder loop: tick every hour and respond to shutdown.

    loop {
        info!("worker: checking for feeds to update");
        
        // 1. Find feeds due for update
        let now = Utc::now();
        let feeds = sqlx::query(
            "SELECT id, url, poll_interval_minutes, adaptive_scheduling FROM feeds WHERE next_poll_at <= ? OR next_poll_at IS NULL"
        )
        .bind(now)
        .fetch_all(&*_db_pool)
        .await;

        match feeds {
            Ok(rows) => {
                if rows.is_empty() {
                    info!("worker: no feeds due for update");
                } else {
                    info!("worker: found {} feeds to update", rows.len());
                    
                    for row in rows {
                        let feed_id: i64 = row.get("id");
                        let url: String = row.get("url");
                        let mut interval: i64 = row.get("poll_interval_minutes");
                        let adaptive: bool = row.get("adaptive_scheduling");
                        
                        info!("worker: processing feed {} ({})", feed_id, url);
                        
                        // Fetch feed
                        let timeout = config.politeness.as_ref()
                            .and_then(|p| p.fetch_timeout_seconds)
                            .unwrap_or(10);
                        // 2. Fetch and parse
                        match newscope::ingestion::fetch_and_parse_feed(&url, timeout).await {
                            Ok(feed) => {
                                info!("Fetched feed '{}': {} items", url, feed.entries.len());
                                let mut new_items_found = false;
                                match newscope::storage::store_feed_items(&_db_pool, feed_id, &feed.entries).await {
                                    Ok(article_ids) => {
                                        info!("Stored {} items for feed '{}'", article_ids.len(), url);
                                        
                                        // 3. Process new articles with LLM if configured
                                        if !article_ids.is_empty() {
                                            new_items_found = true;
                                            if let Some(provider) = &background_llm {
                                                info!("Processing {} new articles with LLM...", article_ids.len());
                                                // Spawn processing in background or run here?
                                                // For simplicity in this worker loop, we await it, but we might want to spawn it.
                                                // Given we have concurrency limit on fetches, maybe awaiting is fine or spawn.
                                                // Let's spawn to not block the fetch slots, but we need to clone the provider.
                                                let provider = provider.clone(); // Clone the Arc
                                                let pool = _db_pool.clone();
                                                
                                                // Extract model name for processing
                                                let model = config.llm.as_ref()
                                                    .and_then(|l| l.remote.as_ref())
                                                    .and_then(|r| r.model.as_deref())
                                                    .unwrap_or("unknown")
                                                    .to_string();

                                                tokio::spawn(async move {
                                                    if let Err(e) = newscope::processing::batch_process_articles(
                                                        &pool,
                                                        &article_ids,
                                                        provider,
                                                        &model
                                                    ).await {
                                                        error!("Error processing articles: {:?}", e);
                                                    }
                                                });
                                            }
                                        }
                                    }
                                    Err(e) => error!("worker: failed to store items for feed {}: {}", feed_id, e),
                                }
                                
                                // Adaptive scheduling update
                                if adaptive {
                                    if new_items_found {
                                        interval = (interval / 2).max(15);
                                    } else {
                                        interval = (interval + (interval / 2)).min(1440);
                                    }
                                }
                                
                                // Update next_poll_at
                                let next_poll = Utc::now() + chrono::Duration::minutes(interval);
                                let _ = sqlx::query(
                                    "UPDATE feeds SET next_poll_at = ?, poll_interval_minutes = ?, last_checked = ? WHERE id = ?"
                                )
                                .bind(next_poll)
                                .bind(interval)
                                .bind(Utc::now())
                                .bind(feed_id)
                                .execute(&*_db_pool)
                                .await;
                            }
                            Err(e) => {
                                error!("worker: failed to fetch feed {}: {}", feed_id, e);
                                
                                // Scheduler Backoff: Double the interval to avoid spamming a failing feed
                                // Cap at 24 hours (1440 minutes)
                                let new_interval = (interval * 2).min(1440);
                                info!("worker: feed {} failed, backing off interval from {} to {} minutes", feed_id, interval, new_interval);
                                
                                let next_poll = Utc::now() + chrono::Duration::minutes(new_interval);
                                let _ = sqlx::query(
                                    "UPDATE feeds SET next_poll_at = ?, poll_interval_minutes = ? WHERE id = ?"
                                )
                                    .bind(next_poll)
                                    .bind(new_interval)
                                    .bind(feed_id)
                                    .execute(&*_db_pool)
                                    .await;
                            }
                        }
                    }
                }
            }
            Err(e) => error!("worker: failed to query feeds: {}", e),
        }

        // 4. Process missing embeddings (Phase 1)
        if let Some(provider) = &background_llm {
            let provider = provider.clone();
            let pool = _db_pool.clone();
            let model = config.llm.as_ref()
                .and_then(|l| l.remote.as_ref())
                .and_then(|r| r.model.as_deref())
                .unwrap_or("unknown")
                .to_string();

            // Spawn to avoid blocking the loop
            tokio::spawn(async move {
                if let Err(e) = newscope::processing::process_missing_embeddings(
                    &pool,
                    provider,
                    &model, 
                    20 // Limit batch size for embeddings
                ).await {
                     error!("Error processing embeddings: {:?}", e);
                }
            });
        }

        select! {
            _ = tokio::time::sleep(Duration::from_secs(60)) => {
                // Loop again
            },
            _ = shutdown_notify.notified() => {
                info!("worker: shutdown requested, exiting loop");
                break;
            }
        }
    }

    info!("worker: cleanup complete");
    Ok(())
}
