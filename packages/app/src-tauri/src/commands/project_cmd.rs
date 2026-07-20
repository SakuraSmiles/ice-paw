//! Project 相关 Tauri Commands（Phase 2）

use tauri::State;
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::db::models::{Conversation, NewProject, Project, ProjectMember};
use crate::db::repo;
use crate::error::{AppError, AppResult};

/// 列出全部项目（含每个项目下的 Agent 成员）
#[tauri::command]
pub async fn list_projects(pool: State<'_, SqlitePool>) -> AppResult<Vec<Project>> {
    let rows = repo::project::list(pool.inner()).await?;
    let mut projects: Vec<Project> = rows.into_iter().map(Project::from).collect();
    for proj in &mut projects {
        let agent_rows = repo::project::list_agents(pool.inner(), &proj.id).await?;
        proj.agents = agent_rows
            .into_iter()
            .map(|r| ProjectMember {
                agent_id: r.agent_id,
                role: r.role,
            })
            .collect();
    }
    Ok(projects)
}

/// 创建项目
#[tauri::command]
pub async fn create_project(
    pool: State<'_, SqlitePool>,
    input: NewProject,
) -> AppResult<Project> {
    if input.name.trim().is_empty() {
        return Err(AppError::Validation("项目名称不能为空".into()));
    }
    let id = Uuid::new_v4().to_string();
    let row = repo::project::create(pool.inner(), &id, &input).await?;
    Ok(Project::from(row))
}

/// 更新项目名称 / 描述
#[tauri::command]
pub async fn update_project(
    pool: State<'_, SqlitePool>,
    id: String,
    name: Option<String>,
    description: Option<String>,
) -> AppResult<Project> {
    // 校验：如果传了 name，不能为空字符串
    if let Some(ref n) = name {
        if n.trim().is_empty() {
            return Err(AppError::Validation("项目名称不能为空".into()));
        }
    }
    let row = repo::project::update(
        pool.inner(),
        &id,
        name.as_deref(),
        description.as_deref(),
    )
    .await?;
    Ok(Project::from(row))
}

/// 删除项目（conversations.project_id → NULL，project_agents CASCADE 删除）
#[tauri::command]
pub async fn delete_project(pool: State<'_, SqlitePool>, id: String) -> AppResult<()> {
    repo::project::delete(pool.inner(), &id).await
}

/// 排序（批量更新 sort_order）
#[tauri::command]
pub async fn reorder_projects(
    pool: State<'_, SqlitePool>,
    ordered_ids: Vec<String>,
) -> AppResult<()> {
    repo::project::reorder(pool.inner(), &ordered_ids).await
}

/// 添加 Agent 到项目
#[tauri::command]
pub async fn add_project_agent(
    pool: State<'_, SqlitePool>,
    project_id: String,
    agent_id: String,
    role: Option<String>,
) -> AppResult<()> {
    let role = role.as_deref().unwrap_or("member");
    // 校验 role 合法值
    if role != "lead" && role != "member" {
        return Err(AppError::Validation(format!(
            "无效的角色: {}，只支持 lead/member",
            role
        )));
    }
    repo::project::add_agent(pool.inner(), &project_id, &agent_id, role).await
}

/// 从项目移除 Agent
#[tauri::command]
pub async fn remove_project_agent(
    pool: State<'_, SqlitePool>,
    project_id: String,
    agent_id: String,
) -> AppResult<()> {
    repo::project::remove_agent(pool.inner(), &project_id, &agent_id).await
}

/// 列出某项目下的全部会话（project_id = None → 默认项目）
#[tauri::command]
pub async fn list_conversations_by_project(
    pool: State<'_, SqlitePool>,
    project_id: Option<String>,
) -> AppResult<Vec<Conversation>> {
    let rows =
        repo::conversation::list_by_project(pool.inner(), project_id.as_deref()).await?;
    Ok(rows.into_iter().map(Conversation::from).collect())
}

/// 移动会话到指定项目（project_id = None → 移回默认项目）
#[tauri::command]
pub async fn move_conversation_to_project(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
    project_id: Option<String>,
) -> AppResult<()> {
    repo::conversation::move_to_project(
        pool.inner(),
        &conversation_id,
        project_id.as_deref(),
    )
    .await
}
