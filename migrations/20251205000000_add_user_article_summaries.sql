-- Migration: Add user-personalized article summaries
-- This enables pre-computed personalization for fast session generation

-- Table for storing personalized summaries per user
CREATE TABLE IF NOT EXISTS user_article_summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    article_id INTEGER NOT NULL,
    
    -- Relevance evaluation
    relevance_score REAL NOT NULL,          -- 0.0 to 1.0
    relevance_reasons TEXT,                 -- JSON: why relevant/not
    is_relevant BOOLEAN NOT NULL DEFAULT 1, -- Binary filter
    
    -- Personalized summary
    personalized_headline TEXT NOT NULL,
    personalized_bullets TEXT NOT NULL,     -- JSON array
    personalized_details TEXT,
    
    -- Personalization metadata
    language TEXT NOT NULL,                 -- 'en', 'fr', etc.
    complexity_level TEXT,                  -- 'simple', 'medium', 'detailed'
    summary_length TEXT,                    -- 'short', 'medium', 'long'
    
    -- Processing metadata
    created_at TIMESTAMP DEFAULT (datetime('now')),
    llm_model TEXT,
    prompt_tokens INTEGER,
    completion_tokens INTEGER,
    
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (article_id) REFERENCES articles(id) ON DELETE CASCADE,
    UNIQUE(user_id, article_id)
);

CREATE INDEX IF NOT EXISTS idx_user_article_summaries_user 
    ON user_article_summaries(user_id);

CREATE INDEX IF NOT EXISTS idx_user_article_summaries_relevance 
    ON user_article_summaries(user_id, is_relevant, relevance_score DESC);

CREATE INDEX IF NOT EXISTS idx_user_article_summaries_article 
    ON user_article_summaries(article_id);

CREATE INDEX IF NOT EXISTS idx_user_article_summaries_created 
    ON user_article_summaries(user_id, created_at DESC);

-- Extend user_preferences with personalization fields
ALTER TABLE user_preferences ADD COLUMN language TEXT DEFAULT 'en';
ALTER TABLE user_preferences ADD COLUMN complexity_level TEXT DEFAULT 'medium';
ALTER TABLE user_preferences ADD COLUMN interests TEXT; -- JSON array of interest topics
