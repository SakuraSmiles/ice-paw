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
        // 拼装请求 URL（智能识别 base_url 是否已含版本路径）
        let url = build_chat_url(&self.base_url);

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
        // 容量 256：长输出（代码生成 5000+ token）时避免 SSE 解析协程因通道满而阻塞。
        // 每个 chunk 约几十~几百字节，256 × ~500B ≈ 128KB 内存占用，可接受。
        let (tx, rx) = mpsc::channel::<AppResult<ChatDelta>>(256);

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
}
