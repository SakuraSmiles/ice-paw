//! 全局聊天状态管理
//!
//! 维护 `conversation_id → CancellationToken` 映射，
//! 用于跟踪哪些会话正在流式生成，以及支持用户手动停止。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::llm::cancel::CancellationToken;

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

    /// 注册一个会话的生成任务
    ///
    /// 返回新建的 CancellationToken，供流式协程持有。
    /// 如果同一会话已有在途生成，会被覆盖（理论上不应发生，前端应做串行守门）。
    pub fn start(&self, conv_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        let mut map = self.inner.lock().expect("ChatState: mutex poisoned");
        map.insert(conv_id.to_string(), token.clone());
        token
    }

    /// 直接注册一个已有的 CancellationToken
    pub fn register(&self, conv_id: &str, token: CancellationToken) {
        let mut map = self.inner.lock().expect("ChatState: mutex poisoned");
        map.insert(conv_id.to_string(), token);
    }

    /// 触发某会话的取消（用户点击「停止」）
    ///
    /// 返回是否命中（true = 确有在途生成并被取消）
    pub fn stop(&self, conv_id: &str) -> bool {
        let map = self.inner.lock().expect("ChatState: mutex poisoned");
        if let Some(token) = map.get(conv_id) {
            token.cancel();
            true
        } else {
            false
        }
    }

    /// 注销某会话的令牌（流式协程结束时调用）
    pub fn unregister(&self, conv_id: &str) {
        let mut map = self.inner.lock().expect("ChatState: mutex poisoned");
        map.remove(conv_id);
    }

    /// 某会话是否正在流式生成
    pub fn is_streaming(&self, conv_id: &str) -> bool {
        let map = self.inner.lock().expect("ChatState: mutex poisoned");
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
