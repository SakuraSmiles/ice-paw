//! 频道 v1 串行共享流引擎（项目频道 = 多成员共享一个对话流）。
//!
//! 与既有两条协作通道的分工：**委派**（delegate）是同步阻塞的父子任务单元、
//! **跨会话投递**（inbox）是异步的对话单元——频道是**共址**单元：成员不互发
//! 消息，而是读同一份会话历史（共享流），一时刻只有一名成员发言（chat_state
//! 单写者结构性保证）。被 @ 的成员跑一个完整回合（复用
//! [`session_runner::run_agent_turn`] 全链路：预算/工具/hooks/事件照常）。
//!
//! ## 路由（C3）
//!
//! - 用户消息带结构化 mentions（前端 @ 弹层）→ 被 @ 成员按出现序串行接令；
//! - 无 @ 广播 → 统筹者（project_agents.role='coordinator'）接令；
//! - 统筹空缺 → 批 2 选举接入前：消息诚实落流等待（warn 日志）；
//! - 成员回复终文里的 @（文本解析，C9 成员侧规则）→ 回合结束触发接力。
//!
//! ## 发送两步拆分（C8）
//!
//! 用户消息**物化无条件成功**（行 + user_message 事件 + 附件即时落流，气泡
//! 立即可见）；触发是第二步——会话在途就只落流（DB 的「链头之后新 user 行」
//! 即积压真相），回合结束触发点接管：取消剩余接力 → CHAIN_HEAD_QUIET 静默
//! 窗口（说完一起听）→ 积压全部消息合并为新链头（mentions = 各条顺序并集
//! 去重）→ 走路由。真实还原群聊体验：执行期间插话不会丢也不会打断当前回合。
//!
//! ## 事实分层（C10b）
//!
//! 内容性事实（谁说了什么）物化 messages 行；行为性事实（引擎做了什么——
//! 路由/接力/护栏拦截）只 append `channel_mention` 事件，不物化系统消息行
//! （对账平面保持构造性零 diff）；对 agent 的告知走**频道事实简报**（回合
//! 启动注入 llm_blocks、用完即弃不落库）。
//!
//! ## 链状态（内存态）
//!
//! 链头锚定 / 跳计数 / 有序对重复表 / 唤醒频率窗口全部内存（重启清零 = 链
//! 死，诚实可接受——链是分钟级生命周期；落库可观测性全靠 `channel_mention`
//! 事件，重启后轨迹仍完整）。**积压以 DB 为真相源**（流里链头之后的 user
//! 锚点行），内存标记只是触发捷径——两者漂移时抢占检查按 DB 为准。
//!
//! ## 并发（设计稿 §4 定案：不追求 runtime 完美互斥）
//!
//! chat_state.start 是最终仲裁：两路并发触发时一路 start 成功、另一路把跳
//! 放回队首静默退出，backlog 机制保证消息不丢自愈。已知残余：极窄竞速窗口
//! 下同一跳可能多跑一回合（方向是「多干活」非丢消息，诚实可接受）。
//!
//! ## 护栏（防接力链无限延长与循环互 @）
//!
//! 串行共享流下不存在并发风暴，风暴只剩链无限延长（MAX_CHAIN_TURNS）与
//! 乒乓互 @（MAX_PAIR_REPEAT）两形态。哲学同 inbox 的 MANUAL_REPLY_GUARD：
//! **护栏不依赖模型听话**（系统级计数拦截）。频率闸（MENTION_WINDOW /
//! MENTION_MAX_PER_MEMBER）只拦成员触发的跳——用户触发的跳不占频率（用户
//! 治理权最大，且积压合并机制下用户跳不存在风暴面）。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures::StreamExt;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};

use crate::commands::agent_cmd::AgentCmd;
use crate::db::models::{ConversationRow, NewMessage};
use crate::db::repo::{self, message::TurnAnchor};
use crate::error::{AppError, AppResult};
use crate::harness::chat_state::ChatState;
use crate::harness::event_log::{
    self, ChannelElectionPayload, ChannelElectionResult, ChannelElectionTallyItem,
    ChannelElectionVote, ChannelCoordinatorPayload, ChannelMentionPayload, EventCtx,
};
use crate::harness::provider;
use crate::harness::session_runner::{self, AgentTurnInput, TurnEnv};
use crate::infra::cancel::CancellationToken;
use crate::infra::protocol::{AttachedFile, ChatMessage, ContentBlock};

/// 单条用户消息（合并后的新链头）触发的连续成员回合上限。
/// 达到上限不再触发，`channel_mention` 记 chain_limit（前端提示条指路用户推进）。
const MAX_CHAIN_TURNS: usize = 8;
/// 同一有序对（发起者→被@者）在一条链内出现次数上限——A@B、B@A、A@B 第三次拦。
const MAX_PAIR_REPEAT: usize = 2;
/// 单条消息（或合并新链头）的 @ 数量上限，超出截断 + 诚实事件提示。
const MAX_MENTIONS_PER_MSG: usize = 5;
/// 成员唤醒频率窗口与上限（对齐 inbox AUTO_WINDOW/AUTO_CONSUME_MAX 同源语义：
/// 「外部触发的回合频率」；只拦成员触发的跳，用户跳不占）。
const MENTION_WINDOW: Duration = Duration::from_secs(600);
const MENTION_MAX_PER_MEMBER: usize = 6;
/// 用户积压 → 新链头启动前的静默窗口（C8）：窗口内又有新用户消息则重置
/// ——「说完一起听」，防连发被拆成多条碎链。
const CHAIN_HEAD_QUIET: Duration = Duration::from_secs(3);
/// 等会话静默的轮询节奏与上限（inbox drain 同款：turn_ended 广播先于 cleanup
/// unregister，正常 cleanup 在广播后数百 ms 内完成）。
const QUIET_POLL: Duration = Duration::from_secs(2);
const QUIET_TIMEOUT: Duration = Duration::from_secs(30);
/// 统筹者接令回合连续失败阈值（C5 换帅第二档）：达到即罢免 + 自动补选
/// （failed-over）。失败口径 = turn_ended.termination='error'（interrupted 是
/// 用户手势、length 是额度顶格，都不归咎统筹者）。
const COORD_FAIL_STREAK: usize = 2;
/// 选举投票 mini 回合小额度（兼任健康检查——投票本身就是一次真实请求，
/// 连不上的成员天然弃权，凭据/模型故障在选票里可见）。
const ELECTION_VOTE_MAX_TOKENS: i32 = 128;

// =========================================================================
// 纯函数：路由与 @ 文本解析（测试不依赖 DB）
// =========================================================================

/// 用户消息的路由决策（C3 路由表的函数化）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ChannelRoute {
    /// 串行触发这些成员（mentions 点名，或广播 → 统筹者单元素）
    Members(Vec<String>),
    /// 统筹空缺（项目无 role='coordinator' 成员）——批 2 选举接入
    NeedsCoordinator,
}

/// 路由附注（诚实披露：截断与非成员过滤都让用户/前端可见，不静默吞）。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RouteNotes {
    /// 被 MAX_MENTIONS_PER_MSG 截断掉的数量
    pub truncated: usize,
    /// 被成员资格过滤掉的 mention 数
    pub non_member: usize,
}

/// 路由一条（可能已合并多条积压的）用户消息。
///
/// mentions 非空 → 校验成员资格 + 去重保序 + 上限截断；空 → 统筹者接令；
/// 统筹空缺 → NeedsCoordinator。统筹者同时出现在 mentions 里不特殊处理
/// （@统筹者 = 点名，与广播接令同效，C3）。
pub(crate) fn route_user_message(
    mentions: &[String],
    member_ids: &[String],
    coordinator_id: Option<&str>,
) -> (ChannelRoute, RouteNotes) {
    let mut notes = RouteNotes::default();
    let mut valid: Vec<String> = Vec::new();
    for m in mentions {
        if !member_ids.contains(m) {
            notes.non_member += 1;
            continue;
        }
        if !valid.contains(m) {
            valid.push(m.clone());
        }
    }
    if valid.len() > MAX_MENTIONS_PER_MSG {
        notes.truncated = valid.len() - MAX_MENTIONS_PER_MSG;
        valid.truncate(MAX_MENTIONS_PER_MSG);
    }
    if !valid.is_empty() {
        return (ChannelRoute::Members(valid), notes);
    }
    match coordinator_id {
        Some(c) => (ChannelRoute::Members(vec![c.to_string()]), notes),
        None => (ChannelRoute::NeedsCoordinator, notes),
    }
}

/// 成员回复终文的 @ 文本解析（C9 成员侧规则）。
///
/// - `@` 后按**最长前缀**精确匹配成员名（有「张三」「张三丰」时 @张三丰 命中后者）；
/// - 重名歧义（同长度多个不同成员命中）不触发——诚实不猜；
/// - 非成员名忽略（提及非成员不是指令）；
/// - email 场景防御：`@` 前是字母/数字/下划线（xx@yy.com）不算 mention；
/// - 结果去重保序，超过 [`MAX_MENTIONS_PER_MSG`] 截断；
/// - `exclude`（发起者自身）过滤——@自己无接力意义。
pub(crate) fn parse_agent_mentions(
    text: &str,
    members: &[(String, String)],
    exclude: Option<&str>,
) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    for i in 0..chars.len() {
        if chars[i] != '@' {
            continue;
        }
        // email 防御：@ 紧跟在词字符之后不算 mention
        if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
            continue;
        }
        let rest: String = chars[i + 1..].iter().collect();
        // 最长前缀匹配；同长多命中（重名）= 歧义，跳过该位置
        let mut best: Option<(&String, usize)> = None;
        let mut ambiguous = false;
        for (id, name) in members {
            if name.is_empty() || !rest.starts_with(name.as_str()) {
                continue;
            }
            match best {
                Some((_, len)) if name.chars().count() > len => {
                    best = Some((id, name.chars().count()));
                    ambiguous = false;
                }
                Some((_, len)) if name.chars().count() == len => {
                    // 同名不同 id 才算歧义（同 id 重复入列不会发生）
                    if best.map(|(b, _)| b.as_str()) != Some(id.as_str()) {
                        ambiguous = true;
                    }
                }
                _ => best = Some((id, name.chars().count())),
            }
        }
        let Some((id, _name_len)) = best else { continue };
        if ambiguous {
            tracing::info!(target: "ice_paw.channel", "成员名重名歧义，@ 不触发接力");
            continue;
        }
        let id = id.clone();
        if Some(id.as_str()) == exclude {
            continue;
        }
        if !out.contains(&id) {
            out.push(id);
        }
    }
    if out.len() > MAX_MENTIONS_PER_MSG {
        out.truncate(MAX_MENTIONS_PER_MSG);
    }
    out
}

