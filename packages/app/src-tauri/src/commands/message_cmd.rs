//! Message 相关 Tauri Commands
//!
//! Frontend 调用入口见 `icepaw-cleanup-plan.md` §2.3。

use std::collections::HashMap;

use tauri::State;
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::context::history::parse_content_blocks;
use crate::db::models::{Message, MessageFacts, MessageRow, NewMessage};
use crate::db::repo;
use crate::error::AppResult;
use crate::infra::protocol::ContentBlock;

/// 列出会话内的消息（支持复合游标分页）
///
/// - `limit`：上限 1000，默认 100
/// - `before`：复合游标 `[created_at, rowid]`，由前端从上一页结果的最末一条
///   消息的对应字段取出来回传；表示「取这两个游标之前的消息」。
///
/// 设计说明：`before` 原本只是 `created_at` 字符串，但 SQLite 的 `datetime('now')`
/// 是秒级精度，同一秒内的多条消息（user → assistant 对）共享同一时间戳。
/// 单 `created_at < ?` 在翻页时会跳过同秒的消息。改用 `(created_at, rowid)` 后，
/// 严格小于两段都满足才入选，规避该 bug（详见 `repo::message` 注释）。
///
/// Tauri v2 的 `invoke` 会把数组参数转成 `serde_json::Value`，落地到 Rust 这里
/// 用 `serde_json::Value` 接住再解析 —— 上游已经是字符串 + 整数，因此直接
/// `as_str()` / `as_i64()` 取值即可。这样比引入新结构体更轻量，且与 TS 端的
/// `[string, number]` 元组签名完全对应。
#[tauri::command]
pub async fn list_messages(
    state: State<'_, SqlitePool>,
    conversation_id: String,
    limit: Option<i64>,
    before: Option<serde_json::Value>,
) -> AppResult<Vec<Message>> {
    let cursor = parse_before_cursor(before)?;

    let rows =
        repo::message::list_by_conversation(state.inner(), &conversation_id, limit, cursor).await?;
    let mut msgs: Vec<Message> = rows.into_iter().map(Message::from).collect();
    enrich_channel_sender_meta(state.inner(), &conversation_id, &mut msgs).await?;
    Ok(msgs)
}

/// 回合制分页返回形状（`list_messages_by_turns`）：rows 时间正序 +
/// has_more + 下页游标（本页最旧纳入回合的锚 rowid；has_more=false 时缺席）。
#[derive(Debug, serde::Serialize)]
pub struct MessageTurnPage {
    pub rows: Vec<Message>,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_before_anchor_rowid: Option<i64>,
}

/// 回合制分页（消息列表主读路径的换轨目标）：以真实 user 消息锚为回合头
/// 取「最近 N 个回合」——每页必含可见内容、页界 = 回合边界（细节见
/// `repo::message::list_by_turn_page` 文档）。
///
/// 游标必须原样回传 `before_anchor_rowid`，勿从前端 messages[0] 自造
/// （乐观行 rowid:0 / 本地冻结行都不是锚）。编排镜像 [`list_messages`：
/// 含频道 sender enrichment——漏掉只损 `sender_agent_name` /
/// `turn_duration_ms` 两个 skip-if-none 字段，1v1 零感知]。
#[tauri::command]
pub async fn list_messages_by_turns(
    state: State<'_, SqlitePool>,
    conversation_id: String,
    turns: Option<i64>,
    before_anchor_rowid: Option<i64>,
) -> AppResult<MessageTurnPage> {
    if let Some(before) = before_anchor_rowid {
        if before <= 0 {
            return Err(crate::error::AppError::Validation(
                "before_anchor_rowid 须为正 rowid".into(),
            ));
        }
    }
    let page = repo::message::list_by_turn_page(
        state.inner(),
        &conversation_id,
        turns,
        before_anchor_rowid,
    )
    .await?;
    // Layer B：读时算好统计事实（须在 From 转换前——compute 吃 MessageRow 切片）
    let facts = compute_page_facts(&page.rows);
    let mut msgs: Vec<Message> = page.rows.into_iter().map(Message::from).collect();
    for (m, f) in msgs.iter_mut().zip(facts) {
        m.stats = f;
    }
    enrich_channel_sender_meta(state.inner(), &conversation_id, &mut msgs).await?;
    Ok(MessageTurnPage {
        rows: msgs,
        has_more: page.has_more,
        next_before_anchor_rowid: page.next_before_anchor_rowid,
    })
}

