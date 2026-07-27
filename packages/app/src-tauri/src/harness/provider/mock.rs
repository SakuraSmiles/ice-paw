//! `harness::provider::mock` — REQ-XC-011 MockProvider
//!
//! 一个**不依赖网络**的 `LlmProvider` 实现，用于：
//! - 单元测试 / 集成测试中替代真实 OpenAI / Anthropic Adapter
//! - 前端开发模式中无需 API key 也能跑通流式链路
//! - 验证异常流（429 / 503 / 超时 / 空响应）的处理逻辑
//!
//! ## 设计
//!
//! `MockProvider` 通过一个 [`MockScenario`] 枚举来描述「这次请求应该产生什么流」，
//! 调用方在构造时选定场景，调用 `stream_chat` 时按场景生成对应 `ChatDelta` 序列：
//!
//! | Scenario           | 行为                                                |
//! |--------------------|-----------------------------------------------------|
//! | `NormalReply`      | 产出 `"Hello from MockProvider"` + Done             |
//! | `RateLimited`      | 立即返回 `AppError::Llm("HTTP 429 ...")` 错误      |
//! | `ServiceUnavailable` | 立即返回 `AppError::Llm("HTTP 503 ...")` 错误    |
//! | `Timeout`          | 持有 cancel 检查，永不主动 send，直到外部 cancel   |
//! | `EmptyResponse`    | 仅产出 Done，不发任何 Delta（模拟 LLM 沉默）        |
//! | `EchoUser`         | 把最后一条 user 消息的文本作为回复（用于交互测试）  |
//!
//! 所有 Scenario 都尊重 `cancel.is_cancelled()`：收到取消信号会立刻 yield Done{finish_reason=abort}
//! 并结束流，保证「调用方主动取消不会卡死消费方」。
//!
//! ## 用法示例
//!
//! ```ignore
//! use ice_paw_lib::harness::provider::mock::{MockProvider, MockScenario};
//!
//! let provider = MockProvider::new("mock-model", MockScenario::NormalReply);
//! let stream = provider.stream_chat("sk-fake", messages, None, 0.7, 100, None, cancel).await?;
//! ```
//!
//! ## 为什么不需要 wiremock
//!
//! `MockProvider` 是**纯内存**实现，零网络 / 零端口开销。
//! 对比 `tests/openai_test.rs` 中用 `wiremock` 起本地 HTTP server 的方式，
//! MockProvider 更适合「不希望被网络层干扰」的纯逻辑测试。

use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;

use crate::error::{AppError, AppResult};
use crate::harness::chat_state::CancellationToken;
use crate::infra::protocol::{ChatDelta, ChatMessage, LlmProvider, TokenUsage};

// =========================================================================
// Scenario 枚举
// =========================================================================

/// MockProvider 行为场景
///
/// 见模块级文档表格说明每个变体的语义。
/// 所有变体都尊重 [`CancellationToken`] —— 收到 cancel 时立刻终止。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MockScenario {
    /// 正常流：发送 `"Hello from MockProvider"` + `Done`
    ///
    /// 附带 `Usage` 增量（prompt=10, completion=4）便于测试消费方对 Token 用量的处理。
    NormalReply,

    /// HTTP 429 限流：`stream_chat` 立即返回 `Err`
    ///
    /// 错误信息形如 `"HTTP 429: rate limit exceeded (mock)"`，
    /// 调用方可通过 `AppError::is_retryable()` 判定为可重试。
    RateLimited,

    /// HTTP 503 服务不可用：`stream_chat` 立即返回 `Err`
    ///
    /// 错误信息形如 `"HTTP 503: service unavailable (mock)"`，
    /// 同样是可重试错误。
    ServiceUnavailable,

    /// 永不主动产出任何 `ChatDelta`（模拟请求/响应挂起）
    ///
    /// 内部实现为：每 50ms 检查 `cancel.is_cancelled()`，
    /// 发现取消就 yield `Done{finish_reason=abort}` 并结束。
    /// 这样单元测试可以靠 `cancel.cancel()` 来打破「挂起」并验证取消链路。
    Timeout,

    /// 空响应：仅 yield `Done{finish_reason=stop}`，不发任何 Delta
    ///
    /// 模拟「LLM 调用成功但模型选择不输出任何内容」的边界场景。
    EmptyResponse,

    /// 回显最后一条 user 消息的文本内容
    ///
    /// 用于「前端测试能不能正确把 user 输入送回聊天区」的轻量场景。
    /// 找不到 user 消息时回落到 [`MockScenario::NormalReply`]。
    EchoUser,

    /// 自定义文本回复（携带在 Scenario 内）
    ///
    /// 给定字符串在流中按字符切分（每字符一个 chunk）模拟增量输出，
    /// 末尾 yield `Done{finish_reason=stop}`。
    /// 用于「测试特定内容流式输出」的场景。
    CustomText(String),
}

