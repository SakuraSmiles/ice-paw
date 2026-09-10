//! MA-3 跨会话通讯引擎——异步对等通道（与委派互补不替代）。
//!
//! 委派（`mcp::delegate`）是父→子**同步阻塞**形态；本模块提供会话 A 的
//! agent 向会话 B 的**异步投递**：来件 append `cross_session_message` 事件
//! 排队（不写 messages 表——与目标会话在途回合零冲突），目标空闲时消费 =
//! 复用 [`session_runner::run_agent_turn`] 跑一回合，把来件物化为带来源
//! 标注的 user 消息（derive 零改动）。
//!
//! ## 收件三态（conversations.inbox_policy，migration 52）
//!
//! - `accept`（默认）——排队、目标空闲自动消费。默认值是 2026-09-10 测试
//!   反馈后拍板：单用户应用所有会话同属一人，「在 A 发起、切到 B 批准」的
//!   行为流反人类；防 agent 互扰由下方护栏（队列/配额）兜底，hold 降级为
//!   显式治理档（想逐件过目时切）；
//! - `hold`——来件扣住待用户批准（收件箱放行）；
//! - `refuse`——拒收，投递方工具立即报错（源 agent 可感知）。
//!   **is_reply 例外**：expect_reply 回投件不受 hold 扣（accept 排队消费）——
//!   回投落的是源会话（用户自己发起的对话回路），再扣一次等于打断自己；
//!   refuse 仍拦（用户治理权最大）。
//!
//! ## 项目边界（硬边界，2026-09-10 拍板）
//!
//! 源与目标会话必须挂**同一项目**（project_id 同值且皆非 NULL）——散落
//! 会话（未挂项目）双向不可投。执行期校验（deliver 内 Err 指路「把会话挂
//! 到同一项目」），非注册期隐藏工具：agent 能通过 `list_conversations`
//! 看到全部会话的概述元信息（可见性），投递层拦权限——知道 ≠ 能发。
//!
//! ## 触发三源
//!
//! 1. 投递时目标空闲且可自动消费（accept / is_reply）→ 立即 spawn 消费
//!    （[`deliver`]）；
//! 2. accept 会话回合结束 → [`spawn_drain_watcher`]（EVENT_BUS 订阅
//!    turn_ended）排空检查，队列非空且配额未尽 → 下一条——消费回合自身
//!    结束再广播 turn_ended，链式排空到队列空或配额尽；
//! 3. hold 会话用户在收件箱批准 → `respond_inbox_item` 命令（by=
//!    user-approval，不占自动配额）。
//!
//! ## 护栏（防来件风暴 / 回合链失控）
//!
//! - pending 队列上限 [`MAX_PENDING_INBOX`]（超限投递 Err 拒收）；
//! - 同会话 10 分钟窗口自动消费上限 [`AUTO_CONSUME_MAX`]（超限留队待手动
//!   放行——用户批准不占配额）；内存窗口，重启清零可接受（护栏非计费）；
//! - 投递只入 kind='chat' 会话（delegation 子会话拒——防子会话侧信道绕过
//!   委派深度护栏；工具注册同款按 kind 判定）；
//! - 回投（expect_reply=true）恒 expect_reply=false——链一次止。
//!
//! ## 中继不带用户权威（CC 经验不变式）
//!
//! 来件事件 actor = `agent:<源agent_id>` 诚实归因；消费回合物化的 user
//! 消息是**双块结构**（[`compose_incoming_blocks`]：来源标注块 + 正文块，
//! 系统组装非 agent 手写）+ `incoming_source` 元数据（messages 列 migration
//! 53 + user_message 事件 payload，前端 incoming 卡的权威数据源）——目标
//! agent 与用户都能看到「这条消息来自谁」，不冒充用户权威。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::commands::agent_cmd::AgentCmd;
use crate::db::models::{ConversationRow, SessionEventRow};
use crate::db::repo::{self, session_event};
use crate::error::{AppError, AppResult};
use crate::harness::chat_state::ChatState;
use crate::harness::provider;
use crate::harness::event_log::{
    self, CrossSessionMessagePayload, CrossSessionMessageSettledPayload, EventCtx,
};
use crate::harness::session_runner::{self, AgentTurnInput, TurnEnv};
use crate::infra::protocol::ContentBlock;

