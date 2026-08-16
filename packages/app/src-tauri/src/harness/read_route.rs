//! 读路径路由（session-event-log Phase 2A）：事件日志转新会话的主读路径。
//!
//! Phase 0/1 让 `session_events` 影子表与 legacy 多表并存并完成对账（真机零 diff）。
//! 本模块把「事件回放」从**对账 B 侧**升级为**干净会话的历史加载源**——按会话路由：
//!
//! - **Derive**：有事件且对账零 diff、且无事件纪元前的 legacy 行（纯事件纪元会话）→
//!   从事件回放出 `Vec<MessageRow>`，走与 legacy 完全相同的下游 Pipeline。
//! - **Legacy**：其余全部（零事件旧会话 / 对账有 diff / 混合纪元 / 被偏好强制）→
//!   原路径不变。
//!
//! ## 为什么零风险
//!
//! - 派生出的 `MessageRow` 与 legacy 行**同构**（`content` + 序列化 `content_blocks`），
//!   锚回**真实 rowid**（`id_rowid_map`），下游 [`crate::context::history`]
//!   `load_history_with_window` 跑的是**同一函数同一输入**——reconcile 已证明两侧
//!   原始形态逐字节相等，故派生视图与 legacy 视图对干净会话**逐条相同**。
//! - 摘要连续性：`source_rowid` 取真 rowid，MemoryStage 的 `covered_until_rowid`
//!   按值定位在切换前后都能命中。
//! - legacy 拼装**永不删除**：它降级为 fallback，Derive 失效（出现 diff）的会话
//!   自动回退。这是"长期全绿"从「要等的闸门」变成「路由器维护的运行时属性」的关键。
//!
//! ## 路由判据（[`ReadRouteRegistry::resolve`]）
//!
//! 1. 偏好 `session_read_path = "legacy"` → 强制 Legacy（一键回滚）。
//! 2. `max_seq == 0`（零事件，pre-Phase-0 旧会话）→ Legacy。
//! 3. 对账 `diffs` 非空（某写路径漏发事件 / 行被外部改）→ Legacy。
//! 4. 对账 `skipped` 含 `legacy_epoch_rows`（事件纪元前有旧行，派生看不到它们）→
//!    Legacy（混合纪元会话）。
//! 5. 否则 → Derive。
//!
//! 缓存：以 `(max_seq, max_rowid)` 作会话数据指纹；指纹未变即直接复用上次决策，
//! 免去每轮全量对账（两个标量查询 vs 全表读 + 回放）。指纹追踪**新数据**（新事件涨
//! max_seq、新行涨 max_rowid）——生产事件 append-only、行 append-mostly，正常演进每轮
//! 都会触发重对账。原地篡改行内容（无新行/新事件，仅 buggy 写路径所为）不被指纹察觉，
//! 但活跃会话下一轮即刷新；休眠会话不被读取故无影响。**始终新鲜**的 ground truth 用
//! `reconcile_session` 命令（不受缓存影响，每次全量对账）。

use std::collections::HashMap;
use std::sync::RwLock;

use serde::Serialize;
use sqlx::SqlitePool;

use crate::db::models::MessageRow;
use crate::db::repo;
use crate::error::AppResult;
use crate::harness::derive::derive_history;
use crate::harness::reconcile::{reconcile_session, ReconcileReport};

/// 读路径选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadRoute {
    /// legacy 多表拼装（原路径，fallback）。
    Legacy,
    /// 事件回放派生（新会话的主读路径）。
    Derive,
}

/// 一次路由决策 + 人可读原因（日志/诊断用）。
#[derive(Debug, Clone, Serialize)]
pub struct RouteDecision {
    pub route: ReadRoute,
    /// 如 `green` / `no_events` / `reconcile_diffs:3` / `mixed_epoch` / `forced`。
    pub reason: String,
    /// 决策所依据对账报告的事件总数（零事件会话为 0）。
    pub events_total: usize,
    /// 对账 diff 数（Derive 必为 0）。
    pub diffs: usize,
}

