//! 会话事件日志的 typed emitters（session-event-log Phase 0）。
//!
//! 单一 append-only 事件日志的写入入口：每个持久化事实对应一个 `log_*`
//! emitter，payload 为强类型 struct（含 `v` 版本字段），序列化后经
//! [`crate::db::repo::session_event::append`] 落库。Phase 0 影子定位：
//! **append 失败仅 warn，不阻断主流程**——产生的缺口无 seq 空洞（MAX+1
//! 连续），只能靠 Phase 1 derive 对账发现，是已文档化的定位取舍。
//!
//! 硬规则（保序）：事件一律 inline `.await`，禁止 `tokio::spawn` 包裹。
//! `turn_ended` 必须在 cleanup() unregister 之前落，保证跨 turn 的 seq 序
//! 确定（同会话 turn 串行由 ChatState 保证）。进程死亡绕过全部退出路径的
//! 兜底 = [`sweep_interrupted_turns`]（lib.rs 启动时调用，幂等）。
//!
//! 接线一律在 harness/command 语义层，不放 repo 层（repo 不知 turn 语境，
//! 且「占位 create + finalize」两点会双记）。
//!
//! 词表与不入日志项的完整清单见 `migrations/44_session_events.sql` 头注释
//! 与 docs（BatchWriter 流式 flush / 合成续写 prompt / 工具排序均不入）。

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::OnceLock;
use tokio::sync::broadcast;

use crate::db::repo::{self, session_event};
use crate::infra::protocol::{ContentBlock, TokenUsage};

/// 一个 turn 的事件上下文——同一 `send_message` 周期构造一次，全程复用。
///
/// `turn_id` 即 user_msg_id（每 turn 现生成 UUID v4，1:1 于 turn；重发 =
/// 新 turn）。`agent_id` 用于 actor 标注（本期会话恒为该 agent，多 agent
/// 通道落地时复用同一结构）。
pub struct EventCtx {
    pub conv_id: String,
    pub turn_id: String,
    pub agent_id: String,
}

impl EventCtx {
    /// 从 LoopConfig 字段构造（conv_id / user_msg_id / agent_id）。
    pub fn new(conv_id: &str, turn_id: &str, agent_id: &str) -> Self {
        Self {
            conv_id: conv_id.to_string(),
            turn_id: turn_id.to_string(),
            agent_id: agent_id.to_string(),
        }
    }

    /// actor 列取值：`agent:<uuid>`。
    pub fn agent_actor(&self) -> String {
        format!("agent:{}", self.agent_id)
    }
}

/// actor 列取值：`user`。
pub fn actor_user() -> &'static str {
    "user"
}

// =========================================================================
// 事件通知总线（轨迹 live v2：append 即通知，前端事件驱动拉增量）
// =========================================================================

/// 「事件已落库」通知（轻 payload：只带定位所需字段，前端按会话过滤后
/// 用已载 max_seq 作游标 `list_after` 拉增量——不内嵌事件本体，避免双写）。
#[derive(Debug, Clone, Serialize)]
pub struct SessionEventAppended {
    pub conversation_id: String,
    pub kind: String,
}

/// 进程内广播通道。append_event 是全部 13 kind 的唯一汇聚点，在这里 send
/// 一条通知即可覆盖所有事件源（含未来新增 kind），无需逐调用方接线。
/// 无订阅者时 send 返回 Err——直接忽略（dev 测试 / 订阅任务未起时安静跳过）。
static EVENT_BUS: OnceLock<broadcast::Sender<SessionEventAppended>> = OnceLock::new();

pub fn event_bus() -> &'static broadcast::Sender<SessionEventAppended> {
    EVENT_BUS.get_or_init(|| broadcast::channel(256).0)
}

// =========================================================================
// 事件类型常量（kind 词表）
// =========================================================================

