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
                max_history_messages, enabled_tools, context_window,
                supports_vision, description, avatar,
                workspace_path, created_at, updated_at
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
                max_history_messages, enabled_tools, context_window,
                supports_vision, description, avatar,
                workspace_path, created_at, updated_at
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
    let supports_vision_i = if new_agent.supports_vision {
        1i32
    } else {
        0i32
    };

    sqlx::query(
        "INSERT INTO agents
           (id, name, provider, model, system_prompt, api_key_ref, base_url,
            temperature, max_tokens, extra_params, sort_order, cache_prompt,
            max_history_messages, enabled_tools, supports_vision,
            workspace_path, context_window, avatar)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(
        new_agent
            .enabled_tools
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_default()),
    )
    .bind(supports_vision_i)
    .bind(new_agent.workspace_path.as_deref())
    .bind(new_agent.context_window)
    .bind(new_agent.avatar.as_deref())
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// 部分更新（partial update）：None 字段不动
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
    context_window: Option<Option<i32>>,
    enabled_tools: Option<Option<Vec<String>>>,
    supports_vision: Option<bool>,
    workspace_path: Option<Option<&str>>,
    avatar: Option<Option<&str>>,
) -> AppResult<AgentRow> {
    // 先读出来再合并，避免拼接动态 SQL
    let mut current = get_by_id(pool, id).await?;

    if let Some(v) = name {
        current.name = v.to_string();
    }
    if let Some(v) = provider {
        current.provider = v.to_string();
    }
    if let Some(v) = model {
        current.model = v.to_string();
    }
    if let Some(v) = system_prompt {
        current.system_prompt = v.to_string();
    }
    if let Some(v) = base_url {
        current.base_url = v.map(String::from);
    }
    if let Some(v) = temperature {
        current.temperature = v;
    }
    if let Some(v) = max_tokens {
        current.max_tokens = v;
    }
    if let Some(v) = sort_order {
        current.sort_order = v;
    }
    if let Some(v) = extra_params {
        current.extra_params = serde_json::to_string(v)?;
    }
    // P2-3: bool → i32 (0/1)
    if let Some(v) = cache_prompt {
        current.cache_prompt = if v { 1 } else { 0 };
    }
    // A3-2: 双层 Option 语义（None=不改 / Some(None)=清空 / Some(Some(N))=设定）
    if let Some(v) = max_history_messages {
        current.max_history_messages = v;
    }
    // Phase 0: 双层 Option 语义（None=不改 / Some(None)=清空 / Some(Some(n))=设定）
    if let Some(v) = context_window {
        current.context_window = v;
    }
    // Task 4: 双层 Option 语义（None=不改 / Some(None)=清空即全部启用 / Some(Some(vec))=设定白名单）
    if let Some(v) = enabled_tools {
        current.enabled_tools = v.map(|names| serde_json::to_string(&names).unwrap_or_default());
    }
    if let Some(v) = supports_vision {
        current.supports_vision = if v { 1 } else { 0 };
    }
    // Phase 3: 双层 Option（None=不改 / Some(None)=清空 / Some(Some)=设定）
    if let Some(v) = workspace_path {
        current.workspace_path = v.map(String::from);
    }
    // 头像双层 Option（None=不改 / Some(None)=清空 / Some(Some)=设定）
    if let Some(v) = avatar {
        current.avatar = v.map(String::from);
    }

    sqlx::query(
        "UPDATE agents
            SET name = ?, provider = ?, model = ?, system_prompt = ?,
                base_url = ?, temperature = ?, max_tokens = ?, extra_params = ?, sort_order = ?,
                cache_prompt = ?, max_history_messages = ?,
                enabled_tools = ?, supports_vision = ?, workspace_path = ?, context_window = ?,
                avatar = ?
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
    .bind(&current.enabled_tools)
    .bind(current.supports_vision)
    .bind(&current.workspace_path)
    .bind(current.context_window)
    .bind(&current.avatar)
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
    let affected = sqlx::query("UPDATE agents SET api_key_ref = ?, base_url = ? WHERE id = ?")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// in-memory SQLite + 全量 migrations
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

    fn new_agent(avatar: Option<&str>) -> NewAgent {
        NewAgent {
            id: "a1".into(),
            name: "测试".into(),
            provider: "openai".into(),
            model: "m".into(),
            system_prompt: String::new(),
            api_key: String::new(),
            base_url: None,
            temperature: 0.7,
            max_tokens: 4096,
            extra_params: None,
            sort_order: 0,
            cache_prompt: true,
            supports_vision: false,
            max_history_messages: None,
            context_window: None,
            enabled_tools: None,
            workspace_path: None,
            avatar: avatar.map(String::from),
        }
    }

    #[tokio::test]
    async fn avatar_roundtrip() {
        let pool = test_pool().await;
        // create 带头像（migration 20 列）
        create(&pool, &new_agent(Some("data:image/png;base64,xxx")), "a1", "a1")
            .await
            .expect("create");
        let row = get_by_id(&pool, "a1").await.expect("get");
        assert_eq!(row.avatar.as_deref(), Some("data:image/png;base64,xxx"));

        // 不带头像创建 → NULL（渲染层走渐变兜底）
        create(&pool, &new_agent(None), "a2", "a2")
            .await
            .expect("create 2");
        let row = get_by_id(&pool, "a2").await.expect("get 2");
        assert_eq!(row.avatar, None);
    }

    #[tokio::test]
    async fn avatar_update_double_option_semantics() {
        let pool = test_pool().await;
        create(&pool, &new_agent(None), "a1", "a1")
            .await
            .expect("create");

        // 全 None = 不改（name..avatar 17 项）
        let row = update(&pool, "a1", None, None, None, None, None, None, None, None, None,
            None, None, None, None, None, None, None)
            .await
            .expect("update no-op");
        assert_eq!(row.avatar, None);

        // Some(Some) = 设定
        let row = update(&pool, "a1", None, None, None, None, None, None, None, None, None,
            None, None, None, None, None, None, Some(Some("data:image/webp;base64,yyy")))
            .await
            .expect("update set");
        assert_eq!(row.avatar.as_deref(), Some("data:image/webp;base64,yyy"));

        // Some(None) = 清空
        let row = update(&pool, "a1", None, None, None, None, None, None, None, None, None,
            None, None, None, None, None, None, Some(None))
            .await
            .expect("update clear");
        assert_eq!(row.avatar, None);
    }

    /// 2026-08-31 生产实案回归：set_agent_enabled_tools 写 yaml 后镜像 DB 列依赖
    /// 本双层语义——只摘 yaml 不清 DB，旧白名单会在下次加载复活（工具静默缺失）。
    /// 摘除 → Some(None)=NULL（组装 filter 见 None 走全量）；收窄 → Some(Some)=同值 JSON。
    #[tokio::test]
    async fn enabled_tools_update_double_option_semantics() {
        let pool = test_pool().await;
        create(&pool, &new_agent(None), "a1", "a1").await.expect("create");
        let row = get_by_id(&pool, "a1").await.expect("get");
        assert_eq!(row.enabled_tools, None);

        // Some(Some) = 设定白名单（收窄镜像路径）
        let list = vec!["read_file".to_string(), "edit_docx".to_string()];
        let row = update(&pool, "a1", None, None, None, None, None, None, None, None, None,
            None, None, None, Some(Some(list)), None, None, None)
            .await
            .expect("update set");
        assert_eq!(row.enabled_tools.as_deref(), Some(r#"["read_file","edit_docx"]"#));

        // Some(None) = 清空即全部启用（摘除镜像路径）
        let row = update(&pool, "a1", None, None, None, None, None, None, None, None, None,
            None, None, None, Some(None), None, None, None)
            .await
            .expect("update clear");
        assert_eq!(row.enabled_tools, None);
    }
}
