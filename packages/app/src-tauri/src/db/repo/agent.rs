//! `agents` 表的 SQL 操作
//!
//! 约定：
//! - `api_key_ref` 仅作引用 key（默认 = agent_id），密文存在 stronghold 里
//! - 上层负责调用 strongbold 写入/删除；本模块只更新 DB 这一行

use sqlx::SqlitePool;

use crate::db::models::{AgentRow, NewAgent};
use crate::error::{AppError, AppResult};

/// 列出全部 agent，按 sort_order asc, created_at asc
pub async fn list(pool: &SqlitePool) -> AppResult<Vec<AgentRow>> {
    let rows = sqlx::query_as::<_, AgentRow>(
        "SELECT id, name, provider, model, system_prompt, api_key_ref, base_url,
                temperature, max_tokens, extra_params, sort_order, cache_prompt,
                max_history_messages, tool_trim_threshold, enabled_tools,
                supports_vision, embedding_model, description, avatar,
                created_at, updated_at
           FROM agents
          ORDER BY sort_order ASC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 按 id 取一条；找不到返回 `AppError::NotFound`
pub async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<AgentRow> {
    let row = sqlx::query_as::<_, AgentRow>(
        "SELECT id, name, provider, model, system_prompt, api_key_ref, base_url,
                temperature, max_tokens, extra_params, sort_order, cache_prompt,
                max_history_messages, tool_trim_threshold, enabled_tools,
                supports_vision, embedding_model, description, avatar,
                created_at, updated_at
           FROM agents WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "agent",
        id: id.to_string(),
    })?;
    Ok(row)
}

/// 创建 agent，id 由调用方（通常是 uuid）生成，api_key_ref 同时填好（默认 = id）
pub async fn create(
    pool: &SqlitePool,
    new_agent: &NewAgent,
    id: &str,
    api_key_ref: &str,
) -> AppResult<AgentRow> {
    let extra = new_agent
        .extra_params
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));
    let extra_str = serde_json::to_string(&extra)?;
    // P2-3: bool → i32 (0/1) for SQLite storage
    let cache_prompt_i = if new_agent.cache_prompt { 1i32 } else { 0i32 };
    let supports_vision_i = if new_agent.supports_vision { 1i32 } else { 0i32 };

    sqlx::query(
        "INSERT INTO agents
           (id, name, provider, model, system_prompt, api_key_ref, base_url,
            temperature, max_tokens, extra_params, sort_order, cache_prompt,
            max_history_messages, tool_trim_threshold, enabled_tools, supports_vision)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&new_agent.name)
    .bind(&new_agent.provider)
    .bind(&new_agent.model)
    .bind(&new_agent.system_prompt)
    .bind(api_key_ref)
    .bind(new_agent.base_url.as_deref())
    .bind(new_agent.temperature)
    .bind(new_agent.max_tokens)
    .bind(&extra_str)
    .bind(new_agent.sort_order)
    .bind(cache_prompt_i)
    .bind(new_agent.max_history_messages)
    .bind(new_agent.tool_trim_threshold)
    .bind(new_agent.enabled_tools.as_ref().map(|v| serde_json::to_string(v).unwrap_or_default()))
    .bind(supports_vision_i)
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// 部分更新（partial update）：None 字段不动
///
/// `max_history_messages` 字段语义与 `base_url` 一致：
///   - 外层 `None` → 调用方没传，不更新
///   - 外层 `Some(None)` → 调用方传了但要清空（恢复为系统默认）
///   - 外层 `Some(Some(N))` → 设成 N
#[allow(clippy::too_many_arguments)]
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    name: Option<&str>,
    provider: Option<&str>,
    model: Option<&str>,
    system_prompt: Option<&str>,
    base_url: Option<Option<&str>>,
    temperature: Option<f64>,
    max_tokens: Option<i32>,
    extra_params: Option<&serde_json::Value>,
    sort_order: Option<i32>,
    cache_prompt: Option<bool>,
    max_history_messages: Option<Option<i32>>,
    tool_trim_threshold: Option<Option<i32>>,
    enabled_tools: Option<Option<Vec<String>>>,
    supports_vision: Option<bool>,
) -> AppResult<AgentRow> {
    // 先读出来再合并，避免拼接动态 SQL
    let mut current = get_by_id(pool, id).await?;

    if let Some(v) = name { current.name = v.to_string(); }
    if let Some(v) = provider { current.provider = v.to_string(); }
    if let Some(v) = model { current.model = v.to_string(); }
    if let Some(v) = system_prompt { current.system_prompt = v.to_string(); }
    if let Some(v) = base_url { current.base_url = v.map(String::from); }
    if let Some(v) = temperature { current.temperature = v; }
    if let Some(v) = max_tokens { current.max_tokens = v; }
    if let Some(v) = sort_order { current.sort_order = v; }
    if let Some(v) = extra_params { current.extra_params = serde_json::to_string(v)?; }
    // P2-3: bool → i32 (0/1)
    if let Some(v) = cache_prompt { current.cache_prompt = if v { 1 } else { 0 }; }
    // A3-2: 双层 Option 语义（None=不改 / Some(None)=清空 / Some(Some(N))=设定）
    if let Some(v) = max_history_messages { current.max_history_messages = v; }
    // M1.2 A2-4: 双层 Option 语义（同 max_history_messages）
    if let Some(v) = tool_trim_threshold { current.tool_trim_threshold = v; }
    // Task 4: 双层 Option 语义（None=不改 / Some(None)=清空即全部启用 / Some(Some(vec))=设定白名单）
    if let Some(v) = enabled_tools {
        current.enabled_tools = v.map(|names| serde_json::to_string(&names).unwrap_or_default());
    }
    if let Some(v) = supports_vision {
        current.supports_vision = if v { 1 } else { 0 };
    }

    sqlx::query(
        "UPDATE agents
            SET name = ?, provider = ?, model = ?, system_prompt = ?,
                base_url = ?, temperature = ?, max_tokens = ?, extra_params = ?, sort_order = ?,
                cache_prompt = ?, max_history_messages = ?, tool_trim_threshold = ?,
                enabled_tools = ?, supports_vision = ?
          WHERE id = ?",
    )
    .bind(&current.name)
    .bind(&current.provider)
    .bind(&current.model)
    .bind(&current.system_prompt)
    .bind(&current.base_url)
    .bind(current.temperature)
    .bind(current.max_tokens)
    .bind(&current.extra_params)
    .bind(current.sort_order)
    .bind(current.cache_prompt)
    .bind(current.max_history_messages)
    .bind(current.tool_trim_threshold)
    .bind(&current.enabled_tools)
    .bind(current.supports_vision)
    .bind(id)
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// 仅更新 api_key_ref 和 base_url（来自 rotate_agent_api_key 命令）
pub async fn rotate_key_ref(
    pool: &SqlitePool,
    id: &str,
    api_key_ref: &str,
    base_url: Option<&str>,
) -> AppResult<()> {
    let affected = sqlx::query(
        "UPDATE agents SET api_key_ref = ?, base_url = ? WHERE id = ?",
    )
    .bind(api_key_ref)
    .bind(base_url)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "agent",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 删除 agent（依赖外键 CASCADE 自动清理 conversations / messages）
pub async fn delete(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let affected = sqlx::query("DELETE FROM agents WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "agent",
            id: id.to_string(),
        });
    }
    Ok(())
}
