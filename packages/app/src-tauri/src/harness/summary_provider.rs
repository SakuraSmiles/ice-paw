//! M1.5: `LlmSummaryProvider` — 基于 LLM 的滚动摘要实现
//!
//! 职责：
//! - 持有一个 `LlmProvider` + API Key，调用 `stream_summary`（摘要专用通道，
//!   GLM 端点注入思考开关）走标准的 Anthropic / OpenAI 流式协议
//! - prompt 模板：system 描述摘要员职责（最多 3 句，保留目标/事实/工具/错误），
//!   user 是被摘要的消息列表（`[role]: content` 格式）
//! - 流消费：把所有 `ChatDelta::Delta { content }` 拼成一个完整字符串返回
//! - **熔断器**：连续空结果 / 调用失败（thinking 模型烧光额度等 provider 级
//!   故障）按模型熔断——停止每轮徒劳重试（6~11s 延迟 + 一次全量输入计费），
//!   冷却期满半开探测（见下方 `SummaryBreakerState`）
//!
//! 设计要点（dev1 评审）：
//! - **依赖倒置**：实现 `context::memory::SummaryProvider` trait（context 层
//!   定义，harness 层实现），保持 context 层不依赖 harness 层
//! - **复用 ChatState 的 cancel**：调用方传入 `&CancellationToken`，
//!   provider 在每次 chunk 消费前检查 `is_cancelled()`，
//!   已取消立刻停止 stream 消费
//! - **温度 = 0**：摘要应稳定可复现
//! - **max_tokens = 512 硬上限**：摘要最多 3 句话，远低于 512 tokens 实际限制
//! - **不启用 tools**：摘要阶段不调用工具

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::StreamExt;
use tracing::{error, info, warn};

use crate::context::memory::SummaryProvider;
use crate::error::AppResult;
use crate::infra::cancel::CancellationToken;
use crate::infra::protocol::{ChatDelta, ChatMessage, ContentBlock, LlmProvider};

/// 系统 prompt：定义摘要员职责与输出约束
///
/// 关键约束（dev2 设计 § M1.5 / Phase 2 滚动折叠）：
/// - **最多 3 句话**：控制摘要长度，摘要本身也会被注入 LLM 上下文
/// - **保留**：用户目标 / 关键事实 / 文件路径 / 工具名 / 错误信息
/// - **忽略**：客套与 Markdown 格式
/// - **增量扩展**：滚动折叠会把前序摘要作为首条消息喂入；模型应在其基础上
///   扩展更新，而非从零重写——勿丢已捕获事实
const SUMMARY_SYSTEM_PROMPT: &str = "你是一位对话摘要员。将早期对话历史压缩为最多3句话的摘要。\
保留：用户目标与意图、已确认的关键事实与偏好、已读文件路径、\
已调用的工具名称、关键错误信息。\
代码块仅保留函数名和用途。忽略：客套与 Markdown 格式。\
若提供了前序摘要，在其基础上扩展更新，切勿丢弃已捕获的事实。";

/// 摘要 LLM 调用的 max_tokens 硬上限
///
/// 最多 3 句话 + 标记词 ≈ 200 tokens；给到 512 留 buffer 避免被服务端截断。
const SUMMARY_MAX_TOKENS: i32 = 512;

/// 摘要 prompt 中**文本 / 工具入参**的截断字符数
///
/// 一次失败的 tool_result 可能是数百 KB；不截断会把摘要 prompt 自己撑爆。
/// 摘要只需要点，500 字符足够。
const SUMMARY_FIELD_MAX_INPUT: usize = 500;
/// 摘要 prompt 中**工具结果 content**的截断字符数（结果通常比入参更长）
const SUMMARY_FIELD_MAX_RESULT: usize = 1000;

// =========================================================================
// 摘要熔断器（进程级，按模型分组）
// =========================================================================

/// 连续空结果多少次后熔断
const SUMMARY_BREAKER_TRIP_AFTER: u32 = 3;
/// 熔断冷却时长（期满后半开：放行一次探测，再空则立即重开）
const SUMMARY_BREAKER_COOLDOWN_MS: u64 = 10 * 60 * 1000;

