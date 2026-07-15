//! 全局聊天状态管理 + 取消令牌
//!
//! - `CancellationToken`：可跨线程 clone 的取消令牌（共享 `Arc<AtomicBool>`）
//! - `ChatState`：维护 `conversation_id → CancellationToken` 映射，
//!   用于跟踪哪些会话正在流式生成，以及支持用户手动停止。
//!
//! **W2.3**：从 `llm/chat_state.rs`（93 行）+ `llm/cancel.rs`（46 行）合并迁入。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::{AppError, AppResult};

// =========================================================================
// CancellationToken（从 llm/cancel.rs 合并）
// =========================================================================

/// 可跨线程 clone 的取消令牌（共享内部标志位）
///
/// 设计简单：内部用 `Arc<AtomicBool>`，clone 后共享同一标志位。
/// 流式循环每次 yield 前 `is_cancelled()` 检查，发现取消则提前结束。
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

// =========================================================================
// ChatState（从 llm/chat_state.rs 迁入）
// =========================================================================

/// 全局聊天状态（注入到 Tauri managed state）
pub struct ChatState {
    /// 会话 ID → 取消令牌
    inner: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl ChatState {
    /// 创建空实例
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 获取锁，自动从毒化状态恢复
    ///
    /// 当持有锁的线程 panic 时，Mutex 会被标记为 "poisoned"。
    /// 我们选择恢复数据而非 panic 传播，因为 ChatState 是全局状态，
    /// 单次 panic 不应导致整个应用不可用。
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CancellationToken>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 注册一个会话的生成任务
    ///
    /// 返回新建的 CancellationToken，供流式协程持有。
    /// 如果同一会话已有在途生成，返回错误而非静默覆盖。
    pub fn start(&self, conv_id: &str) -> AppResult<CancellationToken> {
        let mut map = self.lock();
        if map.contains_key(conv_id) {
            return Err(AppError::Internal("会话已有在途生成任务".into()));
        }
        let token = CancellationToken::new();
        map.insert(conv_id.to_string(), token.clone());
        Ok(token)
    }

    /// 直接注册一个已有的 CancellationToken
    pub fn register(&self, conv_id: &str, token: CancellationToken) {
        let mut map = self.lock();
        map.insert(conv_id.to_string(), token);
    }

    /// 触发某会话的取消（用户点击「停止」）
    ///
    /// 返回是否命中（true = 确有在途生成并被取消）
    pub fn stop(&self, conv_id: &str) -> bool {
        let map = self.lock();
        if let Some(token) = map.get(conv_id) {
            token.cancel();
            true
        } else {
            false
        }
    }

    /// 注销某会话的令牌（流式协程结束时调用）
    pub fn unregister(&self, conv_id: &str) {
        let mut map = self.lock();
        map.remove(conv_id);
    }

    /// 某会话是否正在流式生成
    pub fn is_streaming(&self, conv_id: &str) -> bool {
        let map = self.lock();
        map.contains_key(conv_id)
    }
}

impl Default for ChatState {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ChatState {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
