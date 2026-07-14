//! Agent 相关 Tauri Commands
//!
//! Frontend 调用入口见 `icepaw-cleanup-plan.md` §2.3。

use tauri::{AppHandle, State};
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::crypto;
use crate::db::models::{Agent, AgentRow, AgentUpdate, NewAgent, RotateAgentKey};
use crate::db::repo;
use crate::error::{AppError, AppResult};

/// 列出全部 agent（不含敏感字段）
#[tauri::command]
pub async fn list_agents(state: State<'_, SqlitePool>) -> AppResult<Vec<Agent>> {
    let rows = repo::agent::list(state.inner()).await?;
    Ok(rows.into_iter().map(Agent::from).collect())
}

/// 创建 agent（含 api_key 写入 stronghold）
#[tauri::command]
pub async fn create_agent(
    app: AppHandle,
    state: State<'_, SqlitePool>,
    input: NewAgent,
) -> AppResult<Agent> {
    // 入参基础校验
    if input.name.trim().is_empty() {
        return Err(AppError::Validation("name 不能为空".into()));
    }
    if input.api_key.trim().is_empty() {
        return Err(AppError::Validation("api_key 不能为空".into()));
    }

    let id = Uuid::new_v4().to_string();
    // stronghold 引用 key = agent_id（M2 约定：api_key_ref 与 id 同值）
    crypto::store_api_key(&app, &id, &input.api_key, input.base_url.as_deref())?;

    let row: AgentRow = repo::agent::create(state.inner(), &input, &id, &id).await?;
    Ok(Agent::from(row))
}

/// 部分更新 agent
#[tauri::command]
pub async fn update_agent(
    state: State<'_, SqlitePool>,
    input: AgentUpdate,
) -> AppResult<Agent> {
    let row = repo::agent::update(
        state.inner(),
        &input.id,
        input.name.as_deref(),
        input.provider.as_deref(),
        input.model.as_deref(),
        input.system_prompt.as_deref(),
        input.base_url.as_ref().map(|opt| opt.as_deref()),
        input.temperature,
        input.max_tokens,
        input.extra_params.as_ref(),
        input.sort_order,
        input.cache_prompt,
    )
    .await?;
    Ok(Agent::from(row))
}

/// 单独轮换 api_key（避免误改其他字段）
#[tauri::command]
pub async fn rotate_agent_api_key(
    app: AppHandle,
    state: State<'_, SqlitePool>,
    input: RotateAgentKey,
) -> AppResult<Agent> {
    if input.api_key.trim().is_empty() {
        return Err(AppError::Validation("api_key 不能为空".into()));
    }
    crypto::store_api_key(&app, &input.agent_id, &input.api_key, input.base_url.as_deref())?;
    repo::agent::rotate_key_ref(
        state.inner(),
        &input.agent_id,
        &input.agent_id,
        input.base_url.as_deref(),
    )
    .await?;
    // 取回带敏感字段的 row 转 Agent
    let row = repo::agent::get_by_id(state.inner(), &input.agent_id).await?;
    Ok(Agent::from(row))
}

/// 删除 agent（级联清理 conversations + messages）
#[tauri::command]
pub async fn delete_agent(
    app: AppHandle,
    state: State<'_, SqlitePool>,
    id: String,
) -> AppResult<()> {
    // 先清 stronghold 中的 key（容错忽略）
    let _ = crypto::delete_api_key(&app, &id);
    repo::agent::delete(state.inner(), &id).await
}
