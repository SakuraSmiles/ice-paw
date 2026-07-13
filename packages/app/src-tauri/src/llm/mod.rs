//! LLM 抽象层
//!
//! 设计要点：
//! - `LlmProvider` trait 定义统一的流式聊天接口，各厂商 Adapter 实现之
//! - `ChatMessage` / `ChatDelta` 为跨层数据结构
//! - `create_provider` 工厂根据 provider 名称返回对应 Adapter
//!   - OpenAI 兼容：openai / glm / deepseek
//!   - Anthropic 兼容：anthropic / minimax / minimax-cn
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
/// 支持两类协议：
/// - OpenAI Chat Completions 兼容：openai / glm / deepseek
/// - Anthropic Messages API 兼容：anthropic / minimax / minimax-cn
///
/// 未识别的 provider 兜底走 OpenAI 兼容（向后兼容），并打 warn 日志。
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

    // 调试用：对已知协议打印最终 chat URL（含智能拼接）。
    // 排查 base_url 路径问题时一眼看出拼接是否正确。
    let chat_url_preview: Option<String> = match provider {
        "openai" | "glm" | "deepseek" => Some(adapters::openai::build_chat_url(&url)),
        "anthropic" | "minimax" | "minimax-cn" => {
            Some(format!("{}/v1/messages", url.trim_end_matches('/')))
        }
        _ => None,
    };

    tracing::info!(
        target: "ice_paw.llm",
        "创建 Provider: {} | model={} | base_url={}{}",
        provider,
        model,
        url,
        chat_url_preview
            .as_deref()
            .map(|u| format!(" | chat_url={}", u))
            .unwrap_or_default(),
    );

    match provider {
        // OpenAI Chat Completions 兼容厂商
        "openai" | "glm" | "deepseek" => Ok(Arc::new(
            adapters::openai::OpenAiAdapter::new(model.to_string(), url),
        )),
        // Anthropic Messages API 兼容厂商（Anthropic 官方 + MiniMax）
        "anthropic" | "minimax" | "minimax-cn" => Ok(Arc::new(
            adapters::anthropic::AnthropicAdapter::new(model.to_string(), url),
        )),
        // 兜底：未识别 provider 走 OpenAI 兼容（向后兼容）
        _ => {
            tracing::warn!(
                target: "ice_paw.llm",
                "未知 provider '{}'，兜底走 OpenAI 兼容",
                provider
            );
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
        "anthropic" => "https://api.anthropic.com".to_string(),
        "minimax" => "https://api.minimaxi.com/anthropic".to_string(),
        "minimax-cn" => "https://api.minimaxi.cn/anthropic".to_string(),
        // 兜底：返回空串让上层报错（调用方应在 agent 配置里写 base_url）
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 工厂应当返回 Ok（不应报错；URL / 模型透传）
    #[test]
    fn factory_returns_ok_for_known_providers() {
        for p in [
            "openai",
            "glm",
            "deepseek",
            "anthropic",
            "minimax",
            "minimax-cn",
        ] {
            let r = create_provider(p, "model-x", Some("https://x.com"));
            assert!(
                r.is_ok(),
                "known provider '{}' 应返回 Ok，实际: {:?}",
                p,
                r.err()
            );
            // 模型名应透传（以 URL 为 url, 验证创建不 panic）
            let _ = r.unwrap();
        }
    }

    /// 未知 provider 应当被接收（兑底走 OpenAI 兼容），不报错（向后兼容）
    #[test]
    fn factory_unknown_provider_falls_back() {
        let r = create_provider("totally-unknown-thing", "m", Some("https://x.com"));
        assert!(r.is_ok());
    }

    /// provider 未传 base_url 时会用 provider 默认值
    #[test]
    fn factory_uses_default_url_when_base_url_missing() {
        // glm 的默认 URL 应被使用（不需 Ok 中解析出 URL，但不应报错且不应 panic）
        let r = create_provider("glm", "m", None);
        assert!(r.is_ok());
        let r = create_provider("minimax-cn", "m", None);
        assert!(r.is_ok());
    }

    /// 默认 URL 表必须准确：三个新增 Anthropic 协议供应商
    #[test]
    fn default_base_urls() {
        assert_eq!(default_base_url("anthropic"), "https://api.anthropic.com");
        assert_eq!(
            default_base_url("minimax"),
            "https://api.minimaxi.com/anthropic"
        );
        assert_eq!(
            default_base_url("minimax-cn"),
            "https://api.minimaxi.cn/anthropic"
        );
        // 回归：原有三个不变
        assert_eq!(default_base_url("openai"), "https://api.openai.com");
        assert_eq!(
            default_base_url("glm"),
            "https://open.bigmodel.cn/api/paas/v4"
        );
        assert_eq!(default_base_url("deepseek"), "https://api.deepseek.com");
        // 兑底返回空串
        assert_eq!(default_base_url(""), "");
        assert_eq!(default_base_url("totally-unknown"), "");
    }
}
