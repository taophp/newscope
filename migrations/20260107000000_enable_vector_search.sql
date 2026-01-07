-- Migration: Enable Vector Search (sqlite-vec)
-- This enables native vector storage and similarity search using the vec0 table format

-- 1. Create a virtual table for article embeddings
-- We use FLOAT for embeddings and 1024 dimensions by default
-- Most modern embedding models (e.g. mxbai-embed-large, jina-embeddings-v3) fit or can be projected to this.
-- If using a 1536-dim model like OpenAI text-embedding-3-small, this migration might need adjustment,
-- but for local models 768 or 1024 is the standard.
CREATE VIRTUAL TABLE IF NOT EXISTS vec_articles USING vec0(
  article_id INTEGER PRIMARY KEY,
  embedding FLOAT[1024]
);

-- 2. Optional: Transfer existing embeddings (Logic removed as article_embeddings table is deprecated/missing)

-- 3. Update the processing_jobs definition to track embedding quality (optional but good for debugging)
-- (No schema change needed for jobs table, we'll just log more)
