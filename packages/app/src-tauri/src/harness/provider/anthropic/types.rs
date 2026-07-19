//! Anthropic API 请求/响应类型 + ChatMessage 格式转换
//!
//! 内容：
//! - `ChatRequest` / `AnthropicMessage` / `AnthropicTool` — 序列化到 Anthropic Messages API 的请求体
//! - `ApiErrorBody` / `ApiErrorDetail` — 反序列化 Anthropic 错误响应
//! - `split_system_prompt()` — 把 `ChatMessage` 列表拆分为 (system, messages)
//!   （Anthropic 不允许 system 在 messages 里，所以单独剥离到顶层字段）
//! - `chat_message_to_anthropic_content()` — 把 `ChatMessage.content` 转换为
//!   Anthropic 格式的 content block 数组（text / image / tool_use / tool_result）

use serde::{Deserialize, Serialize};

use crate::infra::protocol::{ChatMessage, ContentBlock};

// =========================================================================
// 请求 / 响应 结构
// =========================================================================

/// 请求体（发给 Anthropic Messages API 的 JSON）
#[derive(Serialize)]
pub(crate) struct ChatRequest<'a> {
    pub(crate) model: &'a str,
    pub(crate) max_tokens: i32,
    pub(crate) temperature: f64,
    /// 顶层 system 字段；为 `None` 时不序列化。
    /// P2-3: 当 cache_prompt 启用时使用数组格式（支持 cache_control 断点）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) system: Option<&'a serde_json::Value>,
    pub(crate) messages: &'a [serde_json::Value],
    /// 工具定义
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<AnthropicTool>>,
    pub(crate) stream: bool,
}

/// Anthropic 的单条消息（content 支持 string 或 content block 数组）
#[derive(Serialize)]
pub(crate) struct AnthropicMessage {
    /// 只能是 `"user"` 或 `"assistant"`（system 在顶层字段）
    pub(crate) role: String,
    /// 序列化为 string（纯文本）或数组（含工具调用的 content block）
    pub(crate) content: serde_json::Value,
}

/// Anthropic 格式工具定义
#[derive(Serialize)]
pub(crate) struct AnthropicTool {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: serde_json::Value,
}

/// Anthropic 错误响应的 body 结构
#[derive(Deserialize)]
pub(crate) struct ApiErrorBody {
    pub(crate) error: ApiErrorDetail,
}

#[derive(Deserialize)]
pub(crate) struct ApiErrorDetail {
    /// 错误类型：`authentication_error` / `invalid_request_error` / `rate_limit_error` 等
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
}

// =========================================================================
// 消息格式转换
// =========================================================================

