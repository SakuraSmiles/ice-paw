//! L2 Loop Engine — 主循环调度（W3.3 + W4.1 + W6.2）
//!
//! ## 架构
//!
//! ```text
//! stream_loop() wrapper
//!   ├─ BatchWriter::spawn()
//!   ├─ stream_loop_inner()
//!   │   ├─ 'tool_round: 工具执行循环（最多 max_tool_rounds 轮）
//!   │   │   ├─ cancel 检查
//!   │   │   ├─ list_tool_defs_with_query()   // 动态工具评分
//!   │   │   ├─ 'retry_loop: 重试循环（最多 MAX_ATTEMPTS 次）
//!   │   │   │   ├─ provider.stream_chat()
//!   │   │   │   ├─ stream_consumer::consume_stream()
//!   │   │   │   ├─ 重试分类: classify_retry_reason()
//!   │   │   │   └─ 失败 → 指数退避 → continue 'retry_loop
//!   │   │   ├─ 停滞检测: compute_round_key() + should_terminate_stuck()
//!   │   │   ├─ 工具执行: execute_tool_round()
//!   │   │   ├─ 工具结果持久化 + 下轮 assistant 占位
//!   │   │   └─ Token 预算检查 → break 'tool_round
//!   │   └─ finalize_success / finalize_error / finalize_cancel
//!   ├─ BatchWriter::shutdown()
//!   └─ auth_session.clear()
//! ```
//!
//! ## 职责
//!
//! 编排工具执行循环（tool_round loop）+ 重试循环（retry loop），
//! 调用 `stream_consumer::consume_stream` 消费 LLM 流，
//! 调用 `tool_executor::execute_tool_round` 执行工具，
//! 统一 emit Tauri 事件。
//!
//! ## 子模块
//!
//! - `r#loop::stuck_detect` — 停滞检测（compute_round_key + should_terminate_stuck）
//! - `r#loop::token_usage` — 多轮 usage 合成（synthesize_usage）
//!
//! ## 拆分历史
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
//!
//! M2.1: 停滞检测（B1-4）—— dev1 三级级联 L1 简化方案：
//!   用 64-bit hash 跟踪本轮累计文本 + 已完成工具调用 ID，
//!   连续 `stuck_threshold` 轮无变化时 emit `finish_reason="stuck"` 终止对话。
//!   检测点在外层 `for tool_round` 循环底部，不在 inner `'retry_loop` 中
//!   （重试是网络层行为，不构成"停滞"语义）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::harness::cleanup::{
    fail_round_and_cancel, finalize_assistant_message, finalize_cancel, finalize_success,
};
use crate::harness::error_mapping::error_kind;
use crate::db::models::{HookConfig, HookPoint, NewMessage};
use crate::db::repo;
use crate::infra::protocol::{
    ChatAssistantStartPayload, ChatMessage, ChatRetryingPayload, ContentBlock,
    LlmProvider, TokenUsage,
};
use crate::harness::budget::LoopBudget;
use crate::harness::chat_state::CancellationToken;
use crate::harness::hooks::{has_actions, run_hooks};
use crate::harness::observable::{RoundState, RoundTimer};
use crate::harness::retry::{RetryContext, RetryState};
use crate::harness::mcp::McpRegistry;
use crate::harness::authority::{PathAuthSession, PathWhitelistConfig};

use super::batch_writer;
use super::stream_consumer::{consume_stream, CollectedToolCall};
use super::tool_executor::{build_tool_ctx, execute_tool_round, ToolAuthRegistry};

// classify_retry_reason 已迁移到 crate::harness::r#loop::reason
use crate::harness::r#loop::reason::classify_retry_reason;

