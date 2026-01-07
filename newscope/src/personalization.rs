use serde::{Deserialize, Serialize};

/// User profile for personalization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: i64,
    pub language: String,
    pub complexity_level: String,
    pub reading_speed: i32, // Words per minute
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

use anyhow::{Context, Result};
use sqlx::{SqlitePool, Row};
use crate::llm::{LlmProvider, LlmRequest};

/// Evaluate article relevance for a specific user
pub async fn evaluate_article_relevance(
    llm: &dyn LlmProvider,
    summary: &crate::llm::Summary,
    user: &UserProfile,
) -> Result<RelevanceEvaluation> {
    let interests_str = if user.interests.is_empty() {
        "general news".to_string()
    } else {
        user.interests.join(", ")
    };

    let categories_str = if user.preferred_categories.is_empty() {
        "all topics".to_string()
    } else {
        user.preferred_categories.join(", ")
    };

    let prompt = format!(
        "Evaluate if this article is relevant for a user interested in: {}

Article: {}
Key points: {}

User interests: {}
Preferred categories: {}

Rate relevance (0.0-1.0) and explain why in 1-2 sentences.
Return ONLY valid JSON: {{\"score\": 0.8, \"reasons\": [\"matches interest in AI\", \"recent topic\"]}}",
        interests_str,
        summary.headline,
        summary.bullets.join(", "),
        interests_str,
        categories_str,
    );

    let response = llm.generate(LlmRequest {
        prompt,
        max_tokens: Some(200),
        temperature: Some(0.3),
        timeout_seconds: Some(15),
    }).await.context("Failed to generate relevance evaluation")?;

    // Parse JSON response with robustness
    let cleaned_content = crate::llm::extract_json_from_text(&response.content).unwrap_or(response.content.clone());
    match serde_json::from_str::<RelevanceEvaluation>(&cleaned_content) {
        Ok(eval) => Ok(eval),
        Err(_) => {
            // Fallback: default moderate relevance if parsing fails
            tracing::warn!("Failed to parse relevance JSON, using default: {}", response.content);
            Ok(RelevanceEvaluation {
                score: 0.5,
                reasons: vec!["Unable to evaluate".to_string()],
            })
        }
    }
}

/// Generate personalized summary adapted to user profile
pub async fn generate_personalized_summary(
    llm: &dyn LlmProvider,
    generic: &crate::llm::Summary,
    user: &UserProfile,
    relevance: f32,
) -> Result<PersonalizedSummary> {
    // Determine target length based on relevance
    let (target_bullets, length_str) = match relevance {
        r if r > 0.8 => (5, "long"),
        r if r > 0.5 => (3, "medium"),
        _ => (2, "short"),
    };

    let interests_context = if user.interests.is_empty() {
        String::new()
    } else {
        format!("- Focus on aspects relevant to: {}\n", user.interests.join(", "))
    };

    let prompt = format!(
        "Adapt this article summary for a {} speaker with {} complexity level.

Original headline: {}
Key points: {}

Instructions:
- Language: {} (respond entirely in this language)
- Complexity: {} (adjust vocabulary and detail accordingly)
- Target length: {} key points
{}
Return ONLY valid JSON:
{{
  \"headline\": \"adapted headline in {}\",
  \"bullets\": [\"point 1 in {}\", \"point 2\", \"...\"],
  \"details\": \"optional additional context\"
}}",
        user.language,
        user.complexity_level,
        generic.headline,
        generic.bullets.join("\n- "),
        user.language,
        user.complexity_level,
        target_bullets,
        interests_context,
        user.language,
        user.language,
    );

    let response = llm.generate(LlmRequest {
        prompt,
        max_tokens: Some(1000),
        temperature: Some(0.7),
        timeout_seconds: Some(30),
    }).await.context("Failed to generate personalized summary")?;

    // Parse JSON response
    #[derive(Deserialize)]
    struct PersonalizedJson {
        headline: String,
        bullets: Vec<String>,
        details: Option<String>,
    }

    // Parse JSON response with robustness
    let cleaned_content = crate::llm::extract_json_from_text(&response.content).unwrap_or(response.content.clone());
    match serde_json::from_str::<PersonalizedJson>(&cleaned_content) {
        Ok(json) => Ok(PersonalizedSummary {
            headline: json.headline,
            bullets: json.bullets,
            details: json.details,
            length: length_str.to_string(),
            usage: response.usage,
        }),
        Err(_) => {
            // Fallback: use generic summary if parsing fails
            tracing::warn!("Failed to parse personalized JSON, using generic");
            Ok(PersonalizedSummary {
                headline: generic.headline.clone(),
                bullets: generic.bullets.clone(),
                details: generic.details.clone(),
                length: "medium".to_string(),
                usage: response.usage,
            })
        }
    }
}

/// Fetch user profile from database
pub async fn get_user_profile(pool: &SqlitePool, user_id: i64) -> Result<UserProfile> {
    let row = sqlx::query(
        "SELECT 
            u.id,
            COALESCE(up.language, 'en') as language,
            COALESCE(up.complexity_level, 'medium') as complexity_level,
            COALESCE(up.reading_speed, 250) as reading_speed,
            up.interests
         FROM users u
         LEFT JOIN user_profiles up ON u.id = up.user_id
         WHERE u.id = ?"
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("Failed to fetch user profile")?
    .ok_or_else(|| anyhow::anyhow!("User {} not found", user_id))?;

    let id: i64 = row.get("id");
    let language: String = row.get("language");
    let complexity_level: String = row.get("complexity_level");
    let reading_speed: i32 = row.get("reading_speed");

    let interests: Vec<String> = row
        .try_get::<String, _>("interests")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    // Preferred categories and keyword boosts from user_preferences table
    let prefs = sqlx::query(
        "SELECT preference_type, preference_key, preference_value FROM user_preferences WHERE user_id = ?"
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut preferred_categories = Vec::new();
    let mut keyword_boosts = std::collections::HashMap::new();

    for pref in prefs {
        let p_type: String = pref.get("preference_type");
        let p_key: String = pref.get("preference_key");
        let p_val: f32 = pref.get("preference_value");

        match p_type.as_str() {
            "category_filter" if p_val > 0.0 => preferred_categories.push(p_key),
            "keyword_boost" => { keyword_boosts.insert(p_key, p_val); },
            _ => {}
        }
    }

    Ok(UserProfile {
        id,
        language,
        complexity_level,
        reading_speed,
        interests,
        preferred_categories,
        keyword_boosts,
    })
}

/// Fetch user interest vector from vec_users table
pub async fn get_user_vector(pool: &SqlitePool, user_id: i64) -> Result<Option<Vec<f32>>> {
    let row = sqlx::query(
        "SELECT embedding FROM vec_users WHERE user_id = ?"
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        let bytes: Vec<u8> = r.get("embedding");
        bytes.chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect()
    }))
}

/// Helper to convert Vec<f32> to bytes for BLOB storage
fn f32_vec_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Update or insert user interest vector
pub async fn update_user_vector(pool: &SqlitePool, user_id: i64, embedding: &[f32]) -> Result<()> {
    let bytes = f32_vec_to_bytes(embedding);
    sqlx::query(
        "INSERT OR REPLACE INTO vec_users (user_id, embedding) VALUES (?, ?)"
    )
    .bind(user_id)
    .bind(bytes)
    .execute(pool)
    .await?;
    Ok(())
}