impl RouteDecision {
    fn legacy(reason: impl Into<String>, events_total: usize, diffs: usize) -> Self {
        Self {
            route: ReadRoute::Legacy,
            reason: reason.into(),
            events_total,
            diffs,
        }
    }
    fn derive(events_total: usize) -> Self {
        Self {
            route: ReadRoute::Derive,
            reason: "green".into(),
            events_total,
            diffs: 0,
        }
    }
}

/// 缓存项：上次对账指纹 + 决策。
#[derive(Clone)]
struct CacheEntry {
    max_seq: i64,
    max_rowid: i64,
    decision: RouteDecision,
}

/// 诊断快照里的一行（[`ReadRouteRegistry::snapshot`]）。
#[derive(Debug, Clone, Serialize)]
pub struct RouteEntry {
    pub conversation_id: String,
    pub route: ReadRoute,
    pub reason: String,
    pub events_total: usize,
    pub diffs: usize,
}

/// `get_read_route_status` 命令返回体（session-events Phase 2A 诊断出口）。
#[derive(Debug, Clone, Serialize)]
pub struct ReadRouteStatus {
    /// 路由器缓存的所有会话条目（send_message 触发过的会话）。
    pub entries: Vec<RouteEntry>,
    /// 若命令传了 `conversation_id`，此处为其**当场解析**的决策（覆盖缓存）。
    pub resolved: Option<RouteDecision>,
}

/// 全局读路径路由缓存（注入 Tauri State）。
///
/// 无并发 await 持锁（缓存读写是纯内存瞬态操作），故用 `std::sync::RwLock`。
#[derive(Default)]
pub struct ReadRouteRegistry {
    cache: RwLock<HashMap<String, CacheEntry>>,
}

impl ReadRouteRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 命中缓存（指纹未变）则返回上次决策，否则 None。
    fn cached(&self, conv_id: &str, max_seq: i64, max_rowid: i64) -> Option<RouteDecision> {
        let cache = self.cache.read().ok()?;
        let e = cache.get(conv_id)?;
        if e.max_seq == max_seq && e.max_rowid == max_rowid {
            Some(e.decision.clone())
        } else {
            None
        }
    }

    fn put(&self, conv_id: &str, max_seq: i64, max_rowid: i64, decision: RouteDecision) {
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(
                conv_id.to_string(),
                CacheEntry {
                    max_seq,
                    max_rowid,
                    decision,
                },
            );
        }
    }

    /// 诊断出口：克隆当前所有缓存条目（路由器看过哪些会话、各走哪条路径、原因）。
    pub fn snapshot(&self) -> Vec<RouteEntry> {
        let cache = match self.cache.read() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        cache
            .iter()
            .map(|(conv_id, e)| RouteEntry {
                conversation_id: conv_id.clone(),
                route: e.decision.route,
                reason: e.decision.reason.clone(),
                events_total: e.decision.events_total,
                diffs: e.decision.diffs,
            })
            .collect()
    }

    /// 解析一个会话的读路径（带指纹缓存）。
    ///
    /// `force_legacy`：偏好 `session_read_path = "legacy"` 时为 true（一键回滚）。
    /// 首次或指纹变化时跑一次全量 [`reconcile_session`]（只读，安全）；之后命中缓存
    /// 仅花两个标量查询。
    pub async fn resolve(
        &self,
        pool: &SqlitePool,
        conversation_id: &str,
        force_legacy: bool,
    ) -> AppResult<RouteDecision> {
        if force_legacy {
            return Ok(RouteDecision::legacy("forced", 0, 0));
        }
        // 指纹：事件侧 max_seq + 行侧 max_rowid。任一变化才会重跑对账。
        let max_seq = repo::session_event::max_seq(pool, conversation_id).await?;
        let max_rowid = repo::message::max_rowid(pool, conversation_id).await?;

        if let Some(d) = self.cached(conversation_id, max_seq, max_rowid) {
            return Ok(d);
        }

        let decision = if max_seq == 0 {
            // 零事件：pre-Phase-0 旧会话，无事件可派生。
            RouteDecision::legacy("no_events", 0, 0)
        } else {
            let report = reconcile_session(pool, conversation_id).await?;
            let d = classify(&report);
            if !report.diffs.is_empty() {
                tracing::warn!(
                    target: "ice_paw.read_route",
                    conv = conversation_id,
                    diffs = report.diffs.len(),
                    "会话对账存在差异 → 回退 legacy 读路径（差异即 bug 嫌疑，见 reconcile_session）"
                );
            }
            d
        };

        tracing::info!(
            target: "ice_paw.read_route",
            conv = conversation_id,
            route = ?decision.route,
            reason = %decision.reason,
            events = decision.events_total,
            diffs = decision.diffs,
            "读路径路由决策"
        );
        self.put(conversation_id, max_seq, max_rowid, decision.clone());
        Ok(decision)
    }
}

