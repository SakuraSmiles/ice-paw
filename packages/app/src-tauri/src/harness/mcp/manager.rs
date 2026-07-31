//! MCP Server 管理器 — 生命周期管理
//!
//! Phase 2: 管理所有外部 MCP Server 的启动/关闭/注册。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};

use super::client::McpRegistry;
use super::external::{ExternalMcpServer, ExternalToolProxy};
use super::types::{McpServerConfig, McpToolDefinition, WORKSPACE_PLACEHOLDER};

/// MCP Server 管理器
///
/// 持有所有活跃的 `ExternalMcpServer` 以及它们注册到 `McpRegistry` 的代理。
pub struct McpServerManager {
    /// 活跃服务器（global scope）：id → ExternalMcpServer
    servers: RwLock<HashMap<String, Arc<ExternalMcpServer>>>,
    /// 工具缓存：id → 该 server 启动时注册的工具清单（供前端查看，避免重发 tools/list）
    tools_cache: RwLock<HashMap<String, Vec<McpToolDefinition>>>,
    /// per-agent server 池：key=(agent_id, config_id) → server 实例（复用，不重复启动）
    per_agent_servers: RwLock<HashMap<(String, String), Arc<ExternalMcpServer>>>,
    /// per-agent server 的工具缓存（避免复用时重发 tools/list）
    per_agent_tools: RwLock<HashMap<(String, String), Vec<McpToolDefinition>>>,
}

impl McpServerManager {
    /// 创建空管理器
    pub fn new() -> Self {
        Self {
            servers: RwLock::new(HashMap::new()),
            tools_cache: RwLock::new(HashMap::new()),
            per_agent_servers: RwLock::new(HashMap::new()),
            per_agent_tools: RwLock::new(HashMap::new()),
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

    /// 停止所有服务器（global + per-agent）
    pub async fn stop_all(&self) {
        let mut all: Vec<Arc<ExternalMcpServer>> = {
            let mut s = self.servers.write().await;
            s.drain().map(|(_, v)| v).collect()
        };
        {
            let mut cache = self.tools_cache.write().await;
            cache.clear();
        }
        // per-agent 池也一并清理
        {
            let mut pool = self.per_agent_servers.write().await;
            all.extend(pool.drain().map(|(_, v)| v));
            let mut tc = self.per_agent_tools.write().await;
            tc.clear();
        }

        for server in all {
            server.shutdown().await;
        }

        tracing::info!(target: "ice_paw.mcp", "所有 MCP Server 已关闭（含 per-agent）");
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

    /// 确保 per-agent MCP Server 就绪（首次启动或复用池中实例）。
    ///
    /// args 中的 `{workspace}` 替换为 agent workspace。返回 (server, tools)，
    /// 调用方负责把 tools 注册到本次对话的 registry。
    pub async fn ensure_per_agent(
        &self,
        config: &McpServerConfig,
        agent_id: &str,
        workspace: &str,
    ) -> AppResult<(Arc<ExternalMcpServer>, Vec<McpToolDefinition>)> {
        let key = (agent_id.to_string(), config.id.clone());

        // 复用池中实例
        {
            let pool = self.per_agent_servers.read().await;
            if let Some(server) = pool.get(&key) {
                let tools_cache = self.per_agent_tools.read().await;
                let tools = tools_cache.get(&key).cloned().unwrap_or_default();
                tracing::info!(
                    target: "ice_paw.mcp",
                    "复用 per-agent MCP Server: agent={} config={}",
                    agent_id, config.name
                );
                return Ok((server.clone(), tools));
            }
        }

        // 首次启动：args 替换 {workspace}
        let args: Vec<String> = config
            .args
            .iter()
            .map(|a| a.replace(WORKSPACE_PLACEHOLDER, workspace))
            .collect();
        let server = Arc::new(
            ExternalMcpServer::spawn(
                config.id.clone(),
                config.name.clone(),
                &config.command,
                &args,
            )
            .await?,
        );
        let tools = server.list_tools().await?;

        self.per_agent_servers.write().await.insert(key.clone(), server.clone());
        self.per_agent_tools.write().await.insert(key, tools.clone());

        tracing::info!(
            target: "ice_paw.mcp",
            "启动 per-agent MCP Server: agent={} config={} workspace={} → {} 工具",
            agent_id, config.name, workspace, tools.len()
        );
        Ok((server, tools))
    }

    /// 停止某 agent 的所有 per-agent MCP Server
    pub async fn stop_per_agent(&self, agent_id: &str) {
        let to_stop: Vec<Arc<ExternalMcpServer>> = {
            let mut pool = self.per_agent_servers.write().await;
            let keys: Vec<(String, String)> = pool
                .keys()
                .filter(|(a, _)| a == agent_id)
                .cloned()
                .collect();
            let mut stopped = Vec::new();
            for k in keys {
                if let Some(s) = pool.remove(&k) {
                    stopped.push(s);
                }
            }
            let mut tc = self.per_agent_tools.write().await;
            tc.retain(|(a, _), _| a != agent_id);
            stopped
        };
        for server in to_stop {
            server.shutdown().await;
        }
        tracing::info!(target: "ice_paw.mcp", "agent {} 的 per-agent server 已关闭", agent_id);
    }

    /// 探测 per-agent server 的工具清单（临时启动 → list_tools → 关闭 → 缓存到 tools_cache）。
    ///
    /// per_agent server 不全局常驻，但工具清单对所有 agent 一样（args 只影响允许目录，
    /// 不影响工具列表）。启动时探测一次缓存，让 McpSettings 能展示工具能力。
    /// {workspace} 替换为 temp 目录（工具清单不依赖允许目录）。
    pub async fn probe_tools(&self, config: &McpServerConfig) -> AppResult<Vec<McpToolDefinition>> {
        // 已缓存则直接返回
        {
            let cache = self.tools_cache.read().await;
            if let Some(tools) = cache.get(&config.id) {
                return Ok(tools.clone());
            }
        }
        let temp = std::env::temp_dir().to_string_lossy().to_string();
        let args: Vec<String> = config
            .args
            .iter()
            .map(|a| a.replace(WORKSPACE_PLACEHOLDER, &temp))
            .collect();
        let server = ExternalMcpServer::spawn(
            config.id.clone(),
            config.name.clone(),
            &config.command,
            &args,
        )
        .await?;
        let tools = server.list_tools().await?;
        server.shutdown().await;

        self.tools_cache.write().await.insert(config.id.clone(), tools.clone());
        tracing::info!(
            target: "ice_paw.mcp",
            "探测 per-agent MCP Server '{}' 工具清单: {} 个",
            config.name, tools.len()
        );
        Ok(tools)
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
