//! `harness::provider` — LLM provider adapters + 工厂函数
//!
//! - W2.1：从 `llm/adapters/openai.rs` 迁入 OpenAI 兼容 Adapter
//! - W2.2：从 `llm/adapters/anthropic.rs` 迁入 Anthropic 兼容 Adapter
//! - W2.3：合并 `llm/mod.rs` 中的 `create_provider()` + `default_base_url()` 工厂函数，
//!   删除整个 `llm/` 目录
//!
//! 设计要点：
//! - `LlmProvider` trait 定义统一的流式聊天接口，各厂商 Adapter 实现之
//! - `create_provider` 工厂根据 provider 名称返回对应 Adapter
//!   - OpenAI 兼容：openai / glm / deepseek
//!   - Anthropic 兼容：anthropic / minimax / minimax-cn
//! - API Key 不存储于 Adapter，每次调用时传入，降低泄露风险
//!
//! 详见 Sprint 计划 W2.1–W2.3。

pub mod anthropic;
pub mod embedding;
pub mod mock;
pub mod openai;
pub use anthropic::AnthropicAdapter;
pub use mock::{MockProvider, MockScenario};
pub use openai::OpenAiAdapter;

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;

use crate::error::AppResult;
use crate::harness::chat_state::CancellationToken;
use crate::infra::protocol::{ChatDelta, ChatMessage, ToolDef};

/// LLM 提供方接口（从 `infra::protocol` 迁入，归属 provider 模块）
///
/// 实现方需提供 `stream_chat`，返回一个异步 Stream 逐块产出 `ChatDelta`。
/// 调用方在消费 Stream 时应定期检查 `cancel.is_cancelled()` 以支持用户停止。
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// 流式聊天
    #[allow(clippy::too_many_arguments)]
    async fn stream_chat(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDef>>,
        temperature: f64,
        max_tokens: i32,
        model: Option<&str>,
        cancel: CancellationToken,
    ) -> AppResult<Pin<Box<dyn futures::Stream<Item = AppResult<ChatDelta>> + Send>>>;

    /// 返回当前 Provider 实际使用的模型名（用于消息级记录）
    fn model_name(&self) -> &str;
}

// =========================================================================
// 工厂函数（从 llm/mod.rs 迁入）
// =========================================================================

/// Provider 描述符：名称 → 协议类型 + 默认 URL
struct ProviderDesc {
    name: &'static str,
    protocol: ProviderProtocol,
    default_url: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProviderProtocol {
    OpenAI,
    Anthropic,
}

/// 数据驱动的 provider 注册表（单一真相源，消除 create_provider 与
/// default_base_url 两份 match 语句的同步风险）。
const PROVIDERS: &[ProviderDesc] = &[
    ProviderDesc { name: "openai",    protocol: ProviderProtocol::OpenAI,    default_url: "https://api.openai.com" },
    ProviderDesc { name: "glm",       protocol: ProviderProtocol::OpenAI,    default_url: "https://open.bigmodel.cn/api/paas/v4" },
    ProviderDesc { name: "deepseek",  protocol: ProviderProtocol::OpenAI,    default_url: "https://api.deepseek.com" },
    ProviderDesc { name: "anthropic", protocol: ProviderProtocol::Anthropic, default_url: "https://api.anthropic.com" },
    ProviderDesc { name: "minimax",   protocol: ProviderProtocol::Anthropic, default_url: "https://api.minimaxi.com/anthropic" },
    ProviderDesc { name: "minimax-cn",protocol: ProviderProtocol::Anthropic, default_url: "https://api.minimaxi.com/anthropic" },
];

fn find_provider(name: &str) -> Option<&'static ProviderDesc> {
    PROVIDERS.iter().find(|p| p.name == name)
}

/// 按 provider 名称创建对应的 LLM Adapter。
///
/// 数据驱动：从 `PROVIDERS` 注册表查找协议类型 + 默认 URL，
/// 消除 create_provider / default_base_url 两份 match 同步风险。
///
/// 未识别的 provider 兜底走 OpenAI 兼容（向后兼容），并打 warn 日志。
pub fn create_provider(
    provider: &str,
    model: &str,
    base_url: Option<&str>,
    cache_prompt: bool,
) -> AppResult<Arc<dyn LlmProvider>> {
    let desc = find_provider(provider);
    let url = match base_url {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => desc.map(|d| d.default_url).unwrap_or("").to_string(),
    };

    let protocol = desc.map(|d| d.protocol);

    tracing::info!(
        target: "ice_paw.llm",
        "创建 Provider: {} | model={} | base_url={} | protocol={}",
        provider,
        model,
        url,
        match protocol {
            Some(ProviderProtocol::OpenAI) => "openai",
            Some(ProviderProtocol::Anthropic) => "anthropic",
            None => "unknown(fallback=openai)",
        },
    );

    match protocol {
        Some(ProviderProtocol::OpenAI) => Ok(Arc::new(
            OpenAiAdapter::new(model.to_string(), url)?,
        )),
        Some(ProviderProtocol::Anthropic) => Ok(Arc::new(
            AnthropicAdapter::new(model.to_string(), url, cache_prompt)?,
        )),
        None => {
            tracing::warn!(
                target: "ice_paw.llm",
                "未知 provider '{}'，兜底走 OpenAI 兼容",
                provider
            );
            Ok(Arc::new(OpenAiAdapter::new(model.to_string(), url)?))
        }
    }
}

/// 各 provider 的默认 base_url（委托给 PROVIDERS 注册表）。
/// 仅测试使用；生产代码内联读取 `PROVIDERS`。
#[allow(dead_code)]
fn default_base_url(provider: &str) -> String {
    find_provider(provider)
        .map(|d| d.default_url)
        .unwrap_or("")
        .to_string()
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
            let r = create_provider(p, "model-x", Some("https://x.com"), true);
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
        let r = create_provider("totally-unknown-thing", "m", Some("https://x.com"), false);
        assert!(r.is_ok());
    }

    /// provider 未传 base_url 时会用 provider 默认值
    #[test]
    fn factory_uses_default_url_when_base_url_missing() {
        // glm 的默认 URL 应被使用（不需 Ok 中解析出 URL，但不应报错且不应 panic）
        let r = create_provider("glm", "m", None, false);
        assert!(r.is_ok());
        let r = create_provider("minimax-cn", "m", None, false);
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
            "https://api.minimaxi.com/anthropic"
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
