//! session-events 对账器（Phase 1）：derive-on-read diff，差异即 bug 清单。
//!
//! A 侧 = legacy 行提取（messages 表按 rowid 全量读，`parse_content_blocks`
//! 还原原始 blocks，**不跑** sanitize/split/window——投影变换会抹掉真差异）；
//! B 侧 = [`crate::harness::derive::derive_history`] 事件回放。两侧按
//! message_id 对齐，按 turn 分组截断。
//!
//! ## 差异类别（diffs）
//! - `MISSING_IN_DERIVED`：行有事件无——某持久化路径没发事件（真 bug 嫌疑）
//! - `MISSING_IN_LEGACY`：事件有行无——行被删/未写
//! - `CONTENT_MISMATCH`：role / content / blocks 任一不等
//! - `ORDER_MISMATCH`：turn 内事件首现序 ≠ 行 rowid 序
//! - `DERIVE_ISSUE`：事件 payload 损坏 / 未知 kind / 缺 message_id
//!
//! ## 预期差异（skipped，不算 bug——均已文档化）
//! - `pre_phase0_no_events`：migration 44 之前的会话（零事件，无从对账）
//! - `legacy_epoch_rows`：事件纪元之前的行（首个事件 turn 之前）
//! - `incomplete_turn`：无 turn_ended 的 turn（崩溃/强杀，Phase 0 已知缺口）
//! - `error_row`：error 非空的行（内容空，LLM 双侧不可见；message_error 事件在场）
//! - `discarded_row`：有 message_discarded/message_error 事件的 message_id 行不存在
//!   （终止守卫删占位；cancel_top 场景行保留为空）
//! - `empty_placeholder`：空 assistant 占位（content+blocks 全空、无 error 无事件）
//!   ——step-0 修复前的历史数据容忍项，新数据不应再出现
//! - `summary_row` / `non_conversational_role`：摘要行 / tool 角色行（双侧本就不进上下文）
//! - `derived_unmapped_turn`：回放消息无法归属任何事件 turn（异常，待观察）
//!
//! 完成判据：真实库对账 diffs 为空、skipped 全部可解释。

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use sqlx::SqlitePool;

use crate::context::history::parse_content_blocks;
use crate::db::models::MessageRow;
use crate::db::repo;
use crate::db::repo::summary::SUMMARY_PREFIX;
use crate::error::AppResult;
use crate::harness::derive::{derive_history, hydrate_image_refs, DerivedMessage};
use crate::harness::event_log::PayloadBlock;
use crate::infra::protocol::ContentBlock;

