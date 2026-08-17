//! `stream_loop` 辅助模块
//!
//! 从 `harness::loop_engine` 按职责拆分的子模块：
//!
//! - `context`：循环输入封装 — `LoopConfig`（不可变配置）+ `LoopContext`（配置 + 可变消息）
//! - `stuck_detect`：停滞检测
//!   - `compute_round_key()` — 计算本轮进度指纹 hash
//!   - `should_terminate_stuck()` — 判断是否连续 N 轮无进展
//! - `token_usage`：多轮 usage 合成
//!   - `synthesize_usage()` — 合并多轮工具调用的 prompt/completion token
//! - `reason`：retry reason 分类
//!   - `classify_retry_reason()` — 将 AppError 映射为 retry reason 字符串
//! - `retry_round`：单轮流式 + 退避重试
//!   - `stream_with_retry()` — 带重试地拉取一轮 LLM 流，结果归类为 `RoundStreamResult`
//! - `events`：loop 事件发射
//!   - `emit_intermediate_round_state()` — 发射中间 round-state 事件
//!
//! 这些函数无业务副作用（`events` 仅 emit Tauri 事件），便于独立单元测试。

pub(crate) mod context;
pub(crate) mod emitter;
pub(crate) mod events;
pub(crate) mod reason;
pub(crate) mod retry_round;
pub(crate) mod stuck_detect;
pub(crate) mod token_usage;
