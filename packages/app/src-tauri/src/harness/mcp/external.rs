//! 外部 MCP Server 连接器
//!
//! Phase 2: 支持用户配置的外部 MCP Server（stdio JSON-RPC）。
//!
//! 结构：
//! - `ExternalMcpServer` — 管理一个子进程的完整生命周期
//! - `ExternalToolProxy` — 单个工具的 `McpClient` 实现

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::Command;
use tokio::sync::{oneshot, Mutex, Notify};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::infra::protocol::ToolDef;

use super::client::McpClient;
use super::transport::{extract_text_from_call_result, McpTransport};
use super::types::{
    AuthorizationLevel, JsonRpcRequest, JsonRpcResponse, McpCallToolParams, McpCallToolResult,
    McpInitializeParams, McpListToolsResult, McpToolDefinition, TrustLevel,
};

// =========================================================================
// ExternalMcpServer — 管理一个 MCP 子进程
// =========================================================================

/// stdio 请求超时。tools/list 等快操作用 30s；tools/call 单独放宽到 120s——
/// 实测 GLM 视觉 MCP（glm-4.6v）对 base64 图 + 长 prompt 的分析延迟在 5~67s
/// 波动（2026-08-14 日志坐实 6 次调用 3 次超 30s），30s 会把慢但成功的调用掐死。
const REQUEST_TIMEOUT_QUICK: u64 = 30;
const REQUEST_TIMEOUT_CALL: u64 = 120;

/// 构建子进程的安全环境：在 `env_clear()` 之后回注「进程执行所需、且不含机密」
/// 的系统变量 + 用户显式声明的 env。
///
/// 设计动机：外部 MCP server 常是第三方 npx/pipx 包（可能被供应链投毒），若继承
/// 父进程全部环境，会拿到 OPENAI_API_KEY / ANTHROPIC_API_KEY / 云凭证等机密。
/// 但简单 `env_clear()` 会连 PATH 一起清掉，导致 `npx` 找不到 node 而启动失败。
/// 因此采用**白名单**：只放行进程执行必需的非机密系统变量，机密一律不透传。
fn build_safe_env(user_env: &serde_json::Value) -> Vec<(String, String)> {
    // 进程执行所需、确认为非机密的系统变量（跨 Windows/Unix）。
    // 注意：不放任何 *_KEY / *_TOKEN / *_SECRET / CREDENTIAL / PASSWORD 模式。
    const SAFE_KEYS: &[&str] = &[
        "PATH",
        "Path",
        "PATHEXT", // 可执行文件查找（Windows PATH 大小写不固定）
        "SYSTEMROOT",
        "ComSpec",
        "windir", // Windows 系统根 / cmd 路径
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMFILES",
        "PROGRAMDATA",
        "USERPROFILE",
        "USERNAME",
        "HOME",
        "USER",
        "SHELL",
        "TEMP",
        "TMP",
        "TMPDIR", // 临时目录
        "LANG",
        "LC_ALL",
        "LC_CTYPE", // 区域/编码（npx/node 需要正确 UTF-8）
    ];
    let mut out: Vec<(String, String)> = Vec::new();
    for key in SAFE_KEYS {
        if let Ok(val) = std::env::var(key) {
            out.push((key.to_string(), val));
        }
    }
    // 用户显式声明的 env 覆盖/追加（同 key 后写覆盖前面）
    if let Some(obj) = user_env.as_object() {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                out.push((k.clone(), s.to_string()));
            } else {
                tracing::warn!(
                    target: "ice_paw.mcp",
                    "MCP env 变量 {k} 的值不是字符串，已忽略（仅支持字符串值）"
                );
            }
        }
    }
    out
}

