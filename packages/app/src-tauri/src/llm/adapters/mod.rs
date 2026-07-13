//! LLM Adapter 集合
//!
//! - Phase 1：OpenAI 兼容（OpenAI / GLM / DeepSeek / 自定义 base_url）
//! - Phase 2：Anthropic Messages API 兼容（Anthropic / MiniMax / 其他 anthropic-messages 厂商）

pub mod anthropic;
pub mod openai;