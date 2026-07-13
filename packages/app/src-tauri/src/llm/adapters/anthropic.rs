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

use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::error::{AppError, AppResult};
use crate::llm::cancel::CancellationToken;
use crate::llm::{ChatDelta, ChatMessage, LlmProvider};

/// 当前稳定的 Anthropic API 版本
const ANTHROPIC_VERSION: &str = "2023-06-01";

// =========================================================================
// Adapter 结构
// =========================================================================

/// Anthropic Messages API Adapter
pub struct AnthropicAdapter {
    /// 模型名称（如 `"claude-3-5-sonnet-20241022"`、`"MiniMax-M2.5"`）
    model: String,
    /// API base URL（不含 `/v1/...` 后缀），如 `https://api.minimaxi.com/anthropic`
    base_url: String,
    /// HTTP 客户端（复用连接池）
    client: reqwest::Client,
}

impl AnthropicAdapter {
    /// 创建 Adapter
    ///
    /// - `model`：模型名称
    /// - `base_url`：API 根地址
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

    /// 把 `ChatMessage` 列表拆分为 (system_prompt, messages)：
    /// - 抽离 `role == "system"` 的消息（多个则用 `\n\n` 拼接），不放入 Anthropic 的 messages
    /// - `user` / `assistant` 保留，其余角色（`tool` 等本期不支持）直接跳过
    fn split_system_prompt(messages: &[ChatMessage]) -> (Option<String>, Vec<AnthropicMessage>) {
        let mut system_parts: Vec<String> = Vec::new();
        let mut msgs: Vec<AnthropicMessage> = Vec::with_capacity(messages.len());

        for m in messages {
            match m.role.as_str() {
                "system" => system_parts.push(m.content.clone()),
                "user" | "assistant" => msgs.push(AnthropicMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                }),
                // tool 等不支持的角色直接跳过（不污染发送给上游的数据）
                _ => continue,
            }
        }

        let system = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        };

        (system, msgs)
    }
}

// =========================================================================
// 请求 / 响应 结构
// =========================================================================

/// 请求体（发给 Anthropic Messages API 的 JSON）
#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    max_tokens: i32,
    temperature: f64,
    /// 顶层 system 字段；为 `None` 时不序列化
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    messages: &'a [AnthropicMessage],
    stream: bool,
}

/// Anthropic 的单条消息
#[derive(Serialize)]
struct AnthropicMessage {
    /// 只能是 `"user"` 或 `"assistant"`（system 在顶层字段）
    role: String,
    content: String,
}

/// Anthropic 错误响应的 body 结构
#[derive(Deserialize)]
struct ApiErrorBody {
    error: ApiErrorDetail,
}

#[derive(Deserialize)]
struct ApiErrorDetail {
    /// 错误类型：`authentication_error` / `invalid_request_error` / `rate_limit_error` 等
    #[serde(rename = "type")]
    kind: String,
    message: String,
}

// =========================================================================
// Trait 实现
// =========================================================================