/// 单模型熔断状态（纯数据；判定/记录逻辑在下方纯函数，便于单测时间旅行）
#[derive(Default)]
struct SummaryBreakerState {
    consecutive_empty: u32,
    open_until_ms: u64,
}

/// 进程级注册表。`LlmSummaryProvider` 每次 send 都重新构造（持的是
/// `Arc<dyn LlmProvider>`），状态必须外置；按 `model_name()` 分组——
/// 故障是模型级的（thinking 模型烧光额度），不能让别的模型连坐。
static SUMMARY_BREAKERS: OnceLock<Mutex<HashMap<String, SummaryBreakerState>>> = OnceLock::new();

fn summary_breakers() -> &'static Mutex<HashMap<String, SummaryBreakerState>> {
    SUMMARY_BREAKERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 是否处于熔断开启期（应跳过 LLM 调用）
fn breaker_should_skip(st: &SummaryBreakerState, now_ms: u64) -> bool {
    st.open_until_ms > now_ms
}

/// 记录一次空结果。返回是否**新触发**熔断（供调用方升级日志，只报一次）。
fn breaker_record_empty(st: &mut SummaryBreakerState, now_ms: u64) -> bool {
    st.consecutive_empty = st.consecutive_empty.saturating_add(1);
    if st.consecutive_empty >= SUMMARY_BREAKER_TRIP_AFTER {
        let newly_tripped = st.open_until_ms <= now_ms; // 之前未开/冷却已过 → 新一轮熔断
        st.open_until_ms = now_ms.saturating_add(SUMMARY_BREAKER_COOLDOWN_MS);
        newly_tripped
    } else {
        false
    }
}

/// 记录一次成功（清零计数 + 解除熔断）
fn breaker_record_success(st: &mut SummaryBreakerState) {
    st.consecutive_empty = 0;
    st.open_until_ms = 0;
}

/// 记一次摘要失败（空文本**或调用 Err**）并按需升级告警。
///
/// 两类同属 provider 级故障（thinking 烧光额度→空、端点拒收→Err），都应计数；
/// 触顶只打一次 error（可见性：此前 34 次失败只有 warn 刷屏）。
fn breaker_note_failure(model_key: &str) {
    let newly_tripped = {
        let mut breakers = summary_breakers()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        breaker_record_empty(breakers.entry(model_key.to_string()).or_default(), now_ms())
    };
    if newly_tripped {
        error!(
            target: "ice_paw.summary",
            "摘要熔断：模型 {} 连续 {} 次摘要调用失败或返回空文本，{} 分钟内跳过摘要调用。\
             最常见根因：thinking 模型（如 glm-5.2）把 max_tokens={} 全部消耗在思考通道，\
             content 恒为空——滚动摘要失效会导致历史无法折叠、每轮全量重发直到预算熔断。\
             请检查该模型的思考开关是否已禁用（stream_summary 通道）或更换摘要模型。",
            model_key,
            SUMMARY_BREAKER_TRIP_AFTER,
            SUMMARY_BREAKER_COOLDOWN_MS / 60_000,
            SUMMARY_MAX_TOKENS
        );
    }
}

/// 记一次摘要成功（清零计数 + 解除熔断）
fn breaker_note_success(model_key: &str) {
    let mut breakers = summary_breakers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    breaker_record_success(breakers.entry(model_key.to_string()).or_default());
}

/// `LlmSummaryProvider` — 通过 LLM 流式调用实现滚动摘要
///
/// # 字段
/// - `provider`: `Arc<dyn LlmProvider>` —— 复用 `chat_cmd` 阶段已构造好的 provider
///   （不同 agent 可能用不同的 provider，但同一个会话内不变）
/// - `api_key`: 调用时传入，不在 Adapter 中持久化（与 provider 设计一致）
pub struct LlmSummaryProvider {
    provider: Arc<dyn LlmProvider>,
    api_key: String,
}

