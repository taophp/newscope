-- Migration: User Vectorization
-- This enables native vector storage and similarity search for user interest profiles

-- 1. Create a virtual table for user vectors
-- We use FLOAT for embeddings and 1024 dimensions to match article embeddings
CREATE VIRTUAL TABLE IF NOT EXISTS vec_users USING vec0(
  user_id INTEGER PRIMARY KEY,
  embedding FLOAT[1024]
);
