//! MCP 协议消息类型 + 工具授权级别
//!
//! Phase 1: 从 `tool_registry/mod.rs` 迁移 `AuthorizationLevel`。
//! Phase 2: MCP JSON-RPC 协议类型 + Server 配置 + 工具定义。
//!
//! 设计要点：
//! - `AuthorizationLevel` 是为工具系统设计的统一授权级别枚举，
//!   被 `McpClient` trait 和 `authority` 模块同时引用

use serde::{Deserialize, Serialize};

// =========================================================================
// AuthorizationLevel
// =========================================================================

/// 工具授权级别
///
/// - `Always`：无需授权，直接执行（如 `list_directory`）
/// - `PathWhitelist`：路径白名单校验（如 `read_file`）
/// - `Confirm`：需用户确认（未来扩展）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum AuthorizationLevel {
    /// 无需授权
    #[default]
    Always,
    /// 路径白名单校验
    PathWhitelist,
    /// 需用户确认（预留）
    Confirm,
}

// =========================================================================
// MCP JSON-RPC 协议类型
// =========================================================================

/// MCP JSON-RPC 请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// MCP JSON-RPC 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// MCP JSON-RPC 错误
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// =========================================================================
// MCP 协议生命周期 & 工具类型
// =========================================================================

/// MCP 初始化请求参数
#[derive(Debug, Clone, Serialize)]
pub struct McpInitializeParams {
    pub protocol_version: String,
    pub capabilities: McpClientCapabilities,
    pub client_info: McpClientInfo,
}

/// MCP 客户端能力
#[derive(Debug, Clone, Serialize)]
pub struct McpClientCapabilities {}

/// MCP 客户端信息
#[derive(Debug, Clone, Serialize)]
pub struct McpClientInfo {
    pub name: String,
    pub version: String,
}

/// MCP 初始化响应
#[derive(Debug, Clone, Deserialize)]
pub struct McpInitializeResult {
    pub protocol_version: String,
    pub capabilities: McpServerCapabilities,
    pub server_info: McpServerInfo,
}

/// MCP 服务器能力
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerCapabilities {}

/// MCP 服务器信息
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    pub version: String,
}

/// tools/list 响应
#[derive(Debug, Clone, Deserialize)]
pub struct McpListToolsResult {
    pub tools: Vec<McpToolDefinition>,
}

/// MCP 工具定义（来自外部 Server 的 tools/list）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// tools/call 请求参数
#[derive(Debug, Clone, Serialize)]
pub struct McpCallToolParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
}

/// tools/call 响应
#[derive(Debug, Clone, Deserialize)]
pub struct McpCallToolResult {
    pub content: Vec<McpContentItem>,
    #[serde(default)]
    pub is_error: bool,
}

/// MCP 内容项
#[derive(Debug, Clone, Deserialize)]
pub struct McpContentItem {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

// =========================================================================
// TrustLevel — Server 级信任控制
// =========================================================================

/// Server 信任级别：控制该 Server 下的工具是否需要授权确认
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// 不信任：每次调用需用户确认弹窗
    #[default]
    Untrusted,
    /// 信任：工具调用免检（直接执行）
    Trusted,
}

impl TrustLevel {
    pub fn as_str(&self) -> &'static str {
        match self { TrustLevel::Trusted => "trusted", TrustLevel::Untrusted => "untrusted" }
    }
}

impl std::str::FromStr for TrustLevel {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s { "trusted" => Ok(TrustLevel::Trusted), _ => Ok(TrustLevel::Untrusted) }
    }
}

// =========================================================================
// RuntimeKind — Server 运行时类型
// =========================================================================

/// MCP Server 运行时类型：控制 command 如何被解析执行。
///
/// - `System`：command 走系统 PATH（npx / node / pipx 等，依赖系统 node）
/// - `Bundled`：用 IcePaw 内置 node.exe + 打包好的 node_modules（零网络依赖、零系统 node 依赖）
///
/// bundled 模式下，DB 里 command 存占位 "node"、args 存「用户可配参数」
/// （不含包名/入口），`start_server` 解析时把 command 换成内置 node.exe 绝对路径、
/// 并把对应包的 entry script prepend 到 args 前面。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// 走系统 PATH（npx/node/pipx 等）
    #[default]
    System,
    /// IcePaw 内置 node.exe + 打包 node_modules
    Bundled,
}

impl RuntimeKind {
    pub fn as_str(&self) -> &'static str {
        match self { RuntimeKind::System => "system", RuntimeKind::Bundled => "bundled" }
    }
}

impl std::str::FromStr for RuntimeKind {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s { "bundled" => Ok(RuntimeKind::Bundled), _ => Ok(RuntimeKind::System) }
    }
}

// =========================================================================
// Server 配置（DB 行 / 传输用）
// =========================================================================

/// MCP Server 配置（与 DB 行对齐，用于前端传输）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: serde_json::Value,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub trust_level: TrustLevel,
    /// 隔离级别：'global' 全局共享 / 'per_agent' 按 agent 启动
    /// （per_agent 的 server，args 中的 {workspace} 启动时替换为 agent workspace）
    #[serde(default = "default_scope")]
    pub scope: String,
    /// 运行时类型：system（走系统 PATH）或 bundled（内置 node + 预打包包）
    #[serde(default)]
    pub runtime_kind: RuntimeKind,
    pub created_at: String,
    pub updated_at: String,
}

