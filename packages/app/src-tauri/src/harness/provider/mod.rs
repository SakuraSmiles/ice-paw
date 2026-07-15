//! `harness::provider` — LLM provider adapters（OpenAI / Anthropic / ...）
//!
//! - W2.1：从 `llm/adapters/openai.rs` 迁入 OpenAI 兼容 Adapter
//! - W2.2：从 `llm/adapters/anthropic.rs` 迁入 Anthropic 兼容 Adapter
//! - W2.3：合并 `llm/mod.rs` 中的 `create_provider()` + `default_base_url()` 工厂函数
//!
//! 详见 Sprint 计划 W2.1–W2.3。

pub mod openai;
pub use openai::OpenAiAdapter;