//! MCP Server 管理器 — 统一生命周期管理
//!
//! 所有 MCP Server（global + per_agent）通过单一状态机管理：
//! - Boot 时并行启动所有 enabled server
//! - 失败的服务标记 Failed，不重试（用户可在设置页手动重试）
//! - 工具自动注册/反注册到 McpRegistry
//! - per_agent server 的 workspace 在首次需要时后台重启绑定

use std::collections::HashMap;
use std::sync::Arc;

use tauri::AppHandle;
use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};

use super::bundled;
use super::client::McpRegistry;
use super::external::{ExternalMcpServer, ExternalToolProxy};
use super::transport::{HttpMcpTransport, McpTransport};
use super::types::{
    McpServerConfig, McpToolDefinition, RuntimeKind, ServerSnapshot, ServerStatusKind,
    TransportKind, WORKSPACE_PLACEHOLDER,
};

// =========================================================================
// 内部状态类型
// =========================================================================

pub(crate) enum ServerStatus {
    Disabled,
    Starting,
    Running {
        process: Arc<dyn McpTransport>,
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

/// 构造 OpenAI 兼容（`^[a-zA-Z0-9_-]+$`）的工具命名空间名：
/// `t{tool_index}_{raw_tool_name}`。
///
/// 整数前缀 `t{idx}_` 必然合规、跨重启稳定（`tool_index` 持久化于 DB），
/// 替代旧的 `{中文显示名}.{工具名}`——后者同时违反正则的中文字符与点号，
/// 在 deepseek / minimax 等 OpenAI 兼容端点触发 HTTP 400。
///
/// `retain` 兜底外部 server 原始工具名理论上可能含的点号 / 非 ASCII；
/// 注意：**调用方仍须把原始 `raw` 透传给 `ExternalToolProxy::server_tool_name`**
/// （server 只认原始名），合规名仅作 registry key / LLM 可见名。
fn namespaced_tool_name(tool_index: i64, raw: &str) -> String {
    let mut s = format!("t{}_{}", tool_index, raw);
    s.retain(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if s.is_empty() {
        s.push_str("tool");
    }
    s
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
    /// AppHandle：bundled 运行时解析 resource_dir 需要（system 运行时不用）。
    /// 由 lib.rs setup 阶段 `new_with_handle` 注入。
    app_handle: Option<AppHandle>,
}

impl McpServerManager {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            app_handle: None,
        }
    }

    /// 注入 AppHandle，使 bundled 运行时 server 能解析 resource_dir。
    /// lib.rs setup 阶段调用（builder 链上的 `new()` 此时还没 handle）。
    pub fn new_with_handle(app: AppHandle) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            app_handle: Some(app),
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

        // 按 transport 分流得到传输实例（stdio 子进程 / http 远程），失败统一标 Failed。
        // 后续 list_tools / 注册 proxy / 标 Running 完全统一，不感知具体传输类型。
        let server: Arc<dyn McpTransport> = match config.transport {
            TransportKind::Stdio => {
                // 替换 args 中的 {workspace} 占位符（per_agent server 用 agent workspace）
                let args: Vec<String> = if let Some(ws) = workspace {
                    config
                        .args
                        .iter()
                        .map(|a| a.replace(WORKSPACE_PLACEHOLDER, ws))
                        .collect()
                } else {
                    config.args.clone()
                };
                // bundled 运行时：command → 内置 node.exe 绝对路径，entry script prepend；
                // system 运行时保持 DB 里的 command/args/env 不变。
                // （command 为 node.exe 绝对路径时，spawn 的 Windows cmd /C 分支被跳过，直执行）
                let (command, args, env_value): (String, Vec<String>, serde_json::Value) =
                    if config.runtime_kind == RuntimeKind::Bundled {
                        self.resolve_bundled(config, args)?
                    } else {
                        (config.command.clone(), args, config.env.clone())
                    };
                match ExternalMcpServer::spawn(
                    config.id.clone(),
                    config.name.clone(),
                    &command,
                    &args,
                    &env_value,
                )
                .await
                {
                    Ok(s) => Arc::new(s),
                    Err(e) => {
                        self.mark_failed(&id, config, format!("启动失败: {e}"))
                            .await;
                        return Err(e);
                    }
                }
            }
            TransportKind::Http => {
                // http/sse 无 args/workspace/子进程概念，直接连远程端点
                let Some(url) = config.url.as_deref() else {
                    let reason = "HTTP 传输缺少 url".to_string();
                    self.mark_failed(&id, config, reason.clone()).await;
                    return Err(AppError::Internal(reason));
                };
                match HttpMcpTransport::new(config.name.clone(), url.to_string(), &config.headers)
                    .await
                {
                    Ok(t) => Arc::new(t),
                    Err(e) => {
                        self.mark_failed(&id, config, format!("连接失败: {e}"))
                            .await;
                        return Err(e);
                    }
                }
            }
            TransportKind::Sse => {
                // SSE 传输见阶段 2；GLM 的 3 个 Remote 服务均支持 streamable HTTP，暂引导用户用 http。
                let reason = "SSE 传输暂未实现，请改用 http 传输".to_string();
                self.mark_failed(&id, config, reason.clone()).await;
                return Err(AppError::Internal(reason));
            }
        };

        // 获取工具列表
        let tools = match server.list_tools().await {
            Ok(t) => t,
            Err(e) => {
                server.shutdown().await;
                self.mark_failed(&id, config, format!("获取工具列表失败: {e}"))
                    .await;
                return Err(e);
            }
        };

        // 注册工具到 registry（namespaced: t{tool_index}_tool_name —— OpenAI 合规）
        let tool_names: Vec<String> = tools
            .iter()
            .map(|t| namespaced_tool_name(config.tool_index, &t.name))
            .collect();
        for tool_def in &tools {
            let namespaced = namespaced_tool_name(config.tool_index, &tool_def.name);
            // 给外部工具描述前置【server 名】，作用与取舍：
            //  1) 多 server 同类工具时帮 LLM 区分来源（如「【GLM 联网搜索】」vs 内置 web_fetch）；
            //  2) 让 score_tools 的中文 query 能与中文 server 名子串匹配（按 CJK bigram 分词，
            //     纯英文描述无法被中文 query 命中）；
            //  3) 取舍：纯中文 query 下，带 CJK 前缀的远程工具会系统性排在英文描述的内置工具
            //     前；因所有工具始终全量可见（仅顺序变化），LLM 仍可选内置工具，故可接受。
            // 仅外部工具（经 ExternalToolProxy）加前缀；内置工具（client.rs register_builtin）不加。
            let server_name = config.name.trim();
            let description = if server_name.is_empty() {
                // server 名为空时不加无意义的「】」空前缀
                tool_def.description.clone()
            } else {
                // 括号后补空格，使紧邻的英文描述词能被 score_tools 整词匹配（+4）；
                // 否则 split_whitespace 会把首词切成「联网搜索】Search」拿不到整词分。
                format!("【{}】 {}", server_name, tool_def.description)
            };
            let proxy = Arc::new(ExternalToolProxy::new(
                namespaced,
                tool_def.name.clone(),
                description,
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

    /// 统一失败收尾：把某 server 标记为 Failed（去重 start_server 各分支的错误收尾块）。
    async fn mark_failed(&self, id: &str, config: &McpServerConfig, reason: String) {
        let mut entries = self.entries.write().await;
        entries.insert(
            id.to_string(),
            ServerEntry {
                config: config.clone(),
                status: ServerStatus::Failed { reason },
            },
        );
    }

    /// bundled 运行时解析：把 DB 里的占位 command="node" + 「用户参数」args 解析成
    /// 可直接 spawn 的 (node.exe 绝对路径, [entry_script, ...args], 合并后的 env)。
    ///
    /// - node.exe 路径来自 resource_dir（安装包自带）。
    /// - entry script = node_modules/<package_dir>/<entry_script>，prepend 到 args 前。
    /// - env = bundled 模板（如 memory 的 MEMORY_FILE_PATH，已替换占位符）→ 用户 env 覆盖。
    fn resolve_bundled(
        &self,
        config: &McpServerConfig,
        mut args: Vec<String>,
    ) -> AppResult<(String, Vec<String>, serde_json::Value)> {
        let app = self.app_handle.as_ref().ok_or_else(|| {
            AppError::Internal(
                "bundled MCP server 无法启动：McpServerManager 未注入 AppHandle".into(),
            )
        })?;
        let spec = bundled::spec_for(&config.id).ok_or_else(|| {
            AppError::Internal(format!("未注册的 bundled server id: {}", config.id))
        })?;
        let node = bundled::node_exe(app)?;
        let entry = bundled::entry_script(app, spec)?;

        // entry script prepend 到 args（{workspace} 已在上一步替换完成）
        let mut full_args = Vec::with_capacity(args.len() + 1);
        full_args.push(entry.to_string_lossy().replace('\\', "/"));
        full_args.append(&mut args);

        // 合并 env：bundled 模板（渲染 {memory_data_file}）→ 用户 env 覆盖（同 key 后者胜）
        let mut env_map = bundled::render_env_template(spec, app)?;
        if let Some(obj) = config.env.as_object() {
            for (k, v) in obj {
                env_map.insert(k.clone(), v.clone());
            }
        }

        Ok((
            node.to_string_lossy().to_string(),
            full_args,
            serde_json::Value::Object(env_map),
        ))
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
                let names: Vec<String> = tools
                    .iter()
                    .map(|t| namespaced_tool_name(entry.config.tool_index, &t.name))
                    .collect();
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
    pub async fn set_enabled(
        &self,
        id: &str,
        enabled: bool,
        registry: &McpRegistry,
    ) -> AppResult<()> {
        let mut entries = self.entries.write().await;
        let entry = entries.get_mut(id).ok_or_else(|| AppError::NotFound {
            resource: "mcp_server",
            id: id.to_string(),
        })?;

        entry.config.enabled = enabled;
        if !enabled {
            // 禁用 → 关闭进程 + 反注册工具
            if let ServerStatus::Running { tools, process } = &entry.status {
                let names: Vec<String> = tools
                    .iter()
                    .map(|t| namespaced_tool_name(entry.config.tool_index, &t.name))
                    .collect();
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
                    entry
                        .config
                        .args
                        .iter()
                        .any(|a| a.contains(WORKSPACE_PLACEHOLDER))
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
            let _ = self
                .retry_server(config_id, Some(agent_workspace), registry)
                .await;
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
