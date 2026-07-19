//! `infra` — L0/L1 基础设施层（跨层共享的协议类型 + 数据库 + 加密 + 错误）
//!
//! **当前状态**：W1.2 协议归位后，`protocol` 子模块已就位，成为 chat / llm / commands
//! 跨层共享的协议类型"单一数据源"。
//!
//! - `protocol` — 跨层协议类型（W1.2 已迁入：ChatMessage / ContentBlock /
//!   ChatDelta / ToolDef / TokenUsage / LlmProvider trait / 各种 Payload /
//!   `validate_images` / `SUPPORTED_IMAGE_MEDIA_TYPES`）
//! - `cancel` — `CancellationToken`（M1.4 从 harness/chat_state.rs 下沉，
//!   让 context 层 SummaryProvider trait 可直接引用）
//! - `crypto` — stronghold wrapper（W5.6 迁移）
//! - `db` — sqlx 连接池 + migrations + repo（W5.6 迁移）
//! - `error` — AppError / AppResult（W5.6 迁移）
//!
//! 详见 Sprint 计划 W1.2 / W5.6 / M1.4。

pub mod cancel;
pub mod protocol;