pub mod kind {
    pub const TURN_CONTEXT: &str = "turn_context";
    pub const USER_MESSAGE: &str = "user_message";
    pub const ASSISTANT_MESSAGE: &str = "assistant_message";
    pub const TOOL_EXECUTION: &str = "tool_execution";
    pub const TOOL_RESULT_MESSAGE: &str = "tool_result_message";
    pub const ATTACHMENT_STORED: &str = "attachment_stored";
    pub const SUMMARY_CREATED: &str = "summary_created";
    pub const SUMMARY_UPDATED: &str = "summary_updated";
    pub const MESSAGE_ERROR: &str = "message_error";
    pub const MESSAGE_DISCARDED: &str = "message_discarded";
    pub const TURN_ENDED: &str = "turn_ended";
    pub const MODAL_ADAPTED: &str = "modal_adapted";
    pub const HOOK_INJECTED: &str = "hook_injected";
    pub const PLAN_UPDATED: &str = "plan_updated";
}

// =========================================================================
// Payload 类型（每 kind 一个，全部带 v 版本字段）
// =========================================================================

fn version_one() -> u8 {
    1
}

/// turn 开始时的模型/工具/预算快照——「模型看到什么工具、用什么模型」
/// 此前完全不落库，Phase 1 解释行为差异的锚点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnContextPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    pub provider: String,
    pub effective_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    pub tools_enabled: bool,
    /// 本 turn 组装进系统提示的工具名快照（≤50，成员集；排序不入日志）
    pub tool_names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_max_rounds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
}

/// 用户消息（落库原文 + 原始 blocks，含 Attachment 元信息块与图片——
/// 适配前版本；视觉代读结果是 `modal_adapted` 事件的事）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessagePayload {
    #[serde(default = "version_one")]
    pub v: u8,
    pub content: String,
    pub blocks: Vec<ContentBlock>,
}

/// assistant 消息权威快照（每轮 finalize 点一条）。
///
/// **supersede 语义**：自动续写场景同一 message_id 会有多条本事件
/// （全文覆写），回放 last-wins。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessagePayload {
    #[serde(default = "version_one")]
    pub v: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub content: String,
    pub blocks: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_count: Option<i64>,
    /// 本轮生成耗时（stream 开始 → finalize，毫秒；补齐后轨迹耗时投影有模型道
    /// 真实条宽，且不受 created_at 秒精度限制）。旧事件无此字段 → None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// 工具轮序（0 起）
    pub round: u32,
    /// 自动续写（finish_reason=length/max_tokens 触发的同气泡续写）
    pub continuation: bool,
}

/// 工具执行审计事实——镜像 `tool_calls` 表行（同一截断策略）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecutionPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    pub tool_name: String,
    pub arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    pub is_error: bool,
    pub duration_ms: u64,
}

/// 工具结果消息镜像（role='user' 含 ToolResult 块的行，derive 直接用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultMessagePayload {
    #[serde(default = "version_one")]
    pub v: u8,
    pub blocks: Vec<ContentBlock>,
}

/// 附件留存事实——**仅元信息**，正文/字节禁入（防三重冗余：内联首页在
/// user_message.blocks，未内联页经工具读取时出现在 tool_result）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttachmentStoredPayload {
    /// message_attachments 分页文本块
    Pages {
        #[serde(default = "version_one")]
        v: u8,
        items: Vec<AttachmentPageItem>,
    },
    /// message_attachment_files 原始字节（PDF 视觉候选）
    Bytes {
        #[serde(default = "version_one")]
        v: u8,
        items: Vec<AttachmentBytesItem>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentPageItem {
    pub idx: i64,
    pub name: String,
    pub kind: String,
    pub label: String,
    pub token_est: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentBytesItem {
    pub idx: i64,
    pub name: String,
    pub ext: String,
    pub bytes_len: usize,
}

/// 滚动摘要创建/更新（Phase 2 债：covered_until_rowid 是 messages 物理
/// rowid，切事件主源后需改为 covered_until_seq）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    pub summary_message_id: String,
    pub content: String,
    pub covered_until_rowid: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageErrorPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    /// 错误分类（AppErrorKind 名），便于按类检索
    pub kind: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDiscardedPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    pub reason: String,
}

/// turn 终态——终止原因此前完全不落库，本事件是新增价值点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnEndedPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    /// stop | length | max_tokens | tool_use | budget_exceeded | stuck | abort | error |
    /// interrupted（boot 自愈补记：进程死亡时 turn 中断，非任何退出路径产生）
    pub termination: String,
    pub rounds: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_token_count: Option<i32>,
}

