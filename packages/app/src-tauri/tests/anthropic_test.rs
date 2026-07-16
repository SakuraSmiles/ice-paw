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
use ice_paw_lib::harness::chat_state::CancellationToken;
use ice_paw_lib::harness::provider::anthropic::AnthropicAdapter;
use ice_paw_lib::infra::protocol::{ChatDelta, ChatMessage, LlmProvider, TokenUsage};
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
    vec![ChatMessage::from_text("user", "你好")]
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

    let adapter = AnthropicAdapter::new("test-model".into(), server.uri(), false);
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
            Ok(ChatDelta::Done { finish_reason }) => {
                got_done = true;
                assert_eq!(finish_reason, Some("end_turn".into()));
                break;
            }
            Ok(_) => {},
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

    let adapter = AnthropicAdapter::new("test-model".into(), server.uri(), false);
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

    let adapter = AnthropicAdapter::new("test-model".into(), server.uri(), false);
    let cancel = CancellationToken::new();

    let stream = adapter
        .stream_chat("test-key", make_messages(), None, 0.7, 1024, cancel)
        .await
        .expect("stream_chat should succeed");

    let mut content_parts: Vec<String> = Vec::new();
    let mut got_error = false;
    let mut stream = Box::pin(stream);

    while let Some(item) = stream.next().await {
        match item {
            Ok(ChatDelta::Delta { content }) => content_parts.push(content),
            Ok(ChatDelta::Done { .. }) => break,
            Ok(_) => {},
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

// =========================================================================
// P2-1i: 工具调用事件集成测试
// =========================================================================

/// P2-1i: Anthropic tool_use stream → ToolCallStart/ToolCallDelta/ToolCallEnd 事件
#[tokio::test]
async fn anthropic_tool_use_produces_tool_call_events() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(mixed_tool_use_sse()),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("test-model".into(), server.uri(), false);
    let cancel = CancellationToken::new();

    let stream = adapter
        .stream_chat("test-key", make_messages(), None, 0.7, 1024, cancel)
        .await
        .expect("stream_chat should succeed");

    let mut stream = Box::pin(stream);
    let mut start_count = 0;
    let mut delta_count = 0;
    let mut end_count = 0;
    let mut text_count = 0;
    let mut tool_name = String::new();
    let mut tool_id = String::new();

    while let Some(item) = stream.next().await {
        match item {
            Ok(ChatDelta::Delta { .. }) => text_count += 1,
            Ok(ChatDelta::ToolCallStart { id, name }) => {
                start_count += 1;
                tool_id = id;
                tool_name = name;
            }
            Ok(ChatDelta::ToolCallDelta { .. }) => delta_count += 1,
            Ok(ChatDelta::ToolCallEnd { .. }) => end_count += 1,
            Ok(ChatDelta::Thinking { .. }) => {},
            Ok(ChatDelta::Done { .. }) => break,
            Ok(ChatDelta::Usage { .. }) => {},
            Err(_) => break,
        }
    }

    assert_eq!(text_count, 1, "应收到 1 个文本 Delta（前导文本）");
    assert_eq!(start_count, 1, "应收到 1 个 ToolCallStart");
    assert_eq!(tool_name, "read_file", "tool name 应为 read_file");
    assert!(!tool_id.is_empty(), "tool id 不应为空");
    assert_eq!(end_count, 1, "应收到 1 个 ToolCallEnd");
    assert!(delta_count >= 1, "应收到至少 1 个 ToolCallDelta");
}

/// P2-1i: Anthropic tool_use stream 不将 tool_use delta 当作文本
/// （回归验证：P2-1 工具调用 delta 不能混入文本流）
#[tokio::test]
async fn anthropic_tool_use_delta_not_in_text() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(mixed_tool_use_sse()),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("test-model".into(), server.uri(), false);
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

    // 只应收集到前导文本，tool_use 的 input_json_delta 不应出现在文本中
    assert_eq!(content_parts, vec!["让我查一下".to_string()]);
}

// =========================================================================
// P2-3: ChatDelta::Usage 集成测试
// =========================================================================

/// 构造含 message_start.usage 和 message_delta.usage 的 Anthropic SSE 流。
/// 验证前端能收到带 cached_tokens 的 ChatDelta::Usage 事件。
fn usage_sse() -> String {
    r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_usage_01","usage":{"input_tokens":100,"cache_creation_input_tokens":50,"cache_read_input_tokens":30,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"cached"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5,"cache_creation_input_tokens":50,"cache_read_input_tokens":30}}

event: message_stop
data: {"type":"message_stop"}

"#.to_string()
}

/// P2-3: Anthropic SSE 流中的 usage 事件应产生 ChatDelta::Usage
#[tokio::test]
async fn anthropic_usage_event_from_stream() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(usage_sse()),
        )
        .mount(&server)
        .await;

    let adapter = AnthropicAdapter::new("test-model".into(), server.uri(), false);
    let cancel = CancellationToken::new();

    let stream = adapter
        .stream_chat("test-key", make_messages(), None, 0.7, 1024, cancel)
        .await
        .expect("stream_chat should succeed");

    let mut stream = Box::pin(stream);
    let mut usage_events: Vec<TokenUsage> = Vec::new();
    let mut got_done = false;

    while let Some(item) = stream.next().await {
        match item {
            Ok(ChatDelta::Usage { usage }) => usage_events.push(usage),
            Ok(ChatDelta::Done { .. }) => {
                got_done = true;
                break;
            }
            Ok(_) => {}
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    assert!(got_done, "应收到 Done 事件");
    // 应收到 2 个 Usage 事件：message_start + message_delta
    assert_eq!(usage_events.len(), 2, "应收到 2 个 Usage 事件（message_start + message_delta）");

    // 第 1 个 Usage 来自 message_start：prompt_tokens=100, cached_tokens=30
    assert_eq!(
        usage_events[0].prompt_tokens, 100,
        "message_start usage: prompt_tokens 应为 100"
    );
    assert_eq!(
        usage_events[0].cached_tokens, 30,
        "message_start usage: cached_tokens 应为 30"
    );

    // 第 2 个 Usage 来自 message_delta：completion_tokens=5, cached_tokens=30
    assert_eq!(
        usage_events[1].completion_tokens, 5,
        "message_delta usage: completion_tokens 应为 5"
    );
    assert_eq!(
        usage_events[1].cached_tokens, 30,
        "message_delta usage: cached_tokens 应为 30"
    );
}
