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

use crate::harness::cleanup::{finalize_assistant_message, finalize_cancel, finalize_success};
use crate::harness::error_mapping::{error_kind, friendly_error};
use crate::db::models::NewMessage;
use crate::db::repo;
use crate::error::AppError;
use crate::infra::protocol::{
    ChatAssistantStartPayload, ChatErrorPayload, ChatMessage, ChatRetryingPayload, ContentBlock,
    LlmProvider, TokenUsage,
};
use crate::harness::budget::LoopBudget;
use crate::harness::chat_state::CancellationToken;
use crate::harness::observable::{RoundState, RoundTimer};
use crate::harness::retry::{RetryContext, RetryState};
use crate::harness::mcp::McpRegistry;
use crate::harness::authority::{PathAuthSession, PathWhitelistConfig};

use super::batch_writer;
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

/// 中间 round-state 事件发射 — 供前端 ChatStatusBar 实时显示进度。
/// 失败仅记录 warn，不影响主流程。
fn emit_intermediate_round_state(
    app: &AppHandle,
    conv_id: &str,
    observable: &RoundState,
) {
    use crate::infra::protocol::ChatRoundStatePayload;
    let payload = ChatRoundStatePayload {
        conversation_id: conv_id.to_string(),
        round: observable.round,
        elapsed_ms: observable.elapsed_ms,
        tokens_prompt: observable.tokens_prompt,
        tokens_completion: observable.tokens_completion,
        cached_tokens: observable.cached_tokens,
        retry_count: observable.retry_count,
    };
    if let Err(e) = app.emit("chat:round-state", payload) {
        tracing::warn!(
            target: "ice_paw.chat",
            "emit intermediate chat:round-state 失败: conv_id={}, err={}",
            conv_id,
            e
        );
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
    /// M1.3: 用户消息 ID（用于清理阶段回写 token_count）
    pub user_msg_id: String,

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
    pub tool_registry: McpRegistry,
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

    // ---- M1.2: 工具裁剪所需上下文 ----
    /// M1.2: 当前用户消息纯文本（用于工具相关性打分）
    pub query: Option<String>,
    /// M1.2: 最近调用过的工具名列表（顺序不限；用于打分历史权重）
    pub call_history: Vec<String>,

    // ---- P0-3: 会话级 model override ----
    /// P0-3: 覆盖 Agent 默认 model（None = 使用 Agent 默认）。
    /// 透传给 `provider.stream_chat(model: ...)`，
    /// 仅影响本次请求，不改写 Agent 配置。
    pub model: Option<String>,

    /// 助手消息持久化时写入 `messages.model` 的值（effective model =
    /// override 或 Agent 默认）。loop_engine 创建后续轮次的 assistant 占位
    /// 消息时复用此值，保证一次发送产生的所有 assistant 消息 model 一致。
    pub asst_model: Option<String>,
}

impl LoopContext {
    /// 构造 `LoopContext`。这是 W6.2 引入的唯一构造入口。
    ///
    /// 参数数量看似很多，但这就是该结构体的全部职责 —— 把原本散落在
    /// `stream_loop` 形参列表里的 13 个字段集中起来。允许
    /// `clippy::too_many_arguments` 因为这就是本结构体的存在意义。
    ///
    /// M1.2: 新增 `user_msg_id` / `query` / `call_history` 三个字段。
    /// `user_msg_id` 是 M1.3 清理阶段回写 token_count 的关键。
    /// `query` + `call_history` 用于 `list_tool_defs_with_query` 打分。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        conv_id: String,
        asst_msg_id: String,
        user_msg_id: String,
        app: AppHandle,
        pool: SqlitePool,
        provider: Arc<dyn LlmProvider>,
        api_key: String,
        temperature: f64,
        max_tokens: i32,
        messages: Vec<ChatMessage>,
        tool_registry: McpRegistry,
        tools_enabled: bool,
        cancel: CancellationToken,
        budget: LoopBudget,
        auth_registry: ToolAuthRegistry,
        auth_session: PathAuthSession,
        whitelist: PathWhitelistConfig,
        query: Option<String>,
        call_history: Vec<String>,
        model: Option<String>,
        asst_model: Option<String>,
    ) -> Self {
        Self {
            conv_id,
            asst_msg_id,
            user_msg_id,
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
            query,
            call_history,
            model,
            asst_model,
        }
    }
}

