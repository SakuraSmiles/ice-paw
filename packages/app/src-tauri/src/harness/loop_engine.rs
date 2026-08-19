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
//!   │   │   ├─ stream_with_retry()   // 单轮流式+退避重试（抽到 r#loop::retry_round）
//!   │   │   │   ├─ provider.stream_chat()
//!   │   │   │   ├─ stream_consumer::consume_stream()
//!   │   │   │   ├─ 重试分类: classify_retry_reason()
//!   │   │   │   └─ RoundStreamResult::{Ok, RetryExhausted, Aborted}
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
//! ## 子模块（`harness::r#loop`）
//!
//! - `context` — 输入封装 `LoopConfig` + `LoopContext`
//! - `retry_round` — 单轮流式+退避重试 `stream_with_retry`（`RoundStreamResult`）
//! - `stuck_detect` — 停滞检测（compute_round_key + should_terminate_stuck）
//! - `token_usage` — 多轮 usage 合成（synthesize_usage）
//! - `reason` — retry reason 分类（classify_retry_reason）
//! - `events` — 中间 round-state 事件（emit_intermediate_round_state）
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
//!   检测点在外层 `for tool_round` 循环底部，不在 `stream_with_retry` 内
//!   （重试是网络层行为，不构成"停滞"语义）。
//!
//! B1 自动续期：预算（max_total_tokens）与轮数（max_tool_rounds）触顶不再立即
//!   终止——续期额度未尽时 +初始上限继续跑（合法长任务不误杀），额度用尽才真停。
//!   失控循环由 stuck_detect 独立熔断；额度有界保证总开销封顶 = 初始 × (1+额度)。

use std::collections::HashMap;

use uuid::Uuid;

use crate::db::models::{HookPoint, NewMessage};
use crate::db::repo;
use crate::harness::cleanup::{
    fail_round_and_cancel, finalize_assistant_message, finalize_assistant_without_tool_use,
    finalize_cancel, finalize_success, PersistOutcome,
};
use crate::harness::event_log::{self, EventCtx};
use crate::harness::hooks::{has_actions, run_hooks};
use crate::harness::observable::{RoundState, RoundTimer};
use crate::infra::protocol::{ChatAssistantStartPayload, ChatMessage, ContentBlock, TokenUsage};

use super::batch_writer;
use super::stream_consumer::CollectedToolCall;
use super::tool_executor::{build_tool_ctx, execute_tool_round};

// stream_with_retry（含 classify_retry_reason）已迁移到 crate::harness::r#loop::retry_round
use crate::harness::r#loop::retry_round::{stream_with_retry, RoundStreamResult};

// emit_intermediate_round_state 已迁移到 crate::harness::r#loop::events
use crate::harness::r#loop::events::{emit_budget_state, emit_intermediate_round_state};

// ==========================================================================
// W6.2: LoopConfig / LoopContext 已迁移到 crate::harness::r#loop::context
// 此处 re-export 保持调用方（chat_cmd.rs）的 import 路径不变。
// ==========================================================================
pub(crate) use crate::harness::r#loop::context::{LoopConfig, LoopContext};

// synthesize_usage 已迁移到 crate::harness::r#loop::token_usage
use crate::harness::r#loop::token_usage::synthesize_usage;

