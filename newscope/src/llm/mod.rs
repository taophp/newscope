use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Core trait for LLM providers (local or remote)
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    /// Generate completion for a given prompt
    async fn generate(&self, request: LlmRequest) -> Result<LlmResponse>;
    
    /// Generate hierarchical summary for article content
    /// Generate hierarchical summary for article content
    async fn summarize(&self, content: &str, max_tokens: usize) -> Result<Summary>;

    /// Generate vector embedding for text
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

/// Request structure for LLM generation
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub prompt: String,
    pub max_tokens: Option<usize>,
    pub temperature: Option<f32>,
    pub timeout_seconds: Option<u64>,
}

/// Response from LLM generation
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub usage: UsageMetadata,
    pub model: String,
}

/// Hierarchical summary structure (FR-LLM-02)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    /// One-line headline summary
    pub headline: String,
    /// 3-7 key bullet points
    pub bullets: Vec<String>,
    /// Optional expanded context/details
    pub details: Option<String>,
    /// Usage metadata for tracking
    #[serde(skip)]
    pub usage: UsageMetadata,
}

/// Token usage metadata (FR-LLM-06)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageMetadata {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

pub mod remote;
pub mod summarizer;