/// 视觉模态适配（投影期，模型实际看到的内容变更）——「Model-visible
/// means logged」：OCR 代读文本替代了图片，是模型真实消费的内容，入日志。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModalAdaptedPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    /// user_image | tool_image | history
    pub stage: String,
    /// vision_passthrough | ocr_substitute | strip_to_marker 等
    pub mode: String,
    pub items: Vec<ModalAdaptedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModalAdaptedItem {
    /// 图片在 blocks 中的下标
    pub index: usize,
    /// kept | dropped | substituted
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_text: Option<String>,
}

/// 钩子注入（模型可见但此前零持久化的事实）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookInjectedPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    /// conversation_start | before_llm
    pub point: String,
    pub prompt: String,
}

/// 计划快照（`update_plan` 工具整体覆写；回放 last-wins 取最后一条 = 当前计划）。
///
/// 计划是**意图文档**（会话内容），不是任务（执行单元=委派会话）：正交抽象，
/// 靠 [`PlanItem::task_conversation_id`] 引用边关联——条目勾选是 agent 的判断，
/// 不从任务终态自动映射（任务 done ≠ 条目达标，agent 可能判「不行，重派」）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanUpdatedPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    pub items: Vec<PlanItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItem {
    pub text: String,
    /// pending | in_progress | done
    pub status: String,
    /// 条目挂的委派子会话 id（跳转用；None = agent 自己做/未挂接）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_conversation_id: Option<String>,
}

// =========================================================================
// Emitters（全部 warn-only；inline await，禁止 spawn）
// =========================================================================

/// 内部公共入口：序列化 + append + 失败 warn。
async fn append_event(
    pool: &SqlitePool,
    ctx: &EventCtx,
    kind: &str,
    actor: &str,
    message_id: Option<&str>,
    payload: &impl Serialize,
) {
    let json = match serde_json::to_string(payload) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(target: "ice_paw.event_log", "事件 payload 序列化失败 kind={kind} err={e}");
            return;
        }
    };
    match session_event::append(
        pool,
        &ctx.conv_id,
        kind,
        actor,
        Some(&ctx.turn_id),
        message_id,
        &json,
    )
    .await
    {
        Ok(_) => {
            // 落库成功 → 广播通知（同步非阻塞；订阅方 lib.rs 转 Tauri event 推前端）。
            // 不违反「inline await 禁 spawn」：send 是同步操作，无任务逃逸。
            let _ = event_bus().send(SessionEventAppended {
                conversation_id: ctx.conv_id.clone(),
                kind: kind.to_string(),
            });
        }
        Err(e) => {
            tracing::warn!(target: "ice_paw.event_log", "事件写入失败 kind={kind} conv={} err={e}", ctx.conv_id);
        }
    }
}

/// turn 快照（actor=agent：上下文由 agent 侧组装）。
pub async fn log_turn_context(pool: &SqlitePool, ctx: &EventCtx, payload: &TurnContextPayload) {
    append_event(
        pool,
        ctx,
        kind::TURN_CONTEXT,
        &ctx.agent_actor(),
        None,
        payload,
    )
    .await;
}

/// 用户消息落库原文（actor=user）。
pub async fn log_user_message(
    pool: &SqlitePool,
    ctx: &EventCtx,
    message_id: &str,
    payload: &UserMessagePayload,
) {
    append_event(
        pool,
        ctx,
        kind::USER_MESSAGE,
        actor_user(),
        Some(message_id),
        payload,
    )
    .await;
}

/// assistant 权威快照（actor=agent；supersede：同 message_id 多条 last-wins）。
pub async fn log_assistant_message(
    pool: &SqlitePool,
    ctx: &EventCtx,
    message_id: &str,
    payload: &AssistantMessagePayload,
) {
    append_event(
        pool,
        ctx,
        kind::ASSISTANT_MESSAGE,
        &ctx.agent_actor(),
        Some(message_id),
        payload,
    )
    .await;
}

