//! MCP Server 管理器 — 生命周期管理
//!
//! Phase 2: 管理所有外部 MCP Server 的启动/关闭/注册。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};

use super::client::McpRegistry;
use super::external::{ExternalMcpServer, ExternalToolProxy};
use super::types::{McpServerConfig, McpToolDefinition};

/// MCP Server 管理器
///
/// 持有所有活跃的 `ExternalMcpServer` 以及它们注册到 `McpRegistry` 的代理。
pub struct McpServerManager {
    /// 活跃服务器：id → ExternalMcpServer
    servers: RwLock<HashMap<String, Arc<ExternalMcpServer>>>,
    /// 工具缓存：id → 该 server 启动时注册的工具清单（供前端查看，避免重发 tools/list）
    tools_cache: RwLock<HashMap<String, Vec<McpToolDefinition>>>,
}

impl McpServerManager {
    /// 创建空管理器
    pub fn new() -> Self {
        Self {
            servers: RwLock::new(HashMap::new()),
            tools_cache: RwLock::new(HashMap::new()),
        }
    }

    /// 启动一个 MCP Server 并注册所有工具到 registry
    pub async fn start(
        &self,
        config: &McpServerConfig,
        registry: &McpRegistry,
    ) -> AppResult<()> {
        // spawn 子进程
        let server = ExternalMcpServer::spawn(
            config.id.clone(),
            config.name.clone(),
            &config.command,
            &config.args,
        ).await?;

        let server = Arc::new(server);

        // 获取工具列表并注册
        let tools = server.list_tools().await?;
        for tool_def in &tools {
            let proxy = Arc::new(ExternalToolProxy::new(
                tool_def.name.clone(),
                tool_def.description.clone(),
                tool_def.input_schema.clone(),
                server.clone(),
                config.trust_level,
            ));
            registry.register(proxy).await;
        }

        tracing::info!(
            target: "ice_paw.mcp",
            "MCP Server '{}' 已启动，注册 {} 个工具",
            config.name,
            tools.len(),
        );

        // 保存到活跃服务器列表
        {
            let mut servers = self.servers.write().await;
            servers.insert(config.id.clone(), server);
        }

        // 缓存工具清单（供前端查看）
        {
            let mut cache = self.tools_cache.write().await;
            cache.insert(config.id.clone(), tools);
        }

        Ok(())
    }

    /// 停止一个 MCP Server（从 registry 移除工具 + 关闭子进程）
    pub async fn stop(&self, id: &str) {
        // 从列表移除
        let server = {
            let mut servers = self.servers.write().await;
            servers.remove(id)
        };

        // 清掉工具缓存
        {
            let mut cache = self.tools_cache.write().await;
            cache.remove(id);
        }

        if let Some(server) = server {
            server.shutdown().await;
            tracing::info!(target: "ice_paw.mcp", "MCP Server '{}' 已关闭", server.name);
        }
    }

    /// 停止所有服务器
    pub async fn stop_all(&self) {
        let servers: Vec<Arc<ExternalMcpServer>> = {
            let mut s = self.servers.write().await;
            s.drain().map(|(_, v)| v).collect()
        };

        {
            let mut cache = self.tools_cache.write().await;
            cache.clear();
        }

        for server in servers {
            server.shutdown().await;
        }

        tracing::info!(target: "ice_paw.mcp", "所有 MCP Server 已关闭");
    }

    /// 获取所有活跃服务器的副本（用于排查）
    pub async fn active_server_count(&self) -> usize {
        let servers = self.servers.read().await;
        servers.len()
    }

    /// 列出所有活跃服务器的工具定义（调试/管理用）
    pub async fn list_active_servers(&self) -> Vec<(String, String)> {
        let servers = self.servers.read().await;
        servers.values().map(|s| (s.id.clone(), s.name.clone())).collect()
    }

    /// 查询某个 server 启动时注册的工具清单（仅运行中的 server 有缓存）
    pub async fn list_server_tools(&self, id: &str) -> AppResult<Vec<McpToolDefinition>> {
        let cache = self.tools_cache.read().await;
        cache
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::Internal(format!("MCP Server '{id}' 未运行")))
    }
}

impl Default for McpServerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_new_is_empty() {
        let mgr = McpServerManager::new();
        assert_eq!(mgr.servers.try_read().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn list_server_tools_missing_returns_err() {
        let mgr = McpServerManager::new();
        assert!(mgr.list_server_tools("nope").await.is_err());
    }
}
