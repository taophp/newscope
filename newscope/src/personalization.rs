use serde::{Deserialize, Serialize};

/// User profile for personalization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: i64,
    pub language: String,
    pub complexity_level: String,
    pub interests: Vec<String>,
    pub preferred_categories: Vec<String>,
    pub keyword_boosts: std::collections::HashMap<String, f32>,
}

/// Relevance evaluation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevanceEvaluation {
    pub score: f32,  // 0.0 to 1.0
    pub reasons: Vec<String>,
}

/// Personalized summary for an article
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalizedSummary {
    pub headline: String,
    pub bullets: Vec<String>,
    pub details: Option<String>,
    pub length: String,  // "short", "medium", "long"
    pub usage: crate::llm::UsageMetadata,
}

/// Database row for user_article_summaries
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserArticleSummaryRow {
    pub id: i64,
    pub user_id: i64,
    pub article_id: i64,
    pub relevance_score: f64,
    pub relevance_reasons: Option<String>,
    pub is_relevant: bool,
    pub personalized_headline: String,
    pub personalized_bullets: String,  // JSON
    pub personalized_details: Option<String>,
    pub language: String,
    pub complexity_level: Option<String>,
    pub summary_length: Option<String>,
    pub created_at: String,
    pub llm_model: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
}

impl UserArticleSummaryRow {
    /// Parse bullets from JSON string
    pub fn get_bullets(&self) -> Vec<String> {
        serde_json::from_str(&self.personalized_bullets).unwrap_or_default()
    }

    /// Parse relevance reasons from JSON string
    pub fn get_reasons(&self) -> Vec<String> {
        self.relevance_reasons
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
}
