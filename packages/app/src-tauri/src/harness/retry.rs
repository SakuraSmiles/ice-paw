//! Retry 状态机 — 替代字符串拼接回滚（W3.2 B1-0）
//!
//! 背景：`commands/chat_loop.rs` 原重试循环用 `for attempt in 0..MAX_ATTEMPTS` + 字符串拼接
//! `[以下是上一轮因网络中断已收到的部分回复，请从此处继续]\n{text}` 来构造 retry messages。
//! 这种做法有两个问题：
//! 1. **状态不显式**：`attempt` 是个数字，无法表达"耗尽 / 不可重试"等边界条件。
//! 2. **拼接逻辑散落**：retry 决策（能不能继续）+ 消息构造（怎么续写）+ 等待时间
//!    （wait_secs 计算）混在一个循环里，难测试、难维护。
//!
//! 本模块引入 `RetryState` 枚举 + `RetryContext` 上下文：
//! - `RetryState::FirstAttempt` — 初始状态，未发起过 retry
//! - `RetryState::Retrying { attempt, max_attempts, wait_secs }` — retry 中
//! - `RetryState::Exhausted` — 已达到上限，不再 retry
//!
//! 使用方式（参考 `commands/chat_loop.rs`）：
//! ```ignore
//! let mut state = RetryState::new();
//! loop {
//!     if !state.can_retry() { break; }
//!     let ws = state.wait_secs();
//!     if ws > 0 { tokio::time::sleep(Duration::from_secs(ws)).await; }
//!     let msgs = state.prepare_messages(&RetryContext { round_text, messages });
//!     match provider.stream_chat(...).await {
//!         Ok(s) => { /* 成功跳出循环 */ }
//!         Err(e) if e.is_retryable() => state = state.next_retry(calc_wait(state.attempt_num())),
//!         Err(e) => { /* 不可重试，break */ }
//!     }
//! }
//! ```

use crate::infra::protocol::ChatMessage;

/// Retry 状态机 — 显式表达重试生命周期
///
/// 转移路径：
/// ```text
/// FirstAttempt --(next_retry)--> Retrying{attempt:1}
/// Retrying{attempt:n} --(next_retry)--> Retrying{attempt:n+1}
/// Retrying{attempt:max} --(next_retry)--> Exhausted
/// Exhausted --(next_retry)--> Exhausted  (no-op)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryState {
    /// 首次尝试（未发起过 retry）
    FirstAttempt,
    /// 重试中
    ///
    /// - `attempt`: 当前 retry 序号（1-based，1 表示第 1 次重试）
    /// - `max_attempts`: 最大 retry 次数上限（与 `LoopBudget.max_attempts` 对齐）
    /// - `wait_secs`: 下一次重试前的等待秒数（指数退避）
    Retrying {
        attempt: u32,
        max_attempts: u32,
        wait_secs: u64,
    },
    /// 重试已耗尽（attempt == max_attempts 且又触发 retry 时进入）
    Exhausted,
}

impl RetryState {
    /// 初始状态（FirstAttempt）
    pub fn new() -> Self {
        Self::FirstAttempt
    }

    /// 转移到下一次 retry，返回新状态（不修改 self）
    ///
    /// 转移规则：
    /// - `FirstAttempt` → `Retrying { attempt: 1, max_attempts, wait_secs }`
    /// - `Retrying { attempt, .. }` 若 `attempt < max_attempts` → `Retrying { attempt: attempt+1, .. }`
    /// - `Retrying { attempt, .. }` 若 `attempt >= max_attempts` → `Exhausted`
    /// - `Exhausted` → `Exhausted`（no-op，幂等）
    pub fn next_retry(self, max_attempts: u32, wait_secs: u64) -> Self {
        match self {
            Self::FirstAttempt => Self::Retrying {
                attempt: 1,
                max_attempts,
                wait_secs,
            },
            Self::Retrying {
                attempt,
                max_attempts: prev_max,
                ..
            } => {
                // 上限取本次传入的 max_attempts 与上次状态中的较大者，
                // 防止外部缩小上限导致状态丢失历史信息
                let max = prev_max.max(max_attempts);
                // 若下一次 attempt 序号已达到/超过 max，则说明本轮 retry
                // 用完后不应再有下一次 → Exhausted
                if attempt + 1 >= max {
                    Self::Exhausted
                } else {
                    Self::Retrying {
                        attempt: attempt + 1,
                        max_attempts: max,
                        wait_secs,
                    }
                }
            }
            Self::Exhausted => Self::Exhausted,
        }
    }

