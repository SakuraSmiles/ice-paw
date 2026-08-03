//! `mcp_servers` 表的 SQL 操作
//!
//! Phase 2: 外部 MCP Server 配置的 CRUD。

use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};
use crate::harness::mcp::types::{McpServerConfig, NewMcpServer, UpdateMcpServer, TrustLevel};

const ALL_COLS: &str = "id, name, description, command, args, env, enabled, trust_level, scope, created_at, updated_at";

/// 列出全部 MCP Server 配置，按 created_at 降序
pub async fn list_all(pool: &SqlitePool) -> AppResult<Vec<McpServerConfig>> {
    let rows = sqlx::query_as::<_, McpServerRow>(
        &format!("SELECT {} FROM mcp_servers ORDER BY created_at DESC", ALL_COLS),
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

/// 按 id 取一条
pub async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<McpServerConfig> {
    let row = sqlx::query_as::<_, McpServerRow>(
        &format!("SELECT {} FROM mcp_servers WHERE id = ?", ALL_COLS),
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "mcp_server",
        id: id.to_string(),
    })?;
    Ok(row.into())
}

/// 创建 MCP Server 配置
pub async fn create(pool: &SqlitePool, input: &NewMcpServer) -> AppResult<McpServerConfig> {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let args_str = serde_json::to_string(&input.args)?;
    let env_str = input.env.as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .unwrap_or_else(|| "{}".to_string());

    sqlx::query(
        "INSERT INTO mcp_servers (id, name, description, command, args, env, enabled, trust_level, scope, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.id)
    .bind(&input.name)
    .bind(&input.description)
    .bind(&input.command)
    .bind(&args_str)
    .bind(&env_str)
    .bind(input.enabled as i32)
    .bind(input.trust_level.as_str())
    .bind(&input.scope)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    get_by_id(pool, &input.id).await
}

/// 更新 MCP Server 配置（partial update）
pub async fn update(pool: &SqlitePool, input: &UpdateMcpServer) -> AppResult<McpServerConfig> {
    let existing = get_by_id(pool, &input.id).await?;
    let name = input.name.as_deref().unwrap_or(&existing.name);
    let desc = input.description.as_deref().unwrap_or(&existing.description);
    let cmd = input.command.as_deref().unwrap_or(&existing.command);
    let args = input.args.as_ref().unwrap_or(&existing.args);
    let env = input.env.as_ref().unwrap_or(&existing.env);
    let enabled = input.enabled.unwrap_or(existing.enabled);
    let trust_level = input.trust_level.unwrap_or(existing.trust_level);
    let scope = input.scope.clone().unwrap_or_else(|| existing.scope.clone());
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let args_str = serde_json::to_string(args)?;
    let env_str = serde_json::to_string(env)?;

    sqlx::query(
        "UPDATE mcp_servers SET name=?, description=?, command=?, args=?, env=?, enabled=?, trust_level=?, scope=?, updated_at=? WHERE id=?",
    )
    .bind(name)
    .bind(desc)
    .bind(cmd)
    .bind(&args_str)
    .bind(&env_str)
    .bind(enabled as i32)
    .bind(trust_level.as_str())
    .bind(&scope)
    .bind(&now)
    .bind(&input.id)
    .execute(pool)
    .await?;

    get_by_id(pool, &input.id).await
}

/// 删除 MCP Server 配置
pub async fn delete(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let affected = sqlx::query("DELETE FROM mcp_servers WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "mcp_server",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 默认 MCP Server 配置列表——首次启动时自动安装。
/// 要求：命令用 npx（跨机器通用），不需要 API Key 即可运行。
fn default_mcp_servers() -> Vec<NewMcpServer> {
    vec![
        NewMcpServer {
            id: "builtin-filesystem".into(),
            name: "文件系统工具集".into(),
            description: "文件读写、目录浏览、搜索替换".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@anthropic-ai/mcp-server-filesystem".into(), "{workspace}".into()],
            env: Some(serde_json::json!({})),
            enabled: true,
            trust_level: TrustLevel::Trusted,
            scope: "per_agent".into(),
        },
        NewMcpServer {
            id: "builtin-sqlite".into(),
            name: "SQLite 查询".into(),
            description: "查询本地 SQLite 数据库".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-sqlite".into(), "--db-path".into(), "{workspace}/data.db".into()],
            env: Some(serde_json::json!({})),
            enabled: true,
            trust_level: TrustLevel::Trusted,
            scope: "per_agent".into(),
        },
        NewMcpServer {
            id: "builtin-thinking".into(),
            name: "深度推理".into(),
            description: "多步推理引擎，复杂问题分拆逐步思考".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-sequential-thinking".into()],
            env: Some(serde_json::json!({})),
            enabled: true,
            trust_level: TrustLevel::Trusted,
            scope: "per_agent".into(),
        },
        NewMcpServer {
            id: "builtin-memory".into(),
            name: "知识图谱记忆".into(),
            description: "持久化知识图谱，跨会话记忆实体和关系".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-memory".into()],
            env: Some(serde_json::json!({})),
            enabled: true,
            trust_level: TrustLevel::Trusted,
            scope: "per_agent".into(),
        },
    ]
}

/// 首次启动时种子：如果 mcp_servers 表为空，插入所有默认配置。
/// 幂等——已有配置时不做任何修改。
pub async fn seed_defaults(pool: &SqlitePool) -> AppResult<()> {
    let existing = list_all(pool).await?;
    if !existing.is_empty() {
        return Ok(());
    }
    tracing::info!(target: "ice_paw.mcp", "插入默认 MCP Server 配置");
    for cfg in &default_mcp_servers() {
        create(pool, cfg).await?;
    }
    Ok(())
}

// =========================================================================
// 内部行类型（DB 原始格式 → McpServerConfig）
// =========================================================================

#[derive(sqlx::FromRow)]
struct McpServerRow {
    id: String,
    name: String,
    description: String,
    command: String,
    args: String,
    env: String,
    enabled: i32,
    trust_level: String,
    scope: String,
    created_at: String,
    updated_at: String,
}

impl From<McpServerRow> for McpServerConfig {
    fn from(row: McpServerRow) -> Self {
        McpServerConfig {
            id: row.id,
            name: row.name,
            description: row.description,
            command: row.command,
            args: serde_json::from_str(&row.args).unwrap_or_default(),
            env: serde_json::from_str(&row.env).unwrap_or(serde_json::json!({})),
            enabled: row.enabled != 0,
            trust_level: row.trust_level.parse::<TrustLevel>().unwrap_or_default(),
            scope: row.scope,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
