//! IcePaw MCP 工具系统
//!
//! Phase 1: McpClient trait + McpRegistry + 内置工具客户端。
//! Phase 2: ExternalMcpServer（stdio JSON-RPC）+ McpServerManager。
//!
//! 统一接口 `McpClient` trait，`McpRegistry` 管理所有已注册客户端，
//! 不区分内部/外部工具。

pub mod types;
pub mod client;
pub mod internal;
pub mod external;
pub mod manager;

pub use types::AuthorizationLevel;
pub use client::{McpClient, McpRegistry};
pub use manager::McpServerManager;
