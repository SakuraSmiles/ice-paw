//! Steer（1v1 生成中插话）引擎：自动打断在途回合 + turn_ended 后自动续跑。
//!
//! 机制 = 两个现成能力的接线（docs/steer-design.md §2）：
//! - **打断**半：`chat_cmd` 的 Steer 分支在会话在途时物化用户消息 B，再调
//!   [`ChatState::stop`]（复用 `CancellationToken::cancel()`——loop 在 yield 点
//!   轮询，到达即由 [`crate::harness::cleanup::finalize_cancel`] 对称收尾）；
//! - **续跑**半：本模块订阅 turn_ended 广播 → 等静默 → 查积压（DB 为真相源，
//!   复用频道 C8 的「用户消息无条件物化」）→ `run_agent_turn(pre_materialized=true)`
//!   开新回合 B。
//!
//! 与频道的分工：频道是**共址**共享流（一时刻一成员发言、@ 接力），Steer 是
//! **1v1 无状态 replay**（不引入世界状态/长期记忆）——已完成轮次已落库，新回合
//! 重读历史即得上下文连续性。无新事件 kind、不动 loop 核心、不动 append-only
//! 日志模型（见设计稿 §4/§13）。
//!
//! ## 积压边界（关键）
//!
//! 积压 = 最近一次 `turn_ended` 的 `turn_id`（即该回合的 `user_msg_id`，由
//! `turn_id == user_msg_id` 最深不变式保证）之后的真实用户消息锚点。steer 消息
//! B 在回合 A 在途时物化，rowid 必在 A 的用户消息之后，故被 `list_user_anchors_after`
//! 命中。连发 B1/B2/B3 各自成回合、串行完整执行（每次消费**最旧一条**，
//! `user_msg_id = anchor.message_id`——数据层不合并，见设计稿 §10.2/§11.1）。
//!
//! ## 并发
//!
//! chat_state.start 是最终仲裁（与频道同款哲学）：两路并发触发时一路 start 成功、
//! 另一路把消息留流静默退出，积压以 DB 为真相源自愈。已知残余：极窄竞速窗口下
//! 消息可能稍晚被消费（方向是「多干活」非丢消息，诚实可接受）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};

use crate::commands::agent_cmd::AgentCmd;
use crate::db::models::ConversationRow;
use crate::db::repo;
use crate::error::AppResult;
use crate::harness::chat_state::ChatState;
use crate::harness::event_log;
use crate::harness::provider;
use crate::harness::session_runner::{self, AgentTurnInput, TurnEnv};
use crate::infra::protocol::ContentBlock;

/// steer 触发 cancel 后、开新回合前的静默窗口（复用频道 `CHAIN_HEAD_QUIET`）：
/// 让连发到齐——B2/B3 在 B1 start 前已进积压，不再是「新发送」，不会去 cancel B1。
const CHAIN_HEAD_QUIET: Duration = Duration::from_secs(3);
/// 等会话静默的轮询节奏与上限（inbox drain / 频道同款：turn_ended 广播先于
/// cleanup unregister，正常 cleanup 在广播后数百 ms 内完成）。
const QUIET_POLL: Duration = Duration::from_secs(2);
const QUIET_TIMEOUT: Duration = Duration::from_secs(30);

// =========================================================================
// 事实简报（回合启动注入 llm_blocks，用完即弃不落库）
// =========================================================================

/// Steer 消费回合的事实简报（pre_materialized：用户侧已物化，llm_blocks 只装
/// 插话说明——与频道 `compose_channel_brief` 同款「事实简报」模式）。
///
/// 用户原话已在历史尾部（物化先行 + HistoryStage 全量读），简报只说明插话语境
/// 防双计。`count` = 本次积压条数（>1 时提示连发，按时间先后理解为递进指示）。
pub(crate) fn compose_steer_brief(count: usize) -> Vec<ContentBlock> {
    let text = if count > 1 {
        format!(
            "[插话] 你上一回合执行期间，用户发送了 {count} 条新消息（见上方最近的用户消息，按时间先后理解为递进指示）。上一回合已在工具边界停止，请据此调整方向，优先处理最近一条新消息。"
        )
    } else {
        "[插话] 你上一回合执行期间，用户发送了一条新消息（见上方最近的用户消息）。上一回合已在工具边界停止，请据此调整方向继续。".to_string()
    };
    vec![ContentBlock::text(text)]
}

