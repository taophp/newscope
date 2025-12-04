-- Migration: Add user preferences and article categories
-- This enables smart filtering and personalized press review generation

-- Table for user preferences (category filters, keyword boosts, etc.)
CREATE TABLE IF NOT EXISTS user_preferences (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    preference_type TEXT NOT NULL,  -- 'category_filter', 'keyword_boost', 'source_weight'
    preference_key TEXT NOT NULL,   -- 'faits_divers', 'technology', 'lemonde.fr'
    preference_value REAL NOT NULL, -- -1.0 (block), 0.0 (neutral), 1.0 (boost)
    created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
    UNIQUE(user_id, preference_type, preference_key)
);

CREATE INDEX IF NOT EXISTS idx_user_preferences_user ON user_preferences(user_id);
CREATE INDEX IF NOT EXISTS idx_user_preferences_type ON user_preferences(preference_type);

-- Add categories column to article_summaries (JSON array of category strings)
-- NOTE: This column is already created in 20250101000000_initial_schema.sql
-- So we don't need to add it again here
-- ALTER TABLE article_summaries ADD COLUMN categories TEXT;

-- Example categories: politics, economy, technology, sports, culture, science,
-- local_news, international, faits_divers, health, environment