// ===== Layer B：统计事实读时算好随页返回（前端组级聚合的省解析快路径）=====

/// 整页统计（command 层两趟，先例 [`enrich_channel_sender_meta`]；repo 保持 SQL 纯）。
/// - Pass 1（全页含 user 行——tool_result 配对行是 user 角色）：解析 ToolResult 建
///   `tool_use_id → is_error` 表（同 id 后者胜，对齐前端 toolResultIndex「取最后出现」）。
/// - Pass 2（逐行）：ToolUse 过豁免谓词后计数、查表计错（未配对不计错——前端
///   getToolHasError 缺省 false 对齐）；Thinking 计段。
///
/// 仅 assistant 行产出 Some（零值也是 Some——「算过了确实没有」；user 行 None
/// 缩 payload）。解析器与 reconcile/LLM 读路径同源（`parse_content_blocks`，
/// 空/[]/invalid → 空 Vec）。
fn compute_page_facts(rows: &[MessageRow]) -> Vec<Option<MessageFacts>> {
    let mut result_errors: HashMap<String, bool> = HashMap::new();
    for row in rows {
        for block in parse_content_blocks(&row.content_blocks) {
            if let ContentBlock::ToolResult {
                tool_use_id,
                is_error,
                ..
            } = block
            {
                result_errors.insert(tool_use_id, is_error.unwrap_or(false));
            }
        }
    }
    rows.iter()
        .map(|row| {
            if row.role != "assistant" {
                return None;
            }
            let mut facts = MessageFacts {
                tool_uses: 0,
                tool_errors: 0,
                think_segs: 0,
            };
            for block in parse_content_blocks(&row.content_blocks) {
                match block {
                    ContentBlock::ToolUse { id, name, input } => {
                        let is_err = result_errors.get(&id).copied().unwrap_or(false);
                        if is_structured_card_tool_use(&name, &input, is_err) {
                            continue;
                        }
                        facts.tool_uses += 1;
                        if is_err {
                            facts.tool_errors += 1;
                        }
                    }
                    ContentBlock::Thinking { .. } => facts.think_segs += 1,
                    _ => {}
                }
            }
            Some(facts)
        })
        .collect()
}

/// 豁免判定（前端 ChatMessages.vue `structuredCardKindOf` 的窄化冻结副本——
/// 两侧同 fixture 测试互指，改工具名/形状两边一起动）：该 tool_use 渲染为
/// 结构化卡片（委派/计划）而非通用行，不计入工具折叠摘要——「摘要行数 =
/// 被隐藏的通用行数」两口径天然一致。
fn is_structured_card_tool_use(name: &str, input: &str, result_is_error: bool) -> bool {
    if name == "delegate_to_agent" {
        return true;
    }
    name == "update_plan" && !result_is_error && plan_input_has_steps(input)
}

/// update_plan 参数形状检查（前端 `parsePlanInput` 的镜像：JSON 对象且 steps
/// 是数组——steps:[] 合法=agent 主动清空计划；缺失/非数组/非 JSON → false）。
fn plan_input_has_steps(input: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|v| v.get("steps").and_then(|s| s.as_array()).map(|_| true))
        .unwrap_or(false)
}

