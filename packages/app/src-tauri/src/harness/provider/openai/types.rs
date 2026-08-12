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

/// 把内部 ChatMessage 转换为 OpenAI API 格式消息（可能 1→N）。
///
/// - **含 ToolResult 块** → 展开为「每 `tool_call_id` 一条 `role="tool"` 消息」。
///   OpenAI 协议硬性要求：assistant 的每个 tool_call 必须紧跟一条带 `tool_call_id`
///   的 `role="tool"` 回执，缺任一或多打平成 user 文本都会 400
///   （`insufficient tool messages following tool_calls`）。内部模型按 Anthropic
///   约定把多个 ToolResult 打包进一条 `role=user` 消息（见 loop_engine 阶段 G），
///   这里必须拆开。此前的实现把 ToolResult 当 user 文本拼接 → deepseek/minimax 400。
/// - 其余（纯文本 / assistant(tool_calls) / 图片 / system）→ 单条消息（1:1）。
pub(crate) fn chat_message_to_openai(msg: &ChatMessage) -> AppResult<Vec<OpenAiMessage>> {
    // 工具结果：每个 ToolResult 展开为一条独立的 role="tool" 消息
    let tool_results: Vec<(&String, &String)> = msg
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, content, .. } => Some((tool_use_id, content)),
            _ => None,
        })
        .collect();
    if !tool_results.is_empty() {
        let mut msgs: Vec<OpenAiMessage> = tool_results
            .into_iter()
            .map(|(tuid, content)| OpenAiMessage {
                role: "tool".to_string(),
                content: serde_json::Value::String(content.clone()),
                tool_calls: None,
                tool_call_id: Some(tuid.clone()),
            })
            .collect();
        // Phase B：同消息若含 Image 块（view_attachment_image 回传的渲染图），追加一条
        // role="user" 带 image_url。OpenAI 的 role="tool" 只接受 string、无法携带图片，
        // 若不拆出，Image 会被静默丢弃（Anthropic 协议无此限制——tool_result content 数组
        // 原生支持 image block）。序列：assistant(tool_calls) → tool ×N → user(image)，
        // 视觉模型（gpt-4o 等）可从后续 user 消息读图。
        let has_image = msg.content.iter().any(|b| b.is_image());
        if has_image {
            let arr: Vec<serde_json::Value> = msg
                .content
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
            msgs.push(OpenAiMessage {
                role: "user".to_string(),
                content: serde_json::Value::Array(arr),
                tool_calls: None,
                tool_call_id: None,
            });
        }
        return Ok(msgs);
    }

    // 非工具结果：1:1
    Ok(vec![single_openai_message(msg)?])
}