// =========================================================================
// 回合结束触发点（EVENT_BUS 订阅 turn_ended；inbox drain / 频道 watcher 同款）
// =========================================================================

/// 启动 Steer 观察者（lib.rs setup 调用一次）。
///
/// 触发源 = 1v1（kind='chat'）回合的 `turn_ended` 广播：回合结束（自然完成与
/// 用户终止/steer 打断两态同接）→ 等静默 → 查积压 → 消费最旧一条开新回合。
/// 与频道/收件箱 watcher 同源广播、独立订阅，lagged 互不传染。
pub fn spawn_steer_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut rx = event_log::event_bus().subscribe();
        loop {
            match rx.recv().await {
                Ok(note) => {
                    if note.kind == event_log::kind::TURN_ENDED {
                        let app = app.clone();
                        let conv_id = note.conversation_id.clone();
                        tauri::async_runtime::spawn(async move {
                            on_steer_turn_ended(&app, &conv_id).await;
                        });
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        target: "ice_paw.steer",
                        "Steer 观察者丢帧（{n}），本次触发跳过——下次触发源会再触发"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// 1v1 回合结束处理：取会话 → 非 chat 返回 → 消费积压。
async fn on_steer_turn_ended(app: &AppHandle, conv_id: &str) {
    let pool = app.state::<SqlitePool>().inner().clone();
    let conv = match repo::conversation::get_by_id(&pool, conv_id).await {
        Ok(c) => c,
        Err(_) => return, // 会话已删（CASCADE 清事件），无事可做
    };
    if conv.kind != "chat" {
        return; // Steer 只对 1v1（频道/委派子会话有各自的触发语义）
    }
    if let Err(e) = consume_steer_backlog(app, &pool, &conv).await {
        tracing::warn!(target: "ice_paw.steer", conv = conv_id, "Steer 积压消费失败: {e}");
    }
}

/// 消费 1v1 积压：最近一次 turn_ended 之后的真实用户消息 → 静默窗 → 消费最旧一条。
///
/// 调用源：`on_steer_turn_ended`（turn_ended 广播）与 `chat_cmd` Steer 分支的
/// stop-false 兜底（会话恰在物化期间收尾 → 无新 turn_ended 再触发，手动接管）。
/// 并发安全靠 chat_state.start 单写者兜底（start 失败 = 忙 → 消息留流，下次触发
/// 源接手）。
pub(crate) async fn consume_steer_backlog(
    app: &AppHandle,
    pool: &SqlitePool,
    conv: &ConversationRow,
) -> AppResult<()> {
    // 等静默（turn_ended 广播先于 cleanup unregister；stop-false 兜底路径
    // is_streaming 已 false，此处直过）
    let chat_state = app.state::<ChatState>().inner().clone();
    let deadline = Instant::now() + QUIET_TIMEOUT;
    while chat_state.is_streaming(&conv.id) {
        if Instant::now() >= deadline {
            tracing::info!(
                target: "ice_paw.steer",
                conv = %conv.id,
                "触发点等待静默超时（{QUIET_TIMEOUT:?}），本轮放弃——下次触发源接手"
            );
            return Ok(());
        }
        tokio::time::sleep(QUIET_POLL).await;
    }

    // 积压边界 = 最近一次 turn_ended 的 turn_id（= 该回合 user_msg_id，
    // turn_id == user_msg_id 不变式）
    let Some(boundary) = last_turn_ended_id(pool, &conv.id).await else {
        return Ok(()); // 无已完成回合（异常，正常 turn_ended 触发必命中）
    };

    let mut backlog = repo::message::list_user_anchors_after(pool, &conv.id, &boundary).await?;
    if backlog.is_empty() {
        return Ok(()); // 无 steer 消息（普通回合正常收尾）
    }

    // 静默窗口（连发聚合）：窗口内又有新 user 行 → 重置继续等（说完一起听）
    let mut seen = backlog.len();
    let deadline = Instant::now() + QUIET_TIMEOUT;
    loop {
        tokio::time::sleep(CHAIN_HEAD_QUIET).await;
        match repo::message::list_user_anchors_after(pool, &conv.id, &boundary).await {
            Ok(b) if b.len() > seen => seen = b.len(),
            Ok(_) => break,
            Err(_) => break,
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    backlog = repo::message::list_user_anchors_after(pool, &conv.id, &boundary).await?;
    if backlog.is_empty() {
        return Ok(());
    }

    // 消费**最旧一条**（anchor = backlog.first()；其余留待本轮 turn_ended 触发点
    // 续接——连发各自成回合、串行完整执行，数据层不合并）
    let anchor = backlog.first().expect("backlog 非空必有头").message_id.clone();
    let count = backlog.len();
    run_steer_turn(app, pool, conv, &anchor, count).await
}

/// 最近一次 turn_ended 的 turn_id（即该回合的 user_msg_id）。无则 None。
async fn last_turn_ended_id(pool: &SqlitePool, conv_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT turn_id FROM session_events \
          WHERE session_id = ? AND kind = 'turn_ended' \
            AND turn_id IS NOT NULL AND turn_id != '' \
          ORDER BY seq DESC LIMIT 1",
    )
    .bind(conv_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 消费一条 steer 积压为完整回合（fire-and-forget，频道 `run_next_hop` 同款）。
///
/// `anchor` = 被消费的用户消息 id（turn_id == user_msg_id 不变式）；`count` =
/// 本次积压条数（事实简报用）。pre_materialized=true：用户侧已物化（行/事件/
/// 附件全在流里），回合只负责起跑——跳过用户侧落库，重复 INSERT 会 PK 冲突。
async fn run_steer_turn(
    app: &AppHandle,
    pool: &SqlitePool,
    conv: &ConversationRow,
    anchor: &str,
    count: usize,
) -> AppResult<()> {
    // --- 预检：会话 agent 凭据 + provider（失败留流，下次触发源接手）---
    let agent_cmd = app.state::<Arc<dyn AgentCmd>>().inner().clone();
    let creds = agent_cmd.get_with_credentials(&conv.agent_id).await?;
    let llm_provider = provider::create_provider(
        &creds.agent.provider,
        &creds.agent.model,
        creds.base_url.as_deref(),
        creds.agent.cache_prompt != 0,
    )?;

    // --- 单写者仲裁（忙 = 消息留流，触发点接手）---
    let chat_state = app.state::<ChatState>().inner().clone();
    let Ok(cancel_token) = chat_state.start(&conv.id) else {
        tracing::info!(
            target: "ice_paw.steer",
            conv = %conv.id,
            "会话忙，steer 消费暂缓（回合结束后接手）"
        );
        return Ok(());
    };
    // RAII 兜底：start 成功后、spawn 前的任何 `?` 早退自动 unregister
    //（chat_cmd 同款；spawn 成功后注销责任移交 stream_loop 的 finalize_*）
    let conv_id_guard = conv.id.clone();
    let cancel_guard = scopeguard::guard((), |_| chat_state.unregister(&conv_id_guard));

    // --- 事实简报（用户原话已在历史尾部，简报只说明插话语境）---
    let brief = compose_steer_brief(count);
    let brief_text = ContentBlock::join_text(&brief);
    let fallback =
        crate::harness::fallback_plan::production_fallback_plan(app, pool, &creds.agent);

    // fire-and-forget：完成信号 drop（chat_cmd 用户路径同款），后续积压由
    // turn_ended watcher 驱动——本函数不等待回合完成。
    let _done = session_runner::run_agent_turn(
        &TurnEnv {
            emitter: crate::harness::r#loop::emitter::tauri_emitter(app.clone(), conv.id.clone()),
            tool_app: Some(app.clone()),
            pool: pool.clone(),
            route_registry: app.state::<crate::harness::read_route::ReadRouteRegistry>().inner(),
            chat_state: chat_state.clone(),
            global_registry: Arc::clone(
                app.state::<Arc<crate::harness::mcp::McpRegistry>>().inner(),
            ),
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
            // turn_id == user_msg_id：steer 消息 B 自己的 turn 锚 = B 的 message_id
            user_msg_id: anchor.to_string(),
            content_text: brief_text,
            llm_blocks: brief,
            persist_blocks: Vec::new(), // pre_materialized：不落用户侧
            attach_db_inputs: Vec::new(),
            attach_file_inputs: Vec::new(),
            emit_user_blocks: false,
            incoming_source: None,
            sender_name: None,
            pre_materialized: true,
            tools_enabled: true,
            model_override: None,
            cancel_token,
            fallback,
        },
    )
    .await?;
    // spawn 成功：注销责任已移交 stream_loop，解除守卫
    scopeguard::ScopeGuard::into_inner(cancel_guard);
    tracing::info!(
        target: "ice_paw.steer",
        conv = %conv.id,
        "steer 消费回合已发起（积压 {count} 条，消费最旧一条）"
    );
    Ok(())
}