fn default_enabled() -> bool { true }

/// scope 默认值：global（兼容旧 server，全局共享）
fn default_scope() -> String { "global".into() }

// =========================================================================
// Server 运行时状态（统一 global/per_agent 的启动/运行/失败状态机）
// =========================================================================

/// Server 运行时状态快照（用于前端展示，不含进程句柄）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSnapshot {
    pub id: String,
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: serde_json::Value,
    pub enabled: bool,
    pub trust_level: TrustLevel,
    pub scope: String,
    #[serde(default)]
    pub runtime_kind: RuntimeKind,
    /// 运行时状态
    pub status: ServerStatusKind,
    /// running 时的工具数
    pub tool_count: Option<usize>,
    /// running 时的工具列表
    pub tools: Option<Vec<McpToolDefinition>>,
    /// failed 时的错误信息
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<McpServerConfig> for ServerSnapshot {
    fn from(cfg: McpServerConfig) -> Self {
        ServerSnapshot {
            id: cfg.id,
            name: cfg.name,
            description: cfg.description,
            command: cfg.command,
            args: cfg.args,
            env: cfg.env,
            enabled: cfg.enabled,
            trust_level: cfg.trust_level,
            scope: cfg.scope,
            runtime_kind: cfg.runtime_kind,
            status: ServerStatusKind::Disabled,
            tool_count: None,
            tools: None,
            error: None,
            created_at: cfg.created_at,
            updated_at: cfg.updated_at,
        }
    }
}

/// 前端可见的状态枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatusKind {
    Disabled,
    Starting,
    Running,
    Failed,
}


/// per-agent MCP server 的 args 占位符：启动时替换为 agent workspace_path。
/// 用于 scope=per_agent 的 server（如 filesystem），实现 per-agent 文件访问隔离。
pub const WORKSPACE_PLACEHOLDER: &str = "{workspace}";

/// 创建 MCP Server 入参
#[derive(Debug, Clone, Deserialize)]
pub struct NewMcpServer {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Option<serde_json::Value>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub trust_level: TrustLevel,
    #[serde(default = "default_scope")]
    pub scope: String,
    /// 运行时类型（用户自建 server 默认 system；builtin bundled 由 seed_defaults 指定）
    #[serde(default)]
    pub runtime_kind: RuntimeKind,
}

/// 更新 MCP Server 入参
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateMcpServer {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    #[serde(default)]
    pub trust_level: Option<TrustLevel>,
    pub scope: Option<String>,
    #[serde(default)]
    pub runtime_kind: Option<RuntimeKind>,
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_level_default_is_always() {
        let level = AuthorizationLevel::default();
        assert_eq!(level, AuthorizationLevel::Always);
    }

    #[test]
    fn auth_level_debug() {
        assert_eq!(format!("{:?}", AuthorizationLevel::Always), "Always");
        assert_eq!(format!("{:?}", AuthorizationLevel::PathWhitelist), "PathWhitelist");
        assert_eq!(format!("{:?}", AuthorizationLevel::Confirm), "Confirm");
    }

    #[test]
    fn json_rpc_request_serde() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: "1".into(),
            method: "tools/call".into(),
            params: Some(serde_json::json!({"name": "read_file", "arguments": {}})),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: JsonRpcRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.method, "tools/call");
    }

    #[test]
    fn mcp_tool_definition_deser() {
        let json = r#"{"name":"read_file","description":"Read a file","inputSchema":{"type":"object"}}"#;
        let def: McpToolDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(def.name, "read_file");
        assert_eq!(def.input_schema["type"], "object");
    }

    #[test]
    fn mcp_call_tool_result_deser() {
        let json = r#"{"content":[{"type":"text","text":"hello"}],"is_error":false}"#;
        let r: McpCallToolResult = serde_json::from_str(json).unwrap();
        assert_eq!(r.content.len(), 1);
        assert_eq!(r.content[0].text.as_deref(), Some("hello"));
        assert!(!r.is_error);
    }

    #[test]
    fn trust_level_default() {
        assert_eq!(TrustLevel::default(), TrustLevel::Untrusted);
    }

    #[test]
    fn trust_level_roundtrip() {
        assert_eq!("trusted".parse::<TrustLevel>().unwrap(), TrustLevel::Trusted);
        assert_eq!("untrusted".parse::<TrustLevel>().unwrap(), TrustLevel::Untrusted);
        assert_eq!("unknown".parse::<TrustLevel>().unwrap(), TrustLevel::Untrusted);
        assert_eq!(TrustLevel::Trusted.as_str(), "trusted");
        assert_eq!(TrustLevel::Untrusted.as_str(), "untrusted");
    }

    #[test]
    fn runtime_kind_default_and_roundtrip() {
        assert_eq!(RuntimeKind::default(), RuntimeKind::System);
        assert_eq!("bundled".parse::<RuntimeKind>().unwrap(), RuntimeKind::Bundled);
        assert_eq!("system".parse::<RuntimeKind>().unwrap(), RuntimeKind::System);
        // 未知值回退到 System（容错，避免坏数据阻断启动）
        assert_eq!("unknown".parse::<RuntimeKind>().unwrap(), RuntimeKind::System);
        assert_eq!(RuntimeKind::Bundled.as_str(), "bundled");
        assert_eq!(RuntimeKind::System.as_str(), "system");
    }
}