/// M1.3: 合成最终 usage
///
/// `cleanup_after_success_with_blocks` 需要一个 `Option<TokenUsage>`。
/// 在多轮工具调用场景下，每轮都有自己的 prompt/completion token，
/// 我们用以下策略合成最终 usage：
///
/// - `first_prompt_tokens`：首次出现的 prompt_tokens（整个 prompt 包含所有历史）
/// - `total_completion_tokens`：所有轮的 completion_tokens 之和
///
/// 如果整个流期间 provider 未返回任何 usage，则保留 `None` 让
/// `cleanup` 函数走 estimate_tokens 兑底路径。
fn synthesize_usage(
    first_prompt_tokens: Option<u32>,
    total_completion_tokens: u32,
    last_collected: Option<TokenUsage>,
) -> Option<TokenUsage> {
    match (first_prompt_tokens, last_collected) {
        (Some(p), Some(last)) => Some(TokenUsage {
            prompt_tokens: p,
            completion_tokens: total_completion_tokens,
            cached_tokens: last.cached_tokens,
        }),
        (Some(p), None) => Some(TokenUsage {
            prompt_tokens: p,
            completion_tokens: total_completion_tokens,
            cached_tokens: 0,
        }),
        (None, _) => None,
    }
}

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
/// 工具签名（而非实例 ID）参与计算是为了让"相同文本但不同工具参数"不计入停滞
/// （dev1 L1 三级级联 + and-condition 要求）。
/// 注意：调用方必须在传入前对 Vec 排序，以消除上游 HashMap 迭代顺序不确定性。
fn compute_round_key(all_text: &str, completed_call_ids: &[String]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    all_text.hash(&mut h);
    for id in completed_call_ids {
        id.hash(&mut h);
    }
    h.finish()
}

