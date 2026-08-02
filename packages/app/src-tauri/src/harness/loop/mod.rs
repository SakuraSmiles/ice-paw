//! `stream_loop` 辅助模块
//!
//! 从 `harness::loop_engine` 按职责拆分的纯函数子模块：
//!
//! - `stuck_detect`：停滞检测
//!   - `compute_round_key()` — 计算本轮进度指纹 hash
//!   - `should_terminate_stuck()` — 判断是否连续 N 轮无进展
//! - `token_usage`：多轮 usage 合成
//!   - `synthesize_usage()` — 合并多轮工具调用的 prompt/completion token
//!
//! 这些函数都是纯函数（无副作用），便于独立单元测试。

pub(crate) mod stuck_detect;
pub(crate) mod token_usage;