// emit_intermediate_round_state 已迁移到 crate::harness::r#loop::events
use crate::harness::r#loop::events::emit_intermediate_round_state;

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
/// 对话循环的不可变配置（从 LoopContext 拆分，消除 24 参数构造器）。
///
/// 创建后不被循环修改。通过 `LoopContext` 的 `Deref` 透明访问。
pub(crate) struct LoopConfig {
    // ---- 标识与会话 ----
    pub conv_id: String,
    pub asst_msg_id: String,
    /// M1.3: 用户消息 ID（用于清理阶段回写 token_count）
    pub user_msg_id: String,
    /// RAG: 当前 Agent ID（透传给 ToolContext）
    pub agent_id: String,
    /// RAG: 当前项目 ID
    pub project_id: Option<String>,

    // ---- 基础设施 ----
    pub app: AppHandle,
    pub pool: SqlitePool,

    // ---- LLM Provider ----
    pub provider: Arc<dyn LlmProvider>,
    pub api_key: String,
    pub temperature: f64,
    pub max_tokens: i32,

    // ---- 工具 ----
    pub tool_registry: McpRegistry,
    pub tools_enabled: bool,
    pub auth_registry: ToolAuthRegistry,
    pub auth_session: PathAuthSession,
    pub whitelist: PathWhitelistConfig,

    // ---- 循环控制 ----
    pub cancel: CancellationToken,
    pub budget: LoopBudget,

    // ---- M1.2: 工具裁剪 ----
    pub query: Option<String>,
    pub call_history: Vec<String>,

    // ---- P0-3: 会话级 model override ----
    pub model: Option<String>,
    pub asst_model: Option<String>,

    // ---- 对话钩子 ----
    pub hooks: HookConfig,
}

/// 对话循环上下文：不可变配置 + 可变消息缓冲。
///
/// 通过 `Deref<Target = LoopConfig>` 透明访问配置字段（`ctx.pool`、
/// `ctx.app` 等），`messages` 直接可变访问。构造时传入 `LoopConfig`。
pub(crate) struct LoopContext {
    pub config: LoopConfig,
    pub messages: Vec<ChatMessage>,
}

impl std::ops::Deref for LoopContext {
    type Target = LoopConfig;
    fn deref(&self) -> &LoopConfig {
        &self.config
    }
}

impl LoopContext {
    pub(crate) fn new(config: LoopConfig, messages: Vec<ChatMessage>) -> Self {
        Self { config, messages }
    }
}

// synthesize_usage 已迁移到 crate::harness::r#loop::token_usage
use crate::harness::r#loop::token_usage::synthesize_usage;

/// 流式生成内部协程 — 支持指数退避重试 + 工具执行循环
///
/// W6.2: 13 个输入参数已封装到 [`LoopContext`]，仅保留
/// `observable`（输出遥测状态）作为单独的 `&mut RoundState` 入参。
///
/// A2-3: 外层 wrapper 负责在任意退出路径清空会话级授权表；
///       `stream_loop_inner` 才是真正的循环主体。
///
/// REQ-XC-004: 外层 wrapper 同时负责关闭 BatchWriter，
///       确保所有路径（成功 / 取消 / 错误 / 超时）都能 final flush。
pub(crate) async fn stream_loop(ctx: &mut LoopContext, observable: &mut RoundState) {
    let (writer, handle) = batch_writer::BatchWriter::spawn(
        ctx.pool.clone(),
        ctx.asst_msg_id.clone(),
        batch_writer::DEFAULT_TICK_INTERVAL,
    );
    let writer_for_inner = writer.clone();
    stream_loop_inner(ctx, observable, writer_for_inner).await;
    // REQ-XC-004: 不论退出路径，都关闭 BatchWriter 触发 final flush
    writer.shutdown().await;
    let _ = handle.await;
    // A2-3: 不论正常结束 / 取消 / 错误，都清空会话级授权表
    ctx.auth_session.clear().await;

    // === 钩子：ConversationEnd（对话结束，所有退出路径——成功/取消/错误——都触发一次）===
    // 放在 stream_loop_inner 返回后（其内部 finalize_* 已 emit chat:done + 注销 token），
    // 保证整次对话恰好触发一次。Log/CallTool 用；失败仅 warn 不影响已完成的收尾。
    if has_actions(&ctx.hooks, HookPoint::ConversationEnd) {
        let hook_ctx = build_tool_ctx(
            &ctx.pool,
            ctx.conv_id.clone(),
            ctx.agent_id.clone(),
            ctx.project_id.clone(),
            Some(ctx.api_key.clone()),
        )
        .await;
        if let Err(e) = run_hooks(HookPoint::ConversationEnd, &ctx.hooks, &hook_ctx, &ctx.tool_registry).await {
            tracing::warn!(
                target: "ice_paw.hooks",
                "ConversationEnd 钩子执行失败（忽略）: {}", e
            );
        }
    }
}

