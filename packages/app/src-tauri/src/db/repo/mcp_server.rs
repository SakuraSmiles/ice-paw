//! `mcp_servers` 表的 SQL 操作
//!
//! Phase 2: 外部 MCP Server 配置的 CRUD。

use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};
use crate::harness::mcp::types::{
    McpServerConfig, NewMcpServer, RuntimeKind, TransportKind, TrustLevel, UpdateMcpServer,
};

const ALL_COLS: &str = "id, name, description, command, args, env, enabled, trust_level, scope, runtime_kind, transport, url, headers, tool_index, created_at, updated_at";

/// 列出全部 MCP Server 配置，按 created_at 降序
pub async fn list_all(pool: &SqlitePool) -> AppResult<Vec<McpServerConfig>> {
    let rows = sqlx::query_as::<_, McpServerRow>(&format!(
        "SELECT {} FROM mcp_servers ORDER BY created_at DESC",
        ALL_COLS
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

/// 按 id 取一条
pub async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<McpServerConfig> {
    let row = sqlx::query_as::<_, McpServerRow>(&format!(
        "SELECT {} FROM mcp_servers WHERE id = ?",
        ALL_COLS
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "mcp_server",
        id: id.to_string(),
    })?;
    Ok(row.into())
}

/// tool_index 高水位计数器的 preferences 键。
///
/// 2026-09-12 修复：原 `MAX(tool_index)+1` 分配在删除最高位 server 后会把
/// 编号发给新 server——存量 enabled_tools 白名单里的 `t{idx}_xxx` 名字会静默
/// 错绑到**另一个 server 的工具**（潜在提权口）。编号只进不退：计数器单调
/// 递增，删 server 不回收。键缺失时以表内 MAX 播种（存量库首次创建时一次
/// 对齐，之后只看计数器）。
const TOOL_INDEX_HWM_KEY: &str = "mcp_tool_index_hwm";

/// 分配下一个 tool_index（高水位，永不复用）。
///
/// 种子 = max(计数器, 表内 MAX)（防御计数器被手动清掉/落后于表内值），随后
/// 写回计数器 = 分配值。创建是设置页用户驱动的低频操作，无并发竞态面；计数
/// 器写失败不阻塞创建（下次种子逻辑仍以表内 MAX 兜底防复用）。
async fn next_tool_index(pool: &SqlitePool) -> AppResult<i64> {
    let stored: Option<i64> = super::preferences::get(pool, TOOL_INDEX_HWM_KEY)
        .await?
        .and_then(|v| v.parse().ok());
    let max: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(tool_index), -1) FROM mcp_servers")
        .fetch_one(pool)
        .await?;
    // Option::max（内在方法）会遮蔽 Ord::max——先解包再取大
    let next = stored.unwrap_or(max).max(max) + 1;
    if let Err(e) = super::preferences::set(pool, TOOL_INDEX_HWM_KEY, &next.to_string()).await {
        tracing::warn!(target: "ice_paw.db", "tool_index 高水位计数器写回失败（下次以表内 MAX 兜底）: {e}");
    }
    Ok(next)
}

/// 创建 MCP Server 配置
pub async fn create(pool: &SqlitePool, input: &NewMcpServer) -> AppResult<McpServerConfig> {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let args_str = serde_json::to_string(&input.args)?;
    let env_str = input
        .env
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .unwrap_or_else(|| "{}".to_string());
    let headers_str = serde_json::to_string(&input.headers).unwrap_or_else(|_| "{}".to_string());
    let tool_index = next_tool_index(pool).await?;

    sqlx::query(
        "INSERT INTO mcp_servers (id, name, description, command, args, env, enabled, trust_level, scope, runtime_kind, transport, url, headers, tool_index, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(input.transport.as_str())
    .bind(input.url.as_deref())
    .bind(&headers_str)
    .bind(tool_index)
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
    let desc = input
        .description
        .as_deref()
        .unwrap_or(&existing.description);
    let cmd = input.command.as_deref().unwrap_or(&existing.command);
    let args = input.args.as_ref().unwrap_or(&existing.args);
    let env = input.env.as_ref().unwrap_or(&existing.env);
    let enabled = input.enabled.unwrap_or(existing.enabled);
    let trust_level = input.trust_level.unwrap_or(existing.trust_level);
    let scope = input
        .scope
        .clone()
        .unwrap_or_else(|| existing.scope.clone());
    let runtime_kind = input.runtime_kind.unwrap_or(existing.runtime_kind);
    let transport = input.transport.unwrap_or(existing.transport);
    // url 是 Option<String>（input.url None → 保留 existing.url），用 .or 而非 unwrap_or_else
    let url = input.url.clone().or(existing.url.clone());
    let headers = input
        .headers
        .clone()
        .unwrap_or_else(|| existing.headers.clone());
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let args_str = serde_json::to_string(args)?;
    let env_str = serde_json::to_string(env)?;
    let headers_str = serde_json::to_string(&headers)?;

    sqlx::query(
        "UPDATE mcp_servers SET name=?, description=?, command=?, args=?, env=?, enabled=?, trust_level=?, scope=?, runtime_kind=?, transport=?, url=?, headers=?, updated_at=? WHERE id=?",
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
    .bind(transport.as_str())
    .bind(url.as_deref())
    .bind(&headers_str)
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
            transport: TransportKind::Stdio,
            url: None,
            headers: serde_json::json!({}),
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
            transport: TransportKind::Stdio,
            url: None,
            headers: serde_json::json!({}),
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
            transport: TransportKind::Stdio,
            url: None,
            headers: serde_json::json!({}),
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
    transport: String,
    url: Option<String>,
    headers: Option<String>,
    tool_index: i64,
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
            transport: row.transport.parse::<TransportKind>().unwrap_or_default(),
            url: row.url,
            headers: row
                .headers
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::json!({})),
            tool_index: row.tool_index,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
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

    fn new_server(id: &str) -> NewMcpServer {
        NewMcpServer {
            id: id.into(),
            name: format!("server-{id}"),
            description: String::new(),
            command: "node".into(),
            args: vec![],
            env: Some(serde_json::json!({})),
            enabled: true,
            trust_level: TrustLevel::Trusted,
            scope: "per_agent".into(),
            runtime_kind: RuntimeKind::System,
            transport: TransportKind::Stdio,
            url: None,
            headers: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn tool_index_assigns_sequential() {
        let pool = test_pool().await;
        let s1 = create(&pool, &new_server("s1")).await.expect("s1");
        let s2 = create(&pool, &new_server("s2")).await.expect("s2");
        assert_eq!(s1.tool_index, 0);
        assert_eq!(s2.tool_index, 1);
    }

    #[tokio::test]
    async fn tool_index_never_reused_after_delete() {
        // 2026-09-12 回归锁：删最高位 server 后新 server 不得复用被删编号——
        // 存量 enabled_tools 白名单的 t{idx}_ 前缀会静默错绑另一 server 的工具
        let pool = test_pool().await;
        create(&pool, &new_server("s1")).await.expect("s1");
        create(&pool, &new_server("s2")).await.expect("s2");
        create(&pool, &new_server("s3")).await.expect("s3"); // tool_index = 2

        delete(&pool, "s3").await.expect("delete s3"); // MAX 回落到 1

        let s4 = create(&pool, &new_server("s4")).await.expect("s4");
        assert_eq!(
            s4.tool_index, 3,
            "删除后新建必须走高水位（3），不得回落 MAX+1=2 复用被删编号"
        );

        // 计数器持久：再删到只剩一个，编号依然单调递增
        delete(&pool, "s4").await.expect("delete s4");
        delete(&pool, "s2").await.expect("delete s2");
        let s5 = create(&pool, &new_server("s5")).await.expect("s5");
        assert_eq!(s5.tool_index, 4);
    }

    #[tokio::test]
    async fn tool_index_seeds_from_table_max_for_legacy_db() {
        // 存量库：计数器键缺失（升级到本版本前的库），播种逻辑以表内 MAX 起步
        let pool = test_pool().await;
        // 直接 INSERT 绕过 create——模拟高水位机制上线前建的行（无计数器键）
        sqlx::query(
            "INSERT INTO mcp_servers (id, name, description, command, args, env, enabled, trust_level, scope, runtime_kind, transport, url, headers, tool_index, created_at, updated_at)
             VALUES ('legacy-1', 'legacy', '', 'node', '[]', '{}', 1, 'trusted', 'per_agent', 'system', 'stdio', NULL, '{}', 7, '2026-01-01 00:00:00', '2026-01-01 00:00:00')",
        )
        .execute(&pool)
        .await
        .expect("seed legacy row");

        let next = create(&pool, &new_server("s-new")).await.expect("new");
        assert_eq!(next.tool_index, 8, "计数器缺失时以表内 MAX(7)+1 播种");
    }
}
