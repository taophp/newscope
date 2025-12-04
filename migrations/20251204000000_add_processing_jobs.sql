-- Migration: Add processing jobs tracking and locks
-- This enables monitoring of background LLM processing tasks

-- Table for tracking processing jobs
CREATE TABLE IF NOT EXISTS processing_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_type TEXT NOT NULL,              -- 'article_summary', 'feed_fetch', etc.
    entity_id INTEGER,                    -- article_id or feed_id
    status TEXT NOT NULL,                 -- 'pending', 'running', 'completed', 'failed'
    started_at TIMESTAMP,
    completed_at TIMESTAMP,
    error_message TEXT,
    llm_model TEXT,
    prompt_tokens INTEGER,
    completion_tokens INTEGER,
    processing_time_ms INTEGER,
    created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_processing_jobs_status ON processing_jobs(status);
CREATE INDEX IF NOT EXISTS idx_processing_jobs_type_entity ON processing_jobs(job_type, entity_id);
CREATE INDEX IF NOT EXISTS idx_processing_jobs_created ON processing_jobs(created_at);

-- Table for preventing concurrent processing of same entity
CREATE TABLE IF NOT EXISTS processing_locks (
    lock_key TEXT PRIMARY KEY,
    acquired_at TIMESTAMP NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    worker_id TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_processing_locks_expires ON processing_locks(expires_at);

-- Clean up expired locks (will be called periodically)
-- DELETE FROM processing_locks WHERE expires_at < datetime('now');
