//! 全局聊天状态管理
//!
//! - `ChatState`：维护 `conversation_id → CancellationToken` 映射，
//!   用于跟踪哪些会话正在流式生成，以及支持用户手动停止。
//!
//! **W2.3**：从 `llm/chat_state.rs`（93 行）+ `llm/cancel.rs`（46 行）合并迁入。
//! **M1.4**：`CancellationToken` 下沉到 [`crate::infra::cancel`]（让 context 层
//! 可直接引用，避免反向依赖），本模块保留 re-export 以兼容历史路径。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::error::{AppError, AppResult};

// =========================================================================
// CancellationToken re-export（定义见 `crate::infra::cancel`）
// =========================================================================

pub use crate::infra::cancel::CancellationToken;

// =========================================================================
// ChatState（从 llm/chat_state.rs 迁入）
// =========================================================================

/// 全局聊天状态（注入到 Tauri managed state）
/// （`Clone` 见文件尾手写 impl——同柄 `Arc` 共享，屏幕通道活性查询共用）
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

    /// 快照某会话**此刻在途回合**的取消令牌（Steer 分支用，W1 ②）。
    ///
    /// 与 [`Self::stop`] 的差别 = 回合身份：stop 取消「调用时刻注册表里的那个
    /// 令牌」——大附件物化耗时超过回合 A 自然收尾时，迟到的 stop 会误伤已起跑
    /// 的后续回合 B。本方法把快照令牌交还调用方，由调用方在物化完成后点名取消
    /// 快照令牌；A 已收尾则快照成死令牌，cancel 只是置位一个无人再读的布尔
    ///（[`CancellationToken::cancel`] 幂等无害），不伤后来者。
    pub fn token_of(&self, conv_id: &str) -> Option<CancellationToken> {
        self.lock().get(conv_id).cloned()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 空闲会话（无在途令牌）→ None（Steer 分支据此走普通发送路径）。
    #[test]
    fn token_of_none_when_idle() {
        let state = ChatState::new();
        assert!(state.token_of("conv-idle").is_none());
    }

    /// 在途回合 → Some(共享同柄令牌)：cancel 快照 = cancel 本体。
    #[test]
    fn token_of_snapshots_live_token() {
        let state = ChatState::new();
        let live = state.start("conv-live").unwrap();

        let snapshot = state.token_of("conv-live").expect("在途应有快照");
        snapshot.cancel();
        assert!(live.is_cancelled(), "快照与本体共享底层状态");
        assert!(snapshot.is_cancelled());
    }

    /// W1 ② 核心场景回归锁：快照后 A 收尾注销、新回合 B 注册新令牌——
    /// 迟到的快照 cancel 绝不能打到 B（stop 误伤续跑回合的根因）。
    #[test]
    fn dead_snapshot_cancel_never_hits_new_turn() {
        let state = ChatState::new();
        let _a = state.start("conv-race").unwrap();
        let stale = state.token_of("conv-race").unwrap();

        // A 自然收尾 → 注销；续跑回合 B 起跑 → 新令牌在册
        state.unregister("conv-race");
        let b = state.start("conv-race").unwrap();

        stale.cancel(); // 迟到的取消打在已注销的快照上
        assert!(!b.is_cancelled(), "死令牌 cancel 不得误伤新回合 B");
        assert!(state.is_streaming("conv-race"), "B 仍在途");
    }
}
