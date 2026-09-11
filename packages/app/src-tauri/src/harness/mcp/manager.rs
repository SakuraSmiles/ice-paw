//! MCP Server 管理器 — 统一生命周期管理
//!
//! 所有 MCP Server（global + per_agent）通过单一状态机管理：
//! - Boot 时并行启动所有 enabled server
//! - 失败的服务标记 Failed，不主动重试（用户可在设置页手动重试）；
//!   例外 = **调用期懒重启**（[`McpServerManager::lazy_restart`]，2026-09-11 ②）：
//!   工具调用撞上「传输已断开」类错误时按 server 节流重启一次并重试该调用——
//!   UE 编辑器内置 MCP 等本地 server 重启后 streamable HTTP 旧 session 失效
//!   （404）、stdio 子进程被外部杀掉（BrokenPipe）是真实高频场景
//! - 工具自动注册/反注册到 McpRegistry
//! - per_agent server 的 workspace 在首次需要时后台重启绑定

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

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
    /// 最近一次启动所用的 workspace（per_agent server 的 `{workspace}` 替换值）。
    /// 懒重启 / retry 不带 workspace 时回退到此值，防 per_agent server 重启后
    /// 退回未替换的 `{workspace}` 占位符。
    pub last_workspace: Option<String>,
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
    /// 懒重启节流表：config_id → 最近一次懒重启时刻（std Mutex 短临界区，
    /// 仿 AuthSessionRegistry）。窗口内不重复重启——server 没起来时每个失败
    /// 调用都触发重启只会堆叠启动请求。
    restart_throttle: StdMutex<HashMap<String, Instant>>,
}