/// 把对账报告分类为路由决策（纯函数，便于单测）。
fn classify(report: &ReconcileReport) -> RouteDecision {
    let diffs = report.diffs.len();
    if diffs > 0 {
        return RouteDecision::legacy(
            format!("reconcile_diffs:{diffs}"),
            report.events_total,
            diffs,
        );
    }
    // 混合纪元：事件纪元前还有 legacy 行（派生看不到它们，视图会丢历史）→ 必须留 legacy。
    let epoch_rows = report
        .skipped
        .iter()
        .find(|s| s.reason == "legacy_epoch_rows")
        .map(|s| s.count)
        .unwrap_or(0);
    if epoch_rows > 0 {
        return RouteDecision::legacy("mixed_epoch", report.events_total, 0);
    }
    RouteDecision::derive(report.events_total)
}

// =========================================================================
// 派生历史加载器：events → Vec<MessageRow>（与 legacy loader 同构输入）
// =========================================================================

/// 从事件回放派生 [`HistoryStage`](crate::context::stages::HistoryStage) 所需的
/// `Vec<MessageRow>`（Phase 2A 派生读路径）。
///
/// 流程：全量读事件 → `derive_history` 回放 → `id_rowid_map` 锚回真 rowid →
/// 转为 `MessageRow`（`content_blocks` 序列化，与 legacy 行 `parse_content_blocks`
/// 对称往返）→ tail-limit 到 [`HISTORY_LOAD_LIMIT`](repo::message::HISTORY_LOAD_LIMIT)
/// （与 legacy `list_by_conversation` 的窗口严格一致）。
///
/// 仅对路由判为 Derive 的会话调用（调用方负责）；其余走 legacy。
pub async fn load_history_from_events(
    pool: &SqlitePool,
    conversation_id: &str,
) -> AppResult<Vec<MessageRow>> {
    let events = repo::session_event::list_by_session(pool, conversation_id, None).await?;
    let derived = derive_history(&events);
    // 回放 issues：路由阶段已保证 Derive 会话零 diff（issues 进 DERIVE_ISSUE diff），
    // 这里若非空属异常（路由缓存失效的窄窗），仅 warn，不阻塞——派生出的部分仍可用。
    if !derived.issues.is_empty() {
        tracing::warn!(
            target: "ice_paw.read_route",
            conv = conversation_id,
            n = derived.issues.len(),
            "派生读路径遇到回放 issue（路由缓存可能已过期）；首个: {:?}",
            derived.issues.first()
        );
    }
    let rowid_map = repo::message::id_rowid_map(pool, conversation_id).await?;
    // 首现事件 created_at（message_id → 时间戳），填 MessageRow.created_at。
    let mut created_map: HashMap<String, String> = HashMap::new();
    for ev in &events {
        if let Some(mid) = &ev.message_id {
            created_map
                .entry(mid.clone())
                .or_insert_with(|| ev.created_at.clone());
        }
    }

    let mut rows: Vec<MessageRow> = derived
        .messages
        .iter()
        .map(|m| to_message_row(m, conversation_id, &rowid_map, &created_map))
        .collect();

    // tail-limit 与 legacy 对齐（最近 HISTORY_LOAD_LIMIT 条）。
    let limit = repo::message::HISTORY_LOAD_LIMIT as usize;
    if rows.len() > limit {
        let drop = rows.len() - limit;
        rows.drain(0..drop);
    }
    Ok(rows)
}