/// M2.1: 计算本轮的"进度指纹"hash
///
/// 把 `all_text`（累计文本）和 `completed_calls` 的工具调用签名
///（`name:arguments` 字符串，由调用方在传入前 `sort_unstable()`）一起喂入
/// 64-bit hasher，产出一个稳定指纹。任何一项变化都会得到不同 hash。
///
/// 为什么用 hash 而不是直接字符串比较：
///   - 多轮工具调用后 `all_text` 可能累积数千字，逐字比较是 O(n²)
///   - 64-bit hasher 碰撞概率 ~1/2^64，足够鲁棒
///   - `DefaultHasher` 是 std 自带、无依赖
///
// compute_round_key + should_terminate_stuck 已迁移到 crate::harness::r#loop::stuck_detect
pub(crate) use crate::harness::r#loop::stuck_detect::{compute_round_key, should_terminate_stuck};

/// 流式循环主体（A2-3 后被 `stream_loop` wrapper 包裹）
///
/// REQ-XC-004: `batch_writer` 由外层 `stream_loop` 创建并传入；
/// 本函数内部在每轮 consume_stream 之后 push 一次最新全文，
/// 退出由外层 wrapper 统一 shutdown + final flush。
async fn stream_loop_inner(
    ctx: &mut LoopContext,
    observable: &mut RoundState,
    batch_writer: batch_writer::BatchWriter,
) {
    // 【彻底重构】每轮独立持久化，删除跨轮累积器（原 all_text / all_content_blocks）。
    //
    // `current_asst_msg_id`：循环内所有 emit / DB 写入的「唯一 id 源」。
    // 初始 = 首条 assistant（ctx.asst_msg_id，由 chat_cmd 创建），每轮工具结束后
    // 更新为下一轮的 assistant 占位 id。所有错误 / cancel / 成功路径都必须用它，
    // 绝不能用 ctx.asst_msg_id（那是首条，多轮工具下会标到错误的轮）。
    let mut current_asst_msg_id = ctx.asst_msg_id.clone();
    // `progress_text`：跨轮累积文本，仅供停滞检测使用，**不持久化**。
    // （每轮真实文本由 finalize_assistant_message 即时落盘到对应 assistant 消息。）
    let mut progress_text = String::new();
    let mut collected_usage: Option<TokenUsage> = None;

    // W4.2: Token 预算累计追踪
    let mut cumulative_tokens: usize = 0;

    // M1.3: Token 入库
    // - `first_prompt_tokens`：首次 provider 返回的 prompt_tokens，
    //   作为 user 消息的 token_count（整个 prompt 已包含历史）
    // - `total_completion_tokens`：所有轮的 completion_tokens 之和，
    //   作为 asst 消息的 token_count
    let mut first_prompt_tokens: Option<u32> = None;
    let mut total_completion_tokens: u32 = 0;

    // === M2.1: 停滞检测状态变量 ===
    // `stuck_counter`：连续未进展轮数（每轮结束时由 `should_terminate_stuck` 更新）
    // `last_round_hash`：上一轮的进度指纹 hash；首轮为 None（无论如何不视为停滞）
    let mut stuck_counter: u32 = 0;
    let mut last_round_hash: Option<u64> = None;

    // === 工具执行循环 ===
    for tool_round in 0..ctx.budget.max_tool_rounds {
        if ctx.cancel.is_cancelled() {
            return finalize_cancel(&ctx.app, &ctx.pool, &ctx.conv_id, &current_asst_msg_id);
        }

        let round_timer = RoundTimer::new(tool_round);
        observable.round = tool_round + 1;

        let tools: Option<Vec<crate::infra::protocol::ToolDef>> = if ctx.tools_enabled {
            // M1.2: 使用 query + call_history 做相关性打分与软裁剪
            // 阈值取默认 ContextBudget.tool_trim_threshold = Some(5)
            // 调用上下文循环中可能多次调用；这里每次都重新打分，确保准确反映当前 query
            Some(
                ctx.tool_registry
                    .list_tool_defs_with_query(
                        ctx.query.as_deref().unwrap_or(""),
                        crate::context::token::ContextBudget::default().tool_trim_threshold,
                        crate::context::token::ContextBudget::default().trim_top_k,
                        &ctx.call_history,
                    )
                    .await,
            )
        } else {
            None
        };

        let mut round_text = String::new();
        let mut round_think = String::new();
        let mut round_finish_reason = "stop".to_string();
        let mut tool_calls_map: HashMap<String, CollectedToolCall> = HashMap::new();
        let mut round_success = false;
        // 本轮 provider 返回的 completion_tokens（用于即时落盘该 assistant 的 token_count）
        let mut round_completion_tokens: Option<u32> = None;

        // 第 2 轮起，在消息中注入剩余轮次信息（帮助 LLM 决定是否继续调工具）
        if tool_round > 0 {
            ctx.messages.push(ChatMessage {
                role: "user".into(),
                content: vec![ContentBlock::text(format!(
                    "（第 {}/{} 轮工具调用完毕。如果还有未完成的操作请继续，如果已经完成请直接输出最终回答。）",
                    tool_round, ctx.budget.max_tool_rounds
                ))],
                source_rowid: None,
            });
        }

        // === 钩子：BeforeLlm（每轮 stream_chat 前注入临时 system 消息；核心——每轮强制规范）===
        // 注入的是「临时」消息：只加进本轮发给 provider 的消息，不写回 ctx.messages
        //（故不入 DB 历史、不跨轮累积），每轮重新注入。provider 适配层会把所有 system
        // 消息抽离合并到顶层 system_prompt，故追加在末尾即可生效。
        // 每轮（tool_round）只执行一次，不随网络重试重复触发。
        let round_injected: Option<String> = if has_actions(&ctx.hooks, HookPoint::BeforeLlm) {
            let hook_ctx = build_tool_ctx(
                &ctx.pool,
                ctx.conv_id.clone(),
                ctx.agent_id.clone(),
                ctx.project_id.clone(),
                Some(ctx.api_key.clone()),
            )
            .await;
            run_hooks(HookPoint::BeforeLlm, &ctx.hooks, &hook_ctx, &ctx.tool_registry)
                .await
                .ok()
                .and_then(|o| o.injected_prompt)
        } else {
            None
        };

        // === RetryState 驱动的重试循环 ===
        let mut retry_state = RetryState::new();
        let mut last_retry_reason = String::new();

        'retry_loop: loop {
            if !retry_state.can_retry() {
                break;
            }
            if ctx.cancel.is_cancelled() {
                return finalize_cancel(&ctx.app, &ctx.pool, &ctx.conv_id, &current_asst_msg_id);
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
                        message_id: current_asst_msg_id.clone(),
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
                    return finalize_cancel(&ctx.app, &ctx.pool, &ctx.conv_id, &current_asst_msg_id);
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
                        &current_asst_msg_id,
                    )
                    .await
                    {
                        Ok(sr) => {
                            round_text = sr.text;
                            round_think = sr.think;
                            round_finish_reason = sr.finish_reason;
                            tool_calls_map = sr.tool_calls;
                            if let Some(u) = sr.usage {
                                // M1.3: 累计 token —— 首次出现的 prompt_tokens 作为原始 user 消息 token_count
                                first_prompt_tokens.get_or_insert(u.prompt_tokens);
                                total_completion_tokens = total_completion_tokens
                                    .saturating_add(u.completion_tokens);
                                round_completion_tokens = Some(u.completion_tokens);
                                collected_usage = Some(u);
                            }
                            // 【彻底重构】token_count 由本轮 finalize_assistant_message 即时写入
                            // （每条 assistant 独立持有本轮 completion_tokens）。不再走
                            // batch_writer.set_tokens：避免与 finalize 的 spawn 写竞态、
                            // 也避免跨轮累加值（total_completion_tokens）脏写到新消息。
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
                                return fail_round_and_cancel(
                                    &ctx.app,
                                    &ctx.pool,
                                    &ctx.conv_id,
                                    &current_asst_msg_id,
                                    &error_kind(&e),
                                    &err_msg,
                                )
                                .await;
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
                        return fail_round_and_cancel(
                            &ctx.app,
                            &ctx.pool,
                            &ctx.conv_id,
                            &current_asst_msg_id,
                            &error_kind(&e),
                            &err_msg,
                        )
                        .await;
                    }
                }
            }
        }

        if !round_success {
            // round_success=false 意味着 consume_stream 始终失败，round_text 仍是初始空串
            // （round_text 仅在 Ok 分支赋值，那里会置 round_success=true），故无部分内容可回写。
            let err_msg = format!("连接重试已耗尽（共 {} 次）", ctx.budget.max_attempts);
            return fail_round_and_cancel(
                &ctx.app,
                &ctx.pool,
                &ctx.conv_id,
                &current_asst_msg_id,
                "stream",
                &err_msg,
            )
            .await;
        }

        observable.elapsed_ms = round_timer.elapsed_ms();
        emit_intermediate_round_state(&ctx.app, &ctx.conv_id, observable);
        // 【改】progress_text 跨轮累积，仅供停滞检测（不持久化）
        progress_text.push_str(&round_text);

        // 【改】推「本轮」文本到 BatchWriter（原为跨轮 all_text）
        batch_writer.push_text(round_text.clone()).await;

        // W4.2: Token 预算累计。注意：每轮 prompt_tokens 已含全部历史，跨轮累加会重复
        // 计入早期轮次 → 预算检查偏保守（可能略早触发 budget_exceeded）。这是有意的安全
        // 倾向；若需精确，可改为只累加 completion + 首轮 prompt。
        if let Some(ref usage) = collected_usage {
            cumulative_tokens += usage.prompt_tokens as usize + usage.completion_tokens as usize;
        }

        // 提取本轮已完成的工具调用（id, name, arguments）
        let completed_calls: Vec<(String, String, String)> = tool_calls_map
            .into_values()
            .filter(|tc| tc.ended)
            .map(|tc| (tc.id, tc.name, tc.arguments))
            .collect();

        // 【阶段 B】组装本轮 assistant 的权威 blocks：[thinking?, text?, tool_use*]
        // 多轮工具下每条 assistant 独立持有本轮 thinking + text + tool_use（不含 tool_result）。
        let mut round_blocks: Vec<ContentBlock> = Vec::new();
        if !round_think.is_empty() {
            round_blocks.push(ContentBlock::Thinking {
                thinking: round_think.clone(),
                signature: None,
            });
        }
        if !round_text.is_empty() {
            round_blocks.push(ContentBlock::Text { text: round_text.clone() });
        }
        for (id, name, args) in &completed_calls {
            round_blocks.push(ContentBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: args.clone(),
            });
        }

        // cancel 检查（落盘前）：consume_stream 可能因 cancel 返回部分内容。
        //  - 剔除 tool_use 后仍有内容（thinking/text）→ 只落盘这些，避免孤儿 tool_use（C1：
        //    tool_use 已输出但 cancel 不补 tool_result，留下会让下轮历史触发 400）
        //  - 剔除后无内容（空占位 / 仅未执行的 tool_use）→ 删除占位行（M3）
        if ctx.cancel.is_cancelled() {
            let cancel_blocks: Vec<ContentBlock> = round_blocks
                .iter()
                .filter(|b| !matches!(b, ContentBlock::ToolUse { .. }))
                .cloned()
                .collect();
            if cancel_blocks.is_empty() && round_text.is_empty() {
                if let Err(e) = repo::message::delete(&ctx.pool, &current_asst_msg_id).await {
                    tracing::warn!(
                        target: "ice_paw.chat",
                        "删除 cancel 时的空占位失败: msg_id={}, err={}",
                        current_asst_msg_id,
                        e
                    );
                }
            } else {
                batch_writer.flush_now().await;
                finalize_assistant_message(
                    &ctx.pool,
                    &current_asst_msg_id,
                    &round_text,
                    &cancel_blocks,
                    round_completion_tokens,
                )
                .await;
            }
            return finalize_cancel(&ctx.app, &ctx.pool, &ctx.conv_id, &current_asst_msg_id);
        }

        // 【阶段 C】即时持久化当前 assistant（权威快照：content + blocks + 本轮 token）。
        // 先 flush_now 落盘 BatchWriter 的 streaming 文本，再同步写权威 blocks；本轮结束后
        // set_msg_id 会切到新消息，避免后到的 flush 覆盖本轮 blocks。
        batch_writer.flush_now().await;
        finalize_assistant_message(
            &ctx.pool,
            &current_asst_msg_id,
            &round_text,
            &round_blocks,
            round_completion_tokens,
        )
        .await;

        // W4.2: Token 预算终止检查（当前 assistant 已 finalize，只需收尾）
        if ctx.budget.max_total_tokens != usize::MAX
            && cumulative_tokens > ctx.budget.max_total_tokens
        {
            tracing::warn!(
                target: "ice_paw.chat",
                "Token 预算已超限: cumulative={} > budget={}",
                cumulative_tokens,
                ctx.budget.max_total_tokens,
            );
            return finalize_success(
                &ctx.app,
                &ctx.pool,
                &ctx.conv_id,
                &current_asst_msg_id,
                "budget_exceeded",
                synthesize_usage(first_prompt_tokens, total_completion_tokens, collected_usage),
                &ctx.user_msg_id,
                first_prompt_tokens,
            );
        }

        // === M2.1: 停滞检测（dev1 三级级联 L1 简化方案） ===
        // 计算本轮进度指纹（progress_text + 已完成工具调用 name:arguments），
        // 与上一轮对比，更新连续未进展计数器。
        // 触发条件：连续 `stuck_threshold` 轮 hash 完全相同
        //   （文本未增长 & 工具调用签名未变化）。
        //
        // P0-1 fix: 使用 name+arguments 作为工具调用语义标识，而非实例 ID
        //   （实例 ID 如 toolu_xxx 每轮 LLM 重新生成 → hash 永远变 → 检测永不触发）。
        // P1-1 fix: sort 消除 `tool_calls_map` HashMap 迭代顺序不确定性，
        //   保证相同调用集合在不同 round 顺序下产出相同 hash。
        let mut completed_call_keys: Vec<String> = completed_calls
            .iter()
            .map(|c| format!("{}:{}", c.1, c.2))
            .collect();
        completed_call_keys.sort_unstable();
        let round_key = compute_round_key(&progress_text, &completed_call_keys);
        let (new_stuck_counter, stuck_now) = should_terminate_stuck(
            round_key,
            last_round_hash,
            stuck_counter,
            ctx.budget.stuck_threshold,
        );
        stuck_counter = new_stuck_counter;
        last_round_hash = Some(round_key);

        if stuck_now {
            tracing::info!(
                target: "ice_paw.loop",
                "停滞检测触发：连续 {} 轮无进展（threshold={}），终止对话",
                stuck_counter,
                ctx.budget.stuck_threshold,
            );
            return finalize_success(
                &ctx.app,
                &ctx.pool,
                &ctx.conv_id,
                &current_asst_msg_id,
                "stuck",
                synthesize_usage(first_prompt_tokens, total_completion_tokens, collected_usage),
                &ctx.user_msg_id,
                first_prompt_tokens,
            );
        }

        // 最终轮（本轮无工具调用）→ 当前 assistant 已 finalize，直接收尾
        if completed_calls.is_empty() {
            return finalize_success(
                &ctx.app,
                &ctx.pool,
                &ctx.conv_id,
                &current_asst_msg_id,
                &round_finish_reason,
                synthesize_usage(first_prompt_tokens, total_completion_tokens, collected_usage),
                &ctx.user_msg_id,
                first_prompt_tokens,
            );
        }

        tracing::info!(
            target: "ice_paw.chat",
            "工具调用循环: round={} tool_count={}",
            tool_round,
            completed_calls.len(),
        );

        // 【阶段 E】执行工具，得到 tool_result blocks（execute_tool_round 已 emit chat:tool-result）
        // RAG: 构造工具执行上下文（conv_id/agent_id/project_id/pool）透传给
        // execute_tool_round → dispatch → execute_with_context（search_kb 据此查 KB）。
        // workspace 解析：project 绑定了 workspace_path → 用项目源码根（文件/代码类工具
        // read_file/write_file/run_command/git/search_files 据此切换 current_dir 与路径白名单）；
        // 否则回退 agent workspace。知识库工具（save/search_kb）+ read_agent_config
        // 仍走 agent_id/scope，不依赖此处 workspace，不受影响。
        // 解析逻辑已抽到 tool_executor::build_tool_ctx（与各钩子接入点复用，project 优先）。
        let tool_ctx = build_tool_ctx(
            &ctx.pool,
            ctx.conv_id.clone(),
            ctx.agent_id.clone(),
            ctx.project_id.clone(),
            Some(ctx.api_key.clone()),
        )
        .await;
        let tool_result_blocks: Vec<ContentBlock> = match execute_tool_round(
            &ctx.app,
            &ctx.tool_registry,
            &ctx.auth_registry,
            &ctx.auth_session,
            &ctx.whitelist,
            &completed_calls,
            &tool_ctx,
            &current_asst_msg_id,
            &ctx.cancel,
            &ctx.hooks,
        )
        .await
        {
            Ok(blocks) => blocks,
            Err(e) => {
                // 工具执行编排整体失败 → 无法构造有效的 tool_result，
                // 注入空 user 消息会破坏 Anthropic 协议（tool_use 无对应
                // tool_result），导致后续轮次 400。视为致命错误中断循环。
                let err_msg = format!("工具执行失败: {}", e);
                tracing::error!(target: "ice_paw.chat", "{}", err_msg);
                return fail_round_and_cancel(
                    &ctx.app,
                    &ctx.pool,
                    &ctx.conv_id,
                    &current_asst_msg_id,
                    "internal",
                    &err_msg,
                )
                .await;
            }
        };

        // 【阶段 F】持久化 tool_result 为独立 user 消息（role=user，符合 Anthropic 协议：
        // tool_result 必须在 user 消息里）。多个 tool_result 合并进同一条 user 消息。
        let user_tool_msg_id = Uuid::new_v4().to_string();
        if let Err(e) = repo::message::create(
            &ctx.pool,
            &user_tool_msg_id,
            &NewMessage {
                conversation_id: ctx.conv_id.clone(),
                role: "user".into(),
                content: String::new(),
                token_count: None,
                error: None,
                model: None,
            },
        )
        .await
        {
            let err_msg = format!("持久化工具结果消息失败: {}", e);
            tracing::warn!(
                target: "ice_paw.chat",
                "{}: conv_id={}",
                err_msg,
                ctx.conv_id
            );
            return fail_round_and_cancel(
                &ctx.app,
                &ctx.pool,
                &ctx.conv_id,
                &current_asst_msg_id,
                "internal",
                &err_msg,
            )
            .await;
        }
        let result_json =
            serde_json::to_string(&tool_result_blocks).unwrap_or_else(|_| "[]".to_string());
        if let Err(e) =
            repo::message::update_content_blocks(&ctx.pool, &user_tool_msg_id, &result_json).await
        {
            tracing::warn!(
                target: "ice_paw.chat",
                "回写 tool_result content_blocks 失败: msg_id={}, err={}",
                user_tool_msg_id,
                e
            );
        }

        // 【阶段 G】ctx.messages 追加本轮 assistant(tool_use) + user(tool_result)。
        // 统一 role=user：Anthropic 适配层直接支持 user 消息携带 tool_result；
        // OpenAI 适配层在 chat_message_to_openai 里把含 ToolResult 的消息展开为
        // 多条 role="tool"（每 tool_call_id 一条），满足其「tool_calls 后必须紧跟
        // tool 回执」的协议要求。
        let mut asst_blocks: Vec<ContentBlock> = Vec::new();
        if !round_text.is_empty() {
            asst_blocks.push(ContentBlock::Text { text: round_text.clone() });
        }
        for (id, name, args) in &completed_calls {
            asst_blocks.push(ContentBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: args.clone(),
            });
        }
        ctx.messages.push(ChatMessage {
            role: "assistant".into(),
            content: asst_blocks,
            source_rowid: None,
        });
        ctx.messages.push(ChatMessage {
            role: "user".into(),
            content: tool_result_blocks,
            source_rowid: None,
        });

        tracing::info!(
            target: "ice_paw.chat",
            "工具执行完成: round={}，准备下一轮 LLM 调用",
            tool_round,
        );

        // 【阶段 H】若是最后一轮，当前 assistant（已 finalize）作为最终消息收尾；
        // 否则创建下一轮 assistant 占位 + 切 BatchWriter + emit chat:assistant-start。
        if tool_round + 1 >= ctx.budget.max_tool_rounds {
            tracing::info!(
                target: "ice_paw.chat",
                "已达最大工具调用轮数（{}），终止对话",
                ctx.budget.max_tool_rounds,
            );
            return finalize_success(
                &ctx.app,
                &ctx.pool,
                &ctx.conv_id,
                &current_asst_msg_id,
                "tool_use",
                synthesize_usage(first_prompt_tokens, total_completion_tokens, collected_usage),
                &ctx.user_msg_id,
                first_prompt_tokens,
            );
        }

        let next_asst_id = Uuid::new_v4().to_string();
        if let Err(e) = repo::message::create(
            &ctx.pool,
            &next_asst_id,
            &NewMessage {
                conversation_id: ctx.conv_id.clone(),
                role: "assistant".into(),
                content: String::new(),
                token_count: None,
                error: None,
                model: ctx.asst_model.clone(),
            },
        )
        .await
        {
            let err_msg = format!("创建下一轮 assistant 占位失败: {}", e);
            tracing::warn!(target: "ice_paw.chat", "{}", err_msg);
            return fail_round_and_cancel(
                &ctx.app,
                &ctx.pool,
                &ctx.conv_id,
                &current_asst_msg_id,
                "internal",
                &err_msg,
            )
            .await;
        }
        // 切 BatchWriter 到新 assistant（内部先 flush 当前 pending 再切 id）
        batch_writer.flush_now().await;
        batch_writer.set_msg_id(next_asst_id.clone()).await;
        // 通知前端：冻结上一条 assistant（写入其 tool_use/text/thinking）+ 插入 user(tool_result)
        // + 重置 streaming 状态 + push 新 assistant 占位。
        if let Err(e) = ctx.app.emit(
            "chat:assistant-start",
            ChatAssistantStartPayload {
                conversation_id: ctx.conv_id.clone(),
                message_id: next_asst_id.clone(),
            },
        ) {
            tracing::warn!(
                target: "ice_paw.chat",
                "emit chat:assistant-start 失败: conv_id={}, err={}",
                ctx.conv_id,
                e
            );
        }
        current_asst_msg_id = next_asst_id;
    }

    // 兜底：逻辑上不可达（最后一轮必在阶段 H 的 tool_use 分支 return），
    // 保留以满足函数返回类型。current_asst_msg_id 此时为最后一条有内容的 assistant。
    finalize_success(
        &ctx.app,
        &ctx.pool,
        &ctx.conv_id,
        &current_asst_msg_id,
        "tool_use",
        synthesize_usage(first_prompt_tokens, total_completion_tokens, collected_usage),
        &ctx.user_msg_id,
        first_prompt_tokens,
    );
}

