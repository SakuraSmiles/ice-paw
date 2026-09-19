//! Steer（1v1 生成中插话）引擎：自动打断在途回合 + turn_ended 后自动续跑。
//!
//! 机制 = 两个现成能力的接线（docs/steer-design.md §2）：
//! - **打断**半：`chat_cmd` 的 Steer 分支先快照在途回合令牌（[`ChatState::token_of`]），
//!   物化用户消息 B 后**点名取消快照令牌**（W1 ②：stop 无回合身份——大附件
//!   物化超过回合 A 自然收尾时，迟到的 stop 会误伤已起跑的续跑回合 B；死令牌
//!   cancel = 无害 no-op），loop 在 yield 点轮询，到达即由
//!   [`crate::harness::cleanup::finalize_cancel`] 对称收尾；
//! - **续跑**半：本模块订阅 turn_ended 广播 → 等静默 → 查积压（DB 为真相源，
//!   复用频道 C8 的「用户消息无条件物化」）→ `run_agent_turn(pre_materialized=true)`
//!   开新回合 B。
//!
//! 与频道的分工：频道是**共址**共享流（一时刻一成员发言、@ 接力），Steer 是
//! **1v1 无状态 replay**（不引入世界状态/长期记忆）——已完成轮次已落库，新回合
//! 重读历史即得上下文连续性。无新事件 kind、不动 loop 核心、不动 append-only
//! 日志模型（见设计稿 §4/§13）。
//!
//! ## 积压边界（marker 记账，W1 ①）
//!
//! 积压 = 「已入账、未消费」的真实用户消息锚点（
//! `repo::message::list_unconsumed_user_anchors`）：`user_message` 事件在场
//! （已入账，同时排除 Phase 0 前零事件旧行）且无 `turn_context`/`turn_ended`
//! 标记（未消费；turn_context 恒在每回合起跑落库，turn_ended 兜 backfill
//! 合成形态）。旧版「最近 turn_ended turn_id 之后」的推断边界有两个洞——撞忙
//! 放弃会把边界让给并发回合（积压无痕消失）、boot 后无从得知未消费面（崩溃
//! 搁浅）——marker 记账两侧皆治：账面即真相源，任何触发源（turn_ended
//! watcher / chat_cmd 兜底 / boot 扫尾）读到同一积压。连发 B1/B2/B3 各自成
//! 回合、串行完整执行（每次消费**最旧一条**，`user_msg_id = anchor.message_id`
//! ——数据层不合并，见设计稿 §10.2/§11.1）。
//!
//! ## 并发
//!
//! chat_state.start 是最终仲裁（与频道同款哲学）：撞忙不放弃——退避回循环
//! 头等静默、按剩余积压续接（设计稿 §10.3 承诺的退避重试，W1 ① 落地）。
//! 两路并发触发时一路 start 成功、另一路循环等待；账面（marker 记账）保证
//! 不丢不重。已知残余：极窄竞速窗口下消息可能稍晚被消费（方向是「多干活」
//! 非丢消息，诚实可接受）。

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

/// 消费 1v1 积压：未消费的用户消息锚点（marker 记账）→ 静默窗 → 消费最旧
/// 一条；撞忙不放弃，退避重试直到发起成功或超时。
///
/// 调用源：`on_steer_turn_ended`（turn_ended 广播）、`chat_cmd` Steer 分支的
/// 死令牌兜底（A 恰在物化期间自然收尾 → 无新 turn_ended 再触发，手动接管）、
/// boot 扫尾 [`spawn_boot_sweep`]。并发安全靠 chat_state.start 单写者兜底：
/// 撞忙 = 并发触发源已起跑，回到循环头等它收尾再按剩余积压续接（§10.3）。
pub(crate) async fn consume_steer_backlog(
    app: &AppHandle,
    pool: &SqlitePool,
    conv: &ConversationRow,
) -> AppResult<()> {
    let chat_state = app.state::<ChatState>().inner().clone();
    let deadline = Instant::now() + QUIET_TIMEOUT;
    loop {
        // 等静默（turn_ended 广播先于 cleanup unregister；兜底/boot 路径
        // is_streaming 已 false，此处直过）
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

        // 积压 = 未消费锚点（marker 记账，见模块头「积压边界」）
        let mut backlog = repo::message::list_unconsumed_user_anchors(pool, &conv.id).await?;
        if backlog.is_empty() {
            return Ok(()); // 无积压（普通回合正常收尾 / 已被并发触发源消费）
        }

        // 静默窗口（连发聚合）：窗口内又有新 user 行 → 重置继续等（说完一起听）
        let mut seen = backlog.len();
        let quiet_deadline = Instant::now() + QUIET_TIMEOUT;
        loop {
            tokio::time::sleep(CHAIN_HEAD_QUIET).await;
            match repo::message::list_unconsumed_user_anchors(pool, &conv.id).await {
                Ok(b) if b.len() > seen => seen = b.len(),
                Ok(_) => break,
                Err(_) => break,
            }
            if Instant::now() >= quiet_deadline {
                break;
            }
        }
        backlog = repo::message::list_unconsumed_user_anchors(pool, &conv.id).await?;
        if backlog.is_empty() {
            return Ok(()); // 窗口期被并发触发源消费完毕
        }

        // 消费**最旧一条**（anchor = backlog.first()；其余留待本轮 turn_ended 触发点
        // 续接——连发各自成回合、串行完整执行，数据层不合并）
        let anchor = backlog.first().expect("backlog 非空必有头").message_id.clone();
        let count = backlog.len();
        match run_steer_turn(app, pool, conv, &anchor, count).await? {
            SteerTurnOutcome::Dispatched => return Ok(()),
            // 撞忙 = 并发触发源已起跑：退避回循环头等它收尾、按剩余积压续接
            //（§10.3）——旧版在此放弃，推断边界下积压可能就此搁浅（W1 ①）。
            SteerTurnOutcome::Busy => continue,
        }
    }
}

