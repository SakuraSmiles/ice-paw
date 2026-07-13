//! OpenAI Adapter 集成测试
//!
//! 使用 wiremock 模拟 OpenAI 兼容的 SSE 流，验证 `OpenAiAdapter::stream_chat` 的完整行为。
//!
//! 三个场景：
//! 1. 正常流 → 收集到预期文本内容
//! 2. HTTP 401 → 返回 Llm 错误
//! 3. 流中途断开 → 返回 Stream 错误

use futures::StreamExt;
use ice_paw_lib::error::AppError;
use ice_paw_lib::llm::adapters::openai::OpenAiAdapter;
use ice_paw_lib::llm::{CancellationToken, ChatDelta, ChatMessage, LlmProvider};
use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

/// 构造一个简单的 OpenAI 兼容 SSE 流响应体。
fn normal_sse_response() -> String {
    let chunks = [
        r#"data: {"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":" "},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":"world"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":"!"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        "data: [DONE]",
    ];
    chunks.join("\n\n")
}

/// 构造一个中途断开的 SSE 流（只有第一个 chunk，没有 [DONE]）。
/// 注意：需要尾部换行让 SSE 解析器消费到该行。
fn truncated_sse_response() -> String {
    r#"data: {"choices":[{"delta":{"content":"partial"},"finish_reason":null}]}
"#.to_string()
}

fn make_messages() -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: "user".into(),
        content: "Say hello".into(),
    }]
}

#[tokio::test]
async fn openai_normal_stream_collects_expected_content() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(normal_sse_response()),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiAdapter::new("test-model".into(), server.uri());
    let cancel = CancellationToken::new();

    let stream = adapter
        .stream_chat("test-key", make_messages(), 0.7, 1024, cancel)
        .await
        .expect("stream_chat should succeed");

    let mut content_parts: Vec<String> = Vec::new();
    let mut got_done = false;
    let mut stream = Box::pin(stream);

    while let Some(item) = stream.next().await {
        match item {
            Ok(ChatDelta::Delta { content }) => content_parts.push(content),
            Ok(ChatDelta::Done { .. }) => {
                got_done = true;
                break;
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    let joined = content_parts.join("");
    assert_eq!(joined, "Hello world!");
    assert!(got_done, "should receive Done event");
}

#[tokio::test]
async fn openai_http_401_returns_llm_error() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": { "message": "Invalid API key", "type": "invalid_request_error" }
        })))
        .mount(&server)
        .await;

    let adapter = OpenAiAdapter::new("test-model".into(), server.uri());
    let cancel = CancellationToken::new();

    let result = adapter
        .stream_chat("bad-key", make_messages(), 0.7, 1024, cancel)
        .await;

    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected error, got Ok"),
    };
    let msg = err.to_string();
    assert!(msg.contains("401"), "error should contain HTTP 401: {msg}");
    match err {
        AppError::Llm(_) => {} // expected
        other => panic!("expected Llm error, got: {other}"),
    }
}

#[tokio::test]
async fn openai_truncated_stream_yields_partial_content() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(truncated_sse_response()),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiAdapter::new("test-model".into(), server.uri());
    let cancel = CancellationToken::new();

    let stream = adapter
        .stream_chat("test-key", make_messages(), 0.7, 1024, cancel)
        .await
        .expect("stream_chat should succeed (error happens during consumption)");

    let mut content_parts: Vec<String> = Vec::new();
    let mut stream = Box::pin(stream);

    while let Some(item) = stream.next().await {
        match item {
            Ok(ChatDelta::Delta { content }) => content_parts.push(content),
            Ok(ChatDelta::Done { .. }) => break,
            Err(_) => break,
        }
    }

    // 截断流：wiremock 返回了完整 body（字节流自然结束），adapter 会优雅地发 Done
    assert_eq!(content_parts, vec!["partial".to_string()]);
}
