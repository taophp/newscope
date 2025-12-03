-- Initial schema migration recreated from existing database
-- This migration consolidates all tables from the original MyNewsLens/Newscope database

-- Users table: stores user accounts with authentication
CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT,
    password_hash TEXT,
    prefs_json TEXT,
    created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    last_login TIMESTAMP
);

-- Feeds table: RSS/Atom feeds being monitored
CREATE TABLE feeds (
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

-- Subscriptions table: links users to feeds they follow
CREATE TABLE subscriptions (
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

-- Articles table: deduplicated articles from all feeds
CREATE TABLE articles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    canonical_url TEXT,
    title TEXT,
    content TEXT,
    full_content TEXT,
    published_at TIMESTAMP,
    first_seen_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    canonical_hash TEXT,
    processing_status TEXT DEFAULT 'pending',
    processed_at TIMESTAMP
);

-- Article occurrences: tracks which feeds an article appeared in
CREATE TABLE article_occurrences (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    article_id INTEGER NOT NULL,
    feed_id INTEGER NOT NULL,
    feed_item_id TEXT,
    discovered_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    FOREIGN KEY(article_id) REFERENCES articles(id) ON DELETE CASCADE,
    FOREIGN KEY(feed_id) REFERENCES feeds(id) ON DELETE CASCADE
);

-- Article summaries: LLM-generated summaries for articles
CREATE TABLE article_summaries (
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

-- LLM usage log: tracks LLM API usage for monitoring
CREATE TABLE llm_usage_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation TEXT,
    model TEXT,
    prompt_tokens INTEGER,
    completion_tokens INTEGER,
    success BOOLEAN,
    error_message TEXT,
    created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Sessions table: chat sessions with time-boxed news exploration
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    start_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    duration_requested_seconds INTEGER,
    digest_summary_id INTEGER,
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
);

-- Chat messages: conversation history within sessions
CREATE TABLE chat_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL,
    author TEXT NOT NULL,
    message TEXT,
    created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
);

-- Summaries table: digest summaries for sessions
CREATE TABLE summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER,
    summary_text TEXT,
    by_model TEXT,
    tokens_used INTEGER,
    created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
);
