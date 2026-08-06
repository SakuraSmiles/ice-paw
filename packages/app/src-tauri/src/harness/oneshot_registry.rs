//! 泛型 Oneshot 通道注册表 — 消除 ToolAuthRegistry / ProposalRegistry 重复
//!
//! 两个注册表完全同构（~100 行重复），仅响应类型和事件名不同。
//! 提取为泛型 `OneshotRegistry<T: RegistryResponse>`，两者均通过此实现。

use std::collections::HashMap;
use std::sync::Arc;

use tauri::AppHandle;
use tokio::sync::{oneshot, Mutex};

use crate::infra::protocol::{ConfigProposalResponse, ToolAuthResponse};

/// oneshot 响应的统一 trait：每个响应类型必须能提供 request_id
/// 供 `OneshotRegistry::respond()` 按 ID 匹配等待者。
pub trait RegistryResponse: Send + 'static {
    fn request_id(&self) -> &str;
}

type SenderMap<T> = Arc<Mutex<HashMap<String, oneshot::Sender<T>>>>;

/// 泛型 oneshot 通道注册表。
///
/// 维护 `request_id → oneshot::Sender<T>`，供前端响应事件按 request_id
/// 解锁对应等待者。
///
/// 生命周期：在 `lib.rs::setup` 阶段调用 `install_listener()` 注册 Tauri
/// 事件监听。
pub struct OneshotRegistry<T: RegistryResponse> {
    inner: SenderMap<T>,
}

impl<T: RegistryResponse> Clone for OneshotRegistry<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: RegistryResponse> Default for OneshotRegistry<T> {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl<T: RegistryResponse> std::fmt::Debug for OneshotRegistry<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OneshotRegistry").finish_non_exhaustive()
    }
}

impl<T: RegistryResponse + serde::de::DeserializeOwned> OneshotRegistry<T> {
    /// 新建空注册表
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个新的等待者，返回 receiver
    pub async fn register(&self, request_id: String) -> oneshot::Receiver<T> {
        let (tx, rx) = oneshot::channel();
        let mut map = self.inner.lock().await;
        map.insert(request_id, tx);
        rx
    }

    /// 取出并删除一个等待者（用于取消/超时时清理）
    pub async fn take(&self, request_id: &str) -> Option<oneshot::Sender<T>> {
        let mut map = self.inner.lock().await;
        map.remove(request_id)
    }

    /// 用响应唤醒一个等待者
    pub async fn respond(&self, response: T) -> bool {
        let mut map = self.inner.lock().await;
        if let Some(tx) = map.remove(response.request_id()) {
            // send 失败说明 receiver 已被 drop（例如上层取消）→ 忽略
            let _ = tx.send(response);
            true
        } else {
            false
        }
    }

    /// 当前等待者数量（仅供调试/测试）
    #[cfg(test)]
    pub async fn pending_count(&self) -> usize {
        let map = self.inner.lock().await;
        map.len()
    }

    /// 安装前端事件监听器。
    ///
    /// - `event_name`: Tauri 事件名（如 `"chat:tool-auth-response"`）
    /// - `on_unknown`: 收到未知 request_id 时的日志消息
    /// - `on_parse_error`: 解析失败时的日志消息
    ///
    /// 在 `lib.rs::setup` 阶段调用一次。内部 clone 后 spawn，与原始
    /// ToolAuthRegistry 相同的验证过的模式（`&self` → clone → spawn）。
    pub fn install_listener(
        &self,
        app: &AppHandle,
        event_name: String,
        on_unknown: String,
        on_parse_error: String,
    ) {
        let registry = self.clone();
        let app_handle = app.clone();
        let ename = event_name.clone();
        tauri::async_runtime::spawn(async move {
            use tauri::Listener;
            tracing::info!(
                target: "ice_paw.mgmt",
                "注册事件监听器: event={}", ename
            );
            app_handle.listen(ename.clone(), move |ev| {
                let reg = registry.clone();
                let payload_str = ev.payload().to_string();
                tracing::info!(
                    target: "ice_paw.mgmt",
                    "收到事件: event={} payload_len={}", ename, payload_str.len()
                );
                let response: Result<T, _> = serde_json::from_str(&payload_str);
                let um = on_unknown.clone();
                let pe = on_parse_error.clone();
                tauri::async_runtime::spawn(async move {
                    match response {
                        Ok(r) => {
                            if !reg.respond(r).await {
                                tracing::warn!("{}", um);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("{}: {} (payload={})", pe, e, payload_str);
                        }
                    }
                });
            });
        });
    }
}

// =========================================================================
// 具体类型的 RegistryResponse 实现
// =========================================================================

impl RegistryResponse for ToolAuthResponse {
    fn request_id(&self) -> &str {
        &self.request_id
    }
}

impl RegistryResponse for ConfigProposalResponse {
    fn request_id(&self) -> &str {
        &self.request_id
    }
}

// =========================================================================
// 类型别名：消除 ToolAuthRegistry / ProposalRegistry 代码重复
// =========================================================================

/// 工具授权响应注册表（A2-3）
pub type ToolAuthRegistry = OneshotRegistry<ToolAuthResponse>;

/// 配置提案响应注册表（Phase 1）
pub type ProposalRegistry = OneshotRegistry<ConfigProposalResponse>;
