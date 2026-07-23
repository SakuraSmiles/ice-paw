//! Anthropic Messages API Adapter
//!
//! 适配 Anthropic 协议（Anthropic / MiniMax / 其他 anthropic-messages 兼容厂商）。
//!
//! 核心流程：
//! 1. POST `{base_url}/v1/messages`
//!    headers: `x-api-key`、`anthropic-version: 2023-06-01`、`content-type`、`accept`
//! 2. 把 messages 里的 `role=system` 抽离出来，放到顶层 `system` 字段
//!    （Anthropic 不允许 system 在 messages 里；chat_cmd.rs 把 system 塞进了
//!    messages 第一条，由 adapter 剥离）
//! 3. 按 SSE 协议解析 `event:` + `data:` 双行事件块
//! 4. `content_block_delta` → `ChatDelta::Delta { content }`
//! 5. `message_delta` 记录 `stop_reason`，`message_stop` → `ChatDelta::Done`
//! 6. 每步检查 `cancel.is_cancelled()`，发现取消则提前结束
//!
//! 实现策略：与 `openai.rs` 一致，用 `tokio::sync::mpsc` + `ReceiverStream`
//! 将同步 SSE 解析转换为异步 Stream。
//!
//! 与 OpenAI adapter 的核心差异：
//! - 鉴权 header：`x-api-key`（非 `Authorization: Bearer`）
//! - 版本 header：`anthropic-version: 2023-06-01`
//! - 端点：`/v1/messages`（非 `/v1/chat/completions`）
//! - system prompt：顶层 `system` 字段（messages 里不允许）
//! - SSE：双行格式（`event:` + `data:`），非 OpenAI 的单行 `data:` 格式
//! - 流结束：`event: message_stop`（非 `data: [DONE]`）
//! - 增量字段：`delta.delta.text`（嵌套两层）
//!
//! ## 模块组织（M2.3 拆分）
//!
//! - [`types`] — 请求/响应结构体 + `split_system_prompt()` + `chat_message_to_anthropic_content()`
//! - [`cache`] — `inject_cache_breakpoints()` + `MAX_CACHE_BREAKPOINTS`
//! - [`streaming`] — `parse_sse_stream()` 协程入口

use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::error::{AppError, AppResult};
use crate::harness::chat_state::CancellationToken;
use crate::infra::protocol::{ChatDelta, ChatMessage, LlmProvider, ToolDef};

pub(crate) mod cache;
pub(crate) mod streaming;
pub(crate) mod types;

pub(crate) use types::{AnthropicTool, ApiErrorBody, ChatRequest};

/// 当前稳定的 Anthropic API 版本
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic Messages API Adapter
pub struct AnthropicAdapter {
    /// 模型名称（如 `"claude-3-5-sonnet-20241022"`、`"MiniMax-M2.5"`）
    model: String,
    /// API base URL（不含 `/v1/...` 后缀），如 `https://api.minimaxi.com/anthropic`
    base_url: String,
    /// HTTP 客户端（复用连接池）
    client: reqwest::Client,
    /// P2-3: 是否启用 prompt caching（注入 cache_control 断点）
    cache_prompt: bool,
}

impl AnthropicAdapter {
    /// 创建 Adapter
    ///
    /// - `model`：模型名称
    /// - `base_url`：API 根地址
    /// - `cache_prompt`：P2-3 是否启用 prompt caching（默认 true）
    pub fn new(model: String, base_url: String, cache_prompt: bool) -> Self {
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
            cache_prompt,
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicAdapter {
    async fn stream_chat(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDef>>,
        temperature: f64,
        max_tokens: i32,
        cancel: CancellationToken,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<ChatDelta>> + Send>>> {
        // 1. 拆分 system + 把 ChatMessage 转换为 AnthropicMessage
        let (system_prompt, msgs) = types::split_system_prompt(&messages);

        // P2-3: 构造 system 为 JSON（cache_prompt 启用时使用数组格式）
        let mut system_json: Option<Vec<serde_json::Value>> = system_prompt.map(|s| {
            vec![serde_json::json!({ "type": "text", "text": s })]
        });

        // P2-3: 将 AnthropicMessage 转为 JSON Value（用于注入 cache_control）
        let mut msgs_json: Vec<serde_json::Value> = msgs
            .into_iter()
            .filter_map(|m| serde_json::to_value(m).ok())
            .collect();

        // P2-3: 注入 cache_control 断点（必须在 system_json_ref 快照前执行，
        // 否则注入到 system_json 的 cache_control 不会出现在请求体中）。
        if self.cache_prompt {
            cache::inject_cache_breakpoints(&mut system_json, &mut msgs_json);
        }

        // P0: system_json_ref 必须在 inject_cache_breakpoints 之后创建，
        // 这样 ChatRequest.system 才会包含刚注入的 cache_control 断点。
        let system_json_ref: Option<serde_json::Value> =
            system_json.as_ref().map(|v| serde_json::json!(v));

        // 2. 转 tool 定义
        let anthropic_tools = tools.map(|t| {
            t.into_iter()
                .map(|td| AnthropicTool {
                    name: td.name,
                    description: td.description,
                    input_schema: td.parameters,
                })
                .collect::<Vec<_>>()
        });

        // 3. 拼 URL + body
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let body = ChatRequest {
            model: &self.model,
            max_tokens,
            temperature,
            system: system_json_ref.as_ref(),
            messages: &msgs_json,
            tools: anthropic_tools,
            stream: true,
        };

        // 4. 发请求
        let response = self
            .client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Llm(format!("HTTP 请求失败: {e}")))?;

        // 5. 检查 HTTP 状态码；非成功 → 解析 Anthropic 风格错误体
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            // 尝试解析为 `{ type: "error", error: { type, message } }`
            let detail = serde_json::from_str::<ApiErrorBody>(&text)
                .map(|b| format!("{}: {}", b.error.kind, b.error.message))
                .unwrap_or_else(|_| text.chars().take(500).collect());
            return Err(AppError::Llm(format!("HTTP {status}: {detail}")));
        }

        // 6. SSE 解析（mpsc 模式）— 委托给 streaming::parse_sse_stream
        // 容量 256：长输出（代码生成 5000+ token）时避免 SSE 解析协程因通道满而阻塞。
        // 与 OpenAI adapter 保持一致。
        let (tx, rx) = mpsc::channel::<AppResult<ChatDelta>>(256);
        let byte_stream = response.bytes_stream();
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

    #[test]
    fn url_strips_trailing_slash() {
        // 验证 URL 拼接：trim_end_matches('/') 后再追加 /v1/messages
        let adapter = AnthropicAdapter::new("m".into(), "https://x.com/anthropic/".into(), false);
        let url = format!("{}/v1/messages", adapter.base_url.trim_end_matches('/'));
        assert_eq!(url, "https://x.com/anthropic/v1/messages");
    }

    #[test]
    fn url_no_trailing_slash() {
        let adapter = AnthropicAdapter::new("m".into(), "https://x.com/anthropic".into(), false);
        let url = format!("{}/v1/messages", adapter.base_url.trim_end_matches('/'));
        assert_eq!(url, "https://x.com/anthropic/v1/messages");
    }
}
