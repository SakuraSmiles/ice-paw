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

// W2.6: 将 AppError 分类为 retry reason 字符串（用于 chat:retrying payload）
fn classify_retry_reason(e: &crate::error::AppError) -> String {
    use crate::error::AppError::*;
    let msg = match e {
        Llm(s) | Stream(s) | Internal(s) | Stronghold(s) => s.as_str(),
        Io(_) => return "network_error".into(),
        Tauri(s) => s.as_str(),
        _ => return "unknown_error".into(),
    };
    let lower = msg.to_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        "timeout".into()
    } else if lower.contains("rate_limit") || lower.contains("429") || lower.contains("too many requests") {
        "rate_limited".into()
    } else if lower.contains("500") || lower.contains("502") || lower.contains("503")
        || lower.contains("server_error") || lower.contains("internal server error")
        || lower.contains("upstream")
    {
        "server_error_5xx".into()
    } else if lower.contains("connection") || lower.contains("network")
        || lower.contains("dns") || lower.contains("refused")
        || lower.contains("broken pipe") || lower.contains("reset")
    {
        "network_error".into()
    } else {
        "unknown_error".into()
    }
}

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
use crate::harness::budget::{MAX_ATTEMPTS, MAX_TOOL_ROUNDS};
use crate::harness::chat_state::CancellationToken;
use crate::harness::observable::{RoundState, RoundTimer};
use crate::harness::retry::{RetryContext, RetryState};
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
    observable: &mut RoundState,
) {
    use futures::StreamExt;
    use std::collections::HashMap;
    use std::time::Duration;

    // W3.1: 循环上限常量已迁至 `harness::budget::{MAX_TOOL_ROUNDS, MAX_ATTEMPTS}`
    // （与原硬编码值完全一致，行为不变）

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

        // W2.4: 开启本轮计时
        let round_timer = RoundTimer::new(tool_round);
        observable.round = tool_round + 1;

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

        // === W3.2: RetryState 驱动的重试循环（替代原 for attempt + 字符串拼接）===
        let mut retry_state = RetryState::new();
        let mut last_retry_reason = String::new();

        'retry_loop: loop {
            if !retry_state.can_retry() {
                break;
            }
            if cancel.is_cancelled() {
                return cleanup(&app, &pool, &conv_id);
            }

            // sleep（仅 retry 时）
            let ws = retry_state.wait_secs();
            if ws > 0 {
                tracing::info!(
                    target: "ice_paw.chat",
                    "重试 LLM 请求: tool_round={} attempt={}/{}，等待 {}s",
                    tool_round, retry_state.attempt_num() + 1, MAX_ATTEMPTS, ws,
                );
                // W2.4: 累积 retry 计数
                observable.retry_count += 1;
                let _ = app.emit(
                    "chat:retrying",
                    ChatRetryingPayload {
                        conversation_id: conv_id.clone(),
                        message_id: asst_msg_id.clone(),
                        attempt: retry_state.attempt_num() + 1,
                        max_attempts: MAX_ATTEMPTS,
                        reason: last_retry_reason.clone(),
                    },
                );
                tokio::time::sleep(Duration::from_secs(ws)).await;
                if cancel.is_cancelled() {
                    return cleanup(&app, &pool, &conv_id);
                }
            }

            // W3.2: 用 RetryState.prepare_messages 替代字符串拼接
            let retry_ctx = RetryContext::with_round_text(
                messages.clone(),
                round_text.clone(),
            );
            let retry_messages = retry_state.prepare_messages(&retry_ctx);

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
                                collected_usage = Some(u.clone());
                                // W2.4: 累积到 observable
                                observable.tokens_prompt = u.prompt_tokens;
                                observable.tokens_completion = u.completion_tokens;
                                observable.cached_tokens = u.cached_tokens;
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
                                    // W2.6: 记录重试原因
                                    last_retry_reason = classify_retry_reason(&e);
                                    tracing::warn!(
                                        target: "ice_paw.chat",
                                        "流中可重试错误 (round={} attempt={}/{}): {}",
                                        tool_round, retry_state.attempt_num() + 1, MAX_ATTEMPTS, e
                                    );
                                    // W3.2: 转移状态，下次循环的 wait_secs() 即为新的退避值
                                    retry_state = retry_state.next_retry(MAX_ATTEMPTS, 1u64 << retry_state.attempt_num());
                                    break; // 跳出 inner while，进入 'retry_loop 下一轮
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
                        // W2.6: 记录重试原因
                        last_retry_reason = classify_retry_reason(&e);
                        tracing::warn!(
                            target: "ice_paw.chat",
                            "请求失败可重试 (round={} attempt={}/{}): {}",
                            tool_round, retry_state.attempt_num() + 1, MAX_ATTEMPTS, e
                        );
                        // W3.2: 转移状态，下次循环的 wait_secs() 即为新的退避值
                        retry_state = retry_state.next_retry(MAX_ATTEMPTS, 1u64 << retry_state.attempt_num());
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
            // W3.2: 若执行流中 retryable error 后跳出 inner while，
            // retry_state 已在 match 分支中转移；循环顶部会再次检查 can_retry()
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

        // W2.4: 更新本轮耗时
        observable.elapsed_ms = round_timer.elapsed_ms();

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
    // W2.4: observable already has latest round data from last iteration
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