/// 对账报告（`reconcile_session` 命令直接序列化返回）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReconcileReport {
    pub conversation_id: String,
    pub events_total: usize,
    /// 事件流中出现的 turn 数（按 turn_id 首现去重）
    pub turns_total: usize,
    /// 参与比对的 turn 数（有 turn_ended 的完整 turn）
    pub turns_compared: usize,
    /// legacy 行总数（含跳过行）
    pub legacy_rows_total: usize,
    /// 参与比对的 legacy 行数
    pub legacy_rows_compared: usize,
    /// 参与比对的回放消息数
    pub derived_messages_compared: usize,
    pub diffs: Vec<ReconcileDiff>,
    pub skipped: Vec<ReconcileSkip>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReconcileDiff {
    pub category: &'static str,
    pub turn_id: Option<String>,
    pub message_id: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReconcileSkip {
    pub reason: &'static str,
    pub count: usize,
}

/// 对账入口：一个会话的 legacy 行提取 vs 事件回放，按 message_id 对齐。
pub async fn reconcile_session(
    pool: &SqlitePool,
    conversation_id: &str,
) -> AppResult<ReconcileReport> {
    let events = repo::session_event::list_by_session(pool, conversation_id, None).await?;
    let rows = repo::message::list_all_by_rowid(pool, conversation_id).await?;

    let mut report = ReconcileReport {
        conversation_id: conversation_id.to_string(),
        events_total: events.len(),
        turns_total: 0,
        turns_compared: 0,
        legacy_rows_total: rows.len(),
        legacy_rows_compared: 0,
        derived_messages_compared: 0,
        diffs: Vec::new(),
        skipped: Vec::new(),
    };
    let mut skip = SkipCounter::new(&mut report.skipped);

    if events.is_empty() {
        // migration 44 之前的会话：零事件，整会话无从对账
        skip.add("pre_phase0_no_events", rows.len());
        return Ok(report);
    }

    // B 侧：事件回放（issues 直接进 diffs——事件侧异常必须可见）
    let mut derived = derive_history(&events);
    for issue in &derived.issues {
        report.diffs.push(ReconcileDiff {
            category: "DERIVE_ISSUE",
            turn_id: None,
            message_id: None,
            detail: format!("seq={} kind={} {}", issue.seq, issue.kind, issue.reason),
        });
    }

    // Image 引用水合（S1 阶段 3）：ref 就地还原为所指行 `content_blocks` 下标的
    // 完整块。resolver 只为 ref 指向的行 parse（图片少的会话零开销）。ref 本就
    // 指向 A 侧同一行——水合后与行侧逐字节相等是构造性的，平面等式保持。
    // 未命中（行已删 / 下标变形）记 warn；随后 compare_content 自然给出
    // CONTENT_MISMATCH（marker ≠ 行内 Image），不会静默吞。
    let ref_ids: HashSet<String> = derived
        .messages
        .iter()
        .flat_map(|m| m.blocks.iter().filter_map(|b| match b {
            PayloadBlock::ImageRef { message_id, .. } => Some(message_id.clone()),
            _ => None,
        }))
        .collect();
    if !ref_ids.is_empty() {
        let index: HashMap<String, Vec<ContentBlock>> = rows
            .iter()
            .filter(|r| ref_ids.contains(r.id.as_str()))
            .map(|r| (r.id.clone(), parse_content_blocks(&r.content_blocks)))
            .collect();
        let missed = hydrate_image_refs(&mut derived.messages, &|mid, idx| {
            index.get(mid).and_then(|bs| bs.get(idx)).cloned()
        });
        if missed > 0 {
            tracing::warn!(
                target: "ice_paw.reconcile",
                "Image 引用水合未命中 {missed} 处（行已删 / content_blocks 变形），将以 CONTENT_MISMATCH 上报"
            );
        }
    }

    // turn 集合：首现序 + 完整性（有 turn_ended）
    let mut turn_order: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for ev in &events {
        if let Some(t) = &ev.turn_id {
            if seen.insert(t.clone()) {
                turn_order.push(t.clone());
            }
        }
    }
    let ended_turns: HashSet<&str> = events
        .iter()
        .filter(|e| e.kind == "turn_ended")
        .filter_map(|e| e.turn_id.as_deref())
        .collect();
    // 本 turn 内「行可以不存在」的 message_id（终止守卫删占位 / cancel_top 占位 / 错误行）
    let mut invisible_ids_by_turn: HashMap<&str, HashSet<&str>> = HashMap::new();
    for ev in &events {
        if matches!(ev.kind.as_str(), "message_error" | "message_discarded") {
            if let (Some(t), Some(mid)) = (ev.turn_id.as_deref(), ev.message_id.as_deref()) {
                invisible_ids_by_turn.entry(t).or_default().insert(mid);
            }
        }
    }
    report.turns_total = turn_order.len();

    // A 侧：legacy 行提取 + 行→turn 归属（锚点 = 该 turn 的 user 行；其后的行
    // 顺次归属，直到下一个事件 turn 锚点。锚点前的行 = 事件纪元之前）。
    let evented_turn_ids: HashSet<&str> = turn_order.iter().map(|s| s.as_str()).collect();
    let mut legacy_by_turn: HashMap<String, Vec<LegacyRow>> = HashMap::new();
    let mut current_turn: Option<String> = None;
    for row in rows {
        if row.role == "system" && row.content.starts_with(SUMMARY_PREFIX) {
            skip.add("summary_row", 1);
            continue;
        }
        if !matches!(row.role.as_str(), "user" | "assistant" | "system") {
            skip.add("non_conversational_role", 1);
            continue;
        }
        if row.role == "user" && evented_turn_ids.contains(row.id.as_str()) {
            current_turn = Some(row.id.clone());
        }
        let raw_blocks = parse_content_blocks(&row.content_blocks);
        let entry = LegacyRow {
            row,
            blocks: raw_blocks,
        };
        match &current_turn {
            Some(t) => legacy_by_turn.entry(t.clone()).or_default().push(entry),
            None => skip.add("legacy_epoch_rows", 1),
        }
    }

    // 逐 turn 比对（只比完整 turn；不完整 turn 的两侧行数计入 skipped）
    for turn in &turn_order {
        let legacy_rows = legacy_by_turn.get(turn).cloned().unwrap_or_default();
        if !ended_turns.contains(turn.as_str()) {
            skip.add("incomplete_turn", 1);
            skip.add("incomplete_turn_legacy_rows", legacy_rows.len());
            continue;
        }
        report.turns_compared += 1;
        report.legacy_rows_compared += legacy_rows.len();
        let derived_msgs: Vec<&DerivedMessage> = derived
            .messages
            .iter()
            .filter(|m| m.turn_id.as_deref() == Some(turn.as_str()))
            .collect();
        report.derived_messages_compared += derived_msgs.len();
        let invisible = invisible_ids_by_turn
            .get(turn.as_str())
            .cloned()
            .unwrap_or_default();

        let legacy_map: HashMap<&str, &LegacyRow> =
            legacy_rows.iter().map(|e| (e.row.id.as_str(), e)).collect();
        let derived_map: HashMap<&str, &DerivedMessage> = derived_msgs
            .iter()
            .map(|m| (m.message_id.as_str(), *m))
            .collect();

        // 行有事件无
        for entry in &legacy_rows {
            let id = entry.row.id.as_str();
            match derived_map.get(id) {
                None => {
                    if entry.row.error.is_some() {
                        // 错误行：内容空，双侧 LLM 均不可见（message_error 事件在场）
                        skip.add("error_row", 1);
                    } else if entry.is_empty_assistant_placeholder() {
                        skip.add("empty_placeholder", 1);
                    } else {
                        report.diffs.push(ReconcileDiff {
                            category: "MISSING_IN_DERIVED",
                            turn_id: Some(turn.clone()),
                            message_id: Some(id.to_string()),
                            detail: format!(
                                "role={} content_len={} blocks={} 行存在但事件缺失",
                                entry.row.role,
                                entry.row.content.len(),
                                serde_json::to_string(&entry.blocks).unwrap_or_default()
                            ),
                        });
                    }
                }
                Some(d) => compare_content(&mut report.diffs, turn, id, entry, d),
            }
        }
        // 事件有行无
        for (id, d) in &derived_map {
            if !legacy_map.contains_key(*id) {
                if invisible.contains(id) {
                    skip.add("discarded_row", 1);
                } else {
                    report.diffs.push(ReconcileDiff {
                        category: "MISSING_IN_LEGACY",
                        turn_id: Some(turn.clone()),
                        message_id: Some((*id).to_string()),
                        detail: format!(
                            "role={} content_len={} 事件存在但行缺失",
                            d.role,
                            d.content.len()
                        ),
                    });
                }
            }
        }

        // 顺序：turn 内行 rowid 序 vs 事件首现序。只比两侧共有的 id——
        // 容忍缺口（error/discarded/空占位）后序列本就不同，不能当乱序。
        let legacy_order: Vec<&str> = legacy_rows
            .iter()
            .map(|e| e.row.id.as_str())
            .filter(|id| derived_map.contains_key(*id))
            .collect();
        let mut derived_order: Vec<(&str, i64)> = derived_msgs
            .iter()
            .map(|m| (m.message_id.as_str(), m.first_seq))
            .filter(|(id, _)| legacy_map.contains_key(*id))
            .collect();
        derived_order.sort_by_key(|(_, seq)| *seq);
        let derived_ids: Vec<&str> = derived_order.into_iter().map(|(id, _)| id).collect();
        if !legacy_order.is_empty() && legacy_order != derived_ids {
            report.diffs.push(ReconcileDiff {
                category: "ORDER_MISMATCH",
                turn_id: Some(turn.clone()),
                message_id: None,
                detail: format!("rowid 序 {:?} ≠ 事件首现序 {:?}", legacy_order, derived_ids),
            });
        }
    }

    // 回放侧无法归属任何事件 turn 的消息（异常，计数可见）
    let mapped: HashSet<&str> = turn_order.iter().map(|s| s.as_str()).collect();
    let unmapped = derived
        .messages
        .iter()
        .filter(|m| {
            m.turn_id
                .as_deref()
                .map(|t| !mapped.contains(t))
                .unwrap_or(true)
        })
        .count();
    if unmapped > 0 {
        skip.add("derived_unmapped_turn", unmapped);
    }

    Ok(report)
}

/// legacy 行的原始形态提取结果（blocks 已解析未回退——空占位判定需要原始空态）。
#[derive(Clone)]
struct LegacyRow {
    row: MessageRow,
    blocks: Vec<crate::infra::protocol::ContentBlock>,
}

impl LegacyRow {
    /// 空 assistant 占位：content+blocks 全空、无 error。step-0 修复（loop 顶
    /// cancel 补 message_discarded）之后新数据不应再出现；历史数据容忍。
    fn is_empty_assistant_placeholder(&self) -> bool {
        self.row.role == "assistant"
            && self.row.content.trim().is_empty()
            && self.blocks.is_empty()
            && self.row.error.is_none()
    }

    /// 与 derive 侧同构的空回退：blocks 空 → [Text(content)]。
    fn effective_blocks(&self) -> Vec<crate::infra::protocol::ContentBlock> {
        if self.blocks.is_empty() {
            vec![crate::infra::protocol::ContentBlock::Text {
                text: self.row.content.clone(),
            }]
        } else {
            self.blocks.clone()
        }
    }
}

fn compare_content(
    diffs: &mut Vec<ReconcileDiff>,
    turn: &str,
    id: &str,
    entry: &LegacyRow,
    d: &DerivedMessage,
) {
    let mut mismatches: Vec<String> = Vec::new();
    if entry.row.role != d.role {
        mismatches.push(format!("role {} != {}", entry.row.role, d.role));
    }
    if entry.row.content != d.content {
        mismatches.push(format!(
            "content 长度 {} != {}（legacy 前缀 {:?} / derive 前缀 {:?}）",
            entry.row.content.len(),
            d.content.len(),
            truncate_preview(&entry.row.content),
            truncate_preview(&d.content),
        ));
    }
    if entry.effective_blocks() != d.to_content_blocks() {
        mismatches.push(format!(
            "blocks 不等（legacy {} 块 / derive {} 块）",
            entry.effective_blocks().len(),
            d.to_content_blocks().len()
        ));
    }
    if !mismatches.is_empty() {
        diffs.push(ReconcileDiff {
            category: "CONTENT_MISMATCH",
            turn_id: Some(turn.to_string()),
            message_id: Some(id.to_string()),
            detail: mismatches.join("; "),
        });
    }
}

fn truncate_preview(s: &str) -> String {
    const MAX: usize = 40;
    let mut t: String = s.chars().take(MAX).collect();
    if s.chars().count() > MAX {
        t.push('…');
    }
    t
}

/// skipped 计数器：同 reason 聚合追加。
struct SkipCounter<'a> {
    out: &'a mut Vec<ReconcileSkip>,
}

