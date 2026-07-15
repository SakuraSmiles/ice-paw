//! Chat 调度循环：LLM 流式消费 + 指数退避重试 + 工具执行编排
//!
//! 这是 chat_cmd.rs 拆分（Step 5）的目标模块，承接：
//! - `stream_loop` 异步协程（流式消费 / 重试 / 工具编排 / DB 回写）
//! - 内部辅助 struct `CollectedToolCall`
//!
//! 职责边界：
//! - 入口：`chat_cmd::send_message` 通过 `super::chat_loop::stream_loop(...)` 调用
//! - 错误映射：依赖 `super::chat_error::error_kind` / `friendly_error`
//! - 收尾：依赖 `super::chat_cleanup::cleanup` / `cleanup_after_success_with_blocks`
//! - 事件 Payload：来自 `crate::infra::protocol::*`
//!
//! 后续可继续拆分（本步骤不做）：
//! - `consume_stream()` — 拆出流式消费 + Delta 路由逻辑（~120 行）
//! - `execute_tools()` — 拆出工具执行 + 结果回传逻辑（~90 行）

use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use sqlx::SqlitePool;

use crate::db::repo;
use crate::infra::protocol::{
    ChatDelta, ChatMessage, ChatChunkPayload, ChatErrorPayload, ChatRetryingPayload,
    ChatThinkingPayload, ChatToolCallDeltaPayload, ChatToolCallEndPayload,
    ChatToolCallStartPayload, ChatToolResultPayload, ContentBlock,
    LlmProvider, TokenUsage,
};
use crate::harness::chat_state::CancellationToken;
use crate::harness::tool_registry::ToolRegistry;

use super::chat_cleanup::{cleanup, cleanup_after_success_with_blocks};
use super::chat_error::{error_kind, friendly_error};

