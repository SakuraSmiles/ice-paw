//! `context` — L2 Context 层（组装 LLM 调用前的完整上下文）
//!
//! W5.1–W5.2 起：逐步从 `commands/chat_context.rs` 迁入子模块。
//!
//! - `template`       — `{{var}}` 模板变量渲染（W5.1）
//! - `os_context`     — OS 环境信息注入（W5.2）
//! - `system_prompt`  — system prompt 构造逻辑（W5.2）
//! - `history`        — 历史消息加载（W5.3）
//! - `pipeline`       — 完整组装管线 `assemble_context()`（W5.3）

pub(crate) mod os_context;
pub(crate) mod system_prompt;
pub(crate) mod template;