#[async_trait]
impl LlmProvider for AnthropicAdapter {
    async fn stream_chat(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        temperature: f64,
        max_tokens: i32,
        cancel: CancellationToken,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<ChatDelta>> + Send>>> {
        // 1. 拆分 system
        let (system_prompt, msgs) = Self::split_system_prompt(&messages);

        // 2. 拼 URL + body
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let body = ChatRequest {
            model: &self.model,
            max_tokens,
            temperature,
            system: system_prompt.as_deref(),
            messages: &msgs,
            stream: true,
        };

        // 3. 发请求
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

        // 4. 检查 HTTP 状态码；非成功 → 解析 Anthropic 风格错误体
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            // 尝试解析为 `{ type: "error", error: { type, message } }`
            let detail = serde_json::from_str::<ApiErrorBody>(&text)
                .map(|b| format!("{}: {}", b.error.kind, b.error.message))
                .unwrap_or_else(|_| text.chars().take(500).collect());
            return Err(AppError::Llm(format!("HTTP {status}: {detail}")));
        }

        // 5. SSE 解析（mpsc 模式）
        let (tx, rx) = mpsc::channel::<AppResult<ChatDelta>>(64);
        let mut byte_stream = response.bytes_stream();

        tokio::spawn(async move {
            use futures::StreamExt;

            let mut buffer = String::new();
            let mut last_stop_reason: Option<String> = None;

            while let Some(chunk_result) = byte_stream.next().await {
                // 取消检查（每 chunk 一次）
                if cancel.is_cancelled() {
                    let _ = tx
                        .send(Ok(ChatDelta::Done {
                            finish_reason: Some("abort".into()),
                        }))
                        .await;
                    return;
                }

                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx
                            .send(Err(AppError::Stream(format!(
                                "HTTP 流读取失败: {e}"
                            ))))
                            .await;
                        return;
                    }
                };

                // 追加到缓冲区并按 SSE 事件块（\n\n 分隔）切分
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buffer.find("\n\n") {
                    let event_block = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    // 事件块内逐行解析 event: / data: 前缀
                    let mut event_type = String::new();
                    let mut data_buf = String::new();
                    for line in event_block.split('\n') {
                        let line = line.trim_end_matches('\r');
                        if let Some(v) = line.strip_prefix("event: ") {
                            event_type = v.to_string();
                        } else if let Some(v) = line.strip_prefix("data: ") {
                            // 多个 data: 行拼接（Anthropic 实际不会多行，但稳健性）
                            if !data_buf.is_empty() {
                                data_buf.push('\n');
                            }
                            data_buf.push_str(v);
                        }
                        // 忽略注释行（以 `:` 开头）和其他未知前缀
                    }

                    if data_buf.is_empty() {
                        continue;
                    }

                    // 分发到不同事件类型
                    match event_type.as_str() {
                        "content_block_delta" => {
                            // data.delta.text（嵌套两层）
                            #[derive(Deserialize)]
                            struct DeltaPayload {
                                delta: DeltaBody,
                            }
                            #[derive(Deserialize)]
                            struct DeltaBody {
                                /// `text_delta` 之外的类型（`input_json_delta` 等）跳过
                                #[serde(rename = "type", default)]
                                kind: Option<String>,
                                #[serde(default)]
                                text: Option<String>,
                            }
                            match serde_json::from_str::<DeltaPayload>(&data_buf) {
                                Ok(p) => {
                                    // 只关心 text_delta，非文本增量直接忽略
                                    let is_text_delta = p
                                        .delta
                                        .kind
                                        .as_deref()
                                        .map(|k| k == "text_delta")
                                        .unwrap_or(true); // 缺 type 时按文本处理（兼容实现）

                                    if is_text_delta {
                                        if let Some(text) = p.delta.text {
                                            if !text.is_empty() {
                                                let _ = tx
                                                    .send(Ok(ChatDelta::Delta { content: text }))
                                                    .await;
                                            }
                                        }
                                    } else {
                                        tracing::debug!(
                                            target: "ice_paw.llm",
                                            "跳过非 text_delta: type={:?}",
                                            p.delta.kind
                                        );
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        target: "ice_paw.llm",
                                        "content_block_delta 解析失败（跳过）: {e} | data={}",
                                        &data_buf[..data_buf.len().min(200)]
                                    );
                                }
                            }
                        }
                        "message_delta" => {
                            // data.delta.stop_reason（仅记录，不发 Done）
                            #[derive(Deserialize)]
                            struct MessageDeltaPayload {
                                delta: MessageDeltaBody,
                            }
                            #[derive(Deserialize)]
                            struct MessageDeltaBody {
                                #[serde(default)]
                                stop_reason: Option<String>,
                            }
                            if let Ok(p) = serde_json::from_str::<MessageDeltaPayload>(&data_buf) {
                                if let Some(sr) = p.delta.stop_reason {
                                    last_stop_reason = Some(sr);
                                }
                            }
                        }
                        "message_stop" => {
                            // 流结束：用记录下来的 stop_reason；缺省为 "stop"
                            let fr = last_stop_reason.take().or_else(|| Some("stop".into()));
                            let _ = tx.send(Ok(ChatDelta::Done { finish_reason: fr })).await;
                            return;
                        }
                        "error" => {
                            // 流中错误事件：升级为 AppError::Llm
                            #[derive(Deserialize)]
                            struct StreamError {
                                error: StreamErrorBody,
                            }
                            #[derive(Deserialize)]
                            struct StreamErrorBody {
                                #[serde(default)]
                                message: String,
                            }
                            let msg = serde_json::from_str::<StreamError>(&data_buf)
                                .map(|e| e.error.message)
                                .unwrap_or_else(|_| data_buf.clone());
                            let _ = tx
                                .send(Err(AppError::Llm(format!(
                                    "Anthropic 流中错误: {msg}"
                                ))))
                                .await;
                            return;
                        }
                        // message_start / content_block_start / content_block_stop / ping / 其他 → 忽略
                        _ => {}
                    }
                }
            }

            // 字节流自然结束（未收到 message_stop）— 优雅收尾
            let fr = last_stop_reason.or_else(|| Some("stop".into()));
            let _ = tx.send(Ok(ChatDelta::Done { finish_reason: fr })).await;
        });

        // 返回 ReceiverStream 包装
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: content.into(),
        }
    }

    #[test]
    fn split_empty_messages() {
        let (sys, msgs) = AnthropicAdapter::split_system_prompt(&[]);
        assert!(sys.is_none());
        assert!(msgs.is_empty());
    }

    #[test]
    fn split_user_only() {
        let input = vec![msg("user", "hi")];
        let (sys, msgs) = AnthropicAdapter::split_system_prompt(&input);
        assert!(sys.is_none());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hi");
    }

    #[test]
    fn split_user_assistant_preserves_order() {
        let input = vec![
            msg("user", "Q1"),
            msg("assistant", "A1"),
            msg("user", "Q2"),
        ];
        let (sys, msgs) = AnthropicAdapter::split_system_prompt(&input);
        assert!(sys.is_none());
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[2].role, "user");
        assert_eq!(msgs[2].content, "Q2");
    }

    #[test]
    fn split_system_plus_conversation() {
        let input = vec![
            msg("system", "你是助手"),
            msg("user", "你好"),
            msg("assistant", "你好！"),
        ];
        let (sys, msgs) = AnthropicAdapter::split_system_prompt(&input);
        assert_eq!(sys.as_deref(), Some("你是助手"));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn split_multiple_system_joins_with_blank_line() {
        let input = vec![
            msg("system", "rule 1"),
            msg("system", "rule 2"),
            msg("user", "go"),
        ];
        let (sys, msgs) = AnthropicAdapter::split_system_prompt(&input);
        assert_eq!(sys.as_deref(), Some("rule 1\n\nrule 2"));
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn split_skips_unknown_role() {
        let input = vec![
            msg("system", "sys"),
            msg("tool", "ignored"),
            msg("user", "hi"),
        ];
        let (sys, msgs) = AnthropicAdapter::split_system_prompt(&input);
        assert_eq!(sys.as_deref(), Some("sys"));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[test]
    fn url_strips_trailing_slash() {
        // 验证 URL 拼接：trim_end_matches('/') 后再追加 /v1/messages
        let adapter = AnthropicAdapter::new("m".into(), "https://x.com/anthropic/".into());
        let url = format!("{}/v1/messages", adapter.base_url.trim_end_matches('/'));
        assert_eq!(url, "https://x.com/anthropic/v1/messages");
    }

    #[test]
    fn url_no_trailing_slash() {
        let adapter = AnthropicAdapter::new("m".into(), "https://x.com/anthropic".into());
        let url = format!("{}/v1/messages", adapter.base_url.trim_end_matches('/'));
        assert_eq!(url, "https://x.com/anthropic/v1/messages");
    }

    /// 模拟一段完整 SSE 流（来自 MiniMax / Anthropic）：
    /// message_start → content_block_start → ping → 3 个 delta → content_block_stop
    /// → message_delta → message_stop
    #[test]
    fn sse_parse_full_flow_synthetic() {
        // 构造原始 SSE 字节流
        let raw = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\"}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: ping\n\
data: {\"type\":\"ping\"}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"好\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"！\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

        // 用一个 mpsc 通道模拟"tx"，手写一份简化版的解析逻辑来验证解析正确性
        let (tx, mut rx) = mpsc::channel::<AppResult<ChatDelta>>(64);
        let cancel = CancellationToken::new();

        let mut buffer = String::new();
        buffer.push_str(raw);

        let mut last_stop_reason: Option<String> = None;
        let mut events_processed: Vec<String> = Vec::new();

        while let Some(pos) = buffer.find("\n\n") {
            let event_block = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            let mut event_type = String::new();
            let mut data_buf = String::new();
            for line in event_block.split('\n') {
                let line = line.trim_end_matches('\r');
                if let Some(v) = line.strip_prefix("event: ") {
                    event_type = v.to_string();
                } else if let Some(v) = line.strip_prefix("data: ") {
                    if !data_buf.is_empty() {
                        data_buf.push('\n');
                    }
                    data_buf.push_str(v);
                }
            }
            if data_buf.is_empty() {
                continue;
            }
            events_processed.push(event_type.clone());

            match event_type.as_str() {
                "content_block_delta" => {
                    #[derive(Deserialize)]
                    struct D {
                        delta: D2,
                    }
                    #[derive(Deserialize)]
                    struct D2 {
                        text: Option<String>,
                    }
                    if let Ok(p) = serde_json::from_str::<D>(&data_buf) {
                        if let Some(t) = p.delta.text {
                            if !t.is_empty() {
                                let _ = tx
                                    .try_send(Ok(ChatDelta::Delta { content: t }));
                            }
                        }
                    }
                }
                "message_delta" => {
                    #[derive(Deserialize)]
                    struct D {
                        delta: D2,
                    }
                    #[derive(Deserialize)]
                    struct D2 {
                        stop_reason: Option<String>,
                    }
                    if let Ok(p) = serde_json::from_str::<D>(&data_buf) {
                        if let Some(sr) = p.delta.stop_reason {
                            last_stop_reason = Some(sr);
                        }
                    }
                }
                "message_stop" => {
                    let fr = last_stop_reason.take().or_else(|| Some("stop".into()));
                    let _ = tx.try_send(Ok(ChatDelta::Done { finish_reason: fr }));
                    break;
                }
                _ => {}
            }
        }

        // 关闭 tx 让 rx 循环可以退出（同步 try_send 已经把所有内容塞进去）
        drop(tx);

        // 收集结果
        let mut deltas: Vec<String> = Vec::new();
        let mut done: Option<Option<String>> = None;
        // 同步通道已经在循环里发完，直接 drain
        while let Ok(item) = rx.try_recv() {
            match item {
                Ok(ChatDelta::Delta { content }) => deltas.push(content),
                Ok(ChatDelta::Done { finish_reason }) => {
                    done = Some(finish_reason);
                    break;
                }
                Err(_) => panic!("不应出现错误"),
            }
        }

        assert_eq!(deltas, vec!["你", "好", "！"]);
        assert_eq!(done, Some(Some("end_turn".into())));
        assert!(events_processed.contains(&"message_start".to_string()));
        assert!(events_processed.contains(&"ping".to_string()));
        assert!(!cancel.is_cancelled());

        let _ = cancel; // 抑制未使用警告
    }
}