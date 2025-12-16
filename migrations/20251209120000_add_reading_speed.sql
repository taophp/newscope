-- Add reading_speed column to user_preferences
ALTER TABLE user_preferences ADD COLUMN reading_speed INTEGER DEFAULT 250;
