//! `harness` — L2 Harness 层（Provider / ChatState / Loop / Tools / Observable）
//!
//! **当前状态**：W1.1 建壳占位模块，仅声明子模块目录。
//!
//! 后续 Sprint（W2.x / W3.x / W4.x / W5.6）将逐步从 `commands/chat_loop.rs` /
//! `llm/` / `commands/chat_cleanup.rs` / `commands/chat_error.rs` 迁入：
//!
//! - `provider`        — LLM provider adapters（openai、anthropic 等）
//! - `tool_registry`   — 工具注册表 + 内置工具 + 权限策略
//! - `chat_state`      — 全局 ChatState + CancellationToken（W2.3 合并 cancel.rs）
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

// W1.1: 仅声明 W1.1 阶段已建壳的两个子模块。
// 其他子模块将在后续 Sprint 迁移时再补 `pub mod` 声明。
pub mod provider;
pub mod tool_registry;