//! L2 Loop Engine — 主循环调度（W3.3 + W4.1 + W6.2）
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
//! W6.2: 把 `stream_loop` 的 13 个输入参数封装到 `LoopContext` 结构体，
//! 消除 `clippy::too_many_arguments` 告警；`observable` 作为单独的
//! `&mut RoundState` 入参保留（属于输出遥测，不属于输入配置）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use sqlx::SqlitePool;

use crate::harness::cleanup::{cleanup, cleanup_after_success_with_blocks};
use crate::harness::error_mapping::{error_kind, friendly_error};
use crate::db::repo;
use crate::error::AppError;
use crate::infra::protocol::{
    ChatErrorPayload, ChatMessage, ChatRetryingPayload, ContentBlock, LlmProvider, TokenUsage,
};
use crate::harness::budget::LoopBudget;
use crate::harness::chat_state::CancellationToken;
use crate::harness::observable::{RoundState, RoundTimer};
use crate::harness::retry::{RetryContext, RetryState};
use crate::harness::tool_registry::{
    authority::{PathAuthSession, PathWhitelistConfig},
    ToolRegistry,
};

use super::stream_consumer::{consume_stream, CollectedToolCall};
use super::tool_executor::{execute_tool_round, ToolAuthRegistry};

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

// ==========================================================================
// W6.2: LoopContext — 流式循环的输入配置封装
// ==========================================================================

/// `stream_loop` 的输入配置封装。
///
/// 13 个原本独立的参数（app / pool / provider / api_key / messages /
/// temperature / max_tokens / cancel / conv_id / asst_msg_id /
/// tool_registry / tools_enabled / budget）整合到一个结构体中：
/// - 消除 `clippy::too_many_arguments`
/// - 让 `stream_loop` 的 signature 保持 `fn(&mut LoopContext, &mut RoundState)`
/// - 为后续扩展（如加上 tools 缓存、agent 配置、continue-from 等）提供容器
///
/// `RoundState`（observable）刻意未收入此结构体，因为它是循环过程中
/// 累积写入的**输出**遥测状态，而不是配置输入。
#[allow(clippy::too_many_arguments)]
pub(crate) struct LoopContext {
    // ---- 标识与会话 ----
    pub conv_id: String,
    pub asst_msg_id: String,

    // ---- 基础设施 ----
    pub app: AppHandle,
    pub pool: SqlitePool,

    // ---- LLM Provider ----
    pub provider: Arc<dyn LlmProvider>,
    pub api_key: String,
    pub temperature: f64,
    pub max_tokens: i32,

    // ---- 对话消息缓冲（循环中会 push 新消息） ----
    pub messages: Vec<ChatMessage>,

    // ---- 工具 ----
    pub tool_registry: ToolRegistry,
    pub tools_enabled: bool,
    /// A2-3: 工具授权响应全局注册表（前端响应 → Rust oneshot 解锁）
    pub auth_registry: ToolAuthRegistry,
    /// A2-3: 本次会话已授权路径表（同一会话内用户允许过的路径不再弹窗）
    pub auth_session: PathAuthSession,
    /// A2-3: 路径白名单配置
    pub whitelist: PathWhitelistConfig,

    // ---- 循环控制 ----
    pub cancel: CancellationToken,
    pub budget: LoopBudget,
}

impl LoopContext {
    /// 构造 `LoopContext`。这是 W6.2 引入的唯一构造入口。
    ///
    /// 参数数量看似很多，但这就是该结构体的全部职责 —— 把原本散落在
    /// `stream_loop` 形参列表里的 13 个字段集中起来。允许
    /// `clippy::too_many_arguments` 因为这就是本结构体的存在意义。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        conv_id: String,
        asst_msg_id: String,
        app: AppHandle,
        pool: SqlitePool,
        provider: Arc<dyn LlmProvider>,
        api_key: String,
        temperature: f64,
        max_tokens: i32,
        messages: Vec<ChatMessage>,
        tool_registry: ToolRegistry,
        tools_enabled: bool,
        cancel: CancellationToken,
        budget: LoopBudget,
        auth_registry: ToolAuthRegistry,
        auth_session: PathAuthSession,
        whitelist: PathWhitelistConfig,
    ) -> Self {
        Self {
            conv_id,
            asst_msg_id,
            app,
            pool,
            provider,
            api_key,
            temperature,
            max_tokens,
            messages,
            tool_registry,
            tools_enabled,
            auth_registry,
            auth_session,
            whitelist,
            cancel,
            budget,
        }
    }
}

