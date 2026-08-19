//! `projects` + `project_agents` 表的 SQL 操作
//!
//! DB schema 已由 migration 13/14/21 建好。本模块补 Rust CRUD 层。

use sqlx::SqlitePool;

use crate::db::models::{NewProject, ProjectAgentRow, ProjectRow, UpdateProject};
use crate::error::{AppError, AppResult};

const PROJECT_COLS: &str = "id, name, description, icon, sort_order, workspace_path, theme_color, avatar, archived, created_at, updated_at";

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
        "INSERT INTO projects (id, name, description, icon, workspace_path, theme_color, avatar)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&input.name)
    .bind(input.description.as_deref().unwrap_or(""))
    .bind(input.icon.as_deref().unwrap_or("folder"))
    .bind(input.workspace_path.as_deref())
    .bind(input.theme_color.as_deref())
    .bind(input.avatar.as_deref())
    .execute(pool)
    .await?;

    // 插入初始成员（容错：外键违规等仅 warn，不阻断项目创建）
    for agent_id in &input.agent_ids {
        if let Err(e) = sqlx::query(
            "INSERT OR IGNORE INTO project_agents (project_id, agent_id, role) VALUES (?, ?, 'member')",
        )
        .bind(id)
        .bind(agent_id)
        .execute(pool)
        .await
        {
            tracing::warn!(target: "ice_paw.project",
                "创建项目 {id} 时添加成员 {agent_id} 失败: {e}");
        }
    }

    get_by_id(pool, id).await
}

