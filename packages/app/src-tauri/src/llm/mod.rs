//! LLM 抽象层
//!
//! 设计要点：
//! - `LlmProvider` trait 定义统一的流式聊天接口，各厂商 Adapter 实现之
//! - `ChatMessage` / `ChatDelta` 为跨层数据结构
//! - `create_provider` 工厂根据 provider 名称返回对应 Adapter
//!   - OpenAI 兼容：openai / glm / deepseek
//!   - Anthropic 兼容：anthropic / minimax / minimax-cn
//! - API Key 不存储于 Adapter，每次调用时传入，降低泄露风险
//!
//! P2-1 升级：
//! - `ChatMessage.content` 改为 `Vec<ContentBlock>`（同时保留 `content_text` 兼容旧消息）
//! - `ChatDelta` 新增工具调用 / 思考变体
//! - `LlmProvider::stream_chat` 新增 `tools` 参数
//! - 新增 `ToolDef` / `ContentBlock` 类型

pub mod adapters;
pub mod cancel;
pub mod chat_state;
pub mod tool_registry;

pub use cancel::CancellationToken;
pub use chat_state::ChatState;
pub use tool_registry::ToolRegistry;

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::error::AppResult;

// =========================================================================
// 数据结构
// =========================================================================

/// 消息内容块 — 替代原来的 `content: String`
///
/// 采用 `#[serde(tag = "type")]` 实现多态 JSON 序列化，
/// 与 OpenAI / Anthropic 的 content block 格式自然对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// 文本块
    Text { text: String },
    /// P2-2: 图片块（Vision 输入）
    ///
    /// - `data`：base64 编码的图片数据（**不含** `data:image/...;base64,` 前缀，
    ///   前缀在 adapter 里拼接；这样前端只传裸 base64，存储/校验更干净）
    /// - `media_type`：MIME 类型，支持 `"image/png" | "image/jpeg" | "image/gif" | "image/webp"`
    ///
    /// 序列化格式（与前端 `types/index.ts` 对齐）：
    /// ```json
    /// { "type": "image", "data": "iVBORw0KG...", "media_type": "image/png" }
    /// ```
    Image {
        data: String,
        media_type: String,
    },
    /// 工具调用（LLM 产出）
    ToolUse {
        id: String,
        name: String,
        /// JSON 字符串（arguments / input）
        input: String,
    },
    /// 工具结果（回传给 LLM）
    ToolResult {
        tool_use_id: String,
        /// 结果内容（JSON 字符串或纯文本）
        content: String,
        /// 是否出错
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    /// 思考过程（Anthropic extended thinking）
    Thinking {
        thinking: String,
        /// 签名（Anthropic 用于验证）
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
}

impl ContentBlock {
    /// 从纯文本构造 Text block
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }

    /// P2-2: 构造 Image block（裸 base64，无 data URL 前缀）
    pub fn image(data: impl Into<String>, media_type: impl Into<String>) -> Self {
        ContentBlock::Image {
            data: data.into(),
            media_type: media_type.into(),
        }
    }

    /// 提取纯文本内容（仅 Text 变体有）
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text { text } => Some(text),
            _ => None,
        }
    }

    /// P2-2: 是否是 Image 块
    pub fn is_image(&self) -> bool {
        matches!(self, ContentBlock::Image { .. })
    }

    /// 把所有 Text block 的文本拼接成一个 String（兼容旧代码）
    pub fn join_text(blocks: &[ContentBlock]) -> String {
        let mut buf = String::new();
        for b in blocks {
            if let ContentBlock::Text { text } = b {
                buf.push_str(text);
            }
        }
        buf
    }
}

/// P2-2: 支持的图片 MIME 类型白名单
///
/// 与前端 `ImagePicker.vue` 的 `accept` 属性保持一致。
/// Anthropic 支持 `image/jpeg | image/png | image/gif | image/webp`；
/// OpenAI Vision 支持同等集合（部分模型额外支持 `image/png` 高分辨率）。
pub const SUPPORTED_IMAGE_MEDIA_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
];

/// P2-2: 校验 media_type 是否在白名单内
pub fn is_supported_image_media_type(mt: &str) -> bool {
    SUPPORTED_IMAGE_MEDIA_TYPES.contains(&mt)
}

/// 聊天消息（发给 LLM 的上下文中的单条）
///
/// P2-1 升级：`content` 改为 `Vec<ContentBlock>`。
/// 对旧消息（纯文本）使用 `ChatMessage::from_text` 构造。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// 角色："system" | "user" | "assistant" | "tool"
    pub role: String,
    /// 消息内容块数组
    pub content: Vec<ContentBlock>,
}

impl ChatMessage {
    /// 从纯文本快速构造（等同旧版行为）
    pub fn from_text(role: impl Into<String>, content: impl Into<String>) -> Self {
        ChatMessage {
            role: role.into(),
            content: vec![ContentBlock::text(content)],
        }
    }