/// 把一条派生消息转为 [`MessageRow`]。
///
/// - `content_blocks` = 序列化 blocks（legacy 行经 `parse_content_blocks` 还原 → 严格对称）。
/// - `rowid` 取真值（`id_rowid_map`）；缺失以 `first_seq` 兜底（单调唯一，仅防异常，
///   Derive 路由下不应发生——发生则该会话已被 diff 判回 legacy）。
/// - `token_count` / `model` / `error` / `summary_id`：派生不携带（下游 Pipeline 不读这些字段）。
fn to_message_row(
    m: &crate::harness::derive::DerivedMessage,
    conversation_id: &str,
    rowid_map: &HashMap<String, i64>,
    created_map: &HashMap<String, String>,
) -> MessageRow {
    let rowid = rowid_map.get(&m.message_id).copied().unwrap_or(m.first_seq);
    let content_blocks = serde_json::to_string(&m.blocks).unwrap_or_else(|_| "[]".to_string());
    MessageRow {
        id: m.message_id.clone(),
        conversation_id: conversation_id.to_string(),
        role: m.role.clone(),
        content: m.content.clone(),
        content_blocks,
        token_count: None,
        error: None,
        created_at: created_map.get(&m.message_id).cloned().unwrap_or_default(),
        rowid,
        summary_id: None,
        model: None,
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::reconcile::{ReconcileDiff, ReconcileReport, ReconcileSkip};

    fn report(events: usize, diffs: usize, epoch: usize) -> ReconcileReport {
        ReconcileReport {
            conversation_id: "c".into(),
            events_total: events,
            turns_total: 1,
            turns_compared: 1,
            legacy_rows_total: 0,
            legacy_rows_compared: 0,
            derived_messages_compared: 0,
            diffs: vec![
                ReconcileDiff {
                    category: "MISSING_IN_DERIVED",
                    turn_id: None,
                    message_id: None,
                    detail: "x".into()
                };
                diffs
            ],
            skipped: if epoch > 0 {
                vec![ReconcileSkip {
                    reason: "legacy_epoch_rows",
                    count: epoch,
                }]
            } else {
                Vec::new()
            },
        }
    }

    #[test]
    fn classify_green_when_zero_diffs_no_epoch() {
        let r = report(127, 0, 0);
        let d = classify(&r);
        assert_eq!(d.route, ReadRoute::Derive);
        assert_eq!(d.reason, "green");
        assert_eq!(d.events_total, 127);
    }

    #[test]
    fn classify_legacy_when_diffs_present() {
        let r = report(40, 3, 0);
        let d = classify(&r);
        assert_eq!(d.route, ReadRoute::Legacy);
        assert_eq!(d.reason, "reconcile_diffs:3");
        assert_eq!(d.diffs, 3);
    }

    #[test]
    fn classify_legacy_when_mixed_epoch() {
        // 有事件但纪元前还有旧行 → 派生会丢历史，必须 legacy
        let r = report(30, 0, 12);
        let d = classify(&r);
        assert_eq!(d.route, ReadRoute::Legacy);
        assert_eq!(d.reason, "mixed_epoch");
    }

    #[test]
    fn registry_caches_decision_until_fingerprint_changes() {
        let reg = ReadRouteRegistry::new();
        let d = RouteDecision::derive(10);
        reg.put("c1", 5, 100, d.clone());
        // 指纹命中
        assert_eq!(reg.cached("c1", 5, 100).unwrap().route, ReadRoute::Derive);
        // 任一指纹变化 → miss
        assert!(reg.cached("c1", 6, 100).is_none(), "max_seq 变应 miss");
        assert!(reg.cached("c1", 5, 101).is_none(), "max_rowid 变应 miss");
        assert!(reg.cached("c2", 5, 100).is_none(), "其他会话 miss");
    }

    #[test]
    fn to_message_row_serializes_blocks_and_maps_rowid() {
        use crate::harness::derive::DerivedMessage;
        use crate::infra::protocol::ContentBlock;
        let m = DerivedMessage {
            message_id: "msg-1".into(),
            role: "assistant".into(),
            content: "你好".into(),
            blocks: vec![ContentBlock::text("你好")],
            turn_id: Some("t".into()),
            first_seq: 7,
            last_seq: 7,
        };
        let mut rowid_map = HashMap::new();
        rowid_map.insert("msg-1".into(), 42i64);
        let mut created_map = HashMap::new();
        created_map.insert("msg-1".into(), "2026-08-14T00:00:00Z".into());

        let row = to_message_row(&m, "conv", &rowid_map, &created_map);
        assert_eq!(row.id, "msg-1");
        assert_eq!(row.role, "assistant");
        assert_eq!(row.content, "你好");
        assert_eq!(row.rowid, 42, "取真 rowid");
        // content_blocks 是合法 JSON，parse 后还原
        let parsed = crate::context::history::parse_content_blocks(&row.content_blocks);
        assert_eq!(parsed.len(), 1);
        match &parsed[0] {
            ContentBlock::Text { text } => assert_eq!(text, "你好"),
            other => panic!("expected Text block, got {other:?}"),
        }
    }

    #[test]
    fn to_message_row_rowid_fallbacks_to_first_seq_when_unmapped() {
        use crate::harness::derive::DerivedMessage;
        use crate::infra::protocol::ContentBlock;
        let m = DerivedMessage {
            message_id: "msg-x".into(),
            role: "user".into(),
            content: "q".into(),
            blocks: vec![ContentBlock::text("q")],
            turn_id: None,
            first_seq: 9,
            last_seq: 9,
        };
        let empty_rowid: HashMap<String, i64> = HashMap::new();
        let empty_created: HashMap<String, String> = HashMap::new();
        let row = to_message_row(&m, "conv", &empty_rowid, &empty_created);
        assert_eq!(row.rowid, 9, "缺映射兜底 first_seq（单调唯一）");
    }

    // =====================================================================
    // 集成（in-crate，可触达 pub(crate) 的 load_history_with_window）
    // =====================================================================

    use crate::context::history::load_history_with_window;
    use crate::db::models::NewMessage;
    use crate::harness::event_log::{
        log_assistant_message, log_turn_context, log_turn_ended, log_user_message,
        AssistantMessagePayload, EventCtx, TurnContextPayload, TurnEndedPayload,
        UserMessagePayload,
    };
    use crate::infra::protocol::ContentBlock;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap()
    }

    async fn seeded_pool() -> SqlitePool {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref,
                 temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES ('a1', 't', 'anthropic', 'glm-5.2', '', '', 0.7, 1024, '{}', 0, 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES ('c1', 'a1', 't')")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn write_row(
        pool: &SqlitePool,
        id: &str,
        role: &str,
        content: &str,
        blocks: &[ContentBlock],
    ) {
        crate::db::repo::message::create(
            pool,
            id,
            &NewMessage {
                conversation_id: "c1".into(),
                role: role.into(),
                content: content.into(),
                token_count: None,
                error: None,
                model: None,
            },
        )
        .await
        .unwrap();
        let blocks_json = serde_json::to_string(blocks).unwrap();
        crate::db::repo::message::update_content_blocks(pool, id, &blocks_json)
            .await
            .unwrap();
    }

    /// 生产序一致脚本：user(text) → assistant(text+tool_use) → tool_result → assistant(终答)。
    async fn script(pool: &SqlitePool) {
        let ev = EventCtx::new("c1", "turn-1", "a1");
        let u = vec![ContentBlock::text("读一下 README")];
        write_row(pool, "turn-1", "user", "读一下 README", &u).await;
        log_user_message(
            pool,
            &ev,
            "turn-1",
            &UserMessagePayload {
                v: 1,
                content: "读一下 README".into(),
                blocks: u,
            },
        )
        .await;
        log_turn_context(
            pool,
            &ev,
            &TurnContextPayload {
                v: 1,
                provider: "anthropic".into(),
                effective_model: "glm-5.2".into(),
                model_override: None,
                tools_enabled: true,
                tool_names: vec!["read_file".into()],
                temperature: Some(0.7),
                max_tokens: Some(16384),
                tool_max_rounds: Some(12),
                budget_max_tokens: None,
                context_window: None,
            },
        )
        .await;
        let a1 = vec![
            ContentBlock::text("我来看看"),
            ContentBlock::ToolUse {
                id: "tu1".into(),
                name: "read_file".into(),
                input: "{\"path\":\"README.md\"}".into(),
            },
        ];
        write_row(pool, "m-a1", "assistant", "我来看看", &a1).await;
        log_assistant_message(
            pool,
            &ev,
            "m-a1",
            &AssistantMessagePayload {
                v: 1,
                model: Some("glm-5.2".into()),
                content: "我来看看".into(),
                blocks: a1,
                token_count: Some(12),
                duration_ms: Some(2_100),
                round: 0,
                continuation: false,
            },
        )
        .await;
        let tr = vec![ContentBlock::ToolResult {
            tool_use_id: "tu1".into(),
            content: "# IcePaw".into(),
            is_error: Some(false),
        }];
        write_row(pool, "m-tr", "user", "", &tr).await;
        crate::harness::event_log::log_tool_result_message(pool, &ev, "m-tr", &tr).await;
        let a2 = vec![ContentBlock::text("README 说这是本地优先工作站。")];
        write_row(
            pool,
            "m-a2",
            "assistant",
            "README 说这是本地优先工作站。",
            &a2,
        )
        .await;
        log_assistant_message(
            pool,
            &ev,
            "m-a2",
            &AssistantMessagePayload {
                v: 1,
                model: Some("glm-5.2".into()),
                content: "README 说这是本地优先工作站。".into(),
                blocks: a2,
                token_count: Some(20),
                duration_ms: Some(1_800),
                round: 1,
                continuation: false,
            },
        )
        .await;
        log_turn_ended(
            pool,
            &ev,
            Some("m-a2"),
            &TurnEndedPayload {
                v: 1,
                termination: "stop".into(),
                rounds: 2,
                usage: None,
                user_token_count: Some(3000),
            },
        )
        .await;
    }

    /// **核心不变式**：派生 MessageRows 经 legacy loader 的视图 == legacy 行经同一 loader 的视图。
    /// 读路径切换「零 diff」的直接证据：同库 → 同函数 → 同输出（含 source_rowid 摘要锚点）。
    #[tokio::test]
    async fn derived_view_matches_legacy_loader_view() {
        let pool = seeded_pool().await;
        script(&pool).await;

        let legacy_rows = crate::db::repo::message::list_all_by_rowid(&pool, "c1")
            .await
            .unwrap();
        let derived_rows = load_history_from_events(&pool, "c1").await.unwrap();

        let legacy_view = load_history_with_window(&legacy_rows, None);
        let derived_view = load_history_with_window(&derived_rows, None);

        assert_eq!(
            legacy_view.len(),
            derived_view.len(),
            "派生与 legacy 视图消息数应一致\nlegacy={:#?}\nderived={:#?}",
            legacy_view,
            derived_view
        );
        for (i, (l, d)) in legacy_view.iter().zip(derived_view.iter()).enumerate() {
            assert_eq!(l.role, d.role, "msg#{i} role 不一致");
            assert_eq!(
                l.content, d.content,
                "msg#{i} blocks 不一致\nlegacy={:?}\nderived={:?}",
                l.content, d.content
            );
            // source_rowid 必须一致——MemoryStage 摘要连续性依赖它（切换前后按值命中）
            assert_eq!(
                l.source_rowid, d.source_rowid,
                "msg#{i} source_rowid 不一致"
            );
        }
    }

    /// 路由：一致脚本 → Derive（green）；出现「有行无事件」的写路径漏 → 指纹变 → 重解析 → Legacy。
    ///
    /// 生产事件 append-only、行 append-mostly；指纹 `(max_seq, max_rowid)` 追踪**新数据**：
    /// 新事件涨 max_seq、新行涨 max_rowid，任一即触发重对账。本测试用「写了一条无对应事件
    /// 的行」（真实写路径漏的形态）制造 max_rowid 上涨 → 缓存失效 → 重解析命中
    /// MISSING_IN_DERIVED → 回退 Legacy。
    #[tokio::test]
    async fn resolve_green_then_legacy_after_divergence() {
        let pool = seeded_pool().await;
        let reg = ReadRouteRegistry::new();
        script(&pool).await;

        let d1 = reg.resolve(&pool, "c1", false).await.unwrap();
        assert_eq!(d1.route, ReadRoute::Derive);
        assert_eq!(d1.reason, "green");
        assert_eq!(d1.diffs, 0);

        // 缓存命中：指纹未变 → 第二次不重跑对账（仍 green）
        let d1b = reg.resolve(&pool, "c1", false).await.unwrap();
        assert_eq!(d1b.reason, "green");

        // 制造分叉：插一条无对应事件的行（写路径漏发事件的形态）→ max_rowid 涨 → 重解析
        write_row(
            &pool,
            "rogue-msg",
            "assistant",
            "幽灵回复",
            &[ContentBlock::text("幽灵回复")],
        )
        .await;
        let d2 = reg.resolve(&pool, "c1", false).await.unwrap();
        assert_eq!(
            d2.route,
            ReadRoute::Legacy,
            "有行无事件（写路径漏）应回退 legacy"
        );
        assert!(
            d2.reason.starts_with("reconcile_diffs"),
            "原因应反映 diff: {}",
            d2.reason
        );
    }

    /// 路由：零事件（pre-Phase-0 旧会话）→ Legacy（no_events），且不报错。
    #[tokio::test]
    async fn resolve_legacy_for_no_events_conversation() {
        let pool = seeded_pool().await;
        let reg = ReadRouteRegistry::new();
        // 只有行、零事件
        write_row(&pool, "m-u", "user", "hi", &[ContentBlock::text("hi")]).await;
        write_row(
            &pool,
            "m-a",
            "assistant",
            "hello",
            &[ContentBlock::text("hello")],
        )
        .await;

        let d = reg.resolve(&pool, "c1", false).await.unwrap();
        assert_eq!(d.route, ReadRoute::Legacy);
        assert_eq!(d.reason, "no_events");
    }

    /// 回滚开关：force_legacy 直接返回 Legacy，不查指纹/不跑对账。
    #[tokio::test]
    async fn force_legacy_short_circuits_resolve() {
        let pool = seeded_pool().await;
        let reg = ReadRouteRegistry::new();
        script(&pool).await;
        let d = reg.resolve(&pool, "c1", true).await.unwrap();
        assert_eq!(d.route, ReadRoute::Legacy);
        assert_eq!(d.reason, "forced");
    }
}
