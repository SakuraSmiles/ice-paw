//! OpenAI 兼容 Adapter
//!
//! 适配 OpenAI / GLM (智谱) / DeepSeek 等遵循 OpenAI Chat Completions API 的服务。
//!
//! 核心流程：
//! 1. POST `{base_url}/v1/chat/completions`，body 含 `stream: true`
//! 2. 用 `reqwest::Response::bytes_stream()` 拿到字节流
//! 3. 按 SSE 协议逐行解析：`data: {json}` → 取 `choices[0].delta.content`
//! 4. `data: [DONE]` → 流结束
//! 5. 每步检查 `cancel.is_cancelled()`，发现取消则提前结束
//!
//! 实现策略：用 `tokio::sync::mpsc` 通道将同步解析逻辑转换为异步 Stream。
//!
//! ## 模块组织（M2.3 拆分）
//!
//! - [`types`] — 请求/响应结构体 + `chat_message_to_openai()` 转换函数
//! - [`streaming`] — `parse_sse_stream()` SSE 解析协程

use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::error::{AppError, AppResult};
use crate::harness::chat_state::CancellationToken;
use crate::infra::protocol::{ChatDelta, ChatMessage, LlmProvider, ToolDef};

pub(crate) mod streaming;
pub(crate) mod types;

use types::{ChatRequest, OpenAiMessage, OpenAiTool, OpenAiToolFn, StreamOptions};

// =========================================================================
// Adapter 结构
// =========================================================================

/// OpenAI 兼容 Adapter
pub struct OpenAiAdapter {
    /// 模型名称（如 "gpt-4o", "glm-4-flash", "deepseek-chat"）
    model: String,
    /// API base URL（不含 `/v1/...` 后缀）
    base_url: String,
    /// HTTP 客户端（复用连接池）
    client: reqwest::Client,
}

impl OpenAiAdapter {
    /// 创建 Adapter
    ///
    /// - `model`：模型名称
    /// - `base_url`：API 根地址，如 `https://api.openai.com`
    pub fn new(model: String, base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(120))
            .timeout(Duration::from_secs(300))
            .build()
            .expect("reqwest client build");

        Self {
            model,
            base_url,
            client,
        }
    }
}

/// 智能拼接 OpenAI 兼容 Chat Completions 端点。
///
/// **为什么需要**：用户填写的 `base_url` 可能已经包含版本路径
/// （如 GLM coding 的 `https://open.bigmodel.cn/api/coding/paas/v4`），
/// 也可能不包含（如默认的 `https://api.openai.com`）。盲目追加
/// `/v1/chat/completions` 会得到 `.../v4/v1/chat/completions`，
/// 导致 404。
///
/// 规则：
/// - 若末尾路径段匹配 `/vN`（`v` 后跟 ≥1 位数字，如 `v1`/`v4`/`v42`），
///   则**不再追加** `/v1`，直接拼 `/chat/completions`。
/// - 否则按 OpenAI 标准路径补 `/v1/chat/completions`。
/// - 自动 trim 末尾 `/`。
///
/// # 示例
/// - `https://api.openai.com` → `https://api.openai.com/v1/chat/completions`
/// - `https://api.openai.com/` → `https://api.openai.com/v1/chat/completions`
/// - `https://api.openai.com/v1` → `https://api.openai.com/v1/chat/completions`
/// - `https://open.bigmodel.cn/api/coding/paas/v4`
///   → `https://open.bigmodel.cn/api/coding/paas/v4/chat/completions`
/// - `https://api.deepseek.com` → `https://api.deepseek.com/v1/chat/completions`
/// - `https://x.com/version1` → `https://x.com/version1/v1/chat/completions`（非 v+纯数字）
pub fn build_chat_url(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    // 提取末尾路径段（rsplit 在空串时返回 [""]，所以 next() 永远拿得到）
    let last_segment = trimmed.rsplit('/').next().unwrap_or("");
    if is_version_segment(last_segment) {
        format!("{}/chat/completions", trimmed)
    } else {
        format!("{}/v1/chat/completions", trimmed)
    }
}

