//! 旧会话事件 backfill（session-event-log Phase 2B 前置，2026-08-17 设计定稿）。
//!
//! 给 Phase 0 之前就存在、只有 messages 行、零 session_events 的旧会话反向
//! 合成事件日志，使其获得与新生会话同等的 Derive 读路径资格——S1（legacy
//! 读路径退役）前的最后一块前置。
//!
//! ## 核心原则：backfill 是 reconcile 的逆函数
//!
//! 「行 → 事件」的合成与 reconcile A 侧的行提取共用同一套原语：同一
//! [`parse_content_blocks`]、同样的空回退对称（blocks 空 → `[Text(content)]`）、
//! 同样的容忍清单（error 行 / 空占位 / 摘要行 / tool 角色行不产消息事件）。
//! 构造性保证 backfill 后对账零 diff → [`read_route`](crate::harness::read_route)
//! 自动路由 Derive（判据 3/4 天然不触发，**零改动**）。就算形态判错，失败
//! 模式是 diff → 自动回退 Legacy——错也错得安全。
//!
//! ## 范围界定
//!
//! 只补「零事件」会话。混合纪元会话（有真实事件 + 纪元前旧行）不补：seq
//! 是 MAX+1 追加语义，合成事件只能落流尾，旧消息排到新消息之后 = 历史错序；
//! 治它得重排真实事件的 seq，违反 append-only。真机实证（08-15/16 日志 46
//! 次路由决策）混合纪元为 0，该形态不存在。
//!
//! ## 合成词表（与生产形态对齐）
//!
//! - user 行（非纯 ToolResult blocks）→ `user_message`，turn_id = row.id——
//!   生产不变式 turn_id==user_msg_id 对旧行天然成立（user 行 id 就是当轮
//!   user_msg_id），无需发明 id
//! - user 行（纯 ToolResult）→ `tool_result_message`（归属当前 turn）
//! - assistant 行（无 error、非空占位）→ `assistant_message`
//! - assistant 行（error 非空）→ `message_error`（kind="legacy"）
//! - 空占位 / 摘要行 / tool 角色行 / 非摘要 system 行 → 不合成（reconcile
//!   容忍清单内，或不合成就 diff → Legacy 安全降级）
//! - 每 turn 收尾 → `turn_ended`（termination=[`TERMINATION_BACKFILL`] 诚实
//!   标注合成来源；rounds=assistant 行数；usage=None 无从考）
//! - `turn_context` **不合成**——payload 要求 provider/model/工具快照，旧行
//!   没有，填当前 agent 配置等于伪造。核对过依赖面：derive 跳过该 kind、
//!   reconcile 只要求 turn_ended 在场、find_open_turns 以 turn_context 为键
//!   （不合成 → boot 崩溃扫尾永不误伤 backfill turn）、read_route 不读它。
//!
//! actor 统一 [`BACKFILL_ACTOR`]——幂等标记（重跑删自己重写）+ 轨迹里自带
//! 来源标注（这行是合成的，不是缺陷是特性）。created_at 直传行原始时间戳
//! （两表同为 SQLite datetime 文本格式），旧轨迹时间线不坍缩成 backfill 那一刻。
//!
//! ## 已知取舍
//!
//! - Image 块经 [`refify_blocks`] 换轻量引用（S1 阶段 3）——合成 payload 不落
//!   base64 双份；读侧水合见 derive/reconcile。版本 2 重跑前已 backfill 的
//!   会话保留 v1 内联形态（derive 照解 Full，读侧零迁移）。
//! - 首锚点前的孤儿行不合成（计入 report.epoch_rows）→ 该会话对账出现
//!   legacy_epoch_rows → 路由 mixed_epoch → Legacy（读路径与今天一致；合成
//!   出的事件仍服务轨迹视图）。
//! - backfill 绕过 `append_event` 走 repo 批量路径：显式 created_at、单会话
//!   事务、不触发 event_bus 广播（boot 时千条通知毫无意义）。

use serde::Serialize;
use sqlx::SqlitePool;

use crate::context::history::parse_content_blocks;
use crate::db::models::MessageRow;
use crate::db::repo::summary::SUMMARY_PREFIX;
use crate::db::repo::{self, session_event};
use crate::error::AppResult;
use crate::harness::event_log::{
    kind, refify_blocks, AssistantMessagePayload, MessageErrorPayload, ToolResultMessagePayload,
    TurnEndedPayload, UserMessagePayload,
};
use crate::infra::protocol::ContentBlock;

