//! Project 相关 Tauri Commands（Phase 2）

use tauri::State;
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::db::models::{Conversation, NewProject, ProjectAgentInput, Project, ProjectMember, ProjectPatch};
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

/// 创建项目（仅基础信息，不含 members）
///
/// **保留** 作向后兼容；新弹窗主流程走 `create_project_with_agents`。
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

/// 创建项目 + 一次性写入初始 Agent 成员（推荐入口）
///
/// 事务保证：项目基本信息与成员关联要么全部成功，要么全部回滚。
/// 失败时（如 name 为空）返回 Validation 错误，前端弹 Toast 即可。
#[tauri::command]
pub async fn create_project_with_agents(
    pool: State<'_, SqlitePool>,
    input: NewProject,
) -> AppResult<Project> {
    if input.name.trim().is_empty() {
        return Err(AppError::Validation("项目名称不能为空".into()));
    }

    // 校验 agents 中 agent_id 必须非空，且校验 role 合法性
    for m in &input.agents {
        if m.agent_id.trim().is_empty() {
            return Err(AppError::Validation("成员 agent_id 不能为空".into()));
        }
        if m.role != "lead" && m.role != "member" {
            return Err(AppError::Validation(format!(
                "无效的角色: {}，只支持 lead/member",
                m.role
            )));
        }
    }

    let id = Uuid::new_v4().to_string();
    let row = repo::project::create_with_agents(pool.inner(), &id, &input).await?;

    // 填充 agents 字段（命令层组装完整 Project 返回给前端）
    let mut project = Project::from(row);
    let agent_rows = repo::project::list_agents(pool.inner(), &project.id).await?;
    project.agents = agent_rows
        .into_iter()
        .map(|r| ProjectMember {
            agent_id: r.agent_id,
            role: r.role,
        })
        .collect();
    Ok(project)
}

/// 更新项目（partial update，双层 Option 语义）
///
/// **保留旧 command 不动**，旧调用方继续使用。
/// 新流程（编辑弹窗）走 `update_project_full`，原子处理字段+成员。
///
/// Rust 侧入参是 `ProjectPatch` 结构体，字段类型为 `Option<Option<T>>`：
/// - 字段缺失 → 不更新
/// - 字段为 null → 清空（description/icon 设为空串/默认值，workspace_path 设为 NULL）
/// - 字段为字符串 → 覆盖
///
/// ⚠️ **snake_case 要求**：前端 `invoke("update_project_full", ...)` 传参时，
/// 对象 key 必须用 snake_case（如 `project_id`、`agent_id`），
/// 因为 Tauri command 参数反序列化默认使用 snake_case。
#[tauri::command]
pub async fn update_project(
    pool: State<'_, SqlitePool>,
    id: String,
    name: Option<String>,
    description: Option<String>,
) -> AppResult<Project> {
    // 显式校验：传了 name 但为空
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

/// 原子更新项目（字段 + 可选成员替换，推荐入口）
///
/// 事务保证：项目字段更新与成员替换在同一事务内，要么全成功要么全回滚.
///
/// - `patch`：字段更新（双层 Option 语义）
/// - `members`：可选成员替换
///   - `None`：不动 project_agents 表
///   - `Some(vec)`：事务内 DELETE + INSERT（空数组 = 清空全部成员）
///
/// ⚠️ 前端 invoke 传参必须用 snake_case：
/// ```ts
/// invoke("update_project_full", {
///   id: "uuid",
///   patch: { name: "新名字" },
///   members: [{ agent_id: "x", role: "lead" }]
/// })
/// ```
#[tauri::command]
pub async fn update_project_full(
    pool: State<'_, SqlitePool>,
    id: String,
    patch: ProjectPatch,
    members: Option<Vec<ProjectAgentInput>>,
) -> AppResult<Project> {
    // 显式校验：传了 name 但为空
    if let Some(ref n) = patch.name {
        if n.trim().is_empty() {
            return Err(AppError::Validation("项目名称不能为空".into()));
        }
    }

    // 校验 members 中的 agent_id 不能重复（去重校验）
    if let Some(ref members) = members {
        let mut seen = std::collections::HashSet::new();
        for m in members {
            if m.agent_id.trim().is_empty() {
                return Err(AppError::Validation("成员 agent_id 不能为空".into()));
            }
            if m.role != "lead" && m.role != "member" {
                return Err(AppError::Validation(format!(
                    "无效的角色: {}，只支持 lead/member",
                    m.role
                )));
            }
            if !seen.insert(&m.agent_id) {
                return Err(AppError::Validation(format!(
                    "成员 agent_id 重复: {}",
                    m.agent_id
                )));
            }
        }
    }

    let row = repo::project::update_project_full(
        pool.inner(),
        &id,
        &patch,
        members.as_deref(),
    )
    .await?;

    // 组装完整 Project（含 agents 字段）
    let mut project = Project::from(row);
    let agent_rows = repo::project::list_agents(pool.inner(), &project.id).await?;
    project.agents = agent_rows
        .into_iter()
        .map(|r| ProjectMember {
            agent_id: r.agent_id,
            role: r.role,
        })
        .collect();
    Ok(project)
}

/// 编辑场景：整体替换项目的 Agent 成员
///
/// - 传空数组 → 清空所有成员
/// - 传非空数组 → 删除原成员后批量写入新成员（事务保证原子性）
///
/// 与 `add_project_agent` / `remove_project_agent` 的细粒度入口并存；
/// 弹窗主流程走本接口，避免多次 await 的 race 风险。
#[tauri::command]
pub async fn set_project_agents(
    pool: State<'_, SqlitePool>,
    project_id: String,
    agents: Vec<ProjectAgentInput>,
) -> AppResult<()> {
    // 校验项目存在
    let _ = repo::project::get_by_id(pool.inner(), &project_id).await?;

    // ⚠️ 去重校验：agent_id 不能重复（避免 INSERT 静默丢数据）
    let mut seen = std::collections::HashSet::new();
    for m in &agents {
        if m.agent_id.trim().is_empty() {
            return Err(AppError::Validation("成员 agent_id 不能为空".into()));
        }
        if m.role != "lead" && m.role != "member" {
            return Err(AppError::Validation(format!(
                "无效的角色: {}，只支持 lead/member",
                m.role
            )));
        }
        if !seen.insert(&m.agent_id) {
            return Err(AppError::Validation(format!(
                "成员 agent_id 重复: {}",
                m.agent_id
            )));
        }
    }

    repo::project::replace_agents(pool.inner(), &project_id, &agents).await
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

/// 添加 Agent 到项目（细粒度入口，弹窗主流程不走）
#[tauri::command]
pub async fn add_project_agent(
    pool: State<'_, SqlitePool>,
    project_id: String,
    agent_id: String,
    role: Option<String>,
) -> AppResult<()> {
    let role = role.as_deref().unwrap_or("member");
    if role != "lead" && role != "member" {
        return Err(AppError::Validation(format!(
            "无效的角色: {}，只支持 lead/member",
            role
        )));
    }
    repo::project::add_agent(pool.inner(), &project_id, &agent_id, role).await
}

/// 从项目移除 Agent（细粒度入口）
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