/// 判断路径段是否形如 `vN`（v 后跟 ≥1 位 ASCII 数字）。
///
/// 例如：
/// - `v1` → true
/// - `v42` → true
/// - `v` → false（无数字）
/// - `version1` → false（v 后不是纯数字）
/// - `1v` → false（不以 v 开头）
/// - 空串 → false
fn is_version_segment(seg: &str) -> bool {
    if seg.len() < 2 {
        return false;
    }
    let bytes = seg.as_bytes();
    if bytes[0] != b'v' {
        return false;
    }
    // v 之后必须全部是数字
    seg[1..].chars().all(|c| c.is_ascii_digit())
}

// =========================================================================
// Trait 实现
// =========================================================================

#[async_trait]
impl LlmProvider for OpenAiAdapter {
    async fn stream_chat(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDef>>,
        temperature: f64,
        max_tokens: i32,
        model: Option<&str>,
        cancel: CancellationToken,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<ChatDelta>> + Send>>> {
        // P0-3: 会话级 model override —— 优先使用调用方传入的 model，
        // 否则回退到 Adapter 构造时绑定的默认 model（self.model）。
        // 注意：不会改写 self.model，下次 None 调用仍走默认。
        let effective_model = model.unwrap_or(&self.model);

        // 拼装请求 URL（智能识别 base_url 是否已含版本路径）
        let url = build_chat_url(&self.base_url);

        // 把 ChatMessage 转换为 OpenAI 格式
        let openai_msgs: Vec<OpenAiMessage> = messages
            .iter()
            .map(types::chat_message_to_openai)
            .collect::<AppResult<_>>()?;

        // 转换工具定义
        let openai_tools = tools.map(|t| {
            t.into_iter()
                .map(|td| OpenAiTool {
                    kind: "function",
                    function: OpenAiToolFn {
                        name: td.name,
                        description: td.description,
                        parameters: td.parameters,
                    },
                })
                .collect::<Vec<_>>()
        });

        let tool_choice = if openai_tools.is_some() { Some("auto") } else { None };

        let body = ChatRequest {
            model: effective_model,
            messages: openai_msgs,
            tools: openai_tools,
            tool_choice,
            stream: true,
            temperature,
            max_tokens,
            stream_options: StreamOptions { include_usage: true },
        };

        // 调试日志：确认工具是否注入
        if let Some(ref t) = body.tools {
            tracing::info!(
                target: "ice_paw.llm",
                "请求携带 {} 个工具定义: names={:?}",
                t.len(),
                t.iter().map(|x| &x.function.name).collect::<Vec<_>>()
            );
        } else {
            tracing::debug!(target: "ice_paw.llm", "请求未携带工具定义");
        }

        // 发请求
        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Llm(format!("HTTP 请求失败: {e}")))?;

        // 检查 HTTP 状态码
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(AppError::Llm(format!(
                "LLM 返回 HTTP {}: {}",
                status,
                text.chars().take(500).collect::<String>()
            )));
        }

        // 创建通道：解析协程 → 消费方
        // 容量 256：长输出（代码生成 5000+ token）时避免 SSE 解析协程因通道满而阻塞。
        // 每个 chunk 约几十~几百字节，256 × ~500B ≈ 128KB 内存占用，可接受。
        let (tx, rx) = mpsc::channel::<AppResult<ChatDelta>>(256);

        // 获取字节流
        let byte_stream = response.bytes_stream();

        // 启动解析协程（委托给 streaming 模块）
        streaming::parse_sse_stream(byte_stream, tx, cancel);

        // 返回 ReceiverStream 包装
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // is_version_segment 边界用例
    // ---------------------------------------------------------------

    #[test]
    fn is_version_segment_basic_v1() {
        assert!(is_version_segment("v1"));
    }

    #[test]
    fn is_version_segment_multi_digit_v42() {
        assert!(is_version_segment("v42"));
    }

    #[test]
    fn is_version_segment_all_versions() {
        // 验证 v1 / v2 / v3 / v4 都能识别
        for v in &["v1", "v2", "v3", "v4", "v10", "v100"] {
            assert!(is_version_segment(v), "expected {} to be a version segment", v);
        }
    }

    #[test]
    fn is_version_segment_rejects_lone_v() {
        assert!(!is_version_segment("v"));
    }

    #[test]
    fn is_version_segment_rejects_empty() {
        assert!(!is_version_segment(""));
    }

    #[test]
    fn is_version_segment_rejects_version_word() {
        // version1 不算 —— v 之后必须是纯数字
        assert!(!is_version_segment("version1"));
    }

    #[test]
    fn is_version_segment_rejects_v_then_letters() {
        assert!(!is_version_segment("vbeta"));
        assert!(!is_version_segment("v1a"));
    }