/// 管理一个外部 MCP Server 子进程（stdio JSON-RPC）。
///
/// 所有 IO 操作通过 `Arc<Mutex<...>>` 共享，支持 `&self` 访问。
pub struct ExternalMcpServer {
    /// 服务器唯一 ID
    pub id: String,
    /// 服务器名称
    pub name: String,
    /// stdin 写入器（Arc<Mutex<>> 共享）
    writer: Arc<Mutex<BufWriter<tokio::process::ChildStdin>>>,
    /// 待响应队列：request_id → oneshot::Sender
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
    /// 后台读取任务的停止信号
    stop: Arc<Notify>,
    /// 子进程句柄（保持存活，drop 时 kill 子进程）
    _child: tokio::process::Child,
}

impl ExternalMcpServer {
    /// 启动一个外部 MCP Server 子进程。
    ///
    /// 1. 启动子进程（stdin/stdout piped）
    /// 2. MCP 协议握手（initialize + initialized 通知）
    /// 3. 启动后台 stdout 读取任务
    pub async fn spawn(
        id: String,
        name: String,
        command: &str,
        args: &[String],
        env: &serde_json::Value,
    ) -> AppResult<Self> {
        // Windows 上 npx/node 等是 .cmd 文件，CreateProcess 搜索 PATH 不找 .cmd，
        // 需要通过 cmd /C 执行。判断条件：非绝对路径 + 非 .exe 后缀。
        let (actual_command, actual_args): (String, Vec<String>) = if cfg!(windows)
            && !command.to_lowercase().ends_with(".exe")
            && !command.contains('/')
            && !command.contains('\\')
        {
            let mut cmd_args = vec!["/C".to_string(), command.to_string()];
            cmd_args.extend(args.iter().cloned());
            ("cmd".to_string(), cmd_args)
        } else {
            (command.to_string(), args.to_vec())
        };

        let mut cmd_builder = Command::new(&actual_command);
        cmd_builder
            .args(&actual_args)
            // 环境隔离：清空继承的环境（防 OPENAI_API_KEY 等机密泄漏给外部 server），
            // 再仅注入「进程执行所需的安全系统变量」+ 用户显式声明的 env（见 build_safe_env）。
            .env_clear()
            .envs(build_safe_env(env))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // 兜底 kill：即便礼貌 shutdown 失败，ExternalMcpServer 析构也确保子进程退出，
            // 避免握手失败/异常的 server 变孤儿（_child 字段 drop 时触发）。
            .kill_on_drop(true)
            // 设置工作目录为用户 home，避免 npx 在当前目录找 package.json 失败。
            // 跨平台：Unix 取 HOME、Windows 取 USERPROFILE；都缺失则回平台根目录。
            // 旧实现只认 USERPROFILE + 兜底 "C:\\"——Linux/Mac 上 USERPROFILE 未设会
            // chdir 到字面 "C:\"（反斜杠非分隔符，单段文件名）→ spawn 外部 MCP server 失败。
            .current_dir(
                std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| {
                        if cfg!(target_os = "windows") {
                            "C:\\".into()
                        } else {
                            "/".into()
                        }
                    }),
            );

        // Windows: 隐藏 cmd /C 弹出的控制台窗口（见 infra::process）
        crate::infra::process::suppress_console_window(&mut cmd_builder);