/// turn_ended 合成终态值：诚实标注「此轮无运行时观测，事件系事后合成」。
/// 刻意不入生产 termination 词表（stop/abort/...）——匹配器（如
/// delegate 的 is_normal_completion）不会误把它当真实观测值。
pub const TERMINATION_BACKFILL: &str = "backfill";

/// backfill 合成逻辑的版本号。
///
/// 版本升级（修 bug）时自增：boot 检测库内标记落后 → 把「纯 backfill 会话」
/// （零真实事件）一并纳入删旧重写，自愈无需 UI；冻结会话（已混入真实事件）
/// 永不重写（重写会造成错序，见模块头范围界定）。
pub const BACKFILL_VERSION: u32 = 1;

/// 库内版本标记的 preferences key（内部标记，经 [`repo::preferences::get`]
/// 原始读取，不进用户偏好 struct）。
const PREF_KEY: &str = "session_backfill_version";

/// boot 日志与测试断言用的执行汇总。
#[derive(Debug, Clone, Default, Serialize)]
pub struct BackfillReport {
    /// 本次待处理会话数（零事件 + 版本落后时的纯 backfill 会话）
    pub candidates: usize,
    /// 成功合成写入的会话数
    pub backfilled: usize,
    /// 写入事件总数
    pub events_written: usize,
    /// 合成 payload 总字节（Image 双份等磁盘计量）
    pub payload_bytes: u64,
    /// 合成失败跳过的会话数（意外形态；维持 no_events → Legacy）
    pub failed: usize,
    /// 首锚点前孤儿行总数（这些会话将路由 mixed_epoch → Legacy）
    pub epoch_rows: usize,
    /// 本次是否为版本落后的强制重跑
    pub forced: bool,
    /// 冻结会话数（backfill 行 + 真实事件混合，重跑永不触碰，仅诊断）
    pub frozen: usize,
}

/// 单会话合成结果。
struct SessionOutcome {
    events: usize,
    payload_bytes: u64,
    epoch_rows: usize,
}

/// boot 入口（幂等）：给全部零事件旧会话补合成事件；库内版本落后时把
/// 纯 backfill 会话一并删旧重写（版本化自愈）。
///
/// 定位与 sweep_interrupted_turns / heal_checksum_drift 同款 boot 自愈：
/// 单会话失败仅 warn 跳过，绝不阻断启动。
pub async fn backfill_legacy_sessions(pool: &SqlitePool) -> BackfillReport {
    // 版本标记：落后于代码版本 → 强制重跑（重写所有可重写的合成行）
    let stored = repo::preferences::get(pool, PREF_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let forced = stored < BACKFILL_VERSION;

    let mut convs = match session_event::find_zero_event_sessions(pool).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(target: "ice_paw.backfill", "候选会话查询失败（不影响启动）: {e}");
            return BackfillReport::default();
        }
    };
    if forced {
        match session_event::find_pure_backfill_sessions(pool).await {
            Ok(mut rerun) => convs.append(&mut rerun),
            Err(e) => {
                tracing::warn!(target: "ice_paw.backfill", "重跑候选查询失败（跳过重跑）: {e}")
            }
        }
    }
    let frozen = session_event::count_frozen_backfill_sessions(pool)
        .await
        .unwrap_or(0);

    let mut report = BackfillReport {
        candidates: convs.len(),
        forced,
        frozen,
        ..Default::default()
    };
    for conv_id in convs {
        match backfill_session(pool, &conv_id).await {
            Ok(out) => {
                report.backfilled += 1;
                report.events_written += out.events;
                report.payload_bytes += out.payload_bytes;
                report.epoch_rows += out.epoch_rows;
            }
            Err(e) => {
                report.failed += 1;
                tracing::warn!(
                    target: "ice_paw.backfill",
                    conv = %conv_id,
                    "单会话 backfill 失败（跳过，维持 legacy 读路径）: {e}"
                );
            }
        }
    }

    // 版本推进：全部成功才写（有失败保留旧版本 → 下次 boot 自动重试）。
    // frozen 不算失败——它是文档化的终态，不是待重试项。
    if forced {
        if report.failed == 0 {
            if let Err(e) =
                repo::preferences::set(pool, PREF_KEY, &BACKFILL_VERSION.to_string()).await
            {
                tracing::warn!(target: "ice_paw.backfill", "版本标记写入失败（下次 boot 会重跑）: {e}");
            }
        } else {
            tracing::warn!(
                target: "ice_paw.backfill",
                failed = report.failed,
                "存在失败会话，版本标记未推进（下次 boot 重试）"
            );
        }
    }
    report
}

