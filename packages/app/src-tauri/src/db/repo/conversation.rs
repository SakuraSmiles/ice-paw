//! `conversations` 表的 SQL 操作

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::db::models::{ConversationRow, NewConversation};
use crate::error::{AppError, AppResult};

/// 列出某 agent 下的全部会话，按 `pinned DESC, updated_at DESC`
pub async fn list_by_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> AppResult<Vec<ConversationRow>> {
    let rows = sqlx::query_as::<_, ConversationRow>(
        "SELECT id, agent_id, title, pinned, created_at, updated_at, tools_override
           FROM conversations
          WHERE agent_id = ?
          ORDER BY pinned DESC, updated_at DESC",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 取一条
pub async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<ConversationRow> {
    let row = sqlx::query_as::<_, ConversationRow>(
        "SELECT id, agent_id, title, pinned, created_at, updated_at, tools_override
           FROM conversations WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "conversation",
        id: id.to_string(),
    })?;
    Ok(row)
}

/// 创建会话，title 可空
pub async fn create(
    pool: &SqlitePool,
    id: &str,
    new_conv: &NewConversation,
) -> AppResult<ConversationRow> {
    let title = new_conv.title.as_deref().unwrap_or("");
    sqlx::query(
        "INSERT INTO conversations (id, agent_id, title) VALUES (?, ?, ?)",
    )
    .bind(id)
    .bind(&new_conv.agent_id)
    .bind(title)
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// 重命名
pub async fn rename(
    pool: &SqlitePool,
    id: &str,
    new_title: &str,
) -> AppResult<()> {
    let affected = sqlx::query("UPDATE conversations SET title = ? WHERE id = ?")
        .bind(new_title)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "conversation",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 置顶 / 取消置顶
pub async fn set_pinned(
    pool: &SqlitePool,
    id: &str,
    pinned: bool,
) -> AppResult<()> {
    let affected = sqlx::query("UPDATE conversations SET pinned = ? WHERE id = ?")
        .bind(if pinned { 1_i32 } else { 0_i32 })
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "conversation",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// Task 3b: 更新对话级工具覆盖。
///
/// - `override_map = None`：清除覆盖，恢复继承 Agent 配置。
/// - `override_map = Some(map)`：写入 JSON 字符串。
///
/// 空.HashMap 也会被序列化为 `{}`，语义上表示「全部禁用」。
pub async fn update_tools_override(
    pool: &SqlitePool,
    conv_id: &str,
    override_map: Option<&HashMap<String, bool>>,
) -> AppResult<()> {
    let json = override_map.map(|m| serde_json::to_string(m).unwrap_or_default());
    let affected = sqlx::query("UPDATE conversations SET tools_override = ? WHERE id = ?")
        .bind(json)
        .bind(conv_id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "conversation",
            id: conv_id.to_string(),
        });
    }
    Ok(())
}

/// 删除（依赖外键 CASCADE 自动清理 messages）
pub async fn delete(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let affected = sqlx::query("DELETE FROM conversations WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "conversation",
            id: id.to_string(),
        });
    }
    Ok(())
}
