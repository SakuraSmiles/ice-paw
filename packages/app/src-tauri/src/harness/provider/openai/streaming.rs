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

// =========================================================================
// MiniMax-M3 工具调用协议 sentinel 截断过滤器
// =========================================================================
//
// 背景（vLLM issue #51073）：MiniMax-M3 用一套原生工具调用协议——namespace sentinel
// `]<]minimax[>[`（tokenizer 真实 added token）+ XML 标签（<tool_call>/<invoke>/<filename>/
// <command>/<content>…）。当模型工具调用解析失败（如漏掉首个参数开标签），整套 markup
// 原样泄漏进 `choices[0].delta.content`，且 `finish_reason="stop"`（非 length/max_tokens，
// 故不触发自动续写）。表现为正文末尾出现 `SD]<]minimax[>[</command>]<]minimax[>[</invoke>…`
// 这类乱码。
//
// 策略（治本）：在 transport 层流式截断——一旦发现首个 sentinel，标记 leaked 并永久丢弃
// 后续 content。理由：
//  - 截断而非 strip 标签：tag 名会变（<filename>/<command>/<content>…），枚举必漏变体；
//    且 strip 会把参数值（如文件名 offerta.docx）漏进正文。
//  - sentinel 恒定（`]<]minimax[>[`），是可靠的泄漏边界；sentinel 之后必为工具 markup，
//    绝非用户文本，故截断无损。
//  - 流式而非末端清理：避免生成过程中乱码闪现。
//
// 跨 chunk 安全：sentinel 可能被 chunk 边界切成两半，故每次暂扣末尾 sentinel.len()-1 字节，
// 与下个 chunk 拼接后再判定。仅在模型名含 "minimax" 时启用，其他 OpenAI 兼容模型零影响。

/// MiniMax-M3 工具调用协议的 namespace sentinel（tokenizer 真实 added token）。
const MINIMAX_SENTINEL: &str = "]<]minimax[>[";

/// 是否需要对该模型启用 sentinel 截断（仅 MiniMax-M3 系列已知泄漏）。
pub(super) fn needs_sentinel_scrub(model: &str) -> bool {
    model.to_ascii_lowercase().contains("minimax")
}

/// 把字节索引向下对齐到 UTF-8 字符边界（stable 替代 nightly `str::floor_char_boundary`）。
/// 用于 holdback 切片时避免落在多字节字符中间（会 panic）。
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 流式 sentinel 截断过滤器（见模块注释）。
struct SentinelScrubber {
    enabled: bool,
    pending: String, // 跨 chunk 未决的尾部（可能是 sentinel 前缀）
    leaked: bool,    // 命中 sentinel 后，丢弃所有后续
}

impl SentinelScrubber {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            pending: String::new(),
            leaked: false,
        }
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 喂入一个 content chunk，返回可安全下发的文本。命中 sentinel 后返回空串并永久
    /// 丢弃后续。仅在 `is_enabled()` 时调用。
    fn feed(&mut self, chunk: &str) -> String {
        debug_assert!(self.enabled, "SentinelScrubber::feed 仅在 enabled 时调用");
        if self.leaked {
            return String::new();
        }
        if chunk.is_empty() {
            return String::new();
        }
        let mut combined = std::mem::take(&mut self.pending);
        combined.push_str(chunk);
        if let Some(pos) = combined.find(MINIMAX_SENTINEL) {
            // sentinel 首字节是 ASCII `]`（0x5D），必为 UTF-8 字符边界，[..pos] 安全。
            self.leaked = true;
            self.pending.clear();
            combined[..pos].to_string()
        } else {
            // 末尾 hold_back 字节可能是 sentinel 前缀，暂扣；其余下发。
            let hold_back = MINIMAX_SENTINEL.len() - 1;
            if combined.len() <= hold_back {
                self.pending = combined;
                String::new()
            } else {
                let target = combined.len() - hold_back;
                let split = floor_char_boundary(&combined, target);
                self.pending = combined[split..].to_string();
                combined[..split].to_string()
            }
        }
    }

    /// 流结束：冲刷未决尾部（EOS 的部分前缀就是普通文本，下发）。leaked 时返回空。
    fn flush(&mut self) -> String {
        if self.leaked {
            return String::new();
        }
        std::mem::take(&mut self.pending)
    }
}