impl MockScenario {
    /// 场景简称（用于日志 / 调试）
    pub fn name(&self) -> &'static str {
        match self {
            MockScenario::NormalReply => "NormalReply",
            MockScenario::RateLimited => "RateLimited",
            MockScenario::ServiceUnavailable => "ServiceUnavailable",
            MockScenario::Timeout => "Timeout",
            MockScenario::EmptyResponse => "EmptyResponse",
            MockScenario::EchoUser => "EchoUser",
            MockScenario::CustomText(_) => "CustomText",
        }
    }
}

// =========================================================================
// MockProvider 结构
// =========================================================================

/// Mock LLM Provider（不发起任何 HTTP 请求）
///
/// - `model`：记录的模型名（`model_name()` 直接返回）
/// - `scenario`：行为场景，每次 `stream_chat` 都按这个场景生成流
///
/// 字段对外暴露（`pub`）以便测试代码可以：
/// - `let provider = MockProvider::new("m", MockScenario::EmptyResponse);`
/// - `provider.scenario = MockScenario::RateLimited;`  // 切换场景
#[derive(Debug, Clone)]
pub struct MockProvider {
    /// 模型名称
    pub model: String,
    /// 当前场景（`pub` 允许测试中切换）
    pub scenario: MockScenario,
}

impl MockProvider {
    /// 创建 MockProvider
    pub fn new(model: impl Into<String>, scenario: MockScenario) -> Self {
        Self {
            model: model.into(),
            scenario,
        }
    }

    /// 便利构造：默认 NormalReply 场景
    pub fn normal(model: impl Into<String>) -> Self {
        Self::new(model, MockScenario::NormalReply)
    }

    /// 便利构造：限流场景
    pub fn rate_limited(model: impl Into<String>) -> Self {
        Self::new(model, MockScenario::RateLimited)
    }

    /// 便利构造：503 场景
    pub fn service_unavailable(model: impl Into<String>) -> Self {
        Self::new(model, MockScenario::ServiceUnavailable)
    }

    /// 便利构造：超时场景
    pub fn timeout(model: impl Into<String>) -> Self {
        Self::new(model, MockScenario::Timeout)
    }

    /// 便利构造：空响应场景
    pub fn empty(model: impl Into<String>) -> Self {
        Self::new(model, MockScenario::EmptyResponse)
    }
}

// =========================================================================
// LlmProvider 实现
// =========================================================================

#[async_trait]
impl LlmProvider for MockProvider {
    fn model_name(&self) -> &str {
        &self.model
    }