/// budget_exceeded 终止时的 fallback 提示文案（纯函数便于单测）。
///
/// 两分支：显式硬上限（续期额度 0）给出「注释掉该行恢复自适应+续期」的
/// 自助指引；默认额度用尽则说明续期次数已耗完。数字与 chat:budget 终态
/// 事件一致（计费口径：缓存命中按 1/10 折扣），用户在提示行即可看到
/// 「已用多少 / 上限多少」。
fn budget_exceeded_fallback(cumulative: usize, cap: usize, max_renewals: u32) -> String {
    if max_renewals == 0 {
        format!(
            "（本次累计已消耗 {cumulative} tokens，达到显式预算上限 {cap}，已停止。\
             发送新消息即可继续。注：agent.yaml 显式设置的 max_total_tokens 为\
             硬上限、不自动续期，长对话会频繁触顶；注释掉该行可恢复按上下文\
             窗口 3× 自适应并自动续期。）"
        )
    } else {
        format!(
            "（本次累计已消耗 {cumulative} tokens，已达预算上限 {cap} 且自动续期\
             额度（{max_renewals} 次）用尽，已停止。发送新消息即可继续。）"
        )
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
        if let Err(e) = run_hooks(
            HookPoint::ConversationEnd,
            &ctx.hooks,
            &hook_ctx,
            &ctx.tool_registry,
        )
        .await
        {
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

/// 终止守卫 + 事件镜像：[`finalize_assistant_without_tool_use`] 落盘成功 → 发
/// `assistant_message`（镜像 PersistOutcome 里的实际写入值，含 round/continuation
/// 等 loop 语境——这就是事件发在 loop_engine 侧而非 cleanup 侧的原因）；删占位 →
/// 发 `message_discarded`。
///
/// 返回值透传 PersistOutcome（调用方若需区分处理仍可用）。
// 11 个参数均为该守卫点独立输入且仅 5 个调用点（全在本文件）；事件化收纳进
// EventCtx 后已是最小参数面，进一步收敛需把 round 语境打包成 struct，收益不足。
#[allow(clippy::too_many_arguments)]
async fn finalize_guard_logged(
    pool: &sqlx::SqlitePool,
    batch_writer: &batch_writer::BatchWriter,
    ev: &EventCtx,
    asst_msg_id: &str,
    msg_text: &str,
    round_blocks: &[ContentBlock],
    completion_tokens: Option<u32>,
    fallback_text: Option<&str>,
    round: u32,
    model: Option<&str>,
    continuation: bool,
    duration_ms: u64,
) -> PersistOutcome {
    let outcome = finalize_assistant_without_tool_use(
        pool,
        batch_writer,
        asst_msg_id,
        msg_text,
        round_blocks,
        completion_tokens,
        fallback_text,
    )
    .await;
    match &outcome {
        PersistOutcome::Persisted { content, blocks } => {
            event_log::log_assistant_message(
                pool,
                ev,
                asst_msg_id,
                model,
                content,
                blocks,
                completion_tokens.map(|t| t.max(1) as i64),
                Some(duration_ms),
                round,
                continuation,
            )
            .await;
        }
        PersistOutcome::Deleted => {
            event_log::log_message_discarded(pool, ev, asst_msg_id, "termination_guard_no_text")
                .await;
        }
    }
    outcome
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
    // session-events（Phase 0）：本 turn 的事件上下文（conv/turn/agent 三元组），
    // 全函数复用。事件全部 inline await（保序硬规则，见 event_log 模块注释）。
    let ev = EventCtx::new(&ctx.conv_id, &ctx.user_msg_id, &ctx.agent_id);

    // 【彻底重构】每轮独立持久化，删除跨轮累积器（原 all_text / all_content_blocks）。
    //
    // `current_asst_msg_id`：循环内所有 emit / DB 写入的「唯一 id 源」。
    // 初始 = 首条 assistant（ctx.asst_msg_id，由 chat_cmd 创建），每轮工具结束后
    // 更新为下一轮的 assistant 占位 id。所有错误 / cancel / 成功路径都必须用它，
    // 绝不能用 ctx.asst_msg_id（那是首条，多轮工具下会标到错误的轮）。
    let mut current_asst_msg_id = ctx.asst_msg_id.clone();
    // `current_asst_finalized`：当前 assistant 占位是否已在本轮内 finalize（落库+事件）。
    // loop 顶 cancel 检测到未 finalize 的占位时补发 message_discarded——否则该行
    // 「存在但零事件」，对账会记为差异（session-event-log Phase 1 步骤 0 接线补漏）。
    let mut current_asst_finalized = false;
    // `progress_text`：跨轮累积文本，仅供停滞检测使用，**不持久化**。
    // （每轮真实文本由 finalize_assistant_message 即时落盘到对应 assistant 消息。）
    let mut progress_text = String::new();
    let mut collected_usage: Option<TokenUsage> = None;

    // W4.2: Token 预算累计追踪（cumulative_tokens = 计费口径，见 budget::billed_tokens；
    // cached/prompt 两路毛口径累计仅供 HUD「缓存命中 X%」展示，不参与熔断判断）
    let mut cumulative_tokens: usize = 0;
    let mut cumulative_cached_tokens: usize = 0;
    let mut cumulative_prompt_tokens: usize = 0;

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

    // === 自动续写状态（模式 C 治本：finish_reason=length/max_tokens 不再当终态）===
    // `continue_full_text`：续写激活后持有「本消息累积全文」，供下轮拼接 + 全文覆写落盘
    //（push_text 是覆写语义，后端必须自行累积，否则第 N+1 轮覆盖第 N 轮）。空串=未激活。
    let mut continue_full_text = String::new();
    let mut continue_rounds: u32 = 0;
    const MAX_CONTINUE_ROUNDS: u32 = 8;
    // `prev_round_had_tools`：守卫 line 231 的「工具调用完毕」注入——续写轮（前一轮纯文本）
    // 不该触发该提示。正常工具流下恒 true（与改造前逐字等价）。
    let mut prev_round_had_tools = false;

    // === B1 自动续期状态（预算/轮数触顶 + 仍有额度 → 抬升上限续跑）===
    // 语义：撞上限不再是立即终态——先看续期额度，有则 +初始上限继续（合法长任务不
    // 误杀），额度用尽才真停。失控保护不靠这里：stuck_detect（连续 stuck_threshold
    // 轮无进展）独立触发终止；额度有界保证「看似有进展的失控循环」总开销仍封顶
    // = 初始上限 × (1+额度)。agent.yaml 显式 max_total_tokens / tool_max_rounds →
    // chat_cmd 置额度 0（显式硬上限不被静默突破）。
    let initial_max_tokens = ctx.budget.max_total_tokens;
    let mut effective_max_tokens = ctx.budget.max_total_tokens;
    let mut budget_renewals: u32 = 0;
    let initial_max_rounds = ctx.budget.max_tool_rounds;
    let mut effective_max_rounds = ctx.budget.max_tool_rounds;
    let mut round_renewals: u32 = 0;

    // === 工具执行循环 ===
    // 标签 'tool_round 供自动续写（finish_reason=length/max_tokens）跳回循环顶，
    // 复用同一 assistant 消息无缝拼接长输出（模式 C 治本）。
    // B1: 无限 range + 顶部硬闸——轮数上限由 effective_max_rounds 动态决定
    //（阶段 H 续期后抬升，max_tool_rounds=0 等边界由闸兜住），不再绑死在 range 上。
    'tool_round: for tool_round in 0u32.. {
        if ctx.cancel.is_cancelled() {
            // 残留占位补事件：此处占位行必然是「新鲜空行」（chat_cmd 首占位，或上一轮
            // 阶段 H 建的下一轮占位，本轮尚未流式输出）。续写轮复用同 id 且阶段 C 已发过
            // assistant_message，finalized 标记避免对同一行重复 discard。行本身不删
            //（与 guard 的 Delete 语义不同——这里不介入 legacy 持久化行为）。
            if !current_asst_finalized {
                event_log::log_message_discarded(
                    &ctx.pool,
                    &ev,
                    &current_asst_msg_id,
                    "cancel_top_placeholder",
                )
                .await;
            }
            return finalize_cancel(
                ctx.emitter.as_ref(),
                &ctx.pool,
                &ev,
                &current_asst_msg_id,
                tool_round,
            )
            .await;
        }

        // 【B1 轮数上限硬闸】无限 range 的等价界：正常流必在阶段 H 判定（终止或
        // 续期抬升 effective_max_rounds 后放行），任何路径跳过阶段 H 时由此兜底。
        // 此时上一轮 assistant 已 finalize + tool_result 已落盘，直接收尾安全。
        if tool_round >= effective_max_rounds {
            return finalize_success(
                ctx.emitter.as_ref(),
                &ctx.pool,
                &ev,
                &current_asst_msg_id,
                "tool_use",
                synthesize_usage(
                    first_prompt_tokens,
                    total_completion_tokens,
                    collected_usage,
                ),
                first_prompt_tokens,
                tool_round,
            )
            .await;
        }

        let round_timer = RoundTimer::new(tool_round);
        observable.round = tool_round + 1;

        let tools: Option<Vec<crate::infra::protocol::ToolDef>> = if ctx.tools_enabled {
            // 工具数超过阈值时按相关性排序（相关工具靠前），始终全量发送、不降级不裁剪。
            // turn 内 query/call_history 固定（session_runner 组装期加载一次），逐轮
            // 重取只是防御注册表快照中途变化；顺序确定性由 list_tool_defs 出口按名
            // 排序保证（provider 上下文缓存按请求前缀匹配，工具列表在最前缀）。
            // 阈值用 scoring 模块的专用常量，不再误用 ContextBudget 的 token 预算默认值
            // （后者属不同维度，误用导致 per-agent 配置成为 dead config）。
            Some(
                ctx.tool_registry
                    .list_tool_defs_with_query(
                        ctx.query.as_deref().unwrap_or(""),
                        Some(crate::harness::scoring::DEFAULT_TOOL_SORT_THRESHOLD),
                        &ctx.call_history,
                    )
                    .await,
            )
        } else {
            None
        };

        // round_* 在下方 stream_with_retry 的 Ok 分支赋值（其余分支均 return），
        // 故无需初始化——编译器可证明到达后续使用前必然已赋值。
        let round_text: String;
        let round_think: String;
        let round_finish_reason: String;
        let tool_calls_map: HashMap<String, CollectedToolCall>;
        // 本轮 provider 返回的 completion_tokens（用于即时落盘该 assistant 的 token_count）
        let mut round_completion_tokens: Option<u32> = None;

        // 第 2 轮起，在消息中注入剩余轮次信息（帮助 LLM 决定是否继续调工具）。
        // 仅当上一轮真的有工具调用时注入——续写链（前一轮纯文本截断）下不注入，
        // 避免给模型「工具调用完毕」的误导提示。分母用 effective_max_rounds
        //（B1 续期抬升后提示中的总轮数保持真实）。
        if tool_round > 0 && prev_round_had_tools {
            ctx.messages.push(ChatMessage {
                role: "user".into(),
                content: vec![ContentBlock::text(format!(
                    "（第 {}/{} 轮工具调用完毕。如果还有未完成的操作请继续，如果已经完成请直接输出最终回答。）",
                    tool_round, effective_max_rounds
                ))],
                source_rowid: None,
                source_seq: None,
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
            run_hooks(
                HookPoint::BeforeLlm,
                &ctx.hooks,
                &hook_ctx,
                &ctx.tool_registry,
            )
            .await
            .ok()
            .and_then(|o| o.injected_prompt)
        } else {
            None
        };
        // session-events：钩子注入是模型可见事实（「Model-visible means logged」），
        // 每轮一条（BeforeLlm 本就每轮注入一次，事件密度与之对齐）。
        if let Some(inj) = &round_injected {
            event_log::log_hook_injected(
                &ctx.pool,
                &ev,
                &event_log::HookInjectedPayload {
                    v: 1,
                    point: "before_llm".into(),
                    prompt: inj.clone(),
                },
            )
            .await;
        }

        // === RetryState 驱动的重试循环（已抽到 stream_with_retry）===
        match stream_with_retry(
            ctx,
            observable,
            tool_round,
            tools,
            round_injected,
            &current_asst_msg_id,
        )
        .await
        {
            RoundStreamResult::Ok(sr) => {
                round_text = sr.text;
                round_think = sr.think;
                round_finish_reason = sr.finish_reason;
                tool_calls_map = sr.tool_calls;
                if let Some(u) = sr.usage {
                    // M1.3: 累计 token —— 首次出现的 prompt_tokens 作为原始 user 消息 token_count
                    first_prompt_tokens.get_or_insert(u.prompt_tokens);
                    total_completion_tokens =
                        total_completion_tokens.saturating_add(u.completion_tokens);
                    round_completion_tokens = Some(u.completion_tokens);
                    // W4.2→预算诚实化: Token 预算累计 —— 按「计费口径」（缓存命中
                    // 1/10 折扣，见 budget::billed_tokens）：provider 对命中部分只收
                    // 1/10~1/50 费用，按毛成本 Σ(prompt+completion) 计量会把 96% 命中
                    // 的长任务当失控熔断（生产实证）。关键：只能用本轮 `u`，不可复用
                    // 跨轮 collected_usage——后者在 provider 间歇不回 usage 时保留
                    // 上一轮旧值，会被每轮重复累加导致虚高。
                    //（须在下方 `collected_usage = Some(u)` move 之前读取 u 的字段。）
                    cumulative_tokens = cumulative_tokens.saturating_add(
                        crate::harness::budget::billed_tokens(
                            u.prompt_tokens as u64,
                            u.cached_tokens as u64,
                            u.completion_tokens as u64,
                        ) as usize,
                    );
                    cumulative_cached_tokens =
                        cumulative_cached_tokens.saturating_add(u.cached_tokens as usize);
                    cumulative_prompt_tokens =
                        cumulative_prompt_tokens.saturating_add(u.prompt_tokens as usize);
                    collected_usage = Some(u);
                }
                // 【彻底重构】token_count 由本轮 finalize_assistant_message 即时写入
                //（每条 assistant 独立持有本轮 completion_tokens）。不再走
                // batch_writer.set_tokens：避免与 finalize 的 spawn 写竞态、
                // 也避免跨轮累加值（total_completion_tokens）脏写到新消息。
            }
            RoundStreamResult::RetryExhausted => {
                // consume_stream 始终失败，round_text 仍是初始空串，故无部分内容可回写。
                let err_msg = format!("连接重试已耗尽（共 {} 次）", ctx.budget.max_attempts);
                return fail_round_and_cancel(
                    ctx.emitter.as_ref(),
                    &ctx.pool,
                    &ev,
                    &current_asst_msg_id,
                    "stream",
                    &err_msg,
                    tool_round,
                )
                .await;
            }
            RoundStreamResult::Aborted => {
                // cancel 或不可重试错误（错误 emit 已由 stream_with_retry 内部完成）。
                return finalize_cancel(
                    ctx.emitter.as_ref(),
                    &ctx.pool,
                    &ev,
                    &current_asst_msg_id,
                    tool_round,
                )
                .await;
            }
        }

        // 本轮生成窗口（stream 开始 → 返回）：assistant_message 事件的 duration_ms 用它，
        // 不用轮末重取——751/825 守卫点在工具执行后，重取会把工具耗时算进「生成耗时」。
        let round_gen_ms = round_timer.elapsed_ms();
        observable.elapsed_ms = round_gen_ms;
        emit_intermediate_round_state(ctx.emitter.as_ref(), &ctx.conv_id, observable);
        // 预算 HUD 数据源：本轮 usage 累计后的会话级状态（renewed=false 常规更新）
        emit_budget_state(
            ctx.emitter.as_ref(),
            &ctx.conv_id,
            tool_round,
            cumulative_tokens,
            cumulative_cached_tokens,
            cumulative_prompt_tokens,
            effective_max_tokens,
            initial_max_tokens,
            budget_renewals,
            ctx.budget.max_budget_renewals,
            false,
        );
        // 【改】progress_text 跨轮累积，仅供停滞检测（不持久化）
        progress_text.push_str(&round_text);

        // 自动续写：续写激活（continue_full_text 非空）时，本消息应落盘的全文 =
        // 历史 continue_full_text + 本轮 round_text；否则即 round_text。
        // msg_text 用于所有持久化点（push_text / round_blocks / finalize / 终止守卫），
        // 保证续写中任意退出路径（cancel/budget/stuck/正常收尾）都不丢前缀。
        // continue_full_text 仅在下方「续写决策点」更新；progress_text / ctx.messages 仍用 round_text。
        let msg_text = if continue_full_text.is_empty() {
            round_text.clone()
        } else {
            format!("{continue_full_text}{round_text}")
        };

        // 【改】推「本消息全文」到 BatchWriter（续写时含前缀；push_text 是覆写语义）
        batch_writer.push_text(msg_text.clone()).await;

        // W4.2: Token 预算累计已在上方 Ok 分支内完成（基于本轮 usage），此处不再累加。

        // 提取本轮已完成的工具调用（id, name, arguments）
        let completed_calls: Vec<(String, String, String)> = tool_calls_map
            .into_values()
            .filter(|tc| tc.ended)
            .map(|tc| (tc.id, tc.name, tc.arguments))
            .collect();
        // 守卫下一轮 line-231 注入：本轮是否有工具调用（续写链恒空 → 不注入误导提示）
        prev_round_had_tools = !completed_calls.is_empty();

        // 【阶段 B】组装本轮 assistant 的权威 blocks：[thinking?, text?, tool_use*]
        // 多轮工具下每条 assistant 独立持有本轮 thinking + text + tool_use（不含 tool_result）。
        let mut round_blocks: Vec<ContentBlock> = Vec::new();
        if !round_think.is_empty() {
            round_blocks.push(ContentBlock::Thinking {
                thinking: round_think.clone(),
                signature: None,
            });
        }
        if !msg_text.is_empty() {
            round_blocks.push(ContentBlock::Text {
                text: msg_text.clone(),
            });
        }
        for (id, name, args) in &completed_calls {
            round_blocks.push(ContentBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: args.clone(),
            });
        }

        // 【阶段 C0】终止检查关口（落盘前）—— cancel / budget / stuck 三者都在 tool_use
        // 落盘（阶段 C 的 finalize_assistant_message）之前判定。命中终止时由守卫
        // finalize_assistant_without_tool_use 剔除 ToolUse 后落盘（或删占位），杜绝「有
        // tool_use 无 tool_result」孤儿（→ thinking-only → OpenAI 400 会话卡死）。
        // 守卫判定用 has_text（与 sanitize_history 对齐），比原 cancel 的 is_empty() 更严格。

        // cancel：fallback=None（无文本则删占位；有 text/thinking 则保留剔除 tool_use 后版本）
        if ctx.cancel.is_cancelled() {
            let _ = finalize_guard_logged(
                &ctx.pool,
                &batch_writer,
                &ev,
                &current_asst_msg_id,
                &msg_text,
                &round_blocks,
                round_completion_tokens,
                None,
                tool_round,
                ctx.asst_model.as_deref(),
                !continue_full_text.is_empty(),
                round_gen_ms,
            )
            .await;
            return finalize_cancel(
                ctx.emitter.as_ref(),
                &ctx.pool,
                &ev,
                &current_asst_msg_id,
                tool_round + 1,
            )
            .await;
        }

        // W4.2: Token 预算终止检查（落盘前）。B1：触顶先尝试自动续期（+初始上限，
        // 失控由 stuck_detect 独立兜底，额度有界保证总开销封顶），额度用尽才终止。
        // fallback=Some 保证纯 tool_use / thinking-only 轮也写入终止说明，
        // msg_id 恒有效（finalize_success 需 final_asst_msg_id 指向真实消息）。
        if effective_max_tokens != usize::MAX && cumulative_tokens > effective_max_tokens {
            if budget_renewals < ctx.budget.max_budget_renewals {
                budget_renewals += 1;
                let prev_cap = effective_max_tokens;
                effective_max_tokens = effective_max_tokens.saturating_add(initial_max_tokens);
                tracing::info!(
                    target: "ice_paw.chat",
                    "Token 预算自动续期: cumulative={} 触顶 {}，续期 {}/{} → 新上限 {}",
                    cumulative_tokens,
                    prev_cap,
                    budget_renewals,
                    ctx.budget.max_budget_renewals,
                    effective_max_tokens,
                );
                // 续期 toast 数据源（renewed=true，前端非阻塞提示后继续）
                emit_budget_state(
                    ctx.emitter.as_ref(),
                    &ctx.conv_id,
                    tool_round,
                    cumulative_tokens,
                    cumulative_cached_tokens,
                    cumulative_prompt_tokens,
                    effective_max_tokens,
                    initial_max_tokens,
                    budget_renewals,
                    ctx.budget.max_budget_renewals,
                    true,
                );
            } else {
                tracing::warn!(
                    target: "ice_paw.chat",
                    "Token 预算已超限（续期额度用尽）: cumulative={} > budget={}",
                    cumulative_tokens,
                    effective_max_tokens,
                );
                // 终态事件：让 HUD 停在最终值（与终止提示行的数字一致）
                emit_budget_state(
                    ctx.emitter.as_ref(),
                    &ctx.conv_id,
                    tool_round,
                    cumulative_tokens,
                    cumulative_cached_tokens,
                    cumulative_prompt_tokens,
                    effective_max_tokens,
                    initial_max_tokens,
                    budget_renewals,
                    ctx.budget.max_budget_renewals,
                    false,
                );
                finalize_guard_logged(
                    &ctx.pool,
                    &batch_writer,
                    &ev,
                    &current_asst_msg_id,
                    &msg_text,
                    &round_blocks,
                    round_completion_tokens,
                    Some(&budget_exceeded_fallback(
                        cumulative_tokens,
                        effective_max_tokens,
                        ctx.budget.max_budget_renewals,
                    )),
                    tool_round,
                    ctx.asst_model.as_deref(),
                    !continue_full_text.is_empty(),
                    round_gen_ms,
                )
                .await;
                return finalize_success(
                    ctx.emitter.as_ref(),
                    &ctx.pool,
                    &ev,
                    &current_asst_msg_id,
                    "budget_exceeded",
                    synthesize_usage(
                        first_prompt_tokens,
                        total_completion_tokens,
                        collected_usage,
                    ),
                    first_prompt_tokens,
                    tool_round + 1,
                )
                .await;
            }
        }

        // === M2.1: 停滞检测（落盘前）===
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
            finalize_guard_logged(
                &ctx.pool,
                &batch_writer,
                &ev,
                &current_asst_msg_id,
                &msg_text,
                &round_blocks,
                round_completion_tokens,
                Some("（检测到工具调用循环停滞，已停止。发送新消息即可继续。）"),
                tool_round,
                ctx.asst_model.as_deref(),
                !continue_full_text.is_empty(),
                round_gen_ms,
            )
            .await;
            return finalize_success(
                ctx.emitter.as_ref(),
                &ctx.pool,
                &ev,
                &current_asst_msg_id,
                "stuck",
                synthesize_usage(
                    first_prompt_tokens,
                    total_completion_tokens,
                    collected_usage,
                ),
                first_prompt_tokens,
                tool_round + 1,
            )
            .await;
        }

        // 【阶段 C】即时持久化当前 assistant（权威快照：content + blocks + 本轮 token）。
        // 到达此处 = 未命中任何终止条件（本轮有 tool_use 且即将执行工具）→ 此时落盘
        // tool_use 安全（紧随其后的阶段 E 会配对 tool_result）。
        // 先 flush_now 落盘 BatchWriter 的 streaming 文本，再同步写权威 blocks；本轮结束后
        // set_msg_id 会切到新消息，避免后到的 flush 覆盖本轮 blocks。
        batch_writer.flush_now().await;
        finalize_assistant_message(
            &ctx.pool,
            &current_asst_msg_id,
            &msg_text,
            &round_blocks,
            round_completion_tokens,
        )
        .await;
        // 权威快照事件（与上面落库的 content/blocks 同值；supersede：续写链同
        // message_id 多条，回放 last-wins）。
        event_log::log_assistant_message(
            &ctx.pool,
            &ev,
            &current_asst_msg_id,
            ctx.asst_model.as_deref(),
            &msg_text,
            &round_blocks,
            round_completion_tokens.map(|t| t.max(1) as i64),
            Some(round_gen_ms),
            tool_round,
            !continue_full_text.is_empty(),
        )
        .await;
        // 阶段 C 已对当前占位落库+发事件 → loop 顶 cancel 不再补 discard（见标记定义处）
        current_asst_finalized = true;

        // 最终轮（本轮无工具调用）→ 当前 assistant 已 finalize。
        // round_blocks 此时无 ToolUse，落盘安全。
        if completed_calls.is_empty() {
            // === 自动续写决策点（模式 C 治本）===
            // finish_reason=length(OpenAI)/max_tokens(Anthropic) 表示单轮输出被截断（可恢复态，
            // 非终态）。在续写次数预算内，跳回循环顶复用同一 assistant 消息续接输出：
            // 不发 chat:done、不建新占位、不 set_msg_id → 前端 streamingText 自然续接，单气泡。
            let truncated = matches!(round_finish_reason.as_str(), "length" | "max_tokens");
            if truncated && continue_rounds < MAX_CONTINUE_ROUNDS {
                continue_rounds += 1;
                // 累积器 = 当前完整全文，供下一轮 msg_text 拼接 + 全文覆写落盘
                continue_full_text = msg_text.clone();
                // 模型上下文：推【本轮文本】（非全文，避免 O(K²) 膨胀）+ 续写指令
                ctx.messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: vec![ContentBlock::text(round_text.clone())],
                    source_rowid: None,
                    source_seq: None,
                });
                ctx.messages.push(ChatMessage {
                    role: "user".into(),
                    content: vec![ContentBlock::text(
                        "（上一段因长度限制被中断。请直接从中断处继续输出剩余内容，不要重复已输出部分。）",
                    )],
                    source_rowid: None,
                    source_seq: None,
                });
                tracing::info!(
                    target: "ice_paw.chat",
                    "输出截断（finish_reason={}），自动续写第 {} 次",
                    round_finish_reason,
                    continue_rounds,
                );
                continue 'tool_round;
            }
            // 终态：自然结束（stop）或续写次数用尽仍截断（length/max_tokens → 前端显示「已达长度上限」）。
            // 上方阶段 C 的 finalize_assistant_message 已用 msg_text 落盘完整全文。
            return finalize_success(
                ctx.emitter.as_ref(),
                &ctx.pool,
                &ev,
                &current_asst_msg_id,
                &round_finish_reason,
                synthesize_usage(
                    first_prompt_tokens,
                    total_completion_tokens,
                    collected_usage,
                ),
                first_prompt_tokens,
                tool_round + 1,
            )
            .await;
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
            ctx.emitter.as_ref(),
            ctx.tool_app.as_ref(),
            &ctx.tool_registry,
            &ctx.auth_registry,
            &ctx.auth_session,
            &ctx.whitelist,
            &completed_calls,
            &tool_ctx,
            &current_asst_msg_id,
            &ev,
            &ctx.cancel,
            &ctx.hooks,
        )
        .await
        {
            Ok(blocks) => blocks,
            Err(e) => {
                // 工具执行编排整体失败 → 无法构造有效的 tool_result，已落盘的 tool_use
                // （阶段 C）会成孤儿。对称清场：剔除 tool_use 后 re-finalize（UPDATE 同行
                // 幂等覆盖），再 fail。若 round 无 text → 守卫删占位，update_error 会
                // NotFound（warn），但 chat:error / chat:done 事件照常 emit。
                let err_msg = format!("工具执行失败: {}", e);
                tracing::error!(target: "ice_paw.chat", "{}", err_msg);
                let _ = finalize_guard_logged(
                    &ctx.pool,
                    &batch_writer,
                    &ev,
                    &current_asst_msg_id,
                    &msg_text,
                    &round_blocks,
                    round_completion_tokens,
                    None,
                    tool_round,
                    ctx.asst_model.as_deref(),
                    !continue_full_text.is_empty(),
                    round_gen_ms,
                )
                .await;
                return fail_round_and_cancel(
                    ctx.emitter.as_ref(),
                    &ctx.pool,
                    &ev,
                    &current_asst_msg_id,
                    "internal",
                    &err_msg,
                    tool_round + 1,
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
                ctx.emitter.as_ref(),
                &ctx.pool,
                &ev,
                &current_asst_msg_id,
                "internal",
                &err_msg,
                tool_round + 1,
            )
            .await;
        }
        let result_json =
            serde_json::to_string(&tool_result_blocks).unwrap_or_else(|_| "[]".to_string());
        if let Err(e) =
            repo::message::update_content_blocks(&ctx.pool, &user_tool_msg_id, &result_json).await
        {
            // tool_result 写盘失败 → 已落盘的 tool_use（阶段 C）会成孤儿（user 占位
            // content_blocks='[]'，下次加载回退 content="" 被 sanitize 丢弃 → tool_use 无
            // 配对）。对称清场：剔除 assistant 的 tool_use + 删空 user 占位 + fail。
            // 当场暴露 SQLite I/O 故障，优于延迟到下次会话 400 爆炸。
            let err_msg = format!("持久化工具结果失败: {}", e);
            tracing::error!(target: "ice_paw.chat", "{}: msg_id={}", err_msg, user_tool_msg_id);
            let _ = finalize_guard_logged(
                &ctx.pool,
                &batch_writer,
                &ev,
                &current_asst_msg_id,
                &msg_text,
                &round_blocks,
                round_completion_tokens,
                None,
                tool_round,
                ctx.asst_model.as_deref(),
                !continue_full_text.is_empty(),
                round_gen_ms,
            )
            .await;
            if let Err(de) = repo::message::delete(&ctx.pool, &user_tool_msg_id).await {
                tracing::warn!(
                    target: "ice_paw.chat",
                    "删除空 user 占位失败: msg_id={}, err={}",
                    user_tool_msg_id,
                    de
                );
            }
            return fail_round_and_cancel(
                ctx.emitter.as_ref(),
                &ctx.pool,
                &ev,
                &current_asst_msg_id,
                "internal",
                &err_msg,
                tool_round + 1,
            )
            .await;
        }
        // tool_result 消息镜像事件（与上面落库的 content_blocks 同值；borrow 不 move，
        // 阶段 G 还要用 tool_result_blocks 推进 ctx.messages）。
        event_log::log_tool_result_message(&ctx.pool, &ev, &user_tool_msg_id, &tool_result_blocks)
            .await;

        // 【阶段 G】ctx.messages 追加本轮 assistant(tool_use) + user(tool_result)。
        // 统一 role=user：Anthropic 适配层直接支持 user 消息携带 tool_result；
        // OpenAI 适配层在 chat_message_to_openai 里把含 ToolResult 的消息展开为
        // 多条 role="tool"（每 tool_call_id 一条），满足其「tool_calls 后必须紧跟
        // tool 回执」的协议要求。
        let mut asst_blocks: Vec<ContentBlock> = Vec::new();
        if !round_text.is_empty() {
            asst_blocks.push(ContentBlock::Text {
                text: round_text.clone(),
            });
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
            source_seq: None,
        });
        ctx.messages.push(ChatMessage {
            role: "user".into(),
            content: tool_result_blocks,
            source_rowid: None,
            source_seq: None,
        });

        tracing::info!(
            target: "ice_paw.chat",
            "工具执行完成: round={}，准备下一轮 LLM 调用",
            tool_round,
        );

        // 【阶段 H】若是最后一轮，当前 assistant（已 finalize）作为最终消息收尾；
        // 否则创建下一轮 assistant 占位 + 切 BatchWriter + emit chat:assistant-start。
        // B1：轮数触顶先尝试自动续期（+初始轮数，判据同预算续期——失控由 stuck_detect
        // 独立兜底，额度有界保证总轮数封顶 = 初始 × (1+额度)），额度用尽才真停。
        if tool_round + 1 >= effective_max_rounds {
            if round_renewals < ctx.budget.max_round_renewals {
                round_renewals += 1;
                effective_max_rounds = effective_max_rounds.saturating_add(initial_max_rounds);
                tracing::info!(
                    target: "ice_paw.chat",
                    "工具轮数自动续期: 已达 {} 轮，续期 {}/{} → 新上限 {} 轮",
                    tool_round + 1,
                    round_renewals,
                    ctx.budget.max_round_renewals,
                    effective_max_rounds,
                );
            } else {
                tracing::info!(
                    target: "ice_paw.chat",
                    "已达最大工具调用轮数（{}），终止对话",
                    effective_max_rounds,
                );
                return finalize_success(
                    ctx.emitter.as_ref(),
                    &ctx.pool,
                    &ev,
                    &current_asst_msg_id,
                    "tool_use",
                    synthesize_usage(
                        first_prompt_tokens,
                        total_completion_tokens,
                        collected_usage,
                    ),
                    first_prompt_tokens,
                    tool_round + 1,
                )
                .await;
            }
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
                ctx.emitter.as_ref(),
                &ctx.pool,
                &ev,
                &current_asst_msg_id,
                "internal",
                &err_msg,
                tool_round + 1,
            )
            .await;
        }
        // 切 BatchWriter 到新 assistant（内部先 flush 当前 pending 再切 id）
        batch_writer.flush_now().await;
        batch_writer.set_msg_id(next_asst_id.clone()).await;
        // 通知前端：冻结上一条 assistant（写入其 tool_use/text/thinking）+ 插入 user(tool_result)
        // + 重置 streaming 状态 + push 新 assistant 占位。
        crate::harness::r#loop::emitter::emit_ser(
            ctx.emitter.as_ref(),
            "chat:assistant-start",
            &ChatAssistantStartPayload {
                conversation_id: ctx.conv_id.clone(),
                message_id: next_asst_id.clone(),
            },
        );
        current_asst_msg_id = next_asst_id;
        // 新占位未 finalize——loop 顶若命中 cancel 需补 message_discarded
        current_asst_finalized = false;
    }

    // 兜底：逻辑上不可达（`0u32..` 无限 range 下，正常流必在阶段 H 终止分支或
    // 顶部硬闸 return），保留以满足函数返回类型。current_asst_msg_id 此时为最后
    // 一条有内容的 assistant。
    finalize_success(
        ctx.emitter.as_ref(),
        &ctx.pool,
        &ev,
        &current_asst_msg_id,
        "tool_use",
        synthesize_usage(
            first_prompt_tokens,
            total_completion_tokens,
            collected_usage,
        ),
        first_prompt_tokens,
        effective_max_rounds,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::budget_exceeded_fallback;

    /// 显式硬上限分支：含数字 + 自助指引（注释掉该行恢复自适应+续期）
    #[test]
    fn budget_exceeded_fallback_explicit_cap_has_numbers_and_hint() {
        let s = budget_exceeded_fallback(845_000, 800_000, 0);
        assert!(s.contains("845000"), "应含累计数: {s}");
        assert!(s.contains("800000"), "应含上限数: {s}");
        assert!(s.contains("硬上限"), "应说明硬上限语义: {s}");
        assert!(s.contains("注释掉该行"), "应给自助指引: {s}");
    }

    /// 默认额度用尽分支：说明续期次数已耗完
    #[test]
    fn budget_exceeded_fallback_renewed_out_mentions_quota() {
        let s = budget_exceeded_fallback(1_900_000, 1_800_000, 2);
        assert!(s.contains("1900000"), "应含累计数: {s}");
        assert!(s.contains("1800000"), "应含上限数: {s}");
        assert!(s.contains("续期"), "应提及续期额度: {s}");
        assert!(!s.contains("注释掉该行"), "非硬上限不给 yaml 指引: {s}");
    }
}
