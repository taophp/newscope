use newscope::llm::remote::RemoteLlmProvider;
use newscope::llm::{LlmProvider, LlmRequest};

#[tokio::test]
async fn test_remote_provider_with_mock() {
    let mut server = mockito::Server::new_async().await;

    // Mock successful OpenAI response
    let mock = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
                "model": "gpt-4o-mini",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "This is a test response"
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "total_tokens": 15
                }
            }"#,
        )
        .create_async()
        .await;

    let provider = RemoteLlmProvider::new(server.url(), "fake-api-key", "gpt-4o-mini");

    let request = LlmRequest {
        prompt: "Test prompt".to_string(),
        max_tokens: Some(100),
        temperature: Some(0.7),
        timeout_seconds: Some(10),
    };

    let result = provider.generate(request).await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.content, "This is a test response");
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 5);
    assert_eq!(response.usage.total_tokens, 15);
    assert_eq!(response.model, "gpt-4o-mini");

    mock.assert_async().await;
}

#[tokio::test]
async fn test_remote_provider_summarize_with_mock() {
    let mut server = mockito::Server::new_async().await;

    // Mock summary response
    let mock = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{
                "model": "gpt-4o-mini",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "{\n  \"headline\": \"Test Article Summary\",\n  \"bullets\": [\"Point 1\", \"Point 2\", \"Point 3\"],\n  \"details\": \"Additional context about the article\"\n}"
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 50,
                    "total_tokens": 150
                }
            }"#,
        )
        .create_async()
        .await;

    let provider = RemoteLlmProvider::new(server.url(), "fake-api-key", "gpt-4o-mini");

    let result = provider
        .summarize("Long article content here...", 200)
        .await;

    assert!(result.is_ok());
    let summary = result.unwrap();
    assert_eq!(summary.headline, "Test Article Summary");
    assert_eq!(summary.bullets.len(), 3);
    assert_eq!(summary.bullets[0], "Point 1");
    assert!(summary.details.is_some());
    assert_eq!(summary.usage.total_tokens, 150);

    mock.assert_async().await;
}

#[tokio::test]
async fn test_remote_provider_error_handling() {
    let mut server = mockito::Server::new_async().await;

    // Mock API error
    let mock = server
        .mock("POST", "/")
        .with_status(429)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error": {"message": "Rate limit exceeded"}}"#)
        .create_async()
        .await;

    let provider = RemoteLlmProvider::new(server.url(), "fake-api-key", "gpt-4o-mini");

    let request = LlmRequest {
        prompt: "Test".to_string(),
        max_tokens: None,
        temperature: None,
        timeout_seconds: None,
    };

    let result = provider.generate(request).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("429"));

    mock.assert_async().await;
}

#[tokio::test]
async fn test_remote_provider_timeout() {
    let mut server = mockito::Server::new_async().await;

    // Mock slow response
    let _mock = server
        .mock("POST", "/")
        .with_status(200)
        .with_chunked_body(|w| {
            std::thread::sleep(std::time::Duration::from_secs(3));
            w.write_all(b"too late")
        })
        .create_async()
        .await;

    let provider = RemoteLlmProvider::new(server.url(), "fake-api-key", "gpt-4o-mini");

    let request = LlmRequest {
        prompt: "Test".to_string(),
        max_tokens: None,
        temperature: None,
        timeout_seconds: Some(1), // 1 second timeout
    };

    let result = provider.generate(request).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("timed out"));
}