/// 流式生成内部协程 — 支持指数退避重试 + 工具执行循环
///
/// W6.2: 13 个输入参数已封装到 [`LoopContext`]，仅保留
/// `observable`（输出遥测状态）作为单独的 `&mut RoundState` 入参。
///
/// A2-3: 外层 wrapper 负责在任意退出路径清空会话级授权表；
///       `stream_loop_inner` 才是真正的循环主体。
pub(crate) async fn stream_loop(ctx: &mut LoopContext, observable: &mut RoundState) {
    stream_loop_inner(ctx, observable).await;
    // A2-3: 不论正常结束 / 取消 / 错误，都清空会话级授权表
    ctx.auth_session.clear().await;
}

/// 流式循环主体（A2-3 后被 `stream_loop` wrapper 包裹）
async fn stream_loop_inner(ctx: &mut LoopContext, observable: &mut RoundState) {
    let mut all_text = String::new();
    let mut all_content_blocks: Vec<ContentBlock> = Vec::new();
    let mut collected_usage: Option<TokenUsage> = None;

    // W4.2: Token 预算累计追踪
    let mut cumulative_tokens: usize = 0;

    // === 工具执行循环 ===
    for tool_round in 0..ctx.budget.max_tool_rounds {
        if ctx.cancel.is_cancelled() {
            return cleanup(&ctx.app, &ctx.pool, &ctx.conv_id);
        }

        let round_timer = RoundTimer::new(tool_round);
        observable.round = tool_round + 1;

        let tools: Option<Vec<crate::infra::protocol::ToolDef>> = if ctx.tools_enabled {
            Some(ctx.tool_registry.list_tool_defs().await)
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
            if ctx.cancel.is_cancelled() {
                return cleanup(&ctx.app, &ctx.pool, &ctx.conv_id);
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
                let _ = ctx.app.emit(
                    "chat:retrying",
                    ChatRetryingPayload {
                        conversation_id: ctx.conv_id.clone(),
                        message_id: ctx.asst_msg_id.clone(),
                        attempt: retry_state.attempt_num() + 1,
                        max_attempts: ctx.budget.max_attempts,
                        reason: last_retry_reason.clone(),
                    },
                );
                tokio::time::sleep(Duration::from_secs(ws)).await;
                if ctx.cancel.is_cancelled() {
                    return cleanup(&ctx.app, &ctx.pool, &ctx.conv_id);
                }
            }

            let retry_ctx = RetryContext::with_round_text(ctx.messages.clone(), round_text.clone());
            let retry_messages = retry_state.prepare_messages(&retry_ctx);

            let stream_result = ctx
                .provider
                .stream_chat(
                    &ctx.api_key,
                    retry_messages,
                    tools.clone(),
                    ctx.temperature,
                    ctx.max_tokens,
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
                        &ctx.asst_msg_id,
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
                                    ctx.budget.max_attempts,
                                    e
                                );
                                retry_state = retry_state
                                    .next_retry(ctx.budget.max_attempts, 1u64 << retry_state.attempt_num());
                                continue;
                            } else {
                                let err_msg = e.to_string();
                                let _ = ctx.app.emit(
                                    "chat:error",
                                    ChatErrorPayload {
                                        conversation_id: ctx.conv_id.clone(),
                                        message_id: ctx.asst_msg_id.clone(),
                                        kind: error_kind(&e),
                                        message: friendly_error(&err_msg),
                                    },
                                );
                                let _ = repo::message::update_error(
                                    &ctx.pool,
                                    &ctx.asst_msg_id,
                                    &err_msg,
                                )
                                .await;
                                return cleanup(&ctx.app, &ctx.pool, &ctx.conv_id);
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
                        let _ = ctx.app.emit(
                            "chat:error",
                            ChatErrorPayload {
                                conversation_id: ctx.conv_id.clone(),
                                message_id: ctx.asst_msg_id.clone(),
                                kind: error_kind(&e),
                                message: friendly_error(&err_msg),
                            },
                        );
                        let _ = repo::message::update_error(&ctx.pool, &ctx.asst_msg_id, &err_msg)
                            .await;
                        return cleanup(&ctx.app, &ctx.pool, &ctx.conv_id);
                    }
                }
            }
        }

        if !round_success {
            let err_msg = format!(
                "连接重试已耗尽（共 {} 次），已收到部分内容",
                ctx.budget.max_attempts
            );
            if !round_text.is_empty() {
                let _ = repo::message::update_content(&ctx.pool, &ctx.asst_msg_id, &round_text)
                    .await;
            }
            let _ = repo::message::update_error(&ctx.pool, &ctx.asst_msg_id, &err_msg).await;
            let _ = ctx.app.emit(
                "chat:error",
                ChatErrorPayload {
                    conversation_id: ctx.conv_id.clone(),
                    message_id: ctx.asst_msg_id.clone(),
                    kind: "stream".into(),
                    message: friendly_error(&err_msg),
                },
            );
            return cleanup(&ctx.app, &ctx.pool, &ctx.conv_id);
        }

        observable.elapsed_ms = round_timer.elapsed_ms();
        all_text.push_str(&round_text);

        // W4.2: Token 预算累计 — 每个 round 结束后累加 usage
        if let Some(ref usage) = collected_usage {
            cumulative_tokens += usage.prompt_tokens as usize + usage.completion_tokens as usize;
        }

        // W4.2: Token 预算终止检查
        // 在工具调用继续下一轮之前检查：如果累计 token 超过预算，
        // 优雅终止循环并设置 finish_reason = "budget_exceeded"
        if ctx.budget.max_total_tokens != usize::MAX
            && cumulative_tokens > ctx.budget.max_total_tokens
        {
            tracing::warn!(
                target: "ice_paw.chat",
                "Token 预算已超限: cumulative={} > budget={}",
                cumulative_tokens,
                ctx.budget.max_total_tokens,
            );
            let content_for_db = all_text.clone();
            if !all_text.is_empty() {
                all_content_blocks.push(ContentBlock::Text { text: all_text });
            }
            return cleanup_after_success_with_blocks(
                &ctx.app,
                &ctx.pool,
                &ctx.conv_id,
                &ctx.asst_msg_id,
                &content_for_db,
                &all_content_blocks,
                "budget_exceeded",
                collected_usage,
            );
        }

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
                &ctx.app,
                &ctx.pool,
                &ctx.conv_id,
                &ctx.asst_msg_id,
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

        let (tool_use_blocks, tool_result_blocks) = execute_tool_round(
            &ctx.app,
            &ctx.tool_registry,
            &ctx.auth_registry,
            &ctx.auth_session,
            &ctx.whitelist,
            &completed_calls,
            &ctx.conv_id,
            &ctx.asst_msg_id,
            &ctx.cancel,
        )
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
        ctx.messages.push(ChatMessage {
            role: "assistant".into(),
            content: asst_blocks,
        });

        for block in &tool_result_blocks {
            ctx.messages.push(ChatMessage {
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
        &ctx.app,
        &ctx.pool,
        &ctx.conv_id,
        &ctx.asst_msg_id,
        &content_for_db,
        &all_content_blocks,
        "tool_use",
        collected_usage,
    );
}

// ==========================================================================
// W4.2 单元测试 — Token 预算终止逻辑
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::protocol::TokenUsage;

    /// 验证：默认预算（128_000）不会意外触发终止
    #[test]
    fn test_budget_not_exceeded_with_default() {
        let budget = LoopBudget::default();
        assert_eq!(budget.max_total_tokens, 128_000);
        // 模拟一个 round 使用了 5000 tokens → 远低于 128_000
        let cumulative_tokens: usize = 5_000;
        let exceeded = budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
        assert!(!exceeded, "默认预算不应在 5000 tokens 时触发终止");
    }

    /// 验证：自定义小预算在超限时正确标记 exceeded
    #[test]
    fn test_budget_exceeded_with_small_limit() {
        let budget = LoopBudget {
            max_tool_rounds: 5,
            max_attempts: 4,
            stuck_threshold: 3,
            max_total_tokens: 1_000,
        };
        // 模拟 round 1 用了 800 tokens，round 2 累计到 1600 → 超过 1000
        let mut cumulative_tokens: usize = 800;
        let exceeded_1 = budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
        assert!(!exceeded_1, "800 tokens 不应超过 1000 预算");

        cumulative_tokens += 800; // 1600
        let exceeded_2 = budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
        assert!(exceeded_2, "1600 tokens 应超过 1000 预算");
    }

    /// 验证：usize::MAX 预算永远不触发终止（无限模式）
    #[test]
    fn test_budget_unlimited_never_exceeds() {
        let budget = LoopBudget {
            max_tool_rounds: 5,
            max_attempts: 4,
            stuck_threshold: 3,
            max_total_tokens: usize::MAX,
        };
        // 模拟极端大的累计值
        let cumulative_tokens: usize = usize::MAX - 1;
        let exceeded = budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
        assert!(!exceeded, "usize::MAX 预算永远不应触发终止");
    }

    /// 验证：TokenUsage 累加准确性
    #[test]
    fn test_token_accumulation_accuracy() {
        let u1 = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            cached_tokens: 10,
        };
        let u2 = TokenUsage {
            prompt_tokens: 200,
            completion_tokens: 80,
            cached_tokens: 20,
        };
        let mut cumulative: usize = 0;
        cumulative += u1.prompt_tokens as usize + u1.completion_tokens as usize;
        cumulative += u2.prompt_tokens as usize + u2.completion_tokens as usize;
        assert_eq!(cumulative, 430, "累计应为 100+50+200+80=430");
    }
}
