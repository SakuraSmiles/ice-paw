//! LLM Adapter 集合
//!
//! - Phase 1：OpenAI 兼容（OpenAI / GLM / DeepSeek / 自定义 base_url）
//! - Phase 2：Anthropic Messages API 兼容（Anthropic / MiniMax / 其他 anthropic-messages 厂商）
//!
//! **W2.1 起**：OpenAI adapter 已迁至 `crate::harness::provider::openai`。
//! **W2.2 起**：Anthropic adapter 已迁至 `crate::harness::provider::anthropic`。
//! 本目录在 W2.3 后将清空。