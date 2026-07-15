//! `infra` — L0/L1 基础设施层（跨层共享的协议类型 + 数据库 + 加密 + 错误）
//!
//! **当前状态**：W1.2 协议归位后，`protocol` 子模块已就位，成为 chat / llm / commands
//! 跨层共享的协议类型"单一数据源"。
//!
//! - `protocol` — 跨层协议类型（W1.2 已迁入：ChatMessage / ContentBlock /
//!   ChatDelta / ToolDef / TokenUsage / LlmProvider trait / 各种 Payload /
//!   `validate_images` / `SUPPORTED_IMAGE_MEDIA_TYPES`）
//! - `crypto` — stronghold wrapper（W5.6 迁移）
//! - `db` — sqlx 连接池 + migrations + repo（W5.6 迁移）
//! - `error` — AppError / AppResult（W5.6 迁移）
//!
//! 详见 Sprint 计划 W1.2 / W5.6。

pub mod protocol;