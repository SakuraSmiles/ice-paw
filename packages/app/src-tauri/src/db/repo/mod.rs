//! 数据访问层入口
//!
//! 每个子模块对应一张表，函数全部是「纯 SQL + &SqlitePool」风格，
//! 不依赖 Tauri 状态，方便单测和复用。

pub mod agent;
pub mod conversation;
pub mod kb;
pub mod mcp_server;
pub mod memory_embedding;
pub mod memory_store;
pub mod message;
pub mod message_attachment;
pub mod preferences;
pub mod project;
pub mod summary;
pub mod template;
pub mod tool_call;
