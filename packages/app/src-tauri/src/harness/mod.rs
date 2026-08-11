//! `harness` — L2 Harness 层（Provider / ChatState / Loop / Tools / Observable）
//!
//! - W1.1：建壳占位模块
//! - W2.1–W2.3：provider/chat_state/tool_registry 迁入
//! - W3.x：budget/retry/stream_consumer/tool_executor
//! - W4.x：LoopBudget 集成
//! - W5.6：cleanup/error_mapping 从 commands/ 迁入
//! - REQ-XC-004：新增 batch_writer（流式写入批处理器）
//! - Phase 1: tool_registry → mcp + authority + scoring

pub mod authority;
pub mod batch_writer;
pub mod budget;
pub mod chat_state;
pub mod cleanup;
pub mod doc;
pub mod error_mapping;
pub mod hooks;
pub mod kb;
pub mod r#loop;
pub mod loop_engine;
pub mod mcp;
pub mod observable;
pub mod oneshot_registry;
pub mod proposal_guard;
pub mod proposal_registry;
pub mod provider;
pub mod retry;
pub mod scoring;
pub mod stream_consumer;
pub mod summary_provider;
pub mod tool_executor;
