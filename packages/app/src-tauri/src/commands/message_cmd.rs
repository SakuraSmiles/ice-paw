//! Message 相关 Tauri Commands
//!
//! Frontend 调用入口见 `icepaw-cleanup-plan.md` §2.3。

use tauri::State;
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::db::models::{Message, NewMessage};
use crate::db::repo;
use crate::error::AppResult;

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