/// pending 队列上限：超限投递 Err 拒收（防来件风暴压垮收件会话）。
pub const MAX_PENDING_INBOX: usize = 10;
/// 单条来件 content 上限（字符）：超限 Err 拒收不截断（诚实边界，
/// 截断的半句话比拒绝更危险）。
pub const MAX_CONTENT_CHARS: usize = 8_000;
/// 自动消费配额窗口与上限：同会话窗口内自动消费超过 N 次 → 留队待手动放行。
const AUTO_WINDOW: Duration = Duration::from_secs(600);
const AUTO_CONSUME_MAX: usize = 6;
/// 回合结束排空前等会话静默的轮询节奏与上限（turn_ended 先于 cleanup
/// unregister——广播到达时注册可能还在；正常 cleanup 在广播后数百 ms 内完成）。
const DRAIN_QUIET_POLL: Duration = Duration::from_secs(2);
const DRAIN_QUIET_TIMEOUT: Duration = Duration::from_secs(30);

// =========================================================================
// 纯函数（前端 incoming 卡同款锚；测试不依赖 DB）
// =========================================================================

/// 消费回合物化消息的来源前缀检测锚（legacy 兜底路径：前端 `[来自会话「`
/// 开头即 incoming 卡。权威路径是 incoming_source 元数据，前缀只是 LLM
/// 视角投影 + 旧消息兼容）。
pub const INCOMING_PREFIX_HEAD: &str = "[来自会话「";

/// 组装来源标注块（独立 Text 块）：标注「这条消息从哪来、怎么回」——目标
/// agent 需要它回信（target id），用户侧 UI 不再解析它（元数据接管）。
pub fn compose_incoming_annotation(
    source_title: &str,
    agent_name: &str,
    source_conv_id: &str,
) -> String {
    format!(
        "[来自会话「{source_title}」的 agent {agent_name}｜如需回复用 send_message_to_session 工具，target={source_conv_id}]"
    )
}

/// 组装消费回合的 user 消息**双块结构**：[来源标注块, 正文块]（系统组装，
/// agent 不经手格式——「通过系统工具规范」的落点）。
///
/// LLM 视角两块文本自然可读；UI 侧正文块即气泡体（不再从全文剥前缀）。
/// 旧全文形态（标注\n\n正文 单串）仍由 [`compose_incoming_text`] 产出，
/// 用于 messages.content 扁平快照（检索/回退显示）。
pub fn compose_incoming_blocks(
    source_title: &str,
    agent_name: &str,
    source_conv_id: &str,
    content: &str,
) -> Vec<ContentBlock> {
    vec![
        ContentBlock::text(compose_incoming_annotation(
            source_title,
            agent_name,
            source_conv_id,
        )),
        ContentBlock::text(content.to_string()),
    ]
}

/// 组装消费回合的 user 消息全文（扁平快照）：来源标注头 + 原文。
pub fn compose_incoming_text(
    source_title: &str,
    agent_name: &str,
    source_conv_id: &str,
    content: &str,
) -> String {
    format!(
        "{}\n\n{content}",
        compose_incoming_annotation(source_title, agent_name, source_conv_id)
    )
}

// =========================================================================
// 自动消费配额（内存窗口；临界区无 await）
// =========================================================================

static AUTO_QUOTA: OnceLock<Mutex<HashMap<String, VecDeque<Instant>>>> = OnceLock::new();

fn quota_map() -> &'static Mutex<HashMap<String, VecDeque<Instant>>> {
    AUTO_QUOTA.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 检查并登记一次自动消费：窗口内已满 → false（留队待手动放行）。
/// `now` 由调用方传入（测试可控）；用户批准路径不调用本函数（不占配额）。
fn auto_consume_reserve(conv_id: &str, now: Instant) -> bool {
    let mut map = quota_map().lock().unwrap();
    let q = map.entry(conv_id.to_string()).or_default();
    let cutoff = now - AUTO_WINDOW;
    while q.front().is_some_and(|t| *t < cutoff) {
        q.pop_front();
    }
    if q.len() >= AUTO_CONSUME_MAX {
        return false;
    }
    q.push_back(now);
    true
}

// =========================================================================
// 投递（工具 / expect_reply 回投 / 未来用户直投共用）
// =========================================================================