    /// 把所有 content block 拼成纯文本（兼容旧逻辑 / DB 回写）
    pub fn content_text(&self) -> String {
        ContentBlock::join_text(&self.content)
    }
}

/// P2-3: Token 用量信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// P2-3: 缓存命中的 token 数（Anthropic: cache_read_input_tokens, OpenAI: cached_tokens）
    #[serde(default)]
    pub cached_tokens: u32,
}

/// 流式增量 — LLM 返回的每个 chunk
///
/// - `Delta`：文本增量（最常见）
/// - `ToolCallStart`：工具调用开始（id + name 已知）
/// - `ToolCallDelta`：工具调用参数 JSON 片段
/// - `ToolCallEnd`：工具调用参数完毕
/// - `Thinking`：思考过程增量
/// - `Usage`：P2-3 token 用量（OpenAI streaming usage 或 Anthropic message_start）
/// - `Done`：流结束（携带结束原因）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatDelta {
    /// 文本增量
    Delta { content: String },
    /// 工具调用开始
    ToolCallStart { id: String, name: String },
    /// 工具调用参数 JSON 增量
    ToolCallDelta { id: String, delta: String },
    /// 工具调用参数完成
    ToolCallEnd { id: String },
    /// 思考过程增量
    Thinking { content: String },
    /// P2-3: Token 用量
    Usage { usage: TokenUsage },
    /// 流结束
    Done { finish_reason: Option<String> },
}

/// 工具定义（发给 LLM 的 tool schema）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema（parameters）
    pub parameters: serde_json::Value,
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
    /// - `tools`：可选的工具定义列表（None = 不启用工具调用）
    /// - `temperature` / `max_tokens`：模型参数
    /// - `cancel`：取消令牌
    async fn stream_chat(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDef>>,
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
/// - `cache_prompt`：P2-3 是否启用 prompt caching（仅 Anthropic 协议生效）
pub fn create_provider(
    provider: &str,
    model: &str,
    base_url: Option<&str>,
    cache_prompt: bool,
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
            adapters::anthropic::AnthropicAdapter::new(model.to_string(), url, cache_prompt),
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

    // ================================================================
    // P2-2: ContentBlock::Image 序列化 + 白名单
    // ================================================================

    #[test]
    fn image_block_serde_roundtrip() {
        let block = ContentBlock::image("iVBORw0KGgo=", "image/png");
        let json = serde_json::to_string(&block).unwrap();
        // tag = "type", rename_all = "snake_case"
        assert_eq!(
            json,
            r#"{"type":"image","data":"iVBORw0KGgo=","media_type":"image/png"}"#
        );
        // 反序列化回原值
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        match back {
            ContentBlock::Image { data, media_type } => {
                assert_eq!(data, "iVBORw0KGgo=");
                assert_eq!(media_type, "image/png");
            }
            _ => panic!("反序列化后类型不对：{:?}", back),
        }
    }

    #[test]
    fn image_block_helper() {
        let b = ContentBlock::image("abc", "image/jpeg");
        assert!(b.is_image());
        assert!(b.as_text().is_none());
    }

    #[test]
    fn text_block_not_image() {
        let b = ContentBlock::text("hello");
        assert!(!b.is_image());
        assert_eq!(b.as_text(), Some("hello"));
    }

    #[test]
    fn supported_media_types_whitelist() {
        for mt in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
            assert!(
                is_supported_image_media_type(mt),
                "{} 应在白名单内",
                mt
            );
        }
        for mt in ["image/bmp", "image/svg+xml", "application/pdf", "", "png"] {
            assert!(
                !is_supported_image_media_type(mt),
                "{} 不应在白名单内",
                mt
            );
        }
    }

    #[test]
    fn join_text_skips_images() {
        // P2-2: join_text 只拼接 Text 块，忽略 Image/ToolUse 等
        let blocks = vec![
            ContentBlock::text("hello "),
            ContentBlock::image("xxxx", "image/png"),
            ContentBlock::text("world"),
        ];
        assert_eq!(ContentBlock::join_text(&blocks), "hello world");
    }

    /// 混合消息（含图片）的 JSON 序列化结构对齐前端 types/index.ts
    #[test]
    fn mixed_message_json_shape() {
        let blocks = vec![
            ContentBlock::text("看这张图"),
            ContentBlock::image("AAAA", "image/png"),
        ];
        let json = serde_json::to_string(&blocks).unwrap();
        // 验证 JSON 数组中两个对象的结构
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);

        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "看这张图");

        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[1]["data"], "AAAA");
        assert_eq!(arr[1]["media_type"], "image/png");
        // Image 没有其他字段
        assert_eq!(arr[1].as_object().unwrap().len(), 3);
    }
}
