//! L2 Stream Consumer — 流式 Delta 消费逻辑（W3.3）
//!
//! 职责：从 `provider.stream_chat()` 返回的 SSE 流中逐项消费 `ChatDelta`，
//! 收集文本、思考内容、工具调用，最终返回 `StreamResult`。
//!
//! emit 实时事件（`chat:chunk` / `chat:thinking` / `chat:tool-call-*`）
//! 在消费过程中 emit，保持流式渲染体验。
//!
//! 错误透传：可重试/不可重试错误均由调用方 `loop_engine` 在外层统一处理。

use std::collections::HashMap;
use std::pin::Pin;

use futures::Stream;
use tauri::{AppHandle, Emitter};

use crate::error::AppError;
use crate::harness::chat_state::CancellationToken;
use crate::harness::observable::RoundState;
use crate::infra::protocol::{
    ChatChunkPayload, ChatDelta, ChatThinkingPayload, ChatToolCallDeltaPayload,
    ChatToolCallEndPayload, ChatToolCallStartPayload, TokenUsage,
};

/// 单轮流式消费结果
#[derive(Debug, Clone)]
pub struct StreamResult {
    pub text: String,
    pub think: String,
    pub finish_reason: String,
    pub tool_calls: HashMap<String, CollectedToolCall>,
    pub usage: Option<TokenUsage>,
}

/// 一轮流式消费中收集到的工具调用信息
#[derive(Debug, Clone)]
pub struct CollectedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub ended: bool,
}

