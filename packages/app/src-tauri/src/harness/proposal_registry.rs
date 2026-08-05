//! Config Proposal Registry — 提案 oneshot 通道注册表
//!
//! 与 `ToolAuthRegistry` 完全同构，独立的 oneshot 通道管理。
//! 提案工具 emit `chat:config-proposal` 事件后，
//! 用 `request_id` 匹配 oneshot 等待前端响应。
//!
//! 生命周期：在 `lib.rs::setup` 阶段调用 `install_listener()`
//! 注册 Tauri 事件监听。

use std::collections::HashMap;
use std::sync::Arc;

use tauri::AppHandle;
use tokio::sync::{oneshot, Mutex};

use crate::infra::protocol::ConfigProposalResponse;

type ProposalSenderMap = Arc<Mutex<HashMap<String, oneshot::Sender<ConfigProposalResponse>>>>;

#[derive(Clone, Default)]
pub struct ProposalRegistry {
    inner: ProposalSenderMap,
}

impl std::fmt::Debug for ProposalRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProposalRegistry").finish_non_exhaustive()
    }
}

impl ProposalRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 注册一个新的等待者，返回 receiver
    pub async fn register(
        &self,
        request_id: String,
    ) -> oneshot::Receiver<ConfigProposalResponse> {
        let (tx, rx) = oneshot::channel();
        let mut map = self.inner.lock().await;
        map.insert(request_id, tx);
        rx
    }

    /// 取出并删除一个等待者（用于取消/超时时清理）
    pub async fn take(&self, request_id: &str) -> Option<oneshot::Sender<ConfigProposalResponse>> {
        let mut map = self.inner.lock().await;
        map.remove(request_id)
    }

    /// 用响应唤醒一个等待者
    pub async fn respond(&self, response: ConfigProposalResponse) -> bool {
        let mut map = self.inner.lock().await;
        if let Some(tx) = map.remove(&response.request_id) {
            let _ = tx.send(response);
            true
        } else {
            false
        }
    }

    /// 安装前端 `chat:config-proposal-response` 事件监听器
    ///
    /// 在 `lib.rs::run()` setup 阶段调用一次。
    /// 克隆 `self`（内部是 `Arc`，克隆即共享）。
    pub fn install_listener(&self, app: &AppHandle) {
        let registry = self.clone();
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            use tauri::Listener;
            let _ = app_handle.listen("chat:config-proposal-response", move |event| {
                let registry = registry.clone();
                let payload_str = event.payload().to_string();
                let response: Result<ConfigProposalResponse, _> =
                    serde_json::from_str(&payload_str);
                tauri::async_runtime::spawn(async move {
                    match response {
                        Ok(r) => {
                            let handled = registry.respond(r).await;
                            if !handled {
                                tracing::warn!(
                                    target: "ice_paw.mgmt",
                                    "收到未知 request_id 的提案响应（可能已超时）",
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                target: "ice_paw.mgmt",
                                "提案响应解析失败: {} (payload={})",
                                e,
                                payload_str,
                            );
                        }
                    }
                });
            });
        });
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::protocol::ProposalDecision;

    #[tokio::test]
    async fn register_and_respond() {
        let reg = ProposalRegistry::new();
        let req_id = "req-1".to_string();
        let rx = reg.register(req_id.clone()).await;

        let reg2 = reg.clone();
        tokio::spawn(async move {
            reg2.respond(ConfigProposalResponse {
                request_id: req_id.clone(),
                decision: ProposalDecision::Approved,
            })
            .await;
        });

        let resp = rx.await.unwrap();
        match resp.decision {
            ProposalDecision::Approved => {}
            _ => panic!("expected Approved"),
        }
    }

    #[tokio::test]
    async fn respond_unknown_id_returns_false() {
        let reg = ProposalRegistry::new();
        let handled = reg
            .respond(ConfigProposalResponse {
                request_id: "nope".into(),
                decision: ProposalDecision::Rejected { reason: None },
            })
            .await;
        assert!(!handled);
    }

    #[tokio::test]
    async fn take_removes_sender() {
        let reg = ProposalRegistry::new();
        let req_id = "req-take".to_string();
        let _rx = reg.register(req_id.clone()).await;
        let taken = reg.take(&req_id).await;
        assert!(taken.is_some());
        // 二次 take 返回 None
        assert!(reg.take(&req_id).await.is_none());
    }

    #[tokio::test]
    async fn clone_shares_state() {
        let reg1 = ProposalRegistry::new();
        let reg2 = reg1.clone();
        let _rx = reg1.register("shared".into()).await;
        // reg2 也能找到
        let taken = reg2.take("shared").await;
        assert!(taken.is_some());
    }
}
