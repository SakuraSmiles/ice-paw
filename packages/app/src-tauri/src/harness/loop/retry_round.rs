//! 单轮流式 + 指数退避重试：从 `loop_engine::stream_loop_inner` 的 `'retry_loop` 抽出。
//!
//! 职责：在 [`RetryState`] 驱动下最多重试 `budget.max_attempts` 次地拉取一轮 LLM 流，
//! 把结果归类为 [`RoundStreamResult`] 交回主循环。纯"取一轮"，不含工具执行 / 持久化 /
//! 停滞检测（那些留在 `stream_loop_inner`）。
//!
//! 退出路径与原内联重试循环逐条等价：
//! - cancel（顶部 / sleep 后）→ [`RoundStreamResult::Aborted`]（无错误 emit，调用方 finalize_cancel）
//! - consume_stream / stream_chat 不可重试错误 → 内部 [`emit_round_error`] 后 [`RoundStreamResult::Aborted`]
//! - consume_stream 成功 → [`RoundStreamResult::Ok`]（token 累加由调用方处理）
//! - `can_retry()` 耗尽 → [`RoundStreamResult::RetryExhausted`]

use std::time::Duration;

use tauri::Emitter;

use crate::harness::cleanup::emit_round_error;
use crate::harness::error_mapping::error_kind;
use crate::harness::event_log::EventCtx;
use crate::harness::observable::RoundState;
use crate::harness::retry::{RetryContext, RetryState};
use crate::harness::stream_consumer::{consume_stream, StreamResult};
use crate::infra::protocol::{ChatMessage, ChatRetryingPayload, ToolDef};

use super::context::LoopContext;
use super::reason::classify_retry_reason;

/// `stream_with_retry` 的结果：本轮流式的归宿。
pub(crate) enum RoundStreamResult {
    /// 流式成功（含本轮 usage，调用方负责 token 累加）。
    Ok(StreamResult),
    /// 重试耗尽（consume_stream 始终失败，round_text 仍空）。
    RetryExhausted,
    /// 已完成自身收尾（cancel 或不可重试错误已 emit chat:error+update_error），
    /// 调用方只需 `return finalize_cancel(...)`。
    Aborted,
}

/// 带退避重试地拉取一轮 LLM 流。
///
/// `round_injected` 为 BeforeLlm 钩子注入的临时 system 消息（每轮由调用方算好传入，
/// 不随网络重试重复触发）；`tool_round` 仅用于日志。`round_text` 在重试期间恒为空
/// （仅成功分支会产出文本，且成功即返回），用于构造 `RetryContext`。
pub(crate) async fn stream_with_retry(
    ctx: &LoopContext,
    observable: &mut RoundState,
    tool_round: u32,
    tools: Option<Vec<ToolDef>>,
    round_injected: Option<String>,
    current_asst_msg_id: &str,
) -> RoundStreamResult {
    // round_text 在重试期间恒为空（仅成功分支产出文本，且成功即返回）。
    let round_text = String::new();
    let mut retry_state = RetryState::new();
    let mut last_retry_reason = String::new();
    // session-events（Phase 0）：message_error 事件上下文（conv/turn/agent）。
    let ev = EventCtx::new(&ctx.conv_id, &ctx.user_msg_id, &ctx.agent_id);

    loop {
        if !retry_state.can_retry() {
            return RoundStreamResult::RetryExhausted;
        }
        if ctx.cancel.is_cancelled() {
            return RoundStreamResult::Aborted;
        }

        let ws = retry_state.wait_secs();
        if ws > 0 {
            tracing::info!(
                target: "ice_paw.chat",
                "重试 LLM 请求: tool_round={} attempt={}/{}，等待 {}s",
                tool_round,
                retry_state.attempt_num() + 1,
                ctx.budget.max_attempts,
                ws,
            );
            observable.retry_count += 1;
            if let Err(e) = ctx.app.emit(
                "chat:retrying",
                ChatRetryingPayload {
                    conversation_id: ctx.conv_id.clone(),
                    message_id: current_asst_msg_id.to_string(),
                    attempt: retry_state.attempt_num() + 1,
                    max_attempts: ctx.budget.max_attempts,
                    reason: last_retry_reason.clone(),
                },
            ) {
                tracing::warn!(
                    target: "ice_paw.chat",
                    "emit chat:retrying 失败: conv_id={}, err={}",
                    ctx.conv_id,
                    e
                );
            }
            tokio::time::sleep(Duration::from_secs(ws)).await;
            if ctx.cancel.is_cancelled() {
                return RoundStreamResult::Aborted;
            }
        }

        let retry_ctx = RetryContext::with_round_text(ctx.messages.clone(), round_text.clone());
        let retry_messages = retry_state.prepare_messages(&retry_ctx);

        // 追加 BeforeLlm 钩子注入的临时 system 消息（若有）；不写回 ctx.messages。
        let mut send_messages = retry_messages;
        if let Some(inj) = &round_injected {
            send_messages.push(ChatMessage::from_text("system", inj.clone()));
        }

        let stream_result = ctx
            .provider
            .stream_chat(
                &ctx.api_key,
                send_messages,
                tools.clone(),
                ctx.temperature,
                ctx.max_tokens,
                ctx.model.as_deref(),
                ctx.cancel.clone(),
            )
            .await;

        match stream_result {
            Ok(mut stream) => {
                match consume_stream(
                    &mut stream,
                    &ctx.app,
                    &ctx.cancel,
                    observable,
                    &ctx.conv_id,
                    current_asst_msg_id,
                )
                .await
                {
                    Ok(sr) => {
                        // token_count 由本轮 finalize_assistant_message 即时写入
                        //（每条 assistant 独立持有本轮 completion_tokens）；token 累加由调用方处理。
                        return RoundStreamResult::Ok(sr);
                    }
                    Err(e) => {
                        if e.is_retryable() {
                            last_retry_reason = classify_retry_reason(&e);
                            tracing::warn!(
                                target: "ice_paw.chat",
                                "流中可重试错误 (round={} attempt={}/{}): {}",
                                tool_round,
                                retry_state.attempt_num() + 1,
                                ctx.budget.max_attempts,
                                e
                            );
                            retry_state = retry_state.next_retry(
                                ctx.budget.max_attempts,
                                1u64 << retry_state.attempt_num(),
                            );
                            continue;
                        } else {
                            let err_msg = e.to_string();
                            emit_round_error(
                                &ctx.app,
                                &ctx.pool,
                                &ev,
                                current_asst_msg_id,
                                &error_kind(&e),
                                &err_msg,
                            )
                            .await;
                            return RoundStreamResult::Aborted;
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
                        ctx.budget.max_attempts,
                        e
                    );
                    retry_state = retry_state
                        .next_retry(ctx.budget.max_attempts, 1u64 << retry_state.attempt_num());
                } else {
                    let err_msg = e.to_string();
                    emit_round_error(
                        &ctx.app,
                        &ctx.pool,
                        &ev,
                        current_asst_msg_id,
                        &error_kind(&e),
                        &err_msg,
                    )
                    .await;
                    return RoundStreamResult::Aborted;
                }
            }
        }
    }
}
