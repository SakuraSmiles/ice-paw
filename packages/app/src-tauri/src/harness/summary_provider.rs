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
                cap,
                None, // 摘要固定走 Agent 默认 model（不受会话级 override 影响）
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
                content,
                is_error,
                ..
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
        assert!(prompt.contains("[调用工具 read_file"), "工具调用应被渲染: {prompt}");
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
        assert!(
            prompt2.contains("[图片已省略]"),
            "图片应占位: {prompt2}"
        );
    }
}