impl<'a> SkipCounter<'a> {
    fn new(out: &'a mut Vec<ReconcileSkip>) -> Self {
        Self { out }
    }
    fn add(&mut self, reason: &'static str, count: usize) {
        if count == 0 {
            return;
        }
        if let Some(existing) = self.out.iter_mut().find(|s| s.reason == reason) {
            existing.count += count;
        } else {
            self.out.push(ReconcileSkip { reason, count });
        }
    }
}

// =========================================================================
// 单元测试（分类规则逐条验证；e2e 见 tests/session_event_log_e2e.rs）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::event_log::{self, EventCtx, TurnEndedPayload};
    use crate::infra::protocol::ContentBlock;
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

    async fn seed(pool: &SqlitePool, conv_id: &str) {
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES ('agent-1', 'a', 'anthropic', 'm', '', '', 0.7, 1024, '{}', 0, 0)",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES (?, ?, 't')")
            .bind(conv_id)
            .bind("agent-1")
            .execute(pool)
            .await
            .unwrap();
    }

    async fn insert_row(
        pool: &SqlitePool,
        conv: &str,
        id: &str,
        role: &str,
        content: &str,
        blocks_json: &str,
    ) {
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, role, content, content_blocks) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(conv)
        .bind(role)
        .bind(content)
        .bind(blocks_json)
        .execute(pool)
        .await
        .unwrap();
    }

    fn text_block(t: &str) -> ContentBlock {
        ContentBlock::Text { text: t.into() }
    }

    /// 生产序完整 turn：user → assistant(tool_use) → tool_result → assistant(终答) → turn_ended。
    /// 行与事件两侧一致写入。**user 行 id == turn**（生产语义：turn_id 即 user_msg_id）。
    async fn script_consistent_turn(pool: &SqlitePool, conv: &str, turn: &str) {
        let ev = EventCtx::new(conv, turn, "agent-1");
        let (a1, r1, a2) = (
            format!("{turn}-a1"),
            format!("{turn}-r1"),
            format!("{turn}-a2"),
        );
        insert_row(
            pool,
            conv,
            turn,
            "user",
            "读文件",
            r#"[{"type":"text","text":"读文件"}]"#,
        )
        .await;
        event_log::log_user_message(pool, &ev, turn, "读文件", &[text_block("读文件")]).await;
        let a1_blocks = r#"[{"type":"tool_use","id":"tu_1","name":"read_file","input":"{}"}]"#;
        insert_row(pool, conv, &a1, "assistant", "", a1_blocks).await;
        event_log::log_assistant_message(
            pool,
            &ev,
            &a1,
            None,
            "",
            &[ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "read_file".into(),
                input: "{}".into(),
            }],
            None,
            None,
            0,
            false,
        )
        .await;
        let r1_blocks =
            r#"[{"type":"tool_result","tool_use_id":"tu_1","content":"内容","is_error":false}]"#;
        insert_row(pool, conv, &r1, "user", "", r1_blocks).await;
        event_log::log_tool_result_message(
            pool,
            &ev,
            &r1,
            &[ContentBlock::ToolResult {
                tool_use_id: "tu_1".into(),
                content: "内容".into(),
                is_error: Some(false),
            }],
        )
        .await;
        insert_row(
            pool,
            conv,
            &a2,
            "assistant",
            "读到了",
            r#"[{"type":"text","text":"读到了"}]"#,
        )
        .await;
        event_log::log_assistant_message(
            pool,
            &ev,
            &a2,
            None,
            "读到了",
            &[text_block("读到了")],
            None,
            None,
            1,
            false,
        )
        .await;
        event_log::log_turn_ended(
            pool,
            &ev,
            Some(&a2),
            &TurnEndedPayload {
                v: 1,
                termination: "stop".into(),
                rounds: 3,
                usage: None,
                user_token_count: None,
            },
        )
        .await;
    }

    fn categories(report: &ReconcileReport) -> Vec<&'static str> {
        report.diffs.iter().map(|d| d.category).collect()
    }
    fn skip_reasons(report: &ReconcileReport) -> Vec<&'static str> {
        report.skipped.iter().map(|s| s.reason).collect()
    }

    #[tokio::test]
    async fn consistent_turn_reports_zero_diffs() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        script_consistent_turn(&pool, "conv", "t1").await;

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert!(report.diffs.is_empty(), "diffs: {:#?}", report.diffs);
        assert_eq!(report.turns_compared, 1);
        assert_eq!(report.legacy_rows_compared, 4);
        assert_eq!(report.derived_messages_compared, 4);
    }

    #[tokio::test]
    async fn deleted_event_fires_missing_in_derived() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        script_consistent_turn(&pool, "conv", "t1").await;
        // 篡改：抹掉终答 assistant_message 事件（行保留）
        sqlx::query(
            "DELETE FROM session_events WHERE kind='assistant_message' AND message_id='t1-a2'",
        )
        .execute(&pool)
        .await
        .unwrap();

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert_eq!(categories(&report), vec!["MISSING_IN_DERIVED"]);
        assert_eq!(report.diffs[0].message_id.as_deref(), Some("t1-a2"));
    }

    #[tokio::test]
    async fn tampered_row_blocks_fire_content_mismatch() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        script_consistent_turn(&pool, "conv", "t1").await;
        // 篡改：行 blocks 与事件不一致
        sqlx::query("UPDATE messages SET content_blocks = '[{\"type\":\"text\",\"text\":\"被改\"}]' WHERE id = 't1-a2'")
            .execute(&pool)
            .await
            .unwrap();

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert_eq!(categories(&report), vec!["CONTENT_MISMATCH"]);
        assert!(
            report.diffs[0].detail.contains("blocks"),
            "detail: {}",
            report.diffs[0].detail
        );
    }

    #[tokio::test]
    async fn deleted_row_fires_missing_in_legacy() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        script_consistent_turn(&pool, "conv", "t1").await;
        // 篡改：删行（事件保留）
        sqlx::query("DELETE FROM messages WHERE id = 't1-a2'")
            .execute(&pool)
            .await
            .unwrap();

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert_eq!(categories(&report), vec!["MISSING_IN_LEGACY"]);
        assert_eq!(report.diffs[0].message_id.as_deref(), Some("t1-a2"));
    }

    #[tokio::test]
    async fn incomplete_turn_is_skipped_not_diffed() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        // turn 有事件但无 turn_ended（崩溃场景），行与事件本就不一致也 single skip
        script_consistent_turn(&pool, "conv", "t1").await;
        sqlx::query("DELETE FROM session_events WHERE kind='turn_ended'")
            .execute(&pool)
            .await
            .unwrap();
        // 追加一条只有行没有事件的残留（崩溃占位）——不应产生 diff
        insert_row(&pool, "conv", "t1-crash", "assistant", "", "[]").await;

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert!(report.diffs.is_empty(), "diffs: {:#?}", report.diffs);
        assert_eq!(report.turns_compared, 0);
        assert!(skip_reasons(&report).contains(&"incomplete_turn"));
    }

    #[tokio::test]
    async fn pre_epoch_rows_are_skipped() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        // 事件纪元之前的行（旧数据）
        insert_row(
            &pool,
            "conv",
            "old-u",
            "user",
            "旧问题",
            r#"[{"type":"text","text":"旧问题"}]"#,
        )
        .await;
        insert_row(
            &pool,
            "conv",
            "old-a",
            "assistant",
            "旧回答",
            r#"[{"type":"text","text":"旧回答"}]"#,
        )
        .await;
        script_consistent_turn(&pool, "conv", "t1").await;

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert!(report.diffs.is_empty(), "diffs: {:#?}", report.diffs);
        assert!(skip_reasons(&report).contains(&"legacy_epoch_rows"));
    }

    #[tokio::test]
    async fn no_events_conversation_reports_pre_phase0() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        insert_row(&pool, "conv", "old-u", "user", "旧问题", "[]").await;

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert!(report.diffs.is_empty());
        assert_eq!(skip_reasons(&report), vec!["pre_phase0_no_events"]);
    }

    #[tokio::test]
    async fn error_and_discarded_rows_are_known_gaps() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        script_consistent_turn(&pool, "conv", "t1").await;

        // 错误行：行在、error 非空、有 message_error 事件（derive 跳过 → 行侧独有）
        insert_row(&pool, "conv", "t1-err", "assistant", "", "[]").await;
        sqlx::query("UPDATE messages SET error = 'boom' WHERE id = 't1-err'")
            .execute(&pool)
            .await
            .unwrap();
        let ev = EventCtx::new("conv", "t1", "agent-1");
        event_log::log_message_error(&pool, &ev, "t1-err", "Network", "boom").await;

        // discarded 容忍（规则 4）：事件有（含 assistant_message）但行不存在
        //（终止守卫已删占位）→ MISSING_IN_LEGACY 豁免为 discarded_row
        event_log::log_assistant_message(
            &pool, &ev, "t1-gone", None, "", &[], None, None, 0, false,
        )
        .await;
        event_log::log_message_discarded(&pool, &ev, "t1-gone", "termination_guard_no_text").await;

        // 空占位（step-0 修复前的历史形态）：行在、零事件
        insert_row(&pool, "conv", "t1-empty", "assistant", "", "[]").await;

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert!(report.diffs.is_empty(), "diffs: {:#?}", report.diffs);
        let reasons = skip_reasons(&report);
        assert!(reasons.contains(&"error_row"), "reasons: {reasons:?}");
        assert!(reasons.contains(&"discarded_row"), "reasons: {reasons:?}");
        assert!(
            reasons.contains(&"empty_placeholder"),
            "reasons: {reasons:?}"
        );
    }

    #[tokio::test]
    async fn empty_placeholder_with_content_diff_still_fires() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        script_consistent_turn(&pool, "conv", "t1").await;
        // 空 assistant 行且**有内容占位**（blocks 非空）→ 不能被启发式吞掉
        insert_row(
            &pool,
            "conv",
            "t1-phantom",
            "assistant",
            "",
            r#"[{"type":"text","text":"幽灵文本"}]"#,
        )
        .await;

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert_eq!(categories(&report), vec!["MISSING_IN_DERIVED"]);
        assert_eq!(report.diffs[0].message_id.as_deref(), Some("t1-phantom"));
    }

    #[tokio::test]
    async fn superseded_row_matches_last_event_content() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        // 自动续写：同一 assistant 行被两轮 finalize 全文覆写，事件两条（last-wins）
        let ev = EventCtx::new("conv", "t1", "agent-1");
        insert_row(
            &pool,
            "conv",
            "t1",
            "user",
            "写长文",
            r#"[{"type":"text","text":"写长文"}]"#,
        )
        .await;
        event_log::log_user_message(&pool, &ev, "t1", "写长文", &[text_block("写长文")]).await;
        insert_row(
            &pool,
            "conv",
            "t1-a",
            "assistant",
            "前半段后半段",
            r#"[{"type":"text","text":"前半段后半段"}]"#,
        )
        .await;
        event_log::log_assistant_message(
            &pool,
            &ev,
            "t1-a",
            None,
            "前半段",
            &[text_block("前半段")],
            None,
            None,
            0,
            true,
        )
        .await;
        event_log::log_assistant_message(
            &pool,
            &ev,
            "t1-a",
            None,
            "前半段后半段",
            &[text_block("前半段后半段")],
            None,
            None,
            1,
            true,
        )
        .await;
        event_log::log_turn_ended(
            &pool,
            &ev,
            Some("t1-a"),
            &TurnEndedPayload {
                v: 1,
                termination: "stop".into(),
                rounds: 2,
                usage: None,
                user_token_count: None,
            },
        )
        .await;

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert!(report.diffs.is_empty(), "diffs: {:#?}", report.diffs);
        assert_eq!(report.legacy_rows_compared, 2);
        assert_eq!(report.derived_messages_compared, 2);
    }

    #[tokio::test]
    async fn row_order_versus_event_order_mismatch_fires() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        // 同 turn 内两条 assistant 行的 rowid 序与事件首现序反转（内容各配对正确，
        // 排除 CONTENT_MISMATCH 干扰）
        let ev = EventCtx::new("conv", "t1", "agent-1");
        insert_row(
            &pool,
            "conv",
            "t1",
            "user",
            "问",
            r#"[{"type":"text","text":"问"}]"#,
        )
        .await;
        event_log::log_user_message(&pool, &ev, "t1", "问", &[text_block("问")]).await;
        // rowid 序：a2 在前；事件序：a1 在前
        insert_row(
            &pool,
            "conv",
            "t1-a2",
            "assistant",
            "答二",
            r#"[{"type":"text","text":"答二"}]"#,
        )
        .await;
        insert_row(
            &pool,
            "conv",
            "t1-a1",
            "assistant",
            "答一",
            r#"[{"type":"text","text":"答一"}]"#,
        )
        .await;
        for (mid, txt) in [("t1-a1", "答一"), ("t1-a2", "答二")] {
            event_log::log_assistant_message(
                &pool,
                &ev,
                mid,
                None,
                txt,
                &[text_block(txt)],
                None,
                None,
                0,
                false,
            )
            .await;
        }
        event_log::log_turn_ended(
            &pool,
            &ev,
            Some("t1-a2"),
            &TurnEndedPayload {
                v: 1,
                termination: "stop".into(),
                rounds: 2,
                usage: None,
                user_token_count: None,
            },
        )
        .await;

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert_eq!(
            categories(&report),
            vec!["ORDER_MISMATCH"],
            "diffs: {:#?}",
            report.diffs
        );
    }

    #[tokio::test]
    async fn summary_and_tool_rows_are_skipped() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;
        script_consistent_turn(&pool, "conv", "t1").await;
        // 摘要行（loader 双注入修复语义：不进 history）
        let prefix = crate::db::repo::summary::SUMMARY_PREFIX;
        insert_row(
            &pool,
            "conv",
            "sum-1",
            "system",
            &format!("{prefix}\n摘要正文"),
            "[]",
        )
        .await;
        // tool 角色行（loader 跳过）
        insert_row(&pool, "conv", "tool-1", "tool", "原始工具输出", "[]").await;

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert!(report.diffs.is_empty(), "diffs: {:#?}", report.diffs);
        let reasons = skip_reasons(&report);
        assert!(reasons.contains(&"summary_row"), "reasons: {reasons:?}");
        assert!(
            reasons.contains(&"non_conversational_role"),
            "reasons: {reasons:?}"
        );
    }

    /// S1 阶段 3：行含 Image + 事件 payload 为 image_ref（v2 形态，手写 JSON
    /// 经 repo append——emitter 3b 才切 refify）→ 水合后零 diff（ref 指向行
    /// 本身，平面等式构造性成立）。
    #[tokio::test]
    async fn image_ref_events_hydrate_to_zero_diff() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv").await;

        let ev = EventCtx::new("conv", "t1", "agent-1");
        // user 行：text + image(base64)；事件 payload 只带轻量 ref（无 base64）
        insert_row(
            &pool,
            "conv",
            "t1",
            "user",
            "看图",
            r#"[{"type":"text","text":"看图"},{"type":"image","data":"QUJD","media_type":"image/png"}]"#,
        )
        .await;
        crate::db::repo::session_event::append(
            &pool,
            "conv",
            "user_message",
            "user",
            Some("t1"),
            Some("t1"),
            r#"{"v":2,"content":"看图","blocks":[{"type":"text","text":"看图"},{"type":"image_ref","message_id":"t1","block_index":1}]}"#,
        )
        .await
        .unwrap();
        insert_row(
            &pool,
            "conv",
            "t1-a",
            "assistant",
            "收到",
            r#"[{"type":"text","text":"收到"}]"#,
        )
        .await;
        event_log::log_assistant_message(
            &pool,
            &ev,
            "t1-a",
            None,
            "收到",
            &[text_block("收到")],
            None,
            None,
            1,
            false,
        )
        .await;
        event_log::log_turn_ended(
            &pool,
            &ev,
            Some("t1-a"),
            &TurnEndedPayload {
                v: 1,
                termination: "stop".into(),
                rounds: 1,
                usage: None,
                user_token_count: None,
            },
        )
        .await;

        let report = reconcile_session(&pool, "conv").await.unwrap();
        assert!(report.diffs.is_empty(), "diffs: {:#?}", report.diffs);
        assert_eq!(report.turns_compared, 1);
        assert_eq!(report.legacy_rows_compared, 2);
    }
}
