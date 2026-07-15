//! L2 Loop Engine — 主循环调度（W3.3 + W4.1）
//!
//! 职责：编排工具执行循环（tool_round loop）+ 重试循环（retry loop），
//! 调用 `stream_consumer::consume_stream` 消费 LLM 流，
//! 调用 `tool_executor::execute_tool_round` 执行工具，
//! 统一 emit Tauri 事件。
//!
//! 拆分来源：`commands/chat_loop.rs` 的 `stream_loop` 函数
//! - 流式消费 → `stream_consumer::consume_stream`（emit chat:chunk/thinking/tool-call-*）
//! - 工具执行 → `tool_executor::execute_tool_round`（emit chat:tool-result）
//! - 主循环骨架 → 本模块（emit chat:retrying / chat:error + DB 回写）
//!
//! W4.1: `stream_loop` 签名增加 `budget: LoopBudget` 参数；原硬编码常量
//! `MAX_TOOL_ROUNDS` / `MAX_ATTEMPTS` 改为读取 budget 字段。
//! W4.2: budget.max_total_tokens 启用 Token 预算终止逻辑。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use sqlx::SqlitePool;

use crate::commands::chat_cleanup::{cleanup, cleanup_after_success_with_blocks};
use crate::commands::chat_error::{error_kind, friendly_error};
use crate::db::repo;
use crate::error::AppError;
use crate::infra::protocol::{
    ChatErrorPayload, ChatMessage, ChatRetryingPayload, ContentBlock, LlmProvider, TokenUsage,
};
use crate::harness::budget::LoopBudget;
use crate::harness::chat_state::CancellationToken;
use crate::harness::observable::{RoundState, RoundTimer};
use crate::harness::retry::{RetryContext, RetryState};
use crate::harness::tool_registry::ToolRegistry;

use super::stream_consumer::{consume_stream, CollectedToolCall};
use super::tool_executor::execute_tool_round;

// W2.6: 将 AppError 分类为 retry reason 字符串
fn classify_retry_reason(e: &AppError) -> String {
    use AppError::*;
    let msg = match e {
        Llm(s) | Stream(s) | Internal(s) | Stronghold(s) => s.as_str(),
        Io(_) => return "network_error".into(),
        Tauri(s) => s.as_str(),
        _ => return "unknown_error".into(),
    };
    let lower = msg.to_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        "timeout".into()
    } else if lower.contains("rate_limit")
        || lower.contains("429")
        || lower.contains("too many requests")
    {
        "rate_limited".into()
    } else if lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("server_error")
        || lower.contains("internal server error")
        || lower.contains("upstream")
    {
        "server_error_5xx".into()
    } else if lower.contains("connection")
        || lower.contains("network")
        || lower.contains("dns")
        || lower.contains("refused")
        || lower.contains("broken pipe")
        || lower.contains("reset")
    {
        "network_error".into()
    } else {
        "unknown_error".into()
    }
}

