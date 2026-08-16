//! L2 Harness 可观测性 — 只产出数据，不 emit 事件
//!
//! 核心数据结构：
//! - `RoundTimer`：单轮计时器，记录 `elapsed_ms()`
//! - `RoundState`：轮次状态快照，由 `chat_loop::stream_loop` 更新，
//!   由 `chat_cmd::spawn_stream_loop` 读取并 emit `chat:round-state`

use std::time::Instant;

/// 轮次计时器（内置，不暴露 emit 逻辑）
pub struct RoundTimer {
    start: Instant,
    pub round_index: u32,
}

impl RoundTimer {
    pub fn new(round_index: u32) -> Self {
        Self {
            start: Instant::now(),
            round_index,
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

/// 轮次状态快照（由 chat_loop.rs 更新，commands/chat_cmd.rs 读取并 emit）
#[derive(Debug, Clone, Default)]
pub struct RoundState {
    pub round: u32,
    pub elapsed_ms: u64,
    pub tokens_prompt: u32,
    pub tokens_completion: u32,
    pub cached_tokens: u32,
    pub retry_count: u32,
}
