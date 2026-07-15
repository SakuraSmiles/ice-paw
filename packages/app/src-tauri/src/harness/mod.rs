//! `harness` — L2 Harness 层（Provider / ChatState / Loop / Tools / Observable）
//!
//! - W1.1：建壳占位模块
//! - W2.1：provider/openai 迁入
//! - W2.2：provider/anthropic 迁入
//! - W2.3：chat_state（合并 cancel）+ tool_registry + factory 迁入，`llm/` 目录删除
//!
//! 后续 Sprint（W3.x / W4.x / W5.6）将逐步从 `commands/chat_loop.rs` /
//! `commands/chat_cleanup.rs` / `commands/chat_error.rs` 迁入：
//!
//! - `loop_engine`     — 主循环调度（W3.x 拆分 budget/retry/stream_consumer/tool_executor）
//! - `budget`          — LoopBudget 三道熔断（W3.1）
//! - `retry`           — RetryState 状态机（W3.2）
//! - `stream_consumer` — 流式消费 + StreamResult（W3.3）
//! - `tool_executor`   — 工具执行编排（W4.1）
//! - `observable`      — RoundState + RoundTimer（W2.4）
//! - `cleanup`         — 收尾清理（W5.6）
//! - `error_mapping`   — 错误映射（W5.6）
//!
//! 详见 Sprint 计划 W2–W5。

pub mod budget;
pub mod chat_state;
pub mod loop_engine;
pub mod observable;
pub mod provider;
pub mod retry;
pub mod stream_consumer;
pub mod tool_executor;
pub mod tool_registry;