/// 投递方身份快照（事件 payload 与回投都用）。
pub struct SourceInfo {
    pub conv_id: String,
    pub conv_title: String,
    pub agent_id: String,
    pub agent_name: String,
    /// 源会话所属项目（项目边界校验用；散落会话 None——散落双向不可投）
    pub project_id: Option<String>,
}

/// 项目边界校验（纯函数）：源与目标必须挂同一项目（同值且皆非 NULL）。
/// 违规返回三段式文案（Err 体），合规返回 None。
pub fn project_boundary_error(
    source_project: Option<&str>,
    target: &ConversationRow,
) -> Option<String> {
    match (source_project, target.project_id.as_deref()) {
        (Some(s), Some(t)) if s == t => None,
        _ => Some(format!(
            "目标会话「{}」与当前会话不在同一项目——跨会话投递仅支持同项目会话{}。\
             请让用户把两个会话挂到同一项目，或改投同项目的其他会话",
            target.title,
            if target.project_id.is_none() && source_project.is_some() {
                "（目标未挂项目）"
            } else if source_project.is_none() {
                "（当前会话未挂项目）"
            } else {
                ""
            }
        )),
    }
}

/// 投递结果（工具 JSON 化回源 agent）。
pub struct DeliveryOutcome {
    /// "delivered"（accept 且已触发消费）/ "queued"（accept 但目标忙，回合
    /// 结束自动排空）/ "held"（hold 扣住待批准）
    pub status: &'static str,
    pub target_title: String,
    pub queue_position: usize,
}

/// 投递一条跨会话消息：校验（项目边界 / refuse / 队列上限）→ append
/// `cross_session_message`（pending 入队）→ 可自动消费（accept 或 is_reply
/// 例外）且空闲则立即触发消费。refuse 在 append 前拦截（投递失败不产生
/// 事实）。
///
/// `is_reply` = expect_reply 回投件（源会话用户自己发起的回路）——不受
/// hold 扣（见模块文档「收件三态」），refuse 仍拦。
#[allow(clippy::too_many_arguments)]
pub async fn deliver(
    app: &AppHandle,
    pool: &SqlitePool,
    source: &SourceInfo,
    target_conv_id: &str,
    content: &str,
    expect_reply: bool,
    is_reply: bool,
) -> AppResult<DeliveryOutcome> {
    let content = content.trim();
    if content.is_empty() {
        return Err(AppError::Validation(
            "send_message_to_session 消息内容为空——请写下实际要转达的内容".into(),
        ));
    }
    if content.chars().count() > MAX_CONTENT_CHARS {
        return Err(AppError::Validation(format!(
            "send_message_to_session 消息内容超长（{} 字符 > 上限 {MAX_CONTENT_CHARS}）——请精简后重发",
            content.chars().count()
        )));
    }

    let target = match repo::conversation::get_by_id(pool, target_conv_id).await {
        Ok(c) => c,
        Err(AppError::NotFound { .. }) => {
            return Err(AppError::Validation(format!(
                "目标会话不存在（{target_conv_id}）——可能已被删除；请确认会话 id 或标题后重试"
            )))
        }
        Err(e) => return Err(e),
    };
    if target.id == source.conv_id {
        return Err(AppError::Validation(
            "不能向本会话投递跨会话消息——需要回复对方时直接输出即可".into(),
        ));
    }
    if target.kind != "chat" {
        return Err(AppError::Validation(format!(
            "目标会话「{}」不是普通会话（kind={}）——委派任务会话不是跨会话通讯单位，请改投其父会话",
            target.title, target.kind
        )));
    }
    if let Some(msg) = project_boundary_error(source.project_id.as_deref(), &target) {
        return Err(AppError::Validation(msg));
    }
    if target.inbox_policy == "refuse" {
        return Err(AppError::Validation(format!(
            "目标会话「{}」已设置为拒收跨会话消息——请如实告知用户，或改用其他会话",
            target.title
        )));
    }

    let pending = session_event::list_pending_inbox(pool, &target.id).await?;
    if pending.len() >= MAX_PENDING_INBOX {
        return Err(AppError::Validation(format!(
            "目标会话「{}」待处理来件已达上限（{MAX_PENDING_INBOX} 条）——对方收件箱积压，请稍后再试",
            target.title
        )));
    }

    // 入队（append-only；EVENT_BUS 广播驱动前端 badge/通知）
    let message_id = Uuid::new_v4().to_string();
    let ctx = EventCtx::new(&target.id, &format!("cross:{message_id}"), &source.agent_id);
    event_log::log_cross_session_message(
        pool,
        &ctx,
        &CrossSessionMessagePayload {
            v: 1,
            message_id: message_id.clone(),
            source_conversation_id: source.conv_id.clone(),
            source_conversation_title: source.conv_title.clone(),
            source_agent_id: source.agent_id.clone(),
            source_agent_name: source.agent_name.clone(),
            content: content.to_string(),
            expect_reply,
            is_reply,
            delivered_at_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        },
    )
    .await;

    let queue_position = pending.len() + 1;
    tracing::info!(
        target: "ice_paw.inbox",
        from_conv = %source.conv_id,
        to_conv = %target.id,
        policy = %target.inbox_policy,
        expect_reply,
        is_reply,
        "跨会话消息入队"
    );

    // 可自动消费：accept，或 is_reply 例外（回投不受 hold 扣）
    if target.inbox_policy == "accept" || (is_reply && target.inbox_policy == "hold") {
        let chat_state = app.state::<ChatState>().inner().clone();
        if !chat_state.is_streaming(&target.id) {
            // 立即消费（后台 spawn；配额在 consume_pending 内部检查，
            // 配额尽时消息留队由 turn_ended watcher / 用户接手）
            spawn_consume_one(app.clone(), pool.clone(), target.clone(), "auto");
            return Ok(DeliveryOutcome {
                status: "delivered",
                target_title: target.title,
                queue_position,
            });
        }
        return Ok(DeliveryOutcome {
            status: "queued",
            target_title: target.title,
            queue_position,
        });
    }
    // hold（默认）及未知值：保守扣住
    Ok(DeliveryOutcome {
        status: "held",
        target_title: target.title,
        queue_position,
    })
}