    /// 是否还能继续 retry
    ///
    /// - `FirstAttempt` → `true`（还没试过）
    /// - `Retrying { attempt, max_attempts }` → `attempt < max_attempts`
    /// - `Exhausted` → `false`
    pub fn can_retry(&self) -> bool {
        match self {
            Self::FirstAttempt => true,
            Self::Retrying {
                attempt,
                max_attempts,
                ..
            } => attempt < max_attempts,
            Self::Exhausted => false,
        }
    }

    /// 当前状态下的等待秒数（仅 `Retrying` 时返回有意义的值）
    pub fn wait_secs(&self) -> u64 {
        match self {
            Self::Retrying { wait_secs, .. } => *wait_secs,
            _ => 0,
        }
    }

    /// 当前 retry 序号（0 = FirstAttempt；Exhausted 也返回 0）
    pub fn attempt_num(&self) -> u32 {
        match self {
            Self::FirstAttempt => 0,
            Self::Retrying { attempt, .. } => *attempt,
            Self::Exhausted => 0,
        }
    }

    /// 构造本轮要发给 LLM 的消息列表
    ///
    /// - `FirstAttempt` / `Exhausted`：直接用 `ctx.messages`（不附加续前消息）
    /// - `Retrying { .. }`：在 `ctx.messages` 末尾追加一条 assistant 消息，
    ///   内容为 `[以下是上一轮因网络中断已收到的部分回复，请从此处继续]\n{round_text}`
    ///   （与原 `commands/chat_loop.rs` 中的字符串拼接行为完全一致）
    ///
    /// `round_text` 为空时不附加（避免空续前消息污染上下文）。
    pub fn prepare_messages(&self, ctx: &RetryContext) -> Vec<ChatMessage> {
        match self {
            Self::FirstAttempt | Self::Exhausted => ctx.messages.clone(),
            Self::Retrying { .. } => {
                if ctx.round_text.is_empty() {
                    return ctx.messages.clone();
                }
                let mut msgs = ctx.messages.clone();
                msgs.push(ChatMessage::from_text(
                    "assistant",
                    format!(
                        "[以下是上一轮因网络中断已收到的部分回复，请从此处继续]\n{}",
                        ctx.round_text
                    ),
                ));
                msgs
            }
        }
    }
}

impl Default for RetryState {
    fn default() -> Self {
        Self::new()
    }
}

/// 重试上下文：retry 时构造消息所需的所有外部输入
#[derive(Debug, Clone)]
pub struct RetryContext {
    /// 上一轮已收集到的部分文本（仅 retry 时用于"续前"消息）
    pub round_text: String,
    /// 当前已有的消息历史（system + user + 之前的 tool round）
    pub messages: Vec<ChatMessage>,
}

impl RetryContext {
    /// 构造空上下文（用于 FirstAttempt）
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            round_text: String::new(),
            messages,
        }
    }

    /// 构造带 round_text 的上下文（用于 retry）
    pub fn with_round_text(messages: Vec<ChatMessage>, round_text: String) -> Self {
        Self {
            round_text,
            messages,
        }
    }
}

