//! Anthropic SSE 流解析协程
//!
//! 从 `mod.rs::stream_chat` 的内联 `tokio::spawn` 闭包提取的独立模块。
//!
//! 核心职责：
//! - 按 `\n\n` 切分 SSE 事件块
//! - 逐行解析 `event:` / `data:` 前缀
//! - 分发到不同事件类型（message_start / content_block_* / message_delta / message_stop / error）
//! - 通过 `mpsc::Sender<ChatDelta>` 产出统一格式的流式 chunk
//! - 每 chunk 检查 `cancel.is_cancelled()` 实现早退
//!
//! Anthropic SSE 与 OpenAI SSE 的核心差异：
//! - Anthropic 使用**双行**格式（`event: <type>\ndata: <json>`），OpenAI 是单行 `data:` 格式
//! - Anthropic 流结束标记是 `event: message_stop`（非 OpenAI 的 `data: [DONE]`）
//! - 增量字段在 `delta.delta.text`（嵌套两层）

use std::collections::HashMap;

use bytes::Bytes;
use futures::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::harness::chat_state::CancellationToken;
use crate::infra::protocol::{ChatDelta, TokenUsage};

/// SSE 流解析入口（在独立 tokio 任务中跑完整段 HTTP body 的事件分发）
///
/// 与 OpenAI 保持一致：容量 256，长输出时避免 SSE 解析协程因通道满而阻塞。
pub(crate) fn parse_sse_stream<S>(byte_stream: S, tx: mpsc::Sender<AppResult<ChatDelta>>, cancel: CancellationToken)
where
    S: futures::Stream<Item = Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
{
    tokio::spawn(async move {
        let mut byte_stream = byte_stream;
        // 用 Vec<u8> 缓冲区避免 UTF-8 跨 chunk 边界截断
        let mut buf: Vec<u8> = Vec::new();
        let mut last_stop_reason: Option<String> = None;
        // 追踪 content block：index → (block_type, id?, name?)
        // 用于在 content_block_start 时记录，content_block_delta 时查询
        let mut block_info: HashMap<i64, (String, Option<String>, Option<String>)> =
            HashMap::new();

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

            // 追加原始字节到缓冲区（避免 UTF-8 跨 chunk 边界截断）
            buf.extend_from_slice(&chunk);

            // SSE 事件块以 \n\n 分隔，在字节缓冲区中查找
            while let Some(pos) = buf.windows(2).position(|w| w == b"\n\n") {
                // 提取事件块字节并解码（完整块，不会截断 UTF-8）
                let event_bytes: Vec<u8> = buf[..pos].to_vec();
                let event_block = String::from_utf8(event_bytes)
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            target: "ice_paw.llm",
                            "Anthropic SSE 事件块 UTF-8 解码失败（容错）: {}",
                            e,
                        );
                        String::from_utf8_lossy(&e.into_bytes()).to_string()
                    });
                // 从缓冲区移除已处理的事件块（含 \n\n）
                buf = buf[pos + 2..].to_vec();

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
                        if let Ok(p) = serde_json::from_str::<MessageStartPayload>(&data_buf) {
                            if let Some(usage) = p.message.usage {
                                let _ = tx
                                    .send(Ok(ChatDelta::Usage {
                                        usage: TokenUsage {
                                            prompt_tokens: usage.input_tokens.unwrap_or(0),
                                            completion_tokens: usage.output_tokens.unwrap_or(0),
                                            cached_tokens: usage.cache_read_input_tokens.unwrap_or(0),
                                        },
                                    }))
                                    .await;
                            }
                        }
                    }
                    "content_block_start" => {
                        // 记录 block 信息（type: text / tool_use / thinking）
                        if let Ok(p) = serde_json::from_str::<BlockStartPayload>(&data_buf) {
                            let id_clone = p.content_block.id.clone();
                            let name_clone = p.content_block.name.clone();
                            block_info.insert(
                                p.index,
                                (
                                    p.content_block.kind.clone(),
                                    id_clone.clone(),
                                    name_clone.clone(),
                                ),
                            );

                            // 如果是 tool_use，发送 ToolCallStart
                            if p.content_block.kind == "tool_use" {
                                if let (Some(id), Some(name)) = (&id_clone, &name_clone) {
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
                    "content_block_delta" => match serde_json::from_str::<DeltaPayload>(&data_buf)
                    {
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
                                            if let Some((_, Some(id), _)) = block_info.get(&p.index)
                                            {
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
                    },
                    "content_block_stop" => {
                        // 检查是否是 tool_use block 结束
                        if let Ok(p) = serde_json::from_str::<BlockStopPayload>(&data_buf) {
                            if let Some((block_type, Some(id), _)) = block_info.get(&p.index) {
                                if block_type == "tool_use" {
                                    let _ = tx
                                        .send(Ok(ChatDelta::ToolCallEnd { id: id.clone() }))
                                        .await;
                                }
                            }
                        }
                    }
                    "message_delta" => {
                        if let Ok(p) = serde_json::from_str::<MessageDeltaPayload>(&data_buf) {
                            if let Some(sr) = p.delta.stop_reason {
                                last_stop_reason = Some(sr);
                            }
                            // P2-3: 发出最终 token usage，供前端展示 cached_tokens。
                            // 如果上游未带 usage 字段（比如只发送了 stop_reason）则跳过。
                            if let Some(usage) = p.usage {
                                let _ = tx
                                    .send(Ok(ChatDelta::Usage {
                                        usage: TokenUsage {
                                            prompt_tokens: usage.input_tokens.unwrap_or(0),
                                            completion_tokens: usage.output_tokens.unwrap_or(0),
                                            cached_tokens: usage.cache_read_input_tokens.unwrap_or(0),
                                        },
                                    }))
                                    .await;
                            }
                        }
                    }
                    "message_stop" => {
                        let fr = last_stop_reason.take().or_else(|| Some("stop".into()));
                        let _ = tx
                            .send(Ok(ChatDelta::Done { finish_reason: fr }))
                            .await;
                        return;
                    }
                    "error" => {
                        let msg = serde_json::from_str::<StreamErrorPayload>(&data_buf)
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
}

// =========================================================================
// SSE 事件 payload 结构（从内联到模块级，避免每次 spawn 时重复定义）
// =========================================================================

/// `message_start` 事件 payload
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

/// `content_block_start` 事件 payload
#[derive(Deserialize)]
struct BlockStartPayload {
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

/// `content_block_delta` 事件 payload
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
    /// `input_json_delta` 的部分 JSON
    #[serde(default)]
    partial_json: Option<String>,
    /// `thinking_delta` 的内容
    #[serde(default)]
    thinking: Option<String>,
}

/// `content_block_stop` 事件 payload
#[derive(Deserialize)]
struct BlockStopPayload {
    index: i64,
}

/// `message_delta` 事件 payload
#[derive(Deserialize)]
struct MessageDeltaPayload {
    delta: MessageDeltaBody,
    // P2-3: Anthropic 在 message_delta 中提供最终 cache_read_input_tokens / output_tokens。
    //   message_delta.usage 不含 input_tokens（仅在 message_start 提供），这里用 default 兼容。
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

/// `error` 事件 payload
#[derive(Deserialize)]
struct StreamErrorPayload {
    error: StreamErrorBody,
}

#[derive(Deserialize)]
struct StreamErrorBody {
    #[serde(default)]
    message: String,
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use crate::infra::protocol::ChatDelta;
    use tokio::sync::mpsc;

    use super::*;

    /// 模拟一段完整 SSE 流（来自 Anthropic）：
    /// message_start → content_block_start → ping → 3 个 delta → content_block_stop
    /// → message_delta → message_stop
    ///
    /// 调用真实 `parse_sse_stream`（而非手写简化版解析器），验证完整 SSE → ChatDelta
    /// 的端到端转换正确性，避免"虚假绿灯"。
    #[tokio::test]
    async fn sse_parse_full_flow_synthetic() {
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

        let bytes = Bytes::from(raw);
        let stream = futures::stream::iter(vec![Ok::<Bytes, reqwest::Error>(bytes)]);

        let (tx, mut rx) = mpsc::channel::<AppResult<ChatDelta>>(64);
        let cancel = CancellationToken::new();

        // 调用真实 parse_sse_stream（内部 spawn，通过 mpsc 产出结果）
        parse_sse_stream(stream, tx, cancel.clone());

        // 收集结果
        let mut deltas: Vec<String> = Vec::new();
        let mut done: Option<Option<String>> = None;
        while let Some(item) = rx.recv().await {
            match item {
                Ok(ChatDelta::Delta { content }) => deltas.push(content),
                Ok(ChatDelta::Done { finish_reason }) => {
                    done = Some(finish_reason);
                    break;
                }
                Ok(_) => { /* text_delta 以外的 delta 类型忽略 */ }
                Err(_) => panic!("不应出现错误"),
            }
        }

        assert_eq!(deltas, vec!["你", "好", "！"]);
        assert_eq!(done, Some(Some("end_turn".into())));
        assert!(!cancel.is_cancelled());
    }
}
