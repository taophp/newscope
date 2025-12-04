-- Migration: Add user article views tracking
-- This prevents showing the same articles repeatedly in press reviews

CREATE TABLE IF NOT EXISTS user_article_views (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    article_id INTEGER NOT NULL,
    session_id INTEGER,  -- Optional: track which session showed this article
    viewed_at TIMESTAMP DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY(article_id) REFERENCES articles(id) ON DELETE CASCADE,
    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE SET NULL,
    UNIQUE(user_id, article_id)  -- Each user sees each article only once
);

CREATE INDEX IF NOT EXISTS idx_user_article_views_user ON user_article_views(user_id);
CREATE INDEX IF NOT EXISTS idx_user_article_views_article ON user_article_views(article_id);
CREATE INDEX IF NOT EXISTS idx_user_article_views_viewed_at ON user_article_views(viewed_at);
