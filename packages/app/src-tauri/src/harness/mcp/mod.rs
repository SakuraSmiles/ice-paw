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
pub mod kb_tool;
pub mod manager;
// 内置 agentic 工具（文件读写编辑 / shell / grep / git / web）
pub mod file_tools;
pub mod shell;
pub mod search;
pub mod git;
pub mod web;
pub mod agent_config;

pub use types::AuthorizationLevel;
pub use client::{McpClient, McpRegistry, ToolContext};
pub use manager::McpServerManager;
