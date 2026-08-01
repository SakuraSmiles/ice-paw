//! `projects` + `project_agents` 表的 SQL 操作
//!
//! DB schema 已由 migration 13/14/21 建好。本模块补 Rust CRUD 层。

use sqlx::SqlitePool;

use crate::db::models::{NewProject, ProjectAgentRow, ProjectRow, UpdateProject};
use crate::error::{AppError, AppResult};

const PROJECT_COLS: &str = "id, name, description, icon, sort_order, workspace_path, theme_color, archived, created_at, updated_at";

// ===== projects CRUD =====

/// 列出全部项目（含已归档，由前端按 archived 拆分活跃/已归档），活跃在前
pub async fn list(pool: &SqlitePool) -> AppResult<Vec<ProjectRow>> {
    let rows = sqlx::query_as::<_, ProjectRow>(&format!(
        "SELECT {PROJECT_COLS} FROM projects ORDER BY archived ASC, sort_order ASC, created_at ASC"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<ProjectRow> {
    let row = sqlx::query_as::<_, ProjectRow>(&format!(
        "SELECT {PROJECT_COLS} FROM projects WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "project",
        id: id.to_string(),
    })?;
    Ok(row)
}

pub async fn create(pool: &SqlitePool, input: &NewProject, id: &str) -> AppResult<ProjectRow> {
    sqlx::query(
        "INSERT INTO projects (id, name, description, icon, workspace_path, theme_color)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&input.name)
    .bind(input.description.as_deref().unwrap_or(""))
    .bind(input.icon.as_deref().unwrap_or("folder"))
    .bind(input.workspace_path.as_deref())
    .bind(input.theme_color.as_deref())
    .execute(pool)
    .await?;

    // 插入初始成员
    for agent_id in &input.agent_ids {
        let _ = sqlx::query(
            "INSERT OR IGNORE INTO project_agents (project_id, agent_id, role) VALUES (?, ?, 'member')",
        )
        .bind(id)
        .bind(agent_id)
        .execute(pool)
        .await;
    }

    get_by_id(pool, id).await
}

/// partial update：None 字段不动
pub async fn update(pool: &SqlitePool, input: &UpdateProject) -> AppResult<ProjectRow> {
    let existing = get_by_id(pool, &input.id).await?;
    let name = input.name.as_deref().unwrap_or(&existing.name);
    let description = input.description.as_deref().unwrap_or(&existing.description);
    let icon = input.icon.as_deref().unwrap_or(&existing.icon);
    let workspace_path = input.workspace_path.as_deref().or(existing.workspace_path.as_deref());
    let theme_color = input.theme_color.as_deref().or(existing.theme_color.as_deref());

    sqlx::query(
        "UPDATE projects SET name=?, description=?, icon=?, workspace_path=?, theme_color=?, updated_at=datetime('now') WHERE id=?",
    )
    .bind(name)
    .bind(description)
    .bind(icon)
    .bind(workspace_path)
    .bind(theme_color)
    .bind(&input.id)
    .execute(pool)
    .await?;

    get_by_id(pool, &input.id).await
}

pub async fn delete(pool: &SqlitePool, id: &str) -> AppResult<()> {
    // project_agents ON DELETE CASCADE 自动删；conversations.project_id ON DELETE SET NULL 自动置空
    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 归档 / 恢复项目（软删除开关）
pub async fn set_archived(pool: &SqlitePool, id: &str, archived: bool) -> AppResult<()> {
    sqlx::query("UPDATE projects SET archived = ?, updated_at = datetime('now') WHERE id = ?")
        .bind(archived)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 永久删除：delete_conversations=true 连同该项目会话一起删；
/// false 则依赖 conversations.project_id ON DELETE SET NULL，会话转为散落。
pub async fn permanent_delete(pool: &SqlitePool, id: &str, delete_conversations: bool) -> AppResult<()> {
    if delete_conversations {
        sqlx::query("DELETE FROM conversations WHERE project_id = ?")
            .bind(id)
            .execute(pool)
            .await?;
    }
    delete(pool, id).await
}

/// 批量更新排序
pub async fn reorder(pool: &SqlitePool, ids: &[String]) -> AppResult<()> {
    for (i, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE projects SET sort_order = ? WHERE id = ?")
            .bind(i as i32)
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

// ===== project_agents 管理 =====

pub async fn list_agents(pool: &SqlitePool, project_id: &str) -> AppResult<Vec<ProjectAgentRow>> {
    let rows = sqlx::query_as::<_, ProjectAgentRow>(
        "SELECT project_id, agent_id, role, joined_at FROM project_agents WHERE project_id = ?",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 全量替换项目成员（先删后插）
pub async fn set_agents(
    pool: &SqlitePool,
    project_id: &str,
    members: &[(String, String)],
) -> AppResult<()> {
    sqlx::query("DELETE FROM project_agents WHERE project_id = ?")
        .bind(project_id)
        .execute(pool)
        .await?;
    for (agent_id, role) in members {
        sqlx::query(
            "INSERT INTO project_agents (project_id, agent_id, role) VALUES (?, ?, ?)",
        )
        .bind(project_id)
        .bind(agent_id)
        .bind(role)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn add_agent(
    pool: &SqlitePool,
    project_id: &str,
    agent_id: &str,
    role: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO project_agents (project_id, agent_id, role) VALUES (?, ?, ?)",
    )
    .bind(project_id)
    .bind(agent_id)
    .bind(role)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_agent(pool: &SqlitePool, project_id: &str, agent_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM project_agents WHERE project_id = ? AND agent_id = ?")
        .bind(project_id)
        .bind(agent_id)
        .execute(pool)
        .await?;
    Ok(())
}
