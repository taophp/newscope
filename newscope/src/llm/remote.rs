use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{LlmProvider, LlmRequest, LlmResponse, Summary, UsageMetadata};

/// Remote LLM provider using OpenAI-compatible HTTP API
pub struct RemoteLlmProvider {
    base_url: String,
    api_key: String,
    model: String,
    default_timeout: Duration,
    default_max_tokens: usize,
    default_temperature: f32,
    client: reqwest::Client,
}

impl RemoteLlmProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            default_timeout: Duration::from_secs(30),
            default_max_tokens: 500,
            default_temperature: 0.7,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_defaults(
        mut self,
        timeout_secs: u64,
        max_tokens: usize,
        temperature: f32,
    ) -> Self {
        self.default_timeout = Duration::from_secs(timeout_secs);
        self.default_max_tokens = max_tokens;
        self.default_temperature = temperature;
        self
    }
}

#[async_trait::async_trait]
impl LlmProvider for RemoteLlmProvider {
    async fn generate(&self, request: LlmRequest) -> Result<LlmResponse> {
        let timeout = request
            .timeout_seconds
            .map(Duration::from_secs)
            .unwrap_or(self.default_timeout);

        let max_tokens = request.max_tokens.unwrap_or(self.default_max_tokens);
        let temperature = request.temperature.unwrap_or(self.default_temperature);

        // Build OpenAI-compatible request
        let req_body = OpenAiRequest {
            model: self.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: request.prompt,
            }],
            max_tokens: Some(max_tokens),
            temperature: Some(temperature),
        };

        // Make HTTP request with timeout
        let response = tokio::time::timeout(
            timeout,
            self.client
                .post(&self.base_url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&req_body)
                .send(),
        )
        .await
        .context("LLM request timed out")?
        .context("LLM HTTP request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("LLM API error {}: {}", status, body);
        }

        let resp_body: OpenAiResponse = response
            .json()
            .await
            .context("Failed to parse LLM response")?;

        let choice = resp_body
            .choices
            .first()
            .context("LLM response has no choices")?;

        let usage = UsageMetadata {
            prompt_tokens: resp_body.usage.prompt_tokens.unwrap_or(0),
            completion_tokens: resp_body.usage.completion_tokens.unwrap_or(0),
            total_tokens: resp_body.usage.total_tokens.unwrap_or(0),
        };

        Ok(LlmResponse {
            content: choice.message.content.clone(),
            usage,
            model: resp_body.model.unwrap_or_else(|| self.model.clone()),
        })
    }

    async fn summarize(&self, content: &str, max_tokens: usize) -> Result<Summary> {
        let prompt = format!(
            r#"You are a news article summarizer. Create a concise, informative summary.

IMPORTANT INSTRUCTIONS:
1. IGNORE all markdown formatting (###, **, __, etc.) - extract only text content
2. Create a REAL summary of the key points (not just the first few lines)
3. Be concise but capture the essential information from the ENTIRE article
4. KEEP THE ORIGINAL LANGUAGE - do not translate (translation happens later)

OUTPUT FORMAT (strict JSON):
{{
  "headline": "one-line summary in original language (max 100 chars)",
  "bullets": ["key point 1", "key point 2", "key point 3"],
  "details": "optional additional context"
}}

Use 3-7 bullet points that capture the most important information.

ARTICLE TO SUMMARIZE:
{}
"#,
            content
        );

        let request = LlmRequest {
            prompt,
            max_tokens: Some(max_tokens),
            temperature: Some(0.5), // Lower temperature for more consistent summarization
            timeout_seconds: None,
        };

        let response = self.generate(request).await?;

        // Robust JSON extraction: handle markdown backticks, preamble, etc.
        let cleaned_json = super::extract_json_from_text(&response.content)
            .context("No valid JSON found in LLM summary response")?;

        let summary_data: SummaryJson = serde_json::from_str(&cleaned_json)
            .context(format!("Failed to parse LLM summary as JSON. Input was: {}", cleaned_json))?;

        Ok(Summary {
            headline: summary_data.headline,
            bullets: summary_data.bullets,
            details: summary_data.details,
            usage: response.usage,
        })
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // Infer embedding URL from base_url (chat endpoint)
        // e.g. http://localhost:11434/v1/chat/completions -> http://localhost:11434/v1/embeddings
        let embedding_url = if self.base_url.ends_with("/embeddings") {
            self.base_url.clone()
        } else if self.base_url.ends_with("/chat/completions") {
            self.base_url.replace("/chat/completions", "/embeddings")
        } else if self.base_url.ends_with("/completions") {
             self.base_url.replace("/completions", "/embeddings")
        } else {
            // Fallback: assume base_url is the root, append /embeddings? 
            // Or just try to append /embeddings if it ends in /v1
            if self.base_url.ends_with("/v1") {
                format!("{}/embeddings", self.base_url)
            } else {
                 // Risky assumption but standard for many
                 format!("{}/embeddings", self.base_url.trim_end_matches('/'))
            }
        };

        let req_body = EmbeddingRequest {
            model: self.model.clone(),
            input: text.to_string(),
        };

        let response = tokio::time::timeout(
            self.default_timeout,
            self.client
                .post(&embedding_url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&req_body)
                .send(),
        )
        .await
        .context("Embedding request timed out")?
        .context("Embedding HTTP request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Embedding API error {}: {} (URL: {})", status, body, embedding_url);
        }

        let body_text = response.text().await.context("Failed to read embedding response body")?;
        
        // Try parsing as standard OpenAI response
        match serde_json::from_str::<EmbeddingResponse>(&body_text) {
            Ok(resp_body) => {
                if let Some(first) = resp_body.data.first() {
                    return Ok(first.embedding.clone());
                }
            }
            Err(e) => {
                // Fallback: try parsing as a raw list of floats (some old/direct providers do this)
                if let Ok(raw_vec) = serde_json::from_str::<Vec<f32>>(&body_text) {
                    return Ok(raw_vec);
                }
                // Fallback: try parsing as a single embedding object
                #[derive(Deserialize)] struct SingleEmbed { embedding: Vec<f32> }
                if let Ok(single) = serde_json::from_str::<SingleEmbed>(&body_text) {
                    return Ok(single.embedding);
                }
                
                anyhow::bail!("Failed to parse Embedding response: {} (Body: {})", e, body_text);
            }
        }

        anyhow::bail!("Embedding response has no data: {}", body_text);
    }
}

// OpenAI API request/response structures
#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    model: Option<String>,
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: Option<usize>,
    #[serde(default)]
    completion_tokens: Option<usize>,
    #[serde(default)]
    total_tokens: Option<usize>,
}

// Internal structure for parsing summary JSON
#[derive(Debug, Deserialize)]
struct SummaryJson {
    headline: String,
    bullets: Vec<String>,
    details: Option<String>,
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    model: String,
    input: String,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    #[serde(default)]
    object: Option<String>, // "embedding"
    embedding: Vec<f32>,
    #[serde(default)]
    index: Option<usize>,
}