/// 频道会话的列表读路径 enrichment：回填 `sender_agent_name` /
/// `turn_duration_ms`（非表列派生字段，见 models.rs 注释）。
///
/// 触发判据 = 页内任一行带 `sender_agent_id`（频道成员回合 sweep 后才有；
/// 1v1 会话恒 NULL 零开销直过）。数据源 = `assistant_message` 事件 payload
/// （seq 正序 last-wins = supersede 语义）——行上已有值不覆写（防御位，
/// 现行行侧恒 None）。翻页各页独立 enrichment，窗口外消息不查。
async fn enrich_channel_sender_meta(
    pool: &SqlitePool,
    conversation_id: &str,
    msgs: &mut [Message],
) -> AppResult<()> {
    if !msgs.iter().any(|m| m.sender_agent_id.is_some()) {
        return Ok(()); // 非频道（或频道尚无成员回合）：零开销直过
    }
    let ids: Vec<String> = msgs
        .iter()
        .filter(|m| m.role == "assistant")
        .map(|m| m.id.clone())
        .collect();
    let metas =
        repo::session_event::assistant_meta_by_message_ids(pool, conversation_id, &ids).await?;
    let mut by_id: std::collections::HashMap<String, (Option<String>, Option<i64>)> =
        std::collections::HashMap::new();
    for (mid, name, dur) in metas {
        by_id.insert(mid, (name, dur)); // seq 正序 → 后写覆盖 = last-wins
    }
    for m in msgs.iter_mut() {
        if m.role != "assistant" {
            continue;
        }
        if let Some((name, dur)) = by_id.get(&m.id) {
            if m.sender_agent_name.is_none() {
                m.sender_agent_name = name.clone();
            }
            if m.turn_duration_ms.is_none() {
                m.turn_duration_ms = *dur;
            }
        }
    }
    Ok(())
}

/// 解析前端传来的 `before` 复合游标。
///
/// 期望格式：`[created_at_str, rowid_int]`。
///
/// 错误情况返回 `AppError::Validation`，由前端错误归一化（`bridge.wrapInvokeError`）
/// 转成可读 Error 上抛。
fn parse_before_cursor(raw: Option<serde_json::Value>) -> AppResult<Option<(String, i64)>> {
    let Some(v) = raw else { return Ok(None) };
    if v.is_null() {
        return Ok(None);
    }

    let arr = v.as_array().ok_or_else(|| {
        crate::error::AppError::Validation("before 参数必须是 [created_at, rowid] 数组".into())
    })?;
    if arr.len() != 2 {
        return Err(crate::error::AppError::Validation(format!(
            "before 参数数组长度必须为 2，实际为 {}",
            arr.len()
        )));
    }

    let ts = arr[0]
        .as_str()
        .ok_or_else(|| {
            crate::error::AppError::Validation("before[0] 必须是字符串（created_at）".into())
        })?
        .to_string();

    let rowid = arr[1].as_i64().ok_or_else(|| {
        // JSON 数字如果是浮点会被 serde_json 解成 f64；to_string 兜底取整
        if let Some(n) = arr[1].as_f64() {
            return crate::error::AppError::Validation(format!(
                "before[1] 必须是整数（rowid），但收到 {n}"
            ));
        }
        crate::error::AppError::Validation("before[1] 必须是整数（rowid）".into())
    })?;

    Ok(Some((ts, rowid)))
}