/// 工具执行审计（arguments/result 与 tool_calls 表同一截断策略）。
// 10 参数逐一镜像 tool_calls 审计行字段（emitter 内做截断，避免调用方各截一遍）；
// 收敛成 struct 会与 ToolExecutionPayload 本体重复。
#[allow(clippy::too_many_arguments)]
pub async fn log_tool_execution(
    pool: &SqlitePool,
    ctx: &EventCtx,
    message_id: &str,
    tool_call_id: &str,
    tool_use_id: Option<&str>,
    tool_name: &str,
    arguments: &str,
    result: Option<&str>,
    is_error: bool,
    duration_ms: u64,
) {
    let payload = ToolExecutionPayload {
        v: 1,
        tool_call_id: tool_call_id.to_string(),
        tool_use_id: tool_use_id.map(str::to_string),
        tool_name: tool_name.to_string(),
        arguments: repo::tool_call::truncate(arguments, repo::tool_call::MAX_ARGUMENTS_LEN),
        result: result.map(|r| repo::tool_call::truncate(r, repo::tool_call::MAX_RESULT_LEN)),
        is_error,
        duration_ms,
    };
    append_event(
        pool,
        ctx,
        kind::TOOL_EXECUTION,
        &ctx.agent_actor(),
        Some(message_id),
        &payload,
    )
    .await;
}

/// 工具结果消息镜像。
pub async fn log_tool_result_message(
    pool: &SqlitePool,
    ctx: &EventCtx,
    message_id: &str,
    blocks: &[ContentBlock],
) {
    let payload = ToolResultMessagePayload {
        v: 1,
        blocks: blocks.to_vec(),
    };
    append_event(
        pool,
        ctx,
        kind::TOOL_RESULT_MESSAGE,
        &ctx.agent_actor(),
        Some(message_id),
        &payload,
    )
    .await;
}

/// 附件留存（仅元信息）。
pub async fn log_attachment_stored(
    pool: &SqlitePool,
    ctx: &EventCtx,
    message_id: &str,
    payload: &AttachmentStoredPayload,
) {
    append_event(
        pool,
        ctx,
        kind::ATTACHMENT_STORED,
        actor_user(),
        Some(message_id),
        payload,
    )
    .await;
}

/// 摘要创建（折叠由 turn 的上下文装配触发，事件序先于同 turn 的 user_message）。
pub async fn log_summary_created(pool: &SqlitePool, ctx: &EventCtx, payload: &SummaryPayload) {
    append_event(
        pool,
        ctx,
        kind::SUMMARY_CREATED,
        &ctx.agent_actor(),
        None,
        payload,
    )
    .await;
}

/// 摘要更新。
pub async fn log_summary_updated(pool: &SqlitePool, ctx: &EventCtx, payload: &SummaryPayload) {
    append_event(
        pool,
        ctx,
        kind::SUMMARY_UPDATED,
        &ctx.agent_actor(),
        None,
        payload,
    )
    .await;
}

/// 消息错误（对应 messages.error 回写）。
pub async fn log_message_error(
    pool: &SqlitePool,
    ctx: &EventCtx,
    message_id: &str,
    error_kind: &str,
    error: &str,
) {
    let payload = MessageErrorPayload {
        v: 1,
        kind: error_kind.to_string(),
        error: error.to_string(),
    };
    append_event(
        pool,
        ctx,
        kind::MESSAGE_ERROR,
        &ctx.agent_actor(),
        Some(message_id),
        &payload,
    )
    .await;
}

/// 消息废弃（终止守卫删占位行）。
pub async fn log_message_discarded(
    pool: &SqlitePool,
    ctx: &EventCtx,
    message_id: &str,
    reason: &str,
) {
    let payload = MessageDiscardedPayload {
        v: 1,
        reason: reason.to_string(),
    };
    append_event(
        pool,
        ctx,
        kind::MESSAGE_DISCARDED,
        &ctx.agent_actor(),
        Some(message_id),
        &payload,
    )
    .await;
}

/// turn 终态。**必须在 cleanup() unregister 之前 inline await**。
pub async fn log_turn_ended(
    pool: &SqlitePool,
    ctx: &EventCtx,
    final_message_id: Option<&str>,
    payload: &TurnEndedPayload,
) {
    append_event(
        pool,
        ctx,
        kind::TURN_ENDED,
        &ctx.agent_actor(),
        final_message_id,
        payload,
    )
    .await;
}

