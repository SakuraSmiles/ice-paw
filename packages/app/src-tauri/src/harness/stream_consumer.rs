//! L2 Stream Consumer — 流式 Delta 消费逻辑（W3.3）
//!
//! 职责：从 `provider.stream_chat()` 返回的 SSE 流中逐项消费 `ChatDelta`，
//! 收集文本、思考内容、工具调用，最终返回 `StreamResult`。
//!
//! emit 实时事件（`chat:chunk` / `chat:thinking` / `chat:tool-call-*`）
//! 在消费过程中 emit，保持流式渲染体验。
//!
//! 错误透传：可重试/不可重试错误均由调用方 `loop_engine` 在外层统一处理。

use std::collections::HashMap;
use std::pin::Pin;
use std::time::{Duration, Instant};

use crate::error::AppError;
use crate::harness::chat_state::CancellationToken;
use crate::harness::observable::RoundState;
use crate::harness::r#loop::emitter::{emit_ser, LoopEmitter};
use crate::infra::protocol::{
    ChatChunkPayload, ChatDelta, ChatThinkingPayload, ChatToolCallDeltaPayload,
    ChatToolCallEndPayload, ChatToolCallStartPayload, TokenUsage,
};
use futures::Stream;

/// 单轮流式消费结果
#[derive(Debug, Clone)]
pub struct StreamResult {
    pub text: String,
    pub think: String,
    pub finish_reason: String,
    pub tool_calls: HashMap<String, CollectedToolCall>,
    pub usage: Option<TokenUsage>,
}

/// 一轮流式消费中收集到的工具调用信息
#[derive(Debug, Clone)]
pub struct CollectedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub ended: bool,
}

// ============================================================================
// DeltaAggregator — token 级进度事件的窗口聚合器
// ============================================================================
//
// 为什么聚合：Windows/WebView2 上后端 emit 的 JS 注入走主线程，逐 delta emit
// 会打满主线程（同步命令 / IPC 分发全体排队）并触发前端全列表重渲染——低配
// 机器放大为生成中全局卡顿。详见 `loop/emitter.rs` 模块注释「事件节奏约定」。
//
// 窗口语义（check-then-append）：push 到达 → 若距上次 flush ≥ 40ms 先发旧缓冲
// → 再 append 新 delta。flush 出去的恒为「上一窗口完整内容」。
//
// 不引定时器（被动 flush）：SSE 流持续有 delta 时每个 push 都是一次过窗机会；
// 流静默时残余最坏滞留 40ms 到下一 delta 或退出点，不丢数据——为消除这点滞留
// 引定时器的复杂度（任务生命周期、与退出竞争）不值。
//
// 【退出纪律】consume_stream 的每一条提前返回路径（cancel / Done / Err / 自然
// 结束）必须先 flush——新增退出路径漏 flush = 前端丢最后 40ms 文本。

/// 聚合窗口（ms）。40ms ≈ 每通道每秒至多 25 次 emit（高于流畅帧率）。
/// 不要调小：窗口越小越接近「逐 delta emit」的旧问题（主线程事件注入洪泛）。
const EMIT_WINDOW_MS: u64 = 40;

pub(crate) struct DeltaAggregator<'a> {
    emitter: &'a dyn LoopEmitter,
    conv_id: &'a str,
    asst_msg_id: &'a str,
    /// `chat:chunk` 待发文本（窗口内拼接）
    pending_text: String,
    /// `chat:thinking` 待发内容
    pending_think: String,
    /// `chat:tool-call-delta` 按 id 分组：(id, 拼接 delta)，保持首达序。
    /// Vec 线性查找即可——一个窗口内同时活跃的 tool id 通常 1~3，天然保序。
    pending_tool_deltas: Vec<(String, String)>,
    /// 窗口锚点：上次 flush 时刻
    last_flush: Instant,
}

impl<'a> DeltaAggregator<'a> {
    /// `now` 由调用方注入（显式时间域，单测可控，生产传 `Instant::now()`）。
    /// `last_flush` 初始化为 now：首个 delta 最多缓冲 40ms（首字延迟无感）。
    fn new(emitter: &'a dyn LoopEmitter, conv_id: &'a str, asst_msg_id: &'a str, now: Instant) -> Self {
        Self {
            emitter,
            conv_id,
            asst_msg_id,
            pending_text: String::new(),
            pending_think: String::new(),
            pending_tool_deltas: Vec::new(),
            last_flush: now,
        }
    }

