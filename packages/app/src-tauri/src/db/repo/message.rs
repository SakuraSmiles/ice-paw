//! `messages` 表的 SQL 操作
//!
//! 关键点：
//! - 列表支持 `limit` 和 `before`（created_at）翻页
//! - 默认按 `created_at ASC, id ASC` 升序输出（chat 展示顺序）

use sqlx::SqlitePool;

use crate::db::models::{MessageRow, NewMessage};
use crate::error::{AppError, AppResult};

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 1000;

/// 列出会话内的消息
///
/// - `before`：传某个 `created_at` 表示「取它之前的」，用于往前翻页
/// - `limit`：上限 1000，默认 100
pub async fn list_by_conversation(
    pool: &SqlitePool,
    conversation_id: &str,
    limit: Option<i64>,
    before: Option<&str>,
) -> AppResult<Vec<MessageRow>> {
    let mut lim = limit.unwrap_or(DEFAULT_LIMIT);
    if lim <= 0 { lim = DEFAULT_LIMIT; }
    if lim > MAX_LIMIT { lim = MAX_LIMIT; }

    let rows = if let Some(before_ts) = before {
        sqlx::query_as::<_, MessageRow>(
            "SELECT id, conversation_id, role, content, token_count, error, created_at
               FROM messages
              WHERE conversation_id = ?
                AND created_at < ?
              ORDER BY created_at DESC, id DESC
              LIMIT ?",
        )
        .bind(conversation_id)
        .bind(before_ts)
        .bind(lim)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, MessageRow>(
            "SELECT id, conversation_id, role, content, token_count, error, created_at
               FROM messages
              WHERE conversation_id = ?
              ORDER BY created_at DESC, id DESC
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
            (id, conversation_id, role, content, token_count, error)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&new_msg.conversation_id)
    .bind(&new_msg.role)
    .bind(&new_msg.content)
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
        "SELECT id, conversation_id, role, content, token_count, error, created_at
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