/// 崩溃自愈扫尾（boot-time，幂等）：为全部未闭合 turn 补记 truthful 终态。
///
/// 终止事件只挂在进程内退出路径上，进程死亡（崩溃/kill/断电/关窗时在途）
/// 绕过所有路径——turn 从此只有开没有关，轨迹永远「进行中」，并毒害未来的
/// turn_ended 派生状态机（MA-2 台账）。本地单进程应用在启动时刻可确定性
/// 判定：任何未闭合 turn 都已死。补记 `termination="interrupted"` 是事实
/// （进程中断时的 turn 确实中断了），非伪造；不违反 append-only（只新增）。
/// derive/reconcile 零影响：turn_ended 是 skip 事件不产生行。
///
/// 返回补记条数（0 = 干净启动，零写入）。查询失败仅 warn 不阻断启动——
/// 与 heal_checksum_drift / fix_orphan_tool_results 同款 boot 自愈定位。
pub async fn sweep_interrupted_turns(pool: &SqlitePool) -> usize {
    let open = match repo::session_event::find_open_turns(pool).await {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(target: "ice_paw.event_log", "崩溃自愈扫尾查询失败（不影响启动）: {e}");
            return 0;
        }
    };
    let mut swept = 0usize;
    for (session_id, turn_id, actor, rounds) in open {
        // actor 复用原 turn_context 行的归属（agent:<uuid>），不自造 system
        // actor——事件闭合的是该 agent 的 turn；EventCtx 仅承载定位三元组
        // （agent_id 不经 agent_actor() 路径，留空）。
        let ctx = EventCtx::new(&session_id, &turn_id, "");
        let payload = TurnEndedPayload {
            v: version_one(),
            termination: "interrupted".to_string(),
            rounds: rounds.max(0) as u32,
            // usage 无法从事后回溯（崩溃时未落盘），None 诚实
            usage: None,
            user_token_count: None,
        };
        append_event(pool, &ctx, kind::TURN_ENDED, &actor, None, &payload).await;
        swept += 1;
    }
    swept
}

/// 视觉模态适配（投影期模型可见内容变更）。
pub async fn log_modal_adapted(pool: &SqlitePool, ctx: &EventCtx, payload: &ModalAdaptedPayload) {
    append_event(
        pool,
        ctx,
        kind::MODAL_ADAPTED,
        &ctx.agent_actor(),
        None,
        payload,
    )
    .await;
}

/// 钩子注入。
pub async fn log_hook_injected(pool: &SqlitePool, ctx: &EventCtx, payload: &HookInjectedPayload) {
    append_event(
        pool,
        ctx,
        kind::HOOK_INJECTED,
        &ctx.agent_actor(),
        None,
        payload,
    )
    .await;
}

/// 计划快照（`update_plan` 工具调用点 emit；message_id=None——工具调用的
/// assistant 关联由同 turn 的 tool_execution 事件承载，这里只需 turn 归组）。
pub async fn log_plan_updated(pool: &SqlitePool, ctx: &EventCtx, payload: &PlanUpdatedPayload) {
    append_event(
        pool,
        ctx,
        kind::PLAN_UPDATED,
        &ctx.agent_actor(),
        None,
        payload,
    )
    .await;
}

