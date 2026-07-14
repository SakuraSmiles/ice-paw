//! `messages` 表的 SQL 操作
//!
//! 关键点：
//! - 列表支持 `limit` 和 `before`（created_at）翻页
//! - 默认按 `created_at ASC` 升序输出（chat 展示顺序）
//! - 同秒内多条消息的次序：用 `rowid` 做兜底排序，避免 UUID 随机化导致
//!   「用户/助手 同一对内顺序反转」的 bug（见 `list_by_conversation`）

use sqlx::SqlitePool;

use crate::db::models::{MessageRow, NewMessage};
use crate::error::{AppError, AppResult};

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 1000;

/// 列出会话内的消息（复合游标分页）
///
/// - `before`：`(created_at, rowid)` 复合游标，表示「取此游标之前的消息」。
///   前一页最末一条消息的 `(created_at, rowid)` 即下一页要传的 `before`。
///   `Some((ts, 0))` 之类的边界值由调用方负责，函数假定传入合法游标。
/// - `limit`：上限 1000，默认 100
///
/// 排序策略：`(created_at DESC, rowid DESC)`，反转后等价于
/// `(created_at ASC, rowid ASC)`。
///
/// 关键：**不要把 `id` 当成 tie-breaker**。
/// `id` 是 TEXT 类型的随机 UUID（v4），字典序与插入顺序无关。
/// SQLite 的 `datetime('now')` 是秒级精度——`send_message` 内
/// 「先 INSERT 用户消息，再 INSERT 助手占位」两次写入通常落在同一秒，
/// 此时如果以 `id` 做兜底排序，约 50% 的概率助手会排在用户之前，
/// 再被 `rows.reverse()` 反转后变成「助手先、用户后」——
/// 表现为页面上 `用户 → AI → AI → 用户` 的顺序错乱。
///
/// 改用 `rowid`（SQLite 的物理行号，单调递增）后，助手占位一定在
/// 用户消息之后被 INSERT，`rowid` 也一定更大，排序与插入顺序一致，
/// 翻转到 ASC 后用户在前、助手在后，行为可预期。
///
/// 同时，向上翻页用 `(created_at, rowid)` 复合游标，**严格小于**两段都满足
/// 的消息才返回。这样可以确保同秒内的多页之间不重不漏：
///
/// ```sql
/// AND (created_at < ? OR (created_at = ? AND rowid < ?))
/// ```
pub async fn list_by_conversation(
    pool: &SqlitePool,
    conversation_id: &str,
    limit: Option<i64>,
    before: Option<(String, i64)>,
) -> AppResult<Vec<MessageRow>> {
    let mut lim = limit.unwrap_or(DEFAULT_LIMIT);
    if lim <= 0 { lim = DEFAULT_LIMIT; }
    if lim > MAX_LIMIT { lim = MAX_LIMIT; }

    let rows = if let Some((before_ts, before_rowid)) = before {
        sqlx::query_as::<_, MessageRow>(
            "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid
               FROM messages
              WHERE conversation_id = ?
                AND (created_at < ? OR (created_at = ? AND rowid < ?))
              ORDER BY created_at DESC, rowid DESC
              LIMIT ?",
        )
        .bind(conversation_id)
        .bind(&before_ts)
        .bind(&before_ts)
        .bind(before_rowid)
        .bind(lim)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, MessageRow>(
            "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid
               FROM messages
              WHERE conversation_id = ?
              ORDER BY created_at DESC, rowid DESC
              LIMIT ?",
        )
        .bind(conversation_id)
        .bind(lim)
        .fetch_all(pool)
        .await?
    };

    // 反转，按时间正序返回（chat 友好的顺序）
    let mut rows = rows;
    rows.reverse();
    Ok(rows)
}

/// 统计会话内的消息总数
///
/// 返回 `i64` 以与 SQL `COUNT(*)` 对齐，且与 SQLite 的上限毫无关系。
/// 用于前端「还有 N 条历史消息」之类的展示（P2 可选，本期不调用）。
pub async fn count_by_conversation(
    pool: &SqlitePool,
    conversation_id: &str,
) -> AppResult<i64> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM messages WHERE conversation_id = ?",
    )
    .bind(conversation_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// 写入新消息
pub async fn create(
    pool: &SqlitePool,
    id: &str,
    new_msg: &NewMessage,
) -> AppResult<MessageRow> {
    // 基本校验
    if !matches!(new_msg.role.as_str(), "system" | "user" | "assistant" | "tool") {
        return Err(AppError::Validation(format!(
            "非法 role: {}, 必须是 system/user/assistant/tool",
            new_msg.role
        )));
    }

    sqlx::query(
        "INSERT INTO messages
            (id, conversation_id, role, content, content_blocks, token_count, error)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&new_msg.conversation_id)
    .bind(&new_msg.role)
    .bind(&new_msg.content)
    .bind("[]")  // content_blocks 默认空数组
    .bind(new_msg.token_count)
    .bind(new_msg.error.as_deref())
    .execute(pool)
    .await?;

    // 更新父会话的 updated_at 触发器虽然有，但 INSERT 不触发；这里手动 update 一下
    sqlx::query(
        "UPDATE conversations SET updated_at = datetime('now') WHERE id = ?",
    )
    .bind(&new_msg.conversation_id)
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<MessageRow> {
    sqlx::query_as::<_, MessageRow>(
        "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid
           FROM messages WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "message",
        id: id.to_string(),
    })
}

/// 更新消息内容（流式生成结束后回写完整文本）
pub async fn update_content(
    pool: &SqlitePool,
    id: &str,
    content: &str,
) -> AppResult<()> {
    let affected = sqlx::query("UPDATE messages SET content = ? WHERE id = ?")
        .bind(content)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "message",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 更新消息的 content_blocks 字段（P2-1 工具调用场景）
pub async fn update_content_blocks(
    pool: &SqlitePool,
    id: &str,
    content_blocks: &str,
) -> AppResult<()> {
    let affected = sqlx::query("UPDATE messages SET content_blocks = ? WHERE id = ?")
        .bind(content_blocks)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "message",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 更新消息错误字段（流式生成失败时记录）
pub async fn update_error(
    pool: &SqlitePool,
    id: &str,
    error: &str,
) -> AppResult<()> {
    let affected = sqlx::query("UPDATE messages SET error = ? WHERE id = ?")
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "message",
            id: id.to_string(),
        });
    }
    Ok(())
}
