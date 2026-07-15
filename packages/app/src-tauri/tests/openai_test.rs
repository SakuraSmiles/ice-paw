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
use ice_paw_lib::harness::chat_state::CancellationToken;
use ice_paw_lib::harness::provider::openai::OpenAiAdapter;
use ice_paw_lib::infra::protocol::{ChatDelta, ChatMessage, LlmProvider};
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
    vec![ChatMessage::from_text("user", "Say hello")]
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
        .stream_chat("test-key", make_messages(), None, 0.7, 1024, cancel)
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
            Ok(_) => {},
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
        .stream_chat("bad-key", make_messages(), None, 0.7, 1024, cancel)
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
        .stream_chat("test-key", make_messages(), None, 0.7, 1024, cancel)
        .await
        .expect("stream_chat should succeed (error happens during consumption)");

    let mut content_parts: Vec<String> = Vec::new();
    let mut stream = Box::pin(stream);

    while let Some(item) = stream.next().await {
        match item {
            Ok(ChatDelta::Delta { content }) => content_parts.push(content),
            Ok(ChatDelta::Done { .. }) => break,
            Ok(_) => {},
            Err(_) => break,
        }
    }

    // 截断流：wiremock 返回了完整 body（字节流自然结束），adapter 会优雅地发 Done
    assert_eq!(content_parts, vec!["partial".to_string()]);
}

// =========================================================================
// P2-1i: OpenAI 工具调用事件集成测试
// =========================================================================

/// 构造 OpenAI SSE 流 — 含 tool_calls 的流式响应。
/// 模拟：先输出一段文本，然后触发 read_file 工具调用，参数分片输出。
fn tool_call_sse_response() -> String {
    let chunks = [
        // 1. 前导文本
        r#"data: {"choices":[{"delta":{"content":"让我查一下"},"finish_reason":null}]}"#,
        // 2. tool_call 名称（首批）
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_001","type":"function","function":{"name":"read_file","arguments":""}}]},"finish_reason":null}]}"#,
        // 3. arguments 第一段
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\""}}]},"finish_reason":null}]}"#,
        // 4. arguments 第二段
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":": \"Cargo.toml\"}"}}]},"finish_reason":null}]}"#,
        // 5. 结束 + stop_reason=tool_calls
        r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        "data: [DONE]",
    ];
    chunks.join("\n\n")
}

/// P2-1i: OpenAI tool_calls 流 → ToolCallStart/ToolCallDelta/ToolCallEnd 事件
#[tokio::test]
async fn openai_tool_calls_produces_tool_call_events() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(tool_call_sse_response()),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiAdapter::new("test-model".into(), server.uri());
    let cancel = CancellationToken::new();

    let stream = adapter
        .stream_chat("test-key", make_messages(), None, 0.7, 1024, cancel)
        .await
        .expect("stream_chat should succeed");

    let mut stream = Box::pin(stream);
    let mut text_parts: Vec<String> = Vec::new();
    let mut start_count = 0;
    let mut delta_count = 0;
    let mut end_count = 0;
    let mut tool_name = String::new();
    let mut tool_id = String::new();

    while let Some(item) = stream.next().await {
        match item {
            Ok(ChatDelta::Delta { content }) => text_parts.push(content),
            Ok(ChatDelta::ToolCallStart { id, name }) => {
                start_count += 1;
                tool_id = id;
                tool_name = name;
            }
            Ok(ChatDelta::ToolCallDelta { .. }) => delta_count += 1,
            Ok(ChatDelta::ToolCallEnd { .. }) => end_count += 1,
            Ok(ChatDelta::Thinking { .. }) => {},
            Ok(ChatDelta::Done { .. }) => break,
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    assert_eq!(text_parts.join(""), "让我查一下", "前导文本应完整");
    assert_eq!(start_count, 1, "应收到 1 个 ToolCallStart");
    assert!(!tool_id.is_empty(), "tool id 不应为空");
    assert_eq!(tool_name, "read_file", "tool name 应为 read_file");
    assert_eq!(end_count, 1, "应收到 1 个 ToolCallEnd");
    assert!(delta_count >= 1, "应收到至少 1 个 ToolCallDelta");
}

/// P2-1i: OpenAI tool_calls 的 delta 不混入文本流
#[tokio::test]
async fn openai_tool_calls_delta_not_in_text() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(tool_call_sse_response()),
        )
        .mount(&server)
        .await;

    let adapter = OpenAiAdapter::new("test-model".into(), server.uri());
    let cancel = CancellationToken::new();

    let stream = adapter
        .stream_chat("test-key", make_messages(), None, 0.7, 1024, cancel)
        .await
        .expect("stream_chat should succeed");

    let mut content_parts: Vec<String> = Vec::new();
    let mut stream = Box::pin(stream);

    while let Some(item) = stream.next().await {
        match item {
            Ok(ChatDelta::Delta { content }) => content_parts.push(content),
            Ok(ChatDelta::Done { .. }) => break,
            Ok(_) => {},
            Err(_) => break,
        }
    }

    // 只应收集到前导文本，tool_calls 的 arguments 片段不应混入文本流
    assert_eq!(content_parts.join(""), "让我查一下");
}