/// 流式生成内部协程 — 支持指数退避重试 + 工具执行循环
///
/// P2-1 工具执行循环：
/// 1. 调 provider.stream_chat(messages, tools?, ...)
/// 2. 消费 stream，收集文本 delta / 思考 delta / 工具调用
/// 3. stream 结束后：
///    a. 如果产生了工具调用（tool_calls 非空）：
///       - 在 Rust 侧通过 ToolRegistry 执行工具
///       - 将 tool_use + tool_result 作为 content block 追加到 messages
///       - emit chat:tool-result
///       - 回到步骤 1（最多 5 轮，防止无限循环）
///       b. 如果没有工具调用 → 正常结束，emit chat:done
///
/// 重试策略：
/// - 首次失败 → 等待 1s → 第 2 次尝试
/// - 二次失败 → 等待 2s → 第 3 次尝试
/// - 三次失败 → 等待 4s → 第 4 次尝试（总计 4 次，即最多 3 次重试）
/// - 超过 4 次 → 放弃，emit chat:error
///
/// 不重试的情况：
/// - 用户主动取消（cancel.is_cancelled()）
/// - 不可重试错误（401/403 等）
pub(crate) async fn stream_loop(
    app: AppHandle,
    pool: SqlitePool,
    provider: Arc<dyn LlmProvider>,
    api_key: String,
    mut messages: Vec<ChatMessage>,
    temperature: f64,
    max_tokens: i32,
    cancel: CancellationToken,
    conv_id: String,
    asst_msg_id: String,
    tool_registry: ToolRegistry,
    tools_enabled: bool,
) {
    use futures::StreamExt;
    use std::collections::HashMap;
    use std::time::Duration;

    const MAX_TOOL_ROUNDS: u32 = 5;
    const MAX_ATTEMPTS: u32 = 4;

    /// 一轮流式消费中收集到的工具调用信息
    #[derive(Debug, Clone)]
    struct CollectedToolCall {
        id: String,
        name: String,
        /// 累积的 arguments JSON 片段
        arguments: String,
        /// 是否已收到 ToolCallEnd
        ended: bool,
    }

    // 累积所有轮次的文本
    let mut all_text = String::new();
    // 累积所有轮次的 content_blocks（用于 DB 回写）
    let mut all_content_blocks: Vec<ContentBlock> = Vec::new();
    // P2-3: 累积 token usage（最后一个 Usage delta 覆盖前面的）
    let mut collected_usage: Option<TokenUsage> = None;

    // === 工具执行循环 ===
    for tool_round in 0..MAX_TOOL_ROUNDS {
        if cancel.is_cancelled() {
            return cleanup(&app, &pool, &conv_id);
        }

        // 准备本轮的 tools 定义
        // 所有轮次都传 tools：messages 中含 assistant 的 tool_calls 时，
        // 部分 API（GLM 等）要求请求必须带 tools 定义，否则返回 400
        let tools: Option<Vec<crate::infra::protocol::ToolDef>> = if tools_enabled {
            Some(tool_registry.list_tool_defs().await)
        } else {
            None
        };

        // 本轮收集
        let mut round_text = String::new();
        let mut round_think = String::new();
        let mut round_finish_reason = "stop".to_string();
        let mut tool_calls_map: HashMap<String, CollectedToolCall> = HashMap::new();
        let mut round_success = false;

        // === 重试循环（每轮内）===
        'retry_loop: for attempt in 0..MAX_ATTEMPTS {
            if cancel.is_cancelled() {
                return cleanup(&app, &pool, &conv_id);
            }

            if attempt > 0 {
                let wait_secs = 1u64 << (attempt - 1);
                tracing::info!(
                    target: "ice_paw.chat",
                    "重试 LLM 请求: tool_round={} attempt={}/{}，等待 {}s",
                    tool_round, attempt + 1, MAX_ATTEMPTS, wait_secs,
                );
                let _ = app.emit(
                    "chat:retrying",
                    ChatRetryingPayload {
                        conversation_id: conv_id.clone(),
                        message_id: asst_msg_id.clone(),
                        attempt: attempt + 1,
                        max_attempts: MAX_ATTEMPTS,
                    },
                );
                tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                if cancel.is_cancelled() {
                    return cleanup(&app, &pool, &conv_id);
                }
            }

            // 构造重试消息
            let retry_messages = if !round_text.is_empty() && attempt > 0 {
                let mut msgs = messages.clone();
                msgs.push(ChatMessage::from_text(
                    "assistant",
                    format!(
                        "[以下是上一轮因网络中断已收到的部分回复，请从此处继续]\n{}",
                        &round_text
                    ),
                ));
                msgs
            } else {
                messages.clone()
            };

            let stream_result = provider
                .stream_chat(
                    &api_key,
                    retry_messages,
                    tools.clone(),
                    temperature,
                    max_tokens,
                    cancel.clone(),
                )
                .await;

            match stream_result {
                Ok(mut stream) => {
                    let mut attempt_ok = true;

                    while let Some(item) = stream.next().await {
                        if cancel.is_cancelled() {
                            return cleanup(&app, &pool, &conv_id);
                        }

                        match item {
                            Ok(ChatDelta::Delta { content: delta }) => {
                                round_text.push_str(&delta);
                                let _ = app.emit(
                                    "chat:chunk",
                                    ChatChunkPayload {
                                        conversation_id: conv_id.clone(),
                                        message_id: asst_msg_id.clone(),
                                        delta,
                                    },
                                );
                            }
                            Ok(ChatDelta::ToolCallStart { id, name }) => {
                                tool_calls_map.insert(
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
                                        conversation_id: conv_id.clone(),
                                        message_id: asst_msg_id.clone(),
                                        id: id.clone(),
                                        name,
                                    },
                                );
                            }
                            Ok(ChatDelta::ToolCallDelta { id, delta: tool_delta }) => {
                                if let Some(tc) = tool_calls_map.get_mut(&id) {
                                    tc.arguments.push_str(&tool_delta);
                                }
                                let _ = app.emit(
                                    "chat:tool-call-delta",
                                    ChatToolCallDeltaPayload {
                                        conversation_id: conv_id.clone(),
                                        message_id: asst_msg_id.clone(),
                                        id,
                                        delta: tool_delta,
                                    },
                                );
                            }
                            Ok(ChatDelta::ToolCallEnd { id }) => {
                                if let Some(tc) = tool_calls_map.get_mut(&id) {
                                    tc.ended = true;
                                }
                                let _ = app.emit(
                                    "chat:tool-call-end",
                                    ChatToolCallEndPayload {
                                        conversation_id: conv_id.clone(),
                                        message_id: asst_msg_id.clone(),
                                        id,
                                    },
                                );
                            }
                            Ok(ChatDelta::Thinking { content: think_content }) => {
                                round_think.push_str(&think_content);
                                let _ = app.emit(
                                    "chat:thinking",
                                    ChatThinkingPayload {
                                        conversation_id: conv_id.clone(),
                                        message_id: asst_msg_id.clone(),
                                        content: think_content,
                                    },
                                );
                            }
                            // P2-3: Token usage
                            Ok(ChatDelta::Usage { usage: u }) => {
                                collected_usage = Some(u);
                            }
                            Ok(ChatDelta::Done { finish_reason: fr }) => {
                                if let Some(fr) = fr {
                                    round_finish_reason = fr;
                                }
                                round_success = true;
                                break 'retry_loop;
                            }
                            Err(e) => {
                                if e.is_retryable() {
                                    attempt_ok = false;
                                    tracing::warn!(
                                        target: "ice_paw.chat",
                                        "流中可重试错误 (round={} attempt={}/{}): {}",
                                        tool_round, attempt + 1, MAX_ATTEMPTS, e
                                    );
                                    break; // 跳出 inner while，进入下一轮重试
                                } else {
                                    let err_msg = e.to_string();
                                    let _ = app.emit(
                                        "chat:error",
                                        ChatErrorPayload {
                                            conversation_id: conv_id.clone(),
                                            message_id: asst_msg_id.clone(),
                                            kind: error_kind(&e),
                                            message: friendly_error(&err_msg),
                                        },
                                    );
                                    let _ = repo::message::update_error(&pool, &asst_msg_id, &err_msg).await;
                                    return cleanup(&app, &pool, &conv_id);
                                }
                            }
                        }
                    }

                    // stream 自然结束但没收到 Done
                    if attempt_ok {
                        round_success = true;
                        break 'retry_loop;
                    }
                }
                Err(e) => {
                    if e.is_retryable() {
                        tracing::warn!(
                            target: "ice_paw.chat",
                            "请求失败可重试 (round={} attempt={}/{}): {}",
                            tool_round, attempt + 1, MAX_ATTEMPTS, e
                        );
                    } else {
                        let err_msg = e.to_string();
                        let _ = app.emit(
                            "chat:error",
                            ChatErrorPayload {
                                conversation_id: conv_id.clone(),
                                message_id: asst_msg_id.clone(),
                                kind: error_kind(&e),
                                message: friendly_error(&err_msg),
                            },
                        );
                        let _ = repo::message::update_error(&pool, &asst_msg_id, &err_msg).await;
                        return cleanup(&app, &pool, &conv_id);
                    }
                }
            }
        }

        if !round_success {
            // 重试耗尽
            let err_msg = format!("连接重试已耗尽（共 {} 次），已收到部分内容", MAX_ATTEMPTS);
            if !round_text.is_empty() {
                let _ = repo::message::update_content(&pool, &asst_msg_id, &round_text).await;
            }
            let _ = repo::message::update_error(&pool, &asst_msg_id, &err_msg).await;
            let _ = app.emit(
                "chat:error",
                ChatErrorPayload {
                    conversation_id: conv_id.clone(),
                    message_id: asst_msg_id.clone(),
                    kind: "stream".into(),
                    message: friendly_error(&err_msg),
                },
            );
            return cleanup(&app, &pool, &conv_id);
        }

        // 累积文本
        all_text.push_str(&round_text);

        // 累积 thinking
        if !round_think.is_empty() {
            all_content_blocks.push(ContentBlock::Thinking {
                thinking: round_think,
                signature: None,
            });
        }

        // 检查是否有工具调用需要执行
        let completed_calls: Vec<(String, String, String)> = tool_calls_map
            .into_values()
            .filter(|tc| tc.ended)
            .map(|tc| (tc.id, tc.name, tc.arguments))
            .collect();

        if completed_calls.is_empty() {
            // 没有工具调用 → 正常结束
            // 先保存文本副本用于 DB content 字段，再 move 进 content block
            let content_for_db = all_text.clone();
            if !all_text.is_empty() {
                all_content_blocks.push(ContentBlock::Text {
                    text: all_text,
                });
            }
            return cleanup_after_success_with_blocks(
                &app, &pool, &conv_id, &asst_msg_id,
                &content_for_db, &all_content_blocks, &round_finish_reason,
                collected_usage,
            );
        }

        // === 执行工具调用 ===
        tracing::info!(
            target: "ice_paw.chat",
            "工具调用循环: round={} tool_count={}",
            tool_round, completed_calls.len(),
        );

        let mut tool_use_blocks: Vec<ContentBlock> = Vec::new();
        let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

        for (tc_id, tc_name, tc_args) in &completed_calls {
            let result = tool_registry.dispatch(tc_name, tc_args).await;

            match result {
                Ok(content) => {
                    let _ = app.emit(
                        "chat:tool-result",
                        ChatToolResultPayload {
                            conversation_id: conv_id.clone(),
                            message_id: asst_msg_id.clone(),
                            tool_use_id: tc_id.clone(),
                            content: content.clone(),
                            is_error: false,
                        },
                    );
                    tool_result_blocks.push(ContentBlock::ToolResult {
                        tool_use_id: tc_id.clone(),
                        content,
                        is_error: Some(false),
                    });
                }
                Err(e) => {
                    let err_content = e.to_string();
                    let _ = app.emit(
                        "chat:tool-result",
                        ChatToolResultPayload {
                            conversation_id: conv_id.clone(),
                            message_id: asst_msg_id.clone(),
                            tool_use_id: tc_id.clone(),
                            content: err_content.clone(),
                            is_error: true,
                        },
                    );
                    tool_result_blocks.push(ContentBlock::ToolResult {
                        tool_use_id: tc_id.clone(),
                        content: err_content,
                        is_error: Some(true),
                    });
                }
            }

            tool_use_blocks.push(ContentBlock::ToolUse {
                id: tc_id.clone(),
                name: tc_name.clone(),
                input: tc_args.clone(),
            });
        }

        // 累积到 content_blocks
        all_content_blocks.extend(tool_use_blocks.clone());
        all_content_blocks.extend(tool_result_blocks.clone());

        // 追加到 messages：assistant 消息含 tool_use + 文本
        // tool_result 以 tool 角色回传（OpenAI 格式要求 role=tool + tool_call_id）
        // Anthropic adapter 的 split_system_prompt 会把 tool 角色转为 user
        let mut asst_blocks: Vec<ContentBlock> = Vec::new();
        if !round_text.is_empty() {
            asst_blocks.push(ContentBlock::Text {
                text: round_text,
            });
        }
        asst_blocks.extend(tool_use_blocks);
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: asst_blocks,
        });

        // 每个 tool_result 作为独立的 tool 角色消息（OpenAI 格式）
        for block in &tool_result_blocks {
            messages.push(ChatMessage {
                role: "tool".into(),
                content: vec![block.clone()],
            });
        }

        tracing::info!(
            target: "ice_paw.chat",
            "工具执行完成: round={}，准备下一轮 LLM 调用",
            tool_round,
        );
    }

    // 达到最大轮数 → 正常结束（所有工具已完成）
    let content_for_db = all_text.clone();
    if !all_text.is_empty() {
        all_content_blocks.push(ContentBlock::Text {
            text: all_text,
        });
    }
    cleanup_after_success_with_blocks(
        &app, &pool, &conv_id, &asst_msg_id,
        &content_for_db, &all_content_blocks, "tool_use",
        collected_usage,
    );
}
