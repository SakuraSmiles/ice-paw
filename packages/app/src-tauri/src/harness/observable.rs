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

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_timer_elapsed() {
        let timer = RoundTimer::new(1);
        // elapsed_ms should be >= 0 and < 100ms (reasonable for an instant check)
        let elapsed = timer.elapsed_ms();
        assert!(elapsed < 100, "elapsed_ms should be near-zero, got {}", elapsed);
    }

    #[test]
    fn test_round_state_default() {
        let state = RoundState::default();
        assert_eq!(state.round, 0);
        assert_eq!(state.elapsed_ms, 0);
        assert_eq!(state.tokens_prompt, 0);
        assert_eq!(state.tokens_completion, 0);
        assert_eq!(state.cached_tokens, 0);
        assert_eq!(state.retry_count, 0);
    }

    #[test]
    fn test_round_state_accumulation() {
        let mut state = RoundState::default();
        // Simulate round 1
        state.round = 1;
        state.tokens_prompt = 100;
        state.tokens_completion = 50;
        state.cached_tokens = 30;
        state.elapsed_ms = 1234;
        state.retry_count = 0;

        // Simulate round 2 (accumulating)
        state.round = 2;
        state.tokens_prompt += 80;
        state.tokens_completion += 40;
        state.cached_tokens += 10;
        state.elapsed_ms = 2500;
        state.retry_count += 1;

        assert_eq!(state.round, 2);
        assert_eq!(state.tokens_prompt, 180);
        assert_eq!(state.tokens_completion, 90);
        assert_eq!(state.cached_tokens, 40);
        assert_eq!(state.elapsed_ms, 2500);
        assert_eq!(state.retry_count, 1);
    }
}
