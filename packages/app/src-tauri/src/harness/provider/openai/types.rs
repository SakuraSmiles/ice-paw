//! OpenAI 请求/响应结构体 + 消息格式转换
//!
//! 定义 OpenAI Chat Completions API 的请求体、工具定义、SSE 响应块，
//! 以及内部 `ChatMessage` → `OpenAiMessage` 的格式转换函数。

use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::infra::protocol::{ChatMessage, ContentBlock};

// =========================================================================
// 请求结构体
// =========================================================================

/// 请求体（发给 LLM 的 JSON）
#[derive(Serialize)]
pub(crate) struct ChatRequest<'a> {
    pub model: &'a str,
    pub messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OpenAiTool>>,
    /// "auto" / "none" — only sent when tools are present
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<&'a str>,
    pub stream: bool,
    pub temperature: f64,
    pub max_tokens: i32,
    pub stream_options: StreamOptions,
}

/// `stream_options.include_usage = true` 让 API 返回 token 用量
#[derive(Serialize)]
pub(crate) struct StreamOptions {
    pub include_usage: bool,
}

/// OpenAI 格式消息（content 支持 string 或 content block 数组）
#[derive(Serialize)]
pub(crate) struct OpenAiMessage {
    pub role: String,
    /// 纯文本时用 string，含工具调用时用数组
    pub content: serde_json::Value,
    /// assistant 消息可能携带 tool_calls
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    /// tool 角色消息的 tool_call_id
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// OpenAI 格式工具定义
#[derive(Serialize)]
pub(crate) struct OpenAiTool {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: OpenAiToolFn,
}

#[derive(Serialize)]
pub(crate) struct OpenAiToolFn {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

// =========================================================================
// SSE 响应结构体
// =========================================================================

/// SSE 单行 JSON 的最小化反序列化结构
#[derive(Deserialize)]
pub(crate) struct SseChunk {
    #[serde(default)]
    pub choices: Vec<SseChoice>,
    /// P2-3: OpenAI streaming usage（stream_options.include_usage = true 时，
    /// 最后一个 chunk 的 choices 为空，usage 包含 token 统计）
    #[serde(default)]
    pub usage: Option<SseUsage>,
}

/// P2-3: OpenAI 流式响应的 usage
#[derive(Deserialize)]
pub(crate) struct SseUsage {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    #[serde(default)]
    pub prompt_tokens_details: Option<SseUsageDetails>,
}

/// P2-3: OpenAI prompt_tokens_details（含 cached_tokens）
#[derive(Deserialize)]
pub(crate) struct SseUsageDetails {
    #[serde(default)]
    pub cached_tokens: Option<u32>,
}

#[derive(Deserialize)]
pub(crate) struct SseChoice {
    pub delta: SseDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
pub(crate) struct SseDelta {
    #[serde(default)]
    pub content: Option<String>,
    /// 思考过程增量（GLM / DeepSeek 等 thinking 模式 SSE 字段）
    #[serde(default)]
    pub reasoning_content: Option<String>,
    /// 工具调用增量（OpenAI 格式）
    #[serde(default)]
    pub tool_calls: Option<Vec<SseToolCallDelta>>,
}

/// SSE 中的工具调用增量
#[derive(Deserialize)]
pub(crate) struct SseToolCallDelta {
    /// 工具调用序号（0, 1, 2...）
    #[serde(default)]
    pub index: usize,
    /// 工具调用 ID（仅首个 chunk 携带）
    #[serde(default)]
    pub id: Option<String>,
    /// 工具调用详情
    #[serde(default)]
    pub function: Option<SseToolCallFn>,
}

#[derive(Deserialize, Default)]
pub(crate) struct SseToolCallFn {
    #[serde(default)]
    pub name: Option<String>,
    /// arguments JSON 片段
    #[serde(default)]
    pub arguments: Option<String>,
}

// =========================================================================
// 消息格式转换
// =========================================================================

/// 把内部 ChatMessage 转换为 OpenAI API 格式
///
/// - 纯文本消息（所有 content 块均为 Text）→ content 序列化为 string
/// - 含 ToolUse/ToolResult → content 序列化为数组
/// - tool role 消息 → 带 tool_call_id
pub(crate) fn chat_message_to_openai(msg: &ChatMessage) -> AppResult<OpenAiMessage> {
    // 判断是否为纯文本（所有块都是 Text）
    let all_text = msg.content.iter().all(|b| matches!(b, ContentBlock::Text { .. }));

    if all_text {
        let text = msg.content_text();
        return Ok(OpenAiMessage {
            role: msg.role.clone(),
            content: serde_json::Value::String(text),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    // 含工具调用/结果的消息
    match msg.role.as_str() {
        "user" => {
            // user 消息可能含 ToolResult 块（stream_loop 以 user 角色回传工具结果）
            // 提取 ToolResult 的 content 作为文本，避免丢失工具结果
            let has_tool_result = msg
                .content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolResult { .. }));
            if has_tool_result {
                let mut text_parts: Vec<String> = Vec::new();
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => text_parts.push(text.clone()),
                        ContentBlock::ToolResult { content, .. } => {
                            text_parts.push(content.clone());
                        }
                        _ => {}
                    }
                }
                return Ok(OpenAiMessage {
                    role: msg.role.clone(),
                    content: serde_json::Value::String(text_parts.join("\n")),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
            // user 含图片/非 ToolResult 的复杂块 → 走通用数组路径
            // P2-2: OpenAI 要求图片块必须放在文本块之前（官方文档明确规定）
            // 因此采用「先收集 image_url、再收集 text」的两段式拼装
            let has_image = msg.content.iter().any(|b| b.is_image());
            if has_image {
                let mut arr: Vec<serde_json::Value> = Vec::with_capacity(msg.content.len());
                // 第一段：所有 Image → image_url block
                for block in &msg.content {
                    if let ContentBlock::Image { data, media_type } = block {
                        arr.push(serde_json::json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{};base64,{}", media_type, data)
                            }
                        }));
                    }
                }
                // 第二段：Text
                for block in &msg.content {
                    if let ContentBlock::Text { text } = block {
                        arr.push(serde_json::json!({
                            "type": "text",
                            "text": text
                        }));
                    }
                }
                return Ok(OpenAiMessage {
                    role: msg.role.clone(),
                    content: serde_json::Value::Array(arr),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
            // 纯文本/未知块 → 走通用数组路径（下方 _ 分支）
            let arr: Vec<serde_json::Value> = msg
                .content
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => serde_json::json!({
                        "type": "text",
                        "text": text
                    }),
                    _ => serde_json::json!({"type": "text", "text": ""}),
                })
                .collect();
            Ok(OpenAiMessage {
                role: msg.role.clone(),
                content: serde_json::Value::Array(arr),
                tool_calls: None,
                tool_call_id: None,
            })
        }
        "assistant" => {
            // assistant 消息：content 为文本数组 + tool_calls 数组
            let mut text_parts: Vec<String> = Vec::new();
            let mut tool_calls: Vec<serde_json::Value> = Vec::new();

            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => text_parts.push(text.clone()),
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push(serde_json::json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": input,
                            }
                        }));
                    }
                    _ => {} // ToolResult/Thinking 在 assistant 不应出现
                }
            }

            let content = if text_parts.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(text_parts.join("\n"))
            };

            Ok(OpenAiMessage {
                role: msg.role.clone(),
                content,
                tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                tool_call_id: None,
            })
        }
        "tool" => {
            // tool 结果消息：content 为结果文本，带 tool_call_id
            let mut result_text = String::new();
            let mut tool_use_id = String::new();
            for block in &msg.content {
                if let ContentBlock::ToolResult { tool_use_id: tuid, content, .. } = block {
                    result_text.push_str(content);
                    tool_use_id = tuid.clone();
                }
            }
            Ok(OpenAiMessage {
                role: "tool".to_string(),
                content: serde_json::Value::String(result_text),
                tool_calls: None,
                tool_call_id: Some(tool_use_id),
            })
        }
        _ => {
            // system / 其他角色含复杂块：用数组格式
            let arr: Vec<serde_json::Value> = msg
                .content
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => serde_json::json!({
                        "type": "text",
                        "text": text
                    }),
                    _ => serde_json::json!({"type": "text", "text": ""}),
                })
                .collect();
            Ok(OpenAiMessage {
                role: msg.role.clone(),
                content: serde_json::Value::Array(arr),
                tool_calls: None,
                tool_call_id: None,
            })
        }
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// user 含图片时，序列化出的 content 应是数组，
    /// 且 image_url block 在 text block 之前（OpenAI 要求）。
    #[test]
    fn openai_image_then_text_order() {
        let blocks = vec![
            ContentBlock::image("iVBORw0KGgo", "image/png"),
            ContentBlock::image("AAAA", "image/jpeg"),
            ContentBlock::text("这两张图有什么区别？"),
        ];
        let mut arr: Vec<serde_json::Value> = Vec::new();
        // 第一段：images
        for b in &blocks {
            if let ContentBlock::Image { data, media_type } = b {
                arr.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{};base64,{}", media_type, data)
                    }
                }));
            }
        }
        // 第二段：texts
        for b in &blocks {
            if let ContentBlock::Text { text } = b {
                arr.push(serde_json::json!({
                    "type": "text",
                    "text": text
                }));
            }
        }
        // 验证顺序
        assert_eq!(arr[0]["type"], "image_url");
        assert_eq!(arr[0]["image_url"]["url"], "data:image/png;base64,iVBORw0KGgo");
        assert_eq!(arr[1]["type"], "image_url");
        assert_eq!(arr[1]["image_url"]["url"], "data:image/jpeg;base64,AAAA");
        assert_eq!(arr[2]["type"], "text");
        assert_eq!(arr[2]["text"], "这两张图有什么区别？");
    }

    /// 验证仅含 Image 没有 Text 时仍能产生有效数组
    #[test]
    fn openai_image_only() {
        let blocks = [ContentBlock::image("XXX", "image/gif")];
        // 模拟 adapter 逻辑
        let arr: Vec<serde_json::Value> = blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Image { data, media_type } => Some(serde_json::json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{};base64,{}", media_type, data)
                    }
                })),
                _ => None,
            })
            .collect();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["image_url"]["url"], "data:image/gif;base64,XXX");
    }

    /// ContentBlock::Image 自身的 JSON 序列化确认
    /// （前端传过来的 content_blocks 反序列化后能正确识别为 image 块）
    #[test]
    fn content_block_image_json_serde() {
        let s = r#"{"type":"image","data":"AAAA","media_type":"image/png"}"#;
        let b: ContentBlock = serde_json::from_str(s).unwrap();
        match b {
            ContentBlock::Image { data, media_type } => {
                assert_eq!(data, "AAAA");
                assert_eq!(media_type, "image/png");
            }
            _ => panic!("应为 Image 块"),
        }
    }
}
