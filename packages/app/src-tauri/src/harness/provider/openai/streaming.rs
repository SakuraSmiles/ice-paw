//! OpenAI SSE 流解析协程
//!
//! 从 `mod.rs::stream_chat` 的内联 `tokio::spawn` 闭包提取的独立模块。
//!
//! 核心职责：
//! - 按 `\n` 切分 SSE 行
//! - 逐行解析 `data: ` 前缀，反序列化 JSON
//! - 从 `choices[0].delta.content` 提取文本增量
//! - 从 `choices[0].delta.tool_calls` 提取工具调用增量
//! - 处理 `data: [DONE]` 流结束标志
//! - 处理 streaming usage（choices 为空但 usage 存在）
//! - 通过 `mpsc::Sender<ChatDelta>` 产出统一格式的流式 chunk
//! - 每 chunk 检查 `cancel.is_cancelled()` 实现早退
//!
//! OpenAI SSE 格式（单行 `data:`）与 Anthropic SSE（双行 `event:` + `data:`）的核心差异：
//! - OpenAI 使用**单行**格式（`data: <json>`），Anthropic 是双行格式
//! - OpenAI 流结束标记是 `data: [DONE]`（非 Anthropic 的 `event: message_stop`）
//! - 增量字段在 `choices[0].delta.content`（单层）

use std::collections::HashMap;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::harness::chat_state::CancellationToken;
use crate::infra::protocol::{ChatDelta, TokenUsage};

use super::types::{SseChunk, SseToolCallDelta};

