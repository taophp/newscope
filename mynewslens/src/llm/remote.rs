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

        Ok(LlmResponse {
            content: choice.message.content.clone(),
            usage: UsageMetadata {
                prompt_tokens: resp_body.usage.prompt_tokens,
                completion_tokens: resp_body.usage.completion_tokens,
                total_tokens: resp_body.usage.total_tokens,
            },
            model: resp_body.model,
        })
    }

    async fn summarize(&self, content: &str, max_tokens: usize) -> Result<Summary> {
        let prompt = format!(
            r#"Summarize the following article in a hierarchical format.

Provide your response as JSON with this exact structure:
{{
  "headline": "one-line summary (max 100 chars)",
  "bullets": ["key point 1", "key point 2", "key point 3"],
  "details": "optional expanded context"
}}

Use 3-7 bullet points. Be concise and informative.

Article:
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

        // Try to parse as JSON
        let summary_data: SummaryJson = serde_json::from_str(&response.content)
            .context("Failed to parse LLM summary as JSON")?;

        Ok(Summary {
            headline: summary_data.headline,
            bullets: summary_data.bullets,
            details: summary_data.details,
            usage: response.usage,
        })
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
    model: String,
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: usize,
    completion_tokens: usize,
    total_tokens: usize,
}

// Internal structure for parsing summary JSON
#[derive(Debug, Deserialize)]
struct SummaryJson {
    headline: String,
    bullets: Vec<String>,
    details: Option<String>,
}