/// 把 `ChatMessage` 列表拆分为 (system_prompt, messages)：
/// - 抽离 `role == "system"` 的消息（多个则用 `\n\n` 拼接），不放入 Anthropic 的 messages
/// - `user` / `assistant` 保留并转换为 Anthropic content block 格式
/// - `tool` 角色转为 `user` + tool_result content block（Anthropic 不支持 tool role）
pub(crate) fn split_system_prompt(
    messages: &[ChatMessage],
) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut msgs: Vec<AnthropicMessage> = Vec::with_capacity(messages.len());

    for m in messages {
        match m.role.as_str() {
            "system" => system_parts.push(m.content_text()),
            "user" | "assistant" => {
                let content = chat_message_to_anthropic_content(m);
                msgs.push(AnthropicMessage {
                    role: m.role.clone(),
                    content,
                });
            }
            "tool" => {
                // Anthropic 没有 tool role，把 tool 结果转为 user 消息中的 tool_result content block
                let content = chat_message_to_anthropic_content(m);
                msgs.push(AnthropicMessage {
                    role: "user".to_string(),
                    content,
                });
            }
            _ => continue,
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    (system, msgs)
}

/// 把内部 ChatMessage 转换为 Anthropic API content 数组
///
/// Anthropic 的 content block 格式：
/// - Text → `{ type: "text", text: "..." }`
/// - Image → `{ type: "image", source: { type: "base64", media_type, data } }` （P2-2）
/// - ToolUse → `{ type: "tool_use", id: "...", name: "...", input: {...} }`
/// - ToolResult → `{ type: "tool_result", tool_use_id: "...", content: "..." }`
/// - Thinking → 跳过（Anthropic 不接受回传的 thinking 块）
fn chat_message_to_anthropic_content(msg: &ChatMessage) -> serde_json::Value {
    // 纯文本消息 → 序列化为 string（更简洁）
    let all_text = msg
        .content
        .iter()
        .all(|b| matches!(b, ContentBlock::Text { .. }));
    if all_text {
        return serde_json::Value::String(msg.content_text());
    }

    // 含工具调用/结果/图片 → 序列化为数组
    let arr: Vec<serde_json::Value> = msg
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(serde_json::json!({
                "type": "text",
                "text": text
            })),
            // P2-2: 图片 → Anthropic image block
            ContentBlock::Image { data, media_type } => Some(serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": media_type,
                    "data": data
                }
            })),
            ContentBlock::ToolUse { id, name, input } => {
                // input 是 JSON 字符串，解析为对象
                let parsed: serde_json::Value =
                    serde_json::from_str(input).unwrap_or(serde_json::Value::Null);
                Some(serde_json::json!({
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": parsed
                }))
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => Some(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": is_error.unwrap_or(false)
            })),
            ContentBlock::Thinking { .. } => None, // 不回传给 Anthropic
        })
        .collect();

    serde_json::Value::Array(arr)
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage::from_text(role, content)
    }

    // ================================================================
    // split_system_prompt 系列
    // ================================================================

    #[test]
    fn split_empty_messages() {
        let (sys, msgs) = split_system_prompt(&[]);
        assert!(sys.is_none());
        assert!(msgs.is_empty());
    }

    #[test]
    fn split_user_only() {
        let input = vec![msg("user", "hi")];
        let (sys, msgs) = split_system_prompt(&input);
        assert!(sys.is_none());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hi");
    }

    #[test]
    fn split_user_assistant_preserves_order() {
        let input = vec![
            msg("user", "Q1"),
            msg("assistant", "A1"),
            msg("user", "Q2"),
        ];
        let (sys, msgs) = split_system_prompt(&input);
        assert!(sys.is_none());
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[2].role, "user");
        assert_eq!(msgs[2].content, "Q2");
    }

    #[test]
    fn split_system_plus_conversation() {
        let input = vec![
            msg("system", "你是助手"),
            msg("user", "你好"),
            msg("assistant", "你好！"),
        ];
        let (sys, msgs) = split_system_prompt(&input);
        assert_eq!(sys.as_deref(), Some("你是助手"));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn split_multiple_system_joins_with_blank_line() {
        let input = vec![
            msg("system", "rule 1"),
            msg("system", "rule 2"),
            msg("user", "go"),
        ];
        let (sys, msgs) = split_system_prompt(&input);
        assert_eq!(sys.as_deref(), Some("rule 1\n\nrule 2"));
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn split_converts_tool_role_to_user() {
        // P2-1: tool role 消息转为 user 消息（Anthropic 不支持 tool role）
        let input = vec![
            msg("system", "sys"),
            msg("tool", "tool result"),
            msg("user", "hi"),
        ];
        let (sys, msgs) = split_system_prompt(&input);
        assert_eq!(sys.as_deref(), Some("sys"));
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user"); // tool → user
        assert_eq!(msgs[1].role, "user");
    }

    // ================================================================
    // P2-2: ContentBlock::Image 转换为 Anthropic image block
    // ================================================================

    #[test]
    fn anthropic_image_block_shape() {
        // 验证期望的 Anthropic image block JSON 结构
        // （与 types::chat_message_to_anthropic_content 等价）
        let data = "iVBORw0KGgo";
        let media_type = "image/png";
        let expected = serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": media_type,
                "data": data
            }
        });
        assert_eq!(expected["type"], "image");
        assert_eq!(expected["source"]["type"], "base64");
        assert_eq!(expected["source"]["media_type"], "image/png");
        assert_eq!(expected["source"]["data"], "iVBORw0KGgo");
    }

    /// Image + Text 混合 → adapter 生成的 Anthropic content 数组
    #[test]
    fn anthropic_mixed_image_text() {
        // 直接模拟 types::chat_message_to_anthropic_content 的 filter_map 逻辑
        let blocks = [
            ContentBlock::text("描述一下这张图"),
            ContentBlock::image("AAAA", "image/jpeg"),
            ContentBlock::text("谢谢"),
        ];

        let arr: Vec<serde_json::Value> = blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(serde_json::json!({
                    "type": "text",
                    "text": text
                })),
                ContentBlock::Image { data, media_type } => Some(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": media_type,
                        "data": data
                    }
                })),
                _ => None,
            })
            .collect();

        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "描述一下这张图");
        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[1]["source"]["data"], "AAAA");
        assert_eq!(arr[1]["source"]["media_type"], "image/jpeg");
        assert_eq!(arr[2]["type"], "text");
        assert_eq!(arr[2]["text"], "谢谢");
    }

    /// split_system_prompt 对 user 含 Image 的消息应保留为 user role
    /// （Anthropic 允许 user 消息中含 image block）
    #[test]
    fn split_user_with_image_preserves_role() {
        let msg = ChatMessage {
            role: "user".into(),
            content: vec![
                ContentBlock::text("看这张图"),
                ContentBlock::image("BBBB", "image/webp"),
            ],
        };
        let (sys, msgs) = split_system_prompt(&[msg]);
        assert!(sys.is_none());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        // content 应该是数组（含 image block）
        assert!(msgs[0].content.is_array());
        let arr = msgs[0].content.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[1]["source"]["data"], "BBBB");
    }
}
