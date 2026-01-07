// Summarizer module
use tracing::{info, warn};

use super::{LlmProvider, Summary, UsageMetadata};

/// Generate hierarchical summary with fallback to extractive summary (FR-LLM-04)
pub async fn summarize_article<P: LlmProvider + ?Sized>(
    provider: &P,
    article_text: &str,
    max_tokens: usize,
) -> Summary {
    match provider.summarize(article_text, max_tokens).await {
        Ok(summary) => {
            info!(
                "LLM summarization successful: {} bullets, {} tokens",
                summary.bullets.len(),
                summary.usage.total_tokens
            );
            summary
        }
        Err(e) => {
            warn!("LLM summarization failed: {}, falling back to extractive summary", e);
            extractive_summary(article_text)
        }
    }
}

/// Fallback extractive summary when LLM fails
fn extractive_summary(text: &str) -> Summary {
    let sentences: Vec<&str> = text
        .split(['.', '!', '?'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    let headline = sentences
        .first()
        .map(|s| truncate(s, 100))
        .unwrap_or_else(|| "No content".to_string());

    let bullets = sentences
        .iter()
        .skip(1)
        .take(5)
        .map(|s| truncate(s, 200))
        .collect();

    Summary {
        headline,
        bullets,
        details: Some(text.chars().take(1000).collect()),
        usage: UsageMetadata::default(),
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extractive_summary() {
        let text = "First sentence is the headline. Second sentence is a bullet. \
                    Third sentence is another bullet. Fourth is yet another. \
                    Fifth sentence here. Sixth and final.";

        let summary = extractive_summary(text);

        assert_eq!(summary.headline, "First sentence is the headline");
        assert_eq!(summary.bullets.len(), 5);
        assert_eq!(summary.bullets[0], "Second sentence is a bullet");
        assert!(summary.details.is_some());
    }

    #[test]
    fn test_extractive_summary_truncation() {
        let long_sentence = "a".repeat(150);
        let text = format!("{}. Second sentence.", long_sentence);

        let summary = extractive_summary(&text);

        assert!(summary.headline.len() <= 103); // 100 + "..."
        assert!(summary.headline.ends_with("..."));
    }
}
