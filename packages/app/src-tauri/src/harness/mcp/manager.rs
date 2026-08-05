//! MCP Server 管理器 — 统一生命周期管理
//!
//! 所有 MCP Server（global + per_agent）通过单一状态机管理：
//! - Boot 时并行启动所有 enabled server
//! - 失败的服务标记 Failed，不重试（用户可在设置页手动重试）
//! - 工具自动注册/反注册到 McpRegistry
//! - per_agent server 的 workspace 在首次需要时后台重启绑定

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};

use super::client::McpRegistry;
use super::external::{ExternalMcpServer, ExternalToolProxy};
use super::types::{
    McpServerConfig, McpToolDefinition, ServerSnapshot, ServerStatusKind, WORKSPACE_PLACEHOLDER,
};

// =========================================================================
// 内部状态类型
// =========================================================================

pub(crate) enum ServerStatus {
    Disabled,
    Starting,
    Running {
        process: Arc<ExternalMcpServer>,
        tools: Vec<McpToolDefinition>,
    },
    Failed {
        reason: String,
    },
}

impl ServerStatus {
    fn to_kind(&self) -> ServerStatusKind {
        match self {
            ServerStatus::Disabled => ServerStatusKind::Disabled,
            ServerStatus::Starting => ServerStatusKind::Starting,
            ServerStatus::Running { .. } => ServerStatusKind::Running,
            ServerStatus::Failed { .. } => ServerStatusKind::Failed,
        }
    }
}

pub(crate) struct ServerEntry {
    pub config: McpServerConfig,
    pub status: ServerStatus,
}

impl ServerEntry {
    fn snapshot(&self) -> ServerSnapshot {
        let mut snap = ServerSnapshot::from(self.config.clone());
        snap.status = self.status.to_kind();
        match &self.status {
            ServerStatus::Running { tools, .. } => {
                snap.tool_count = Some(tools.len());
                snap.tools = Some(tools.clone());
            }
            ServerStatus::Failed { reason } => {
                snap.error = Some(reason.clone());
            }
            _ => {}
        }
        snap
    }
}

// =========================================================================
// McpServerManager
// =========================================================================

pub struct McpServerManager {
    /// 统一 server 状态表：config_id → ServerEntry（pub(crate) 供命令层查询）
    pub(crate) entries: RwLock<HashMap<String, ServerEntry>>,
}

impl McpServerManager {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    // =====================================================================
    // 启动 / 关闭 / 重试
    // =====================================================================

    /// 启动一个 MCP Server（统一 global / per_agent）。
    ///
    /// workspace: 用于替换 args 中的 `{workspace}`。per_agent server 用 agent workspace；
    /// global server 传 None 即可。
    pub async fn start_server(
        &self,
        config: &McpServerConfig,
        workspace: Option<&str>,
        registry: &McpRegistry,
    ) -> AppResult<()> {
        let id = config.id.clone();

        // 标记 Starting
        {
            let mut entries = self.entries.write().await;
            entries.insert(
                id.clone(),
                ServerEntry {
                    config: config.clone(),
                    status: ServerStatus::Starting,
                },
            );
        }

        // 替换 workspace placeholder
        let args: Vec<String> = if let Some(ws) = workspace {
            config
                .args
                .iter()
                .map(|a| a.replace(WORKSPACE_PLACEHOLDER, ws))
                .collect()
        } else {
            config.args.clone()
        };

        // spawn 子进程
        let server = match ExternalMcpServer::spawn(
            config.id.clone(),
            config.name.clone(),
            &config.command,
            &args,
            &config.env,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                let reason = format!("启动失败: {e}");
                let mut entries = self.entries.write().await;
                entries.insert(
                    id.clone(),
                    ServerEntry {
                        config: config.clone(),
                        status: ServerStatus::Failed { reason },
                    },
                );
                return Err(e);
            }
        };
        let server = Arc::new(server);

        // 获取工具列表
        let tools = match server.list_tools().await {
            Ok(t) => t,
            Err(e) => {
                server.shutdown().await;
                let reason = format!("获取工具列表失败: {e}");
                let mut entries = self.entries.write().await;
                entries.insert(
                    id.clone(),
                    ServerEntry {
                        config: config.clone(),
                        status: ServerStatus::Failed { reason },
                    },
                );
                return Err(e);
            }
        };

        // 注册工具到 registry（namespaced: server_name.tool_name）
        let prefix = format!("{}.", config.name);
        let tool_names: Vec<String> = tools.iter().map(|t| format!("{}{}", prefix, t.name)).collect();
        for tool_def in &tools {
            let namespaced = format!("{}{}", prefix, tool_def.name);
            let proxy = Arc::new(ExternalToolProxy::new(
                namespaced,
                tool_def.description.clone(),
                tool_def.input_schema.clone(),
                server.clone(),
                config.trust_level,
            ));
            registry.register(proxy).await;
        }

        // 更新状态为 Running
        {
            let mut entries = self.entries.write().await;
            entries.insert(
                id.clone(),
                ServerEntry {
                    config: config.clone(),
                    status: ServerStatus::Running {
                        process: server,
                        tools: tools.clone(),
                    },
                },
            );
        }