/// 单会话：读行 → 合成 → 事务重写。任何一步 Err 由调用方记账。
async fn backfill_session(pool: &SqlitePool, conv_id: &str) -> AppResult<SessionOutcome> {
    let rows = repo::message::list_all_by_rowid(pool, conv_id).await?;
    let (events, epoch_rows) = synthesize_events(&rows);
    let payload_bytes = events.iter().map(|e| e.payload.len() as u64).sum();
    let n = session_event::rewrite_backfill_batch(pool, conv_id, events).await? as usize;
    Ok(SessionOutcome {
        events: n,
        payload_bytes,
        epoch_rows,
    })
}

/// 合成期间的 turn 累积器（收尾产 turn_ended）。
struct TurnState {
    id: String,
    /// assistant_message 事件数（turn_ended.rounds 口径，与 boot 扫尾同款近似）
    rounds: u32,
    /// turn 内最后一条 assistant 消息 id（turn_ended.message_id 用）
    last_message_id: Option<String>,
    /// turn 内最后一行的时间戳（turn_ended.created_at 用）
    last_created_at: String,
    /// 锚点 user 行的 token_count（真实数据白捡）
    user_token_count: Option<i32>,
}

/// 行 → 合成事件（纯函数，便于单测）。行序须为 rowid 升序（`list_all_by_rowid` 即此序）。
///
/// 返回 (事件列表, 首锚点前孤儿行数)。
fn synthesize_events(rows: &[MessageRow]) -> (Vec<session_event::BackfillEvent>, usize) {
    let mut events = Vec::new();
    let mut current_turn: Option<TurnState> = None;
    let mut epoch_rows = 0usize;

    for row in rows {
        // 与 reconcile A 侧同款跳过：摘要行（MemoryStage 唯一注入，不进 history）
        if row.role == "system" && row.content.starts_with(SUMMARY_PREFIX) {
            continue;
        }
        // tool 角色行（loader 也不进 history）；非摘要 system 行 derive 词表无法
        // 表达（生产不产生），不合成的结果是 MISSING_IN_DERIVED → Legacy，安全降级
        if !matches!(row.role.as_str(), "user" | "assistant") {
            continue;
        }
        let blocks = parse_content_blocks(&row.content_blocks);

        if row.role == "user" {
            if is_tool_result_row(&blocks) {
                // 工具结果行：归属当前 turn（生产形态 content 恒空、blocks 镜像）
                match current_turn.as_mut() {
                    Some(t) => {
                        events.push(backfill_event(
                            kind::TOOL_RESULT_MESSAGE,
                            Some(t.id.clone()),
                            Some(row.id.clone()),
                            &ToolResultMessagePayload {
                                v: 1,
                                blocks: refify_blocks(&row.id, &blocks),
                            },
                            &row.created_at,
                        ));
                        t.last_created_at = row.created_at.clone();
                    }
                    None => epoch_rows += 1,
                }
            } else {
                // turn 锚点：先闭合前一个 turn，再开新 turn
                close_turn(&mut events, current_turn.take());
                events.push(backfill_event(
                    kind::USER_MESSAGE,
                    Some(row.id.clone()),
                    Some(row.id.clone()),
                    &UserMessagePayload {
                        v: 1,
                        content: row.content.clone(),
                        // ref 化：Image 换轻量引用（v2 形态；3b 起 BACKFILL_VERSION=2
                        // 重跑后生效）。空回退产物是 Text，refify 原样过。
                        blocks: refify_blocks(&row.id, &effective_blocks(&row.content, &blocks)),
                    },
                    &row.created_at,
                ));
                current_turn = Some(TurnState {
                    id: row.id.clone(),
                    rounds: 0,
                    last_message_id: None,
                    last_created_at: row.created_at.clone(),
                    user_token_count: row.token_count,
                });
            }
            continue;
        }

        // assistant 行——归属当前 turn
        let Some(t) = current_turn.as_mut() else {
            epoch_rows += 1;
            continue;
        };
        if let Some(err) = row.error.as_deref() {
            // 错误行：镜像生产行为（error 行不产 assistant_message，只记错误事实）
            events.push(backfill_event(
                kind::MESSAGE_ERROR,
                Some(t.id.clone()),
                Some(row.id.clone()),
                &MessageErrorPayload {
                    v: 1,
                    kind: "legacy".into(),
                    error: err.to_string(),
                },
                &row.created_at,
            ));
        } else if is_empty_assistant_placeholder(row, &blocks) {
            // 空占位：不合成——维持 reconcile empty_placeholder 容忍（不洗白历史脏数据）
        } else {
            let round = t.rounds;
            events.push(backfill_event(
                kind::ASSISTANT_MESSAGE,
                Some(t.id.clone()),
                Some(row.id.clone()),
                &AssistantMessagePayload {
                    v: 1,
                    model: row.model.clone(),
                    content: row.content.clone(),
                    blocks: refify_blocks(&row.id, &effective_blocks(&row.content, &blocks)),
                    token_count: row.token_count.map(i64::from),
                    duration_ms: None,
                    round,
                    continuation: false,
                },
                &row.created_at,
            ));
            t.rounds += 1;
            t.last_message_id = Some(row.id.clone());
        }
        t.last_created_at = row.created_at.clone();
    }
    close_turn(&mut events, current_turn.take());
    (events, epoch_rows)
}

