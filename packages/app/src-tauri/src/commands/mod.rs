//! Tauri Commands 总入口
//!
//! - 每个子模块提供一个领域的 commands（agent / conversation / message / chat）
//! - 全部 commands 在 `lib.rs` 里被 `tauri::generate_handler!` 拉起
//! - 所有命令返回 `Result<T, AppError>`，经 `AppError → InvokeError` 自动透传

pub mod agent_cmd;
pub mod chat_cmd;
pub mod conversation_cmd;
pub mod message_cmd;
pub mod preferences_cmd;
