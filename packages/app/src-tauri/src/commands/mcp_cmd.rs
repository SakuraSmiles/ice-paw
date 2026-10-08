//! MCP Server 管理 Tauri Commands
//!
//! 统一接口：基于 McpServerManager 状态机。

use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::db::repo;
use crate::error::AppResult;
use crate::harness::mcp::manager::{ServerEntry, ServerStatus};
use crate::harness::mcp::server_secrets;
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
                        last_workspace: None,
                    });
            }
        }
    }
    Ok(manager.list_snapshots().await)
}

/// 创建 MCP Server 并异步启动
#[tauri::command]
pub async fn create_mcp_server(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    input: NewMcpServer,
) -> AppResult<McpServerConfig> {
    // 2026-10-08 P1 槽位化脱水：实值入 Stronghold 槽位 `mcpserver:{id}`，
    // DB 只落哨兵清单（键名 → __SLOT__）——库文件/list 回传零凭据
    let mut input = input;
    let env = input.env.clone().unwrap_or_else(|| serde_json::json!({}));
    if server_secrets::has_plaintext(&env) || server_secrets::has_plaintext(&input.headers) {
        server_secrets::store_secrets(&app, &input.id, &env, &input.headers)?;
        input.env = Some(server_secrets::to_manifest(&env));
        input.headers = server_secrets::to_manifest(&input.headers);
    } else {
        input.env = Some(env);
    }
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
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    input: UpdateMcpServer,
) -> AppResult<McpServerConfig> {
    // 2026-10-08 P1 槽位化读-改-写：先解析两字段的最终生效实值（None 字段
    // 从槽位取既有值，避免部分更新把另一半清掉），任一含实值 → 槽位 upsert
    // + DB 落哨兵清单
    let existing = repo::mcp_server::get_by_id(pool.inner(), &input.id).await?;
    let final_env = match &input.env {
        Some(v) => v.clone(),
        None => server_secrets::resolve_value(&app, &input.id, "env", &existing.env)?,
    };
    let final_headers = match &input.headers {
        Some(v) => v.clone(),
        None => server_secrets::resolve_value(&app, &input.id, "headers", &existing.headers)?,
    };
    let mut input = input;
    if server_secrets::has_plaintext(&final_env) || server_secrets::has_plaintext(&final_headers) {
        server_secrets::store_secrets(&app, &input.id, &final_env, &final_headers)?;
        input.env = Some(server_secrets::to_manifest(&final_env));
        input.headers = Some(server_secrets::to_manifest(&final_headers));
    }

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
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    manager: State<'_, Arc<McpServerManager>>,
    registry: State<'_, Arc<McpRegistry>>,
    id: String,
) -> AppResult<()> {
    manager.stop_server(&id, &registry).await;
    repo::mcp_server::delete(pool.inner(), &id).await?;
    // 2026-10-08 P1：槽位清理（失败 warn 不卡删除主流程——孤儿密文无害）
    if let Err(e) = crate::crypto::delete_slot(&app, &server_secrets::slot_of(&id)) {
        tracing::warn!(target: "ice_paw.mcp", "删除 MCP Server 凭据槽位失败（孤儿密文无害）: {e}");
    }
    Ok(())
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
    // 同步 DB（失败上抛：内存态已翻转而持久化失败若被吞，命令返回成功但重启后
    // 状态回跳——用户以为改了实际没存）
    repo::mcp_server::update(
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
    .await?;
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
    /// 所属工具组键（tool_scopes::TOOL_GROUPS 反查；None = 未分组——前端落
    /// 「其他」组展示，且不是合法 scope 组）。组语义固定名单快照，见 tool_scopes.rs。
    pub group: Option<String>,
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
            group: crate::harness::mcp::tool_scopes::tool_group_of(&d.name).map(String::from),
            name: d.name,
            description: d.description,
        })
        .collect())
}