/// 频道消费回合的事实简报规格（触发语境两形态）。
pub(crate) enum BriefSpec {
    /// 用户链头消费（count = 合并进本链头的积压条数）
    UserBacklog { count: usize },
    /// 成员接力跳
    Relay { from_name: String },
}

/// 频道消费回合的事实简报（C10b：回合启动注入 llm_blocks，用完即弃不落库）。
///
/// 用户原话已在历史尾部（物化先行 + HistoryStage 全量读），简报只说明触发语境
/// 防双计——这是频道回合「当前轮 user 消息」的全部内容。
pub(crate) fn compose_channel_brief(brief: &BriefSpec) -> Vec<ContentBlock> {
    let text = match brief {
        BriefSpec::UserBacklog { count } => {
            if *count > 1 {
                format!(
                    "[频道事实] 用户发送了 {count} 条新消息（见上方最近的用户消息，发出于上一回合执行期间），按时间先后理解为递进指示，请一并处理。"
                )
            } else {
                "[频道事实] 用户在频道发送了新消息（见上方最近的用户消息），请处理。".to_string()
            }
        }
        BriefSpec::Relay { from_name } => {
            format!("[频道事实] 成员 {from_name} 在最近的回复中 @你并期待你接手，请阅读其消息并处理。")
        }
    };
    vec![ContentBlock::text(text)]
}

/// 从投票回复解析被投候选人（纯函数）。
///
/// 精确名字匹配 > 唯一子串命中；多命中（回复同时含多名成员名）与零命中同判
/// 弃权——不猜。成员自投合法（可投自己）。
pub(crate) fn match_candidate(reply: &str, members: &[(String, String)]) -> Option<String> {
    let t = reply.trim();
    if let Some((id, _)) = members.iter().find(|(_, name)| name == t) {
        return Some(id.clone());
    }
    let hits: Vec<&(String, String)> = members
        .iter()
        .filter(|(_, name)| t.contains(name.as_str()))
        .collect();
    match hits.as_slice() {
        [(id, _)] => Some(id.clone()),
        _ => None, // 弃权 / 歧义 / 未识别
    }
}

/// 计票 + 平票裁决（纯函数）。`members` 须为 joined_at 正序（list_member_profiles
/// 天然如此）——平票时表序首个领先者 = 最早加入者。返回 (票数表按成员序, 胜者,
/// 是否走了平票裁决)。全员弃权（最高票 0）胜者为 None。
pub(crate) fn tally_votes(
    votes: &[Option<String>],
    member_ids: &[String],
) -> (Vec<ChannelElectionTallyItem>, Option<String>, bool) {
    let tally: Vec<ChannelElectionTallyItem> = member_ids
        .iter()
        .map(|id| ChannelElectionTallyItem {
            agent_id: id.clone(),
            votes: votes
                .iter()
                .filter(|v| v.as_deref() == Some(id.as_str()))
                .count() as u32,
        })
        .collect();
    let max = tally.iter().map(|t| t.votes).max().unwrap_or(0);
    if max == 0 {
        return (tally, None, false);
    }
    // 首个领先者 = joined_at 最早（成员表序 = tally 序）；先取值再移 tally（借用序）
    let tie = tally.iter().filter(|t| t.votes == max).count() > 1;
    match tally.iter().position(|t| t.votes == max) {
        Some(i) => {
            let winner = tally[i].agent_id.clone();
            (tally, Some(winner), tie)
        }
        None => (tally, None, false),
    }
}

/// NeedsCoordinator 臂的链头登记值：积压**最早**一条（backlog 按时间正序）。
///
/// 该臂没有派发任何东西——选举完成后的含头重消费（`list_user_anchors_from`）
/// 必须覆盖**全部**积压，头锚只能是首条。取 last（= Members 臂的 new_head 语义）
/// 会把首条消息永久搁浅在 `from(last)` 边界之外（生产实案 2026-09-11：
/// 积压 2 条、选举后只消费 1 条，昨日首条至今无人应答）。
fn election_register_head(backlog: &[TurnAnchor]) -> String {
    backlog
        .first()
        .expect("backlog 非空必有头")
        .message_id
        .clone()
}

// =========================================================================
// 链运行时（内存态；临界区无 await）
// =========================================================================

/// 一跳（待执行或在途）。
#[derive(Debug, Clone)]
struct Hop {
    agent_id: String,
    /// 发起方（None = 用户消息触发——真 @ 点名或广播接令；Some = 成员接力 from）
    from: Option<String>,
    /// 用户消息是广播（无 @）而非点名——事件 payload 的 broadcast 位来源，
    /// 前端据此分词（「广播 · X 接令」vs「用户 点名 X 接力」）
    broadcast: bool,
}

/// 单频道运行时。字段语义见模块文档「链状态」。
#[derive(Debug, Default)]
struct ChannelRuntime {
    /// 链头 = 触发本链的最后一条用户消息 id（run_agent_turn 的 turn_id 锚）
    head_msg_id: Option<String>,
    /// 链头消息是否已被**真正派发**（run_next_hop 成功发起过成员回合）。
    /// false = 头已登记但还没派发（等选举落定/会话忙暂缓）——重消费须走
    /// **含头**查询（`list_user_anchors_from`），否则原消息被永久搁浅；
    /// true = 头已被消费，走严格之后（`list_user_anchors_after`）。
    head_dispatched: bool,
    /// 已**发起**的成员回合计数（含在途；MAX_CHAIN_TURNS 计数口径）
    turns_taken: usize,
    /// 有序对计数（成员→成员；用户跳不计——无乒乓语义）
    pair_counts: HashMap<(String, String), usize>,
    /// 待执行跳队列（front = 下一跳）
    pending: VecDeque<Hop>,
    /// 在途跳（发起时记、回合结束时清——sender sweep 的执行成员来源）
    current: Option<Hop>,
    /// 各成员唤醒时刻窗（频率闸；只登记成员触发的跳）
    wake_history: HashMap<String, VecDeque<Instant>>,
    /// 物化待并集的用户 mentions（内存捷径；重启丢失 → 降级广播路由，可接受）
    backlog_mentions: Vec<String>,
    /// 选举进行中标记（防 NeedsCoordinator 并发触发双选举；选举完成/早退清）
    election_running: bool,
}

static RUNTIMES: OnceLock<Mutex<HashMap<String, ChannelRuntime>>> = OnceLock::new();

/// 统筹者失败 streak（conv_id → 连续 error 终态计数）。**独立于 ChannelRuntime**
/// 存放：finish_chain 清链时 streak 必须跨链存活（第 1 次失败在链 A、第 2 次
/// 在链 B 也要累计）；换帅或成功回合时清零，重启丢失 = 从头计（内存护栏哲学）。
static COORD_STREAKS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

fn coord_streaks() -> &'static Mutex<HashMap<String, usize>> {
    COORD_STREAKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// streak 增一，返回增量后的值。
fn coord_streak_inc(conv_id: &str) -> usize {
    let mut map = coord_streaks().lock().unwrap();
    let n = map.entry(conv_id.to_string()).or_insert(0);
    *n += 1;
    *n
}

/// streak 清零（回合成功 / 换帅完成）。
fn coord_streak_reset(conv_id: &str) {
    coord_streaks().lock().unwrap().remove(conv_id);
}

/// streak 现值（广播降级判定用；无记录 = 0）。
fn coord_streak_get(conv_id: &str) -> usize {
    coord_streaks().lock().unwrap().get(conv_id).copied().unwrap_or(0)
}

/// 统筹者是否用户手动指定（C5 两档治理的出身判定：指定档故障不自动换帅）。
///
/// 扫描该会话全部 `channel_coordinator` 事件，跟踪指向此 agent 的最近一次
/// 出身动作：appointed → true；elected / failed-over → false（failed-over 后
/// 复位须用户手动 appointed，防帅位震荡的「不自动复辟」在事件序上自然成立）。
pub(crate) async fn coordinator_is_user_appointed(
    pool: &SqlitePool,
    conv_id: &str,
    agent_id: &str,
) -> bool {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT payload FROM session_events \
          WHERE session_id = ? AND kind = 'channel_coordinator' ORDER BY seq",
    )
    .bind(conv_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut appointed = false;
    for r in rows {
        let Ok(p) = serde_json::from_str::<ChannelCoordinatorPayload>(&r) else {
            continue;
        };
        if p.agent_id.as_deref() != Some(agent_id) {
            continue;
        }
        match p.action.as_str() {
            "appointed" => appointed = true,
            "elected" | "failed-over" => appointed = false,
            _ => {}
        }
    }
    appointed
}

fn runtimes() -> &'static Mutex<HashMap<String, ChannelRuntime>> {
    RUNTIMES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 唤醒频率闸（成员跳专用）：窗口内已满 → false。
fn wake_reserve(rt: &mut ChannelRuntime, agent_id: &str, now: Instant) -> bool {
    let q = rt.wake_history.entry(agent_id.to_string()).or_default();
    let cutoff = now - MENTION_WINDOW;
    while q.front().is_some_and(|t| *t < cutoff) {
        q.pop_front();
    }
    if q.len() >= MENTION_MAX_PER_MEMBER {
        return false;
    }
    q.push_back(now);
    true
}

// =========================================================================
// 发送入口（chat_cmd 频道分支；C8 两步拆分的第 1 步 + 触发尝试）
// =========================================================================

