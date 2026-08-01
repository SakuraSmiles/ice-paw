//! 项目管理 Tauri Commands
//!
//! - 项目 CRUD + 成员管理
//! - 项目内会话查询/移动（复用 repo::conversation）
//!
//! DB schema 已由 migration 13/14/21 建好，本模块补命令层。

use sqlx::SqlitePool;
use tauri::State;
use uuid::Uuid;

use crate::db::models::{Conversation, NewProject, Project, ProjectRow, UpdateProject};
use crate::db::repo;
use crate::error::AppResult;

/// 列出全部项目（含 agent 成员）
#[tauri::command]
pub async fn list_projects(pool: State<'_, SqlitePool>) -> AppResult<Vec<Project>> {
    let rows = repo::project::list(pool.inner()).await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let agents = repo::project::list_agents(pool.inner(), &row.id).await?;
        result.push(Project { row, agents });
    }
    Ok(result)
}

/// 创建项目（含初始成员）
#[tauri::command]
pub async fn create_project(pool: State<'_, SqlitePool>, input: NewProject) -> AppResult<ProjectRow> {
    let id = Uuid::new_v4().to_string();
    repo::project::create(pool.inner(), &input, &id).await
}

/// 更新项目（partial update）
#[tauri::command]
pub async fn update_project(pool: State<'_, SqlitePool>, input: UpdateProject) -> AppResult<ProjectRow> {
    repo::project::update(pool.inner(), &input).await
}

/// 删除项目（CASCADE 删成员；conversations.project_id 自动 SET NULL）
#[tauri::command]
pub async fn delete_project(pool: State<'_, SqlitePool>, id: String) -> AppResult<()> {
    repo::project::delete(pool.inner(), &id).await
}

/// 批量更新排序
#[tauri::command]
pub async fn reorder_projects(pool: State<'_, SqlitePool>, ids: Vec<String>) -> AppResult<()> {
    repo::project::reorder(pool.inner(), &ids).await
}

/// 全量替换项目成员
#[tauri::command]
pub async fn set_project_agents(
    pool: State<'_, SqlitePool>,
    project_id: String,
    members: Vec<(String, String)>,
) -> AppResult<()> {
    repo::project::set_agents(pool.inner(), &project_id, &members).await
}

/// 添加单个成员
#[tauri::command]
pub async fn add_project_agent(
    pool: State<'_, SqlitePool>,
    project_id: String,
    agent_id: String,
    role: Option<String>,
) -> AppResult<()> {
    let r = role.as_deref().unwrap_or("member");
    repo::project::add_agent(pool.inner(), &project_id, &agent_id, r).await
}

/// 移除单个成员
#[tauri::command]
pub async fn remove_project_agent(
    pool: State<'_, SqlitePool>,
    project_id: String,
    agent_id: String,
) -> AppResult<()> {
    repo::project::remove_agent(pool.inner(), &project_id, &agent_id).await
}

/// 列出项目内的会话（project_id=null → 散落会话）
#[tauri::command]
pub async fn list_conversations_by_project(
    pool: State<'_, SqlitePool>,
    project_id: Option<String>,
) -> AppResult<Vec<Conversation>> {
    let rows = repo::conversation::list_by_project(pool.inner(), project_id.as_deref()).await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

/// 移动会话到项目（project_id=null → 移出项目变散落）
#[tauri::command]
pub async fn move_conversation_to_project(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
    project_id: Option<String>,
) -> AppResult<()> {
    repo::conversation::move_to_project(pool.inner(), &conversation_id, project_id.as_deref()).await
}

/// 归档项目（软删除：从活跃列表收起，会话不动、不丢、不混入散落）
#[tauri::command]
pub async fn archive_project(pool: State<'_, SqlitePool>, id: String) -> AppResult<()> {
    repo::project::set_archived(pool.inner(), &id, true).await
}

/// 恢复归档项目（原样回到活跃列表，会话可见）
#[tauri::command]
pub async fn unarchive_project(pool: State<'_, SqlitePool>, id: String) -> AppResult<()> {
    repo::project::set_archived(pool.inner(), &id, false).await
}

/// 永久删除项目：delete_conversations=true 连同该项目会话一起删；
/// false 则会话转为散落（conversations.project_id ON DELETE SET NULL）。
#[tauri::command]
pub async fn permanent_delete_project(
    pool: State<'_, SqlitePool>,
    id: String,
    delete_conversations: bool,
) -> AppResult<()> {
    repo::project::permanent_delete(pool.inner(), &id, delete_conversations).await
}
