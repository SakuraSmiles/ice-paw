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
    pub const MODEL_SWITCH: &str = "model_switch";
    pub const CROSS_SESSION_MESSAGE: &str = "cross_session_message";
    pub const CROSS_SESSION_MESSAGE_SETTLED: &str = "cross_session_message_settled";
    pub const CONTEXT_BREAKDOWN: &str = "context_breakdown";
}

// =========================================================================
// Payload 类型（每 kind 一个，全部带 v 版本字段）
// =========================================================================

fn version_one() -> u8 {
    1
}

/// 消息类 payload（blocks 含图）的当前版本：v2 = Image 走 `image_ref` 轻量
/// 引用（S1 阶段 3b 起），payload 不再内联 base64。旧事件显式 `"v":1`
/// 照常反序列化；缺 `v` 字段的极旧 payload 保守按 1 解（v1 内联兼容）。
fn version_two() -> u8 {
    2
}

// =========================================================================
// PayloadBlock（消息类 payload 的块形态，S1 阶段 3 Image 双份存储治理）
// =========================================================================

fn image_ref_marker() -> String {
    "image_ref".to_string()
}

/// 消息类事件 payload 里的块。untagged 双形态：
///
/// - `Full`：完整块内联（v1，全部旧事件；非 Image 块恒为此形态）
/// - `ImageRef`：Image 轻量引用（v2）——payload 不再双份存 base64，字节只在
///   messages 行 `content_blocks`；`block_index` 指向该行数组的下标。
///
/// untagged 反序列化先试 `Full`（ContentBlock 以 `type` 判别，`"image_ref"`
/// 不在词表 → 失败）再落 `ImageRef`——v1 事件零迁移可读。**读侧必须经
/// `hydrate_image_refs`（derive.rs）还原后才能进对账 / LLM 视图**。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PayloadBlock {
    Full(ContentBlock),
    ImageRef {
        /// 判别标记，恒 `"image_ref"`（序列化写出；人读 + JSON 级水合定位用）
        #[serde(default = "image_ref_marker", rename = "type")]
        marker: String,
        message_id: String,
        block_index: usize,
    },
}

/// blocks → payload 形态：非 Image 原样（Full），Image 换轻量引用。
///
/// **Image 治理的写侧唯一入口**（emitter / backfill 合成内部调用；v1 事件
/// 已不可再产生）。索引 = 入参数组下标 = 行 `content_blocks` 布局——调用方
/// 必须传「刚落库的同一 blocks」（ref 是行内指针，不是独立拷贝）。
pub fn refify_blocks(message_id: &str, blocks: &[ContentBlock]) -> Vec<PayloadBlock> {
    blocks
        .iter()
        .enumerate()
        .map(|(i, b)| match b {
            ContentBlock::Image { .. } => PayloadBlock::ImageRef {
                marker: image_ref_marker(),
                message_id: message_id.to_string(),
                block_index: i,
            },
            other => PayloadBlock::Full(other.clone()),
        })
        .collect()
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
    #[serde(default = "version_two")]
    pub v: u8,
    pub content: String,
    pub blocks: Vec<PayloadBlock>,
}

/// assistant 消息权威快照（每轮 finalize 点一条）。
///
/// **supersede 语义**：自动续写场景同一 message_id 会有多条本事件
/// （全文覆写），回放 last-wins。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessagePayload {
    #[serde(default = "version_two")]
    pub v: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub content: String,
    pub blocks: Vec<PayloadBlock>,
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
    #[serde(default = "version_two")]
    pub v: u8,
    pub blocks: Vec<PayloadBlock>,
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

/// 滚动摘要创建/更新。锚点双值：`covered_until_seq`（事件纪元主锚，Phase 2B
/// 阶段 2 起）+ `covered_until_rowid`（物理 rowid 兜底，保持旧事件可读）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    pub summary_message_id: String,
    pub content: String,
    pub covered_until_rowid: i64,
    /// 覆盖终点消息的首现事件 seq。旧事件无此字段 → None（`duration_ms` 同款先例）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covered_until_seq: Option<i64>,
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

