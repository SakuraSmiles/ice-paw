//! L0 Infra — 可跨线程 clone 的取消令牌
//!
//! 设计简单：内部用 `Arc<AtomicBool>`，clone 后共享同一标志位。
//! 流式循环每次 yield 前 `is_cancelled()` 检查，发现取消则提前结束。
//!
//! **W2.3 起源**：从 `llm/cancel.rs`（46 行）合并迁入 `harness/chat_state.rs`。
//! **M1.4 再次迁入**：从 `harness/chat_state.rs` 下沉到 `infra/cancel.rs`，
//! 让 `context::memory::SummaryProvider` trait 可在 context 层引用此类型，
//! 避免 context 层反向依赖 harness 层。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 可跨线程 clone 的取消令牌（共享内部标志位）
#[derive(Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// 创建一个未取消的令牌
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 是否已被取消
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// 触发取消（幂等，多次调用无副作用）
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_cancel_is_false() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn cancel_sets_flag() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn cancel_is_idempotent() {
        let token = CancellationToken::new();
        token.cancel();
        token.cancel();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn clone_shares_flag() {
        let token = CancellationToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());
        token.cancel();
        assert!(clone.is_cancelled());
    }

    #[test]
    fn debug_does_not_panic() {
        let token = CancellationToken::new();
        let dbg = format!("{:?}", token);
        assert!(dbg.contains("CancellationToken"));
        assert!(dbg.contains("cancelled"));
    }
}