// =========================================================================
// 单元测试（round-trip：写 → 读 → 反序列化 → 字段断言）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::SessionEventRow;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite")
    }

    async fn seed(pool: &SqlitePool) {
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES ('agent-1', 't', 'anthropic', 'claude-test', '', '', 0.7, 1024, '{}', 0, 0)",
        )
        .execute(pool)
        .await
        .expect("seed agent");
        sqlx::query(
            "INSERT INTO conversations (id, agent_id, title) VALUES ('conv-1', 'agent-1', 't')",
        )
        .execute(pool)
        .await
        .expect("seed conversation");
    }

    fn ctx() -> EventCtx {
        EventCtx::new("conv-1", "turn-1", "agent-1")
    }

    /// 读回该会话唯一一条事件的 payload 并反序列化。
    async fn sole_event_payload<T: serde::de::DeserializeOwned>(pool: &SqlitePool) -> T {
        let rows = session_event::list_by_session(pool, "conv-1", None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "应恰好写入 1 条事件");
        serde_json::from_str(&rows[0].payload).expect("payload 反序列化")
    }

    #[tokio::test]
    async fn user_message_round_trip() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool).await;

        let payload = UserMessagePayload {
            v: 1,
            content: "看这张图".into(),
            blocks: vec![
                ContentBlock::Text {
                    text: "看这张图".into(),
                },
                ContentBlock::Attachment {
                    name: "plan.pdf".into(),
                    kind: "application/pdf".into(),
                    size: 282_000,
                },
            ],
        };
        log_user_message(&pool, &ctx(), "msg-u1", &payload).await;

        let row: SessionEventRow = session_event::list_by_session(&pool, "conv-1", None)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(row.kind, "user_message");
        assert_eq!(row.actor, "user");
        assert_eq!(row.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(row.message_id.as_deref(), Some("msg-u1"));

        let back: UserMessagePayload = serde_json::from_str(&row.payload).unwrap();
        assert_eq!(back.content, "看这张图");
        assert_eq!(back.blocks.len(), 2);
        assert!(matches!(back.blocks[1], ContentBlock::Attachment { .. }));
    }

    #[tokio::test]
    async fn assistant_message_round_trip_and_supersede() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool).await;

        let mk = |content: &str, round: u32, continuation: bool| AssistantMessagePayload {
            v: 1,
            model: Some("glm-5.2".into()),
            content: content.into(),
            blocks: vec![ContentBlock::Text {
                text: content.into(),
            }],
            token_count: Some(42),
            duration_ms: Some(3_500),
            round,
            continuation,
        };
        // 自动续写：同 message_id 两条（supersede last-wins）
        log_assistant_message(&pool, &ctx(), "msg-a1", &mk("前半段", 0, false)).await;
        log_assistant_message(&pool, &ctx(), "msg-a1", &mk("前半段后半段", 1, true)).await;

        let rows = session_event::list_by_session(&pool, "conv-1", None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|r| r.kind == "assistant_message" && r.actor == "agent:agent-1"));
        let last: AssistantMessagePayload = serde_json::from_str(&rows[1].payload).unwrap();
        assert_eq!(last.content, "前半段后半段");
        assert!(last.continuation);
        assert_eq!(last.round, 1);
        assert_eq!(last.duration_ms, Some(3_500));
        // 旧事件（无 duration_ms 字段）反序列化 → None，前端隐式耗时兜底的输入
        let legacy: AssistantMessagePayload = serde_json::from_str(
            r#"{"v":1,"content":"旧","blocks":[],"round":0,"continuation":false}"#,
        )
        .unwrap();
        assert_eq!(legacy.duration_ms, None);
    }

    #[tokio::test]
    async fn tool_execution_truncates_like_audit_row() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool).await;

        let long = "x".repeat(9_000);
        log_tool_execution(
            &pool,
            &ctx(),
            "msg-a1",
            "tc-1",
            Some("tu-1"),
            "write_file",
            &long,
            Some(&long),
            true,
            1_234,
        )
        .await;

        let back: ToolExecutionPayload = sole_event_payload(&pool).await;
        assert_eq!(
            back.arguments.chars().count(),
            4_000 + "…[已截断]".chars().count()
        );
        assert!(back.result.as_deref().unwrap().ends_with("…[已截断]"));
        assert!(back.is_error);
        assert_eq!(back.duration_ms, 1_234);
        assert_eq!(back.tool_use_id.as_deref(), Some("tu-1"));
    }

    #[tokio::test]
    async fn sweep_interrupted_turns_closes_open_and_idempotent() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool).await;
        sqlx::query(
            "INSERT INTO conversations (id, agent_id, title) VALUES ('conv-2', 'agent-1', 't')",
        )
        .execute(&pool)
        .await
        .expect("seed conversation 2");

        // conv-1：崩溃残留——turn_context + assistant_message 已落，无 turn_ended
        // （payload 对扫尾不敏感，只看 kind/turn_id）
        session_event::append(
            &pool,
            "conv-1",
            kind::TURN_CONTEXT,
            "agent:agent-1",
            Some("turn-open"),
            None,
            "{}",
        )
        .await
        .unwrap();
        session_event::append(
            &pool,
            "conv-1",
            kind::ASSISTANT_MESSAGE,
            "agent:agent-1",
            Some("turn-open"),
            Some("msg-a1"),
            "{}",
        )
        .await
        .unwrap();
        // conv-2：正常闭合 turn（扫尾不得触碰）
        let closed_ctx = EventCtx::new("conv-2", "turn-closed", "agent-1");
        session_event::append(
            &pool,
            "conv-2",
            kind::TURN_CONTEXT,
            "agent:agent-1",
            Some("turn-closed"),
            None,
            "{}",
        )
        .await
        .unwrap();
        log_turn_ended(
            &pool,
            &closed_ctx,
            Some("msg-a2"),
            &TurnEndedPayload {
                v: 1,
                termination: "stop".into(),
                rounds: 1,
                usage: None,
                user_token_count: None,
            },
        )
        .await;

        // 扫尾：只补 conv-1 的未闭合 turn
        assert_eq!(sweep_interrupted_turns(&pool).await, 1);
        let rows = session_event::list_by_session(&pool, "conv-1", None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 3, "补记后 conv-1 应有 3 条事件");
        let last = &rows[2];
        assert_eq!(last.kind, kind::TURN_ENDED);
        assert_eq!(last.turn_id.as_deref(), Some("turn-open"));
        assert_eq!(last.actor, "agent:agent-1", "复用原 turn_context 的 actor");
        assert_eq!(last.message_id, None);
        let p: TurnEndedPayload = serde_json::from_str(&last.payload).unwrap();
        assert_eq!(p.termination, "interrupted");
        assert_eq!(p.rounds, 1, "rounds = 已落 assistant_message 事件数");
        assert!(p.usage.is_none());

        // conv-2 不受干扰（仍 2 条，无新增）
        assert_eq!(
            session_event::list_by_session(&pool, "conv-2", None)
                .await
                .unwrap()
                .len(),
            2
        );

        // 幂等：再扫零补记、零写入
        assert_eq!(sweep_interrupted_turns(&pool).await, 0);
        assert_eq!(
            session_event::list_by_session(&pool, "conv-1", None)
                .await
                .unwrap()
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn turn_context_and_turn_ended_round_trip() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool).await;

        log_turn_context(
            &pool,
            &ctx(),
            &TurnContextPayload {
                v: 1,
                provider: "anthropic".into(),
                effective_model: "glm-5.2".into(),
                model_override: Some("glm-5-turbo".into()),
                tools_enabled: true,
                tool_names: vec!["read_file".into(), "run_command".into()],
                temperature: Some(0.7),
                max_tokens: Some(16_384),
                tool_max_rounds: Some(12),
                budget_max_tokens: Some(200_000),
                context_window: Some(1_000_000),
            },
        )
        .await;

        let back: TurnContextPayload = sole_event_payload(&pool).await;
        assert_eq!(back.effective_model, "glm-5.2");
        assert_eq!(back.tool_names.len(), 2);
        assert_eq!(back.v, 1);

        // 换 turn 重新 seed 事件表不可行（append-only），直接在新会话验证 turn_ended
        sqlx::query(
            "INSERT INTO conversations (id, agent_id, title) VALUES ('conv-2', 'agent-1', 't')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let ctx2 = EventCtx::new("conv-2", "turn-2", "agent-1");
        log_turn_ended(
            &pool,
            &ctx2,
            Some("msg-a1"),
            &TurnEndedPayload {
                v: 1,
                termination: "budget_exceeded".into(),
                rounds: 3,
                usage: Some(TokenUsage {
                    prompt_tokens: 10_000,
                    completion_tokens: 2_000,
                    cached_tokens: 512,
                }),
                user_token_count: Some(120),
            },
        )
        .await;

        let rows = session_event::list_by_session(&pool, "conv-2", None)
            .await
            .unwrap();
        let back: TurnEndedPayload = serde_json::from_str(&rows[0].payload).unwrap();
        assert_eq!(back.termination, "budget_exceeded");
        assert_eq!(back.usage.unwrap().completion_tokens, 2_000);
        assert_eq!(back.user_token_count, Some(120));
    }

    #[tokio::test]
    async fn plan_updated_round_trip() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool).await;

        // 全量覆写语义：两次调用两条事件，回放 last-wins（最后一条 = 当前计划）
        log_plan_updated(
            &pool,
            &ctx(),
            &PlanUpdatedPayload {
                v: 1,
                items: vec![
                    PlanItem {
                        text: "调研渲染方案".into(),
                        status: "done".into(),
                        task_conversation_id: None,
                    },
                    PlanItem {
                        text: "设计评审".into(),
                        status: "in_progress".into(),
                        task_conversation_id: Some("conv-child-1".into()),
                    },
                ],
            },
        )
        .await;
        log_plan_updated(
            &pool,
            &ctx(),
            &PlanUpdatedPayload {
                v: 1,
                items: vec![
                    PlanItem {
                        text: "设计评审".into(),
                        status: "done".into(),
                        task_conversation_id: Some("conv-child-1".into()),
                    },
                    PlanItem {
                        text: "终稿交付".into(),
                        status: "pending".into(),
                        task_conversation_id: None,
                    },
                ],
            },
        )
        .await;

        let rows = session_event::list_by_session(&pool, "conv-1", None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| {
            r.kind == "plan_updated" && r.actor == "agent:agent-1" && r.message_id.is_none()
        }));
        let last: PlanUpdatedPayload = serde_json::from_str(&rows[1].payload).unwrap();
        assert_eq!(last.items.len(), 2);
        assert_eq!(last.items[0].status, "done");
        assert_eq!(
            last.items[0].task_conversation_id.as_deref(),
            Some("conv-child-1")
        );
        assert_eq!(last.items[1].task_conversation_id, None);
    }

    #[tokio::test]
    async fn attachment_modal_hook_summary_round_trip() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool).await;

        log_attachment_stored(
            &pool,
            &ctx(),
            "msg-u1",
            &AttachmentStoredPayload::Bytes {
                v: 1,
                items: vec![AttachmentBytesItem {
                    idx: 0,
                    name: "plan.pdf".into(),
                    ext: "pdf".into(),
                    bytes_len: 282_000,
                }],
            },
        )
        .await;
        let back: AttachmentStoredPayload = sole_event_payload(&pool).await;
        match back {
            AttachmentStoredPayload::Bytes { items, .. } => {
                assert_eq!(items[0].bytes_len, 282_000);
                // 元信息 only：payload 里不得出现 BLOB/base64
                assert!(items[0].name == "plan.pdf");
            }
            other => panic!("应反序列化为 Bytes 变体，got {other:?}"),
        }

        // modal_adapted（含 OCR 全文）
        sqlx::query("DELETE FROM session_events")
            .execute(&pool)
            .await
            .unwrap();
        log_modal_adapted(
            &pool,
            &ctx(),
            &ModalAdaptedPayload {
                v: 1,
                stage: "user_image".into(),
                mode: "ocr_substitute".into(),
                items: vec![ModalAdaptedItem {
                    index: 0,
                    outcome: "substituted".into(),
                    ocr_text: Some("一张户型图：三室两厅".into()),
                }],
            },
        )
        .await;
        let back: ModalAdaptedPayload = sole_event_payload(&pool).await;
        assert_eq!(
            back.items[0].ocr_text.as_deref(),
            Some("一张户型图：三室两厅")
        );

        // hook_injected
        sqlx::query("DELETE FROM session_events")
            .execute(&pool)
            .await
            .unwrap();
        log_hook_injected(
            &pool,
            &ctx(),
            &HookInjectedPayload {
                v: 1,
                point: "before_llm".into(),
                prompt: "注意编码规范".into(),
            },
        )
        .await;
        let back: HookInjectedPayload = sole_event_payload(&pool).await;
        assert_eq!(back.point, "before_llm");

        // summary
        sqlx::query("DELETE FROM session_events")
            .execute(&pool)
            .await
            .unwrap();
        log_summary_updated(
            &pool,
            &ctx(),
            &SummaryPayload {
                v: 1,
                summary_message_id: "msg-s1".into(),
                content: "[Previous conversation summary] ...".into(),
                covered_until_rowid: 77,
            },
        )
        .await;
        let back: SummaryPayload = sole_event_payload(&pool).await;
        assert_eq!(back.covered_until_rowid, 77);
    }
}