/// 降级链换档（B2-S3，`stream_with_retry` 内三拦截点触发）。
///
/// 独立 kind（决策 9）：换档是**回合内模型身份变更**，不属于 turn_context 快照
/// 的追加修正；轨迹页按本事件还原「这轮回复实际由谁产出」。legacy 手动主档
/// 的 `from_profile_id` 为 None（快照列族完整性只对引用 agent 成立）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSwitchPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    /// 换出档位 profile id（None = legacy 手动主档，无实体可指）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_profile_id: Option<String>,
    /// 换出模型名（换档前的运行时 model）
    pub from_model: String,
    pub to_profile_id: String,
    /// 换入档位别名（用户起的名，轨迹展示用）
    pub to_alias: String,
    pub to_model: String,
    /// 触发原因 slug：quota（余额/资源包）/ rate_limited（限流退避耗尽）/
    /// network（网络错误退避耗尽）
    pub reason: String,
    /// 本回合第几次换档（1 起——链尽与多次换档的轨迹可分辨）
    pub attempt: u32,
    /// 触发换档的错误原文（截断；换档路径不落 message_error，这里留诊断线）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// MA-3 跨会话来件（投递事实）。pending 语义 = 有本事件无同 message_id 的
/// settled（`repo::session_event::list_pending_inbox`）。
///
/// **不属于任何回合**：turn_id 用独立值 `cross:{message_id}`（append_event
/// 签名恒填 ctx.turn_id，EventCtx 由投递方如此构造）。消费时另有 user_message
/// 事件（run_agent_turn 全链路照常），本 kind 不产消息行——derive skip。
///
/// **中继不带用户权威**（CC 经验不变式）：actor = `agent:<源agent_id>`
/// 诚实归因；标题/agent 名打快照（源会话/agent 删除后收件箱展示不空）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossSessionMessagePayload {
    #[serde(default = "version_one")]
    pub v: u8,
    /// 投递幂等/关联键（settled 事件同值关联）
    pub message_id: String,
    pub source_conversation_id: String,
    /// 源会话标题快照
    pub source_conversation_title: String,
    pub source_agent_id: String,
    /// 源 agent 名快照
    pub source_agent_name: String,
    /// 消息原文（工具入口限长截断）
    pub content: String,
    /// true = 消费回合结束自动把目标 agent 回复回投源会话（链一次止，
    /// 回投消息本字段恒 false）
    pub expect_reply: bool,
    pub delivered_at_unix: u64,
}

/// MA-3 来件终态。`action = consumed`（已被消费回合物化进消息流，或已
/// 批准占位待消费）| `refused`（用户拒收）。
///
/// settle 动作本质都在用户权限域（批准/拒绝是用户手势；auto 是用户 accept
/// 政策的预授权执行）→ actor 恒 user，细分来源进 `by`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossSessionMessageSettledPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    pub message_id: String,
    /// "consumed" | "refused"
    pub action: String,
    /// "user-approval" | "auto" | "user-refused"
    pub by: String,
}

/// 上下文组成清单（③ 可观测化）——一条事件装整回合，构建期快照落库
/// （turn_context 同类：非消息行事实，derive skip）。
///
/// `segments` 是 **Model-visible 口径**：Memory 折叠 / TokenWindow 裁剪 /
/// ModalCapability 剥图之后的终值（context/anatomy.rs 事后聚合）。est 全部
/// 是本地估算（CJK 1/字、其余 ÷4，JSON 偏低估）——与 `actual_prompt_tokens`
/// 的偏差是已知估算债，展示侧必须披露，勿当 provider 计数用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBreakdownPayload {
    #[serde(default = "version_one")]
    pub v: u8,
    pub segments: Vec<ContextBreakdownSegment>,
    /// 各段估算之和
    pub est_total: u64,
    /// provider 回传的回合**首轮** prompt_tokens 真值（None = 无 usage /
    /// 中途失败；后续轮 prompt 单看 `rounds`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_prompt_tokens: Option<u64>,
    pub fingerprint: ContextBreakdownFingerprint,
    /// 逐轮请求序列（每个 LLM 请求一条）
    pub rounds: Vec<ContextBreakdownRound>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBreakdownSegment {
    /// 段 label（词表见 context::anatomy：system_* 五段 / tool_defs / summary /
    /// history / user_message / user_images）
    pub label: String,
    pub est: u64,
    /// 可选计数（工具数/历史消息数/图片数）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
}

