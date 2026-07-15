//! `infra` — L0/L1 基础设施层（跨层共享的协议类型 + 数据库 + 加密 + 错误）
//!
//! **当前状态**：W1.1 建壳占位模块，文件为空。
//!
//! 后续 Sprint（W1.2 / W5.6）将从 `commands/chat_protocol.rs` / `llm/mod.rs`（类型部分）
//! 迁入共享的协议类型 `protocol` 子模块。最终 `crypto.rs` / `db/` / `error.rs` 也将并入
//! `infra/`（W5.6 收尾整合）。
//!
//! 后续计划子模块：
//!
//! - `protocol` — 跨层协议类型（ChatMessage / ContentBlock / ChatDelta /
//!   ToolDef / TokenUsage / LlmProvider trait / 各种 Payload）
//! - `crypto` — stronghold wrapper（W5.6 迁移）
//! - `db` — sqlx 连接池 + migrations + repo（W5.6 迁移）
//! - `error` — AppError / AppResult（W5.6 迁移）
//!
//! 详见 Sprint 计划 W1.2 / W5.6。