/// 非工具结果消息的 1:1 转换（纯文本 / assistant(tool_calls) / 图片 / system）。
///
/// ToolResult 已由 [`chat_message_to_openai`] 预先展开，故此处不再处理 ToolResult。
fn single_openai_message(msg: &ChatMessage) -> AppResult<OpenAiMessage> {
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

    // 含工具调用/复杂块的消息
    match msg.role.as_str() {
        "user" => {
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
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(serde_json::json!({
                        "type": "text",
                        "text": text
                    })),
                    // 附件元信息块：纯 UI，不发给 LLM（与 anthropic 适配层同模式）
                    ContentBlock::Attachment { .. } => None,
                    _ => Some(serde_json::json!({"type": "text", "text": ""})),
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
            // assistant 消息：content 为文本 + tool_calls 数组
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
                    _ => {} // ToolResult/Thinking/Attachment 在 assistant 不应出现
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
        _ => {
            // system / 其他角色含复杂块：用数组格式
            let arr: Vec<serde_json::Value> = msg
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(serde_json::json!({
                        "type": "text",
                        "text": text
                    })),
                    // 附件元信息块：纯 UI，不发给 LLM（与 anthropic 适配层同模式）
                    ContentBlock::Attachment { .. } => None,
                    _ => Some(serde_json::json!({"type": "text", "text": ""})),
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

    /// 含多个 ToolResult 的消息（loop_engine 阶段 G 打包成 role=user）必须展开为
    /// 每 tool_call_id 一条 role="tool" 消息——否则 OpenAI 兼容端点 400
    /// （`insufficient tool messages following tool_calls`）。
    #[test]
    fn openai_tool_results_expand_to_n_role_tool_messages() {
        use crate::infra::protocol::ChatMessage;
        let msg = ChatMessage {
            role: "user".into(),
            content: vec![
                ContentBlock::ToolResult {
                    tool_use_id: "call_A".into(),
                    content: "结果甲".into(),
                    is_error: None,
                },
                ContentBlock::ToolResult {
                    tool_use_id: "call_B".into(),
                    content: "结果乙".into(),
                    is_error: Some(true),
                },
            ],
            source_rowid: None,
        };
        let out = chat_message_to_openai(&msg).unwrap();
        assert_eq!(out.len(), 2, "两条 ToolResult → 两条 role=tool 消息");
        assert_eq!(out[0].role, "tool");
        assert_eq!(out[0].tool_call_id.as_deref(), Some("call_A"));
        assert_eq!(out[0].content.as_str().unwrap(), "结果甲");
        assert!(out[0].tool_calls.is_none());
        assert_eq!(out[1].role, "tool");
        assert_eq!(out[1].tool_call_id.as_deref(), Some("call_B"));
        assert_eq!(out[1].content.as_str().unwrap(), "结果乙");
    }

    /// Phase B：ToolResult + Image 同消息（view_attachment_image 回传）必须拆为
    /// [role=tool(文本), role=user(image_url)]——OpenAI 的 role="tool" 只接受 string，
    /// 不拆出则 Image 被静默丢弃。视觉 agent 从后续 user 消息读图。
    #[test]
    fn openai_tool_result_with_image_splits_into_tool_and_user() {
        use crate::infra::protocol::ChatMessage;
        let msg = ChatMessage {
            role: "user".into(),
            content: vec![
                ContentBlock::ToolResult {
                    tool_use_id: "call_1".into(),
                    content: r#"{"page":1,"note":"image attached"}"#.into(),
                    is_error: Some(false),
                },
                ContentBlock::image("iVBORw0KGgo", "image/png"),
            ],
            source_rowid: None,
        };
        let out = chat_message_to_openai(&msg).unwrap();
        assert_eq!(out.len(), 2, "ToolResult + Image → [tool, user(image)]");
        // 第一条：ToolResult 文本
        assert_eq!(out[0].role, "tool");
        assert_eq!(out[0].tool_call_id.as_deref(), Some("call_1"));
        assert!(out[0].content.as_str().unwrap().contains("page"));
        // 第二条：追加的 user 消息带 image_url
        assert_eq!(out[1].role, "user");
        assert!(out[1].tool_call_id.is_none());
        let arr = out[1].content.as_array().expect("user content 应为数组");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "image_url");
        assert_eq!(
            arr[0]["image_url"]["url"],
            "data:image/png;base64,iVBORw0KGgo"
        );
    }

    /// 纯 ToolResult（无 Image）不应追加多余 user 消息——回归保护。
    #[test]
    fn openai_tool_result_without_image_no_extra_user() {
        use crate::infra::protocol::ChatMessage;
        let msg = ChatMessage {
            role: "user".into(),
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content: "纯文本结果".into(),
                is_error: Some(false),
            }],
            source_rowid: None,
        };
        let out = chat_message_to_openai(&msg).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, "tool");
    }

    /// 纯文本消息仍为单条（1:1），role/content 保留，无 tool_calls/tool_call_id。
    #[test]
    fn openai_plain_text_is_single_message() {
        use crate::infra::protocol::ChatMessage;
        let msg = ChatMessage {
            role: "user".into(),
            content: vec![ContentBlock::text("你好")],
            source_rowid: None,
        };
        let out = chat_message_to_openai(&msg).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, "user");
        assert_eq!(out[0].content.as_str().unwrap(), "你好");
        assert!(out[0].tool_calls.is_none());
        assert!(out[0].tool_call_id.is_none());
    }
}