impl McpServerManager {
    /// 懒重启节流窗口：同 server 30s 内至多自动重启一次。
    const LAZY_RESTART_THROTTLE: Duration = Duration::from_secs(30);

    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            app_handle: None,
            restart_throttle: StdMutex::new(HashMap::new()),
        }
    }

    /// 注入 AppHandle，使 bundled 运行时 server 能解析 resource_dir。
    /// lib.rs setup 阶段调用（builder 链上的 `new()` 此时还没 handle）。
    pub fn new_with_handle(app: AppHandle) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            app_handle: Some(app),
            restart_throttle: StdMutex::new(HashMap::new()),
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
        let ws_owned = workspace.map(|s| s.to_string());

        // 标记 Starting
        {
            let mut entries = self.entries.write().await;
            entries.insert(
                id.clone(),
                ServerEntry {
                    config: config.clone(),
                    status: ServerStatus::Starting,
                    last_workspace: ws_owned.clone(),
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
                config.id.clone(),
                config.name.clone(),
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
                    last_workspace: ws_owned,
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
        // 保留本次启动尝试所用的 workspace（start_server 的 Starting 阶段已写入）——
        // Failed 后的懒重启/手动重试不带 workspace 时按它重跑，per_agent server
        // 不退回未替换的 `{workspace}` 占位符
        let last_workspace = entries.get(id).and_then(|e| e.last_workspace.clone());
        entries.insert(
            id.to_string(),
            ServerEntry {
                config: config.clone(),
                status: ServerStatus::Failed { reason },
                last_workspace,
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

    /// 重试失败的 server（设置页手动重试入口）。
    ///
    /// workspace 缺省时回退到该 server 最近一次启动所用的 workspace——
    /// per_agent server 不因重试退回未替换的 `{workspace}` 占位符。
    pub async fn retry_server(
        &self,
        id: &str,
        workspace: Option<&str>,
        registry: &McpRegistry,
    ) -> AppResult<()> {
        let (config, last_ws) = {
            let entries = self.entries.read().await;
            entries
                .get(id)
                .map(|e| (e.config.clone(), e.last_workspace.clone()))
                .ok_or_else(|| AppError::NotFound {
                    resource: "mcp_server",
                    id: id.to_string(),
                })?
        };
        // 先清理旧状态
        self.stop_server(id, registry).await;
        // 重新启动
        self.start_server(&config, workspace.or(last_ws.as_deref()), registry)
            .await
    }

    /// 调用期懒重启（2026-09-11 ②）：工具调用撞上「传输已断开」类错误时，
    /// 按节流窗口自动重启该 server 一次，调用方随后重试本次调用。
    ///
    /// 两个真实高频场景：UE 编辑器等本地 server 重启 → streamable HTTP 旧
    /// session 失效（404）；stdio 子进程被外部结束 → BrokenPipe。旧行为是
    /// 永久 Failed 等用户到设置页手动重试——对话中途被打断去设置页点按钮，
    /// 体验断裂。
    ///
    /// 返回：
    /// - `Ok(true)`  已执行重启（调用方应重试本次调用）
    /// - `Ok(false)` 节流窗口内已有一次重启（不重复重启，调用方直接给错误）
    /// - `Err`       server 不存在 / 已被用户禁用 / 重启失败
    ///
    /// 不变式：**用户显式禁用的 server 永不被懒重启复活**——禁用是治理动作，
    /// 懒重启只救「意外断线」。
    pub async fn lazy_restart(&self, config_id: &str, registry: &McpRegistry) -> AppResult<bool> {
        let (config, last_ws) = {
            let entries = self.entries.read().await;
            entries
                .get(config_id)
                .map(|e| (e.config.clone(), e.last_workspace.clone()))
                .ok_or_else(|| AppError::NotFound {
                    resource: "mcp_server",
                    id: config_id.to_string(),
                })?
        };
        if !config.enabled || matches!(self.entry_kind(config_id).await, Some(ServerStatusKind::Disabled)) {
            return Err(AppError::Validation(format!(
                "MCP Server '{}' 已禁用，不会自动重连；如需使用请在 设置 → MCP/工具集 中启用它",
                config.name
            )));
        }

        // 节流（check+insert 同临界区，并发调用只放一个进去）
        {
            let mut last = self
                .restart_throttle
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(t) = last.get(config_id) {
                if t.elapsed() < Self::LAZY_RESTART_THROTTLE {
                    return Ok(false);
                }
            }
            last.insert(config_id.to_string(), Instant::now());
        }

        tracing::info!(
            target: "ice_paw.mcp",
            "MCP Server '{}' 传输断开，调用期懒重启",
            config.name
        );
        self.stop_server(config_id, registry).await;
        self.start_server(&config, last_ws.as_deref(), registry)
            .await?;
        Ok(true)
    }

    /// 某 server 当前状态 kind（无锁竞争的窄读；不存在返回 None）。
    async fn entry_kind(&self, config_id: &str) -> Option<ServerStatusKind> {
        let entries = self.entries.read().await;
        entries.get(config_id).map(|e| e.status.to_kind())
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
// 传输断开错误分类（懒重启触发判据）
// =========================================================================

/// 判断工具错误文本是否属于「外部 server 传输已断开」家族（懒重启触发判据）。
///
/// 匹配 external.rs / transport.rs 的实际错误形态（`AppError::to_string()` 后
/// 小写匹配）：
/// - stdio：`写入 MCP Server stdin 失败`（Io BrokenPipe 家族）/ `MCP Server 通道关闭`
/// - HTTP：`MCP HTTP '...' 请求失败 (POST)`（连不上）/ `MCP HTTP '...' ... 返回 404`
///   （streamable HTTP server 重启后 `Mcp-Session-Id` 失效的典型返回）
/// - Io 错误 kind 不进 Display 文本，按 io 错误消息关键词兜底（broken pipe /
///   connection reset / connection aborted / not connected）
///
/// **超时不算**（`请求超时`）——慢 ≠ 死，重启重试只会再等一遍 120s。
pub(crate) fn is_transport_down_error(msg: &str) -> bool {
    let s = msg.to_lowercase();
    // stdio：进程退出/管道断开（写入失败家族前缀见 external.rs write_line）
    s.contains("写入 mcp server stdin 失败")
        || s.contains("broken pipe")
        || s.contains("connection reset")
        || s.contains("connection aborted")
        || s.contains("not connected")
        // stdio：send_request 的 rx 通道关闭（子进程死后读端结束）
        || s.contains("mcp server 通道关闭")
        // HTTP：连接失败 / server 重启后旧 session 404
        || (s.contains("mcp http") && s.contains("请求失败"))
        || (s.contains("mcp http") && s.contains("返回 404"))
}

/// 懒重启路径的最终错误文案（三段式：发生了什么 + 为什么 + 怎么办；
/// 家族前缀 `MCP 连接失败:` 在最前——doom_loop 错误签名按「工具名 + 首行
/// 冒号前缀」计算，家族词必须稳定不被 server 名/路径污染）。
pub(crate) fn transport_down_message(server_name: &str, detail: &str) -> String {
    format!(
        "MCP 连接失败: 与外部 Server「{server_name}」的传输已断开，本次调用未送达。\
         原因: Server 进程退出或网络中断，自动重连未能恢复。\
         建议: 请稍后重试本工具；若持续失败，请在 设置 → MCP/工具集 中手动重试该 Server。\
         （{detail}）"
    )
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

    // ===== ② 懒重启：传输断开错误分类 =====

    #[test]
    fn transport_down_error_matches_stdio_family() {
        assert!(is_transport_down_error(
            "IO 错误: 写入 MCP Server stdin 失败 (broken pipe)"
        ));
        // external.rs 实际形态：send_request 的 rx 关闭（无 server 名）
        assert!(is_transport_down_error("内部错误: MCP Server 通道关闭"));
        assert!(is_transport_down_error(
            "IO 错误: connection reset by peer"
        ));
    }

    #[test]
    fn transport_down_error_matches_http_family() {
        // transport.rs 两形态：请求失败（连不上）/ 非 2xx（404 = 旧 session 失效）
        assert!(is_transport_down_error(
            "内部错误: MCP HTTP 'UE' 请求失败 (POST): connection refused"
        ));
        assert!(is_transport_down_error(
            "内部错误: MCP HTTP 'UE' tools/call 返回 404 Not Found"
        ));
        // 其它非 2xx 状态码不算断线（如 500 是 server 活着的瞬态错误）
        assert!(!is_transport_down_error(
            "内部错误: MCP HTTP 'UE' tools/call 返回 500 Internal Server Error"
        ));
    }

    #[test]
    fn transport_down_error_excludes_timeout_and_unrelated() {
        // 慢 ≠ 死：超时不触发懒重启
        assert!(!is_transport_down_error(
            "内部错误: MCP Server 'slow' 请求超时（120s）: tools/call"
        ));
        // 参数校验等业务错误更不算
        assert!(!is_transport_down_error("参数校验失败: bad json"));
        assert!(!is_transport_down_error(""));
    }

    #[test]
    fn transport_down_message_family_prefix_stable() {
        // doom_loop 签名 = 工具名 + 首行冒号前缀——家族词必须在前且不含 server 名
        let msg = transport_down_message("UE 编辑器", "whatever");
        assert!(msg.starts_with("MCP 连接失败: "));
        let prefix = msg.split(':').next().unwrap();
        assert_eq!(prefix, "MCP 连接失败");
    }

    // ===== ② 懒重启：禁用不复活 + 节流窗口 =====

    /// 测试用最小 stdio 配置（command 必然启动失败——验证失败路径不 panic）
    fn bogus_stdio_config(id: &str, enabled: bool) -> McpServerConfig {
        McpServerConfig {
            id: id.to_string(),
            name: format!("server-{id}"),
            description: String::new(),
            transport: TransportKind::Stdio,
            command: "definitely-not-a-real-command-xyz".into(),
            args: vec![],
            env: serde_json::Value::Object(Default::default()),
            url: None,
            headers: serde_json::Value::Object(Default::default()),
            trust_level: super::super::types::TrustLevel::Untrusted,
            scope: "global".into(),
            enabled,
            runtime_kind: RuntimeKind::System,
            tool_index: 1,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    /// 测试用「秒拒」HTTP 配置——连本机必关端口（port 9），连接拒绝即失败。
    /// ⚠️ 勿用 bogus_stdio_config 验证会真正走启动的路径：不存在的 command 在
    /// Windows 经 cmd /C 起得来但握手永不至，要吃满 60s 初始化超时——节流窗
    /// 口（30s）会被它自己耗穿，测试既慢又假红。
    fn refused_http_config(id: &str) -> McpServerConfig {
        McpServerConfig {
            url: Some("http://127.0.0.1:9/mcp".into()),
            transport: TransportKind::Http,
            command: String::new(),
            ..bogus_stdio_config(id, true)
        }
    }

    #[tokio::test]
    async fn lazy_restart_never_resurrects_disabled_server() {
        let mgr = McpServerManager::new();
        let registry = McpRegistry::new();
        mgr.entries.write().await.insert(
            "srv-disabled".into(),
            ServerEntry {
                config: bogus_stdio_config("srv-disabled", false),
                status: ServerStatus::Disabled,
                last_workspace: None,
            },
        );
        let err = mgr.lazy_restart("srv-disabled", &registry).await.unwrap_err();
        assert!(err.to_string().contains("已禁用"), "实际: {err}");
    }

    #[tokio::test]
    async fn lazy_restart_throttles_within_window() {
        let mgr = McpServerManager::new();
        let registry = McpRegistry::new();
        mgr.entries.write().await.insert(
            "srv-dead".into(),
            ServerEntry {
                // HTTP 拒连（本机 port 9）——连接拒绝秒败，第一次重启在毫秒级
                // 返回 Err；若用 stdio bogus 命令，握手挂满 60s 超时会把 30s
                // 节流窗自己耗穿（见 refused_http_config 注释）
                config: refused_http_config("srv-dead"),
                status: ServerStatus::Failed {
                    reason: "连接失败".into(),
                },
                last_workspace: Some("/ws/agent-a".into()),
            },
        );
        // 第一次：尝试重启（连接拒绝 → Err），节流表已登记
        assert!(mgr.lazy_restart("srv-dead", &registry).await.is_err());
        // 第二次（30s 窗口内）：不再尝试，Ok(false)
        assert!(!mgr.lazy_restart("srv-dead", &registry).await.unwrap());
    }

    #[tokio::test]
    async fn lazy_restart_unknown_server_not_found() {
        let mgr = McpServerManager::new();
        let registry = McpRegistry::new();
        let err = mgr.lazy_restart("nope", &registry).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound { .. }));
    }
}