/// 频道用户消息入口：物化（无条件成功）→ 尝试触发。
///
/// 预处理（附件 materialize / @ 引用展开）与 1v1 同款复用；物化块镜像
/// `run_agent_turn` 的 `!pre_materialized` 段（行 + blocks + 附件 + user_message
/// 事件——消费回合 pre_materialized=true 跳过同段，两处形态必须保持一致）。
/// 会话在途就只落流（回合结束触发点读 DB 积压接管）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_user_send(
    app: &AppHandle,
    pool: &SqlitePool,
    chat_state: &ChatState,
    conv: ConversationRow,
    user_msg_id: String,
    blocks: Vec<ContentBlock>,
    content_text: String,
    files: Option<Vec<AttachedFile>>,
    mentions: Option<Vec<String>>,
) -> AppResult<()> {
    if conv.archived_at.is_some() {
        return Err(AppError::Validation(format!(
            "频道「{}」已归档（原项目已删除）——记录保留为只读，无法继续发送。\
             如需继续协作，请在新项目开启新频道",
            conv.title
        )));
    }
    if conv.project_id.is_none() {
        // C1 边界：散落频道不存在（ensure 只建挂项目的），防御性拒绝
        return Err(AppError::Validation(
            "频道未挂载项目（数据异常）——请重建频道或反馈问题".into(),
        ));
    }

    // --- 附件物化（1v1 同款：纯函数不写 DB，spawn_blocking 离开 async worker）---
    let (persist_blocks, attach_db_inputs, attach_file_inputs) = match files {
        Some(files) if !files.is_empty() => {
            crate::infra::file_validation::validate_files(&files)?;
            let mid = user_msg_id.clone();
            tokio::task::spawn_blocking(move || {
                crate::harness::attachments::materialize_file_blocks(&mid, blocks, &files)
            })
            .await
            .map_err(|e| AppError::Internal(format!("附件处理任务失败: {e}")))??
        }
        _ => (blocks, Vec::new(), Vec::new()),
    };
    // @ 引用展开（快照注入）：与 1v1 同款，失效降级不阻塞
    let persist_blocks = crate::harness::references::materialize_reference_blocks(
        pool,
        &conv.id,
        persist_blocks,
        &content_text,
    )
    .await;

    // --- 物化（无条件成功：行 + blocks + 附件 + user_message 事件）---
    let blocks_json = serde_json::to_string(&persist_blocks).unwrap_or_else(|_| "[]".into());
    repo::message::create(
        pool,
        &user_msg_id,
        &NewMessage {
            conversation_id: conv.id.clone(),
            role: "user".into(),
            content: content_text.clone(),
            token_count: None,
            error: None,
            model: None,
        },
    )
    .await?;
    repo::message::update_content_blocks(pool, &user_msg_id, &blocks_json).await?;
    if !attach_db_inputs.is_empty() {
        repo::message_attachment::delete_by_message(pool, &user_msg_id).await?;
        repo::message_attachment::insert_batch(pool, &user_msg_id, &attach_db_inputs).await?;
    }
    if !attach_file_inputs.is_empty() {
        repo::message_attachment_file::delete_by_message(pool, &user_msg_id).await?;
        repo::message_attachment_file::insert_batch(pool, &user_msg_id, &attach_file_inputs)
            .await?;
    }
    let ev = EventCtx::new(&conv.id, &user_msg_id, &conv.agent_id);
    event_log::log_user_message(pool, &ev, &user_msg_id, &content_text, &persist_blocks, None)
        .await;
    if !attach_db_inputs.is_empty() {
        event_log::log_attachment_stored(
            pool,
            &ev,
            &user_msg_id,
            &event_log::AttachmentStoredPayload::Pages {
                v: 1,
                items: attach_db_inputs
                    .iter()
                    .map(|c| event_log::AttachmentPageItem {
                        idx: c.idx,
                        name: c.name.clone(),
                        kind: c.kind.clone(),
                        label: c.label.clone(),
                        token_est: c.token_est,
                    })
                    .collect(),
            },
        )
        .await;
    }
    if !attach_file_inputs.is_empty() {
        event_log::log_attachment_stored(
            pool,
            &ev,
            &user_msg_id,
            &event_log::AttachmentStoredPayload::Bytes {
                v: 1,
                items: attach_file_inputs
                    .iter()
                    .map(|f| event_log::AttachmentBytesItem {
                        idx: f.idx,
                        name: f.name.clone(),
                        ext: f.ext.clone(),
                        bytes_len: f.bytes.len(),
                    })
                    .collect(),
            },
        )
        .await;
    }

    // --- 登记 mentions（内存捷径，consume 时并集）---
    if let Some(list) = mentions.as_deref().filter(|v| !v.is_empty()) {
        let mut map = runtimes().lock().unwrap();
        let rt = map.entry(conv.id.clone()).or_default();
        for m in list {
            if !rt.backlog_mentions.contains(m) {
                rt.backlog_mentions.push(m.clone());
            }
        }
    }

    // --- 触发尝试（第 2 步）：在途只登记（DB 积压是真相源）---
    if chat_state.is_streaming(&conv.id) {
        tracing::info!(
            target: "ice_paw.channel",
            conv = %conv.id,
            "频道在途回合期间收到用户消息，已落流（回合结束后合并处理）"
        );
        return Ok(());
    }
    let app = app.clone();
    let pool = pool.clone();
    let conv_for_spawn = conv.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = consume_backlog(&app, &pool, &conv_for_spawn).await {
            tracing::warn!(target: "ice_paw.channel", "频道积压消费发起失败: {e}");
        }
    });
    Ok(())
}

// =========================================================================
// 积压消费（新链头路由 + 第一跳）
// =========================================================================

/// 消费积压：链头之后的全部用户消息合并为新链头 → 路由 → 触发第一跳。
///
/// 调用源：用户发送（空闲时）与回合结束触发点（抢占检查后）。并发安全靠
/// chat_state.start 单写者兜底（start 失败 = 忙 → 消息留流，下次触发源接手）。
async fn consume_backlog(
    app: &AppHandle,
    pool: &SqlitePool,
    conv: &ConversationRow,
) -> AppResult<()> {
    let members = repo::project::list_member_profiles(
        pool,
        conv.project_id.as_deref().unwrap_or(""),
    )
    .await
    .unwrap_or_default();
    if members.is_empty() {
        tracing::warn!(
            target: "ice_paw.channel",
            conv = %conv.id,
            "频道项目已无成员，消息留在流中不消费"
        );
        return Ok(());
    }
    let coordinator_id = members
        .iter()
        .find(|m| m.role == "coordinator")
        .map(|m| m.agent_id.clone());

    // 积压边界三档（2026-09-10 生产实案修复：全新频道消息静默不被消费）：
    // ① 有链头且已派发 → 严格之后（现行语义）；② 有链头未派发（等选举落定/
    //    会话忙暂缓后重入）→ **含头**——链头消息自己还没被消费过；③ 无
    //    runtime（重启/首条）→ 以「最后一条**真实** assistant 消息」为界
    //    （其后全部 user 锚点 = 未消费积压；全新频道无 assistant 行 → 全部）。
    //    ⚠️ 旧实现回退边界 = 最后一条 user 锚点 = 刚物化的消息自己 → backlog
    //    恒空 → 路由整体静默 no-op（广播/点名全死，消息永久搁浅）。
    let old_state: Option<(String, bool)> = {
        let map = runtimes().lock().unwrap();
        map.get(&conv.id)
            .and_then(|rt| rt.head_msg_id.clone().map(|h| (h, rt.head_dispatched)))
    };
    let backlog: Vec<TurnAnchor> = match old_state {
        Some((head, true)) => {
            repo::message::list_user_anchors_after(pool, &conv.id, &head).await?
        }
        Some((head, false)) => {
            repo::message::list_user_anchors_from(pool, &conv.id, &head).await?
        }
        None => match repo::message::last_assistant_message_id(pool, &conv.id).await? {
            Some(last_reply) => {
                repo::message::list_user_anchors_after(pool, &conv.id, &last_reply).await?
            }
            None => repo::message::list_turn_anchors(pool, &conv.id).await?,
        },
    };
    if backlog.is_empty() {
        return Ok(()); // 无新消息（并发窗口内已被消费）
    }
    let new_head = backlog
        .last()
        .expect("backlog 非空必有尾")
        .message_id
        .clone();

    // 新链：取消旧 pending（user_preempted 事件）+ 重置链内计数（频率窗跨链
    // 保留）。锁段只收集数据，事件发射在锁外（guard 跨 await 会毒化 Send）。
    // ⚠️ 链头提交推迟到路由臂（Members 设头 / NeedsCoordinator 仅空缺时初始
    // 化）——在此先行提交会把「登记了头但还没派发」的消息从重消费边界里吃
    // 掉（选举完成后 on_coordinator_settled 重消费恒空 → 原广播永久搁浅）。
    let (cancelled, head_for_events, mentions_merged): (VecDeque<Hop>, String, Vec<String>) = {
        let mut map = runtimes().lock().unwrap();
        let rt = map.entry(conv.id.clone()).or_default();
        let cancelled: VecDeque<Hop> = std::mem::take(&mut rt.pending);
        let head_for_events = rt
            .head_msg_id
            .clone()
            .unwrap_or_else(|| new_head.clone());
        rt.turns_taken = 0;
        rt.pair_counts.clear();
        let mentions = std::mem::take(&mut rt.backlog_mentions);
        (cancelled, head_for_events, mentions)
    };
    for hop in cancelled {
        emit_mention_blocked(pool, &conv.id, &head_for_events, &hop, "user_preempted").await;
    }

    // C5 指定档故障降级：用户指定的统筹者连续失败达阈值 → 广播不派它接令
    //（点名路由照常——@成员 仍触发）；事实事件提示用户处置（@成员 / 换统筹者 /
    // 让系统补选）。系统自选的统筹者不会停留在此状态（自动换帅先行）。
    if coordinator_id.is_some()
        && mentions_merged.is_empty()
        && coord_streak_get(&conv.id) >= COORD_FAIL_STREAK
        && coordinator_is_user_appointed(
            pool,
            &conv.id,
            coordinator_id.as_deref().unwrap_or(""),
        )
        .await
    {
        let coord_hop = Hop {
            agent_id: coordinator_id.clone().unwrap_or_default(),
            from: None,
            broadcast: true,
        };
        emit_mention_blocked(pool, &conv.id, &new_head, &coord_hop, "coordinator_failed").await;
        tracing::warn!(
            target: "ice_paw.channel",
            conv = %conv.id,
            "统筹者连续故障（用户指定档）——广播降级为需点名，消息已落流待用户处置"
        );
        return Ok(());
    }

    let (route, notes) = route_user_message(
        &mentions_merged,
        &members
            .iter()
            .map(|m| m.agent_id.clone())
            .collect::<Vec<_>>(),
        coordinator_id.as_deref(),
    );
    if notes.truncated > 0 || notes.non_member > 0 {
        tracing::info!(
            target: "ice_paw.channel",
            conv = %conv.id,
            truncated = notes.truncated,
            non_member = notes.non_member,
            "mentions 路由调整（截断/非成员过滤）"
        );
    }
    match route {
        ChannelRoute::Members(list) => {
            let n = list.len();
            {
                let mut map = runtimes().lock().unwrap();
                let rt = map.entry(conv.id.clone()).or_default();
                rt.pending = list
                    .into_iter()
                    .map(|agent_id| Hop {
                        agent_id,
                        from: None,
                        // mentions 空 ⇔ 广播（统筹者接令）——事件词汇分野位
                        broadcast: mentions_merged.is_empty(),
                    })
                    .collect();
                // 链头在此提交（路由确定、即将派发）；dispatched 由 run_next_hop
                // 真正发起成功后置位——本臂到发起之间若失败（会话忙暂缓等），
                // 重消费走含头查询，链头消息不丢。
                rt.head_msg_id = Some(new_head.clone());
                rt.head_dispatched = false;
            }
            tracing::info!(
                target: "ice_paw.channel",
                conv = %conv.id,
                backlog = backlog.len(),
                members = n,
                "频道新链头消费（{} 条积压合并）",
                backlog.len()
            );
            run_next_hop(app, pool, conv, &members, backlog.len()).await
        }
        ChannelRoute::NeedsCoordinator => {
            // C5：统筹空缺 → 触发自选举（消息已落流不丢；选举产出后 consume 接管）
            {
                let mut map = runtimes().lock().unwrap();
                let rt = map.entry(conv.id.clone()).or_default();
                // 仅空缺时初始化链头（dispatched=false → 选举完成后
                // on_coordinator_settled 的重消费走含头查询取回积压）；已有
                // 头不动——选举中 M2 到达重入时推进头会把 M1 搁浅在 from(M2)
                // 边界之外。头锚 = 积压**最早**一条（见 election_register_head）。
                if rt.head_msg_id.is_none() {
                    rt.head_msg_id = Some(election_register_head(&backlog));
                    rt.head_dispatched = false;
                }
                // 还原本轮 take 掉的 mentions（选举后重消费须按原意图路由，不
                // 能降级成广播）；选举期间新登记的追加在后（时间序）。
                if !mentions_merged.is_empty() {
                    let mut restored = mentions_merged;
                    restored.append(&mut rt.backlog_mentions);
                    rt.backlog_mentions = restored;
                }
            }
            tracing::warn!(
                target: "ice_paw.channel",
                conv = %conv.id,
                "频道无统筹者（role=coordinator 成员缺失），触发自选举（积压 {} 条待消费）",
                backlog.len()
            );
            trigger_election(app, pool, conv).await;
            Ok(())
        }
    }
}

