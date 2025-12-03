#[path = "../llm/mod.rs"]
mod llm;

use llm::remote::RemoteLlmProvider;
use llm::LlmProvider;

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let api_key = std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .expect("Set OPENAI_API_KEY or ANTHROPIC_API_KEY environment variable");

    // Allow custom base URL or use OpenAI default
    let base_url = std::env::var("LLM_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());
    
    let model = std::env::var("LLM_MODEL")
        .unwrap_or_else(|_| "gpt-4o-mini".to_string());

    println!("\n{}", "=".repeat(60));
    println!("Testing LLM Provider");
    println!("Base URL: {}", base_url);
    println!("Model: {}", model);
    println!("{}", "=".repeat(60));

    let provider = RemoteLlmProvider::new(&base_url, &api_key, &model)
        .with_defaults(30, 500, 0.7);

    // Test 1: Basic summarization
    let test_article = r#"
Rust is a systems programming language that runs blazingly fast, prevents 
segfaults, and guarantees thread safety. It accomplishes these goals through 
a unique ownership system that enforces memory safety without requiring a 
garbage collector.

The Rust compiler provides helpful error messages and suggestions, making it 
easier to write correct code. The language has a growing ecosystem of libraries 
called "crates" available through Cargo, Rust's package manager.

Many companies are adopting Rust for critical infrastructure, including 
Mozilla, Dropbox, and Microsoft. The language's performance and safety 
guarantees make it ideal for operating systems, web servers, and embedded systems.
    "#;

    println!("\n[Test 1] Summarizing article...");
    match provider.summarize(test_article, 300).await {
        Ok(summary) => {
            println!("✓ Success!");
            println!("  Headline: {}", summary.headline);
            println!("  Bullets ({} items):", summary.bullets.len());
            for (i, bullet) in summary.bullets.iter().enumerate() {
                println!("    {}. {}", i + 1, bullet);
            }
            if let Some(details) = &summary.details {
                println!("  Details: {}...", &details.chars().take(100).collect::<String>());
            }
            println!("  Usage: {} tokens (prompt: {}, completion: {})",
                summary.usage.total_tokens,
                summary.usage.prompt_tokens,
                summary.usage.completion_tokens
            );
        }
        Err(e) => {
            eprintln!("✗ Failed: {}", e);
        }
    }

    // Test 2: Short article (should still work)
    let short_article = "Rust 1.70 was released today with new features.";
    
    println!("\n[Test 2] Summarizing short article...");
    match provider.summarize(short_article, 200).await {
        Ok(summary) => {
            println!("✓ Success!");
            println!("  Headline: {}", summary.headline);
            println!("  Bullets: {:?}", summary.bullets);
            println!("  Tokens: {}", summary.usage.total_tokens);
        }
        Err(e) => {
            eprintln!("✗ Failed: {}", e);
        }
    }

    println!("\n{}", "=".repeat(60));
    println!("Tests completed");
    println!("{}", "=".repeat(60));
}
