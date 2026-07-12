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

// =========================================================================
// 请求 / 响应 结构
// =========================================================================

/// 请求体（发给 LLM 的 JSON）
#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    temperature: f64,
    max_tokens: i32,
    stream_options: StreamOptions,
}

/// `stream_options.include_usage = true` 让 API 返回 token 用量
#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

/// SSE 单行 JSON 的最小化反序列化结构
#[derive(Deserialize)]
struct SseChunk {
    choices: Vec<SseChoice>,
}

#[derive(Deserialize)]
struct SseChoice {
    delta: SseDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
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
        temperature: f64,
        max_tokens: i32,
        cancel: CancellationToken,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<ChatDelta>> + Send>>> {
        // 拼装请求 URL
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));

        let body = ChatRequest {
            model: self.model.clone(),
            messages,
            stream: true,
            temperature,
            max_tokens,
            stream_options: StreamOptions { include_usage: true },
        };

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
        let (tx, rx) = mpsc::channel::<AppResult<ChatDelta>>(64);

        // 获取字节流
        let byte_stream = response.bytes_stream();

        // 启动解析协程
        tokio::spawn(async move {
            let mut byte_stream = byte_stream;
            let mut buffer = String::new();

            use futures::StreamExt;

            while let Some(chunk_result) = byte_stream.next().await {
                // 取消检查
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

                // 追加到缓冲区并按行处理
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                    // 剩余保留
                    buffer = buffer[newline_pos + 1..].to_string();

                    // 空行跳过（SSE 事件分隔符）
                    if line.is_empty() {
                        continue;
                    }

                    // 只处理 `data: ` 开头的行
                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };

                    // 流结束标志
                    if data == "[DONE]" {
                        let _ = tx
                            .send(Ok(ChatDelta::Done {
                                finish_reason: Some("stop".into()),
                            }))
                            .await;
                        return;
                    }

                    // 解析 JSON
                    match serde_json::from_str::<SseChunk>(data) {
                        Ok(parsed) => {
                            if let Some(choice) = parsed.choices.into_iter().next() {
                                // 有 finish_reason 说明这一轮结束
                                if let Some(fr) = choice.finish_reason {
                                    let _ = tx
                                        .send(Ok(ChatDelta::Done {
                                            finish_reason: Some(fr),
                                        }))
                                        .await;
                                    return;
                                }

                                // 正常内容增量
                                if let Some(content) = choice.delta.content {
                                    if !content.is_empty() {
                                        let _ = tx
                                            .send(Ok(ChatDelta::Delta { content }))
                                            .await;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            // SSE 行 JSON 解析失败 — 记录但不中断（容错）
                            tracing::warn!(
                                target: "ice_paw.llm",
                                "SSE JSON 解析失败（跳过）: {e} | line={}",
                                &data[..data.len().min(200)]
                            );
                        }
                    }
                }
            }

            // 字节流自然结束（未收到 [DONE]）— 优雅收尾
            let _ = tx
                .send(Ok(ChatDelta::Done {
                    finish_reason: Some("stop".into()),
                }))
                .await;
        });

        // 返回 ReceiverStream 包装
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