        tracing::info!(
            target: "ice_paw.mcp",
            "MCP Server '{}' 启动成功: {} 个工具 ({} tools registered)",
            config.name,
            tools.len(),
            tool_names.len(),
        );
        Ok(())
    }

    /// 关闭一个 MCP Server（从 registry 反注册工具 + 关闭子进程）
    pub async fn stop_server(&self, id: &str, registry: &McpRegistry) {
        let entry = {
            let mut entries = self.entries.write().await;
            entries.remove(id)
        };

        if let Some(entry) = entry {
            // 反注册工具
            if let ServerStatus::Running { tools, .. } = &entry.status {
                let prefix = format!("{}.", entry.config.name);
                let names: Vec<String> = tools.iter().map(|t| format!("{}{}", prefix, t.name)).collect();
                if !names.is_empty() {
                    registry.unregister(&names).await;
                }
            }
            // 关闭进程
            if let ServerStatus::Running { process, .. } = &entry.status {
                process.shutdown().await;
            }
            tracing::info!(target: "ice_paw.mcp", "MCP Server '{}' 已关闭", entry.config.name);
        }
    }

    /// 重试失败的 server
    pub async fn retry_server(
        &self,
        id: &str,
        workspace: Option<&str>,
        registry: &McpRegistry,
    ) -> AppResult<()> {
        let config = {
            let entries = self.entries.read().await;
            entries
                .get(id)
                .map(|e| e.config.clone())
                .ok_or_else(|| AppError::NotFound {
                    resource: "mcp_server",
                    id: id.to_string(),
                })?
        };
        // 先清理旧状态
        self.stop_server(id, registry).await;
        // 重新启动
        self.start_server(&config, workspace, registry).await
    }

    /// 停止所有 server（应用退出时调用）
    pub async fn stop_all(&self, registry: &McpRegistry) {
        let ids: Vec<String> = {
            let entries = self.entries.read().await;
            entries.keys().cloned().collect()
        };
        for id in &ids {
            self.stop_server(id, registry).await;
        }
        tracing::info!(target: "ice_paw.mcp", "所有 MCP Server 已关闭");
    }

    // =====================================================================
    // 查询接口（供前端 / 命令层使用）
    // =====================================================================

    /// 列出所有 server 的快照（含运行时状态）
    pub async fn list_snapshots(&self) -> Vec<ServerSnapshot> {
        let entries = self.entries.read().await;
        let mut snaps: Vec<_> = entries.values().map(|e| e.snapshot()).collect();
        snaps.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        snaps
    }

    /// 快速启用/禁用（不重启，只更新状态）
    pub async fn set_enabled(&self, id: &str, enabled: bool, registry: &McpRegistry) -> AppResult<()> {
        let mut entries = self.entries.write().await;
        let entry = entries.get_mut(id).ok_or_else(|| AppError::NotFound {
            resource: "mcp_server",
            id: id.to_string(),
        })?;

        entry.config.enabled = enabled;
        if !enabled {
            // 禁用 → 关闭进程 + 反注册工具
            if let ServerStatus::Running { tools, process } = &entry.status {
                let prefix = format!("{}.", entry.config.name);
                let names: Vec<String> = tools.iter().map(|t| format!("{}{}", prefix, t.name)).collect();
                registry.unregister(&names).await;
                process.shutdown().await;
            }
            entry.status = ServerStatus::Disabled;
        }
        // enabled=true 不在这个接口处理——调用方用 retry_server
        Ok(())
    }

    /// per_agent server：确保 workspace 正确。
    /// 如果 server 已在 Running 状态但使用了不同的 workspace，后台重启。
    /// 调用方不应阻塞等待此方法——它用于异步修正 workspace。
    pub async fn rebind_workspace_if_needed(
        &self,
        config_id: &str,
        agent_workspace: &str,
        registry: &McpRegistry,
    ) {
        let needs_rebind = {
            let entries = self.entries.read().await;
            if let Some(entry) = entries.get(config_id) {
                if entry.config.scope == "per_agent" {
                    // 检查当前 args 是否包含正确的 workspace
                    entry.config.args.iter().any(|a| a.contains(WORKSPACE_PLACEHOLDER))
                } else {
                    false
                }
            } else {
                false
            }
        };

        if needs_rebind {
            tracing::info!(
                target: "ice_paw.mcp",
                "per_agent server '{}' 后台重启以绑定 workspace: {}",
                config_id,
                agent_workspace,
            );
            let _ = self.retry_server(config_id, Some(agent_workspace), registry).await;
        }
    }

    /// 检查是否有 Failed 状态的 server（供前端提示用）
    pub async fn failed_server_count(&self) -> usize {
        let entries = self.entries.read().await;
        entries
            .values()
            .filter(|e| matches!(e.status, ServerStatus::Failed { .. }))
            .count()
    }
}

impl Default for McpServerManager {
    fn default() -> Self {
        Self::new()
    }
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_new_is_empty() {
        let mgr = McpServerManager::new();
        assert!(mgr.entries.try_read().unwrap().is_empty());
    }
}