// =========================================================================
// 消费（pending → 目标会话一回合）
// =========================================================================

/// 后台消费一条 pending（fire-and-forget；错误只记日志——消息已入队，
/// 下次触发源会重试同一状态机的检查）。
fn spawn_consume_one(app: AppHandle, pool: SqlitePool, conv: ConversationRow, by: &'static str) {
    tauri::async_runtime::spawn(async move {
        let pending = match session_event::list_pending_inbox(&pool, &conv.id).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(target: "ice_paw.inbox", "收件箱读取失败 conv={} err={e}", conv.id);
                return;
            }
        };
        let Some(ev) = pending.first().cloned() else { return };
        let conv_id = conv.id.clone();
        match consume_pending(&app, &pool, conv, ev, by).await {
            Ok(true) => {}
            Ok(false) => tracing::info!(
                target: "ice_paw.inbox",
                conv = %conv_id,
                "消费未发起（配额尽或会话忙），消息留队"
            ),
            Err(e) => tracing::warn!(target: "ice_paw.inbox", conv = %conv_id, "消费发起失败: {e}"),
        }
    });
}

/// 消费一条 pending：预检 → ChatState 仲裁 → settled 占位出队 →
/// run_agent_turn 全链路（预算/工具/hooks/事件照常）→ expect_reply 回投。
///
/// 返回 `Ok(true)` = 消费回合已发起；`Ok(false)` = 未发起且消息留队
/// （自动配额尽 / 会话忙——软跳过）；`Err` = 预检失败（agent 档案/凭据/
/// provider 不可读——消息同样留队，错误如实上抛给触发方）。
///
/// settled(consumed) 在 start 成功后、spawn 前 append（原子占位防双击双
/// 消费）；此后回合成败都算「已处理」——回合失败在会话内有 message_error
/// 事实，诚实可查。
pub async fn consume_pending(
    app: &AppHandle,
    pool: &SqlitePool,
    conv: ConversationRow,
    ev: SessionEventRow,
    by: &str,
) -> AppResult<bool> {
    if by == "auto" && !auto_consume_reserve(&conv.id, Instant::now()) {
        tracing::info!(
            target: "ice_paw.inbox",
            conv = %conv.id,
            "自动消费配额已尽（{AUTO_WINDOW:?} 内 {AUTO_CONSUME_MAX} 次），来件留队待手动放行"
        );
        return Ok(false);
    }

    let payload: CrossSessionMessagePayload = serde_json::from_str(&ev.payload).map_err(|e| {
        AppError::Internal(format!("来件 payload 解析失败（seq={}）: {e}", ev.seq))
    })?;

    // --- 预检：目标会话 agent 档案 + 凭据 + provider（失败不出队不留痕） ---
    let agent_cmd = app.state::<Arc<dyn AgentCmd>>().inner().clone();
    let creds = agent_cmd.get_with_credentials(&conv.agent_id).await.map_err(|e| {
        AppError::Internal(format!(
            "读取会话 agent（{}）配置/凭据失败: {e}——来件留在收件箱，修复 agent 配置后可再消费",
            conv.agent_id
        ))
    })?;
    let llm_provider = provider::create_provider(
        &creds.agent.provider,
        &creds.agent.model,
        creds.base_url.as_deref(),
        creds.agent.cache_prompt != 0,
    )
    .map_err(|e| {
        AppError::Internal(format!(
            "为会话 agent（{}，{}/{}) 创建 provider 失败: {e}",
            creds.agent.name, creds.agent.provider, creds.agent.model
        ))
    })?;

    // --- 单写者仲裁：start 失败 = 会话忙（用户刚发消息等）→ 不消费留队 ---
    let chat_state = app.state::<ChatState>().inner().clone();
    let Ok(cancel_token) = chat_state.start(&conv.id) else {
        return Ok(false);
    };
    let conv_id_guard = conv.id.clone();
    let cancel_guard = scopeguard::guard((), |_| chat_state.unregister(&conv_id_guard));

    // --- 出队占位（settled consumed；append-only，不回滚） ---
    let settle_ctx =
        EventCtx::new(&conv.id, &format!("cross:{}", payload.message_id), &payload.source_agent_id);
    event_log::log_cross_session_message_settled(
        pool,
        &settle_ctx,
        &CrossSessionMessageSettledPayload {
            v: 1,
            message_id: payload.message_id.clone(),
            action: "consumed".into(),
            by: by.into(),
        },
    )
    .await;

    // --- 消费回合：来件物化为双块结构的 user 消息，run_agent_turn 全链路 ---
    // blocks = [来源标注块, 正文块]（系统组装）；content 扁平快照走
    // compose_incoming_text（检索/回退显示兼容）；incoming_source 元数据
    // 是前端 incoming 卡的权威数据源（messages 列 + 事件 payload 双写）。
    let blocks = compose_incoming_blocks(
        &payload.source_conversation_title,
        &payload.source_agent_name,
        &payload.source_conversation_id,
        &payload.content,
    );
    let text = compose_incoming_text(
        &payload.source_conversation_title,
        &payload.source_agent_name,
        &payload.source_conversation_id,
        &payload.content,
    );
    let incoming_source = crate::harness::event_log::IncomingSourceMeta {
        source_conversation_id: payload.source_conversation_id.clone(),
        source_conversation_title: payload.source_conversation_title.clone(),
        source_agent_name: payload.source_agent_name.clone(),
    };
    let fallback =
        crate::commands::model_profile_cmd::production_fallback_plan(app, pool, &creds.agent);

    let done_rx = session_runner::run_agent_turn(
        &TurnEnv {
            emitter: crate::harness::r#loop::emitter::tauri_emitter(app.clone(), conv.id.clone()),
            tool_app: Some(app.clone()),
            pool: pool.clone(),
            route_registry: app.state::<crate::harness::read_route::ReadRouteRegistry>().inner(),
            chat_state: chat_state.clone(),
            global_registry: Arc::clone(app.state::<Arc<crate::harness::mcp::McpRegistry>>().inner()),
            mcp_manager: Arc::clone(
                app.state::<Arc<crate::harness::mcp::McpServerManager>>().inner(),
            ),
            auth_registry: app
                .state::<crate::harness::tool_executor::ToolAuthRegistry>()
                .inner()
                .clone(),
            auth_sessions: app
                .state::<crate::harness::authority::AuthSessionRegistry>()
                .inner()
                .clone(),
        },
        AgentTurnInput {
            conv: conv.clone(),
            agent: creds.agent.clone(),
            hooks: creds.hooks,
            word_style_profile: creds.word_style_profile,
            provider: llm_provider,
            api_key: creds.api_key,
            user_msg_id: Uuid::new_v4().to_string(),
            content_text: text.clone(),
            llm_blocks: blocks.clone(),
            persist_blocks: blocks,
            attach_db_inputs: Vec::new(),
            attach_file_inputs: Vec::new(),
            emit_user_blocks: false,
            incoming_source: Some(incoming_source),
            tools_enabled: true,
            model_override: None,
            cancel_token,
            // 消费回合 = 目标会话视角的正常回合（非委派）：agent 自己的链
            fallback,
        },
    )
    .await?;
    // spawn 成功：注销责任移交 loop 的 RAII 守卫
    scopeguard::ScopeGuard::into_inner(cancel_guard);

    tracing::info!(
        target: "ice_paw.inbox",
        conv = %conv.id,
        from_conv = %payload.source_conversation_id,
        by,
        expect_reply = payload.expect_reply,
        "跨会话消息消费回合已发起"
    );

    // --- expect_reply 回投：完成后把目标 agent 最终回复投回源会话 ---
    // 链一次止（回投恒 expect_reply=false）；is_reply=true——不受源会话 hold
    // 扣（用户自己发起的回路），refuse 仍拦。回复为空不投（诚实：没内容可回）。
    if payload.expect_reply {
        let app = app.clone();
        let pool = pool.clone();
        let source = SourceInfo {
            conv_id: conv.id.clone(),
            conv_title: conv.title.clone(),
            agent_id: conv.agent_id.clone(),
            agent_name: creds.agent.name.clone(),
            project_id: conv.project_id.clone(),
        };
        let reply_to = payload.source_conversation_id.clone();
        tauri::async_runtime::spawn(async move {
            match done_rx.await {
                Ok(summary) => {
                    let reply = summary.final_text.trim();
                    if reply.is_empty() {
                        tracing::info!(
                            target: "ice_paw.inbox",
                            to_conv = %reply_to,
                            "消费回合无正文，跳过 expect_reply 回投"
                        );
                        return;
                    }
                    if let Err(e) = deliver(&app, &pool, &source, &reply_to, reply, false, true).await {
                        tracing::warn!(target: "ice_paw.inbox", "expect_reply 回投失败: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!(target: "ice_paw.inbox", "消费回合异常退出，跳过回投: {e}");
                }
            }
        });
    }
    Ok(true)
}

// =========================================================================
// 回合结束自动排空（EVENT_BUS 订阅；零侵入 loop 退出路径）
// =========================================================================

/// 启动 turn_ended 排空观察者（lib.rs setup 调用一次）。
///
/// accept 会话的回合结束 → 等静默（turn_ended 先于 cleanup unregister）→
/// pending 非空且配额未尽 → 消费下一条（其回合结束再广播 → 链式排空）。
/// hold/refuse/委派子会话的 turn_ended 直接忽略（policy 检查）。
pub fn spawn_drain_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut rx = event_log::event_bus().subscribe();
        loop {
            match rx.recv().await {
                Ok(note) => {
                    if note.kind != event_log::kind::TURN_ENDED {
                        continue;
                    }
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        maybe_auto_consume(&app, &note.conversation_id).await;
                    });
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        target: "ice_paw.inbox",
                        "排空观察者丢帧（{n}），本次排空跳过——下次 turn_ended / 投递会再触发"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

async fn maybe_auto_consume(app: &AppHandle, conv_id: &str) {
    let pool = app.state::<SqlitePool>().inner().clone();
    let conv = match repo::conversation::get_by_id(&pool, conv_id).await {
        Ok(c) => c,
        Err(_) => return, // 会话已删（CASCADE 清事件），无事可做
    };
    if conv.kind != "chat" || conv.inbox_policy != "accept" {
        return;
    }

    // 等静默：turn_ended 广播先于 cleanup unregister，此刻会话可能仍在表
    let chat_state = app.state::<ChatState>().inner().clone();
    let deadline = Instant::now() + DRAIN_QUIET_TIMEOUT;
    while chat_state.is_streaming(conv_id) {
        if Instant::now() >= deadline {
            tracing::info!(
                target: "ice_paw.inbox",
                conv = conv_id,
                "排空等待静默超时（{DRAIN_QUIET_TIMEOUT:?}），本轮放弃——下次触发源会接手"
            );
            return;
        }
        tokio::time::sleep(DRAIN_QUIET_POLL).await;
    }

    let pending = match session_event::list_pending_inbox(&pool, conv_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(target: "ice_paw.inbox", "排空读取收件箱失败: {e}");
            return;
        }
    };
    if let Some(ev) = pending.first().cloned() {
        match consume_pending(app, &pool, conv, ev, "auto").await {
            Ok(_) => {}
            Err(e) => tracing::warn!(target: "ice_paw.inbox", "自动消费发起失败: {e}"),
        }
    }
}

// =========================================================================
// 单元测试（纯函数 + 配额窗口；async 全链路见 dev 手测）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_incoming_text_carries_prefix_and_reply_hint() {
        let text = compose_incoming_text("主控", "甲", "conv-src", "材质定稿了吗");
        assert!(
            text.starts_with(INCOMING_PREFIX_HEAD),
            "前端检测锚必须在开头: {text}"
        );
        assert!(text.contains("主控」的 agent 甲"), "来源标注: {text}");
        assert!(text.contains("target=conv-src"), "回信指引带源会话 id: {text}");
        assert!(text.ends_with("材质定稿了吗"));
        assert!(text.contains("\n\n"), "标注头与正文之间空行分隔");
    }

    #[test]
    fn compose_incoming_blocks_is_annotation_plus_body() {
        let blocks = compose_incoming_blocks("主控", "甲", "conv-src", "材质定稿了吗");
        assert_eq!(blocks.len(), 2, "双块结构：标注块 + 正文块");
        let ContentBlock::Text { text: head } = &blocks[0] else {
            panic!("首块必须是 Text");
        };
        assert!(head.starts_with(INCOMING_PREFIX_HEAD), "标注块带检测锚: {head}");
        assert!(head.ends_with(']'), "标注块自闭合（不含正文）: {head}");
        let ContentBlock::Text { text: body } = &blocks[1] else {
            panic!("次块必须是 Text");
        };
        assert_eq!(body, "材质定稿了吗", "正文块 = 原文，不混标注");
    }

    fn target_row(project: Option<&str>) -> ConversationRow {
        ConversationRow {
            id: "c-target".into(),
            agent_id: "agent-x".into(),
            title: "材质".into(),
            pinned: 0,
            created_at: String::new(),
            updated_at: String::new(),
            tools_override: None,
            project_id: project.map(str::to_string),
            kind: "chat".into(),
            initiator_type: None,
            initiator_agent_id: None,
            parent_conversation_id: None,
            inbox_policy: "accept".into(),
        }
    }

    #[test]
    fn project_boundary_same_project_passes() {
        assert!(project_boundary_error(Some("p-1"), &target_row(Some("p-1"))).is_none());
    }

    #[test]
    fn project_boundary_scattered_rejected_both_ways() {
        // 散落目标（源挂了项目）
        assert!(project_boundary_error(Some("p-1"), &target_row(None)).is_some());
        // 散落源（目标挂了项目）
        assert!(project_boundary_error(None, &target_row(Some("p-1"))).is_some());
        // 双散落也不可投（边界 = 必须同挂一个项目）
        assert!(project_boundary_error(None, &target_row(None)).is_some());
    }

    #[test]
    fn project_boundary_different_projects_rejected_with_hint() {
        let err = project_boundary_error(Some("p-1"), &target_row(Some("p-2"))).unwrap();
        assert!(err.contains("不在同一项目"), "文案指路项目边界: {err}");
        assert!(err.contains("材质"), "文案点名目标会话: {err}");
    }

    #[test]
    fn auto_consume_quota_window_evicts_and_caps() {
        let conv = "quota-test-conv";
        let t0 = Instant::now();
        // 清干净（单测隔离）
        quota_map().lock().unwrap().clear();

        // 窗口内 6 次全过
        for i in 0..AUTO_CONSUME_MAX {
            assert!(auto_consume_reserve(conv, t0 + Duration::from_secs(i as u64)));
        }
        // 第 7 次拒绝
        assert!(!auto_consume_reserve(conv, t0 + Duration::from_secs(10)));
        // 窗口滑过（最早的登记已出窗）→ 重新放行
        let later = t0 + AUTO_WINDOW + Duration::from_secs(5);
        assert!(auto_consume_reserve(conv, later));

        quota_map().lock().unwrap().clear();
    }
}
