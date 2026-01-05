-- Migration: Normalize user data into user_profiles
-- Consolidate singleton settings from user_preferences and remove redundant users.prefs_json

-- 1. Create the new user_profiles table for singleton settings
CREATE TABLE IF NOT EXISTS user_profiles (
    user_id INTEGER PRIMARY KEY,
    language TEXT NOT NULL DEFAULT 'en',
    complexity_level TEXT NOT NULL DEFAULT 'medium',
    reading_speed INTEGER NOT NULL DEFAULT 250,
    interests TEXT, -- JSON array
    updated_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
);

-- 2. Migrate existing data from user_preferences to user_profiles
-- We pick the first available 'profile' type preference if multiple exist (unlikely but safe)
INSERT INTO user_profiles (user_id, language, complexity_level, reading_speed, interests)
SELECT 
    user_id, 
    COALESCE(language, 'en'), 
    COALESCE(complexity_level, 'medium'), 
    COALESCE(reading_speed, 250), 
    interests
FROM user_preferences
GROUP BY user_id;

-- 3. Cleanup user_preferences
-- In SQLite, we have to recreate the table to drop columns properly
CREATE TABLE user_preferences_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    preference_type TEXT NOT NULL,  -- 'category_filter', 'keyword_boost', 'source_weight'
    preference_key TEXT NOT NULL,   
    preference_value REAL NOT NULL, 
    created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
    UNIQUE(user_id, preference_type, preference_key)
);

-- Copy only the granular preferences back (exclude the ones that were just profile markers)
INSERT INTO user_preferences_new (user_id, preference_type, preference_key, preference_value, created_at, updated_at)
SELECT user_id, preference_type, preference_key, preference_value, created_at, updated_at
FROM user_preferences
WHERE preference_type != 'profile';

DROP TABLE user_preferences;
ALTER TABLE user_preferences_new RENAME TO user_preferences;
CREATE INDEX idx_user_preferences_user ON user_preferences(user_id);

-- 4. Cleanup users table (drop prefs_json)
CREATE TABLE users_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT,
    password_hash TEXT,
    created_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    last_login TIMESTAMP
);

INSERT INTO users_new (id, username, display_name, password_hash, created_at, last_login)
SELECT id, username, display_name, password_hash, created_at, last_login FROM users;

DROP TABLE users;
ALTER TABLE users_new RENAME TO users;