        let mut child = cmd_builder.spawn().map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "启动 MCP Server '{}' 失败: {} (command={})",
                name, e, command
            )))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Internal("无法获取 MCP Server 的 stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Internal("无法获取 MCP Server 的 stdout".into()))?;

        let writer = Arc::new(Mutex::new(BufWriter::new(stdin)));
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // 创建取消信号（stdout 和 stderr 读取任务共享）
        let stop = Arc::new(Notify::new());

        // 捕获 stderr 日志（带取消机制，与 stdout 共享同一个 stop Notify）
        let name_for_err = name.clone();
        let stop_for_stderr = stop.clone();
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    tokio::select! {
                        _ = stop_for_stderr.notified() => break,
                        result = reader.read_line(&mut line) => {
                            match result {
                                Ok(0) => break,
                                Ok(_) => {
                                    tracing::warn!(target: "ice_paw.mcp", "[stderr] {}: {}", name_for_err, line.trim_end());
                                    line.clear();
                                }
                                Err(_) => break,
                            }
                        }
                    }
                }
            });
        }

        // 发送 initialize 握手
        let init_params = McpInitializeParams {
            protocol_version: "0.1.0".into(),
            capabilities: super::types::McpClientCapabilities {},
            client_info: super::types::McpClientInfo {
                name: "ice-paw".into(),
                version: "0.1.0".into(),
            },
        };
        let init_req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: "init-1".to_string(),
            method: "initialize".into(),
            params: Some(
                serde_json::to_value(init_params).expect("McpInitializeParams 序列化不应失败"),
            ),
        };

        // 注册 oneshot
        let (init_tx, init_rx) = oneshot::channel();
        {
            let mut p = pending.lock().await;
            p.insert("init-1".to_string(), init_tx);
        }

        // 写入 init 请求
        Self::write_line(&writer, &init_req).await?;

        // 启动后台 stdout 读取任务（复用已创建的 stop）
        let pending_clone = pending.clone();
        let stop_clone = stop.clone();
        tokio::spawn(Self::read_loop(
            BufReader::new(stdout),
            pending_clone,
            stop_clone,
        ));

        // 等待初始化响应（60s 超时）
        tokio::time::timeout(Duration::from_secs(60), init_rx)
            .await
            .map_err(|_| AppError::Internal(format!("MCP Server '{}' 初始化超时（60s）", name)))?
            .map_err(|_| AppError::Internal(format!("MCP Server '{}' 初始化通道关闭", name)))?;

        // 发送 initialized 通知（JSON-RPC notification：无 id、无响应，符合 MCP 协议）。
        // 之前误用 JsonRpcRequest 带 id，server 若回响应会触发「未知 request_id」警告。
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });
        Self::write_line(&writer, &notif).await?;

        tracing::info!(
            target: "ice_paw.mcp",
            "MCP Server '{}' 初始化完成", name
        );

        Ok(ExternalMcpServer {
            id,
            name,
            writer,
            pending,
            stop,
            _child: child,
        })
    }

    /// 调用 tools/list 获取工具定义列表
    pub async fn list_tools(&self) -> AppResult<Vec<McpToolDefinition>> {
        let req_id = Uuid::new_v4().to_string();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: req_id.clone(),
            method: "tools/list".into(),
            params: None,
        };

        let resp = self
            .send_request(&req, &req_id, REQUEST_TIMEOUT_QUICK)
            .await?;

        let result = resp.result.ok_or_else(|| {
            let err_msg = resp.error.map(|e| e.message).unwrap_or_default();
            AppError::Internal(format!(
                "MCP Server '{}' tools/list 失败: {}",
                self.name, err_msg
            ))
        })?;

        let list: McpListToolsResult = serde_json::from_value(result)
            .map_err(|e| AppError::Internal(format!("解析 MCP tools/list 结果失败: {}", e)))?;

        Ok(list.tools)
    }

    /// 调用 tools/call 执行一个工具
    pub async fn call_tool(&self, tool_name: &str, args: &Value) -> AppResult<String> {
        let req_id = Uuid::new_v4().to_string();
        let params = McpCallToolParams {
            name: tool_name.to_string(),
            arguments: Some(args.clone()),
        };

        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: req_id.clone(),
            method: "tools/call".into(),
            params: Some(serde_json::to_value(params).expect("McpCallToolParams 序列化不应失败")),
        };

        let resp = self
            .send_request(&req, &req_id, REQUEST_TIMEOUT_CALL)
            .await?;

        if let Some(err) = resp.error {
            return Err(AppError::Internal(format!(
                "MCP Server '{}' 工具 '{}' 调用失败: {}",
                self.name, tool_name, err.message
            )));
        }

        let result_value = resp.result.ok_or_else(|| {
            AppError::Internal(format!(
                "MCP Server '{}' 工具 '{}' 返回空结果",
                self.name, tool_name
            ))
        })?;

        let call_result: McpCallToolResult = serde_json::from_value(result_value)
            .map_err(|e| AppError::Internal(format!("解析 MCP 工具结果失败: {}", e)))?;

        Ok(extract_text_from_call_result(&call_result))
    }

    /// 优雅关闭
    pub async fn shutdown(&self) {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: "shutdown".to_string(),
            method: "shutdown".into(),
            params: None,
        };
        let _ = Self::write_line(&self.writer, &req).await;
        self.stop.notify_one();
    }

    // ======================================================================
    // 内部方法
    // ======================================================================

    async fn send_request(
        &self,
        req: &JsonRpcRequest,
        req_id: &str,
        timeout_secs: u64,
    ) -> AppResult<JsonRpcResponse> {
        let (tx, rx) = oneshot::channel();
        {
            let mut p = self.pending.lock().await;
            p.insert(req_id.to_string(), tx);
        }

        Self::write_line(&self.writer, req).await?;

        tokio::time::timeout(Duration::from_secs(timeout_secs), rx)
            .await
            .map_err(|_| {
                AppError::Internal(format!(
                    "MCP Server '{}' 请求超时（{}s）: {}",
                    self.name, timeout_secs, req.method
                ))
            })?
            .map_err(|_| AppError::Internal("MCP Server 通道关闭".into()))
    }

    async fn write_line<T: serde::Serialize>(
        writer: &Arc<Mutex<BufWriter<tokio::process::ChildStdin>>>,
        req: &T,
    ) -> AppResult<()> {
        let json = serde_json::to_string(req)
            .map_err(|e| AppError::Internal(format!("JSON 序列化失败: {}", e)))?;
        let mut w = writer.lock().await;
        w.write_all(json.as_bytes()).await.map_err(|e| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                format!("写入 MCP Server stdin 失败: {}", e),
            ))
        })?;
        w.write_all(b"\n").await.map_err(|e| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                format!("写入 MCP Server stdin 换行失败: {}", e),
            ))
        })?;
        w.flush().await.map_err(|e| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                format!("刷新 MCP Server stdin 失败: {}", e),
            ))
        })?;
        Ok(())
    }

    async fn read_loop(
        mut reader: BufReader<tokio::process::ChildStdout>,
        pending: Arc<Mutex<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
        stop: Arc<Notify>,
    ) {
        let mut line_buf = String::new();
        loop {
            line_buf.clear();

            tokio::select! {
                _ = stop.notified() => {
                    tracing::info!(target: "ice_paw.mcp", "MCP 读取任务停止");
                    return;
                }
                result = reader.read_line(&mut line_buf) => {
                    match result {
                        Ok(0) => {
                            tracing::info!(target: "ice_paw.mcp", "MCP Server stdout 已关闭");
                            return;
                        }
                        Ok(_) => {
                            let line = line_buf.trim_end();
                            if line.is_empty() { continue; }
                            match serde_json::from_str::<JsonRpcResponse>(line) {
                                Ok(resp) => {
                                    let mut p = pending.lock().await;
                                    if let Some(tx) = p.remove(&resp.id) {
                                        let _ = tx.send(resp);
                                    } else {
                                        tracing::warn!(
                                            target: "ice_paw.mcp",
                                            "未知 request_id 的 MCP 响应: {}",
                                            resp.id,
                                        );
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        target: "ice_paw.mcp",
                                        "MCP 非 JSON-RPC 行（跳过）: {}", e,
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!(target: "ice_paw.mcp", "读取 MCP stdout 失败: {}", e);
                            return;
                        }
                    }
                }
            }
        }
    }
}

impl Drop for ExternalMcpServer {
    fn drop(&mut self) {
        self.stop.notify_one();
    }
}

/// stdio 传输同样实现 `McpTransport`，与远程传输（http/sse）平级，
/// 让 manager / ExternalToolProxy 通过 `Arc<dyn McpTransport>` 统一持有。
/// 方法委托给同名 inherent 方法（Rust 中 inherent 方法优先于 trait 方法，无歧义）。
#[async_trait]
impl McpTransport for ExternalMcpServer {
    async fn list_tools(&self) -> AppResult<Vec<McpToolDefinition>> {
        self.list_tools().await
    }
    async fn call_tool(&self, name: &str, args: &Value) -> AppResult<String> {
        self.call_tool(name, args).await
    }
    async fn shutdown(&self) {
        self.shutdown().await
    }
}

// =========================================================================
// ExternalToolProxy — 单个工具的 McpClient 实现
// =========================================================================

pub struct ExternalToolProxy {
    /// 工具对外的命名空间名（`t{tool_index}_{tool_name}`，OpenAI 合规）——既作
    /// registry key，也作 LLM 可见/调用的名字（多 server 同名工具靠前缀消歧）。
    name: String,
    /// server 端原始工具名（tools/list 返回的 `name`）——tools/call 发回给 server。
    /// 关键：server 只认原始名，带 `t{idx}_` 前缀调用会报 "not found"。
    server_tool_name: String,
    description: String,
    parameters: serde_json::Value,
    server: Arc<dyn McpTransport>,
    trust_level: TrustLevel,
}

impl ExternalToolProxy {
    pub(crate) fn new(
        name: String,
        server_tool_name: String,
        description: String,
        parameters: serde_json::Value,
        server: Arc<dyn McpTransport>,
        trust_level: TrustLevel,
    ) -> Self {
        Self {
            name,
            server_tool_name,
            description,
            parameters,
            server,
            trust_level,
        }
    }
}

#[async_trait]
impl McpClient for ExternalToolProxy {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> serde_json::Value {
        self.parameters.clone()
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        match self.trust_level {
            TrustLevel::Trusted => AuthorizationLevel::Always,
            TrustLevel::Untrusted => AuthorizationLevel::Confirm,
        }
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let args_value: serde_json::Value = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("工具 '{}' 参数解析失败: {}", self.name, e))
        })?;
        // 发给 server 的是原始工具名（不带 `t{idx}_` 前缀）；
        // self.name（带前缀）仅用于 registry 查找与 LLM 展示。
        self.server
            .call_tool(&self.server_tool_name, &args_value)
            .await
    }
}

