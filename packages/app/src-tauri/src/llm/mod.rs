//! LLM 抽象层
//!
//! 设计要点：
//! - `LlmProvider` trait 定义统一的流式聊天接口，各厂商 Adapter 实现之
//! - `ChatMessage` / `ChatDelta` 为跨层数据结构
//! - `create_provider` 工厂根据 provider 名称返回对应 Adapter（Phase 1 仅 OpenAI 兼容）
//! - API Key 不存储于 Adapter，每次调用时传入，降低泄露风险

pub mod adapters;
pub mod cancel;
pub mod chat_state;

pub use cancel::CancellationToken;
pub use chat_state::ChatState;

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::error::AppResult;

// =========================================================================
// 数据结构
// =========================================================================

/// 聊天消息（发给 LLM 的上下文中的单条）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// 角色："system" | "user" | "assistant"
    pub role: String,
    /// 消息内容
    pub content: String,
}

/// 流式增量 — LLM 返回的每个 chunk
///
/// - `Delta`：内容增量（最常见）
/// - `Done`：流结束（携带结束原因）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatDelta {
    /// 内容增量
    Delta { content: String },
    /// 流结束
    Done { finish_reason: Option<String> },
}

// =========================================================================
// Provider Trait
// =========================================================================

/// LLM 提供方接口
///
/// 实现方需提供 `stream_chat`，返回一个异步 Stream 逐块产出 `ChatDelta`。
/// 调用方在消费 Stream 时应定期检查 `cancel.is_cancelled()` 以支持用户停止。
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// 流式聊天
    ///
    /// - `api_key`：调用时传入，不在 Adapter 中持久化
    /// - `messages`：完整上下文（含 system / 历史 / 当前用户消息）
    /// - `temperature` / `max_tokens`：模型参数
    /// - `cancel`：取消令牌
    async fn stream_chat(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        temperature: f64,
        max_tokens: i32,
        cancel: CancellationToken,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<ChatDelta>> + Send>>>;
}

// =========================================================================
// 工厂函数
// =========================================================================

/// 按 provider 名称创建对应的 LLM Adapter
///
/// Phase 1 仅支持 OpenAI 兼容协议（OpenAI / GLM / DeepSeek / 自定义 base_url）。
/// Anthropic 留 Phase 2。
///
/// - `provider`：agent.provider 字段
/// - `model`：agent.model 字段
/// - `base_url`：优先取 agent.base_url，为空则用 provider 对应默认值
pub fn create_provider(
    provider: &str,
    model: &str,
    base_url: Option<&str>,
) -> AppResult<Arc<dyn LlmProvider>> {
    let url = match base_url {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => default_base_url(provider),
    };

    tracing::info!(
        target: "ice_paw.llm",
        "创建 Provider: {} | model={} | base_url={}",
        provider, model, url
    );

    match provider {
        "openai" | "glm" | "deepseek" | _ => {
            // 所有 OpenAI 兼容的 provider 走同一 adapter
            Ok(Arc::new(adapters::openai::OpenAiAdapter::new(
                model.to_string(),
                url,
            )))
        }
    }
}

/// 各 provider 的默认 base_url
fn default_base_url(provider: &str) -> String {
    match provider {
        "openai" => "https://api.openai.com".to_string(),
        "glm" => "https://open.bigmodel.cn/api/paas/v4".to_string(),
        "deepseek" => "https://api.deepseek.com".to_string(),
        // 兜底：返回空串让上层报错（调用方应在 agent 配置里写 base_url）
        _ => String::new(),
    }
}
