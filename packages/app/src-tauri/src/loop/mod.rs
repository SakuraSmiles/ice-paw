//! `loop` — L2 Loop 层占位（注意：`loop` 是 Rust 关键字，必须用 `r#loop`）
//!
//! **当前状态**：W1.1 建壳占位模块，文件为空。
//!
//! `loop` 是 Rust 关键字，因此在 `lib.rs` 中通过 raw identifier 语法引用：
//!
//! ```ignore
//! pub mod r#loop;
//! ```
//!
//! 本模块当前**无任何代码**，作为 Sprint 计划的占位入口存在。
//! Loop 相关的实际实现（W3.x 的 `consume_stream` / `execute_tool_round` /
//! `RetryState` / `LoopBudget` 等）均位于 `harness::loop_engine` 及其子模块。
//!
//! 详见 Sprint 计划 W1.1 / W3.x / W4.x。