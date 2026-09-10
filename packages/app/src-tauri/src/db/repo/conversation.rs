//! `conversations` 表的 SQL 操作

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::db::models::{ConversationRow, NewConversation};
use crate::error::{AppError, AppResult};

/// 全列清单（MA-1 起含 kind/initiator/parent 四列，MA-3 加 inbox_policy，
/// 频道 v1 加 archived_at；`query_as<ConversationRow>` 要求 SELECT 覆盖全部
/// 字段，统一收口防止逐站点漂移）
const CONV_COLS: &str = "id, agent_id, title, pinned, created_at, updated_at, tools_override, project_id, kind, initiator_type, initiator_agent_id, parent_conversation_id, inbox_policy, archived_at";

/// 列出全部会话（不限 agent），按 `pinned DESC, updated_at DESC`
pub async fn list_all(pool: &SqlitePool) -> AppResult<Vec<ConversationRow>> {
    let rows = sqlx::query_as::<_, ConversationRow>(&format!(
        "SELECT {CONV_COLS} FROM conversations ORDER BY pinned DESC, updated_at DESC"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 列出某 agent 下的全部会话，按 `pinned DESC, updated_at DESC`
pub async fn list_by_agent(pool: &SqlitePool, agent_id: &str) -> AppResult<Vec<ConversationRow>> {
    let rows = sqlx::query_as::<_, ConversationRow>(&format!(
        "SELECT {CONV_COLS} FROM conversations WHERE agent_id = ? ORDER BY pinned DESC, updated_at DESC"
    ))
    .bind(agent_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 取一条
pub async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<ConversationRow> {
    let row = sqlx::query_as::<_, ConversationRow>(&format!(
        "SELECT {CONV_COLS} FROM conversations WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "conversation",
        id: id.to_string(),
    })?;
    Ok(row)
}

/// 创建会话，title 可空。
///
/// MA-1: kind/initiator/parent 由 NewConversation 携带（默认 chat / 用户发起 / 无父）；
/// initiator_type 从 initiator_agent_id 推导（Some → 'agent'），避免调用方双写字段。
pub async fn create(
    pool: &SqlitePool,
    id: &str,
    new_conv: &NewConversation,
) -> AppResult<ConversationRow> {
    let title = new_conv.title.as_deref().unwrap_or("");
    let kind = new_conv.kind.as_deref().unwrap_or("chat");
    let initiator_type = if new_conv.initiator_agent_id.is_some() {
        Some("agent")
    } else {
        None
    };
    sqlx::query(
        "INSERT INTO conversations (id, agent_id, title, project_id, kind, initiator_type, initiator_agent_id, parent_conversation_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&new_conv.agent_id)
    .bind(title)
    .bind(&new_conv.project_id)
    .bind(kind)
    .bind(initiator_type)
    .bind(&new_conv.initiator_agent_id)
    .bind(&new_conv.parent_conversation_id)
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// 重命名
pub async fn rename(pool: &SqlitePool, id: &str, new_title: &str) -> AppResult<()> {
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
pub async fn set_pinned(pool: &SqlitePool, id: &str, pinned: bool) -> AppResult<()> {
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

/// Phase 2: 列出某项目下的全部会话（NULL = 默认项目）
pub async fn list_by_project(
    pool: &SqlitePool,
    project_id: Option<&str>,
) -> AppResult<Vec<ConversationRow>> {
    let rows = if let Some(pid) = project_id {
        sqlx::query_as::<_, ConversationRow>(&format!(
            "SELECT {CONV_COLS} FROM conversations WHERE project_id = ?
             ORDER BY pinned DESC, updated_at DESC"
        ))
        .bind(pid)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, ConversationRow>(&format!(
            "SELECT {CONV_COLS} FROM conversations WHERE project_id IS NULL
             ORDER BY pinned DESC, updated_at DESC"
        ))
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

/// Phase 2: 移动会话到指定项目（None = 移回默认项目）
pub async fn move_to_project(
    pool: &SqlitePool,
    conversation_id: &str,
    project_id: Option<&str>,
) -> AppResult<()> {
    let affected = sqlx::query("UPDATE conversations SET project_id = ? WHERE id = ?")
        .bind(project_id)
        .bind(conversation_id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "conversation",
            id: conversation_id.to_string(),
        });
    }
    Ok(())
}

/// MA-3: 更新收件政策（'accept' | 'hold' | 'refuse'；合法值由命令层校验，
/// repo 层只管写）
pub async fn update_inbox_policy(
    pool: &SqlitePool,
    conversation_id: &str,
    policy: &str,
) -> AppResult<()> {
    let affected = sqlx::query("UPDATE conversations SET inbox_policy = ? WHERE id = ?")
        .bind(policy)
        .bind(conversation_id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "conversation",
            id: conversation_id.to_string(),
        });
    }
    Ok(())
}

// =========================================================================
// 频道 v1（design §2：kind='channel' + project_id 必填 + 每项目至多一个活频道）
// =========================================================================

/// 项目的活频道（kind='channel' 且未归档）。归档频道不参与路由/ensure——
/// `archived_at IS NULL` 是「活」的唯一定义，所有频道读写入口共用本判定。
pub async fn active_channel_for_project(
    pool: &SqlitePool,
    project_id: &str,
) -> AppResult<Option<ConversationRow>> {
    let row = sqlx::query_as::<_, ConversationRow>(&format!(
        "SELECT {CONV_COLS} FROM conversations \
         WHERE kind = 'channel' AND project_id = ? AND archived_at IS NULL"
    ))
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 幂等确保项目频道存在（不存在则创建）。并发竞态由
/// `idx_channel_per_project` 唯一索引兜底：INSERT 撞约束时重查即得既有频道。
///
/// `agent_id` = 统筹者（调用方解析：现任若在任、否则 joined_at 最早成员）；
/// 零成员会话由调用方拒绝（repo 层不做成员解析——那是引擎的职责域）。
pub async fn ensure_channel(
    pool: &SqlitePool,
    project_id: &str,
    title: &str,
    coordinator_agent_id: &str,
) -> AppResult<ConversationRow> {
    if let Some(existing) = active_channel_for_project(pool, project_id).await? {
        return Ok(existing);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let insert = sqlx::query(
        "INSERT INTO conversations (id, agent_id, title, project_id, kind) \
         VALUES (?, ?, ?, ?, 'channel')",
    )
    .bind(&id)
    .bind(coordinator_agent_id)
    .bind(title)
    .bind(project_id)
    .execute(pool)
    .await;
    match insert {
        Ok(_) => {}
        // 撞唯一索引（并发 ensure）：另一路已建，重查返回
        Err(e) if is_unique_violation(&e) => {}
        Err(e) => return Err(e.into()),
    }
    get_by_id(pool, &id).await
}

/// sqlite 唯一约束违反判定。走消息匹配而非 code()：ensure_channel 撞约束的
/// 唯一来源就是 idx_channel_per_project，误报面为零；仓内无结构化 code 判定
/// 先例，消息匹配是这里最轻的诚实实现。
fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.to_string().contains("UNIQUE constraint failed")
}

/// 归档频道（软删除：活频道判定唯一出口，历史/事件日志全保留——append-only
/// 日志无损不变式）。已归档再归档幂等（`archived_at IS NULL` 守卫）。
pub async fn set_archived(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let affected =
        sqlx::query("UPDATE conversations SET archived_at = datetime('now') WHERE id = ? AND archived_at IS NULL")
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected();
    // 行不存在 → NotFound；已归档（0 行但行在）→ 幂等成功。区分需查存在性：
    // 归档是低频治理动作，多付一次 SELECT 换准确语义。
    if affected == 0 && get_by_id(pool, id).await.is_err() {
        return Err(AppError::NotFound {
            resource: "conversation",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 某 agent 统筹的活频道（删 agent 守卫用：迁移或拒绝删除前先找到它们）。
pub async fn channels_coordinated_by(
    pool: &SqlitePool,
    agent_id: &str,
) -> AppResult<Vec<ConversationRow>> {
    let rows = sqlx::query_as::<_, ConversationRow>(&format!(
        "SELECT {CONV_COLS} FROM conversations \
         WHERE kind = 'channel' AND agent_id = ? AND archived_at IS NULL"
    ))
    .bind(agent_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 迁移频道统筹者（删 agent 守卫的迁移路径 / 换帅两档共用）。
/// 不校验目标 agent 存在性（FK 兜底）；频道的 agent_id 语义 = 当前统筹者。
pub async fn set_conversation_agent(
    pool: &SqlitePool,
    conversation_id: &str,
    agent_id: &str,
) -> AppResult<()> {
    let affected = sqlx::query("UPDATE conversations SET agent_id = ? WHERE id = ?")
        .bind(agent_id)
        .bind(conversation_id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "conversation",
            id: conversation_id.to_string(),
        });
    }
    Ok(())
}
