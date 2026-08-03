//! MCP Server 管理 Tauri Commands
//!
//! Phase 2: 提供 CRUD + 重启功能管理外部 MCP Server。

use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::State;

use crate::db::repo;
use crate::error::AppResult;
use crate::harness::mcp::types::{McpServerConfig, McpToolDefinition, NewMcpServer, UpdateMcpServer};
use crate::harness::mcp::McpRegistry;
use crate::harness::mcp::McpServerManager;

/// 列出所有已配置的 MCP Server
#[tauri::command]
pub async fn list_mcp_servers(
    pool: State<'_, SqlitePool>,
) -> AppResult<Vec<McpServerConfig>> {
    repo::mcp_server::list_all(pool.inner()).await
}

/// 创建新的 MCP Server 配置
#[tauri::command]
pub async fn create_mcp_server(
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    input: NewMcpServer,
) -> AppResult<McpServerConfig> {
    // 写入数据库
    let saved = repo::mcp_server::create(pool.inner(), &input).await?;

    // 如果启用，启动该 MCP Server
    if saved.enabled && saved.scope == "global" {
        manager.start(&saved, &registry).await?;
    }

    Ok(saved)
}

/// 更新 MCP Server 配置
#[tauri::command]
pub async fn update_mcp_server(
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    input: UpdateMcpServer,
) -> AppResult<McpServerConfig> {
    // 先停止旧服务（如果正在运行）
    manager.stop(&input.id, &registry).await;

    // 更新数据库
    let saved = repo::mcp_server::update(pool.inner(), &input).await?;

    // 如果启用，重新启动
    if saved.enabled && saved.scope == "global" {
        manager.start(&saved, &registry).await?;
    }

    Ok(saved)
}

/// 删除 MCP Server 配置
#[tauri::command]
pub async fn delete_mcp_server(
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    id: String,
) -> AppResult<()> {
    // 停止服务（反注册工具 + 关闭子进程）
    manager.stop(&id, &registry).await;

    // 删除数据库记录
    repo::mcp_server::delete(pool.inner(), &id).await
}

/// 重启 MCP Server
#[tauri::command]
pub async fn restart_mcp_server(
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    id: String,
) -> AppResult<()> {
    // 读取配置
    let config = repo::mcp_server::get_by_id(pool.inner(), &id).await?;

    // 停止
    manager.stop(&id, &registry).await;

    // 启动（仅 global；per_agent 在 send_message 时按 agent 启动）
    if config.scope == "global" {
        manager.start(&config, &registry).await?;
    }

    Ok(())
}

/// 获取当前活跃的 MCP Server 列表
#[tauri::command]
pub async fn list_active_mcp_servers(
    manager: State<'_, Arc<McpServerManager>>,
) -> AppResult<Vec<(String, String)>> {
    Ok(manager.list_active_servers().await)
}

/// 列出某个 MCP Server 提供的工具清单（仅运行中的 server）
#[tauri::command]
pub async fn list_mcp_server_tools(
    manager: State<'_, Arc<McpServerManager>>,
    id: String,
) -> AppResult<Vec<McpToolDefinition>> {
    manager.list_server_tools(&id).await
}

/// 检测 Node.js 是否可用（MCP Server 用 npx 启动需要）
#[tauri::command]
pub fn check_nodejs() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