/// SSE 流解析入口（在独立 tokio 任务中跑完整段 HTTP body 的事件分发）
///
/// 与 Anthropic 保持一致：容量 256，长输出时避免 SSE 解析协程因通道满而阻塞。
/// 泛型约束 `S: Stream<Item = Result<Bytes, E>> + Send + Unpin` 与 `bytes_stream()` 的返回类型匹配。
pub(crate) fn parse_sse_stream<S, E>(
    byte_stream: S,
    tx: mpsc::Sender<AppResult<ChatDelta>>,
    cancel: CancellationToken,
) where
    S: Stream<Item = Result<Bytes, E>> + Send + Unpin + 'static,
    E: std::fmt::Display + Send + 'static,
{
    tokio::spawn(async move {
        let mut byte_stream = byte_stream;
        let mut buffer = String::new();
        // 追踪每个工具调用的状态：index → (id, name, arguments_buffer, started)
        let mut tool_call_states: HashMap<usize, (String, String, String, bool)> =
            HashMap::new();

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
                // 未完成的 ToolCallEnd 已由 finish_reason 分支负责发送（若流以 [DONE]
                // 直接收尾而未带 finish_reason，则视为自然结束，发送兜底 Done 即可）。
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
                        // P2-3: 处理 streaming usage（choices 为空但 usage 存在）
                        if parsed.choices.is_empty() {
                            if let Some(usage) = parsed.usage {
                                let _ = tx.send(Ok(ChatDelta::Usage {
                                    usage: TokenUsage {
                                        prompt_tokens: usage.prompt_tokens.unwrap_or(0),
                                        completion_tokens: usage.completion_tokens.unwrap_or(0),
                                        cached_tokens: usage.prompt_tokens_details
                                            .and_then(|d| d.cached_tokens)
                                            .unwrap_or(0),
                                    },
                                })).await;
                            }
                            continue;
                        }

                        if let Some(choice) = parsed.choices.into_iter().next() {
                            // 有 finish_reason 说明这一轮结束
                            if let Some(fr) = choice.finish_reason {
                                // 发送所有已启动工具调用的 End
                                for (id, _, _, started) in tool_call_states.values() {
                                    if *started {
                                        let _ = tx
                                            .send(Ok(ChatDelta::ToolCallEnd {
                                                id: id.clone(),
                                            }))
                                            .await;
                                    }
                                }
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

                            // 思考过程增量（GLM / DeepSeek thinking 模式 SSE 字段）
                            if let Some(rc) = &choice.delta.reasoning_content {
                                if !rc.is_empty() {
                                    if let Err(e) = tx
                                        .send(Ok(ChatDelta::Thinking { content: rc.clone() }))
                                        .await
                                    {
                                        tracing::warn!(
                                            target: "ice_paw.llm",
                                            "send Thinking delta 失败: {}",
                                            e
                                        );
                                        break;
                                    }
                                }
                            }

                            // 工具调用增量
                            if let Some(tc_deltas) = choice.delta.tool_calls {
                                process_tool_call_deltas(
                                    tc_deltas,
                                    &tx,
                                    &mut tool_call_states,
                                )
                                .await;
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
        // 发送未完成的 ToolCallEnd
        for (id, _, _, started) in tool_call_states.values() {
            if *started {
                let _ = tx
                    .send(Ok(ChatDelta::ToolCallEnd { id: id.clone() }))
                    .await;
            }
        }
        let _ = tx
            .send(Ok(ChatDelta::Done {
                finish_reason: Some("stop".into()),
            }))
            .await;
    });
}

/// 处理工具调用增量，追踪状态并发送 ToolCallStart / ToolCallDelta
async fn process_tool_call_deltas(
    tc_deltas: Vec<SseToolCallDelta>,
    tx: &mpsc::Sender<AppResult<ChatDelta>>,
    tool_call_states: &mut HashMap<usize, (String, String, String, bool)>,
) {
    for tc in tc_deltas {
        let entry = tool_call_states
            .entry(tc.index)
            .or_insert_with(|| (String::new(), String::new(), String::new(), false));

        // 首个 chunk 携带 id 和 name
        if let Some(id) = &tc.id {
            entry.0 = id.clone();
        }
        if let Some(func) = &tc.function {
            if let Some(name) = &func.name {
                entry.1 = name.clone();
            }
        }

        // 有 id 和 name 但未启动 → 发送 ToolCallStart
        if !entry.0.is_empty() && !entry.1.is_empty() && !entry.3 {
            entry.3 = true;
            let _ = tx
                .send(Ok(ChatDelta::ToolCallStart {
                    id: entry.0.clone(),
                    name: entry.1.clone(),
                }))
                .await;
        }

        // arguments 片段
        if let Some(func) = &tc.function {
            if let Some(args) = &func.arguments {
                if !args.is_empty() && entry.3 {
                    entry.2.push_str(args);
                    let _ = tx
                        .send(Ok(ChatDelta::ToolCallDelta {
                            id: entry.0.clone(),
                            delta: args.clone(),
                        }))
                        .await;
                }
            }
        }
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::protocol::ChatDelta;
    use bytes::Bytes;
    use futures::stream;
    use tokio::sync::mpsc;

    /// 基础文本流测试（OpenAI 风格）：
    ///
    /// `data: {"choices":[{"delta":{"content":"hi"}}]}` → `ChatDelta::Delta`
    /// `data: [DONE]`                                                → `ChatDelta::Done`
    ///
    /// 故意把两行塞进**同一个** Bytes chunk，让 buffer 按 `\n` 切分循环生效。
    /// 这覆盖了 SSE 行解析 + `[DONE]` 收尾两个核心分支。
    #[tokio::test]
    async fn sse_parse_basic_text_flow_synthetic() {
        // 单 chunk 含两行（最后带 \n），触发 buffer 内 `\n` 切分
        let raw = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\
                   data: [DONE]\n";

        // 错误类型用 std::io::Error（满足 `Display + Send + 'static`）
        let chunks: Vec<Result<Bytes, std::io::Error>> =
            vec![Ok(Bytes::copy_from_slice(raw))];
        let byte_stream = stream::iter(chunks);

        let (tx, mut rx) = mpsc::channel::<AppResult<ChatDelta>>(64);
        let cancel = CancellationToken::new();

        parse_sse_stream(byte_stream, tx, cancel);

        // 任务在发完 Done 后 return → tx drop → rx.recv() 返回 None
        let mut deltas: Vec<String> = Vec::new();
        let mut done_reason: Option<Option<String>> = None;
        while let Some(item) = rx.recv().await {
            match item {
                Ok(ChatDelta::Delta { content }) => deltas.push(content),
                Ok(ChatDelta::Done { finish_reason }) => {
                    done_reason = Some(finish_reason);
                }
                Ok(other) => panic!("不应出现的 ChatDelta 变体: {other:?}"),
                Err(e) => panic!("不应出现错误: {e:?}"),
            }
        }

        assert_eq!(deltas, vec!["hi".to_string()], "应收到 1 个文本增量");
        assert_eq!(
            done_reason,
            Some(Some("stop".to_string())),
            "Done 应携带 finish_reason=stop"
        );
    }

    /// 工具调用增量测试（OpenAI 风格）：
    ///
    /// 跨 chunk 边界模拟完整 tool_call 流：
    /// 1. 第一段 chunk 故意把 JSON 切断在 `arguments` 字段中段
    /// 2. 第二段 chunk 补完 `arguments:""` 并以 `\n` 结束
    ///    → `ChatDelta::ToolCallStart { id, name }`
    /// 3. 第三段 chunk：arguments delta = `{"city":`
    ///    → `ChatDelta::ToolCallDelta`
    /// 4. 第四段 chunk：arguments delta = `"Beijing"}` + finish_reason
    ///    → `ChatDelta::ToolCallDelta` + `ToolCallEnd` + `Done`
    ///
    /// 验证：
    /// - 跨 chunk 累积后行重组正确（buffer 逻辑）
    /// - 完整事件顺序：Start → Delta → Delta → End → Done
    #[tokio::test]
    async fn sse_parse_tool_call_delta_flow_synthetic() {
        // 拆 4 段，前两段衔接处强制跨 chunk 行边界
        let chunks_data: &[&[u8]] = &[
            // chunk 1：在 "argum" 处断开
            b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"function\":{\"name\":\"get_weather\",\"argum",
            // chunk 2：补完 `ents:""}}]}}]}\n` —— 模拟 bytes_stream 半截行
            b"ents\":\"\"}}]}}]}\n",
            // chunk 3：arguments delta = `{"city":`
            b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"city\\\":\"}}]}}]}\n",
            // chunk 4：arguments delta = `"Beijing"}` + finish_reason
            b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"Beijing\\\"}\"}}]}}]}\n",
            b"data: {\"choices\":[{\"finish_reason\":\"tool_calls\",\"delta\":{}}]}\n",
        ];

        let chunks: Vec<Result<Bytes, std::io::Error>> = chunks_data
            .iter()
            .map(|b| Ok(Bytes::copy_from_slice(b)))
            .collect();
        let byte_stream = stream::iter(chunks);

        let (tx, mut rx) = mpsc::channel::<AppResult<ChatDelta>>(64);
        let cancel = CancellationToken::new();

        parse_sse_stream(byte_stream, tx, cancel);

        // 收集所有 delta，任务结束后 rx 返回 None
        let mut events: Vec<ChatDelta> = Vec::new();
        while let Some(item) = rx.recv().await {
            match item {
                Ok(delta) => events.push(delta),
                Err(e) => panic!("不应出现错误: {e:?}"),
            }
        }

        // 期望事件顺序：
        //   Start("call_abc", "get_weather")
        //   Delta("{\"city\":")
        //   Delta("\"Beijing\"}")
        //   End("call_abc")
        //   Done(Some("tool_calls"))
        assert_eq!(
            events.len(),
            5,
            "应有 5 个 delta：Start + Delta×2 + End + Done，实际 {} 个",
            events.len()
        );

        match &events[0] {
            ChatDelta::ToolCallStart { id, name } => {
                assert_eq!(id, "call_abc");
                assert_eq!(name, "get_weather");
            }
            other => panic!("events[0] 应为 ToolCallStart，实际: {other:?}"),
        }
        match &events[1] {
            ChatDelta::ToolCallDelta { id, delta } => {
                assert_eq!(id, "call_abc");
                assert_eq!(delta, "{\"city\":");
            }
            other => panic!("events[1] 应为 ToolCallDelta，实际: {other:?}"),
        }
        match &events[2] {
            ChatDelta::ToolCallDelta { id, delta } => {
                assert_eq!(id, "call_abc");
                assert_eq!(delta, "\"Beijing\"}");
            }
            other => panic!("events[2] 应为 ToolCallDelta，实际: {other:?}"),
        }
        match &events[3] {
            ChatDelta::ToolCallEnd { id } => {
                assert_eq!(id, "call_abc");
            }
            other => panic!("events[3] 应为 ToolCallEnd，实际: {other:?}"),
        }
        match &events[4] {
            ChatDelta::Done { finish_reason } => {
                assert_eq!(finish_reason.as_deref(), Some("tool_calls"));
            }
            other => panic!("events[4] 应为 Done，实际: {other:?}"),
        }
    }

    /// reasoning_content 测试（GLM / DeepSeek thinking 模式）：
    ///
    /// SSE chunk 同时携带 `content` 与 `reasoning_content` 字段：
    /// 1. 第一行：delta 同时含 reasoning_content 与 content
    ///    → 应产出 `ChatDelta::Thinking` 在前 + `ChatDelta::Delta` 在后
    /// 2. 第二行：delta 仅含 reasoning_content（无 content）
    ///    → 应产出 `ChatDelta::Thinking`
    /// 3. 第三行：delta 含空 reasoning_content + content
    ///    → 空 reasoning_content 应被跳过，仅产出 `ChatDelta::Delta`
    /// 4. 第四行：`[DONE]` → `ChatDelta::Done { finish_reason: "stop" }`
    ///
    /// 验证：thinking 与 content 互不混淆，空 reasoning_content 被丢弃。
    #[tokio::test]
    async fn sse_parse_reasoning_content_flow_synthetic() {
        // 注：byte string 字面量必须是 ASCII，故 reasoning_content 用英文标签。
        // 测试聚焦 SSE 字段解析与 ChatDelta::Thinking 转发路径，不依赖 UTF-8。
        let raw = b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking...\",\"content\":\"hi\"}}]}\n\
                   data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"keep thinking\"}}]}\n\
                   data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"\",\"content\":\" world\"}}]}\n\
                   data: [DONE]\n";

        let chunks: Vec<Result<Bytes, std::io::Error>> =
            vec![Ok(Bytes::copy_from_slice(raw))];
        let byte_stream = stream::iter(chunks);

        let (tx, mut rx) = mpsc::channel::<AppResult<ChatDelta>>(64);
        let cancel = CancellationToken::new();

        parse_sse_stream(byte_stream, tx, cancel);

        let mut thinking: Vec<String> = Vec::new();
        let mut deltas: Vec<String> = Vec::new();
        let mut done_reason: Option<Option<String>> = None;
        while let Some(item) = rx.recv().await {
            match item {
                Ok(ChatDelta::Thinking { content }) => thinking.push(content),
                Ok(ChatDelta::Delta { content }) => deltas.push(content),
                Ok(ChatDelta::Done { finish_reason }) => {
                    done_reason = Some(finish_reason);
                }
                Ok(other) => panic!("不应出现的 ChatDelta 变体: {other:?}"),
                Err(e) => panic!("不应出现错误: {e:?}"),
            }
        }

        assert_eq!(
            thinking,
            vec!["thinking...".to_string(), "keep thinking".to_string()],
            "应收到 2 个思考增量（空 reasoning_content 被跳过）"
        );
        assert_eq!(
            deltas,
            vec!["hi".to_string(), " world".to_string()],
            "应收到 2 个文本增量（不含 thinking 内容）"
        );
        assert_eq!(
            done_reason,
            Some(Some("stop".to_string())),
            "Done 应携带 finish_reason=stop"
        );
    }
}
