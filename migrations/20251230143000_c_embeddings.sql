-- Add article_embeddings table for semantic search
CREATE TABLE IF NOT EXISTS article_embeddings (
    article_id INTEGER PRIMARY KEY REFERENCES articles(id) ON DELETE CASCADE,
    embedding BLOB NOT NULL,     -- Stores Vec<f32> as raw bytes (Little Endian)
    model TEXT NOT NULL,         -- Track model version (e.g. "nomic-embed-text")
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
