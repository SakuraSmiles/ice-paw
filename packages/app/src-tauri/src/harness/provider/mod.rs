//! `harness::provider` — LLM provider adapters（OpenAI / Anthropic / ...）
//!
//! **当前状态**：W1.1 建壳占位模块，文件为空。
//!
//! 后续 Sprint（W2.1–W2.3）将从 `llm/adapters/openai.rs` / `llm/adapters/anthropic.rs`
//! 迁入 OpenAI / Anthropic 适配器，同时合并 `llm/mod.rs` 中的 `create_provider()` +
//! `default_base_url()` 工厂函数。最终 `llm/` 目录整体消失（W2.3）。
//!
//! 详见 Sprint 计划 W2.1–W2.3。