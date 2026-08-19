//! MCP Server 管理 Tauri Commands
//!
//! 统一接口：基于 McpServerManager 状态机。

use serde::Serialize;
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
                entries
                    .entry(cfg.id.clone())
                    .or_insert_with(|| ServerEntry {
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
    manager.retry_server(&id, Some(&ws), &registry).await?;

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
            runtime_kind: None,
            transport: None,
            url: None,
            headers: None,
        },
    )
    .await;
    Ok(())
}

/// 检测 Node.js 是否可用（OnceLock 缓存，全进程只检测一次）。
///
/// async 化：原同步命令跑 Tauri 主线程，且首调 `cmd.status()` 同步阻塞等子进程
/// （Windows 含杀毒扫描时可达数百 ms）。热路径 OnceLock 命中纯内存直返；
/// 未命中走 `spawn_blocking`（与 log_cmd::get_logs 同款模式）。
#[tauri::command]
pub async fn check_nodejs() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    if let Some(b) = AVAILABLE.get() {
        return *b;
    }
    let probed = tauri::async_runtime::spawn_blocking(|| {
        let mut cmd = std::process::Command::new("node");
        cmd.arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // Windows: 隐藏 node 检测弹出的控制台窗口
        crate::infra::process::suppress_console_window(&mut cmd);
        cmd.status().map(|s| s.success()).unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    *AVAILABLE.get_or_init(|| probed)
}

/// 内置工具信息（给前端「内置工具」清单展示用）
#[derive(Serialize)]
pub struct BuiltinToolInfo {
    pub name: String,
    pub description: String,
}

/// 列出所有内置工具（read_file / write_file / directory_tree …）。
///
/// **单一事实来源**：直接复用 `McpRegistry::register_builtin()`，前端「内置工具」
/// 清单与计数均取自此处，**不再在前端手抄一份**——避免新增工具时前后端漂移
/// （历史上就因此漏过 directory_tree 等 5 个工具，设置页一直少显示）。
///
/// 注：用 `with_builtin()` 构造一个只含内置工具的临时 registry 再列出，
/// 天然排除了已注册进 global registry 的外部 MCP Server 工具。
#[tauri::command]
pub async fn list_builtin_tools() -> AppResult<Vec<BuiltinToolInfo>> {
    let registry = McpRegistry::with_builtin();
    let mut defs = registry.list_tool_defs().await;
    // HashMap 遍历无序 → 按工具名排序，保证前端展示稳定
    defs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(defs
        .into_iter()
        .map(|d| BuiltinToolInfo {
            name: d.name,
            description: d.description,
        })
        .collect())
}