impl LlmSummaryProvider {
    /// 构造 LlmSummaryProvider
    ///
    /// @param provider  已构造好的 LLM provider（Anthropic / OpenAI / 其他）
    /// @param api_key   API key，每次调用时透传给 provider
    pub fn new(provider: Arc<dyn LlmProvider>, api_key: String) -> Self {
        Self { provider, api_key }
    }
}

#[async_trait]
impl SummaryProvider for LlmSummaryProvider {
    async fn summarize(
        &self,
        messages: &[ChatMessage],
        max_tokens: usize,
        cancel: &CancellationToken,
    ) -> AppResult<String> {
        if messages.is_empty() {
            // 无消息可摘要，返回空字符串（MemoryStage 到达此路径前已保证非空）
            return Ok(String::new());
        }

        // 熔断检查：连续空结果的模型直接跳过本次调用（省一次 6~11s 延迟 +
        // 一次全量输入计费）。返回空串 → MemoryStage 走既有「跳过落库」路径。
        let model_key = self.provider.model_name().to_string();
        {
            let mut breakers = summary_breakers()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let st = breakers.entry(model_key.clone()).or_default();
            if breaker_should_skip(st, now_ms()) {
                drop(breakers); // 先释放锁再打日志（await-free 但保持短临界区纪律）
                info!(
                    target: "ice_paw.summary",
                    "摘要熔断中（{} 连续空结果 ≥{} 次），冷却期内跳过摘要调用",
                    model_key, SUMMARY_BREAKER_TRIP_AFTER
                );
                return Ok(String::new());
            }
        }

        // 调用方（MemoryStage）传入目标 token 上限；clamp 到 [200, 硬上限]，
        // 既尊重调用方意图（折叠批次大小不同），又守住摘要长度纪律。
        let cap = max_tokens.clamp(200, SUMMARY_MAX_TOKENS as usize) as i32;

        info!(
            target: "ice_paw.summary",
            "开始摘要：{} 条消息（cap={} tokens）",
            messages.len(),
            cap
        );

        // 构造 user prompt：把消息列表按 block 渲染（含工具调用 / 结果，带截断）
        let user_prompt = build_summary_user_prompt(messages);

        // 调 LLM 流式 API——走 stream_summary 专用通道（GLM 注入思考开关，
        // 防 thinking 模型把小额度全烧在思考通道上导致 content 恒空）。
        // 建连失败（HTTP 4xx/5xx/网络）与空结果同为 provider 级故障：计入熔断
        // 后原样上抛——MemoryStage 对 Err 降级（跳过折叠不阻塞对话）。
        let stream = match self
            .provider
            .stream_summary(
                &self.api_key,
                vec![
                    ChatMessage::from_text("system", SUMMARY_SYSTEM_PROMPT.to_string()),
                    ChatMessage::from_text("user", user_prompt),
                ],
                0.0, // temperature = 0：摘要应稳定可复现
                cap,
                cancel.clone(),
            )
            .await
        {
            Ok(stream) => stream,
            Err(e) => {
                warn!(
                    target: "ice_paw.summary",
                    "摘要调用失败（{}）：{}",
                    model_key, e
                );
                breaker_note_failure(&model_key);
                return Err(e);
            }
        };

        // 消费 stream 拼接完整文本
        let mut full = String::new();
        tokio::pin!(stream);
        while let Some(result) = stream.next().await {
            // 每次 chunk 前检查取消（与 harness 内部风格一致）
            if cancel.is_cancelled() {
                warn!(
                    target: "ice_paw.summary",
                    "摘要被取消，已累计 {} 字符",
                    full.len()
                );
                break;
            }
            match result {
                Ok(delta) => {
                    if let ChatDelta::Delta { content } = delta {
                        full.push_str(&content);
                    }
                    // 其它 delta（ToolCallStart / Done / Usage 等）忽略
                    // - 摘要不启用 tools，ToolCall* 不应出现
                    // - Usage / Done 是控制信号，不影响文本拼接
                }
                Err(e) => {
                    // 流错误：warn + 终止（避免无限循环）
                    warn!(
                        target: "ice_paw.summary",
                        "摘要流错误，已累计 {} 字符：{}",
                        full.len(),
                        e
                    );
                    break;
                }
            }
        }

        info!(
            target: "ice_paw.summary",
            "摘要完成：{} 条消息 → {} 字符",
            messages.len(),
            full.len()
        );

        // 熔断记账：取消不计（用户主动停止非 provider 故障）；空结果累计，
        // 触顶升级 error 一次性告警（可见性：此前 34 次失败只有 warn 刷屏）。
        if cancel.is_cancelled() {
            return Ok(full);
        }
        if full.trim().is_empty() {
            breaker_note_failure(&model_key);
        } else {
            breaker_note_success(&model_key);
        }

        Ok(full)
    }
}