    /// 追加文本增量，窗口到期则先发旧缓冲（见「窗口语义」）。
    fn push_text(&mut self, delta: &str, now: Instant) {
        self.maybe_flush(now);
        self.pending_text.push_str(delta);
    }

    /// 追加思考内容增量，窗口到期则先发旧缓冲。
    fn push_thinking(&mut self, content: &str, now: Instant) {
        self.maybe_flush(now);
        self.pending_think.push_str(content);
    }

    /// 追加某 tool call 的参数增量（id 不存在则按首达序入组）。
    fn push_tool_delta(&mut self, id: &str, delta: &str, now: Instant) {
        self.maybe_flush(now);
        if let Some((_, buf)) = self.pending_tool_deltas.iter_mut().find(|(i, _)| i == id) {
            buf.push_str(delta);
        } else {
            self.pending_tool_deltas.push((id.to_string(), delta.to_string()));
        }
    }

    /// 窗口到期则 flush（所有 push_* 的公共头部）。空缓冲的 flush 是 no-op，
    /// 但 last_flush 仍推进——避免静默流恢复后首个 delta 被旧窗口误拆。
    fn maybe_flush(&mut self, now: Instant) {
        if now.duration_since(self.last_flush) >= Duration::from_millis(EMIT_WINDOW_MS) {
            self.flush();
            self.last_flush = now;
        }
    }

    /// 无条件发残余（退出前 / 低频事件前保序用）。空缓冲是 no-op——
    /// 低频事件（tool-call-start/end 等）前排空不产生空事件。
    fn flush(&mut self) {
        if !self.pending_text.is_empty() {
            emit_ser(
                self.emitter,
                "chat:chunk",
                &ChatChunkPayload {
                    conversation_id: self.conv_id.to_string(),
                    message_id: self.asst_msg_id.to_string(),
                    delta: std::mem::take(&mut self.pending_text),
                },
            );
        }
        if !self.pending_think.is_empty() {
            emit_ser(
                self.emitter,
                "chat:thinking",
                &ChatThinkingPayload {
                    conversation_id: self.conv_id.to_string(),
                    message_id: self.asst_msg_id.to_string(),
                    content: std::mem::take(&mut self.pending_think),
                },
            );
        }
        for (id, delta) in std::mem::take(&mut self.pending_tool_deltas) {
            emit_ser(
                self.emitter,
                "chat:tool-call-delta",
                &ChatToolCallDeltaPayload {
                    conversation_id: self.conv_id.to_string(),
                    message_id: self.asst_msg_id.to_string(),
                    id,
                    delta,
                },
            );
        }
    }
}