/// 流式生成内部协程 — 支持指数退避重试 + 工具执行循环
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
    budget: LoopBudget,
    observable: &mut RoundState,
) {


    let mut all_text = String::new();
    let mut all_content_blocks: Vec<ContentBlock> = Vec::new();
    let mut collected_usage: Option<TokenUsage> = None;

    // === 工具执行循环 ===
    for tool_round in 0..budget.max_tool_rounds {
        if cancel.is_cancelled() {
            return cleanup(&app, &pool, &conv_id);
        }

        let round_timer = RoundTimer::new(tool_round);
        observable.round = tool_round + 1;

        let tools: Option<Vec<crate::infra::protocol::ToolDef>> = if tools_enabled {
            Some(tool_registry.list_tool_defs().await)
        } else {
            None
        };

        let mut round_text = String::new();
        let mut round_think = String::new();
        let mut round_finish_reason = "stop".to_string();
        let mut tool_calls_map: HashMap<String, CollectedToolCall> = HashMap::new();
        let mut round_success = false;

        // === RetryState 驱动的重试循环 ===
        let mut retry_state = RetryState::new();
        let mut last_retry_reason = String::new();

        'retry_loop: loop {
            if !retry_state.can_retry() {
                break;
            }
            if cancel.is_cancelled() {
                return cleanup(&app, &pool, &conv_id);
            }

            let ws = retry_state.wait_secs();
            if ws > 0 {
                tracing::info!(
                    target: "ice_paw.chat",
                    "重试 LLM 请求: tool_round={} attempt={}/{}，等待 {}s",
                    tool_round,
                    retry_state.attempt_num() + 1,
                    budget.max_attempts,
                    ws,
                );
                observable.retry_count += 1;
                let _ = app.emit(
                    "chat:retrying",
                    ChatRetryingPayload {
                        conversation_id: conv_id.clone(),
                        message_id: asst_msg_id.clone(),
                        attempt: retry_state.attempt_num() + 1,
                        max_attempts: budget.max_attempts,
                        reason: last_retry_reason.clone(),
                    },
                );
                tokio::time::sleep(Duration::from_secs(ws)).await;
                if cancel.is_cancelled() {
                    return cleanup(&app, &pool, &conv_id);
                }
            }

            let retry_ctx = RetryContext::with_round_text(messages.clone(), round_text.clone());
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
                    match consume_stream(
                        &mut stream,
                        &app,
                        &cancel,
                        observable,
                        &conv_id,
                        &asst_msg_id,
                    )
                    .await
                    {
                        Ok(sr) => {
                            round_text = sr.text;
                            round_think = sr.think;
                            round_finish_reason = sr.finish_reason;
                            tool_calls_map = sr.tool_calls;
                            if let Some(u) = sr.usage {
                                collected_usage = Some(u);
                            }
                            round_success = true;
                            break 'retry_loop;
                        }
                        Err(e) => {
                            if e.is_retryable() {
                                last_retry_reason = classify_retry_reason(&e);
                                tracing::warn!(
                                    target: "ice_paw.chat",
                                    "流中可重试错误 (round={} attempt={}/{}): {}",
                                    tool_round,
                                    retry_state.attempt_num() + 1,
                                    budget.max_attempts,
                                    e
                                );
                                retry_state = retry_state
                                    .next_retry(budget.max_attempts, 1u64 << retry_state.attempt_num());
                                continue;
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
                                let _ =
                                    repo::message::update_error(&pool, &asst_msg_id, &err_msg)
                                        .await;
                                return cleanup(&app, &pool, &conv_id);
                            }
                        }
                    }
                }
                Err(e) => {
                    if e.is_retryable() {
                        last_retry_reason = classify_retry_reason(&e);
                        tracing::warn!(
                            target: "ice_paw.chat",
                            "请求失败可重试 (round={} attempt={}/{}): {}",
                            tool_round,
                            retry_state.attempt_num() + 1,
                            budget.max_attempts,
                            e
                        );
                        retry_state = retry_state
                            .next_retry(budget.max_attempts, 1u64 << retry_state.attempt_num());
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
            let err_msg = format!("连接重试已耗尽（共 {} 次），已收到部分内容", budget.max_attempts);
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

        observable.elapsed_ms = round_timer.elapsed_ms();
        all_text.push_str(&round_text);

        if !round_think.is_empty() {
            all_content_blocks.push(ContentBlock::Thinking {
                thinking: round_think,
                signature: None,
            });
        }

        let completed_calls: Vec<(String, String, String)> = tool_calls_map
            .into_values()
            .filter(|tc| tc.ended)
            .map(|tc| (tc.id, tc.name, tc.arguments))
            .collect();

        if completed_calls.is_empty() {
            let content_for_db = all_text.clone();
            if !all_text.is_empty() {
                all_content_blocks.push(ContentBlock::Text { text: all_text });
            }
            return cleanup_after_success_with_blocks(
                &app,
                &pool,
                &conv_id,
                &asst_msg_id,
                &content_for_db,
                &all_content_blocks,
                &round_finish_reason,
                collected_usage,
            );
        }

        tracing::info!(
            target: "ice_paw.chat",
            "工具调用循环: round={} tool_count={}",
            tool_round,
            completed_calls.len(),
        );

        let (tool_use_blocks, tool_result_blocks) =
            execute_tool_round(&app, &tool_registry, &completed_calls, &conv_id, &asst_msg_id)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(target: "ice_paw.chat", "工具执行失败: {}", e);
                    (Vec::new(), Vec::new())
                });

        all_content_blocks.extend(tool_use_blocks.clone());
        all_content_blocks.extend(tool_result_blocks.clone());

        let mut asst_blocks: Vec<ContentBlock> = Vec::new();
        if !round_text.is_empty() {
            asst_blocks.push(ContentBlock::Text { text: round_text });
        }
        asst_blocks.extend(tool_use_blocks);
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: asst_blocks,
        });

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

    let content_for_db = all_text.clone();
    if !all_text.is_empty() {
        all_content_blocks.push(ContentBlock::Text { text: all_text });
    }
    cleanup_after_success_with_blocks(
        &app,
        &pool,
        &conv_id,
        &asst_msg_id,
        &content_for_db,
        &all_content_blocks,
        "tool_use",
        collected_usage,
    );
}