// =========================================================================
// 自选举（C5：统筹空缺时成员互选定统筹者；一届 = started → N×vote → result）
// =========================================================================

/// 单成员投票 mini 回合的产物（引擎视角的一票）。
struct CastVote {
    /// 被投者（None = 弃权，reason 必填）
    candidate: Option<String>,
    reason: Option<String>,
}

/// 跑一届选举（spawn 调用；N 成员串行投票约 10~25s）。
///
/// 事实分层（C10b）：投票**内容**是成员真话——回复原文物化为 assistant 行进
/// 共享流（sender 标注投票者，全员可见选举发言）；引擎观察到的投票事实
/// （投给了谁/为何弃权）走 `channel_election` 事件。投票走 `stream_summary`
/// 通道（小额度 128 + 思考开关纪律），连不上的成员天然弃权——**兼任健康检查**。
/// 平票裁决 = joined_at 最早（成员表序）；全员弃权 = 胜者空缺（广播降级态，
/// 下次 NeedsCoordinator 查 `last_election_all_abstained` 不再自动重选）。
pub(crate) async fn run_election(app: &AppHandle, pool: &SqlitePool, conv: &ConversationRow) {
    let Some(pid) = conv.project_id.clone() else { return };
    let members = repo::project::list_member_profiles(pool, &pid)
        .await
        .unwrap_or_default();
    if members.is_empty() {
        tracing::warn!(target: "ice_paw.channel", conv = %conv.id, "选举取消：项目已无成员");
        return;
    }
    let election_id = uuid::Uuid::new_v4().to_string();
    let turn = format!("election:{election_id}");
    let ctx = EventCtx::new(&conv.id, &turn, &conv.agent_id);
    event_log::log_channel_election(
        pool,
        &ctx,
        &election_id,
        &ChannelElectionPayload {
            v: 1,
            phase: "started".into(),
            vote: None,
            result: None,
        },
    )
    .await;

    // 串行投票（每票一事件；凭据/请求失败的票诚实记弃权原因）
    let member_pairs: Vec<(String, String)> = members
        .iter()
        .map(|m| (m.agent_id.clone(), m.name.clone()))
        .collect();
    let mut votes: Vec<Option<String>> = Vec::with_capacity(members.len());
    for m in &members {
        let cast = cast_vote(app, pool, conv, &turn, &member_pairs, m).await;
        event_log::log_channel_election(
            pool,
            &ctx,
            &election_id,
            &ChannelElectionPayload {
                v: 1,
                phase: "vote".into(),
                vote: Some(ChannelElectionVote {
                    voter_agent_id: m.agent_id.clone(),
                    candidate_agent_id: cast.candidate.clone(),
                    reason: cast.reason.clone(),
                }),
                result: None,
            },
        )
        .await;
        votes.push(cast.candidate);
    }

    // 计票 + result 事件
    let member_ids: Vec<String> = members.iter().map(|m| m.agent_id.clone()).collect();
    let (tally, winner, tie) = tally_votes(&votes, &member_ids);
    event_log::log_channel_election(
        pool,
        &ctx,
        &election_id,
        &ChannelElectionPayload {
            v: 1,
            phase: "result".into(),
            vote: None,
            result: Some(ChannelElectionResult {
                tally,
                winner_agent_id: winner.clone(),
                tie_break: tie.then_some("joined_at".to_string()),
            }),
        },
    )
    .await;

    match winner {
        Some(w) => {
            // 现统筹降回 member（换选场景；同人选连任则跳过）
            if let Some(cur) = members
                .iter()
                .find(|m| m.role == "coordinator" && m.agent_id != w)
            {
                let _ = repo::project::set_member_role(pool, &pid, &cur.agent_id, "member").await;
            }
            if let Err(e) = repo::project::set_member_role(pool, &pid, &w, "coordinator").await {
                tracing::warn!(target: "ice_paw.channel", "选举胜者 role 置位失败: {e}");
                return;
            }
            if let Err(e) = repo::conversation::set_conversation_agent(pool, &conv.id, &w).await {
                tracing::warn!(target: "ice_paw.channel", "选举胜者投影更新失败: {e}");
            }
            coord_streak_reset(&conv.id);
            event_log::log_channel_coordinator(
                pool,
                &EventCtx::new(&conv.id, "", &w),
                &ChannelCoordinatorPayload {
                    v: 1,
                    action: "elected".into(),
                    agent_id: Some(w.clone()),
                    reason: Some("自选举产出".into()),
                },
            )
            .await;
            tracing::info!(target: "ice_paw.channel", conv = %conv.id, "频道选举完成，新统筹者: {w}");
        }
        None => {
            // 全员弃权：广播降级态（不设统筹）；下次 NeedsCoordinator 不自动重选
            tracing::warn!(
                target: "ice_paw.channel",
                conv = %conv.id,
                "频道选举全员弃权，统筹空缺（广播降级）——由用户指定统筹者解除"
            );
        }
    }
}