/// 请求体稳定指纹三元组（各 12 hex，anatomy::fnv1a_12hex）。跨回合 miss
/// 归因的比对基线：下回合轮 0 与本条比对，谁变了归谁。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBreakdownFingerprint {
    /// 工具列表（轮 0 出口序；逐轮子集抖动看 rounds[i].tools_hash）
    pub tools: String,
    /// system 稳定段（persona+tool_hint+delegation+word_style，不含 os）
    pub system_stable: String,
    /// os_context 稳定核（时间行冻结 EPOCH 后——工作目录/时区/project.md 变才变）
    pub os_stable: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBreakdownRound {
    /// 本轮请求 prompt_tokens 真值（provider 回传 usage）
    pub prompt: u64,
    /// 其中缓存命中部分（0 = 全 miss）
    pub cached: u64,
    /// 本轮实际发送的工具列表指纹（12 hex）
    pub tools_hash: String,
    /// 本轮请求前是否有注入（BeforeLlm 钩子 / 预算提醒——前缀外变更源）
    pub injected: bool,
    /// 本轮是否发生过降级链换档（缓存命名空间切换）
    pub model_switched: bool,
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

/// 上下文组成清单（actor=agent，message_id=None：回合级事实不挂单条消息）。
/// emit 单点在 stream_loop wrapper（loop_engine）——19 个 finalize 调用点的
/// 公共唯一下游，零签名扰动；turn_ended 已在 inner finalize 内先落库。
pub async fn log_context_breakdown(
    pool: &SqlitePool,
    ctx: &EventCtx,
    payload: &ContextBreakdownPayload,
) {
    append_event(
        pool,
        ctx,
        kind::CONTEXT_BREAKDOWN,
        &ctx.agent_actor(),
        None,
        payload,
    )
    .await;
}

/// 用户消息落库原文（actor=user）。字段式签名（Image 治理：`blocks` 的
/// ref 化下沉 emitter 内部，调用方只管传与落库同值的 blocks）。
pub async fn log_user_message(
    pool: &SqlitePool,
    ctx: &EventCtx,
    message_id: &str,
    content: &str,
    blocks: &[ContentBlock],
) {
    let payload = UserMessagePayload {
        v: version_two(),
        content: content.to_string(),
        blocks: refify_blocks(message_id, blocks),
    };
    append_event(
        pool,
        ctx,
        kind::USER_MESSAGE,
        actor_user(),
        Some(message_id),
        &payload,
    )
    .await;
}

/// assistant 权威快照（actor=agent；supersede：同 message_id 多条 last-wins）。
// 字段式：Image 治理的 ref 化在 emitter 内部做（调用方传与落库同值的 blocks），
// 10 参数镜像 AssistantMessagePayload 语义字段（log_tool_execution 同款先例）。
#[allow(clippy::too_many_arguments)]
pub async fn log_assistant_message(
    pool: &SqlitePool,
    ctx: &EventCtx,
    message_id: &str,
    model: Option<&str>,
    content: &str,
    blocks: &[ContentBlock],
    token_count: Option<i64>,
    duration_ms: Option<u64>,
    round: u32,
    continuation: bool,
) {
    let payload = AssistantMessagePayload {
        v: version_two(),
        model: model.map(str::to_string),
        content: content.to_string(),
        blocks: refify_blocks(message_id, blocks),
        token_count,
        duration_ms,
        round,
        continuation,
    };
    append_event(
        pool,
        ctx,
        kind::ASSISTANT_MESSAGE,
        &ctx.agent_actor(),
        Some(message_id),
        &payload,
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

/// 工具结果消息镜像。`blocks` 须与刚落库行 content_blocks 同值（Image 治理）。
pub async fn log_tool_result_message(
    pool: &SqlitePool,
    ctx: &EventCtx,
    message_id: &str,
    blocks: &[ContentBlock],
) {
    let payload = ToolResultMessagePayload {
        v: version_two(),
        blocks: refify_blocks(message_id, blocks),
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
/// `boot_at`（UTC，`datetime('now')` 同格式）：只补记本进程启动前遗留的
/// turn（`find_open_turns` 的时间上界）——本函数已后台化（2026-09-07 白屏
/// 根治批），与「用户立刻发消息开新 turn」并发时靠它结构排除误杀。
///
/// 返回补记条数（0 = 干净启动，零写入）。查询失败仅 warn 不阻断启动——
/// 与 heal_checksum_drift / fix_orphan_tool_results 同款 boot 自愈定位。
pub async fn sweep_interrupted_turns(pool: &SqlitePool, boot_at: &str) -> usize {
    let open = match repo::session_event::find_open_turns(pool, boot_at).await {
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

/// 降级链换档成功（message_id=None：换档发生在轮内流式起点，关联当前轮
/// assistant_message 即可，无需独立占位）。
pub async fn log_model_switch(pool: &SqlitePool, ctx: &EventCtx, payload: &ModelSwitchPayload) {
    append_event(
        pool,
        ctx,
        kind::MODEL_SWITCH,
        &ctx.agent_actor(),
        None,
        payload,
    )
    .await;
}

/// MA-3 跨会话来件投递（actor = 源 agent，诚实归因；ctx 由投递方以
/// `turn_id = cross:{message_id}` 构造——不属于任何回合）。
pub async fn log_cross_session_message(
    pool: &SqlitePool,
    ctx: &EventCtx,
    payload: &CrossSessionMessagePayload,
) {
    append_event(
        pool,
        ctx,
        kind::CROSS_SESSION_MESSAGE,
        &ctx.agent_actor(),
        Some(&payload.message_id),
        payload,
    )
    .await;
}

/// MA-3 来件终态（actor 恒 user——批准/拒绝是用户权限域，auto 是 accept
/// 政策的预授权执行，细分来源在 payload.by）。
pub async fn log_cross_session_message_settled(
    pool: &SqlitePool,
    ctx: &EventCtx,
    payload: &CrossSessionMessageSettledPayload,
) {
    append_event(
        pool,
        ctx,
        kind::CROSS_SESSION_MESSAGE_SETTLED,
        actor_user(),
        Some(&payload.message_id),
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

        let blocks = vec![
            ContentBlock::Text {
                text: "看这张图".into(),
            },
            ContentBlock::Attachment {
                name: "plan.pdf".into(),
                kind: "application/pdf".into(),
                size: 282_000,
            },
        ];
        log_user_message(&pool, &ctx(), "msg-u1", "看这张图", &blocks).await;

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
        assert!(matches!(
            back.blocks[1],
            PayloadBlock::Full(ContentBlock::Attachment { .. })
        ));
    }

    #[tokio::test]
    async fn assistant_message_round_trip_and_supersede() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool).await;

        let mk_blocks = |content: &str| {
            vec![ContentBlock::Text {
                text: content.into(),
            }]
        };
        // 自动续写：同 message_id 两条（supersede last-wins）
        log_assistant_message(
            &pool,
            &ctx(),
            "msg-a1",
            Some("glm-5.2"),
            "前半段",
            &mk_blocks("前半段"),
            Some(42),
            Some(3_500),
            0,
            false,
        )
        .await;
        log_assistant_message(
            &pool,
            &ctx(),
            "msg-a1",
            Some("glm-5.2"),
            "前半段后半段",
            &mk_blocks("前半段后半段"),
            Some(42),
            Some(3_500),
            1,
            true,
        )
        .await;

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

    /// refify_blocks：Image 换轻量引用（payload 无 base64），其余原样 Full；
    /// 序列化 wire 形态可被 untagged 往返。
    #[test]
    fn refify_replaces_images_keeps_others() {
        let blocks = vec![
            ContentBlock::text("看图"),
            ContentBlock::Image {
                data: "QUJD".into(),
                media_type: "image/png".into(),
            },
            ContentBlock::ToolUse {
                id: "tu1".into(),
                name: "read_file".into(),
                input: "{}".into(),
            },
        ];
        let payload_blocks = refify_blocks("msg-u1", &blocks);

        // Image → ref（指向下标 1），其余 Full
        assert!(matches!(
            payload_blocks[0],
            PayloadBlock::Full(ContentBlock::Text { .. })
        ));
        match &payload_blocks[1] {
            PayloadBlock::ImageRef {
                marker,
                message_id,
                block_index,
            } => {
                assert_eq!(marker, "image_ref");
                assert_eq!(message_id, "msg-u1");
                assert_eq!(*block_index, 1, "索引 = 数组下标（行布局对齐）");
            }
            other => panic!("expected ImageRef, got {other:?}"),
        }
        assert!(matches!(
            payload_blocks[2],
            PayloadBlock::Full(ContentBlock::ToolUse { .. })
        ));

        // wire 序列化：无 base64 + untagged 往返还原
        let wire = serde_json::to_string(&payload_blocks).unwrap();
        assert!(!wire.contains("QUJD"), "payload 不应含图片字节: {wire}");
        assert!(wire.contains("image_ref"));
        let back: Vec<PayloadBlock> = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, payload_blocks);

        // v1 内联事件（Image 原样）→ 反序列化落 Full，读侧零迁移
        let v1 = r#"[{"type":"text","text":"q"},{"type":"image","data":"QUJD","media_type":"image/png"}]"#;
        let legacy: Vec<PayloadBlock> = serde_json::from_str(v1).unwrap();
        assert!(matches!(
            legacy[1],
            PayloadBlock::Full(ContentBlock::Image { .. })
        ));
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

        // 扫尾：只补 conv-1 的未闭合 turn（before 传远未来 = 全量命中，等价旧行为）
        assert_eq!(
            sweep_interrupted_turns(&pool, "9999-12-31 23:59:59").await,
            1
        );
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
        assert_eq!(
            sweep_interrupted_turns(&pool, "9999-12-31 23:59:59").await,
            0
        );
        assert_eq!(
            session_event::list_by_session(&pool, "conv-1", None)
                .await
                .unwrap()
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn sweep_respects_boot_at_boundary() {
        // sweep 后台化后的进程边界：只补记 boot_at 之前遗留的未闭合 turn，
        // 结构上排除误杀本进程刚开的新 turn（显式 created_at 造早/晚两条）
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool).await;
        // 上一进程遗留（早于 boot_at）
        sqlx::query(
            "INSERT INTO session_events (session_id, seq, kind, actor, turn_id, payload, created_at)
             VALUES ('conv-1', 1, 'turn_context', 'agent:agent-1', 'turn-old', '{}', '2026-01-01 00:00:00')",
        )
        .execute(&pool)
        .await
        .unwrap();
        // 本进程刚开的新 turn（晚于 boot_at，进行中）
        sqlx::query(
            "INSERT INTO session_events (session_id, seq, kind, actor, turn_id, payload, created_at)
             VALUES ('conv-1', 2, 'turn_context', 'agent:agent-1', 'turn-new', '{}', '2026-06-01 00:00:00')",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            sweep_interrupted_turns(&pool, "2026-03-01 00:00:00").await,
            1
        );
        // turn-old 补记 closed；turn-new 保持 open（无 turn_ended）
        let rows = session_event::list_by_session(&pool, "conv-1", None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 3, "只补记 turn-old 一条");
        assert_eq!(rows[2].turn_id.as_deref(), Some("turn-old"));
        assert_eq!(rows[2].kind, kind::TURN_ENDED);
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
    async fn context_breakdown_round_trip() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool).await;

        log_context_breakdown(
            &pool,
            &ctx(),
            &ContextBreakdownPayload {
                v: 1,
                segments: vec![
                    ContextBreakdownSegment {
                        label: "system_persona".into(),
                        est: 120,
                        count: None,
                    },
                    ContextBreakdownSegment {
                        label: "tool_defs".into(),
                        est: 3_400,
                        count: Some(18),
                    },
                ],
                est_total: 3_520,
                actual_prompt_tokens: Some(4_096),
                fingerprint: ContextBreakdownFingerprint {
                    tools: "aaaabbbbcccc".into(),
                    system_stable: "ddddddddeeee".into(),
                    os_stable: "ffff00001111".into(),
                },
                rounds: vec![
                    ContextBreakdownRound {
                        prompt: 4_096,
                        cached: 0,
                        tools_hash: "aaaabbbbcccc".into(),
                        injected: false,
                        model_switched: false,
                    },
                    ContextBreakdownRound {
                        prompt: 6_100,
                        cached: 4_096,
                        tools_hash: "aaaabbbbcccd".into(),
                        injected: true,
                        model_switched: true,
                    },
                ],
            },
        )
        .await;

        let back: ContextBreakdownPayload = sole_event_payload(&pool).await;
        assert_eq!(back.v, 1);
        assert_eq!(back.segments.len(), 2);
        assert_eq!(back.segments[1].count, Some(18));
        assert_eq!(back.est_total, 3_520);
        assert_eq!(back.actual_prompt_tokens, Some(4_096));
        assert_eq!(back.fingerprint.tools, "aaaabbbbcccc");
        assert_eq!(back.rounds.len(), 2);
        assert_eq!(back.rounds[1].cached, 4_096);
        assert!(back.rounds[1].model_switched);

        // 旧 / 局部 JSON：缺 v、actual_prompt_tokens、count 仍可解析（可选字段
        // serde default —— 前端旧轮次与降级路径不炸）
        let minimal: ContextBreakdownPayload = serde_json::from_str(
            r#"{"segments":[{"label":"history","est":900}],"est_total":900,
                "fingerprint":{"tools":"a","system_stable":"b","os_stable":"c"},
                "rounds":[{"prompt":900,"cached":900,"tools_hash":"a","injected":false,"model_switched":false}]}"#,
        )
        .unwrap();
        assert_eq!(minimal.v, 1, "缺 v 默认 1");
        assert_eq!(minimal.actual_prompt_tokens, None);
        assert_eq!(minimal.segments[0].count, None);
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
                covered_until_seq: Some(77),
            },
        )
        .await;
        let back: SummaryPayload = sole_event_payload(&pool).await;
        assert_eq!(back.covered_until_rowid, 77);
    }

    /// MA-3 来件族 round-trip：投递（actor=源 agent、turn 独立、message_id 关联）
    /// + 终态（actor=user）。
    #[tokio::test]
    async fn cross_session_message_round_trip() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool).await;

        // 投递：ctx 按投递方契约构造（turn = cross:{message_id}，agent = 源 agent）
        let relay_ctx = EventCtx::new("conv-1", "cross:xm-1", "agent-src");
        log_cross_session_message(
            &pool,
            &relay_ctx,
            &CrossSessionMessagePayload {
                v: 1,
                message_id: "xm-1".into(),
                source_conversation_id: "conv-src".into(),
                source_conversation_title: "源会话".into(),
                source_agent_id: "agent-src".into(),
                source_agent_name: "源 agent".into(),
                content: "材质参数定稿了吗".into(),
                expect_reply: true,
                delivered_at_unix: 1_750_000_000,
            },
        )
        .await;
        let rows = session_event::list_by_session(&pool, "conv-1", None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "cross_session_message");
        assert_eq!(rows[0].actor, "agent:agent-src", "中继不带用户权威：源 agent 归因");
        assert_eq!(rows[0].turn_id.as_deref(), Some("cross:xm-1"));
        assert_eq!(rows[0].message_id.as_deref(), Some("xm-1"));
        let back: CrossSessionMessagePayload = serde_json::from_str(&rows[0].payload).unwrap();
        assert_eq!(back.source_conversation_title, "源会话");
        assert!(back.expect_reply);

        // 终态：actor 恒 user，message_id 关联
        log_cross_session_message_settled(
            &pool,
            &relay_ctx,
            &CrossSessionMessageSettledPayload {
                v: 1,
                message_id: "xm-1".into(),
                action: "refused".into(),
                by: "user-refused".into(),
            },
        )
        .await;
        let rows = session_event::list_by_session(&pool, "conv-1", None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].kind, "cross_session_message_settled");
        assert_eq!(rows[1].actor, "user");
        assert_eq!(rows[1].message_id.as_deref(), Some("xm-1"));
        let back: CrossSessionMessageSettledPayload =
            serde_json::from_str(&rows[1].payload).unwrap();
        assert_eq!(back.action, "refused");
    }
}
