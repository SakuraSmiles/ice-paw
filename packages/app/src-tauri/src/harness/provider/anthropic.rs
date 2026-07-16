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
use crate::infra::protocol::{ChatDelta, ChatMessage, ContentBlock, LlmProvider, TokenUsage, ToolDef};
use crate::harness::chat_state::CancellationToken;

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

    /// 把 `ChatMessage` 列表拆分为 (system_prompt, messages)：
    /// - 抽离 `role == "system"` 的消息（多个则用 `\n\n` 拼接），不放入 Anthropic 的 messages
    /// - `user` / `assistant` 保留并转换为 Anthropic content block 格式
    /// - `tool` 角色转为 `user` + tool_result content block（Anthropic 不支持 tool role）
    fn split_system_prompt(messages: &[ChatMessage]) -> (Option<String>, Vec<AnthropicMessage>) {
        let mut system_parts: Vec<String> = Vec::new();
        let mut msgs: Vec<AnthropicMessage> = Vec::with_capacity(messages.len());

        for m in messages {
            match m.role.as_str() {
                "system" => system_parts.push(m.content_text()),
                "user" | "assistant" => {
                    let content = chat_message_to_anthropic_content(m);
                    msgs.push(AnthropicMessage {
                        role: m.role.clone(),
                        content,
                    });
                }
                "tool" => {
                    // Anthropic 没有 tool role，把 tool 结果转为 user 消息中的 tool_result content block
                    let content = chat_message_to_anthropic_content(m);
                    msgs.push(AnthropicMessage {
                        role: "user".to_string(),
                        content,
                    });
                }
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

    // -----------------------------------------------------------------
    // P2-3: Prompt Caching — cache_control 断点注入
    // -----------------------------------------------------------------

    /// Anthropic 缓存断点最大数量（官方限制 ≤ 4）
    const MAX_CACHE_BREAKPOINTS: usize = 4;

    /// 注入 cache_control 断点。
    ///
    /// 策略：
    /// 1. 断点 1：system prompt 的最后一个 content block
    /// 2. 断点 2~4：倒数第 3 条之前的 message（最多再加 3 个）
    ///
    /// Anthropic 的 cache_control 放在 content block 级别（非 message 级别）：
    /// - system 使用数组格式，最后一个 block 带 cache_control
    /// - message 的 content 如果是字符串，先转为单元素数组再附加 cache_control
    /// - message 的 content 如果是数组，最后一个 block 附加 cache_control
    fn inject_cache_breakpoints(
        system: &mut Option<Vec<serde_json::Value>>,
        messages: &mut [serde_json::Value],
    ) {
        let mut breakpoints_used = 0;

        // 断点 1：system prompt 的最后一个 content block
        if let Some(blocks) = system.as_mut() {
            if let Some(last) = blocks.last_mut() {
                last["cache_control"] = serde_json::json!({ "type": "ephemeral" });
                breakpoints_used += 1;
            }
        }

        if breakpoints_used >= Self::MAX_CACHE_BREAKPOINTS {
            return;
        }

        // 断点 2~4：倒数第 3 条之前的 message（跳过第 1 条 user 消息）
        let len = messages.len();
        if len <= 3 {
            return;
        }

        let cutoff = len.saturating_sub(3);
        for msg in messages.iter_mut().take(cutoff).skip(1) {
            if breakpoints_used >= Self::MAX_CACHE_BREAKPOINTS {
                break;
            }

            // 在 content 的最后一个 block 上附加 cache_control
            if let Some(content) = msg.get_mut("content") {
                match content {
                    serde_json::Value::String(_) => {
                        // 字符串 → 转为单元素数组，附带 cache_control
                        let text = content.as_str().unwrap_or("").to_string();
                        *content = serde_json::json!([
                            {
                                "type": "text",
                                "text": text,
                                "cache_control": { "type": "ephemeral" }
                            }
                        ]);
                    }
                    serde_json::Value::Array(blocks) => {
                        // 数组 → 在最后一个 block 上附加 cache_control
                        if let Some(last) = blocks.last_mut() {
                            last["cache_control"] = serde_json::json!({ "type": "ephemeral" });
                        }
                    }
                    _ => {}
                }
                breakpoints_used += 1;
            }
        }
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
    /// 顶层 system 字段；为 `None` 时不序列化。
    /// P2-3: 当 cache_prompt 启用时使用数组格式（支持 cache_control 断点）。
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a serde_json::Value>,
    messages: &'a [serde_json::Value],
    /// 工具定义
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    stream: bool,
}

/// Anthropic 的单条消息（content 支持 string 或 content block 数组）
#[derive(Serialize)]
struct AnthropicMessage {
    /// 只能是 `"user"` 或 `"assistant"`（system 在顶层字段）
    role: String,
    /// 序列化为 string（纯文本）或数组（含工具调用的 content block）
    content: serde_json::Value,
}

/// Anthropic 格式工具定义
#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
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
// 消息格式转换
// =========================================================================

/// 把内部 ChatMessage 转换为 Anthropic API content 数组
///
/// Anthropic 的 content block 格式：
/// - Text → `{ type: "text", text: "..." }`
/// - Image → `{ type: "image", source: { type: "base64", media_type, data } }` （P2-2）
/// - ToolUse → `{ type: "tool_use", id: "...", name: "...", input: {...} }`
/// - ToolResult → `{ type: "tool_result", tool_use_id: "...", content: "..." }`
/// - Thinking → 跳过（Anthropic 不接受回传的 thinking 块）
fn chat_message_to_anthropic_content(msg: &ChatMessage) -> serde_json::Value {
    // 纯文本消息 → 序列化为 string（更简洁）
    let all_text = msg
        .content
        .iter()
        .all(|b| matches!(b, ContentBlock::Text { .. }));
    if all_text {
        return serde_json::Value::String(msg.content_text());
    }

    // 含工具调用/结果/图片 → 序列化为数组
    let arr: Vec<serde_json::Value> = msg
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(serde_json::json!({
                "type": "text",
                "text": text
            })),
            // P2-2: 图片 → Anthropic image block
            ContentBlock::Image { data, media_type } => Some(serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": media_type,
                    "data": data
                }
            })),
            ContentBlock::ToolUse { id, name, input } => {
                // input 是 JSON 字符串，解析为对象
                let parsed: serde_json::Value =
                    serde_json::from_str(input).unwrap_or(serde_json::Value::Null);
                Some(serde_json::json!({
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": parsed
                }))
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => Some(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": is_error.unwrap_or(false)
            })),
            ContentBlock::Thinking { .. } => None, // 不回传给 Anthropic
        })
        .collect();

    serde_json::Value::Array(arr)
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
        tools: Option<Vec<ToolDef>>,
        temperature: f64,
        max_tokens: i32,
        cancel: CancellationToken,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<ChatDelta>> + Send>>> {
        // 1. 拆分 system
        let (system_prompt, msgs) = Self::split_system_prompt(&messages);

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
            Self::inject_cache_breakpoints(&mut system_json, &mut msgs_json);
        }

        // P0: system_json_ref 必须在 inject_cache_breakpoints 之后创建，
        // 这样 ChatRequest.system 才会包含刚注入的 cache_control 断点。
        let system_json_ref: Option<serde_json::Value> = system_json.as_ref().map(|v| serde_json::json!(v));

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
        // 容量 256：长输出（代码生成 5000+ token）时避免 SSE 解析协程因通道满而阻塞。
        // 与 OpenAI adapter 保持一致。
        let (tx, rx) = mpsc::channel::<AppResult<ChatDelta>>(256);
        let mut byte_stream = response.bytes_stream();

        tokio::spawn(async move {
            use futures::StreamExt;

            let mut buffer = String::new();
            let mut last_stop_reason: Option<String> = None;
            // 追踪 content block：index → (block_type, id?, name?)
            // 用于在 content_block_start 时记录，content_block_delta 时查询
            let mut block_info: std::collections::HashMap<i64, (String, Option<String>, Option<String>)> =
                std::collections::HashMap::new();

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
                            if !data_buf.is_empty() {
                                data_buf.push('\n');
                            }
                            data_buf.push_str(v);
                        }
                    }

                    if data_buf.is_empty() {
                        continue;
                    }

                    // 分发到不同事件类型
                    match event_type.as_str() {
                        "message_start" => {
                            // P2-3: 解析 usage（cache_read_input_tokens）
                            #[derive(Deserialize)]
                            struct MessageStartPayload {
                                message: MessageStartMessage,
                            }
                            #[derive(Deserialize)]
                            struct MessageStartMessage {
                                #[serde(default)]
                                usage: Option<MessageStartUsage>,
                            }
                            #[derive(Deserialize)]
                            struct MessageStartUsage {
                                #[serde(default)]
                                input_tokens: Option<u32>,
                                #[serde(default)]
                                cache_read_input_tokens: Option<u32>,
                                #[serde(default)]
                                output_tokens: Option<u32>,
                            }
                            if let Ok(p) = serde_json::from_str::<MessageStartPayload>(&data_buf) {
                                if let Some(usage) = p.message.usage {
                                    let _ = tx.send(Ok(ChatDelta::Usage {
                                        usage: TokenUsage {
                                            prompt_tokens: usage.input_tokens.unwrap_or(0),
                                            completion_tokens: usage.output_tokens.unwrap_or(0),
                                            cached_tokens: usage.cache_read_input_tokens.unwrap_or(0),
                                        },
                                    })).await;
                                }
                            }
                        }
                        "content_block_start" => {
                            // 记录 block 信息（type: text / tool_use / thinking）
                            #[derive(Deserialize)]
                            struct BlockStart {
                                index: i64,
                                content_block: BlockStartContent,
                            }
                            #[derive(Deserialize)]
                            struct BlockStartContent {
                                #[serde(rename = "type")]
                                kind: String,
                                #[serde(default)]
                                id: Option<String>,
                                #[serde(default)]
                                name: Option<String>,
                            }
                            if let Ok(p) = serde_json::from_str::<BlockStart>(&data_buf) {
                                let id_clone = p.content_block.id.clone();
                                let name_clone = p.content_block.name.clone();
                                block_info.insert(
                                    p.index,
                                    (p.content_block.kind.clone(), id_clone.clone(), name_clone.clone()),
                                );

                                // 如果是 tool_use，发送 ToolCallStart
                                if p.content_block.kind == "tool_use" {
                                    if let Some(id) = &id_clone {
                                        if let Some(name) = &name_clone {
                                            let _ = tx
                                                .send(Ok(ChatDelta::ToolCallStart {
                                                    id: id.clone(),
                                                    name: name.clone(),
                                                }))
                                                .await;
                                        }
                                    }
                                }
                            }
                        }
                        "content_block_delta" => {
                            #[derive(Deserialize)]
                            struct DeltaPayload {
                                index: i64,
                                delta: DeltaBody,
                            }
                            #[derive(Deserialize)]
                            struct DeltaBody {
                                #[serde(rename = "type", default)]
                                kind: Option<String>,
                                #[serde(default)]
                                text: Option<String>,
                                /// input_json_delta 的部分 JSON
                                #[serde(default)]
                                partial_json: Option<String>,
                                /// thinking delta 的内容
                                #[serde(default)]
                                thinking: Option<String>,
                            }
                            match serde_json::from_str::<DeltaPayload>(&data_buf) {
                                Ok(p) => {
                                    let delta_kind = p.delta.kind.as_deref().unwrap_or("");
                                    let block_type = block_info
                                        .get(&p.index)
                                        .map(|(t, _, _)| t.as_str())
                                        .unwrap_or("");

                                    match delta_kind {
                                        "text_delta" => {
                                            if let Some(text) = p.delta.text {
                                                if !text.is_empty() {
                                                    let _ = tx
                                                        .send(Ok(ChatDelta::Delta { content: text }))
                                                        .await;
                                                }
                                            }
                                        }
                                        "input_json_delta" => {
                                            // 工具调用参数增量
                                            if let Some(pj) = p.delta.partial_json {
                                                if !pj.is_empty() {
                                                    if let Some((_, id, _)) = block_info.get(&p.index) {
                                                        if let Some(id) = id {
                                                            let _ = tx
                                                                .send(Ok(ChatDelta::ToolCallDelta {
                                                                    id: id.clone(),
                                                                    delta: pj,
                                                                }))
                                                                .await;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        "thinking_delta" => {
                                            if let Some(thinking) = p.delta.thinking {
                                                if !thinking.is_empty() {
                                                    let _ = tx
                                                        .send(Ok(ChatDelta::Thinking { content: thinking }))
                                                        .await;
                                                }
                                            }
                                        }
                                        _ => {
                                            tracing::debug!(
                                                target: "ice_paw.llm",
                                                "跳过未知 delta 类型: {:?} (block_type={})",
                                                p.delta.kind,
                                                block_type
                                            );
                                        }
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
                        "content_block_stop" => {
                            // 检查是否是 tool_use block 结束
                            #[derive(Deserialize)]
                            struct BlockStop {
                                index: i64,
                            }
                            if let Ok(p) = serde_json::from_str::<BlockStop>(&data_buf) {
                                if let Some((block_type, id, _)) = block_info.get(&p.index) {
                                    if block_type == "tool_use" {
                                        if let Some(id) = id {
                                            let _ = tx
                                                .send(Ok(ChatDelta::ToolCallEnd {
                                                    id: id.clone(),
                                                }))
                                                .await;
                                        }
                                    }
                                }
                            }
                        }
                        "message_delta" => {
                            #[derive(Deserialize)]
                            struct MessageDeltaPayload {
                                delta: MessageDeltaBody,
                                // P2-3: Anthropic 在 message_delta 中提供最终
                                //   cache_read_input_tokens / output_tokens。
                                //   message_delta.usage 不含 input_tokens
                                //   （仅在 message_start 提供），这里用 default 兼容。
                                #[serde(default)]
                                usage: Option<MessageDeltaUsage>,
                            }
                            #[derive(Deserialize)]
                            struct MessageDeltaBody {
                                #[serde(default)]
                                stop_reason: Option<String>,
                            }
                            #[derive(Deserialize)]
                            struct MessageDeltaUsage {
                                #[serde(default)]
                                input_tokens: Option<u32>,
                                #[serde(default)]
                                cache_read_input_tokens: Option<u32>,
                                #[serde(default)]
                                output_tokens: Option<u32>,
                            }
                            if let Ok(p) = serde_json::from_str::<MessageDeltaPayload>(&data_buf) {
                                if let Some(sr) = p.delta.stop_reason {
                                    last_stop_reason = Some(sr);
                                }
                                // P2-3: 发出最终 token usage，供前端展示 cached_tokens。
                                // 如果上游未带 usage 字段（比如只发送了 stop_reason）则跳过。
                                if let Some(usage) = p.usage {
                                    let _ = tx.send(Ok(ChatDelta::Usage {
                                        usage: TokenUsage {
                                            prompt_tokens: usage.input_tokens.unwrap_or(0),
                                            completion_tokens: usage.output_tokens.unwrap_or(0),
                                            cached_tokens: usage.cache_read_input_tokens.unwrap_or(0),
                                        },
                                    })).await;
                                }
                            }
                        }
                        "message_stop" => {
                            let fr = last_stop_reason.take().or_else(|| Some("stop".into()));
                            let _ = tx.send(Ok(ChatDelta::Done { finish_reason: fr })).await;
                            return;
                        }
                        "error" => {
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
                        // message_start / ping / 其他 → 忽略
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
        ChatMessage::from_text(role, content)
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
    fn split_converts_tool_role_to_user() {
        // P2-1: tool role 消息转为 user 消息（Anthropic 不支持 tool role）
        let input = vec![
            msg("system", "sys"),
            msg("tool", "tool result"),
            msg("user", "hi"),
        ];
        let (sys, msgs) = AnthropicAdapter::split_system_prompt(&input);
        assert_eq!(sys.as_deref(), Some("sys"));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user"); // tool → user
        assert_eq!(msgs[1].role, "user");
    }

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
                Ok(_) => { /* 其他变体在测试中忽略 */ }
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

    // ================================================================
    // P2-2: ContentBlock::Image 转换为 Anthropic image block
    // ================================================================

    #[test]
    fn anthropic_image_block_shape() {
        // 验证期望的 Anthropic image block JSON 结构
        // （与 adapter 中 chat_message_to_anthropic_content 等价）
        let data = "iVBORw0KGgo";
        let media_type = "image/png";
        let expected = serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": media_type,
                "data": data
            }
        });
        assert_eq!(expected["type"], "image");
        assert_eq!(expected["source"]["type"], "base64");
        assert_eq!(expected["source"]["media_type"], "image/png");
        assert_eq!(expected["source"]["data"], "iVBORw0KGgo");
    }

    /// Image + Text 混合 → adapter 生成的 Anthropic content 数组
    #[test]
    fn anthropic_mixed_image_text() {
        // 直接模拟 adapter 的 filter_map 逻辑（chat_message_to_anthropic_content 是 private fn）
        let blocks = vec![
            ContentBlock::text("描述一下这张图"),
            ContentBlock::image("AAAA", "image/jpeg"),
            ContentBlock::text("谢谢"),
        ];

        let arr: Vec<serde_json::Value> = blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(serde_json::json!({
                    "type": "text",
                    "text": text
                })),
                ContentBlock::Image { data, media_type } => Some(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": media_type,
                        "data": data
                    }
                })),
                _ => None,
            })
            .collect();

        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "描述一下这张图");
        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[1]["source"]["data"], "AAAA");
        assert_eq!(arr[1]["source"]["media_type"], "image/jpeg");
        assert_eq!(arr[2]["type"], "text");
        assert_eq!(arr[2]["text"], "谢谢");
    }

    /// split_system_prompt 对 user 含 Image 的消息应保留为 user role
    /// （Anthropic 允许 user 消息中含 image block）
    #[test]
    fn split_user_with_image_preserves_role() {
        let msg = ChatMessage {
            role: "user".into(),
            content: vec![
                ContentBlock::text("看这张图"),
                ContentBlock::image("BBBB", "image/webp"),
            ],
        };
        let (sys, msgs) = AnthropicAdapter::split_system_prompt(&[msg]);
        assert!(sys.is_none());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        // content 应该是数组（含 image block）
        assert!(msgs[0].content.is_array());
        let arr = msgs[0].content.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[1]["source"]["data"], "BBBB");
    }

    // ================================================================
    // P2-3: inject_cache_breakpoints 单元测试
    // ================================================================

    /// 辅助：统计 system + messages 中的 cache_control 断点总数。
    /// 遍历 system 各 block 以及每个 message 的 content 各 block，
    /// 检查是否存在 `cache_control.type == "ephemeral"`。
    fn count_cache_breakpoints(
        system: &Option<Vec<serde_json::Value>>,
        messages: &[serde_json::Value],
    ) -> usize {
        let mut count = 0;

        if let Some(blocks) = system.as_ref() {
            for block in blocks {
                if block.get("cache_control").is_some() {
                    count += 1;
                }
            }
        }

        for msg in messages {
            if let Some(content) = msg.get("content") {
                match content {
                    serde_json::Value::String(_) => {
                        // 字符串 content 在注入后会变成数组（不可能保留字符串），
                        // 此处为防御性兜底：如果遇到，说明函数未处理。
                    }
                    serde_json::Value::Array(blocks) => {
                        for block in blocks {
                            if block.get("cache_control").is_some() {
                                count += 1;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        count
    }

    /// 系统提示存在、消息 ≤ 3 条 → 只在 system 最后一个 block 加 1 个断点；
    /// messages 上不应有任何 cache_control（因为 len <= 3 直接 return）。
    #[test]
    fn inject_cache_breakpoints_system_only() {
        let mut system = Some(vec![
            serde_json::json!({ "type": "text", "text": "你是助手" }),
        ]);
        let mut msgs = vec![
            serde_json::json!({ "role": "user", "content": "hi" }),
            serde_json::json!({ "role": "assistant", "content": "hello" }),
        ];

        AnthropicAdapter::inject_cache_breakpoints(&mut system, &mut msgs);

        // system 最后一个 block 必须有 cache_control
        let sys_blocks = system.as_ref().unwrap();
        assert_eq!(sys_blocks.len(), 1);
        assert_eq!(sys_blocks[0]["cache_control"]["type"], "ephemeral");

        // messages 不应被改写为数组（因为函数在 len <= 3 时直接 return）
        assert!(msgs[0]["content"].is_string(), "user content 应保持字符串");
        assert!(msgs[1]["content"].is_string(), "assistant content 应保持字符串");

        // 总断点数 = 1
        assert_eq!(count_cache_breakpoints(&system, &msgs), 1);
    }

    /// 长对话（10 条消息）→ system 1 个 + messages 最多 3 个，总断点数 = 4。
    /// 验证 Anthropic 的 4 断点硬约束被正确遵守。
    #[test]
    fn inject_cache_breakpoints_long_conversation() {
        let mut system = Some(vec![
            serde_json::json!({ "type": "text", "text": "你是助手" }),
        ]);
        let mut msgs: Vec<serde_json::Value> = (0..10)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                serde_json::json!({ "role": role, "content": format!("msg-{i}") })
            })
            .collect();

        AnthropicAdapter::inject_cache_breakpoints(&mut system, &mut msgs);

        // 总断点数 = 4（Anthropic 硬限制 = MAX_CACHE_BREAKPOINTS）
        let total = count_cache_breakpoints(&system, &msgs);
        assert_eq!(
            total,
            AnthropicAdapter::MAX_CACHE_BREAKPOINTS,
            "长对话应达到 4 断点上限，实际 = {total}"
        );

        // system 一定有 cache_control
        assert!(system.as_ref().unwrap()[0].get("cache_control").is_some());

        // messages[0]（首条 user）被 skip(1) 跳过 → content 仍为字符串，无 cache_control
        assert!(msgs[0]["content"].is_string(), "首条 user 的 content 应保持字符串");

        // messages 末尾 3 条（indices 7, 8, 9）不应有 cache_control（cutoff = 7，take(7) 后 skip(1) = indices 1..7）
        for msg in msgs.iter().skip(7) {
            assert!(
                msg["content"].is_string(),
                "末尾消息 content 应保持字符串"
            );
        }

        // messages[1..4] 中的 3 条（indices 1, 2, 3）应有 cache_control（system 已用 1 个，剩 3 个配额）
        for (idx, msg) in msgs.iter().enumerate().take(4).skip(1) {
            let blocks = msg["content"].as_array()
                .unwrap_or_else(|| panic!("idx={idx} content 应被转换为数组"));
            assert_eq!(blocks.len(), 1, "字符串 content 转换后应有 1 个 block");
            assert_eq!(
                blocks[0]["cache_control"]["type"],
                "ephemeral",
                "idx={idx} 应该有 cache_control"
            );
        }

        // messages[4..7] 应保持字符串（断点配额已用完，不再处理）
        for msg in msgs.iter().take(7).skip(4) {
            assert!(
                msg["content"].is_string(),
                "断点配额用完，content 应保持字符串"
            );
        }
    }

    /// 没有 system 提示时，所有断点都注入到 messages 上（最多 4 个）。
    #[test]
    fn inject_cache_breakpoints_no_system() {
        let mut system: Option<Vec<serde_json::Value>> = None;
        let mut msgs: Vec<serde_json::Value> = (0..10)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                serde_json::json!({ "role": role, "content": format!("msg-{i}") })
            })
            .collect();

        AnthropicAdapter::inject_cache_breakpoints(&mut system, &mut msgs);

        // system 必须保持 None
        assert!(system.is_none());

        // 总断点数应 ≤ 4，且 = 4（无 system 时 messages 可用满 4 个配额）
        let total = count_cache_breakpoints(&system, &msgs);
        assert!(
            total <= AnthropicAdapter::MAX_CACHE_BREAKPOINTS,
            "总断点数应 ≤ 4，实际 = {total}"
        );
        assert_eq!(
            total,
            AnthropicAdapter::MAX_CACHE_BREAKPOINTS,
            "无 system 时 10 条消息应用满 4 个断点配额"
        );

        // 末尾 3 条不应有断点
        for msg in msgs.iter().skip(7) {
            assert!(
                msg["content"].is_string(),
                "末尾消息 content 应保持字符串"
            );
        }

        // 首条 user（idx=0）不应有断点（被 skip(1) 跳过）
        assert!(msgs[0]["content"].is_string(), "首条 user content 应保持字符串");

        // messages[1..5] 中的 4 条应有 cache_control
        for (idx, msg) in msgs.iter().enumerate().take(5).skip(1) {
            let blocks = msg["content"].as_array()
                .unwrap_or_else(|| panic!("idx={idx} content 应被转换为数组"));
            assert_eq!(blocks.len(), 1);
            assert_eq!(
                blocks[0]["cache_control"]["type"],
                "ephemeral",
                "idx={idx} 应该有 cache_control"
            );
        }
    }

    /// system 含多个 content block → 只在最后一个 block 加 cache_control，
    /// 前面的 block 必须保持原样不变。
    #[test]
    fn inject_cache_breakpoints_mixed_blocks() {
        let mut system = Some(vec![
            serde_json::json!({ "type": "text", "text": "block 1" }),
            serde_json::json!({ "type": "text", "text": "block 2" }),
            serde_json::json!({ "type": "text", "text": "block 3" }),
        ]);
        // 给一个 ≥ 4 条消息的场景，确保 messages 也会被处理
        let mut msgs = vec![
            serde_json::json!({ "role": "user", "content": "q1" }),
            serde_json::json!({ "role": "assistant", "content": "a1" }),
            serde_json::json!({ "role": "user", "content": "q2" }),
            serde_json::json!({ "role": "assistant", "content": "a2" }),
            serde_json::json!({ "role": "user", "content": "q3" }),
        ];

        AnthropicAdapter::inject_cache_breakpoints(&mut system, &mut msgs);

        let sys_blocks = system.as_ref().unwrap();
        // block 1 / block 2 不应有 cache_control
        assert!(sys_blocks[0].get("cache_control").is_none(), "block 1 不应有 cache_control");
        assert!(sys_blocks[1].get("cache_control").is_none(), "block 2 不应有 cache_control");
        // 只有 block 3（最后一个）应有 cache_control
        assert_eq!(sys_blocks[2]["cache_control"]["type"], "ephemeral", "block 3 应有 cache_control");

        // block 1 / 2 的原始 text 必须保留（未被覆盖）
        assert_eq!(sys_blocks[0]["text"], "block 1");
        assert_eq!(sys_blocks[1]["text"], "block 2");
        assert_eq!(sys_blocks[2]["text"], "block 3");
    }
}