/// 构造 user prompt：把消息列表按 block 渲染为 `[role]: <body>` 格式
///
/// Phase 2 改为 **block 感知**：旧实现用 `content_text()` 只取 Text 块，
/// 会把整条含 tool_use / tool_result 的消息记成空——而工具调用恰是「用户做了什么」
/// 的关键事实。现在按 block 类型渲染：
/// - `Text`       → 原文（截断到 [`SUMMARY_FIELD_MAX_INPUT`]）
/// - `ToolUse`    → `[调用工具 <name>，入参 <input 截断>]`
/// - `ToolResult` → `[工具结果/失败 <content 截断>]`
/// - `Thinking`   → 跳过（内部推理，对摘要无价值）
/// - `Image`      → `[图片已省略]`（摘要无法承载像素）
///
/// 仅含 Thinking / 空白的消息整条跳过，避免空行。每条消息一行 + 前缀 role，
/// 便于 LLM 在 3 句话里压缩关键信息。
fn build_summary_user_prompt(messages: &[ChatMessage]) -> String {
    let mut text = String::with_capacity(messages.len() * 64);
    text.push_str("以下是对话历史，请生成摘要：\n---\n");
    for m in messages {
        let body = render_message_for_summary(m);
        if body.trim().is_empty() {
            continue;
        }
        text.push_str(&format!("[{}]: {}\n", m.role, body));
    }
    text.push_str("---\n");
    text
}

/// 把单条消息的 content block 渲染为摘要友好的纯文本（截断、丢思考块、图片占位）
fn render_message_for_summary(m: &ChatMessage) -> String {
    use std::fmt::Write;
    let mut parts: Vec<String> = Vec::new();
    for b in &m.content {
        match b {
            ContentBlock::Text { text } => {
                let t = text.trim();
                if !t.is_empty() {
                    parts.push(truncate_str(t, SUMMARY_FIELD_MAX_INPUT));
                }
            }
            ContentBlock::ToolUse { name, input, .. } => {
                let mut s = String::new();
                let _ = write!(
                    s,
                    "[调用工具 {name}，入参 {}]",
                    truncate_str(input, SUMMARY_FIELD_MAX_INPUT)
                );
                parts.push(s);
            }
            ContentBlock::ToolResult {
                content, is_error, ..
            } => {
                let tag = if is_error.unwrap_or(false) {
                    "失败"
                } else {
                    "结果"
                };
                let mut s = String::new();
                let _ = write!(
                    s,
                    "[工具{tag} {}]",
                    truncate_str(content, SUMMARY_FIELD_MAX_RESULT)
                );
                parts.push(s);
            }
            ContentBlock::Thinking { .. } => {} // 内部推理，不进摘要
            ContentBlock::Image { .. } => parts.push("[图片已省略]".to_string()),
            // 附件元信息块：纯 UI，跳过——紧随其后的 Text(extracted) 块以
            // "[附件 name（kind）]" 开头，已携带附件名+正文进摘要，无需重复。
            ContentBlock::Attachment { .. } => {}
        }
    }
    parts.join(" ")
}

