//! `stream_loop` 的辅助模块
//!
//! 从 `harness::loop_engine` 按职责拆分：
//! - `stuck_detect`：停滞检测
//! - `token_usage`：多轮 usage 合成

pub(crate) mod stuck_detect;
pub(crate) mod token_usage;