    async fn stream_chat(
        &self,
        _api_key: &str,
        messages: Vec<ChatMessage>,
        _tools: Option<Vec<crate::infra::protocol::ToolDef>>,
        _temperature: f64,
        _max_tokens: i32,
        model: Option<&str>,
        cancel: CancellationToken,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<ChatDelta>> + Send>>> {
        // P0-3: 与真实 Adapter 保持一致——优先用调用方传入的 model，否则 self.model
        // 仅在「已决定本次调用是否走通」前打印日志使用，不影响流内容。
        let effective_model = model.unwrap_or(&self.model);
        tracing::debug!(
            target: "ice_paw.mock",
            "MockProvider::stream_chat 触发: scenario={}, model={}, messages={}",
            self.scenario.name(),
            effective_model,
            messages.len(),
        );

        // 错误场景：直接返回 Err，stream_chat 调用方会拿到 Result::Err
        match &self.scenario {
            MockScenario::RateLimited => {
                return Err(AppError::Llm(
                    "HTTP 429: rate limit exceeded (mock)".to_string(),
                ));
            }
            MockScenario::ServiceUnavailable => {
                return Err(AppError::Llm(
                    "HTTP 503: service unavailable (mock)".to_string(),
                ));
            }
            _ => {}
        }

        // 正常路径：构造 Stream
        // 这里用同步 stream! 宏（futures crate）+ 异步 channel 二选一；
        // 选 channel 是因为 Timeout 场景需要在 spawn 的任务里死循环检查 cancel，
        // 而正常/自定义场景一次性 yield 完所有 chunk 即可。
        let (tx, rx) = tokio::sync::mpsc::channel::<AppResult<ChatDelta>>(64);
        let scenario = self.scenario.clone();
        let effective_model_owned = effective_model.to_string();

        tokio::spawn(async move {
            // 先检查一次 cancel（极早期取消可避免做无用功）
            if cancel.is_cancelled() {
                let _ = tx
                    .send(Ok(ChatDelta::Done {
                        finish_reason: Some("abort".to_string()),
                    }))
                    .await;
                return;
            }

            match scenario {
                MockScenario::RateLimited | MockScenario::ServiceUnavailable => {
                    // 已在上面 Err 返回，这里走不到；防御性再 yield Done
                    let _ = tx
                        .send(Ok(ChatDelta::Done {
                            finish_reason: Some("error".to_string()),
                        }))
                        .await;
                }

                MockScenario::NormalReply => {
                    // 一段文本 + Usage + Done
                    let _ = tx
                        .send(Ok(ChatDelta::Delta {
                            content: "Hello from MockProvider".to_string(),
                        }))
                        .await;
                    let _ = tx
                        .send(Ok(ChatDelta::Usage {
                            usage: TokenUsage {
                                prompt_tokens: 10,
                                completion_tokens: 4,
                                cached_tokens: 0,
                            },
                        }))
                        .await;
                    let _ = tx
                        .send(Ok(ChatDelta::Done {
                            finish_reason: Some("stop".to_string()),
                        }))
                        .await;
                }

                MockScenario::EchoUser => {
                    // 找到最后一条 user 消息并回显其文本
                    let user_text = messages
                        .iter()
                        .rev()
                        .find(|m| m.role == "user")
                        .map(|m| m.content_text())
                        .unwrap_or_else(|| "Hello from MockProvider".to_string());

                    let _ = tx
                        .send(Ok(ChatDelta::Delta { content: user_text }))
                        .await;
                    let _ = tx
                        .send(Ok(ChatDelta::Done {
                            finish_reason: Some("stop".to_string()),
                        }))
                        .await;
                }

                MockScenario::CustomText(text) => {
                    // 按字符切分（简单起见；真实 LLM 一般按 token 切）
                    for ch in text.chars() {
                        if cancel.is_cancelled() {
                            let _ = tx
                                .send(Ok(ChatDelta::Done {
                                    finish_reason: Some("abort".to_string()),
                                }))
                                .await;
                            return;
                        }
                        let _ = tx
                            .send(Ok(ChatDelta::Delta {
                                content: ch.to_string(),
                            }))
                            .await;
                    }
                    let _ = tx
                        .send(Ok(ChatDelta::Done {
                            finish_reason: Some("stop".to_string()),
                        }))
                        .await;
                }

                MockScenario::EmptyResponse => {
                    // 仅 yield Done，不发任何 Delta
                    let _ = tx
                        .send(Ok(ChatDelta::Done {
                            finish_reason: Some("stop".to_string()),
                        }))
                        .await;
                }

                MockScenario::Timeout => {
                    // 死循环：每 50ms 检查 cancel，直到外部触发
                    // 不 yield 任何 chunk，直到 cancel
                    loop {
                        if cancel.is_cancelled() {
                            let _ = tx
                                .send(Ok(ChatDelta::Done {
                                    finish_reason: Some("abort".to_string()),
                                }))
                                .await;
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }

            // 确保 effective_model_owned 不会被 unused 警告（debug 日志已用过）
            let _ = effective_model_owned;
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

// =========================================================================
// 辅助：构造一个最小可用的消息列表（给测试用）
// =========================================================================

impl MockProvider {
    /// 构造一条 user 消息 "hi"
    pub fn sample_messages() -> Vec<ChatMessage> {
        vec![ChatMessage::from_text("user", "hi")]
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    // -----------------------------------------------------------------
    // 工具：把 stream 消耗到一个 Vec，便于断言
    // -----------------------------------------------------------------
    async fn drain(stream: Pin<Box<dyn Stream<Item = AppResult<ChatDelta>> + Send>>) -> Vec<ChatDelta> {
        let mut out = Vec::new();
        let mut s = stream;
        while let Some(item) = s.next().await {
            // 测试中遇到 Err 直接 panic，便于定位
            out.push(item.expect("stream item should be Ok in mock test"));
        }
        out
    }

    // -----------------------------------------------------------------
    // Scenario::name() 各变体的标签应稳定
    // -----------------------------------------------------------------
    #[test]
    fn scenario_name_is_stable() {
        assert_eq!(MockScenario::NormalReply.name(), "NormalReply");
        assert_eq!(MockScenario::RateLimited.name(), "RateLimited");
        assert_eq!(MockScenario::ServiceUnavailable.name(), "ServiceUnavailable");
        assert_eq!(MockScenario::Timeout.name(), "Timeout");
        assert_eq!(MockScenario::EmptyResponse.name(), "EmptyResponse");
        assert_eq!(MockScenario::EchoUser.name(), "EchoUser");
        assert_eq!(MockScenario::CustomText("x".into()).name(), "CustomText");
    }

    // -----------------------------------------------------------------
    // 工厂方法
    // -----------------------------------------------------------------
    #[test]
    fn factory_helpers_set_correct_scenario() {
        assert_eq!(
            MockProvider::normal("m").scenario,
            MockScenario::NormalReply
        );
        assert_eq!(
            MockProvider::rate_limited("m").scenario,
            MockScenario::RateLimited
        );
        assert_eq!(
            MockProvider::service_unavailable("m").scenario,
            MockScenario::ServiceUnavailable
        );
        assert_eq!(MockProvider::timeout("m").scenario, MockScenario::Timeout);
        assert_eq!(MockProvider::empty("m").scenario, MockScenario::EmptyResponse);
        assert_eq!(MockProvider::new("m", MockScenario::EchoUser).model, "m");
    }

    // -----------------------------------------------------------------
    // model_name 直接透传
    // -----------------------------------------------------------------
    #[test]
    fn model_name_returns_bound_model() {
        let p = MockProvider::normal("claude-mock");
        assert_eq!(p.model_name(), "claude-mock");

        let p = MockProvider::new("custom-model", MockScenario::EmptyResponse);
        assert_eq!(p.model_name(), "custom-model");
    }

    // -----------------------------------------------------------------
    // REQ-XC-011 异常流 #1: HTTP 429（rate limit）
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn rate_limited_returns_429_error() {
        let provider = MockProvider::rate_limited("mock");
        let cancel = CancellationToken::new();

        let result = provider
            .stream_chat(
                "sk-fake",
                MockProvider::sample_messages(),
                None,
                0.7,
                100,
                None,
                cancel,
            )
            .await;

        let err = result.err().expect("429 场景应返回 Err");
        let msg = err.to_string();
        assert!(
            msg.contains("429"),
            "错误消息应包含 HTTP 429，实际：{}",
            msg
        );
        // 限流属于「可重试」错误
        assert!(
            err.is_retryable(),
            "429 限流应被 is_retryable() 判定为可重试"
        );
    }

    // -----------------------------------------------------------------
    // REQ-XC-011 异常流 #2: HTTP 503（service unavailable）
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn service_unavailable_returns_503_error() {
        let provider = MockProvider::service_unavailable("mock");
        let cancel = CancellationToken::new();

        let result = provider
            .stream_chat(
                "sk-fake",
                MockProvider::sample_messages(),
                None,
                0.7,
                100,
                None,
                cancel,
            )
            .await;

        let err = result.err().expect("503 场景应返回 Err");
        let msg = err.to_string();
        assert!(
            msg.contains("503"),
            "错误消息应包含 HTTP 503，实际：{}",
            msg
        );
        assert!(
            err.is_retryable(),
            "503 应被 is_retryable() 判定为可重试"
        );
    }

    // -----------------------------------------------------------------
    // REQ-XC-011 异常流 #3: 超时（永远挂起直到外部 cancel）
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn timeout_hangs_until_cancel() {
        let provider = MockProvider::timeout("mock");
        let cancel = CancellationToken::new();

        let stream = provider
            .stream_chat(
                "sk-fake",
                MockProvider::sample_messages(),
                None,
                0.7,
                100,
                None,
                cancel.clone(),
            )
            .await
            .expect("timeout 场景应能成功构造 stream");

        // spawn 一个任务去消费 stream
        let mut stream = Box::pin(stream);
        let consumer = tokio::spawn(async move {
            let mut got_done = false;
            // 设一个 timeout 防测试卡死：500ms 后取消 token
            let cancel_timer = tokio::spawn({
                let cancel = cancel.clone();
                async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    cancel.cancel();
                }
            });

            // 消费 stream（每次只处理一个 item，无需循环）
            if let Some(item) = stream.next().await {
                match item {
                    Ok(ChatDelta::Done { finish_reason }) => {
                        assert_eq!(finish_reason.as_deref(), Some("abort"));
                        got_done = true;
                    }
                    Ok(_) => {
                        // Timeout 场景下不应有 Delta
                        panic!("Timeout 场景不应产出任何 Delta chunk");
                    }
                    Err(e) => panic!("unexpected error: {e}"),
                }
            }
            let _ = cancel_timer.await;
            got_done
        });

        // 整个流程应在 1s 内结束（200ms 等待 cancel + 余量）
        let got = tokio::time::timeout(Duration::from_secs(2), consumer)
            .await
            .expect("timeout 场景在 cancel 后应能立即结束")
            .expect("consumer task 不应 panic");
        assert!(got, "应在 cancel 后收到 Done{{abort}}");
    }

    // -----------------------------------------------------------------
    // REQ-XC-011 异常流 #4: 空响应（仅 Done，不发 Delta）
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn empty_response_yields_only_done() {
        let provider = MockProvider::empty("mock");
        let cancel = CancellationToken::new();

        let stream = provider
            .stream_chat(
                "sk-fake",
                MockProvider::sample_messages(),
                None,
                0.7,
                100,
                None,
                cancel,
            )
            .await
            .expect("empty 场景应能成功构造 stream");

        let chunks = drain(stream).await;

        // 仅有 1 个 Done，不应有 Delta
        assert_eq!(chunks.len(), 1, "空响应应只有 1 个 Done，实际：{:?}", chunks);
        match &chunks[0] {
            ChatDelta::Done { finish_reason } => {
                assert_eq!(finish_reason.as_deref(), Some("stop"));
            }
            other => panic!("expected Done, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------
    // 正常流（happy path）作为对照
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn normal_reply_yields_text_then_done() {
        let provider = MockProvider::normal("mock");
        let cancel = CancellationToken::new();

        let stream = provider
            .stream_chat(
                "sk-fake",
                MockProvider::sample_messages(),
                None,
                0.7,
                100,
                None,
                cancel,
            )
            .await
            .expect("normal 场景应能构造 stream");

        let chunks = drain(stream).await;

        // 顺序：Delta("Hello from MockProvider") → Usage → Done
        assert_eq!(chunks.len(), 3);
        match &chunks[0] {
            ChatDelta::Delta { content } => {
                assert_eq!(content, "Hello from MockProvider");
            }
            other => panic!("expected Delta, got: {:?}", other),
        }
        match &chunks[1] {
            ChatDelta::Usage { usage } => {
                assert_eq!(usage.prompt_tokens, 10);
                assert_eq!(usage.completion_tokens, 4);
            }
            other => panic!("expected Usage, got: {:?}", other),
        }
        match &chunks[2] {
            ChatDelta::Done { finish_reason } => {
                assert_eq!(finish_reason.as_deref(), Some("stop"));
            }
            other => panic!("expected Done, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------
    // EchoUser：把 user 消息文本回显
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn echo_user_repeats_last_user_message() {
        let provider = MockProvider::new("mock", MockScenario::EchoUser);
        let cancel = CancellationToken::new();

        let messages = vec![
            ChatMessage::from_text("system", "you are helpful"),
            ChatMessage::from_text("user", "what is Rust?"),
        ];

        let stream = provider
            .stream_chat("sk-fake", messages, None, 0.7, 100, None, cancel)
            .await
            .expect("EchoUser 应能构造 stream");

        let chunks = drain(stream).await;
        assert_eq!(chunks.len(), 2);
        match &chunks[0] {
            ChatDelta::Delta { content } => assert_eq!(content, "what is Rust?"),
            other => panic!("expected Delta, got: {:?}", other),
        }
        match &chunks[1] {
            ChatDelta::Done { .. } => {}
            other => panic!("expected Done, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------
    // EchoUser 找不到 user 消息时回落到 NormalReply
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn echo_user_falls_back_when_no_user_message() {
        let provider = MockProvider::new("mock", MockScenario::EchoUser);
        let cancel = CancellationToken::new();

        // 只有 system 消息，没有 user
        let messages = vec![ChatMessage::from_text("system", "you are helpful")];

        let stream = provider
            .stream_chat("sk-fake", messages, None, 0.7, 100, None, cancel)
            .await
            .expect("EchoUser fallback 应能构造 stream");

        let chunks = drain(stream).await;
        match &chunks[0] {
            ChatDelta::Delta { content } => {
                assert_eq!(content, "Hello from MockProvider");
            }
            other => panic!("expected fallback Delta, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------
    // CustomText：按字符切分输出
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn custom_text_yields_one_chunk_per_char() {
        let provider = MockProvider::new("mock", MockScenario::CustomText("abc".into()));
        let cancel = CancellationToken::new();

        let stream = provider
            .stream_chat(
                "sk-fake",
                MockProvider::sample_messages(),
                None,
                0.7,
                100,
                None,
                cancel,
            )
            .await
            .expect("CustomText 应能构造 stream");

        let chunks = drain(stream).await;
        // 3 个字符 + 1 个 Done = 4 个
        assert_eq!(chunks.len(), 4);
        let mut s = String::new();
        for c in &chunks[..3] {
            match c {
                ChatDelta::Delta { content } => s.push_str(content),
                other => panic!("expected Delta, got: {:?}", other),
            }
        }
        assert_eq!(s, "abc");
        assert!(matches!(chunks[3], ChatDelta::Done { .. }));
    }

    // -----------------------------------------------------------------
    // model override 透传：调用方传入的 model 优先于 self.model
    // （通过切换 provider.scenario 验证：CustomText 会带上 effective_model）
    // 注：这里仅断言 model_name() 不变（覆盖不影响默认），不影响流内容
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn model_override_does_not_mutate_provider_model() {
        let provider = MockProvider::normal("default-model");
        let cancel = CancellationToken::new();

        // 用 override model 调用
        let stream = provider
            .stream_chat(
                "sk-fake",
                MockProvider::sample_messages(),
                None,
                0.7,
                100,
                Some("override-model"),
                cancel,
            )
            .await
            .expect("override model 应能构造 stream");

        let chunks = drain(stream).await;
        // 流内容与默认 model 一致（MockProvider 不区分 model 内容）
        assert_eq!(chunks.len(), 3);
        assert!(matches!(chunks[0], ChatDelta::Delta { .. }));

        // 关键断言：provider 自身的 model_name 不应被改写
        assert_eq!(provider.model_name(), "default-model");
    }

    // -----------------------------------------------------------------
    // 取消检查：即使在 NormalReply 场景，预先 cancel 也应得到 Done{abort}
    // （注：当前实现下，NormalReply 是一次性 yield 完所有 chunk 后就退出，
    //  因此「预 cancel」的实际可见效果依赖于实现时机。为降低耦合，本测试
    //  通过 CustomText（多 chunk）验证取消路径。）
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn custom_text_respects_cancellation() {
        let provider = MockProvider::new(
            "mock",
            MockScenario::CustomText("abcdefghij".into()),
        );
        let cancel = CancellationToken::new();
        cancel.cancel(); // 预先取消

        let stream = provider
            .stream_chat(
                "sk-fake",
                MockProvider::sample_messages(),
                None,
                0.7,
                100,
                None,
                cancel,
            )
            .await
            .expect("CustomText 应能构造 stream");

        let chunks = drain(stream).await;

        // CustomText 在循环开头检查 cancel，预先取消 → 立刻 Done{abort}，不发任何 Delta
        assert_eq!(
            chunks.len(),
            1,
            "预先 cancel 应跳过所有 Delta 立刻 Done，实际：{:?}",
            chunks
        );
        match &chunks[0] {
            ChatDelta::Done { finish_reason } => {
                assert_eq!(finish_reason.as_deref(), Some("abort"));
            }
            other => panic!("expected Done{{abort}}, got: {:?}", other),
        }
    }
}