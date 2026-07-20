//! `projects` + `project_agents` 表的 SQL 操作

use sqlx::SqlitePool;

use crate::db::models::{NewProject, ProjectAgentRow, ProjectRow};
use crate::error::{AppError, AppResult};

/// 列出全部项目（按 sort_order ASC, created_at ASC）
pub async fn list(pool: &SqlitePool) -> AppResult<Vec<ProjectRow>> {
    let rows = sqlx::query_as::<_, ProjectRow>(
        "SELECT id, name, description, icon, sort_order, created_at, updated_at
           FROM projects
          ORDER BY sort_order ASC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 按 ID 取一条
pub async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<ProjectRow> {
    let row = sqlx::query_as::<_, ProjectRow>(
        "SELECT id, name, description, icon, sort_order, created_at, updated_at
           FROM projects WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "project",
        id: id.to_string(),
    })?;
    Ok(row)
}

/// 创建项目
pub async fn create(
    pool: &SqlitePool,
    id: &str,
    new_project: &NewProject,
) -> AppResult<ProjectRow> {
    let name = &new_project.name;
    let description = new_project.description.as_deref().unwrap_or("");
    let icon = new_project.icon.as_deref().unwrap_or("folder");

    sqlx::query(
        "INSERT INTO projects (id, name, description, icon) VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(icon)
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// 重命名 / 更新描述（None = 不改该字段）
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
) -> AppResult<ProjectRow> {
    // 只更新非 None 的字段
    if let Some(n) = name {
        let affected = sqlx::query("UPDATE projects SET name = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(n)
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::NotFound {
                resource: "project",
                id: id.to_string(),
            });
        }
    }
    if let Some(d) = description {
        let affected = sqlx::query("UPDATE projects SET description = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(d)
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::NotFound {
                resource: "project",
                id: id.to_string(),
            });
        }
    }

    get_by_id(pool, id).await
}

/// 删除项目（project_agents 级联删除，conversations.project_id SET NULL）
pub async fn delete(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let affected = sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "project",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 排序（批量更新 sort_order）
pub async fn reorder(pool: &SqlitePool, ordered_ids: &[String]) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    for (idx, id) in ordered_ids.iter().enumerate() {
        sqlx::query("UPDATE projects SET sort_order = ? WHERE id = ?")
            .bind(idx as i32)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ============ Project ↔ Agent 关联 ============

/// 列出项目的 Agent 成员
pub async fn list_agents(pool: &SqlitePool, project_id: &str) -> AppResult<Vec<ProjectAgentRow>> {
    let rows = sqlx::query_as::<_, ProjectAgentRow>(
        "SELECT project_id, agent_id, role, joined_at
           FROM project_agents
          WHERE project_id = ?
          ORDER BY role DESC, joined_at ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 添加 Agent 到项目（INSERT OR IGNORE 保证幂等）
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

/// 从项目移除 Agent
pub async fn remove_agent(pool: &SqlitePool, project_id: &str, agent_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM project_agents WHERE project_id = ? AND agent_id = ?")
        .bind(project_id)
        .bind(agent_id)
        .execute(pool)
        .await?;
    Ok(())
}
