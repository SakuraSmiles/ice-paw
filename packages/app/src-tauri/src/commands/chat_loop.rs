//! Chat 调度循环 — W3.3: 退化为薄壳，委托给 `harness::loop_engine`
//!
//! 原始实现已于 W3.3 拆分为：
//! - `harness::loop_engine`     — 主循环调度（stream_loop）
//! - `harness::stream_consumer` — 流式消费（consume_stream）
//! - `harness::tool_executor`   — 工具执行（execute_tool_round）
//! - `harness::retry`           — 重试状态机（RetryState）
//! - `harness::budget`          — 循环上限常量（LoopBudget）
//!
//! 本文件仅保留 `pub use` 重导出，保持 `commands::chat_loop::stream_loop` 路径兼容。
//! 等 Week 5 全局清理时，本文件将被删除，所有调用方直连 `harness::loop_engine`。

pub(crate) use crate::harness::loop_engine::stream_loop;