/// 消费一个 LLM 流，返回 `StreamResult`。
///
/// 在消费过程中 emit chat:chunk / chat:thinking / chat:tool-call-*。
/// 错误透传给调用方（loop_engine 根据 is_retryable() 决定策略）。
pub async fn consume_stream(
    stream: &mut Pin<Box<dyn Stream<Item = Result<ChatDelta, AppError>> + Send>>,
    app: &AppHandle,
    cancel: &CancellationToken,
    round_state: &mut RoundState,
    conv_id: &str,
    asst_msg_id: &str,
) -> Result<StreamResult, AppError> {
    use futures::StreamExt;

    let mut text = String::new();
    let mut think = String::new();
    let mut finish_reason = "stop".to_string();
    let mut tool_calls: HashMap<String, CollectedToolCall> = HashMap::new();
    let mut last_usage: Option<TokenUsage> = None;

    while let Some(item) = stream.next().await {
        if cancel.is_cancelled() {
            return Ok(StreamResult {
                text,
                think,
                finish_reason,
                tool_calls,
                usage: last_usage,
            });
        }

        match item {
            Ok(ChatDelta::Delta { content: delta }) => {
                text.push_str(&delta);
                let _ = app.emit(
                    "chat:chunk",
                    ChatChunkPayload {
                        conversation_id: conv_id.to_string(),
                        message_id: asst_msg_id.to_string(),
                        delta,
                    },
                );
            }
            Ok(ChatDelta::ToolCallStart { id, name }) => {
                tool_calls.insert(
                    id.clone(),
                    CollectedToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: String::new(),
                        ended: false,
                    },
                );
                let _ = app.emit(
                    "chat:tool-call-start",
                    ChatToolCallStartPayload {
                        conversation_id: conv_id.to_string(),
                        message_id: asst_msg_id.to_string(),
                        id,
                        name,
                    },
                );
            }
            Ok(ChatDelta::ToolCallDelta {
                id,
                delta: tool_delta,
            }) => {
                if let Some(tc) = tool_calls.get_mut(&id) {
                    tc.arguments.push_str(&tool_delta);
                }
                let _ = app.emit(
                    "chat:tool-call-delta",
                    ChatToolCallDeltaPayload {
                        conversation_id: conv_id.to_string(),
                        message_id: asst_msg_id.to_string(),
                        id,
                        delta: tool_delta,
                    },
                );
            }
            Ok(ChatDelta::ToolCallEnd { id }) => {
                if let Some(tc) = tool_calls.get_mut(&id) {
                    tc.ended = true;
                }
                let _ = app.emit(
                    "chat:tool-call-end",
                    ChatToolCallEndPayload {
                        conversation_id: conv_id.to_string(),
                        message_id: asst_msg_id.to_string(),
                        id,
                    },
                );
            }
            Ok(ChatDelta::Thinking {
                content: think_content,
            }) => {
                think.push_str(&think_content);
                let _ = app.emit(
                    "chat:thinking",
                    ChatThinkingPayload {
                        conversation_id: conv_id.to_string(),
                        message_id: asst_msg_id.to_string(),
                        content: think_content,
                    },
                );
            }
            Ok(ChatDelta::Usage { usage: u }) => {
                // 字段级合并而非覆盖：Anthropic 分两个事件发 usage——message_start 带
                // input_tokens（prompt）、message_delta 带 output_tokens（completion）且不含
                // input_tokens。直接 last-wins 覆盖会把 prompt 冲成 0（→ user 消息 token_count
                // 恒 0、预算漏算 prompt）。取各字段非零值合并；OpenAI / 单次 usage 路径无影响。
                last_usage = Some(match last_usage {
                    Some(prev) => TokenUsage {
                        prompt_tokens: if u.prompt_tokens > 0 {
                            u.prompt_tokens
                        } else {
                            prev.prompt_tokens
                        },
                        completion_tokens: if u.completion_tokens > 0 {
                            u.completion_tokens
                        } else {
                            prev.completion_tokens
                        },
                        cached_tokens: if u.cached_tokens > 0 {
                            u.cached_tokens
                        } else {
                            prev.cached_tokens
                        },
                    },
                    None => u,
                });
                if let Some(ref merged) = last_usage {
                    round_state.tokens_prompt = merged.prompt_tokens;
                    round_state.tokens_completion = merged.completion_tokens;
                    round_state.cached_tokens = merged.cached_tokens;
                }
            }
            Ok(ChatDelta::Done { finish_reason: fr }) => {
                if let Some(fr) = fr {
                    finish_reason = fr;
                }
                return Ok(StreamResult {
                    text,
                    think,
                    finish_reason,
                    tool_calls,
                    usage: last_usage,
                });
            }
            Err(e) => {
                return Err(e);
            }
        }
    }

    Ok(StreamResult {
        text,
        think,
        finish_reason,
        tool_calls,
        usage: last_usage,
    })
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_usage() -> TokenUsage {
        TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            cached_tokens: 10,
        }
    }

    #[test]
    fn test_stream_result_fields() {
        let r = StreamResult {
            text: "hello world".to_string(),
            think: "thinking...".to_string(),
            finish_reason: "stop".to_string(),
            tool_calls: HashMap::new(),
            usage: Some(make_usage()),
        };
        assert_eq!(r.text, "hello world");
        assert_eq!(r.finish_reason, "stop");
        assert!(r.tool_calls.is_empty());
        let u = r.usage.unwrap();
        assert_eq!(u.prompt_tokens, 100);
        assert_eq!(u.completion_tokens, 50);
        assert_eq!(u.cached_tokens, 10);
    }

    #[test]
    fn test_collected_tool_call() {
        let tc = CollectedToolCall {
            id: "call_123".to_string(),
            name: "read_file".to_string(),
            arguments: r#"{"path":"/test.txt"}"#.to_string(),
            ended: true,
        };
        assert_eq!(tc.name, "read_file");
        assert!(tc.ended);
    }

    #[test]
    fn test_stream_result_tool_calls() {
        let mut tc_map: HashMap<String, CollectedToolCall> = HashMap::new();
        tc_map.insert(
            "call_1".into(),
            CollectedToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"a"}"#.into(),
                ended: true,
            },
        );
        let r = StreamResult {
            text: "".to_string(),
            think: "".to_string(),
            finish_reason: "stop".to_string(),
            tool_calls: tc_map,
            usage: None,
        };
        assert_eq!(r.tool_calls.len(), 1);
        assert!(r.tool_calls.get("call_1").unwrap().ended);
    }
}