/// 闭合一个 turn：补记 turn_ended（termination=backfill）。
fn close_turn(events: &mut Vec<session_event::BackfillEvent>, t: Option<TurnState>) {
    let Some(t) = t else { return };
    events.push(backfill_event(
        kind::TURN_ENDED,
        Some(t.id),
        t.last_message_id,
        &TurnEndedPayload {
            v: 1,
            termination: TERMINATION_BACKFILL.to_string(),
            rounds: t.rounds,
            usage: None,
            user_token_count: t.user_token_count,
        },
        &t.last_created_at,
    ));
}

/// 构造一条合成事件。payload 序列化失败降级 "{}" → derive 记 DERIVE_ISSUE
/// → diff → Legacy（安全链；这些自有 struct 实际不会失败）。
fn backfill_event<T: Serialize>(
    kind_str: &str,
    turn_id: Option<String>,
    message_id: Option<String>,
    payload: &T,
    created_at: &str,
) -> session_event::BackfillEvent {
    session_event::BackfillEvent {
        kind: kind_str.to_string(),
        turn_id,
        message_id,
        payload: serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string()),
        created_at: created_at.to_string(),
    }
}

/// tool_result 行判定：user 角色 + blocks 非空且全为 ToolResult（生产写入形态：
/// content 恒空、每轮独立持久化）。误判的失败模式是 CONTENT_MISMATCH →
/// diff → Legacy，错得安全。
fn is_tool_result_row(blocks: &[ContentBlock]) -> bool {
    !blocks.is_empty()
        && blocks
            .iter()
            .all(|b| matches!(b, ContentBlock::ToolResult { .. }))
}

/// 与 reconcile `LegacyRow::is_empty_assistant_placeholder` 同款判定。
fn is_empty_assistant_placeholder(row: &MessageRow, blocks: &[ContentBlock]) -> bool {
    row.role == "assistant"
        && row.content.trim().is_empty()
        && blocks.is_empty()
        && row.error.is_none()
}

/// 与 reconcile `effective_blocks` 同构的空回退：blocks 空 → `[Text(content)]`。
/// 没有这层对称，derive（blocks 原样）与 A 侧（回退）会打出 CONTENT_MISMATCH。
fn effective_blocks(content: &str, blocks: &[ContentBlock]) -> Vec<ContentBlock> {
    if blocks.is_empty() {
        vec![ContentBlock::Text {
            text: content.to_string(),
        }]
    } else {
        blocks.to_vec()
    }
}