// ============================================================================
// 单元测试（W3.2 — 覆盖 3 条转移路径 + 边界条件）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------- 状态转移：FirstAttempt → Retrying{1} → Retrying{2} → Exhausted --------

    #[test]
    fn test_retry_state_first_to_retrying_1() {
        let s = RetryState::new();
        let s2 = s.next_retry(4, 1);
        assert_eq!(
            s2,
            RetryState::Retrying {
                attempt: 1,
                max_attempts: 4,
                wait_secs: 1,
            }
        );
    }

    #[test]
    fn test_retry_state_retrying_to_retrying_n() {
        let s = RetryState::Retrying {
            attempt: 1,
            max_attempts: 4,
            wait_secs: 1,
        };
        let s2 = s.next_retry(4, 2);
        assert_eq!(
            s2,
            RetryState::Retrying {
                attempt: 2,
                max_attempts: 4,
                wait_secs: 2,
            }
        );
    }

    #[test]
    fn test_retry_state_retrying_to_exhausted() {
        // attempt=3（已重试 3 次），next_retry → attempt=4 == max → Exhausted
        let s = RetryState::Retrying {
            attempt: 3,
            max_attempts: 4,
            wait_secs: 4,
        };
        let s2 = s.next_retry(4, 8);
        assert_eq!(s2, RetryState::Exhausted);
    }

    #[test]
    fn test_retry_state_exhausted_is_terminal() {
        // Exhausted 收到 next_retry 仍是 Exhausted（幂等）
        let s = RetryState::Exhausted;
        let s2 = s.next_retry(4, 1);
        assert_eq!(s2, RetryState::Exhausted);
    }

    // -------- can_retry --------

    #[test]
    fn test_retry_can_retry_first_attempt() {
        assert!(RetryState::FirstAttempt.can_retry());
    }

    #[test]
    fn test_retry_can_retry_under_max() {
        let s = RetryState::Retrying {
            attempt: 2,
            max_attempts: 4,
            wait_secs: 2,
        };
        assert!(s.can_retry());
    }

    #[test]
    fn test_retry_can_retry_at_max_false() {
        // attempt == max → 不可继续
        let s = RetryState::Retrying {
            attempt: 4,
            max_attempts: 4,
            wait_secs: 4,
        };
        assert!(!s.can_retry());
    }

    #[test]
    fn test_retry_can_retry_exhausted_false() {
        assert!(!RetryState::Exhausted.can_retry());
    }

    // -------- attempt_num / wait_secs --------

    #[test]
    fn test_retry_attempt_num() {
        assert_eq!(RetryState::FirstAttempt.attempt_num(), 0);
        assert_eq!(
            RetryState::Retrying {
                attempt: 2,
                max_attempts: 4,
                wait_secs: 2
            }
            .attempt_num(),
            2
        );
        assert_eq!(RetryState::Exhausted.attempt_num(), 0);
    }

    #[test]
    fn test_retry_wait_secs_only_retrying() {
        assert_eq!(RetryState::FirstAttempt.wait_secs(), 0);
        assert_eq!(RetryState::Exhausted.wait_secs(), 0);
        assert_eq!(
            RetryState::Retrying {
                attempt: 1,
                max_attempts: 4,
                wait_secs: 3
            }
            .wait_secs(),
            3
        );
    }

    // -------- prepare_messages --------

    fn make_ctx(text: &str) -> RetryContext {
        RetryContext {
            round_text: text.into(),
            messages: vec![ChatMessage::from_text("user", "hello")],
        }
    }

    #[test]
    fn test_prepare_messages_first_attempt_no_continuation() {
        let s = RetryState::FirstAttempt;
        let ctx = make_ctx("partial text");
        let msgs = s.prepare_messages(&ctx);
        // 不附加续前消息
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[test]
    fn test_prepare_messages_retrying_appends_continuation() {
        let s = RetryState::Retrying {
            attempt: 1,
            max_attempts: 4,
            wait_secs: 1,
        };
        let ctx = make_ctx("partial text");
        let msgs = s.prepare_messages(&ctx);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].role, "assistant");
        let appended = msgs[1].content_text();
        assert!(
            appended.contains("[以下是上一轮因网络中断已收到的部分回复，请从此处继续]"),
            "expected continuation marker, got: {}",
            appended
        );
        assert!(appended.contains("partial text"));
    }

    #[test]
    fn test_prepare_messages_retrying_empty_round_text_no_append() {
        let s = RetryState::Retrying {
            attempt: 1,
            max_attempts: 4,
            wait_secs: 1,
        };
        let ctx = make_ctx(""); // 空 round_text
        let msgs = s.prepare_messages(&ctx);
        // round_text 空时不附加（避免空续前消息污染上下文）
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_prepare_messages_exhausted_no_continuation() {
        let s = RetryState::Exhausted;
        let ctx = make_ctx("partial text");
        let msgs = s.prepare_messages(&ctx);
        // Exhausted 不再 retry，消息保持原样
        assert_eq!(msgs.len(), 1);
    }

    // -------- next_retry 状态不修改 self（Copy 语义） --------

    #[test]
    fn test_retry_state_next_retry_does_not_mutate() {
        let s1 = RetryState::FirstAttempt;
        let _ = s1.next_retry(4, 1);
        // s1 仍是 FirstAttempt（next_retry 接受 self by value 并返回新值）
        assert_eq!(s1, RetryState::FirstAttempt);
    }
}