/// 消费一条 steer 积压的结果（`consume_steer_backlog` 的退避重试循环用）。
enum SteerTurnOutcome {
    /// 回合已发起（fire-and-forget；注销责任已移交 stream_loop 的 finalize_*）
    Dispatched,
    /// 会话忙（chat_state.start 撞忙）——消息留流，调用方退避后按剩余积压续接
    Busy,
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
) -> AppResult<SteerTurnOutcome> {
    // --- 预检：会话 agent 凭据 + provider（失败留流，下次触发源接手）---
    let agent_cmd = app.state::<Arc<dyn AgentCmd>>().inner().clone();
    let creds = agent_cmd.get_with_credentials(&conv.agent_id).await?;
    let llm_provider = provider::create_provider(
        &creds.agent.provider,
        &creds.agent.model,
        creds.base_url.as_deref(),
        creds.agent.cache_prompt != 0,
    )?;

    // --- 单写者仲裁（忙 = 交回调用方退避重试，W1 ①）---
    let chat_state = app.state::<ChatState>().inner().clone();
    let Ok(cancel_token) = chat_state.start(&conv.id) else {
        tracing::info!(
            target: "ice_paw.steer",
            conv = %conv.id,
            "会话忙，steer 消费退避重试（等在途回合收尾后接手）"
        );
        return Ok(SteerTurnOutcome::Busy);
    };
    // RAII 兜底：start 成功后、spawn 前的任何 `?` 早退自动 unregister
    //（chat_cmd 同款；spawn 成功后注销责任移交 stream_loop 的 finalize_*）
    let conv_id_guard = conv.id.clone();
    let cancel_guard = scopeguard::guard((), |_| chat_state.unregister(&conv_id_guard));

    // --- 事实简报（用户原话已在历史尾部，简报只说明插话语境）---
    let brief = compose_steer_brief(count);
    let brief_text = ContentBlock::join_text(&brief);
    // 4.4：简报文本是引擎语境说明非用户意图——检索 query 用被消费消息的
    // 用户原话（anchor 行 content）；行缺失/读失败回落简报（None → runner
    // 用 content_text 兜底）。
    let relevance_query = match repo::message::find_by_id(pool, anchor).await {
        Ok(Some(row)) => Some(row.content),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(target: "ice_paw.steer", "积压消息原话读取失败（query 回落简报）: {e}");
            None
        }
    };
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
            relevance_query,
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
    Ok(SteerTurnOutcome::Dispatched)
}

// =========================================================================
// boot 扫尾（W1 ③：崩溃/重启搁浅积压的自愈入口）
// =========================================================================

/// boot 扫尾（lib.rs setup 调用一次）：扫有未消费 steer 锚点的 1v1 会话，逐会话
/// 消费积压。
///
/// 覆盖场景：steer 消息 B 物化后、消费回合起跑前进程死亡/重启——turn_ended
/// watcher 不再触发、前端「排队中」角标随重启清零，积压若无扫尾即永久搁浅
///（B 永不回答）。幂等：marker 记账下已消费面自然排除，正常 boot（无搁浅）
/// 零命中零成本。后台化 + 失败仅 warn 不阻塞启动（与孤儿 turn 补记等既有
/// boot 扫尾同款纪律）；不动刚 boot 时的在途回合（等静默由
/// `consume_steer_backlog` 自带）。
pub fn spawn_boot_sweep(app: AppHandle, pool: SqlitePool) {
    tauri::async_runtime::spawn(async move {
        let convs = match repo::message::conversations_with_unconsumed_anchors(&pool).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    target: "ice_paw.steer",
                    "boot steer 扫尾查询失败（本次跳过）: {e}"
                );
                return;
            }
        };
        if convs.is_empty() {
            return;
        }
        tracing::info!(
            target: "ice_paw.steer",
            "boot steer 扫尾：发现 {n} 个搁浅积压会话，逐会话消费",
            n = convs.len()
        );
        for conv_id in convs {
            // 查询已按 kind='chat' 过滤；行在此间被删时跳过（CASCADE 清事件后
            // marker 面自洽，get 双保险）
            let Ok(conv) = repo::conversation::get_by_id(&pool, &conv_id).await else {
                continue;
            };
            if conv.kind != "chat" {
                continue;
            }
            if let Err(e) = consume_steer_backlog(&app, &pool, &conv).await {
                tracing::warn!(
                    target: "ice_paw.steer",
                    conv = %conv_id,
                    "boot steer 扫尾消费失败: {e}"
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 简报文案两分支：单条 vs 连发（连发须披露条数与「优先处理最近一条」）。
    #[test]
    fn steer_brief_single_vs_burst() {
        let one = compose_steer_brief(1);
        assert_eq!(one.len(), 1);
        let one_text = ContentBlock::join_text(&one);
        assert!(one_text.contains("一条新消息"));
        assert!(!one_text.contains("优先处理最近一条"));

        let many = compose_steer_brief(3);
        let many_text = ContentBlock::join_text(&many);
        assert!(many_text.contains("3 条新消息"));
        assert!(many_text.contains("优先处理最近一条"));
    }
}