/// 按**字符数**截断字符串，超出则追加省略号 `…`
fn truncate_str(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(max_chars.min(s.len()) + 3);
    let mut chars = s.chars();
    for _ in 0..max_chars {
        match chars.next() {
            Some(c) => out.push(c),
            None => break,
        }
    }
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;

    // ---------------------------------------------------------------
    // 熔断器：纯状态机时间旅行
    // ---------------------------------------------------------------

    #[test]
    fn breaker_state_machine_time_travel() {
        let mut st = SummaryBreakerState::default();
        assert!(!breaker_record_empty(&mut st, 1_000), "第 1 次空不触发");
        assert!(!breaker_record_empty(&mut st, 1_001), "第 2 次空不触发");
        assert!(!breaker_should_skip(&st, 1_002), "触顶前不熔断");
        assert!(breaker_record_empty(&mut st, 1_002), "第 3 次空 → 新触发");
        assert!(breaker_should_skip(&st, 1_003), "熔断开启期跳过");
        let expiry = 1_002 + SUMMARY_BREAKER_COOLDOWN_MS;
        assert!(!breaker_should_skip(&st, expiry), "冷却期满放行（半开）");
        // 半开探测再空 → 立即重开且再报一次新触发
        assert!(breaker_record_empty(&mut st, expiry + 1));
        assert!(breaker_should_skip(&st, expiry + 2));
        // 成功清零：解除熔断 + 计数归零
        breaker_record_success(&mut st);
        assert!(!breaker_should_skip(&st, expiry + 3));
        assert!(
            !breaker_record_empty(&mut st, expiry + 4),
            "清零后重新从 1 计"
        );
    }

    // ---------------------------------------------------------------
    // 熔断器：summarize 集成（计数 Provider 驱动真实路径）
    // ---------------------------------------------------------------

    /// 计数 + 可配置产出的测试 Provider（emit=None 空流 / Some=先产一条 Delta；
    /// fail=true 时 stream_chat 直接 Err，模拟端点拒收 / 网络故障）
    struct CountingProvider {
        model: String,
        calls: std::sync::atomic::AtomicUsize,
        emit: std::sync::Mutex<Option<String>>,
        fail: std::sync::atomic::AtomicBool,
    }

    impl CountingProvider {
        fn new(model: &str, emit: Option<String>) -> Self {
            Self {
                model: model.to_string(),
                calls: std::sync::atomic::AtomicUsize::new(0),
                emit: std::sync::Mutex::new(emit),
                fail: std::sync::atomic::AtomicBool::new(false),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn set_emit(&self, emit: Option<String>) {
            *self.emit.lock().unwrap() = emit;
        }

        fn set_fail(&self, fail: bool) {
            self.fail.store(fail, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl LlmProvider for CountingProvider {
        async fn stream_chat(
            &self,
            _api_key: &str,
            _messages: Vec<ChatMessage>,
            _tools: Option<Vec<crate::infra::protocol::ToolDef>>,
            _temperature: f64,
            _max_tokens: i32,
            _model: Option<&str>,
            _cancel: CancellationToken,
        ) -> AppResult<Pin<Box<dyn futures::Stream<Item = AppResult<ChatDelta>> + Send>>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(crate::error::AppError::Llm("HTTP 400 模拟拒收".into()));
            }
            let (tx, rx) = tokio::sync::mpsc::channel::<AppResult<ChatDelta>>(4);
            if let Some(text) = self.emit.lock().unwrap().clone() {
                let _ = tx.try_send(Ok(ChatDelta::Delta { content: text }));
            }
            drop(tx); // 关闭 → 流立即结束
            Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
        }

        fn model_name(&self) -> &str {
            &self.model
        }
    }

    fn sample_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage::from_text("user", "你好"),
            ChatMessage::from_text("assistant", "你好！"),
        ]
    }

    #[tokio::test]
    async fn breaker_trips_after_consecutive_empties_and_skips_call() {
        let provider = Arc::new(CountingProvider::new("bk-trip-test", None));
        let sp = LlmSummaryProvider::new(provider.clone(), "key".into());
        let cancel = CancellationToken::new();
        let msgs = sample_messages();

        for _ in 0..SUMMARY_BREAKER_TRIP_AFTER {
            let out = sp.summarize(&msgs, 300, &cancel).await.unwrap();
            assert!(out.is_empty());
        }
        assert_eq!(provider.calls(), SUMMARY_BREAKER_TRIP_AFTER as usize);

        // 第 4 次：熔断开启 → 不再调用 provider，直接返回空串
        let out = sp.summarize(&msgs, 300, &cancel).await.unwrap();
        assert!(out.is_empty());
        assert_eq!(
            provider.calls(),
            SUMMARY_BREAKER_TRIP_AFTER as usize,
            "熔断后应跳过 LLM 调用"
        );
    }

    #[tokio::test]
    async fn breaker_cancel_not_counted() {
        // 取消是用户行为非 provider 故障：预取消的 token 连续多轮不得熔断
        let provider = Arc::new(CountingProvider::new("bk-cancel-test", None));
        let sp = LlmSummaryProvider::new(provider.clone(), "key".into());
        let cancel = CancellationToken::new();
        cancel.cancel();
        let msgs = sample_messages();

        for _ in 0..(SUMMARY_BREAKER_TRIP_AFTER + 1) {
            sp.summarize(&msgs, 300, &cancel).await.unwrap();
        }
        assert_eq!(
            provider.calls(),
            (SUMMARY_BREAKER_TRIP_AFTER + 1) as usize,
            "取消不计入熔断，每次都应真正调用"
        );
    }

    /// 调用 Err（端点拒收/网络）与空结果同为 provider 级故障：连续 3 次 Err
    /// 也熔断——第 4 次（即使 provider 已恢复）直接跳过，返回空串。
    #[tokio::test]
    async fn breaker_counts_call_errors() {
        let provider = Arc::new(CountingProvider::new(
            "bk-err-test",
            Some("恢复后的产出".into()),
        ));
        provider.set_fail(true);
        let sp = LlmSummaryProvider::new(provider.clone(), "key".into());
        let cancel = CancellationToken::new();
        let msgs = sample_messages();

        for _ in 0..SUMMARY_BREAKER_TRIP_AFTER {
            assert!(sp.summarize(&msgs, 300, &cancel).await.is_err());
        }
        assert_eq!(provider.calls(), SUMMARY_BREAKER_TRIP_AFTER as usize);

        // provider 恢复，但熔断开启期 → 不再调用，直接空串
        provider.set_fail(false);
        let out = sp.summarize(&msgs, 300, &cancel).await.unwrap();
        assert!(out.is_empty());
        assert_eq!(
            provider.calls(),
            SUMMARY_BREAKER_TRIP_AFTER as usize,
            "调用 Err 连续触顶后应熔断跳过"
        );
    }

    #[tokio::test]
    async fn breaker_success_resets_counter() {
        let provider = Arc::new(CountingProvider::new("bk-reset-test", None));
        let sp = LlmSummaryProvider::new(provider.clone(), "key".into());
        let cancel = CancellationToken::new();
        let msgs = sample_messages();

        // 空×2（计 2）→ 成功 1 次（清零）→ 空×2（重新计 2，未触顶）
        sp.summarize(&msgs, 300, &cancel).await.unwrap();
        sp.summarize(&msgs, 300, &cancel).await.unwrap();
        provider.set_emit(Some("有内容的摘要".into()));
        let out = sp.summarize(&msgs, 300, &cancel).await.unwrap();
        assert_eq!(out, "有内容的摘要");
        provider.set_emit(None);
        sp.summarize(&msgs, 300, &cancel).await.unwrap();
        sp.summarize(&msgs, 300, &cancel).await.unwrap();

        // 未熔断：第 6 次仍真正调用（若成功未清零，第 5 次就触顶跳过了）
        sp.summarize(&msgs, 300, &cancel).await.unwrap();
        assert_eq!(provider.calls(), 6, "成功清零后计数从 1 重计，6 次全真调");
    }

    #[test]
    fn build_summary_user_prompt_includes_role_and_content() {
        let msgs = vec![
            ChatMessage::from_text("user", "你好"),
            ChatMessage::from_text("assistant", "你好！有什么可以帮你的？"),
        ];
        let prompt = build_summary_user_prompt(&msgs);
        assert!(
            prompt.contains("[user]: 你好"),
            "应包含 user 消息: {prompt}"
        );
        assert!(
            prompt.contains("[assistant]: 你好！"),
            "应包含 assistant 消息: {prompt}"
        );
        assert!(prompt.contains("---"), "应包含分隔符");
    }

    #[test]
    fn build_summary_user_prompt_handles_empty_input() {
        let prompt = build_summary_user_prompt(&[]);
        // 空输入应仅返回骨架（开头 + 分隔符 + 结尾），不 panic
        assert!(prompt.starts_with("以下是对话历史"));
        assert!(prompt.contains("---"));
        assert!(prompt.ends_with("---\n"));
    }

    #[test]
    fn system_prompt_has_three_sentence_constraint() {
        // 防止误改 system prompt 导致摘要变长
        assert!(
            SUMMARY_SYSTEM_PROMPT.contains("最多3句话"),
            "system prompt 应包含「最多3句话」约束"
        );
    }

    #[test]
    fn build_summary_user_prompt_renders_tool_blocks_and_truncates() {
        // Phase 2 关键回归：工具块必须被渲染（旧 content_text 路径会丢），且超长内容被截断
        let huge_input = "x".repeat(2000); // > SUMMARY_FIELD_MAX_INPUT(500)
        let huge_result = "y".repeat(2000); // > SUMMARY_FIELD_MAX_RESULT(1000)
        let msgs = vec![ChatMessage {
            role: "assistant".into(),
            content: vec![
                ContentBlock::Text {
                    text: "我来查一下".into(),
                },
                ContentBlock::ToolUse {
                    id: "c1".into(),
                    name: "read_file".into(),
                    input: huge_input.clone(),
                },
            ],
            source_rowid: None,
        }];
        let prompt = build_summary_user_prompt(&msgs);
        assert!(
            prompt.contains("[调用工具 read_file"),
            "工具调用应被渲染: {prompt}"
        );
        assert!(prompt.contains("…"), "超长入参应被截断加省略号: {prompt}");
        // 截断后 prompt 不应含完整 2000 字符原文
        assert!(!prompt.contains(&huge_input), "超长入参不应原样进 prompt");

        // 工具结果（含失败标记）
        let result_msgs = vec![ChatMessage {
            role: "user".into(),
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "c1".into(),
                content: huge_result.clone(),
                is_error: Some(true),
            }],
            source_rowid: None,
        }];
        let prompt2 = build_summary_user_prompt(&result_msgs);
        assert!(prompt2.contains("[工具失败"), "失败结果应有标记: {prompt2}");
        assert!(!prompt2.contains(&huge_result), "超长结果不应原样进 prompt");
    }

    #[test]
    fn build_summary_user_prompt_skips_thinking_and_omits_image() {
        let msgs = vec![ChatMessage {
            role: "assistant".into(),
            content: vec![
                ContentBlock::Thinking {
                    thinking: "internal reasoning".into(),
                    signature: None,
                },
                ContentBlock::text("实际回复"),
            ],
            source_rowid: None,
        }];
        let prompt = build_summary_user_prompt(&msgs);
        assert!(prompt.contains("实际回复"));
        assert!(
            !prompt.contains("internal reasoning"),
            "思考块不应进摘要 prompt: {prompt}"
        );

        let img_msgs = vec![ChatMessage {
            role: "user".into(),
            content: vec![ContentBlock::image("BASE64", "image/png")],
            source_rowid: None,
        }];
        let prompt2 = build_summary_user_prompt(&img_msgs);
        assert!(prompt2.contains("[图片已省略]"), "图片应占位: {prompt2}");
    }
}