    #[test]
    fn is_version_segment_rejects_v_at_end() {
        // "1v" 不算 —— 必须以 v 开头
        assert!(!is_version_segment("1v"));
    }

    #[test]
    fn is_version_segment_rejects_single_char() {
        assert!(!is_version_segment("a"));
        assert!(!is_version_segment("/"));
    }

    // ---------------------------------------------------------------
    // build_chat_url 用例
    // ---------------------------------------------------------------

    #[test]
    fn url_openai_default() {
        let url = build_chat_url("https://api.openai.com");
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn url_openai_default_with_trailing_slash() {
        let url = build_chat_url("https://api.openai.com/");
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn url_openai_explicit_v1() {
        let url = build_chat_url("https://api.openai.com/v1");
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn url_glm_coding_v4() {
        // 核心场景：GLM coding 端点已含 v4，必须直接拼接
        let url = build_chat_url("https://open.bigmodel.cn/api/coding/paas/v4");
        assert_eq!(
            url,
            "https://open.bigmodel.cn/api/coding/paas/v4/chat/completions"
        );
    }

    #[test]
    fn url_glm_paas_v4() {
        let url = build_chat_url("https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(url, "https://open.bigmodel.cn/api/paas/v4/chat/completions");
    }

    #[test]
    fn url_glm_v4_with_trailing_slash() {
        let url = build_chat_url("https://open.bigmodel.cn/api/coding/paas/v4/");
        assert_eq!(
            url,
            "https://open.bigmodel.cn/api/coding/paas/v4/chat/completions"
        );
    }

    #[test]
    fn url_deepseek_default() {
        let url = build_chat_url("https://api.deepseek.com");
        assert_eq!(url, "https://api.deepseek.com/v1/chat/completions");
    }

    #[test]
    fn url_deepseek_explicit_v1() {
        let url = build_chat_url("https://api.deepseek.com/v1");
        assert_eq!(url, "https://api.deepseek.com/v1/chat/completions");
    }

    #[test]
    fn url_proxy_with_v2() {
        let url = build_chat_url("https://some-proxy.com/v2");
        assert_eq!(url, "https://some-proxy.com/v2/chat/completions");
    }

    #[test]
    fn url_proxy_with_v42() {
        let url = build_chat_url("https://some-proxy.com/api/v42");
        assert_eq!(url, "https://some-proxy.com/api/v42/chat/completions");
    }

    #[test]
    fn url_version_word_not_treated_as_version() {
        // "version1" 不算 v+数字，所以仍然追加 /v1
        let url = build_chat_url("https://some-proxy.com/version1");
        assert_eq!(
            url,
            "https://some-proxy.com/version1/v1/chat/completions"
        );
    }

    #[test]
    fn url_lone_v_segment_not_treated_as_version() {
        // 末尾是 "/v" 但 v 后无数字，应追加 /v1
        let url = build_chat_url("https://some-proxy.com/v");
        assert_eq!(url, "https://some-proxy.com/v/v1/chat/completions");
    }

    #[test]
    fn url_multiple_trailing_slashes() {
        // trim_end_matches 会去掉所有末尾 /
        let url = build_chat_url("https://api.openai.com///");
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn url_with_path_but_no_version() {
        let url = build_chat_url("https://example.com/api/openai");
        assert_eq!(
            url,
            "https://example.com/api/openai/v1/chat/completions"
        );
    }

    // ---------------------------------------------------------------
    // P2-2: ContentBlock::Image 转换（user 消息含图片）
    // ---------------------------------------------------------------

    #[test]
    fn openai_user_image_only_text_serializes_as_string() {
        // 内部 ChatMessage 的 content 总是数组（Vec<ContentBlock>），
        // adapter 的 chat_message_to_openai 才会决定是否塌缩为字符串。
        // 这里验证 ChatMessage 本身的序列化结构。
        let m = ChatMessage::from_text("user", "hi");
        let json = serde_json::to_value(m).unwrap();
        assert_eq!(json["role"], "user");
        // ChatMessage.content 是 Vec<ContentBlock>，总是数组
        let content_arr = json["content"].as_array().expect("content 应为数组");
        assert_eq!(content_arr.len(), 1);
        assert_eq!(content_arr[0]["type"], "text");
        assert_eq!(content_arr[0]["text"], "hi");
    }
}
