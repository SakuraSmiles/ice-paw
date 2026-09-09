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
//! - `can_retry()` 耗尽 → [`RoundStreamResult::RetryExhausted`]（携带最后一次失败原因）
//!
//! B2-S3 起叠加**降级链三拦截点**（换档执行在 [`super::fallback::try_switch_model`]，
//! 分类表见 [`super::fallback::fallback_trigger`]）：Quota 族在两处不可重试分支
//! emit_round_error **之前**拦截（确定性失败不退避白等；成功 = 零错误终态）；
//! RateLimited/Network 在 loop-top 耗尽返回 RetryExhausted 之前拦截（退避可能
//! 自愈，耗尽才换）。换档成功 → `RetryState::new()` + continue，等效全新一轮。
//! 无链 / 链尽 → 三点全部落到原终态路径，行为与 legacy 逐字节一致。

use std::time::Duration;

use crate::error::classify_llm_error;
use crate::harness::cleanup::emit_round_error;
use crate::harness::error_mapping::error_kind;
use crate::harness::event_log::EventCtx;
use crate::harness::observable::RoundState;
use crate::harness::retry::{RetryContext, RetryState};
use crate::harness::stream_consumer::{consume_stream, StreamResult};
use crate::infra::protocol::{ChatMessage, ChatRetryingPayload, ToolDef};

use super::context::LoopContext;
use super::fallback::{fallback_trigger, try_switch_model};
use super::reason::classify_retry_reason;

/// 三拦截点共用：错误文本重分类 → 换档 → 成功则重置退避并清失败原因残留。
///
/// 分类从错误文本重算（loop-top 拦截点③时错误对象已丢，`last_error_text`
/// 即原文）——与 `AppError::is_retryable` 同源于 [`classify_llm_error`]，
/// 结论不会分叉。返回 false = 不触发换档 / 链尽，调用方走原终态路径。
///
/// `last_error_text` 刻意不清：它只在点③被读，而能到达点③的前提是新的可重试
/// 失败已把它覆写（成功换档后 round 直接开跑，不经过 loop-top 的耗尽分支）。
async fn switch_and_reset(
    ctx: &mut LoopContext,
    retry_state: &mut RetryState,
    last_retry_reason: &mut String,
    error_text: &str,
    switch_attempt: &mut u32,
    current_asst_msg_id: &str,
) -> bool {
    let kind = classify_llm_error(error_text);
    let Some(trigger) = fallback_trigger(kind) else {
        return false;
    };
    *switch_attempt += 1;
    if !try_switch_model(
        ctx,
        current_asst_msg_id,
        trigger,
        error_text,
        *switch_attempt,
    )
    .await
    {
        return false;
    }
    *retry_state = RetryState::new();
    last_retry_reason.clear();
    true
}

/// `stream_with_retry` 的结果：本轮流式的归宿。
pub(crate) enum RoundStreamResult {
    /// 流式成功（含本轮 usage，调用方负责 token 累加）。
    Ok(StreamResult),
    /// 重试耗尽（consume_stream 始终失败，round_text 仍空）。
    /// `last_reason` = 最后一次失败原因 slug（B2-S1 升载荷，终态文案用）。
    RetryExhausted { last_reason: String },
    /// 已完成自身收尾（cancel 或不可重试错误已 emit chat:error+update_error），
    /// 调用方只需 `return finalize_cancel(...)`。
    Aborted,
}

/// 带退避重试地拉取一轮 LLM 流。
///
/// `round_injected` 为 BeforeLlm 钩子注入的临时 system 消息（每轮由调用方算好传入，
/// 不随网络重试重复触发）；`tool_round` 仅用于日志。`round_text` 在重试期间恒为空
/// （仅成功分支会产出文本，且成功即返回），用于构造 `RetryContext`。
/// ctx 取 `&mut`（B2-S1 起）：降级链（B2-S3）在重试循环内换档——替换
/// LoopContext 上的运行时模型档位字段。
pub(crate) async fn stream_with_retry(
    ctx: &mut LoopContext,
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
    // 最后一次可重试失败原文（B2-S3 点③用——loop-top 时错误对象已丢，
    // 从文本重分类；换档语义见 switch_and_reset 注释）
    let mut last_error_text = String::new();
    // 本回合换档序号（1 起，进 model_switch 事件的 attempt 字段）
    let mut switch_attempt: u32 = 0;
    // session-events（Phase 0）：message_error 事件上下文（conv/turn/agent）。
    let ev = EventCtx::new(&ctx.conv_id, &ctx.user_msg_id, &ctx.agent_id);

    loop {
        if !retry_state.can_retry() {
            // B2-S3 拦截点③：RateLimited/Network 退避耗尽 → 换档重开
            //（退避可能自愈，耗尽才轮到链；last_error_text 空则分类 Unknown 不触发）
            if switch_and_reset(
                ctx,
                &mut retry_state,
                &mut last_retry_reason,
                &last_error_text,
                &mut switch_attempt,
                current_asst_msg_id,
            )
            .await
            {
                continue;
            }
            return RoundStreamResult::RetryExhausted {
                last_reason: last_retry_reason,
            };
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
            super::emitter::emit_ser(
                ctx.emitter.as_ref(),
                "chat:retrying",
                &ChatRetryingPayload {
                    conversation_id: ctx.conv_id.clone(),
                    message_id: current_asst_msg_id.to_string(),
                    attempt: retry_state.attempt_num() + 1,
                    max_attempts: ctx.budget.max_attempts,
                    reason: last_retry_reason.clone(),
                },
            );
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
                    ctx.emitter.as_ref(),
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
                        // B2-S3：健康记录成功档（决策 6——批 1 移交的 agent 主链
                        // 健康监控；legacy 手动主档 active=None → no-op）
                        crate::harness::profile_health::record_ok(
                            Some(&ctx.pool),
                            ctx.fallback.active_profile_id.as_deref(),
                        )
                        .await;
                        return RoundStreamResult::Ok(sr);
                    }
                    Err(e) => {
                        if e.is_retryable() {
                            last_retry_reason = classify_retry_reason(&e);
                            last_error_text = e.to_string();
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
                            // B2-S3 拦截点①：Quota 族确定性失败不退避白等——
                            // emit_round_error **之前**先试换档（成功 = 零错误终态，
                            // 绝不能先 emit 错误再换档）
                            let err_msg = e.to_string();
                            if switch_and_reset(
                                ctx,
                                &mut retry_state,
                                &mut last_retry_reason,
                                &err_msg,
                                &mut switch_attempt,
                                current_asst_msg_id,
                            )
                            .await
                            {
                                continue;
                            }
                            emit_round_error(
                                ctx.emitter.as_ref(),
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
                    last_error_text = e.to_string();
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
                    // B2-S3 拦截点②（与点①同构，另一条错误出口）
                    let err_msg = e.to_string();
                    if switch_and_reset(
                        ctx,
                        &mut retry_state,
                        &mut last_retry_reason,
                        &err_msg,
                        &mut switch_attempt,
                        current_asst_msg_id,
                    )
                    .await
                    {
                        continue;
                    }
                    emit_round_error(
                        ctx.emitter.as_ref(),
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
