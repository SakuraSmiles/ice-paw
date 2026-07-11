//! 取消令牌 — 用于用户手动停止 LLM 流式生成
//!
//! 设计简单：内部用 `Arc<AtomicBool>`，clone 后共享同一标志位。
//! 流式循环每次 yield 前 `is_cancelled()` 检查，发现取消则提前结束。

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
