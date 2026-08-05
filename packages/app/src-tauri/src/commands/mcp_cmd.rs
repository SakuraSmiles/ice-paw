//! MCP Server 管理 Tauri Commands
//!
//! 统一接口：基于 McpServerManager 状态机。

use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::State;

use crate::db::repo;
use crate::error::AppResult;
use crate::harness::mcp::manager::{ServerEntry, ServerStatus};
use crate::harness::mcp::types::{
    McpServerConfig, McpToolDefinition, NewMcpServer, ServerSnapshot, UpdateMcpServer,
};
use crate::harness::mcp::McpRegistry;
use crate::harness::mcp::McpServerManager;

/// 列出所有 MCP Server 及其运行时状态
#[tauri::command]
pub async fn list_mcp_servers(
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
) -> AppResult<Vec<ServerSnapshot>> {
    let configs = repo::mcp_server::list_all(pool.inner()).await?;
    // 确保 DB 配置已同步到 manager（DB 有但 manager 没有 → 初始化为 Disabled）
    {
        let entries = manager.entries.read().await;
        let missing: Vec<McpServerConfig> = configs
            .iter()
            .filter(|c| !entries.contains_key(&c.id))
            .cloned()
            .collect();
        drop(entries);
        if !missing.is_empty() {
            let mut entries = manager.entries.write().await;
            for cfg in missing {
                entries.entry(cfg.id.clone()).or_insert_with(|| ServerEntry {
                    config: cfg,
                    status: ServerStatus::Disabled,
                });
            }
        }
    }
    Ok(manager.list_snapshots().await)
}

/// 创建 MCP Server 并异步启动
#[tauri::command]
pub async fn create_mcp_server(
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    input: NewMcpServer,
) -> AppResult<McpServerConfig> {
    let saved = repo::mcp_server::create(pool.inner(), &input).await?;

    // 后台启动（不阻塞返回）
    if saved.enabled {
        let mgr = Arc::clone(&manager);
        let reg = Arc::clone(&registry);
        let cfg = saved.clone();
        let ws = if saved.scope == "per_agent" {
            Some(std::env::temp_dir().to_string_lossy().to_string())
        } else {
            None
        };
        tokio::spawn(async move {
            if let Err(e) = mgr.start_server(&cfg, ws.as_deref(), &reg).await {
                tracing::warn!(target: "ice_paw.mcp", "新 MCP Server '{}' 启动失败: {}", cfg.name, e);
            }
        });
    }

    Ok(saved)
}

/// 更新 MCP Server 配置并异步重启
#[tauri::command]
pub async fn update_mcp_server(
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    input: UpdateMcpServer,
) -> AppResult<McpServerConfig> {
    // 先停止旧服务
    manager.stop_server(&input.id, &registry).await;

    // 更新数据库（清除 probe_cache）
    let saved = repo::mcp_server::update(pool.inner(), &input).await?;

    // 后台重启
    if saved.enabled {
        let mgr = Arc::clone(&manager);
        let reg = Arc::clone(&registry);
        let cfg = saved.clone();
        let ws = if saved.scope == "per_agent" {
            Some(std::env::temp_dir().to_string_lossy().to_string())
        } else {
            None
        };
        tokio::spawn(async move {
            if let Err(e) = mgr.start_server(&cfg, ws.as_deref(), &reg).await {
                tracing::warn!(target: "ice_paw.mcp", "MCP Server '{}' 重启失败: {}", cfg.name, e);
            }
        });
    }

    Ok(saved)
}

/// 删除 MCP Server
#[tauri::command]
pub async fn delete_mcp_server(
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    id: String,
) -> AppResult<()> {
    manager.stop_server(&id, &registry).await;
    repo::mcp_server::delete(pool.inner(), &id).await
}

/// 重试失败的 MCP Server
#[tauri::command]
pub async fn retry_mcp_server(
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    id: String,
) -> AppResult<Vec<McpToolDefinition>> {
    let ws = std::env::temp_dir().to_string_lossy().to_string();
    manager
        .retry_server(&id, Some(&ws), &registry)
        .await?;

    // 返回工具列表
    let entries = manager.entries.read().await;
    let tools = entries
        .get(&id)
        .and_then(|e| match &e.status {
            ServerStatus::Running { tools, .. } => Some(tools.clone()),
            _ => None,
        })
        .unwrap_or_default();
    Ok(tools)
}

/// 快速启用/禁用
#[tauri::command]
pub async fn set_mcp_enabled(
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    id: String,
    enabled: bool,
) -> AppResult<()> {
    manager.set_enabled(&id, enabled, &registry).await?;
    // 同步 DB
    let _ = repo::mcp_server::update(
        pool.inner(),
        &UpdateMcpServer {
            id,
            name: None,
            description: None,
            command: None,
            args: None,
            env: None,
            enabled: Some(enabled),
            trust_level: None,
            scope: None,
        },
    )
    .await;
    Ok(())
}

/// 检测 Node.js 是否可用（OnceLock 缓存，全进程只检测一次）
#[tauri::command]
pub fn check_nodejs() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::process::Command::new("node")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}