/// SSE 流解析入口（在独立 tokio 任务中跑完整段 HTTP body 的事件分发）
///
/// 与 Anthropic 保持一致：容量 256，长输出时避免 SSE 解析协程因通道满而阻塞。
/// 泛型约束 `S: Stream<Item = Result<Bytes, E>> + Send + Unpin` 与 `bytes_stream()` 的返回类型匹配。
pub(crate) fn parse_sse_stream<S, E>(
    byte_stream: S,
    tx: mpsc::Sender<AppResult<ChatDelta>>,
    cancel: CancellationToken,
    model: String,
) where
    S: Stream<Item = Result<Bytes, E>> + Send + Unpin + 'static,
    E: std::fmt::Display + Send + 'static,
{
    tokio::spawn(async move {
        let mut byte_stream = byte_stream;
        // 用 Vec<u8> 缓冲区避免 UTF-8 跨 chunk 边界截断
        let mut buf: Vec<u8> = Vec::new();
        // 追踪每个工具调用的状态：index → (id, name, arguments_buffer, started)
        let mut tool_call_states: HashMap<usize, (String, String, String, bool)> =
            HashMap::new();
        // OpenAI 的 usage chunk 排在 finish_reason 之后，finish_reason 分支只记下原因、
        // 不立即发 Done；真正的 Done 由 [DONE] 分支或流自然结束兜底带此原因发出。
        let mut pending_finish_reason: Option<String> = None;
        // MiniMax-M3 sentinel 截断器（仅模型名含 "minimax" 时启用；其他模型 is_enabled()=false，
        // content 走零开销透传分支）。见上方 SentinelScrubber 注释。
        let mut scrubber = SentinelScrubber::new(needs_sentinel_scrub(&model));

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

            // 把原始字节追加到缓冲区（避免 from_utf8_lossy 在多字节 UTF-8 边界切断）
            buf.extend_from_slice(&chunk);

            // 按字节查找 \n，提取完整行
            while let Some(newline_pos) = buf.iter().position(|&b| b == b'\n') {
                // 提取这行（不含 \n），去除尾部的 \r
                let line_bytes: Vec<u8> = buf[..newline_pos]
                    .iter()
                    .copied()
                    .filter(|&b| b != b'\r')
                    .collect();
                // 从缓冲区移除已处理的行（含 \n）
                buf = buf[newline_pos + 1..].to_vec();

                // 空行跳过（SSE 事件分隔符）
                if line_bytes.is_empty() {
                    continue;
                }

                // 只解码完整的行，确保 UTF-8 不会被截断
                let line = String::from_utf8(line_bytes)
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            target: "ice_paw.llm",
                            "SSE 行 UTF-8 解码失败（容错回退）: {}",
                            e,
                        );
                        // 容错：丢弃无效字节
                        String::from_utf8_lossy(&e.into_bytes()).to_string()
                    });

                // 只处理 `data: ` 开头的行
                let Some(data) = line.strip_prefix("data: ") else {
                    continue;
                };

                // 流结束标志
                // 未完成的 ToolCallEnd 已由 finish_reason 分支负责发送（若流以 [DONE]
                // 直接收尾而未带 finish_reason，则视为自然结束，发送兜底 Done 即可）。
                if data == "[DONE]" {
                    // 流正式结束：先冲刷 sentinel 截断器的未决尾部（holdback，正常内容），再发 Done。
                    let tail = scrubber.flush();
                    if !tail.is_empty() {
                        let _ = tx.send(Ok(ChatDelta::Delta { content: tail })).await;
                    }
                    let _ = tx
                        .send(Ok(ChatDelta::Done {
                            finish_reason: pending_finish_reason
                                .take()
                                .or_else(|| Some("stop".into())),
                        }))
                        .await;
                    return;
                }

                // 解析 JSON
                match serde_json::from_str::<SseChunk>(data) {
                    Ok(mut parsed) => {
                        // P2-3: 处理 streaming usage。OpenAI/deepseek 开 include_usage 后，
                        // usage 出现在流末尾的独立 chunk（choices 为空），排在 finish_reason
                        // 之后；少数兼容端点把 usage 与 finish_reason 放同一 chunk。统一用
                        // take() 在处理 choices 前提取，两种情况都覆盖（否则 usage 丢失 →
                        // token_count=0、max_total_tokens 预算熔断对 OpenAI 路径失效）。
                        if let Some(usage) = parsed.usage.take() {
                            let _ = tx
                                .send(Ok(ChatDelta::Usage {
                                    usage: TokenUsage {
                                        prompt_tokens: usage.prompt_tokens.unwrap_or(0),
                                        completion_tokens: usage.completion_tokens.unwrap_or(0),
                                        cached_tokens: usage.prompt_tokens_details
                                            .and_then(|d| d.cached_tokens)
                                            .unwrap_or(0),
                                    },
                                }))
                                .await;
                        }
                        // choices 为空（纯 usage chunk 或心跳）→ 无内容增量
                        if parsed.choices.is_empty() {
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
                                // 工具调用已全部 End，清空避免流自然结束兜底时重复发送。
                                tool_call_states.clear();
                                // 记下 finish_reason，但不立即发 Done / return：OpenAI 的
                                // usage chunk 紧随 finish_reason，提前 return 会让 usage 永远
                                // 读不到。Done 改由 [DONE] 分支或流结束兜底带此原因发出。
                                pending_finish_reason = Some(fr);
                                continue;
                            }

                            // 正常内容增量
                            if let Some(content) = choice.delta.content {
                                // MiniMax-M3 sentinel 截断（仅 minimax 启用，其他模型零开销透传）：
                                // 命中首个 ]<]minimax[>[ 后永久截断后续（工具调用 markup 泄漏，非用户文本）。
                                if scrubber.is_enabled() {
                                    let emit = scrubber.feed(&content);
                                    if !emit.is_empty() {
                                        let _ = tx
                                            .send(Ok(ChatDelta::Delta { content: emit }))
                                            .await;
                                    }
                                } else if !content.is_empty() {
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
        // 发送未完成的 ToolCallEnd（finish_reason 分支已 clear，通常为空）
        for (id, _, _, started) in tool_call_states.values() {
            if *started {
                let _ = tx
                    .send(Ok(ChatDelta::ToolCallEnd { id: id.clone() }))
                    .await;
            }
        }
        // 冲刷 sentinel 截断器的未决尾部（与 [DONE] 分支对称）
        let tail = scrubber.flush();
        if !tail.is_empty() {
            let _ = tx.send(Ok(ChatDelta::Delta { content: tail })).await;
        }
        let _ = tx
            .send(Ok(ChatDelta::Done {
                finish_reason: pending_finish_reason.or_else(|| Some("stop".into())),
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
    use crate::infra::protocol::{ChatDelta, TokenUsage};
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

        parse_sse_stream(byte_stream, tx, cancel, "gpt-4o".to_string());

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

    /// 回归：usage chunk 在 finish_reason 之后的独立 chunk（OpenAI/deepseek 开
    /// include_usage 的真实顺序）。修复前 finish_reason 分支提前 return，usage chunk
    /// 永远读不到 → token_count=0、max_total_tokens 预算熔断对 OpenAI 路径失效。
    #[tokio::test]
    async fn sse_parse_usage_chunk_after_finish_reason() {
        // 真实顺序：内容 → finish_reason chunk → usage chunk（choices 空）→ [DONE]
        let raw = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\
                   data: {\"choices\":[{\"finish_reason\":\"stop\",\"delta\":{}}]}\n\
                   data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":4}}\n\
                   data: [DONE]\n";

        let chunks: Vec<Result<Bytes, std::io::Error>> =
            vec![Ok(Bytes::copy_from_slice(raw))];
        let byte_stream = stream::iter(chunks);
        let (tx, mut rx) = mpsc::channel::<AppResult<ChatDelta>>(64);
        let cancel = CancellationToken::new();
        parse_sse_stream(byte_stream, tx, cancel, "gpt-4o".to_string());

        let mut deltas: Vec<String> = Vec::new();
        let mut got_usage: Option<TokenUsage> = None;
        let mut done_reason: Option<Option<String>> = None;
        while let Some(item) = rx.recv().await {
            match item {
                Ok(ChatDelta::Delta { content }) => deltas.push(content),
                Ok(ChatDelta::Usage { usage }) => got_usage = Some(usage),
                Ok(ChatDelta::Done { finish_reason }) => done_reason = Some(finish_reason),
                Ok(other) => panic!("不应出现的 ChatDelta 变体: {other:?}"),
                Err(e) => panic!("不应出现错误: {e:?}"),
            }
        }

        assert_eq!(deltas, vec!["hi".to_string()]);
        let usage = got_usage.expect("必须收到 usage（修复前此处为 None）");
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 4);
        assert_eq!(
            done_reason,
            Some(Some("stop".to_string())),
            "Done 仍应携带 finish_reason"
        );
    }

    /// 回归：少数兼容端点把 usage 与 finish_reason 放在同一 chunk（choices 非空）。
    /// take() 在处理 choices 前提取，此情况也覆盖。
    #[tokio::test]
    async fn sse_parse_usage_in_finish_reason_chunk() {
        let raw = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\
                   data: {\"choices\":[{\"finish_reason\":\"length\",\"delta\":{}}],\"usage\":{\"prompt_tokens\":20,\"completion_tokens\":8}}\n\
                   data: [DONE]\n";

        let chunks: Vec<Result<Bytes, std::io::Error>> =
            vec![Ok(Bytes::copy_from_slice(raw))];
        let byte_stream = stream::iter(chunks);
        let (tx, mut rx) = mpsc::channel::<AppResult<ChatDelta>>(64);
        let cancel = CancellationToken::new();
        parse_sse_stream(byte_stream, tx, cancel, "gpt-4o".to_string());

        let mut got_usage: Option<TokenUsage> = None;
        let mut done_reason: Option<Option<String>> = None;
        while let Some(item) = rx.recv().await {
            match item {
                Ok(ChatDelta::Delta { .. }) => {} // 忽略内容增量
                Ok(ChatDelta::Usage { usage }) => got_usage = Some(usage),
                Ok(ChatDelta::Done { finish_reason }) => done_reason = Some(finish_reason),
                Ok(other) => panic!("不应出现的 ChatDelta 变体: {other:?}"),
                Err(e) => panic!("不应出现错误: {e:?}"),
            }
        }

        let usage = got_usage.expect("同 chunk 的 usage 也必须提取");
        assert_eq!(usage.completion_tokens, 8);
        assert_eq!(
            done_reason,
            Some(Some("length".to_string())),
            "Done 应携带 finish_reason=length"
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

        parse_sse_stream(byte_stream, tx, cancel, "gpt-4o".to_string());

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

        parse_sse_stream(byte_stream, tx, cancel, "gpt-4o".to_string());

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

    // ====================================================================
    // SentinelScrubber（MiniMax-M3 sentinel 截断）单元测试
    // ====================================================================

    #[test]
    fn needs_sentinel_scrub_detection() {
        assert!(needs_sentinel_scrub("MiniMax-M3"));
        assert!(needs_sentinel_scrub("minimax-m1"));
        assert!(needs_sentinel_scrub("Something-MiniMax-Chat"));
        assert!(!needs_sentinel_scrub("gpt-4o"));
        assert!(!needs_sentinel_scrub("GLM-5.2"));
        assert!(!needs_sentinel_scrub("deepseek-v4"));
        assert!(!needs_sentinel_scrub(""));
    }

    #[test]
    fn scrub_disabled_not_enabled() {
        let s = SentinelScrubber::new(false);
        assert!(!s.is_enabled());
    }

    #[test]
    fn scrub_passthrough_no_sentinel() {
        let mut s = SentinelScrubber::new(true);
        // 长 content 无 sentinel：feed 因 holdback 暂扣末尾，flush 补齐 → 无损
        let src = "这是一段不含 sentinel 的正常中文输出内容，长度足够超过 holdback。";
        let mut out = String::new();
        out.push_str(&s.feed(src));
        out.push_str(&s.flush());
        assert_eq!(out, src);
    }

    #[test]
    fn scrub_truncate_at_first_sentinel_single_chunk() {
        let mut s = SentinelScrubber::new(true);
        // sentinel 在中段：前缀 "读 SD" 保留，sentinel 及之后全截断（用户实测 case）
        let emit = s.feed("读 SD]<]minimax[>[</command>]<]minimax[>[</invoke>");
        assert_eq!(emit, "读 SD");
        // 已 leaked，后续 feed 与 flush 全丢
        assert_eq!(s.feed("更多文本"), "");
        assert_eq!(s.flush(), "");
    }

    #[test]
    fn scrub_sentinel_split_across_chunks() {
        let mut s = SentinelScrubber::new(true);
        // 把 sentinel 切在 "minim" / "ax[>[" 之间 → 验证 holdback 跨 chunk 重组
        let mut out = String::new();
        out.push_str(&s.feed("读 SD]<]minim"));
        out.push_str(&s.feed("ax[>[</command>]<]minimax[>[</invoke>"));
        out.push_str(&s.flush());
        assert_eq!(out, "读 SD");
    }

    #[test]
    fn scrub_leaked_drops_subsequent() {
        let mut s = SentinelScrubber::new(true);
        assert_eq!(s.feed("ok]<]minimax[>[junk"), "ok");
        assert_eq!(s.feed("still junk"), "");
        assert_eq!(s.feed("more"), "");
        assert_eq!(s.flush(), "");
    }

    #[test]
    fn scrub_sentinel_at_start_yields_empty() {
        let mut s = SentinelScrubber::new(true);
        // 整条消息就是一个失败的工具调用（sentinel 在最前）→ 截断为空
        let emit = s.feed("]<]minimax[>[<tool_call>\n]<]minimax[>[<invoke name=\"x\">");
        assert_eq!(emit, "");
        assert_eq!(s.flush(), "");
    }

    /// 逐字符喂入（CJK 多字节 + 极小 chunk）——验证 floor_char_boundary 不 panic、
    /// 且无 sentinel 时无损重组。
    #[test]
    fn scrub_feed_single_chars_no_sentinel() {
        let mut s = SentinelScrubber::new(true);
        let src = "读SD列表内容";
        let mut out = String::new();
        for ch in src.chars() {
            out.push_str(&s.feed(&ch.to_string()));
        }
        out.push_str(&s.flush());
        assert_eq!(out, src);
    }

    /// 逐字符喂入含 sentinel 的串——验证极小 chunk 下仍能正确截断在 sentinel 边界。
    #[test]
    fn scrub_feed_single_chars_with_sentinel() {
        let mut s = SentinelScrubber::new(true);
        let mut out = String::new();
        for ch in "读SD]<]minimax[>[</command>".chars() {
            out.push_str(&s.feed(&ch.to_string()));
        }
        out.push_str(&s.flush());
        assert_eq!(out, "读SD");
    }

    /// 端到端：MiniMax-M3 模型 + content 含 sentinel → 经 parse_sse_stream 截断，
    /// 前端只收到 sentinel 之前的干净文本，finish_reason 不变。
    #[tokio::test]
    async fn sse_parse_minimax_sentinel_truncated() {
        let content = "读 SD]<]minimax[>[</command>]<]minimax[>[</invoke>";
        let raw = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}\n\
             data: [DONE]\n",
            serde_json::to_string(content).unwrap()
        );
        let chunks: Vec<Result<Bytes, std::io::Error>> =
            vec![Ok(Bytes::copy_from_slice(raw.as_bytes()))];
        let byte_stream = stream::iter(chunks);
        let (tx, mut rx) = mpsc::channel::<AppResult<ChatDelta>>(64);
        let cancel = CancellationToken::new();
        parse_sse_stream(byte_stream, tx, cancel, "MiniMax-M3".to_string());

        let mut deltas: Vec<String> = Vec::new();
        let mut done_reason: Option<Option<String>> = None;
        while let Some(item) = rx.recv().await {
            match item {
                Ok(ChatDelta::Delta { content }) => deltas.push(content),
                Ok(ChatDelta::Done { finish_reason }) => done_reason = Some(finish_reason),
                Ok(other) => panic!("不应出现的 ChatDelta 变体: {other:?}"),
                Err(e) => panic!("不应出现错误: {e:?}"),
            }
        }
        assert_eq!(deltas.concat(), "读 SD", "sentinel 之后的 markup 应被截断");
        assert_eq!(
            done_reason,
            Some(Some("stop".to_string())),
            "截断不改 finish_reason"
        );
    }

    /// 端到端：非 minimax 模型 + 同样的乱码 content → scrubber 不启用，原样透传
    /// （回归保护：gate 不能误伤其他 provider）。
    #[tokio::test]
    async fn sse_parse_non_minimax_no_scrub_passthrough() {
        let content = "读 SD]<]minimax[>[</command>";
        let raw = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}\n\
             data: [DONE]\n",
            serde_json::to_string(content).unwrap()
        );
        let chunks: Vec<Result<Bytes, std::io::Error>> =
            vec![Ok(Bytes::copy_from_slice(raw.as_bytes()))];
        let byte_stream = stream::iter(chunks);
        let (tx, mut rx) = mpsc::channel::<AppResult<ChatDelta>>(64);
        let cancel = CancellationToken::new();
        parse_sse_stream(byte_stream, tx, cancel, "gpt-4o".to_string());

        let mut deltas: Vec<String> = Vec::new();
        while let Some(item) = rx.recv().await {
            if let Ok(ChatDelta::Delta { content }) = item {
                deltas.push(content);
            }
        }
        assert_eq!(deltas.concat(), content, "非 minimax 模型应原样透传，不截断");
    }
}
