//! 跨层协议类型 — 单一数据源（目录模块）
//!
//! chat / llm / commands 跨层共享的协议类型按类分家：
//!
//! - [`llm`]：LLM 数据结构（`ContentBlock` / `ChatMessage` / `ChatDelta` /
//!   `ToolDef` / `TokenUsage`）
//! - [`input`]：前端入参（`SendMessageInput` / `TemplateInput` / `AttachedFile`）
//! - [`events`]：事件 Payload（chat:* 流式事件、工具授权、配置提案）
//!
//! 子模块内容经 `pub use *` 汇出，全库既有 `infra::protocol::X` 导入路径不变。
//!
//! 兼容 re-export（历史上从本模块迁出，保留路径）：
//! - 图片校验 → `infra::image_validation`
//! - `LlmProvider` trait → `harness::provider`

mod events;
mod input;
mod llm;

pub use events::*;
pub use input::*;
pub use llm::*;

// Re-export: 图片校验（从本模块迁至 image_validation，保留兼容路径）
pub use super::image_validation::{
    is_supported_image_media_type, strip_empty_image_blocks, validate_images, MAX_IMAGE_COUNT,
    MAX_IMAGE_SIZE, SUPPORTED_IMAGE_MEDIA_TYPES,
};

// Re-export: LlmProvider trait（从 infra/protocol 迁至 harness/provider，保留兼容路径）
pub use crate::harness::provider::LlmProvider;
