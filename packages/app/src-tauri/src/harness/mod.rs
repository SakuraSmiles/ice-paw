//! `harness` — L2 Harness 层（Provider / ChatState / Loop / Tools / Observable）
//!
//! - W1.1：建壳占位模块
//! - W2.1–W2.3：provider/chat_state/tool_registry 迁入
//! - W3.x：budget/retry/stream_consumer/tool_executor
//! - W4.x：LoopBudget 集成
//! - W5.6：cleanup/error_mapping 从 commands/ 迁入
//! - REQ-XC-004：新增 batch_writer（流式写入批处理器）

pub mod batch_writer;
pub mod budget;
pub mod chat_state;
pub mod cleanup;
pub mod error_mapping;
pub mod loop_engine;
pub mod observable;
pub mod provider;
pub mod retry;
pub mod stream_consumer;
pub mod summary_provider;
pub mod tool_executor;
pub mod tool_registry;
