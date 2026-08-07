//! `mcp_servers` 表的 SQL 操作
//!
//! Phase 2: 外部 MCP Server 配置的 CRUD。

use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};
use crate::harness::mcp::types::{McpServerConfig, NewMcpServer, RuntimeKind, UpdateMcpServer, TrustLevel};

const ALL_COLS: &str = "id, name, description, command, args, env, enabled, trust_level, scope, runtime_kind, created_at, updated_at";

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
        "INSERT INTO mcp_servers (id, name, description, command, args, env, enabled, trust_level, scope, runtime_kind, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(input.runtime_kind.as_str())
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
    let runtime_kind = input.runtime_kind.unwrap_or(existing.runtime_kind);
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let args_str = serde_json::to_string(args)?;
    let env_str = serde_json::to_string(env)?;

    sqlx::query(
        "UPDATE mcp_servers SET name=?, description=?, command=?, args=?, env=?, enabled=?, trust_level=?, scope=?, runtime_kind=?, updated_at=? WHERE id=?",
    )
    .bind(name)
    .bind(desc)
    .bind(cmd)
    .bind(&args_str)
    .bind(&env_str)
    .bind(enabled as i32)
    .bind(trust_level.as_str())
    .bind(&scope)
    .bind(runtime_kind.as_str())
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
///
/// 前 2 个（thinking / memory）用 **bundled 运行时**：IcePaw 内置 node.exe 与预打包
/// node_modules，零网络依赖、零系统 node 依赖。DB 里 command 存占位 "node"、
/// args 存「用户可配参数」（不含包名/入口），包名与 entry script 由 start_server
/// 解析时注入（见 harness::mcp::bundled）。
///
/// playwright 仍走 **system 运行时**（npx），依赖系统 node。
///
/// 注：文件操作（read / write / edit / delete / move_file / create_directory /
/// directory_tree / get_file_info / read_multiple_files / search_files）已由 native
/// 内置工具提供（见 harness::mcp::internal / file_tools / search），无需独立 MCP Server。
fn default_mcp_servers() -> Vec<NewMcpServer> {
    vec![
        NewMcpServer {
            id: "builtin-thinking".into(),
            name: "深度推理".into(),
            description: "多步推理引擎，复杂问题分拆逐步思考".into(),
            command: "node".into(),
            args: vec![],
            env: Some(serde_json::json!({})),
            enabled: true,
            trust_level: TrustLevel::Trusted,
            scope: "per_agent".into(),
            runtime_kind: RuntimeKind::Bundled,
        },
        NewMcpServer {
            id: "builtin-memory".into(),
            name: "知识图谱记忆".into(),
            description: "持久化知识图谱，跨会话记忆实体和关系".into(),
            command: "node".into(),
            args: vec![],
            env: Some(serde_json::json!({})),
            enabled: true,
            trust_level: TrustLevel::Trusted,
            scope: "per_agent".into(),
            runtime_kind: RuntimeKind::Bundled,
        },
        NewMcpServer {
            id: "builtin-playwright".into(),
            name: "浏览器自动化".into(),
            description: "浏览器操作——截图、填表单、爬取动态页面、自动化测试".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@playwright/mcp".into()],
            env: Some(serde_json::json!({})),
            enabled: true,
            trust_level: TrustLevel::Trusted,
            scope: "per_agent".into(),
            runtime_kind: RuntimeKind::System,
        },
    ]
}

/// 启动时种子：逐个检查默认 MCP Server，不存在就补上。
/// 已有的配置不覆盖（用户可能改过名称/参数）。
pub async fn seed_defaults(pool: &SqlitePool) -> AppResult<()> {
    let existing = list_all(pool).await?;
    let existing_ids: std::collections::HashSet<&str> =
        existing.iter().map(|e| e.id.as_str()).collect();

    for cfg in &default_mcp_servers() {
        if existing_ids.contains(cfg.id.as_str()) {
            continue;
        }
        tracing::info!(target: "ice_paw.mcp", "补种默认 MCP Server: {}", cfg.name);
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
    runtime_kind: String,
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
            runtime_kind: row.runtime_kind.parse::<RuntimeKind>().unwrap_or_default(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
