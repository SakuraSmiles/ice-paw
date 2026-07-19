//! M1.5: `LlmSummaryProvider` — 基于 LLM 的滚动摘要实现
//!
//! 职责：
//! - 持有一个 `LlmProvider` + API Key，调用 `stream_chat` 走标准的
//!   Anthropic / OpenAI 流式协议
//! - prompt 模板：system 描述摘要员职责（最多 3 句，保留目标/事实/工具/错误），
//!   user 是被摘要的消息列表（`[role]: content` 格式）
//! - 流消费：把所有 `ChatDelta::Delta { content }` 拼成一个完整字符串返回
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

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use tracing::{info, warn};

use crate::context::memory::SummaryProvider;
use crate::error::AppResult;
use crate::infra::cancel::CancellationToken;
use crate::infra::protocol::{ChatMessage, ChatDelta, LlmProvider};

/// 系统 prompt：定义摘要员职责与输出约束
///
/// 关键约束（dev2 设计 § M1.5）：
/// - **最多 3 句话**：控制摘要长度，摘要本身也会被注入 LLM 上下文
/// - **保留**：用户目标 / 关键事实 / 文件路径 / 工具名 / 错误信息
/// - **忽略**：客套与 Markdown 格式
const SUMMARY_SYSTEM_PROMPT: &str = "你是一位对话摘要员。将早期对话历史压缩为最多3句话的摘要。\
保留：用户目标与意图、已确认的关键事实与偏好、已读文件路径、\
已调用的工具名称、关键错误信息。\
代码块仅保留函数名和用途。忽略：客套与 Markdown 格式。";

/// 摘要 LLM 调用的 max_tokens 硬上限
///
/// 最多 3 句话 + 标记词 ≈ 200 tokens；给到 512 留 buffer 避免被服务端截断。
const SUMMARY_MAX_TOKENS: i32 = 512;

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
        _max_tokens: usize,
        cancel: &CancellationToken,
    ) -> AppResult<String> {
        if messages.is_empty() {
            // 无消息可摘要，返回空字符串
            // 调用方（MemoryStage）应已用 compute_split_idx 防御性检查
            return Ok(String::new());
        }

        info!(
            target: "ice_paw.summary",
            "开始摘要：{} 条消息",
            messages.len()
        );

        // 构造 user prompt：把消息列表转成 `[role]: content` 文本格式
        let user_prompt = build_summary_user_prompt(messages);

        // 调 LLM 流式 API
        let stream = self
            .provider
            .stream_chat(
                &self.api_key,
                vec![
                    ChatMessage::from_text("system", SUMMARY_SYSTEM_PROMPT.to_string()),
                    ChatMessage::from_text("user", user_prompt),
                ],
                None, // 摘要不启用工具
                0.0,  // temperature = 0：摘要应稳定可复现
                SUMMARY_MAX_TOKENS,
                cancel.clone(),
            )
            .await?;

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

        Ok(full)
    }
}

/// 构造 user prompt：把消息列表转为 `[role]: content\n` 格式
///
/// 设计选择：每条消息一行 + 前缀 role，便于 LLM 在 3 句话里压缩关键信息
fn build_summary_user_prompt(messages: &[ChatMessage]) -> String {
    use std::fmt::Write;

    let mut text = String::with_capacity(messages.len() * 64);
    text.push_str("以下是对话历史，请生成摘要：\n---\n");
    for m in messages {
        // Write::write_fmt 不会失败（写 String 永不失败）
        let _ = writeln!(text, "[{}]: {}", m.role, m.content_text());
    }
    text.push_str("---\n");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_summary_user_prompt_includes_role_and_content() {
        let msgs = vec![
            ChatMessage::from_text("user", "你好"),
            ChatMessage::from_text("assistant", "你好！有什么可以帮你的？"),
        ];
        let prompt = build_summary_user_prompt(&msgs);
        assert!(prompt.contains("[user]: 你好"), "应包含 user 消息: {prompt}");
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
}