/// M2.1: 纯函数形式的停滞判定（便于单元测试）
///
/// 输入：本轮进度指纹 + 上一轮指纹 + 当前连续未进展计数 + 阈值
/// 输出：`(new_counter, should_terminate)` —— 调用方负责把 new_counter 写回
///
/// 规则：
/// - 本轮 hash 与上一轮相同 → `new_counter = stuck_counter + 1`，否则归零
/// - 当 `new_counter >= threshold` → 触发终止
pub(crate) fn should_terminate_stuck(
    round_key: u64,
    last_round_hash: Option<u64>,
    stuck_counter: u32,
    threshold: u32,
) -> (u32, bool) {
    let no_progress = Some(round_key) == last_round_hash;
    let new_counter = if no_progress {
        stuck_counter.saturating_add(1)
    } else {
        0
    };
    let should_terminate = new_counter >= threshold;
    (new_counter, should_terminate)
}

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
            });
        }

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

            let stream_result = ctx
                .provider
                .stream_chat(
                    &ctx.api_key,
                    retry_messages,
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
                                if let Err(em) = ctx.app.emit(
                                    "chat:error",
                                    ChatErrorPayload {
                                        conversation_id: ctx.conv_id.clone(),
                                        message_id: current_asst_msg_id.clone(),
                                        kind: error_kind(&e),
                                        message: friendly_error(&err_msg),
                                    },
                                ) {
                                    tracing::warn!(
                                        target: "ice_paw.chat",
                                        "emit chat:error 失败: conv_id={}, err={}",
                                        ctx.conv_id,
                                        em
                                    );
                                }
                                if let Err(eu) = repo::message::update_error(
                                    &ctx.pool,
                                    &current_asst_msg_id,
                                    &err_msg,
                                )
                                .await
                                {
                                    tracing::warn!(
                                        target: "ice_paw.chat",
                                        "回写 asst 错误信息失败: msg_id={}, err={}",
                                        current_asst_msg_id,
                                        eu
                                    );
                                }
                                return finalize_cancel(&ctx.app, &ctx.pool, &ctx.conv_id, &current_asst_msg_id);
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
                        if let Err(em) = ctx.app.emit(
                            "chat:error",
                            ChatErrorPayload {
                                conversation_id: ctx.conv_id.clone(),
                                message_id: current_asst_msg_id.clone(),
                                kind: error_kind(&e),
                                message: friendly_error(&err_msg),
                            },
                        ) {
                            tracing::warn!(
                                target: "ice_paw.chat",
                                "emit chat:error 失败: conv_id={}, err={}",
                                ctx.conv_id,
                                em
                            );
                        }
                        if let Err(eu) =
                            repo::message::update_error(&ctx.pool, &current_asst_msg_id, &err_msg).await
                        {
                            tracing::warn!(
                                target: "ice_paw.chat",
                                "回写 asst 错误信息失败: msg_id={}, err={}",
                                current_asst_msg_id,
                                eu
                            );
                        }
                        return finalize_cancel(&ctx.app, &ctx.pool, &ctx.conv_id, &current_asst_msg_id);
                    }
                }
            }
        }

        if !round_success {
            // round_success=false 意味着 consume_stream 始终失败，round_text 仍是初始空串
            // （round_text 仅在 Ok 分支赋值，那里会置 round_success=true），故无部分内容可回写。
            let err_msg = format!("连接重试已耗尽（共 {} 次）", ctx.budget.max_attempts);
            if let Err(eu) =
                repo::message::update_error(&ctx.pool, &current_asst_msg_id, &err_msg).await
            {
                tracing::warn!(
                    target: "ice_paw.chat",
                    "回写 asst 错误信息失败: msg_id={}, err={}",
                    current_asst_msg_id,
                    eu
                );
            }
            if let Err(em) = ctx.app.emit(
                "chat:error",
                ChatErrorPayload {
                    conversation_id: ctx.conv_id.clone(),
                    message_id: current_asst_msg_id.clone(),
                    kind: "stream".into(),
                    message: friendly_error(&err_msg),
                },
            ) {
                tracing::warn!(
                    target: "ice_paw.chat",
                    "emit chat:error 失败: conv_id={}, err={}",
                    ctx.conv_id,
                    em
                );
            }
            return finalize_cancel(&ctx.app, &ctx.pool, &ctx.conv_id, &current_asst_msg_id);
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
        let tool_result_blocks: Vec<ContentBlock> = execute_tool_round(
            &ctx.app,
            &ctx.tool_registry,
            &ctx.auth_registry,
            &ctx.auth_session,
            &ctx.whitelist,
            &completed_calls,
            &ctx.conv_id,
            &current_asst_msg_id,
            &ctx.cancel,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(target: "ice_paw.chat", "工具执行失败: {}", e);
            Vec::new()
        });

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
            let _ = repo::message::update_error(&ctx.pool, &current_asst_msg_id, &err_msg).await;
            let _ = ctx.app.emit(
                "chat:error",
                ChatErrorPayload {
                    conversation_id: ctx.conv_id.clone(),
                    message_id: current_asst_msg_id.clone(),
                    kind: "internal".into(),
                    message: friendly_error(&err_msg),
                },
            );
            return finalize_cancel(&ctx.app, &ctx.pool, &ctx.conv_id, &current_asst_msg_id);
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
        // 统一 role=user（两 provider 适配层均已支持 user 消息携带 tool_result）。
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
        });
        ctx.messages.push(ChatMessage {
            role: "user".into(),
            content: tool_result_blocks,
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
            let _ = repo::message::update_error(&ctx.pool, &current_asst_msg_id, &err_msg).await;
            let _ = ctx.app.emit(
                "chat:error",
                ChatErrorPayload {
                    conversation_id: ctx.conv_id.clone(),
                    message_id: current_asst_msg_id.clone(),
                    kind: "internal".into(),
                    message: friendly_error(&err_msg),
                },
            );
            return finalize_cancel(&ctx.app, &ctx.pool, &ctx.conv_id, &current_asst_msg_id);
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

    // ========================================================================
    // M2.1: 停滞检测单元测试（B1-4）
    //
    // 全部使用已提取出的纯函数 `should_terminate_stuck` / `compute_round_key`
    // 测试，不依赖任何 IO / Tauri / DB，CI 友好且可在毫秒内跑完。
    // ========================================================================

    /// T1: stuck_threshold 默认值应为 5（dev1 评审：默认 3 误判率过高）
    #[test]
    fn stuck_detection_threshold_defaults_to_five() {
        let budget = LoopBudget::default();
        assert_eq!(
            budget.stuck_threshold, 5,
            "stuck_threshold 默认值应为 5（M2.1 修改）"
        );
    }

    /// T2: stuck_threshold 字段可被自定义覆盖
    #[test]
    fn stuck_detection_custom_threshold_accepted() {
        let budget = LoopBudget {
            max_tool_rounds: 5,
            max_attempts: 4,
            stuck_threshold: 7,
            max_total_tokens: 128_000,
        };
        assert_eq!(budget.stuck_threshold, 7, "自定义 stuck_threshold 应被接受");

        // 验证自定义阈值在判定函数中能正确生效
        // 首轮 last_hash=None → counter=0；后续轮同 hash 累加
        // 7 轮连续相同 hash（threshold=7 时 counter 最多到 6）不应触发
        let mut counter: u32 = 0;
        let mut last_hash: Option<u64> = None;
        let key = compute_round_key("hello", &[]);
        for round in 1..=6 {
            let (c, terminate) = should_terminate_stuck(key, last_hash, counter, 7);
            counter = c;
            last_hash = Some(key);
            // 第 1 轮 counter=0，第 2~6 轮 counter=1..5
            let expected = if round == 1 { 0 } else { (round - 1) as u32 };
            assert_eq!(counter, expected, "第 {} 轮 counter={}", round, expected);
            assert!(!terminate, "前 6 轮在 threshold=7 时不应触发");
        }
        // 第 7 轮 counter=6 < 7，仍不触发
        let (c, terminate) = should_terminate_stuck(key, last_hash, counter, 7);
        assert_eq!(c, 6);
        assert!(!terminate, "第 7 轮 counter=6 仍 < threshold=7");
        // 第 8 轮 counter=7 达到阈值，应触发
        let (final_counter, terminate) = should_terminate_stuck(key, last_hash, c, 7);
        assert_eq!(final_counter, 7);
        assert!(terminate, "第 8 轮无进展在 threshold=7 时应触发");
    }

    /// T3: 连续 N 轮无进展（hash 完全相同）触发 stuck
    ///
    /// 验证默认 threshold=5 场景：
    /// - 首轮 last_hash=None → counter 归零（无法比较）
    /// - 第 2~5 轮 hash 相同 → counter 累加到 1,2,3,4
    /// - 第 6 轮 counter=5 ≥ threshold → 触发停滞
    #[test]
    fn stuck_detection_triggers_after_n_rounds_with_no_progress() {
        let threshold: u32 = 5;
        let mut counter: u32 = 0;
        let mut last_hash: Option<u64> = None;
        // 同一文本 + 同一组工具调用 → hash 永远相同
        let key = compute_round_key("stuck text", &["tool-1".to_string()]);

        // 前 5 轮：不触发
        for round in 1..=5 {
            let (new_counter, terminate) = should_terminate_stuck(key, last_hash, counter, threshold);
            counter = new_counter;
            last_hash = Some(key);
            // 第 1 轮 counter=0，后续轮累加
            assert_eq!(
                counter,
                if round == 1 { 0 } else { (round - 1) as u32 },
                "第 {} 轮 counter={}",
                round,
                counter
            );
            assert!(!terminate, "前 5 轮不应触发（threshold=5）");
        }
        // 第 6 轮：counter=5 ≥ threshold → 触发
        let (final_counter, terminate) = should_terminate_stuck(key, last_hash, counter, threshold);
        assert_eq!(final_counter, 5);
        assert!(terminate, "第 6 轮（threshold=5）应触发停滞");
    }

    /// T4: 工具调用签名变化 → 计数器归零
    ///
    /// 验证 dev1 设计的 and-condition：仅文本相同但工具签名变了不算停滞。
    /// 用 hash 直接验证：相同文本 + 不同 `name:arguments` → hash 不同 → counter 归零。
    #[test]
    fn stuck_detection_resets_on_tool_call_change() {
        let threshold: u32 = 3;

        // 1) 前 3 轮：相同文本 + 相同工具签名 → counter 累加
        // 第 1 轮 None → 0；第 2,3 轮相同 → counter 累加到 2
        // P0-1 fix: 使用 name:arguments 格式（与生产代码一致）
        let key_a =
            compute_round_key("hello", &["read_file:{\"path\":\"/a\"}".to_string()]);
        let mut counter: u32 = 0;
        let mut last_hash: Option<u64> = None;
        for _ in 0..3 {
            let (c, _) = should_terminate_stuck(key_a, last_hash, counter, threshold);
            counter = c;
            last_hash = Some(key_a);
        }
        assert_eq!(counter, 2, "3 轮相同 hash 后 counter 应为 2（首轮 0 + 累加 2）");

        // 2) 第 4 轮：相同文本但换工具参数 → counter 应归零
        let key_b =
            compute_round_key("hello", &["read_file:{\"path\":\"/b\"}".to_string()]);
        assert_ne!(key_a, key_b, "不同工具签名应产出不同 hash");
        let (new_counter, terminate) = should_terminate_stuck(key_b, last_hash, counter, threshold);
        assert_eq!(new_counter, 0, "工具变化时 counter 应归零");
        assert!(!terminate, "工具变化时不应触发停滞");
    }

    /// T5: 文本变化 → 计数器归零
    ///
    /// 验证 hash 包含 all_text：哪怕工具签名也相同，只要文本增长就不算停滞。
    /// 同时验证首轮（last_hash=None）counter 归零。
    #[test]
    fn stuck_detection_resets_on_text_change() {
        let threshold: u32 = 3;

        // 1) 前 3 轮：相同文本 → counter 累加到 2
        // P0-1 fix: 使用 name:arguments 格式（与生产代码一致）
        let key_a =
            compute_round_key("part1", &["read_file:{\"path\":\"/a\"}".to_string()]);
        let mut counter: u32 = 0;
        let mut last_hash: Option<u64> = None;
        for _ in 0..3 {
            let (c, _) = should_terminate_stuck(key_a, last_hash, counter, threshold);
            counter = c;
            last_hash = Some(key_a);
        }
        assert_eq!(counter, 2);

        // 2) 第 4 轮：文本增长 → counter 应归零
        let key_b =
            compute_round_key("part1 part2", &["read_file:{\"path\":\"/a\"}".to_string()]);
        assert_ne!(key_a, key_b, "文本变化应产出不同 hash");
        let (new_counter, terminate) = should_terminate_stuck(key_b, last_hash, counter, threshold);
        assert_eq!(new_counter, 0, "文本变化时 counter 应归零");
        assert!(!terminate, "文本增长时不应触发停滞");

        // 3) 首轮（last_hash = None）counter 归零（无法比较）
        let (first_counter, terminate) = should_terminate_stuck(key_a, None, 0, 1);
        assert_eq!(first_counter, 0, "首轮 last_hash=None → counter 归零");
        assert!(!terminate, "首轮不应触发");

        // 4) 不同文本 + 相同工具签名：hash 必然不同
        let key_c =
            compute_round_key("different", &["read_file:{\"path\":\"/a\"}".to_string()]);
        assert_ne!(key_a, key_c);
        let (reset_counter, terminate) = should_terminate_stuck(key_c, Some(key_a), 5, 3);
        assert_eq!(reset_counter, 0, "文本变化重置 counter");
        assert!(!terminate);
    }

    /// T6: 相同工具调用集合在不同顺序下产出相同 hash（P1-1 fix 验证）
    ///
    /// 模拟生产代码：先把 `name:arguments` 字符串收集进 Vec，再 `sort_unstable()`。
    /// 验证排序后的 Vec 与原始顺序不同的 Vec 在 sort 后产出相同 hash。
    #[test]
    fn stuck_detection_hash_independent_of_iteration_order() {
        let call_keys = vec![
            "read_file:{\"path\":\"/a\"}".to_string(),
            "write_file:{\"path\":\"/b\"}".to_string(),
            "bash:{}".to_string(),
        ];
        // 逆序版（模拟 HashMap::into_values() 在不同 run 下的不同迭代顺序）
        let reversed: Vec<String> = {
            let mut v = call_keys.clone();
            v.reverse();
            v
        };
        // shuffle 版（更激进的顺序扰动）
        let mut shuffled = call_keys.clone();
        shuffled.swap(0, 2);

        let mut sorted_original = call_keys.clone();
        sorted_original.sort_unstable();
        let mut sorted_reversed = reversed.clone();
        sorted_reversed.sort_unstable();
        let mut sorted_shuffled = shuffled.clone();
        sorted_shuffled.sort_unstable();

        let key_original = compute_round_key("same text", &sorted_original);
        let key_reversed = compute_round_key("same text", &sorted_reversed);
        let key_shuffled = compute_round_key("same text", &sorted_shuffled);

        assert_eq!(
            key_original, key_reversed,
            "sort 后逆序集合与正序集合应产出相同 hash"
        );
        assert_eq!(
            key_original, key_shuffled,
            "sort 后乱序集合与正序集合应产出相同 hash"
        );
        // 反向断言：未排序的原始 Vec 应该产生不同 hash（验证 sort 是必要的）
        assert_ne!(
            compute_round_key("same text", &call_keys),
            compute_round_key("same text", &reversed),
            "未 sort 的不同顺序应产出不同 hash（sort 是必需的）"
        );
    }

    /// T7: 工具调用实例 ID 变化不影响进度指纹（P0-1 fix 核心意图验证）
    ///
    /// 模拟两轮：实例 ID 不同（toolu_aaa vs toolu_bbb），
    /// 但 name+args 相同。验证 hash 相等——证明实例 ID 不参与计算。
    #[test]
    fn stuck_detection_hash_ignores_instance_id() {
        // 生产代码现在使用 name:args（不含实例 ID），两轮输入完全相同
        let round1_keys = vec!["read_file:{\"path\":\"/etc/hosts\"}".to_string()];
        let round2_keys = vec!["read_file:{\"path\":\"/etc/hosts\"}".to_string()];

        let key1 = compute_round_key("result", &round1_keys);
        let key2 = compute_round_key("result", &round2_keys);
        assert_eq!(
            key1, key2,
            "name+args 相同时 hash 必须相等（实例 ID 不参与计算）"
        );

        // 反向断言：如果错误地混入实例 ID，hash 会不同（这里手工拼接错误格式演示）
        // 使用 P0-1 改前的错误格式（带 toolu_ 实例 ID），验证它确实导致 hash 不稳定
        let buggy_round1 = vec!["toolu_aaa:read_file:{\"path\":\"/etc/hosts\"}".to_string()];
        let buggy_round2 = vec!["toolu_bbb:read_file:{\"path\":\"/etc/hosts\"}".to_string()];
        let buggy_key1 = compute_round_key("result", &buggy_round1);
        let buggy_key2 = compute_round_key("result", &buggy_round2);
        assert_ne!(
            buggy_key1, buggy_key2,
            "错误格式（含实例 ID）应产出不同 hash——证明实例 ID 必须从 hash 输入中排除"
        );
    }
}
