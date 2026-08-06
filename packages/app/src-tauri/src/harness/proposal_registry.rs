//! Config Proposal Registry — 提案 oneshot 通道注册表
//!
//! 基于泛型 `OneshotRegistry<T>` 实现，与 `ToolAuthRegistry` 共享同一套
//! 注册/响应/监听逻辑，仅响应类型和事件名不同。
//!
//! 生命周期：在 `lib.rs::setup` 阶段调用 `install_listener()`
//! 注册 Tauri 事件监听。

pub use crate::harness::oneshot_registry::ProposalRegistry;

// =========================================================================
// 单元测试（已由 oneshot_registry 泛型测试覆盖；此处保留集成测试）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::protocol::{ConfigProposalResponse, ProposalDecision};

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