/// 单成员投票 mini 回合：直连 provider（不占 chat_state 单写者——共识机制非
/// 频道发言权）→ 回复原文物化 assistant 行 + assistant_message 事件（sender
/// 标注投票者）→ 解析被投者。任何失败诚实弃权（不抛出——选举不因单票故障中止）。
async fn cast_vote(
    app: &AppHandle,
    pool: &SqlitePool,
    conv: &ConversationRow,
    turn: &str,
    member_pairs: &[(String, String)],
    m: &repo::project::ProjectMemberProfile,
) -> CastVote {
    let agent_cmd = app.state::<Arc<dyn AgentCmd>>().inner().clone();
    let creds = match agent_cmd.get_with_credentials(&m.agent_id).await {
        Ok(c) => c,
        Err(e) => {
            return CastVote {
                candidate: None,
                reason: Some(format!("凭据/档案不可用（弃权）: {e}")),
            }
        }
    };
    let llm = match provider::create_provider(
        &creds.agent.provider,
        &creds.agent.model,
        creds.base_url.as_deref(),
        creds.agent.cache_prompt != 0,
    ) {
        Ok(p) => p,
        Err(e) => {
            return CastVote {
                candidate: None,
                reason: Some(format!("provider 创建失败（弃权）: {e}")),
            }
        }
    };
    let names = member_pairs
        .iter()
        .map(|(_, n)| n.as_str())
        .collect::<Vec<_>>()
        .join("、");
    let prompt = format!(
        "频道「{}」统筹者选举进行中。统筹者负责在无人点名时接手任务并协调成员分工。\n候选人（可投自己）：{names}。\n请只回复一个候选人名字（不带标点、不解释）；无法决定时回复：弃权。",
        conv.title
    );
    let cancel = CancellationToken::new();
    let stream = match llm
        .stream_summary(
            &creds.api_key,
            vec![
                ChatMessage::from_text(
                    "system",
                    "你是频道成员，正在参与统筹者选举投票，按成员的可靠性与能力判断。",
                ),
                ChatMessage::from_text("user", prompt),
            ],
            0.0, // 稳定可复现（摘要通道同款纪律）
            ELECTION_VOTE_MAX_TOKENS,
            cancel,
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            return CastVote {
                candidate: None,
                reason: Some(format!("投票请求失败（弃权）: {e}")),
            }
        }
    };
    let mut full = String::new();
    tokio::pin!(stream);
    while let Some(result) = stream.next().await {
        if let Ok(crate::infra::protocol::ChatDelta::Delta { content }) = result {
            full.push_str(&content);
        }
    }
    let reply = full.trim();
    if reply.is_empty() {
        return CastVote {
            candidate: None,
            reason: Some("模型返回空（弃权）".into()),
        };
    }
    // 投票内容 = 成员真话：原文物化 assistant 行进共享流（sender 标注投票者）
    let mid = uuid::Uuid::new_v4().to_string();
    if repo::message::create(
        pool,
        &mid,
        &NewMessage {
            conversation_id: conv.id.clone(),
            role: "assistant".into(),
            content: reply.to_string(),
            token_count: None,
            error: None,
            model: Some(creds.agent.model.clone()),
        },
    )
    .await
    .is_ok()
    {
        // 出生打标投票者（行侧 sender 列）：投票行创建即带真实归属——否则
        // sweep 的 IS NULL 兜底会在统筹者回合结束后把它们全部盖成统筹者
        // （生产实案 2026-09-11：4 条投票行 sender 全被误标为同一名成员）。
        if let Err(e) = repo::message::set_sender_agent(pool, &mid, &m.agent_id).await {
            tracing::warn!(target: "ice_paw.channel", "投票行 sender 出生打标失败: {e}");
        }
        let ctx = EventCtx::new(&conv.id, turn, &m.agent_id)
            .with_sender_name(Some(m.name.clone()));
        event_log::log_assistant_message(
            pool,
            &ctx,
            &mid,
            Some(&creds.agent.model),
            reply,
            &[ContentBlock::text(reply.to_string())],
            None,
            None,
            0,
            false,
        )
        .await;
    }
    match match_candidate(reply, member_pairs) {
        Some(id) => CastVote {
            candidate: Some(id),
            reason: None,
        },
        None => CastVote {
            candidate: None,
            reason: Some(format!("回复未识别出候选（弃权）: {reply}")),
        },
    }
}

/// 最近一届选举是否全员弃权（胜者空缺）——NeedsCoordinator 自动重选的熔断：
/// 全员弃权说明成员都不愿/不能承担，自动重选只会空转；用户指定统筹者可解。
async fn last_election_all_abstained(pool: &SqlitePool, conv_id: &str) -> bool {
    let payload: Option<String> = sqlx::query_scalar(
        "SELECT payload FROM session_events \
          WHERE session_id = ? AND kind = 'channel_election' \
          ORDER BY seq DESC LIMIT 1",
    )
    .bind(conv_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    // 最近一条选举事件非 result（选举进行中/中断）不视为弃权终局
    payload
        .and_then(|p| serde_json::from_str::<ChannelElectionPayload>(&p).ok())
        .and_then(|p| p.result)
        .is_some_and(|r| r.winner_agent_id.is_none())
}

/// NeedsCoordinator 臂的选举触发（自动档：全员弃权熔断生效）。
async fn trigger_election(app: &AppHandle, pool: &SqlitePool, conv: &ConversationRow) {
    schedule_election(app, pool, conv, false).await;
}

/// 用户手势的补选入口（频道头部「让系统补选」按钮）——绕过全员弃权熔断
///（用户治理权最大），election_running 守卫与完成后重消费照常。
pub(crate) async fn manual_reelect(app: &AppHandle, pool: &SqlitePool, conv: &ConversationRow) {
    schedule_election(app, pool, conv, true).await;
}

/// 选举调度公共体：全员弃权熔断（force 绕过）→ election_running 守卫 → spawn
///（完成/早退清标记）→ run_election。
///
/// **选举后的积压重消费不在此处**——走事件总线（watcher 监听
/// `channel_coordinator` 事件触发 [`on_coordinator_settled`]）。这不只是解耦
/// 美感：consume_backlog 的 NeedsCoordinator 臂会再触发选举，直接 await 会形成
/// 「schedule_election → consume_backlog → schedule_election」互递归，rustc 对
/// 互相递归的 async future 无法自证 Send（spawn 边同样要求 Send，装箱也断不
/// 了 trait 推断环）——经总线广播是结构性断环，且顺带让 appointed/removed
/// 等用户治理动作也获得「统筹位落定 → 尝试消费积压」的同一语义。
async fn schedule_election(
    app: &AppHandle,
    pool: &SqlitePool,
    conv: &ConversationRow,
    force: bool,
) {
    if !force && last_election_all_abstained(pool, &conv.id).await {
        tracing::warn!(
            target: "ice_paw.channel",
            conv = %conv.id,
            "统筹空缺且上次选举全员弃权——不自动重选，消息留流（用户可指定统筹者或手动补选）"
        );
        return;
    }
    {
        let mut map = runtimes().lock().unwrap();
        let rt = map.entry(conv.id.clone()).or_default();
        if rt.election_running {
            return; // 一届选举进行中，本触发静默让路（统筹位落定事件会再触发消费）
        }
        rt.election_running = true;
    }
    let app = app.clone();
    let pool = pool.clone();
    let conv = conv.clone();
    tauri::async_runtime::spawn(async move {
        // 任何出口（含早退/panic 展开）都清 running 标记——只清标记不删 runtime
        //（链状态 [head/pending/频率窗] 与选举正交，误删会把活链打成孤儿）
        let conv_id = conv.id.clone();
        let _guard = scopeguard::guard((), |_| {
            if let Some(rt) = runtimes().lock().unwrap().get_mut(&conv_id) {
                rt.election_running = false;
            }
        });
        run_election(&app, &pool, &conv).await;
    });
}



/// 锁段判定产物（guard 不跨 await——数据在锁内收集，动作在锁外执行）。
enum NextStep {
    /// 链达上限：取消全部剩余跳
    ChainLimit { head: String, cancelled: VecDeque<Hop> },
    /// 无链 / 队列空且无在途 → 链自然终结（或本就无事可做）
    Stop { finish: bool },
    /// 有序对乒乓闸拦截（拦此跳，链继续）
    PairBlocked { head: String, hop: Hop },
    /// 频率闸拦截（拦此跳，链继续）
    FreqBlocked { head: String, hop: Hop },
    /// 跳正常起跑
    Go {
        hop: Hop,
        head: String,
        hop_index: u32,
        chain_remaining: u32,
    },
}

/// 执行 pending 队列的下一跳。护栏拦截的跳记事件后跳过（继续后续跳）；
/// 凭据/档案读不出的跳同样跳过；队列空 = 链自然终止（清理 runtime）。
async fn run_next_hop(
    app: &AppHandle,
    pool: &SqlitePool,
    conv: &ConversationRow,
    members: &[repo::project::ProjectMemberProfile],
    backlog_count: usize,
) -> AppResult<()> {
    loop {
        // --- 锁内：取跳 + 护栏判定（临界区无 await；borrow 在块内收口）---
        let step = {
            let mut map = runtimes().lock().unwrap();
            let Some(rt) = map.get_mut(&conv.id) else {
                return Ok(());
            };
            if rt.turns_taken >= MAX_CHAIN_TURNS {
                let head = rt.head_msg_id.clone().unwrap_or_default();
                let cancelled: VecDeque<Hop> = std::mem::take(&mut rt.pending);
                rt.current = None;
                NextStep::ChainLimit { head, cancelled }
            } else if let Some(hop) = rt.pending.pop_front() {
                let head = rt.head_msg_id.clone().unwrap_or_default();
                // 有序对乒乓闸（成员跳专用；用户跳不计不入）
                let pair_blocked = hop.from.as_deref().is_some_and(|from| {
                    let key = (from.to_string(), hop.agent_id.clone());
                    let count = rt.pair_counts.entry(key).or_insert(0);
                    if *count >= MAX_PAIR_REPEAT {
                        true
                    } else {
                        *count += 1;
                        false
                    }
                });
                if pair_blocked {
                    NextStep::PairBlocked { head, hop }
                } else if hop.from.is_some()
                    && !wake_reserve(rt, &hop.agent_id, Instant::now())
                {
                    // 频率闸（成员跳专用；用户跳不占频率）
                    NextStep::FreqBlocked { head, hop }
                } else {
                    rt.turns_taken += 1;
                    let hop_index = rt.turns_taken as u32;
                    let chain_remaining = rt.pending.len() as u32;
                    rt.current = Some(hop.clone());
                    NextStep::Go {
                        hop,
                        head,
                        hop_index,
                        chain_remaining,
                    }
                }
            } else {
                // 队列空：链是否终结取决于在途跳（current）——本函数只在无在途时被调
                NextStep::Stop {
                    finish: rt.current.is_none(),
                }
            }
        };

        let (hop, head, hop_index, chain_remaining) = match step {
            NextStep::ChainLimit { head, cancelled } => {
                for hop in cancelled {
                    emit_mention_blocked(pool, &conv.id, &head, &hop, "chain_limit").await;
                }
                finish_chain(&conv.id);
                tracing::info!(
                    target: "ice_paw.channel",
                    conv = %conv.id,
                    "接力已达上限（{MAX_CHAIN_TURNS} 跳），链终止——请用户推进"
                );
                return Ok(());
            }
            NextStep::Stop { finish } => {
                if finish {
                    finish_chain(&conv.id);
                }
                return Ok(());
            }
            NextStep::PairBlocked { head, hop } => {
                emit_mention_blocked(pool, &conv.id, &head, &hop, "pair_repeat").await;
                continue; // 拦此跳，链继续走后续跳
            }
            NextStep::FreqBlocked { head, hop } => {
                emit_mention_blocked(pool, &conv.id, &head, &hop, "frequency").await;
                continue;
            }
            NextStep::Go {
                hop,
                head,
                hop_index,
                chain_remaining,
            } => (hop, head, hop_index, chain_remaining),
        };

        // --- 凭据预检（失败跳过该跳，链继续；跳没跑成不发正常事件）---
        let agent_cmd = app.state::<Arc<dyn AgentCmd>>().inner().clone();
        let creds = match agent_cmd.get_with_credentials(&hop.agent_id).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    target: "ice_paw.channel",
                    conv = %conv.id,
                    member = %hop.agent_id,
                    "成员凭据/档案读取失败（跳过该跳）: {e}"
                );
                clear_current(&conv.id, &hop.agent_id);
                continue;
            }
        };
        let llm_provider = match provider::create_provider(
            &creds.agent.provider,
            &creds.agent.model,
            creds.base_url.as_deref(),
            creds.agent.cache_prompt != 0,
        ) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    target: "ice_paw.channel",
                    conv = %conv.id,
                    member = %hop.agent_id,
                    "成员 provider 创建失败（跳过该跳）: {e}"
                );
                clear_current(&conv.id, &hop.agent_id);
                continue;
            }
        };

        // --- 单写者仲裁（忙 = 消息留流，触发点接手）---
        let chat_state = app.state::<ChatState>().inner().clone();
        let Ok(cancel_token) = chat_state.start(&conv.id) else {
            // 放回队首；本轮结束的 turn_ended watcher 会再走 consume/run。
            // 计数不回退（pair/wake 已记）——保守方向是「多拦」不是「漏拦」。
            let mut map = runtimes().lock().unwrap();
            if let Some(rt) = map.get_mut(&conv.id) {
                rt.pending.push_front(hop.clone());
                rt.current = None;
            }
            tracing::info!(
                target: "ice_paw.channel",
                conv = %conv.id,
                "频道会话忙，跳暂缓（回合结束后接手）"
            );
            return Ok(());
        };

        // RAII 兜底：start 成功后、spawn 前的任何 `?` 早退自动 unregister
        //（chat_cmd 同款；spawn 成功后注销责任移交 stream_loop 的 finalize_*）
        let conv_id_guard = conv.id.clone();
        let cancel_guard = scopeguard::guard((), |_| chat_state.unregister(&conv_id_guard));

        // --- 跳确定发生：mention 事实事件（此时才发——凭据失败/忙回退不发）---
        emit_mention(pool, &conv.id, &head, &hop, hop_index, chain_remaining).await;

        // --- 回合起跑（pre_materialized：用户侧已物化，llm_blocks 只装事实简报）---
        let brief_spec = match &hop.from {
            None => BriefSpec::UserBacklog {
                count: backlog_count.max(1),
            },
            Some(_) => BriefSpec::Relay {
                from_name: hop
                    .from
                    .as_deref()
                    .and_then(|id| members.iter().find(|m| m.agent_id == id))
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| "成员".to_string()),
            },
        };
        let brief = compose_channel_brief(&brief_spec);
        let brief_text = ContentBlock::join_text(&brief);
        let fallback =
            crate::commands::model_profile_cmd::production_fallback_plan(app, pool, &creds.agent);

        // fire-and-forget：完成信号 drop（chat_cmd 用户路径同款），后续跳由
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
                // 链头锚定（reconcile 对齐）：链内所有跳的 turn_id 恒 = head
                user_msg_id: head.clone(),
                content_text: brief_text,
                llm_blocks: brief,
                persist_blocks: Vec::new(), // pre_materialized：不落用户侧
                attach_db_inputs: Vec::new(),
                attach_file_inputs: Vec::new(),
                emit_user_blocks: false,
                incoming_source: None,
                sender_name: Some(creds.agent.name.clone()),
                pre_materialized: true,
                tools_enabled: true,
                model_override: None,
                cancel_token,
                // 成员自己的链（委派子会话同权：自链优先）
                fallback,
            },
        )
        .await?;
        // spawn 成功：注销责任已移交 stream_loop，解除守卫
        scopeguard::ScopeGuard::into_inner(cancel_guard);
        // 链头此刻起才算「已派发」——重消费边界从含头切回严格之后。
        //（busy 回退/凭据失败 continue 等未发起路径不置位：跳没跑成 = 未派发。）
        {
            let mut map = runtimes().lock().unwrap();
            if let Some(rt) = map.get_mut(&conv.id) {
                rt.head_dispatched = true;
            }
        }
        tracing::info!(
            target: "ice_paw.channel",
            conv = %conv.id,
            member = %hop.agent_id,
            "频道成员回合已发起"
        );
        return Ok(());
    }
}