/// 写入新消息
#[tauri::command]
pub async fn create_message(state: State<'_, SqlitePool>, input: NewMessage) -> AppResult<Message> {
    if input.conversation_id.trim().is_empty() {
        return Err(crate::error::AppError::Validation(
            "conversation_id 不能为空".into(),
        ));
    }
    if input.content.is_empty() {
        return Err(crate::error::AppError::Validation(
            "content 不能为空".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    let row = repo::message::create(state.inner(), &id, &input).await?;
    Ok(Message::from(row))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小 MessageRow 构造器（models.rs 15 字段全量——sqlx(default) 字段在纯 Rust
    /// 构造时无默认，须显式给值）。
    fn row(id: &str, role: &str, content_blocks: &str) -> MessageRow {
        MessageRow {
            id: id.into(),
            conversation_id: "c1".into(),
            role: role.into(),
            content: String::new(),
            content_blocks: content_blocks.into(),
            token_count: None,
            error: None,
            created_at: "2026-09-16 10:00:00".into(),
            rowid: 1,
            summary_id: None,
            model: None,
            source_seq: None,
            incoming_source: None,
            sender_agent_id: None,
            sender_agent_name: None,
            turn_duration_ms: None,
        }
    }

    fn facts_of(tool_uses: u32, tool_errors: u32, think_segs: u32) -> Option<MessageFacts> {
        Some(MessageFacts { tool_uses, tool_errors, think_segs })
    }

    // ===== compute_page_facts：豁免三态（同 fixture 互指 ChatMessages.tool-collapse.test.ts）=====

    #[test]
    fn facts_delegate_always_exempt_even_error() {
        // 前端用例「豁免不计数」同 fixture：delegate_to_agent 恒豁免（含 Err 兜底行）
        let rows = [
            row("a1", "assistant", r#"[{"type":"tool_use","id":"e-dl","name":"delegate_to_agent","input":"{\"agent_id\":\"dev-2\",\"task\":\"t\"}"}]"#),
            row("u1", "user", r#"[{"type":"tool_result","tool_use_id":"e-dl","content":"err","is_error":true}]"#),
        ];
        assert_eq!(compute_page_facts(&rows), vec![facts_of(0, 0, 0), None]);
    }

    #[test]
    fn facts_update_plan_steps_exempt_but_error_not() {
        // steps 是数组合法豁免（is_error=false 配对）；配对 Err 则落回通用行计错
        let ok = row("a1", "assistant", r#"[{"type":"tool_use","id":"e-pl","name":"update_plan","input":"{\"steps\":[{\"text\":\"a\",\"status\":\"pending\"}]}"}]"#);
        let ok_result = row("u1", "user", r#"[{"type":"tool_result","tool_use_id":"e-pl","content":"{\"ok\":true}","is_error":false}]"#);
        assert_eq!(compute_page_facts(&[ok, ok_result]), vec![facts_of(0, 0, 0), None]);

        let err = row("a2", "assistant", r#"[{"type":"tool_use","id":"e-pl2","name":"update_plan","input":"{\"steps\":[]}"}]"#);
        let err_result = row("u2", "user", r#"[{"type":"tool_result","tool_use_id":"e-pl2","content":"bad","is_error":true}]"#);
        assert_eq!(compute_page_facts(&[err, err_result]), vec![facts_of(1, 1, 0), None]);
    }

    #[test]
    fn facts_update_plan_missing_steps_not_exempt() {
        // steps 缺失（{"ok":true}——前端 fixture 同款）→ 非计划卡，通用行计数
        let rows = [
            row("a1", "assistant", r#"[{"type":"tool_use","id":"x","name":"update_plan","input":"{\"ok\":true}"}]"#),
            row("u1", "user", r#"[{"type":"tool_result","tool_use_id":"x","content":"ok","is_error":false}]"#),
        ];
        assert_eq!(compute_page_facts(&rows), vec![facts_of(1, 0, 0), None]);
    }

    // ===== compute_page_facts：配对与计数 =====

    #[test]
    fn facts_cross_row_pairing_and_unpaired_no_error() {
        // 跨行配对：assistant tool_use + user tool_result（is_error=true）计错；
        // 未配对（无 result 行）不计错——前端 getToolHasError 缺省 false 对齐
        let rows = [
            row("a1", "assistant", r#"[
                {"type":"tool_use","id":"t1","name":"read_file","input":"{\"path\":\"a.md\"}"},
                {"type":"tool_use","id":"t2","name":"read_file","input":"{\"path\":\"b.md\"}"}
            ]"#),
            row("u1", "user", r#"[{"type":"tool_result","tool_use_id":"t1","content":"文件不存在: a.md","is_error":true}]"#),
        ];
        assert_eq!(compute_page_facts(&rows), vec![facts_of(2, 1, 0), None]);
    }

    #[test]
    fn facts_same_id_last_wins() {
        // 同 id 多条 tool_result 后者胜（supersede 形态；前端 toolResultIndex 同口径）
        let rows = [
            row("a1", "assistant", r#"[{"type":"tool_use","id":"t1","name":"read_file","input":"{}"}]"#),
            row("u1", "user", r#"[{"type":"tool_result","tool_use_id":"t1","content":"v1","is_error":false}]"#),
            row("u2", "user", r#"[{"type":"tool_result","tool_use_id":"t1","content":"v2","is_error":true}]"#),
        ];
        assert_eq!(compute_page_facts(&rows), vec![facts_of(1, 1, 0), None, None]);
    }

    #[test]
    fn facts_think_segs_counted() {
        let rows = [row(
            "a1",
            "assistant",
            r#"[
                {"type":"thinking","thinking":"第一段"},
                {"type":"text","text":"答"},
                {"type":"thinking","thinking":"第二段","duration_ms":3000}
            ]"#,
        )];
        assert_eq!(compute_page_facts(&rows), vec![facts_of(0, 0, 2)]);
    }

    #[test]
    fn facts_invalid_json_and_empty_yield_zero_some() {
        // 解析器与 reconcile/LLM 读路径同源：空/[]/invalid → 空 Vec → 零值 Some
        // （「算过了确实没有」——零值也是计算结果，与 user 行 None 区分）
        let rows = [
            row("a1", "assistant", "[]"),
            row("a2", "assistant", "not-json{{{"),
        ];
        assert_eq!(compute_page_facts(&rows), vec![facts_of(0, 0, 0), facts_of(0, 0, 0)]);
    }

    #[test]
    fn facts_user_rows_none_assistant_zero_some() {
        let rows = [
            row("u1", "user", r#"[{"type":"tool_result","tool_use_id":"t1","content":"ok","is_error":false}]"#),
            row("a1", "assistant", r#"[]"#),
        ];
        assert_eq!(compute_page_facts(&rows), vec![None, facts_of(0, 0, 0)]);
    }

    // ===== 豁免谓词表驱动（前端 structuredCardKind/parsePlanInput 的窄化冻结镜像）=====

    #[test]
    fn exemption_predicate_table() {
        // (name, input, result_is_error, expected_exempt)
        // 工具名/形状改动须与前端 ChatMessages.vue structuredCardKind 两边一起动——
        // 前端回归锁在 ChatMessages.tool-collapse.test.ts 用例 4/5。
        let cases: &[(&str, &str, bool, bool)] = &[
            ("delegate_to_agent", r#"{"agent_id":"dev-2","task":"t"}"#, false, true),
            ("delegate_to_agent", r#"{"agent_id":"dev-2"}"#, true, true), // 委派恒豁免（Err 兜底通用行也藏进卡）
            ("update_plan", r#"{"steps":[{"text":"a","status":"pending"}]}"#, false, true),
            ("update_plan", r#"{"steps":[]}"#, false, true), // 空 steps 合法（agent 主动清空）
            ("update_plan", r#"{"steps":[]}"#, true, false), // 配对 Err 落回通用行
            ("update_plan", r#"{"ok":true}"#, false, false), // steps 缺失
            ("update_plan", r#"{"steps":"not-array"}"#, false, false), // steps 非数组
            ("update_plan", "not-json", false, false), // 参数非 JSON
            ("read_file", r#"{"path":"a.md"}"#, false, false), // 普通工具永不豁免
        ];
        for (name, input, is_err, expected) in cases {
            assert_eq!(
                is_structured_card_tool_use(name, input, *is_err),
                *expected,
                "case: {name} / {input} / err={is_err}"
            );
        }
    }

    #[test]
    fn plan_input_has_steps_shapes() {
        assert!(plan_input_has_steps(r#"{"steps":[]}"#));
        assert!(plan_input_has_steps(r#"{"steps":[{"text":"a"}]}"#));
        assert!(!plan_input_has_steps(r#"{"ok":true}"#));
        assert!(!plan_input_has_steps(r#"{"steps":1}"#));
        assert!(!plan_input_has_steps(""));
        assert!(!plan_input_has_steps(r#"["steps"]"#)); // 顶层非对象
        assert!(!plan_input_has_steps("null")); // JSON 字面量
    }
}