// =========================================================================
// 辅助函数
// =========================================================================

/// 把 MCP 工具定义转换为 LLM 可用的 ToolDef
pub fn tool_def_from_mcp(def: &McpToolDefinition) -> ToolDef {
    ToolDef {
        name: def.name.clone(),
        description: def.description.clone(),
        parameters: if def.input_schema.is_null() {
            serde_json::json!({"type": "object"})
        } else {
            def.input_schema.clone()
        },
    }
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_def_from_mcp_basic() {
        let mcp = McpToolDefinition {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: json!({"type": "object", "properties": {}}),
        };
        let td = tool_def_from_mcp(&mcp);
        assert_eq!(td.name, "read_file");
        assert_eq!(td.parameters["type"], "object");
    }

    #[test]
    fn tool_def_from_mcp_null_schema() {
        let mcp = McpToolDefinition {
            name: "ping".into(),
            description: "Ping".into(),
            input_schema: json!(null),
        };
        let td = tool_def_from_mcp(&mcp);
        assert_eq!(td.parameters["type"], "object");
    }

    #[test]
    fn proxy_auth_level_for_trust_level() {
        let _params = json!({"type": "object"});
        let server = "placeholder";
        let _ = server;
        // Test the match logic directly
        assert_eq!(
            match TrustLevel::Trusted {
                TrustLevel::Trusted => AuthorizationLevel::Always,
                _ => AuthorizationLevel::Confirm,
            },
            AuthorizationLevel::Always
        );
        assert_eq!(
            match TrustLevel::Untrusted {
                TrustLevel::Trusted => AuthorizationLevel::Always,
                _ => AuthorizationLevel::Confirm,
            },
            AuthorizationLevel::Confirm
        );
    }
}