// =========================================================================
// 回合结束触发点（EVENT_BUS 订阅 turn_ended；inbox drain watcher 同款模式）
// =========================================================================

/// 启动频道观察者（lib.rs setup 调用一次）。两路触发源：
/// - `turn_ended` → 回合结束 → 等静默 → sender sweep → 抢占检查（有积压 =
///   新链头消费）/ 无积压 = 解析终文 @ 接力。自然完成与用户终止两态同接。
/// - `channel_coordinator` → 统筹位落定（elected/appointed/removed/failed-over）
///   → 等静默 → 尝试消费积压。选举/治理完成后的重消费统一走此口（断开
///   consume_backlog ↔ schedule_election 的互递归 Send 推断环，见
///   [`schedule_election`] 注释）。
pub fn spawn_channel_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut rx = event_log::event_bus().subscribe();
        loop {
            match rx.recv().await {
                Ok(note) => {
                    let is_turn = note.kind == event_log::kind::TURN_ENDED;
                    let is_coord = note.kind == event_log::kind::CHANNEL_COORDINATOR;
                    if is_turn || is_coord {
                        let app = app.clone();
                        let conv_id = note.conversation_id.clone();
                        tauri::async_runtime::spawn(async move {
                            // 两 handler 返回各自的 impl Future，无法装进同一
                            // fn 指针——分支直调（各自的 Send 由 spawn 边验证）
                            if is_turn {
                                on_channel_turn_ended(&app, &conv_id).await;
                            } else {
                                on_coordinator_settled(&app, &conv_id).await;
                            }
                        });
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        target: "ice_paw.channel",
                        "频道观察者丢帧（{n}），本次触发跳过——下次触发源会再触发"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// 统筹位落定后的积压消费（election/appointed/removed/failed-over 全触发：
/// 统筹位变化 = 路由前提变化，此前扣在流里的消息值得一试）。
///
/// 等静默与 turn_ended 路径同款（appointed 可能在会话在途时发生）；无统筹 /
/// 全员弃权熔断会在 consume_backlog 内部诚实停手，无空转风险。
async fn on_coordinator_settled(app: &AppHandle, conv_id: &str) {
    let pool = app.state::<SqlitePool>().inner().clone();
    let conv = match repo::conversation::get_by_id(&pool, conv_id).await {
        Ok(c) => c,
        Err(_) => return,
    };
    if conv.kind != "channel" || conv.archived_at.is_some() {
        return;
    }
    let chat_state = app.state::<ChatState>().inner().clone();
    let deadline = Instant::now() + QUIET_TIMEOUT;
    while chat_state.is_streaming(conv_id) {
        if Instant::now() >= deadline {
            tracing::info!(
                target: "ice_paw.channel",
                conv = conv_id,
                "统筹位落定后等待静默超时，本次放弃——下次触发源接手"
            );
            return;
        }
        tokio::time::sleep(QUIET_POLL).await;
    }
    if let Err(e) = consume_backlog(app, &pool, &conv).await {
        tracing::warn!(target: "ice_paw.channel", "统筹位落定后积压消费失败: {e}");
    }
}

/// 频道回合结束处理（核心触发点）。
async fn on_channel_turn_ended(app: &AppHandle, conv_id: &str) {
    let pool = app.state::<SqlitePool>().inner().clone();
    let conv = match repo::conversation::get_by_id(&pool, conv_id).await {
        Ok(c) => c,
        Err(_) => return, // 会话已删（CASCADE 清事件），无事可做
    };
    if conv.kind != "channel" || conv.archived_at.is_some() {
        return;
    }

    // 等静默（turn_ended 广播先于 cleanup unregister）
    let chat_state = app.state::<ChatState>().inner().clone();
    let deadline = Instant::now() + QUIET_TIMEOUT;
    while chat_state.is_streaming(conv_id) {
        if Instant::now() >= deadline {
            tracing::info!(
                target: "ice_paw.channel",
                conv = conv_id,
                "触发点等待静默超时（{QUIET_TIMEOUT:?}），本轮放弃——下次触发源接手"
            );
            return;
        }
        tokio::time::sleep(QUIET_POLL).await;
    }

    // --- sender sweep（C10 行侧：链头起、IS NULL 的 assistant 行打执行成员）---
    let (head, current) = {
        let mut map = runtimes().lock().unwrap();
        match map.get_mut(conv_id) {
            Some(rt) => {
                let head = rt.head_msg_id.clone();
                let cur = rt.current.take();
                (head, cur)
            }
            None => return, // 无链状态（非频道引擎发起的回合）
        }
    };
    let Some(head) = head else { return };
    if let Some(hop) = &current {
        match repo::message::sweep_sender_for_channel_turn(&pool, conv_id, &head, &hop.agent_id)
            .await
        {
            Ok(n) if n > 0 => {
                tracing::debug!(
                    target: "ice_paw.channel",
                    conv = conv_id,
                    member = %hop.agent_id,
                    "sender sweep: {n} 条 assistant 行归属已标注"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(target: "ice_paw.channel", "sender sweep 失败: {e}"),
        }
    }
    // --- C5 换帅：统筹者接令回合终态失败计数 ---
    // 仅当本回合执行者 == 现任统筹者才动计数（成员回合成败不归咎统筹者）；
    // 失败口径 = termination='error'（interrupted=用户手势、length=额度顶格不算）；
    // 成功清零、跨链累计（streak 独立存储）。达阈值 → 罢免 + 补选。
    if let Some(hop) = &current {
        let coordinator = repo::project::list_member_profiles(
            &pool,
            conv.project_id.as_deref().unwrap_or(""),
        )
        .await
        .ok()
        .and_then(|ms| ms.into_iter().find(|m| m.role == "coordinator"));
        if coordinator.as_ref().map(|c| c.agent_id.as_str()) == Some(hop.agent_id.as_str()) {
            let failed =
                last_turn_termination(&pool, conv_id, &head).await.as_deref() == Some("error");
            if failed {
                let n = coord_streak_inc(conv_id);
                tracing::warn!(
                    target: "ice_paw.channel",
                    conv = conv_id,
                    streak = n,
                    "统筹者接令回合失败（error 终态）"
                );
                if n >= COORD_FAIL_STREAK {
                    if coordinator_is_user_appointed(&pool, conv_id, &hop.agent_id).await {
                        // C5 指定档：不自动换帅（治理权不越权）——广播降级为
                        // 「需点名」，下次广播发 blocked 事实事件提示用户处置。
                        tracing::warn!(
                            target: "ice_paw.channel",
                            conv = conv_id,
                            streak = n,
                            "用户指定的统筹者连续失败——不自动换帅，广播降级为需点名"
                        );
                    } else {
                        demote_and_reelect(app, &pool, &conv, &hop.agent_id, n).await;
                    }
                }
            } else {
                coord_streak_reset(conv_id);
            }
        }
    }

    // --- 抢占检查：链头之后有新 user 行 = 用户在回合期间插话 ---
    let backlog = match repo::message::list_user_anchors_after(&pool, conv_id, &head).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(target: "ice_paw.channel", "积压查询失败: {e}");
            return;
        }
    };
    if !backlog.is_empty() {
        // 静默窗口：说完一起听（窗口内又有新 user 行 → 重置继续等）
        let mut seen = backlog.len();
        let deadline = Instant::now() + QUIET_TIMEOUT;
        loop {
            tokio::time::sleep(CHAIN_HEAD_QUIET).await;
            match repo::message::list_user_anchors_after(&pool, conv_id, &head).await {
                Ok(b) if b.len() > seen => seen = b.len(),
                Ok(_) => break,
                Err(_) => break,
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        if let Err(e) = consume_backlog(app, &pool, &conv).await {
            tracing::warn!(target: "ice_paw.channel", "抢占后积压消费失败: {e}");
        }
        return;
    }

    // --- 无积压：接力检查（解析本回合终文的 @，C9 成员侧规则）---
    let members = repo::project::list_member_profiles(
        &pool,
        conv.project_id.as_deref().unwrap_or(""),
    )
    .await
    .unwrap_or_default();
    if members.is_empty() {
        finish_chain(conv_id);
        return;
    }
    let last_speaker = current.as_ref().map(|h| h.agent_id.clone());
    let final_text = match last_turn_final_text(&pool, conv_id, &head).await {
        Some(t) => t,
        None => {
            // 无终文（异常终态）——链按现状继续 pending（若有）
            tracing::debug!(target: "ice_paw.channel", conv = conv_id, "本回合无终文可解析");
            String::new()
        }
    };
    let member_pairs: Vec<(String, String)> = members
        .iter()
        .map(|m| (m.agent_id.clone(), m.name.clone()))
        .collect();
    let mentioned = parse_agent_mentions(&final_text, &member_pairs, last_speaker.as_deref());
    if !mentioned.is_empty() {
        let hops: VecDeque<Hop> = mentioned
            .into_iter()
            .map(|agent_id| Hop {
                agent_id,
                from: last_speaker.clone(),
                broadcast: false,
            })
            .collect();
        let mut map = runtimes().lock().unwrap();
        match map.get_mut(conv_id) {
            Some(rt) => rt.pending.extend(hops),
            None => return,
        }
    }
    // pending 可能有货（上述 extend 或忙时放回的跳）——继续走
    let has_pending = {
        let map = runtimes().lock().unwrap();
        map.get(conv_id)
            .map(|rt| !rt.pending.is_empty())
            .unwrap_or(false)
    };
    if has_pending {
        if let Err(e) = run_next_hop(app, &pool, &conv, &members, 1).await {
            tracing::warn!(target: "ice_paw.channel", "接力跳发起失败: {e}");
        }
    } else {
        // 无 @ 无 pending：链自然终止
        finish_chain(conv_id);
    }
}

// =========================================================================
// 事件发射与 runtime 小工具
// =========================================================================

/// 跳正常发生的 mention 事实（from=None → actor=user）。
///
/// turn_id/message_id 都锚链头——`chain:{链头id}` 三侧同归组键（事件词表
/// 注释 / 引擎 / 前端轨迹），事件归组到链头回合；对账侧 channel_mention 属
/// derive skip 臂，不参与行派生。
async fn emit_mention(
    pool: &SqlitePool,
    conv_id: &str,
    head: &str,
    hop: &Hop,
    hop_index: u32,
    chain_remaining: u32,
) {
    let ctx = EventCtx::new(conv_id, &format!("chain:{head}"), &hop.agent_id);
    event_log::log_channel_mention(
        pool,
        &ctx,
        head,
        &ChannelMentionPayload {
            v: 1,
            from_agent_id: hop.from.clone(),
            to_agent_id: hop.agent_id.clone(),
            broadcast: hop.broadcast,
            hop_index,
            chain_remaining,
            blocked_reason: None,
        },
    )
    .await;
}

/// 护栏拦截的 mention 事实（blocked_reason 必填；hop 计数无意义置 0）。
async fn emit_mention_blocked(
    pool: &SqlitePool,
    conv_id: &str,
    head: &str,
    hop: &Hop,
    reason: &str,
) {
    let ctx = EventCtx::new(conv_id, &format!("chain:{head}"), &hop.agent_id);
    event_log::log_channel_mention(
        pool,
        &ctx,
        head,
        &ChannelMentionPayload {
            v: 1,
            from_agent_id: hop.from.clone(),
            to_agent_id: hop.agent_id.clone(),
            broadcast: hop.broadcast,
            hop_index: 0,
            chain_remaining: 0,
            blocked_reason: Some(reason.to_string()),
        },
    )
    .await;
}

/// 链终结：清理 runtime（head 一并丢弃——下次用户消息新建链）。
fn finish_chain(conv_id: &str) {
    runtimes().lock().unwrap().remove(conv_id);
}

/// 清除在途跳标记（凭据失败等未发起场景）。
fn clear_current(conv_id: &str, agent_id: &str) {
    let mut map = runtimes().lock().unwrap();
    if let Some(rt) = map.get_mut(conv_id) {
        if rt.current.as_ref().map(|h| h.agent_id.as_str()) == Some(agent_id) {
            rt.current = None;
        }
    }
}

/// 本回合（turn_id = head 的最新 turn_ended）最终 assistant 正文。
///
/// turn_ended.message_id = 最后 assistant 消息（event_log 词表约定）——
/// 接力 @ 解析读它的正文。
async fn last_turn_final_text(pool: &SqlitePool, conv_id: &str, head: &str) -> Option<String> {
    let mid = last_turn_message_id(pool, conv_id, head).await?;
    sqlx::query_scalar::<_, String>("SELECT content FROM messages WHERE id = ?")
        .bind(&mid)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

/// 本回合终文的消息 id。
async fn last_turn_message_id(pool: &SqlitePool, conv_id: &str, head: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT message_id FROM session_events \
          WHERE session_id = ? AND turn_id = ? AND kind = 'turn_ended' \
          ORDER BY seq DESC LIMIT 1",
    )
    .bind(conv_id)
    .bind(head)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten()
}

/// 本回合（turn_id = head 的最新 turn_ended）终止原因——换帅失败口径判定用。
async fn last_turn_termination(pool: &SqlitePool, conv_id: &str, head: &str) -> Option<String> {
    let payload: Option<String> = sqlx::query_scalar(
        "SELECT payload FROM session_events \
          WHERE session_id = ? AND turn_id = ? AND kind = 'turn_ended' \
          ORDER BY seq DESC LIMIT 1",
    )
    .bind(conv_id)
    .bind(head)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    payload
        .and_then(|p| {
            serde_json::from_str::<crate::harness::event_log::TurnEndedPayload>(&p).ok()
        })
        .map(|p| p.termination)
}

/// 换帅（C5 第二档达阈值）：罢免现任（降回 member）+ failed-over 事件 + 清链 +
/// 触发补选。不自动重跑失败消息（失败终态在流里对用户可见，用户可重发——1v1
/// 同款体验）；projection（conv.agent_id）由选举胜者接管，全员弃权时停在
/// joined_at 最早成员的 ensure 语义上。
async fn demote_and_reelect(
    app: &AppHandle,
    pool: &SqlitePool,
    conv: &ConversationRow,
    demoted_id: &str,
    streak: usize,
) {
    let Some(pid) = conv.project_id.clone() else {
        return;
    };
    if let Err(e) = repo::project::set_member_role(pool, &pid, demoted_id, "member").await {
        tracing::warn!(target: "ice_paw.channel", "换帅罢免 role 置位失败: {e}");
        return;
    }
    coord_streak_reset(&conv.id);
    event_log::log_channel_coordinator(
        pool,
        &EventCtx::new(&conv.id, "", demoted_id),
        &ChannelCoordinatorPayload {
            v: 1,
            action: "failed-over".into(),
            agent_id: Some(demoted_id.to_string()),
            reason: Some(format!(
                "连续 {streak} 次接令回合失败（error 终态），自动换帅"
            )),
        },
    )
    .await;
    tracing::warn!(
        target: "ice_paw.channel",
        conv = %conv.id,
        demoted = demoted_id,
        "统筹者连续失败达阈值，已罢免并触发补选"
    );
    finish_chain(&conv.id);
    trigger_election(app, pool, conv).await;
}

// =========================================================================
// 单元测试（纯函数面；async 全链路见手测——inbox 同款纪律）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- route_user_message 表驱动 ----------

    #[test]
    fn route_mentions_point_named_members_in_order() {
        let (route, notes) = route_user_message(
            &["a".into(), "b".into(), "a".into()],
            &["a".into(), "b".into(), "c".into()],
            Some("c"),
        );
        assert_eq!(route, ChannelRoute::Members(vec!["a".into(), "b".into()]));
        assert_eq!(notes, RouteNotes::default());
    }

    #[test]
    fn route_broadcast_goes_to_coordinator() {
        let (route, _) = route_user_message(&[], &["a".into(), "b".into()], Some("b"));
        assert_eq!(route, ChannelRoute::Members(vec!["b".into()]));
    }

    #[test]
    fn route_no_coordinator_needs_election() {
        let (route, _) = route_user_message(&[], &["a".into()], None);
        assert_eq!(route, ChannelRoute::NeedsCoordinator);
    }

    #[test]
    fn route_non_member_mentions_filtered_and_noted() {
        // 全部无效 → 回落广播；部分无效 → 过滤但有效者仍点名
        let (route, notes) = route_user_message(
            &["ghost".into()],
            &["a".into()],
            Some("a"),
        );
        assert_eq!(route, ChannelRoute::Members(vec!["a".into()]));
        assert_eq!(notes.non_member, 1);

        let (route, notes) = route_user_message(
            &["ghost".into(), "a".into()],
            &["a".into()],
            None,
        );
        assert_eq!(route, ChannelRoute::Members(vec!["a".into()]));
        assert_eq!(notes.non_member, 1);
    }

    #[test]
    fn route_truncates_to_mention_cap() {
        let mentions: Vec<String> = (0..8).map(|i| format!("m{i}")).collect();
        let members = mentions.clone();
        let (route, notes) = route_user_message(&mentions, &members, None);
        match route {
            ChannelRoute::Members(list) => {
                assert_eq!(list.len(), MAX_MENTIONS_PER_MSG);
                assert_eq!(list[0], "m0"); // 保序截断
            }
            r => panic!("expected Members, got {r:?}"),
        }
        assert_eq!(notes.truncated, 8 - MAX_MENTIONS_PER_MSG);
    }

    // ---------- parse_agent_mentions ----------

    fn members_fixture() -> Vec<(String, String)> {
        vec![
            ("agent-zhang".into(), "张三".into()),
            ("agent-zhangf".into(), "张三丰".into()),
            ("agent-li".into(), "李四".into()),
        ]
    }

    #[test]
    fn parse_hits_member_and_longest_prefix_wins() {
        let out = parse_agent_mentions(
            "@张三 看下这个，@张三丰 你也参与",
            &members_fixture(),
            None,
        );
        assert_eq!(out, vec!["agent-zhang".to_string(), "agent-zhangf".to_string()]);
    }

    #[test]
    fn parse_email_defense() {
        let out = parse_agent_mentions("发邮件到 someone@李四 试试", &members_fixture(), None);
        // @ 前是字母数字 → 不算 mention
        assert!(out.is_empty());
    }

    #[test]
    fn parse_non_member_and_self_ignored() {
        let out = parse_agent_mentions(
            "@王五 不在频道，@李四 来",
            &members_fixture(),
            Some("agent-li"), // 李四自己发起，@李四 不触发
        );
        assert!(out.is_empty());
    }

    #[test]
    fn parse_dedup_preserves_order_and_caps() {
        let text = "@李四 @张三 @李四";
        let out = parse_agent_mentions(text, &members_fixture(), None);
        assert_eq!(out, vec!["agent-li".to_string(), "agent-zhang".to_string()]);
    }

    #[test]
    fn parse_ambiguous_same_name_different_ids_skipped() {
        // 两个不同成员同名「阿宝」→ 歧义不触发；同文本里 @李四 仍命中
        let members = vec![
            ("a1".into(), "阿宝".into()),
            ("a2".into(), "阿宝".into()),
            ("li".into(), "李四".into()),
        ];
        let out = parse_agent_mentions("@阿宝 和 @李四 讨论下", &members, None);
        assert_eq!(out, vec!["li".to_string()]);
    }

    // ---------- compose_channel_brief ----------

    #[test]
    fn brief_single_and_multi_backlog_wording() {
        let b1 = compose_channel_brief(&BriefSpec::UserBacklog { count: 1 });
        let s1 = ContentBlock::join_text(&b1);
        assert!(s1.contains("新消息") && !s1.contains("递进指示"));

        let b3 = compose_channel_brief(&BriefSpec::UserBacklog { count: 3 });
        let s3 = ContentBlock::join_text(&b3);
        assert!(s3.contains("3 条新消息") && s3.contains("递进指示"));
    }

    // ---------- 护栏内存态（构造驱动）----------

    #[test]
    fn wake_reserve_caps_within_window() {
        let mut rt = ChannelRuntime::default();
        let now = Instant::now();
        for _ in 0..MENTION_MAX_PER_MEMBER {
            assert!(wake_reserve(&mut rt, "m", now));
        }
        // 窗口内第 7 次 → 拒
        assert!(!wake_reserve(&mut rt, "m", now));
        // 窗口外（未来时刻）旧记录滑出 → 又可用
        assert!(wake_reserve(&mut rt, "m", now + MENTION_WINDOW + Duration::from_secs(1)));
        // 不同成员互不干扰
        assert!(wake_reserve(&mut rt, "other", now));
    }

    // ---------- 选举（match_candidate / tally_votes / streak 存储） ----------

    #[test]
    fn match_candidate_exact_then_unique_substring() {
        let members = vec![("a1".into(), "张三".into()), ("a2".into(), "李四".into())];
        // 精确名（trim 后）优先
        assert_eq!(match_candidate("张三", &members), Some("a1".into()));
        assert_eq!(match_candidate("  张三  ", &members), Some("a1".into()));
        // 唯一子串命中（模型回了句实话）
        assert_eq!(
            match_candidate("我投张三，他最靠谱", &members),
            Some("a1".into())
        );
        // 多命中 = 歧义弃权；零命中 = 弃权；「弃权」字面非成员名 → None
        assert_eq!(match_candidate("张三和李四都行", &members), None);
        assert_eq!(match_candidate("弃权", &members), None);
        assert_eq!(match_candidate("王五", &members), None);
    }

    #[test]
    fn tally_votes_winner_tie_and_all_abstain() {
        let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        // 普通多数
        let (t, w, tie) = tally_votes(
            &[Some("a".into()), Some("a".into()), Some("b".into())],
            &ids,
        );
        assert_eq!(w, Some("a".into()));
        assert!(!tie);
        assert_eq!(t.iter().find(|x| x.agent_id == "a").unwrap().votes, 2);
        // 平票 → 表序首个（joined_at 最早）
        let (_, w, tie) = tally_votes(&[Some("b".into()), Some("a".into()), None], &ids);
        assert_eq!(w, Some("a".into()));
        assert!(tie);
        // 全员弃权 → 胜者空缺（不自动重选熔断的事实基础）
        let (_, w, tie) = tally_votes(&[None, None, None], &ids);
        assert_eq!(w, None);
        assert!(!tie);
    }

    // ---------- 链头登记值（③ 生产实案回归锁） ----------

    /// NeedsCoordinator 臂头锚 = 积压最早一条：该臂未派发任何东西，选举后的
    /// 含头重消费面必须覆盖全部积压。生产实案 2026-09-11：头锚误取 last
    /// → 积压 2 条只消费 1 条，首条永久搁浅。
    #[test]
    fn election_register_head_is_earliest_backlog() {
        let anchor = |id: &str| crate::db::repo::message::TurnAnchor {
            message_id: id.to_string(),
            preview: String::new(),
            created_at: String::new(),
        };
        assert_eq!(
            election_register_head(&[anchor("m1"), anchor("m2"), anchor("m3")]),
            "m1"
        );
        // 单条积压：first == last，语义不变
        assert_eq!(election_register_head(&[anchor("only")]), "only");
    }

    #[test]
    fn coord_streak_survives_chain_finish_and_resets() {
        coord_streak_reset("conv-streak-test");
        assert_eq!(coord_streak_get("conv-streak-test"), 0);
        assert_eq!(coord_streak_inc("conv-streak-test"), 1);
        // finish_chain 清 runtime（链状态），streak 独立存储跨链存活
        finish_chain("conv-streak-test");
        assert_eq!(coord_streak_get("conv-streak-test"), 1);
        assert_eq!(coord_streak_inc("conv-streak-test"), 2);
        coord_streak_reset("conv-streak-test");
        assert_eq!(coord_streak_get("conv-streak-test"), 0);
    }

    // ---------- 链头派发状态机（consume_backlog 边界三档的输入） ----------

    /// consume_backlog 读 old_state 的同款锁读：None → DB 回退；Some((h,false))
    /// → 含头重消费（等选举落定/暂缓期间消息不搁浅）；Some((h,true)) → 严格
    /// 之后。生产实案 2026-09-10：旧实现无此区分且回退边界取错 → 首条广播
    /// 静默 no-op。
    #[test]
    fn head_dispatch_state_readback_and_default() {
        let conv = "conv-head-state-test";
        finish_chain(conv); // 清残留

        // 全新频道：无 runtime → old_state = None（DB 回退路径）
        let none = {
            let map = runtimes().lock().unwrap();
            map.get(conv)
                .and_then(|rt| rt.head_msg_id.clone().map(|h| (h, rt.head_dispatched)))
        };
        assert_eq!(none, None);

        // NeedsCoordinator 登记（仅 head=None 时初始化，Default 派生 dispatched=false）
        {
            let mut map = runtimes().lock().unwrap();
            let rt = map.entry(conv.to_string()).or_default();
            if rt.head_msg_id.is_none() {
                rt.head_msg_id = Some("m1".to_string());
            }
        }
        // 选举中 M2 重入不推进头（NeedsCoordinator 臂仅空缺时初始化）
        {
            let mut map = runtimes().lock().unwrap();
            let rt = map.entry(conv.to_string()).or_default();
            if rt.head_msg_id.is_none() {
                rt.head_msg_id = Some("m2".to_string());
            }
        }
        let registered = {
            let map = runtimes().lock().unwrap();
            map.get(conv)
                .and_then(|rt| rt.head_msg_id.clone().map(|h| (h, rt.head_dispatched)))
        };
        assert_eq!(
            registered,
            Some(("m1".to_string(), false)),
            "登记未派发：重消费须含头，且头不被选举期重入推进"
        );

        // run_next_hop 派发成功语义：置 true → 下次走严格之后
        {
            let mut map = runtimes().lock().unwrap();
            if let Some(rt) = map.get_mut(conv) {
                rt.head_dispatched = true;
            }
        }
        let dispatched = {
            let map = runtimes().lock().unwrap();
            map.get(conv)
                .and_then(|rt| rt.head_msg_id.clone().map(|h| (h, rt.head_dispatched)))
        };
        assert_eq!(dispatched, Some(("m1".to_string(), true)));

        finish_chain(conv); // 链终结：下次走 old_state=None → DB 回退，衔接正确
    }
}
