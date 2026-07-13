//! Anthropic Adapter 集成测试
//!
//! 使用 wiremock 模拟 Anthropic Messages API 的 SSE 流，
//! 验证 `AnthropicAdapter::stream_chat` 的完整行为。
//!
//! 三个场景：
//! 1. 正常文本流 → 收集到预期文本
//! 2. 含 tool_use 的混合流 → 只收集文本，跳过 tool_use delta
//! 3. 流中 error 事件 → 升级为 LlmError

use futures::StreamExt;
use ice_paw_lib::error::AppError;
use ice_paw_lib::llm::adapters::anthropic::AnthropicAdapter;
use ice_paw_lib::llm::{CancellationToken, ChatDelta, ChatMessage, LlmProvider};
use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

/// 构造正常的 Anthropic SSE 流（双行格式 event+data）。
/// 输出 "你好世界！"
fn normal_anthropic_sse() -> String {
    r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_01"}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"你好"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"世界"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"！"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}

event: message_stop
data: {"type":"message_stop"}

"#.to_string()
}

/// 构造含 tool_use 混合流的 Anthropic SSE。
/// 预期：只收集 text_delta 部分，tool_use/input_json_delta 被跳过。
fn mixed_tool_use_sse() -> String {
    r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_02"}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"让我查一下"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"read_file"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"/tmp/test.txt\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}

event: message_stop
data: {"type":"message_stop"}

"#.to_string()
}

/// 构造含 error 事件的 Anthropic SSE 流。
fn error_event_sse() -> String {
    r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_03"}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"开始"}}

event: error
data: {"type":"error","error":{"type":"api_error","message":"模型过载，请稍后重试"}}

"#.to_string()
}

fn make_messages() -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: "user".into(),
        content: "你好".into(),
    }]
}

#[tokio::test]
async fn anthropic_normal_text_stream_collects_expected() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(normal_anthropic_sse()),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("test-model".into(), server.uri());
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
            Ok(ChatDelta::Done { finish_reason }) => {
                got_done = true;
                assert_eq!(finish_reason, Some("end_turn".into()));
                break;
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    assert_eq!(
        content_parts,
        vec!["你好".to_string(), "世界".to_string(), "！".to_string()]
    );
    assert!(got_done);
}

#[tokio::test]
async fn anthropic_mixed_tool_use_only_collects_text() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(mixed_tool_use_sse()),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("test-model".into(), server.uri());
    let cancel = CancellationToken::new();

    let stream = adapter
        .stream_chat("test-key", make_messages(), 0.7, 1024, cancel)
        .await
        .expect("stream_chat should succeed");

    let mut content_parts: Vec<String> = Vec::new();
    let mut stream = Box::pin(stream);

    while let Some(item) = stream.next().await {
        match item {
            Ok(ChatDelta::Delta { content }) => content_parts.push(content),
            Ok(ChatDelta::Done { .. }) => break,
            Err(_) => break,
        }
    }

    // 只应收集到文本 "让我查一下"，tool_use 的 input_json_delta 不应出现
    assert_eq!(content_parts, vec!["让我查一下".to_string()]);
}

#[tokio::test]
async fn anthropic_error_event_returns_llm_error() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(error_event_sse()),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("test-model".into(), server.uri());
    let cancel = CancellationToken::new();

    let stream = adapter
        .stream_chat("test-key", make_messages(), 0.7, 1024, cancel)
        .await
        .expect("stream_chat should succeed");

    let mut content_parts: Vec<String> = Vec::new();
    let mut got_error = false;
    let mut stream = Box::pin(stream);

    while let Some(item) = stream.next().await {
        match item {
            Ok(ChatDelta::Delta { content }) => content_parts.push(content),
            Ok(ChatDelta::Done { .. }) => break,
            Err(e) => {
                got_error = true;
                match e {
                    AppError::Llm(msg) => {
                        assert!(
                            msg.contains("Anthropic"),
                            "error should mention Anthropic: {msg}"
                        );
                    }
                    other => panic!("expected Llm error, got: {other}"),
                }
                break;
            }
        }
    }

    // 应收到 "开始" 文本后遇到 error
    assert_eq!(content_parts, vec!["开始".to_string()]);
    assert!(got_error, "should receive Llm error from stream error event");
}