/// partial update：None 字段不动；双层 Option 字段 None=不改 / Some(None)=清空 / Some(Some(v))=设定
pub async fn update(pool: &SqlitePool, input: &UpdateProject) -> AppResult<ProjectRow> {
    let mut current = get_by_id(pool, &input.id).await?;

    if let Some(v) = input.name.as_deref() {
        current.name = v.to_string();
    }
    if let Some(v) = input.description.as_deref() {
        current.description = v.to_string();
    }
    if let Some(v) = input.icon.as_deref() {
        current.icon = v.to_string();
    }
    // 双层 Option：None=不改, Some(None)=清空, Some(Some(v))=设定
    if let Some(v) = &input.workspace_path {
        current.workspace_path = v.clone();
    }
    if let Some(v) = &input.theme_color {
        current.theme_color = v.clone();
    }
    if let Some(v) = &input.avatar {
        current.avatar = v.clone();
    }

    sqlx::query(
        "UPDATE projects SET name=?, description=?, icon=?, workspace_path=?, theme_color=?, avatar=?, updated_at=datetime('now') WHERE id=?",
    )
    .bind(&current.name)
    .bind(&current.description)
    .bind(&current.icon)
    .bind(&current.workspace_path)
    .bind(&current.theme_color)
    .bind(&current.avatar)
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
///
/// 整个操作在事务中执行——若删项目失败，会话不会丢。
pub async fn permanent_delete(
    pool: &SqlitePool,
    id: &str,
    delete_conversations: bool,
) -> AppResult<()> {
    let mut txn = pool.begin().await?;
    if delete_conversations {
        sqlx::query("DELETE FROM conversations WHERE project_id = ?")
            .bind(id)
            .execute(&mut *txn)
            .await?;
    }
    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(id)
        .execute(&mut *txn)
        .await?;
    txn.commit().await?;
    Ok(())
}

/// 批量更新排序（事务内全成功或全回滚）
pub async fn reorder(pool: &SqlitePool, ids: &[String]) -> AppResult<()> {
    let mut txn = pool.begin().await?;
    for (i, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE projects SET sort_order = ? WHERE id = ?")
            .bind(i as i32)
            .bind(id)
            .execute(&mut *txn)
            .await?;
    }
    txn.commit().await?;
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

/// 一次性查出所有项目的 agent 关联，按 project_id 分组。
///
/// 替代 list_projects 里逐项目 N+1 的模式——两次查询代替 1+N 次。
pub async fn list_all_agents_grouped(
    pool: &SqlitePool,
) -> AppResult<std::collections::HashMap<String, Vec<ProjectAgentRow>>> {
    let rows = sqlx::query_as::<_, ProjectAgentRow>(
        "SELECT project_id, agent_id, role, joined_at FROM project_agents ORDER BY project_id, joined_at",
    )
    .fetch_all(pool)
    .await?;
    let mut map: std::collections::HashMap<String, Vec<ProjectAgentRow>> =
        std::collections::HashMap::new();
    for row in rows {
        map.entry(row.project_id.clone()).or_default().push(row);
    }
    Ok(map)
}

/// 全量替换项目成员（先删后插，事务保证原子性）。
///
/// 若任一 INSERT 失败，整个操作回滚（DELETE 不生效），原成员原样保留。
pub async fn set_agents(
    pool: &SqlitePool,
    project_id: &str,
    members: &[(String, String)],
) -> AppResult<()> {
    let mut txn = pool.begin().await?;
    sqlx::query("DELETE FROM project_agents WHERE project_id = ?")
        .bind(project_id)
        .execute(&mut *txn)
        .await?;
    for (agent_id, role) in members {
        sqlx::query("INSERT INTO project_agents (project_id, agent_id, role) VALUES (?, ?, ?)")
            .bind(project_id)
            .bind(agent_id)
            .bind(role)
            .execute(&mut *txn)
            .await?;
    }
    txn.commit().await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// in-memory SQLite + 全量 migrations（migration 48 引入 projects.avatar）
    async fn test_pool() -> SqlitePool {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        pool
    }

    fn new_project(avatar: Option<&str>) -> NewProject {
        NewProject {
            name: "测试项目".into(),
            description: None,
            icon: Some("🚀".into()),
            workspace_path: None,
            theme_color: None,
            avatar: avatar.map(String::from),
            agent_ids: vec![],
        }
    }

    #[tokio::test]
    async fn project_avatar_roundtrip() {
        let pool = test_pool().await;
        create(&pool, &new_project(Some("data:image/webp;base64,xxx")), "p1")
            .await
            .expect("create");
        let row = get_by_id(&pool, "p1").await.expect("get");
        assert_eq!(row.avatar.as_deref(), Some("data:image/webp;base64,xxx"));
        assert_eq!(row.icon.as_str(), "🚀");

        // 不带头像创建 → NULL（渲染层走渐变兜底）
        create(&pool, &new_project(None), "p2").await.expect("create 2");
        assert_eq!(get_by_id(&pool, "p2").await.expect("get 2").avatar, None);
    }

    #[tokio::test]
    async fn project_avatar_update_double_option_semantics() {
        let pool = test_pool().await;
        create(&pool, &new_project(None), "p1").await.expect("create");

        // 全 None = 不改
        let row = update(
            &pool,
            &UpdateProject {
                id: "p1".into(),
                name: None,
                description: None,
                icon: None,
                workspace_path: None,
                theme_color: None,
                avatar: None,
            },
        )
        .await
        .expect("update no-op");
        assert_eq!(row.avatar, None);

        // Some(Some) = 设定
        let row = update(
            &pool,
            &UpdateProject {
                id: "p1".into(),
                name: None,
                description: None,
                icon: None,
                workspace_path: None,
                theme_color: None,
                avatar: Some(Some("data:image/webp;base64,yyy".into())),
            },
        )
        .await
        .expect("update set");
        assert_eq!(row.avatar.as_deref(), Some("data:image/webp;base64,yyy"));

        // Some(None) = 清空
        let row = update(
            &pool,
            &UpdateProject {
                id: "p1".into(),
                name: None,
                description: None,
                icon: None,
                workspace_path: None,
                theme_color: None,
                avatar: Some(None),
            },
        )
        .await
        .expect("update clear");
        assert_eq!(row.avatar, None);
    }
}
