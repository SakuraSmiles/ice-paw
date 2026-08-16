//! IcePaw MCP 工具系统
//!
//! Phase 1: McpClient trait + McpRegistry + 内置工具客户端。
//! Phase 2: ExternalMcpServer（stdio JSON-RPC）+ McpServerManager。
//!
//! 统一接口 `McpClient` trait，`McpRegistry` 管理所有已注册客户端，
//! 不区分内部/外部工具。

pub mod attachment_image_tool;
pub mod bundled;
pub mod client;
pub mod external;
pub mod internal;
pub mod kb_tool;
pub mod manager;
pub mod read_attachment_tool;
pub mod transport;
pub mod types;
// 内置 agentic 工具（文件读写编辑 / shell / grep / git / web）
pub mod agent_config;
pub mod delegate;
pub mod file_tools;
pub mod git;
pub mod plan_tool;
pub mod proposal_tool;
pub mod search;
pub mod shell;
pub mod web;

pub use client::{McpClient, McpRegistry, ToolContext};
pub use manager::McpServerManager;
pub use types::AuthorizationLevel;
