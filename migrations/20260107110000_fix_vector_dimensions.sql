-- Migration: Fix Vector Dimensions
-- Recreate vec_articles and vec_users with 384 dimensions to match all-minilm

DROP TABLE IF EXISTS vec_articles;
CREATE VIRTUAL TABLE vec_articles USING vec0(
  article_id INTEGER PRIMARY KEY,
  embedding FLOAT[384]
);

DROP TABLE IF EXISTS vec_users;
CREATE VIRTUAL TABLE vec_users USING vec0(
  user_id INTEGER PRIMARY KEY,
  embedding FLOAT[384]
);