/// 消费一个 LLM 流，返回 `StreamResult`。
///
/// 在消费过程中 emit chat:chunk / chat:thinking / chat:tool-call-*。
/// 错误透传给调用方（loop_engine 根据 is_retryable() 决定策略）。
pub(crate) async fn consume_stream(
    stream: &mut Pin<Box<dyn Stream<Item = Result<ChatDelta, AppError>> + Send>>,
    emitter: &dyn crate::harness::r#loop::emitter::LoopEmitter,
    cancel: &CancellationToken,
    round_state: &mut RoundState,
    conv_id: &str,
    asst_msg_id: &str,
) -> Result<StreamResult, AppError> {
    use futures::StreamExt;

    let mut text = String::new();
    let mut think = String::new();
    let mut finish_reason = "stop".to_string();
    let mut tool_calls: HashMap<String, CollectedToolCall> = HashMap::new();
    let mut last_usage: Option<TokenUsage> = None;
    let mut agg = DeltaAggregator::new(emitter, conv_id, asst_msg_id, Instant::now());

    while let Some(item) = stream.next().await {
        if cancel.is_cancelled() {
            agg.flush();
            return Ok(StreamResult {
                text,
                think,
                finish_reason,
                tool_calls,
                usage: last_usage,
            });
        }

        match item {
            Ok(ChatDelta::Delta { content: delta }) => {
                text.push_str(&delta);
                agg.push_text(&delta, Instant::now());
            }
            Ok(ChatDelta::ToolCallStart { id, name }) => {
                // 低频事件前排空聚合缓冲：前端先收齐此前的文本，再建 tool 卡条目
                agg.flush();
                tool_calls.insert(
                    id.clone(),
                    CollectedToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: String::new(),
                        ended: false,
                    },
                );
                crate::harness::r#loop::emitter::emit_ser(
                    emitter,
                    "chat:tool-call-start",
                    &ChatToolCallStartPayload {
                        conversation_id: conv_id.to_string(),
                        message_id: asst_msg_id.to_string(),
                        id,
                        name,
                    },
                );
            }
            Ok(ChatDelta::ToolCallDelta {
                id,
                delta: tool_delta,
            }) => {
                if let Some(tc) = tool_calls.get_mut(&id) {
                    tc.arguments.push_str(&tool_delta);
                }
                agg.push_tool_delta(&id, &tool_delta, Instant::now());
            }
            Ok(ChatDelta::ToolCallEnd { id }) => {
                // 该 id 的参数增量必须全部到达后才能发 end——否则前端拿到的
                // arguments 不全（truncateJson / delegate 卡片解析参数缺尾）
                agg.flush();
                if let Some(tc) = tool_calls.get_mut(&id) {
                    tc.ended = true;
                }
                crate::harness::r#loop::emitter::emit_ser(
                    emitter,
                    "chat:tool-call-end",
                    &ChatToolCallEndPayload {
                        conversation_id: conv_id.to_string(),
                        message_id: asst_msg_id.to_string(),
                        id,
                    },
                );
            }
            Ok(ChatDelta::Thinking {
                content: think_content,
            }) => {
                think.push_str(&think_content);
                agg.push_thinking(&think_content, Instant::now());
            }
            Ok(ChatDelta::Usage { usage: u }) => {
                // 字段级合并而非覆盖：Anthropic 分两个事件发 usage——message_start 带
                // input_tokens（prompt）、message_delta 带 output_tokens（completion）且不含
                // input_tokens。直接 last-wins 覆盖会把 prompt 冲成 0（→ user 消息 token_count
                // 恒 0、预算漏算 prompt）。取各字段非零值合并；OpenAI / 单次 usage 路径无影响。
                last_usage = Some(match last_usage {
                    Some(prev) => TokenUsage {
                        prompt_tokens: if u.prompt_tokens > 0 {
                            u.prompt_tokens
                        } else {
                            prev.prompt_tokens
                        },
                        completion_tokens: if u.completion_tokens > 0 {
                            u.completion_tokens
                        } else {
                            prev.completion_tokens
                        },
                        cached_tokens: if u.cached_tokens > 0 {
                            u.cached_tokens
                        } else {
                            prev.cached_tokens
                        },
                    },
                    None => u,
                });
                // 归一守卫：修复「prompt 只报 miss」的兼容端点（cached > prompt 时
                // 补上命中部分）——对已规范 usage 是幂等 no-op。放汇聚点一处治三条
                // 出口（预算累加 / round-state 上屏 / turn_ended 落库）。
                last_usage = last_usage.map(TokenUsage::into_canonical);
                if let Some(ref merged) = last_usage {
                    round_state.tokens_prompt = merged.prompt_tokens;
                    round_state.tokens_completion = merged.completion_tokens;
                    round_state.cached_tokens = merged.cached_tokens;
                }
            }
            Ok(ChatDelta::Done { finish_reason: fr }) => {
                if let Some(fr) = fr {
                    finish_reason = fr;
                }
                agg.flush();
                return Ok(StreamResult {
                    text,
                    think,
                    finish_reason,
                    tool_calls,
                    usage: last_usage,
                });
            }
            Err(e) => {
                // 错误前已收文本补发到前端——与「逐 delta 直发」旧语义对齐
                //（错误发生前的 delta 本就已实时发出）
                agg.flush();
                return Err(e);
            }
        }
    }

    // provider 流枯竭无 Done 的兜底路径
    agg.flush();
    Ok(StreamResult {
        text,
        think,
        finish_reason,
        tool_calls,
        usage: last_usage,
    })
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_usage() -> TokenUsage {
        TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            cached_tokens: 10,
        }
    }

    #[test]
    fn test_stream_result_fields() {
        let r = StreamResult {
            text: "hello world".to_string(),
            think: "thinking...".to_string(),
            finish_reason: "stop".to_string(),
            tool_calls: HashMap::new(),
            usage: Some(make_usage()),
        };
        assert_eq!(r.text, "hello world");
        assert_eq!(r.finish_reason, "stop");
        assert!(r.tool_calls.is_empty());
        let u = r.usage.unwrap();
        assert_eq!(u.prompt_tokens, 100);
        assert_eq!(u.completion_tokens, 50);
        assert_eq!(u.cached_tokens, 10);
    }

    #[test]
    fn test_collected_tool_call() {
        let tc = CollectedToolCall {
            id: "call_123".to_string(),
            name: "read_file".to_string(),
            arguments: r#"{"path":"/test.txt"}"#.to_string(),
            ended: true,
        };
        assert_eq!(tc.name, "read_file");
        assert!(tc.ended);
    }

    #[test]
    fn test_stream_result_tool_calls() {
        let mut tc_map: HashMap<String, CollectedToolCall> = HashMap::new();
        tc_map.insert(
            "call_1".into(),
            CollectedToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"a"}"#.into(),
                ended: true,
            },
        );
        let r = StreamResult {
            text: "".to_string(),
            think: "".to_string(),
            finish_reason: "stop".to_string(),
            tool_calls: tc_map,
            usage: None,
        };
        assert_eq!(r.tool_calls.len(), 1);
        assert!(r.tool_calls.get("call_1").unwrap().ended);
    }

    // ========================================================================
    // DeltaAggregator 测试
    // ========================================================================

    /// 收集型 LoopEmitter（照 session_runner_e2e 的 CollectEmitter 模式，只要 events）
    #[derive(Default)]
    struct SinkEmitter {
        events: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl LoopEmitter for SinkEmitter {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.events
                .lock()
                .expect("sink lock")
                .push((event.to_string(), payload));
        }
    }

    impl SinkEmitter {
        fn take(&self) -> Vec<(String, serde_json::Value)> {
            std::mem::take(&mut *self.events.lock().expect("sink lock"))
        }
        fn joined(&self, event: &str, field: &str) -> String {
            self.events
                .lock()
                .expect("sink lock")
                .iter()
                .filter(|(n, _)| n == event)
                .filter_map(|(_, p)| p[field].as_str().map(String::from))
                .collect()
        }
    }

    #[test]
    fn text_deltas_within_window_merge() {
        let sink = SinkEmitter::default();
        let t0 = Instant::now();
        let mut agg = DeltaAggregator::new(&sink, "c1", "m1", t0);
        agg.push_text("你", t0);
        agg.push_text("好", t0 + Duration::from_millis(10));
        agg.push_text("世", t0 + Duration::from_millis(20));
        assert!(sink.take().is_empty(), "窗口内不应 emit");
        agg.flush();
        let events = sink.take();
        assert_eq!(events.len(), 1, "同窗 3 次 push 合并为 1 条");
        assert_eq!(events[0].0, "chat:chunk");
        assert_eq!(events[0].1["delta"], "你好世");
        assert_eq!(events[0].1["conversation_id"], "c1");
        assert_eq!(events[0].1["message_id"], "m1");
    }

    #[test]
    fn window_expiry_flushes_on_next_push() {
        let sink = SinkEmitter::default();
        let t0 = Instant::now();
        let mut agg = DeltaAggregator::new(&sink, "c1", "m1", t0);
        agg.push_text("a", t0);
        // 41ms 后的 push：先发旧缓冲（仅 "a"），新 delta 进新窗口（check-then-append）
        agg.push_text("b", t0 + Duration::from_millis(41));
        let events = sink.take();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1["delta"], "a");
        // last_flush 已随 flush 推进：同窗后续 push 不再触发
        agg.push_text("c", t0 + Duration::from_millis(42));
        assert!(sink.take().is_empty());
        agg.flush();
        let events = sink.take();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1["delta"], "bc");
    }

    #[test]
    fn empty_flush_is_noop() {
        let sink = SinkEmitter::default();
        let t0 = Instant::now();
        let mut agg = DeltaAggregator::new(&sink, "c1", "m1", t0);
        agg.flush();
        assert!(sink.take().is_empty(), "空缓冲 flush 不产空事件");
    }

    #[test]
    fn tool_deltas_grouped_by_id_in_arrival_order() {
        let sink = SinkEmitter::default();
        let t0 = Instant::now();
        let mut agg = DeltaAggregator::new(&sink, "c1", "m1", t0);
        agg.push_tool_delta("a", "x", t0);
        agg.push_tool_delta("b", "y", t0 + Duration::from_millis(5));
        agg.push_tool_delta("a", "z", t0 + Duration::from_millis(10));
        agg.flush();
        let events = sink.take();
        assert_eq!(events.len(), 2, "按 id 分组各 1 条");
        assert_eq!(events[0].1["id"], "a");
        assert_eq!(events[0].1["delta"], "xz");
        assert_eq!(events[1].1["id"], "b");
        assert_eq!(events[1].1["delta"], "y");
    }

    #[test]
    fn channel_order_text_then_thinking_then_tool() {
        let sink = SinkEmitter::default();
        let t0 = Instant::now();
        let mut agg = DeltaAggregator::new(&sink, "c1", "m1", t0);
        // 故意乱序 push：flush 顺序仍固定 text → thinking → tool-delta
        agg.push_tool_delta("t", "1", t0);
        agg.push_thinking("想", t0 + Duration::from_millis(5));
        agg.push_text("文", t0 + Duration::from_millis(10));
        agg.flush();
        let events = sink.take();
        let names: Vec<&str> = events.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec!["chat:chunk", "chat:thinking", "chat:tool-call-delta"]
        );
    }

    // ---- consume_stream 集成：时间无关断言（拼接守恒 + 保序），CI 慢机不 flaky ----

    fn boxed_stream(
        items: Vec<Result<ChatDelta, AppError>>,
    ) -> Pin<Box<dyn Stream<Item = Result<ChatDelta, AppError>> + Send>> {
        Box::pin(futures::stream::iter(items))
    }

    fn round_state() -> RoundState {
        RoundState {
            round: 1,
            elapsed_ms: 0,
            tokens_prompt: 0,
            tokens_completion: 0,
            cached_tokens: 0,
            retry_count: 0,
        }
    }

    #[tokio::test]
    async fn consume_stream_merge_conserves_and_orders() {
        let sink = SinkEmitter::default();
        let cancel = CancellationToken::new();
        let mut rs = round_state();
        let mut stream = boxed_stream(vec![
            Ok(ChatDelta::Delta { content: "你".into() }),
            Ok(ChatDelta::Delta { content: "好".into() }),
            Ok(ChatDelta::Thinking { content: "思考".into() }),
            Ok(ChatDelta::ToolCallStart { id: "t1".into(), name: "read_file".into() }),
            Ok(ChatDelta::ToolCallDelta { id: "t1".into(), delta: r#"{"pa"#.into() }),
            Ok(ChatDelta::ToolCallDelta { id: "t1".into(), delta: r#"th":1}"#.into() }),
            Ok(ChatDelta::ToolCallEnd { id: "t1".into() }),
            Ok(ChatDelta::Done { finish_reason: Some("tool_use".into()) }),
        ]);
        let sr = consume_stream(&mut stream, &sink, &cancel, &mut rs, "c1", "m1")
            .await
            .expect("consume ok");
        // 数据面不受聚合影响
        assert_eq!(sr.text, "你好");
        assert_eq!(sr.think, "思考");
        assert_eq!(sr.tool_calls["t1"].arguments, r#"{"path":1}"#);
        assert_eq!(sr.finish_reason, "tool_use");

        // 拼接守恒：无论真实时间如何分窗，事件拼接必等于原文
        assert_eq!(sink.joined("chat:chunk", "delta"), "你好");
        assert_eq!(sink.joined("chat:thinking", "content"), "思考");
        assert_eq!(sink.joined("chat:tool-call-delta", "delta"), r#"{"path":1}"#);

        // 保序：最后一条 tool-call-delta 必须先于 tool-call-end（End 前 flush 锁定）
        let events = sink.take();
        let idx_last_delta = events
            .iter()
            .rposition(|(n, _)| n == "chat:tool-call-delta")
            .expect("has tool delta");
        let idx_end = events
            .iter()
            .position(|(n, _)| n == "chat:tool-call-end")
            .expect("has tool end");
        assert!(idx_last_delta < idx_end);
    }

    #[tokio::test]
    async fn consume_stream_err_flushes_pending() {
        let sink = SinkEmitter::default();
        let cancel = CancellationToken::new();
        let mut rs = round_state();
        let mut stream = boxed_stream(vec![
            Ok(ChatDelta::Delta { content: "a".into() }),
            Ok(ChatDelta::Delta { content: "b".into() }),
            Err(AppError::Stream("boom".into())),
        ]);
        let err = consume_stream(&mut stream, &sink, &cancel, &mut rs, "c1", "m1")
            .await
            .expect_err("should propagate");
        assert!(err.to_string().contains("boom"));
        // 错误前已收文本已补发（与逐 delta 直发的旧语义对齐）
        assert_eq!(sink.joined("chat:chunk", "delta"), "ab");
    }

    #[tokio::test]
    async fn consume_stream_cancel_before_start_emits_nothing() {
        let sink = SinkEmitter::default();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let mut rs = round_state();
        let mut stream = boxed_stream(vec![
            Ok(ChatDelta::Delta { content: "x".into() }),
            Ok(ChatDelta::Done { finish_reason: None }),
        ]);
        let sr = consume_stream(&mut stream, &sink, &cancel, &mut rs, "c1", "m1")
            .await
            .expect("cancel 走 Ok 返回");
        assert_eq!(sr.text, "");
        assert!(sink.take().is_empty(), "首个 delta 前取消：无任何事件");
    }
}