// =========================================================================
// 测试：合成 → 对账零 diff → 路由 Derive 的构造性验证
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::history::load_history_with_window;
    use crate::harness::read_route::{load_history_from_events, ReadRoute, ReadRouteRegistry};
    use crate::harness::reconcile::reconcile_session;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    /// 注：in-memory SQLite 每连接各一个库，pool 必须 max_connections(1)。
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

    async fn migrated_pool() -> SqlitePool {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref,
                 temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES ('agent-1', 't', 'anthropic', 'm', '', '', 0.7, 1024, '{}', 0, 0)",
        )
        .execute(&pool)
        .await
        .expect("seed agent");
        pool
    }

    async fn seed_conv(pool: &SqlitePool, conv_id: &str) {
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES (?, 'agent-1', 't')")
            .bind(conv_id)
            .execute(pool)
            .await
            .expect("seed conversation");
    }

    /// 插入一条 legacy 行（显式 created_at——保真断言的基准值）。
    async fn insert_row(
        pool: &SqlitePool,
        conv: &str,
        id: &str,
        role: &str,
        content: &str,
        blocks_json: &str,
        created_at: &str,
    ) {
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, role, content, content_blocks, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(conv)
        .bind(role)
        .bind(content)
        .bind(blocks_json)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("insert legacy row");
    }

    /// 全形态 legacy 夹具（conv-bf，3 个 turn + 全部特殊行形态）：
    ///
    /// - t1=u1：文本问答 + 工具轮（assistant[ToolUse] + user[ToolResult]）
    /// - t2=u2：error 行 + 空占位 + 摘要行 + tool 角色行（全部不产消息事件）
    /// - t3=u3：附件块 user 行 + 终答（含 model / token_count 真实数据）
    async fn seed_full_shape_legacy(pool: &SqlitePool) {
        seed_conv(pool, "conv-bf").await;
        let prefix = SUMMARY_PREFIX;
        insert_row(
            pool,
            "conv-bf",
            "u1",
            "user",
            "你好",
            r#"[{"type":"text","text":"你好"}]"#,
            "2026-08-01 10:00:00",
        )
        .await;
        insert_row(
            pool,
            "conv-bf",
            "a1",
            "assistant",
            "你好！有什么可以帮你？",
            r#"[{"type":"text","text":"你好！有什么可以帮你？"}]"#,
            "2026-08-01 10:00:05",
        )
        .await;
        insert_row(
            pool,
            "conv-bf",
            "tr1",
            "user",
            "",
            r#"[{"type":"tool_result","tool_use_id":"tu_1","content":"内容","is_error":false}]"#,
            "2026-08-01 10:00:10",
        )
        .await;
        insert_row(
            pool,
            "conv-bf",
            "a2",
            "assistant",
            "",
            r#"[{"type":"tool_use","id":"tu_1","name":"read_file","input":"{}"}]"#,
            "2026-08-01 10:00:15",
        )
        .await;
        insert_row(
            pool,
            "conv-bf",
            "u2",
            "user",
            "继续",
            r#"[{"type":"text","text":"继续"}]"#,
            "2026-08-01 10:00:20",
        )
        .await;
        insert_row(
            pool,
            "conv-bf",
            "a3",
            "assistant",
            "",
            "[]",
            "2026-08-01 10:00:25",
        )
        .await;
        sqlx::query("UPDATE messages SET error = 'boom' WHERE id = 'a3'")
            .execute(pool)
            .await
            .expect("set error");
        insert_row(
            pool,
            "conv-bf",
            "a4",
            "assistant",
            "",
            "[]",
            "2026-08-01 10:00:30",
        )
        .await;
        insert_row(
            pool,
            "conv-bf",
            "sum1",
            "system",
            &format!("{prefix}\n摘要正文"),
            "[]",
            "2026-08-01 10:00:31",
        )
        .await;
        insert_row(
            pool,
            "conv-bf",
            "tool1",
            "tool",
            "原始工具输出",
            "[]",
            "2026-08-01 10:00:32",
        )
        .await;
        insert_row(pool, "conv-bf", "u3", "user", "看附件", r#"[{"type":"text","text":"看附件"},{"type":"attachment","name":"plan.pdf","kind":"pdf","size":282000}]"#, "2026-08-01 10:00:40").await;
        insert_row(
            pool,
            "conv-bf",
            "a5",
            "assistant",
            "收到",
            r#"[{"type":"text","text":"收到"}]"#,
            "2026-08-01 10:00:45",
        )
        .await;
        sqlx::query("UPDATE messages SET token_count = 120, model = 'glm-5.2' WHERE id = 'u1'")
            .execute(pool)
            .await
            .expect("set u1 token_count");
        sqlx::query("UPDATE messages SET token_count = 42, model = 'glm-5.2' WHERE id = 'a1'")
            .execute(pool)
            .await
            .expect("set a1 token/model");
    }

    async fn events_of(pool: &SqlitePool, conv: &str) -> Vec<crate::db::models::SessionEventRow> {
        session_event::list_by_session(pool, conv, None)
            .await
            .expect("list events")
    }

    fn kinds_of(rs: &[crate::db::models::SessionEventRow]) -> Vec<&str> {
        rs.iter().map(|r| r.kind.as_str()).collect()
    }

    /// 测试 1（核心）：全形态夹具 → backfill → 对账零 diff + 路由 Derive(green)。
    ///
    /// 这条测试是「backfill 是 reconcile 的逆函数」的直接证据：合成规则与
    /// A 侧提取同源 → 构造性零 diff → read_route 判据放行。
    #[tokio::test]
    async fn full_shape_backfill_reconciles_zero_diff_and_routes_derive() {
        let pool = migrated_pool().await;
        seed_full_shape_legacy(&pool).await;

        let report = backfill_legacy_sessions(&pool).await;
        assert_eq!(report.candidates, 1);
        assert_eq!(report.backfilled, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(report.epoch_rows, 0);
        assert_eq!(report.events_written, 11);

        // 事件形态：kind 序 + seq 严格连续 + actor 全 backfill
        let events = events_of(&pool, "conv-bf").await;
        assert_eq!(
            kinds_of(&events),
            vec![
                "user_message",        // u1
                "assistant_message",   // a1
                "tool_result_message", // tr1
                "assistant_message",   // a2
                "turn_ended",          // t1 闭合（rounds=2）
                "user_message",        // u2
                "message_error",       // a3（error 行不产 assistant_message）
                "turn_ended",          // t2 闭合（rounds=0；a4 空占位/摘要/tool 行不合成）
                "user_message",        // u3
                "assistant_message",   // a5
                "turn_ended",          // t3 闭合
            ]
        );
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.seq, (i + 1) as i64, "seq 应从 1 严格连续");
            assert_eq!(e.actor, session_event::BACKFILL_ACTOR);
        }
        // turn 归属
        assert_eq!(events[0].turn_id.as_deref(), Some("u1"));
        assert_eq!(events[4].turn_id.as_deref(), Some("u1"));
        assert_eq!(events[7].turn_id.as_deref(), Some("u2"));
        assert_eq!(events[10].turn_id.as_deref(), Some("u3"));
        // turn_ended payload：termination/rounds/user_token_count（u1 行真实数据）
        let te1: TurnEndedPayload = serde_json::from_str(&events[4].payload).unwrap();
        assert_eq!(te1.termination, TERMINATION_BACKFILL);
        assert_eq!(te1.rounds, 2);
        assert_eq!(te1.user_token_count, Some(120));
        assert!(te1.usage.is_none());
        // assistant payload：model / token_count 白捡
        let a1: AssistantMessagePayload = serde_json::from_str(&events[1].payload).unwrap();
        assert_eq!(a1.model.as_deref(), Some("glm-5.2"));
        assert_eq!(a1.token_count, Some(42));
        assert_eq!(a1.round, 0);
        // created_at 保真（行原始时间戳直传）
        assert_eq!(events[0].created_at, "2026-08-01 10:00:00");
        assert_eq!(
            events[4].created_at, "2026-08-01 10:00:15",
            "turn_ended 取 turn 内最后行时间"
        );

        // 对账零 diff + 容忍清单齐全 + 无 epoch 行
        let rep = reconcile_session(&pool, "conv-bf").await.unwrap();
        assert!(rep.diffs.is_empty(), "diffs: {:#?}", rep.diffs);
        assert!(
            !rep.skipped.iter().any(|s| s.reason == "legacy_epoch_rows"),
            "skipped: {:?}",
            rep.skipped
        );
        for reason in [
            "error_row",
            "empty_placeholder",
            "summary_row",
            "non_conversational_role",
        ] {
            assert!(
                rep.skipped.iter().any(|s| s.reason == reason),
                "容忍项 {reason} 应在 skipped: {:?}",
                rep.skipped
            );
        }
        assert_eq!(rep.turns_compared, 3);

        // 路由：Derive green（零 diff + 无 epoch 行 → 现有判据直接放行）
        let reg = ReadRouteRegistry::new();
        let d = reg.resolve(&pool, "conv-bf").await.unwrap();
        assert_eq!(d.route, ReadRoute::Derive);
        assert_eq!(d.reason, "green");
    }

    /// 测试 2：视图等价——backfill 后派生历史与 legacy 行经同一 loader 产出
    /// 逐条相同（含 source_rowid 摘要锚点）。镜像 read_route 的核心不变式测试。
    #[tokio::test]
    async fn derived_view_matches_legacy_loader_view_after_backfill() {
        let pool = migrated_pool().await;
        seed_full_shape_legacy(&pool).await;
        backfill_legacy_sessions(&pool).await;

        let legacy_rows = repo::message::list_all_by_rowid(&pool, "conv-bf")
            .await
            .unwrap();
        let derived_rows = load_history_from_events(&pool, "conv-bf").await.unwrap();

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
            assert_eq!(l.content, d.content, "msg#{i} blocks 不一致");
            assert_eq!(
                l.source_rowid, d.source_rowid,
                "msg#{i} source_rowid 不一致"
            );
        }
    }

    /// 测试 3：幂等——已 backfill 会话不再进候选（零事件资格已消耗）。
    #[tokio::test]
    async fn backfill_is_idempotent_across_runs() {
        let pool = migrated_pool().await;
        seed_full_shape_legacy(&pool).await;

        let r1 = backfill_legacy_sessions(&pool).await;
        assert_eq!(r1.backfilled, 1);
        let events_after_first = events_of(&pool, "conv-bf").await;

        let r2 = backfill_legacy_sessions(&pool).await;
        assert_eq!(r2.candidates, 0, "有事件会话不再进候选");
        assert_eq!(r2.backfilled, 0);

        let events_after_second = events_of(&pool, "conv-bf").await;
        assert_eq!(
            kinds_of(&events_after_first),
            kinds_of(&events_after_second)
        );
    }

    /// 测试 4：混合纪元会话（真实事件 + 旧行）不被触碰。
    #[tokio::test]
    async fn mixed_epoch_conversation_is_untouched() {
        let pool = migrated_pool().await;
        seed_conv(&pool, "conv-mix").await;
        // 纪元前旧行 + 真实事件（升级前后继续聊的形态）
        insert_row(
            &pool,
            "conv-mix",
            "old-u",
            "user",
            "旧问题",
            r#"[{"type":"text","text":"旧问题"}]"#,
            "2026-07-01 09:00:00",
        )
        .await;
        insert_row(
            &pool,
            "conv-mix",
            "new-u",
            "user",
            "新问题",
            r#"[{"type":"text","text":"新问题"}]"#,
            "2026-08-10 09:00:00",
        )
        .await;
        session_event::append(
            &pool,
            "conv-mix",
            kind::USER_MESSAGE,
            "user",
            Some("new-u"),
            Some("new-u"),
            r#"{"v":1,"content":"新问题","blocks":[{"type":"text","text":"新问题"}]}"#,
        )
        .await
        .unwrap();

        let report = backfill_legacy_sessions(&pool).await;
        assert_eq!(report.candidates, 0, "有真实事件的会话不进候选");
        assert_eq!(report.backfilled, 0);

        let events = events_of(&pool, "conv-mix").await;
        assert_eq!(events.len(), 1, "真实事件原样，零合成");
        assert_eq!(events[0].actor, "user");
    }

    /// 测试 5：首锚点前孤儿行 → 不合成 + 计数 + 该会话路由 mixed_epoch → Legacy
    /// （安全降级：读路径行为与 backfill 前完全一致）。
    #[tokio::test]
    async fn orphan_rows_before_first_anchor_degrade_to_legacy() {
        let pool = migrated_pool().await;
        seed_conv(&pool, "conv-orphan").await;
        // 病理形态：assistant 行先于任何 user 锚点（生产 send_message 不会产生，
        // 防御旧库手工修改等异常数据）
        insert_row(
            &pool,
            "conv-orphan",
            "ghost",
            "assistant",
            "孤儿回复",
            r#"[{"type":"text","text":"孤儿回复"}]"#,
            "2026-08-01 10:00:00",
        )
        .await;
        insert_row(
            &pool,
            "conv-orphan",
            "u1",
            "user",
            "你好",
            r#"[{"type":"text","text":"你好"}]"#,
            "2026-08-01 10:00:05",
        )
        .await;
        insert_row(
            &pool,
            "conv-orphan",
            "a1",
            "assistant",
            "你好！",
            r#"[{"type":"text","text":"你好！"}]"#,
            "2026-08-01 10:00:10",
        )
        .await;

        let report = backfill_legacy_sessions(&pool).await;
        assert_eq!(report.backfilled, 1);
        assert_eq!(report.epoch_rows, 1, "孤儿行被计数跳过");

        let rep = reconcile_session(&pool, "conv-orphan").await.unwrap();
        assert!(
            rep.skipped.iter().any(|s| s.reason == "legacy_epoch_rows"),
            "孤儿行进 epoch 容忍: {:?}",
            rep.skipped
        );
        let reg = ReadRouteRegistry::new();
        let d = reg.resolve(&pool, "conv-orphan").await.unwrap();
        assert_eq!(d.route, ReadRoute::Legacy);
        assert_eq!(d.reason, "mixed_epoch");
    }

    /// 测试 6：版本化重跑——库内版本落后时纯 backfill 会话被删旧重写，
    /// 成功后版本标记推进（修 bug 自愈闭环，无需 UI）。
    #[tokio::test]
    async fn versioned_rerun_rewrites_pure_backfill_sessions() {
        let pool = migrated_pool().await;
        seed_full_shape_legacy(&pool).await;
        backfill_legacy_sessions(&pool).await;

        // 首跑即 forced（stored 缺省 0 < 1）→ 版本标记已推进
        let v = repo::preferences::get(&pool, "session_backfill_version")
            .await
            .unwrap();
        assert_eq!(v, Some(BACKFILL_VERSION.to_string()));

        // 模拟 v1 合成 bug：污染一条合成行（对账会出 DERIVE_ISSUE）+ 版本标记回落
        sqlx::query(
            "UPDATE session_events SET payload = 'garbage'
              WHERE actor = 'backfill' AND message_id = 'a1'",
        )
        .execute(&pool)
        .await
        .unwrap();
        repo::preferences::set(&pool, "session_backfill_version", "0")
            .await
            .unwrap();

        let r = backfill_legacy_sessions(&pool).await;
        assert!(r.forced);
        assert_eq!(r.candidates, 1, "纯 backfill 会话进重跑候选");
        assert_eq!(r.backfilled, 1);
        assert_eq!(r.failed, 0);

        // 污染被重写治愈：对账零 diff、路由 Derive、版本再次推进
        let rep = reconcile_session(&pool, "conv-bf").await.unwrap();
        assert!(rep.diffs.is_empty(), "diffs: {:#?}", rep.diffs);
        let reg = ReadRouteRegistry::new();
        assert_eq!(
            reg.resolve(&pool, "conv-bf").await.unwrap().route,
            ReadRoute::Derive
        );
        let v2 = repo::preferences::get(&pool, "session_backfill_version")
            .await
            .unwrap();
        assert_eq!(v2, Some(BACKFILL_VERSION.to_string()));
    }

    /// 测试 7：冻结规则——backfill 后用户又聊过（混入真实事件）的会话
    /// 永不重写（重写会把合成事件追到流尾造成错序），即使版本落后。
    #[tokio::test]
    async fn frozen_sessions_with_real_events_are_never_rerun() {
        let pool = migrated_pool().await;
        seed_full_shape_legacy(&pool).await;
        backfill_legacy_sessions(&pool).await;

        // 用户又聊了一轮：行 + 真实事件（追加在合成事件之后，时序天然正确）
        insert_row(
            &pool,
            "conv-bf",
            "u9",
            "user",
            "追加问题",
            r#"[{"type":"text","text":"追加问题"}]"#,
            "2026-08-02 09:00:00",
        )
        .await;
        session_event::append(
            &pool,
            "conv-bf",
            kind::USER_MESSAGE,
            "user",
            Some("u9"),
            Some("u9"),
            r#"{"v":1,"content":"追加问题","blocks":[{"type":"text","text":"追加问题"}]}"#,
        )
        .await
        .unwrap();

        // 版本落后触发强制重跑 → 冻结会话不在候选
        repo::preferences::set(&pool, "session_backfill_version", "0")
            .await
            .unwrap();
        let r = backfill_legacy_sessions(&pool).await;
        assert!(r.forced);
        assert_eq!(r.candidates, 0, "冻结会话不进重跑候选");
        assert_eq!(r.backfilled, 0);
        assert_eq!(r.frozen, 1);

        // 事件原样：12 条 = 11 合成 + 1 真实，未被删改
        let events = events_of(&pool, "conv-bf").await;
        assert_eq!(events.len(), 12);
        assert_eq!(events[11].actor, "user", "真实事件原样保留");
        // 版本仍推进——frozen 是文档化终态，不是待重试失败
        let v = repo::preferences::get(&pool, "session_backfill_version")
            .await
            .unwrap();
        assert_eq!(v, Some(BACKFILL_VERSION.to_string()));
    }
}
