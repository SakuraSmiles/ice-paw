//! `projects` + `project_agents` 表的 SQL 操作

use sqlx::SqlitePool;

use crate::db::models::{NewProject, ProjectAgentInput, ProjectAgentRow, ProjectPatch, ProjectRow};
use crate::error::{AppError, AppResult};

/// 列出全部项目（按 sort_order ASC, created_at ASC）
pub async fn list(pool: &SqlitePool) -> AppResult<Vec<ProjectRow>> {
    let rows = sqlx::query_as::<_, ProjectRow>(
        "SELECT id, name, description, icon, workspace_path, sort_order, created_at, updated_at
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
        "SELECT id, name, description, icon, workspace_path, sort_order, created_at, updated_at
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

/// 创建项目（仅基础信息，不写 members）
///
/// **保留** 作向后兼容；新流程走 `create_with_agents`。
pub async fn create(
    pool: &SqlitePool,
    id: &str,
    new_project: &NewProject,
) -> AppResult<ProjectRow> {
    let name = &new_project.name;
    let description = new_project.description.as_deref().unwrap_or("");
    let icon = new_project.icon.as_deref().unwrap_or("folder");

    sqlx::query(
        "INSERT INTO projects (id, name, description, icon, workspace_path) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(icon)
    .bind(new_project.workspace_path.as_deref())
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// 创建项目（含一次性写入初始 Agent 成员）
///
/// 事务保证：INSERT projects + N 条 INSERT project_agents 要么全成功，要么全回滚。
pub async fn create_with_agents(
    pool: &SqlitePool,
    id: &str,
    new_project: &NewProject,
) -> AppResult<ProjectRow> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO projects (id, name, description, icon, workspace_path)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&new_project.name)
    .bind(new_project.description.as_deref().unwrap_or(""))
    .bind(new_project.icon.as_deref().unwrap_or("folder"))
    .bind(new_project.workspace_path.as_deref())
    .execute(&mut *tx)
    .await?;

    for member in &new_project.agents {
        let role = if member.role == "lead" { "lead" } else { "member" };
        sqlx::query(
            "INSERT INTO project_agents (project_id, agent_id, role)
             VALUES (?, ?, ?)",
        )
        .bind(id)
        .bind(&member.agent_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    get_by_id(pool, id).await
}

/// 部分更新项目 + 可选替换成员（双层 Option + 原子事务）
///
/// 行为：
/// - `patch.name = Some(s)`              → UPDATE name=s
/// - `patch.description = Some(None)`    → UPDATE description=''（列定义 NOT NULL DEFAULT ''）
/// - `patch.description = Some(Some(s))` → UPDATE description=s
/// - `patch.description = None`          → 不更新 description
/// - `members = None`                    → 不动 project_agents 表
/// - `members = Some(vec)`               → 事务内 DELETE + INSERT project_agents
///
/// 校验：
/// - 若传了 name，必须 trim 非空
/// - 若传了 members，去重校验（agent_id 不能重复）
pub async fn update_project_full(
    pool: &SqlitePool,
    id: &str,
    patch: &ProjectPatch,
    members: Option<&[ProjectAgentInput]>,
) -> AppResult<ProjectRow> {
    let mut tx = pool.begin().await?;
    let mut any_update = false;

    if let Some(n) = &patch.name {
        if n.trim().is_empty() {
            return Err(AppError::Validation("项目名称不能为空".into()));
        }
        let affected = sqlx::query(
            "UPDATE projects SET name = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(n)
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected == 0 {
            return Err(AppError::NotFound {
                resource: "project",
                id: id.to_string(),
            });
        }
        any_update = true;
    }

    // ⚠️ description 列 DDL: TEXT NOT NULL DEFAULT ''
    // Some(None) → 写入空字符串 ''，不是 NULL
    if let Some(d) = &patch.description {
        let affected = sqlx::query(
            "UPDATE projects SET description = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(d.as_deref().unwrap_or(""))
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected == 0 {
            return Err(AppError::NotFound {
                resource: "project",
                id: id.to_string(),
            });
        }
        any_update = true;
    }

    if let Some(i) = &patch.icon {
        let affected = sqlx::query(
            "UPDATE projects SET icon = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(i.as_deref().unwrap_or("folder"))
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected == 0 {
            return Err(AppError::NotFound {
                resource: "project",
                id: id.to_string(),
            });
        }
        any_update = true;
    }

    if let Some(w) = &patch.workspace_path {
        let affected = sqlx::query(
            "UPDATE projects SET workspace_path = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(w.as_deref())
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected == 0 {
            return Err(AppError::NotFound {
                resource: "project",
                id: id.to_string(),
            });
        }
        any_update = true;
    }

    // 原子处理成员替换（与字段更新在同一事务内）
    if let Some(members) = members {
        // ⚠️ command 层负责去重校验，repo 层信任输入
        sqlx::query("DELETE FROM project_agents WHERE project_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        for m in members {
            let role = if m.role == "lead" { "lead" } else { "member" };
            sqlx::query(
                "INSERT INTO project_agents (project_id, agent_id, role)
                 VALUES (?, ?, ?)",
            )
            .bind(id)
            .bind(&m.agent_id)
            .bind(role)
            .execute(&mut *tx)
            .await?;
        }
        any_update = true;
    }

    // 注意：即使 patch/members 全是 None（空更新）也不报错。
    if !any_update {
        drop(tx); // 自动 rollback 行为无害
        return get_by_id(pool, id).await;
    }

    tx.commit().await?;
    get_by_id(pool, id).await
}

/// 部分更新项目（仅字段，不含成员）
///
/// 使用事务保证原子性：name 和 description 要么同时成功，要么同时回滚。
///
/// **保留** 作向后兼容；新流程走 `update_project_full`。
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
) -> AppResult<ProjectRow> {
    let mut tx = pool.begin().await?;

    if let Some(n) = name {
        let affected = sqlx::query("UPDATE projects SET name = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(n)
            .bind(id)
            .execute(&mut *tx)
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
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::NotFound {
                resource: "project",
                id: id.to_string(),
            });
        }
    }

    tx.commit().await?;
    get_by_id(pool, id).await
}

/// 删除项目
///
/// DDL 已配置级联策略（见 13_projects.sql）：
/// - project_agents: ON DELETE CASCADE（自动删除关联成员）
/// - conversations: ON DELETE SET NULL（会话自动回到默认项目）
///
/// ⚠️ 前提：SQLite 连接初始化时已执行 `PRAGMA foreign_keys = ON;`，
/// 否则级联策略不生效。若未启用 foreign_keys，需在 repo 层显式事务处理。
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

/// 整体替换项目的 Agent 成员（独立入口，供 set_project_agents command 调用）
///
/// 事务内：DELETE 旧 → 批量 INSERT 新，保留 lead/member 顺序。
/// 注意：使用普通 INSERT 而非 INSERT OR IGNORE（command 层已做去重校验）。
pub async fn replace_agents(
    pool: &SqlitePool,
    project_id: &str,
    members: &[ProjectAgentInput],
) -> AppResult<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM project_agents WHERE project_id = ?")
        .bind(project_id)
        .execute(&mut *tx)
        .await?;

    for m in members {
        let role = if m.role == "lead" { "lead" } else { "member" };
        sqlx::query(
            "INSERT INTO project_agents (project_id, agent_id, role)
             VALUES (?, ?, ?)",
        )
        .bind(project_id)
        .bind(&m.agent_id)
        .bind(role)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// 添加 Agent 到项目（INSERT OR IGNORE 保证幂等）
pub async fn add_agent(
    pool: &SqlitePool,
    project_id: &str,
    agent_id: &str,
    role: &str,
) -> AppResult<()> {
    let role = if role == "lead" { "lead" } else { "member" };
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
///
/// 如果记录不存在，返回 `NotFound` 错误（而非静默成功）。
pub async fn remove_agent(pool: &SqlitePool, project_id: &str, agent_id: &str) -> AppResult<()> {
    let affected = sqlx::query("DELETE FROM project_agents WHERE project_id = ? AND agent_id = ?")
        .bind(project_id)
        .bind(agent_id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "project_agent",
            id: format!("{}::{}", project_id, agent_id),
        });
    }
    Ok(())
}
