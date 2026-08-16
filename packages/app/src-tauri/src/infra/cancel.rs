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
///
/// MA-1 委派级联取消：`child_token()` 派生的子令牌持有父链——父被取消时
/// 子的 `is_cancelled()` 为真（向下游传播），但子取消不影响父（不向上传播）。
/// 父链在派生时快照（各节点内部是共享 `Arc<AtomicBool>`，快照后依然活引用），
/// 无需通知机制、无轮询、无 watcher 任务泄漏。
#[derive(Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
    /// 父令牌链（空 = 根令牌）。委派深度=1，链长恒 ≤2，递归检查代价可忽略。
    parents: Vec<CancellationToken>,
}

impl CancellationToken {
    /// 创建一个未取消的令牌
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            parents: Vec::new(),
        }
    }

    /// 派生子令牌：父取消 → 子视为已取消（级联停止整棵委派树）。
    ///
    /// 子自身被 cancel 不影响父——只停子会话；父会话「停止生成」时
    /// 父 token.cancel() 自然级联全部子。
    pub fn child_token(&self) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            parents: vec![self.clone()],
        }
    }

    /// 是否已被取消（自身或任一父链祖先被取消即真）
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst) || self.parents.iter().any(|p| p.is_cancelled())
    }

    /// 触发取消（幂等，多次调用无副作用；只取消自身，不向上传染）
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

    // ===== MA-1 委派级联取消（child_token）=====

    #[test]
    fn child_starts_uncancelled() {
        let parent = CancellationToken::new();
        let child = parent.child_token();
        assert!(!child.is_cancelled(), "父未取消时子应为未取消");
    }

    #[test]
    fn parent_cancel_cascades_to_child() {
        let parent = CancellationToken::new();
        let child = parent.child_token();
        parent.cancel();
        assert!(
            child.is_cancelled(),
            "父取消必须级联到子（停止生成一键停整棵委派树）"
        );
    }

    #[test]
    fn child_cancel_does_not_affect_parent() {
        let parent = CancellationToken::new();
        let child = parent.child_token();
        child.cancel();
        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled(), "子取消不得向上传染父");
    }

    #[test]
    fn grandchild_cascades_through_chain() {
        let parent = CancellationToken::new();
        let child = parent.child_token();
        let grandchild = child.child_token();
        parent.cancel();
        assert!(grandchild.is_cancelled(), "取消沿父链传播到孙（链式递归）");
    }

    #[test]
    fn parent_cancel_after_child_completes_is_harmless() {
        // 子已自行 cancel（完成/超时）后父再取消：幂等，无副作用
        let parent = CancellationToken::new();
        let child = parent.child_token();
        child.cancel();
        parent.cancel();
        assert!(child.is_cancelled());
    }
}
