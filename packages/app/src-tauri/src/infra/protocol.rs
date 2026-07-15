//! 跨层协议类型 — 单一数据源
//!
//! 本模块是 chat / llm / commands 跨层共享的协议类型"单一数据源"。
//!
//! 内容分类：
//!
//! 1. **LLM 数据结构**：`ContentBlock` / `ChatMessage` / `ChatDelta` /
//!    `ToolDef` / `TokenUsage` / `LlmProvider` trait
//! 2. **图片支持**：`SUPPORTED_IMAGE_MEDIA_TYPES` / `is_supported_image_media_type()` /
//!    `MAX_IMAGE_SIZE` / `MAX_IMAGE_COUNT` / `validate_images()`
//! 3. **前端入参**：`SendMessageInput` / `TemplateInput`
//! 4. **事件 Payload**：`ChatStartPayload` / `ChatChunkPayload` / `ChatDonePayload` /
//!    `ChatErrorPayload` / `ChatRetryingPayload` / 工具调用 + 思考 payload

use std::pin::Pin;

use async_trait::async_trait;
use base64::Engine as _;
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

// =========================================================================
// LLM 数据结构
// =========================================================================

/// 消息内容块 — 替代原来的 `content: String`
///
/// 采用 `#[serde(tag = "type")]` 实现多态 JSON 序列化，
/// 与 OpenAI / Anthropic 的 content block 格式自然对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// 文本块
    Text { text: String },
    /// P2-2: 图片块（Vision 输入）
    ///
    /// - `data`：base64 编码的图片数据（**不含** `data:image/...;base64,` 前缀，
    ///   前缀在 adapter 里拼接；这样前端只传裸 base64，存储/校验更干净）
    /// - `media_type`：MIME 类型，支持 `"image/png" | "image/jpeg" | "image/gif" | "image/webp"`
    ///
    /// 序列化格式（与前端 `types/index.ts` 对齐）：
    /// ```json
    /// { "type": "image", "data": "iVBORw0KG...", "media_type": "image/png" }
    /// ```
    Image {
        data: String,
        media_type: String,
    },
    /// 工具调用（LLM 产出）
    ToolUse {
        id: String,
        name: String,
        /// JSON 字符串（arguments / input）
        input: String,
    },
    /// 工具结果（回传给 LLM）
    ToolResult {
        tool_use_id: String,
        /// 结果内容（JSON 字符串或纯文本）
        content: String,
        /// 是否出错
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    /// 思考过程（Anthropic extended thinking）
    Thinking {
        thinking: String,
        /// 签名（Anthropic 用于验证）
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
}

impl ContentBlock {
    /// 从纯文本构造 Text block
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }

    /// P2-2: 构造 Image block（裸 base64，无 data URL 前缀）
    pub fn image(data: impl Into<String>, media_type: impl Into<String>) -> Self {
        ContentBlock::Image {
            data: data.into(),
            media_type: media_type.into(),
        }
    }

    /// 提取纯文本内容（仅 Text 变体有）
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text { text } => Some(text),
            _ => None,
        }
    }

    /// P2-2: 是否是 Image 块
    pub fn is_image(&self) -> bool {
        matches!(self, ContentBlock::Image { .. })
    }

    /// 把所有 Text block 的文本拼接成一个 String（兼容旧代码）
    pub fn join_text(blocks: &[ContentBlock]) -> String {
        let mut buf = String::new();
        for b in blocks {
            if let ContentBlock::Text { text } = b {
                buf.push_str(text);
            }
        }
        buf
    }
}

/// P2-2: 支持的图片 MIME 类型白名单
///
/// 与前端 `ImagePicker.vue` 的 `accept` 属性保持一致。
/// Anthropic 支持 `image/jpeg | image/png | image/gif | image/webp`；
/// OpenAI Vision 支持同等集合（部分模型额外支持 `image/png` 高分辨率）。
pub const SUPPORTED_IMAGE_MEDIA_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
];

/// P2-2: 校验 media_type 是否在白名单内
pub fn is_supported_image_media_type(mt: &str) -> bool {
    SUPPORTED_IMAGE_MEDIA_TYPES.contains(&mt)
}

/// 聊天消息（发给 LLM 的上下文中的单条）
///
/// P2-1 升级：`content` 改为 `Vec<ContentBlock>`。
/// 对旧消息（纯文本）使用 `ChatMessage::from_text` 构造。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// 角色："system" | "user" | "assistant" | "tool"
    pub role: String,
    /// 消息内容块数组
    pub content: Vec<ContentBlock>,
}

impl ChatMessage {
    /// 从纯文本快速构造（等同旧版行为）
    pub fn from_text(role: impl Into<String>, content: impl Into<String>) -> Self {
        ChatMessage {
            role: role.into(),
            content: vec![ContentBlock::text(content)],
        }
    }

    /// 把所有 content block 拼成纯文本（兼容旧逻辑 / DB 回写）
    pub fn content_text(&self) -> String {
        ContentBlock::join_text(&self.content)
    }
}

/// P2-3: Token 用量信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// P2-3: 缓存命中的 token 数（Anthropic: cache_read_input_tokens, OpenAI: cached_tokens）
    #[serde(default)]
    pub cached_tokens: u32,
}

/// 流式增量 — LLM 返回的每个 chunk
///
/// - `Delta`：文本增量（最常见）
/// - `ToolCallStart`：工具调用开始（id + name 已知）
/// - `ToolCallDelta`：工具调用参数 JSON 片段
/// - `ToolCallEnd`：工具调用参数完毕
/// - `Thinking`：思考过程增量
/// - `Usage`：P2-3 token 用量（OpenAI streaming usage 或 Anthropic message_start）
/// - `Done`：流结束（携带结束原因）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatDelta {
    /// 文本增量
    Delta { content: String },
    /// 工具调用开始
    ToolCallStart { id: String, name: String },
    /// 工具调用参数 JSON 增量
    ToolCallDelta { id: String, delta: String },
    /// 工具调用参数完成
    ToolCallEnd { id: String },
    /// 思考过程增量
    Thinking { content: String },
    /// P2-3: Token 用量
    Usage { usage: TokenUsage },
    /// 流结束
    Done { finish_reason: Option<String> },
}

/// 工具定义（发给 LLM 的 tool schema）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema（parameters）
    pub parameters: serde_json::Value,
}

// =========================================================================
// Provider Trait
// =========================================================================

/// LLM 提供方接口
///
/// 实现方需提供 `stream_chat`，返回一个异步 Stream 逐块产出 `ChatDelta`。
/// 调用方在消费 Stream 时应定期检查 `cancel.is_cancelled()` 以支持用户停止。
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// 流式聊天
    ///
    /// - `api_key`：调用时传入，不在 Adapter 中持久化
    /// - `messages`：完整上下文（含 system / 历史 / 当前用户消息）
    /// - `tools`：可选的工具定义列表（None = 不启用工具调用）
    /// - `temperature` / `max_tokens`：模型参数
    /// - `cancel`：取消令牌
    async fn stream_chat(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDef>>,
        temperature: f64,
        max_tokens: i32,
        cancel: crate::harness::chat_state::CancellationToken,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<ChatDelta>> + Send>>>;
}

// =========================================================================
// 图片校验常量
// =========================================================================

/// P2-2: 单张图片的最大字节数（base64 解码后的原始字节大小）
///
/// 5MB 限制与 OpenAI / Anthropic 官方建议接近：
/// - OpenAI Vision: 单图 base64 ≤ ~20MB，但实践中 5MB 内体验最佳
/// - Anthropic: 单图 ≤ 5MB（推荐），超过会被服务端拒绝
///
/// 用 `base64` 解码后的字节数校验（不是 base64 字符串长度），
/// 避免「字符串看起来不大但解码后超限」的错误。
pub(crate) const MAX_IMAGE_SIZE: usize = 5 * 1024 * 1024; // 5 MiB

/// P2-2: 单条消息最多图片张数
///
/// OpenAI 文档建议 ≤ 20 张/请求；Anthropic 限制更严格（实测 ≤ 100），
/// 这里统一用 20 保持一致。
pub(crate) const MAX_IMAGE_COUNT: usize = 20;

// =========================================================================
// 图片校验
// =========================================================================

/// P2-2: 校验 content_blocks 中的图片（含尺寸 / 张数 / 类型 / base64 合法性）
///
/// 在 `send_message` 入口处调用，**先于**任何 DB 写入或 LLM 调用。
///
/// 错误信息直接返回给前端用于 toast 提示（使用 `AppError::Validation`
/// → 前端 kind=`"validation"`，可识别为业务级错误）。
pub(crate) fn validate_images(blocks: &[ContentBlock]) -> AppResult<()> {
    let mut image_count = 0usize;

    for (idx, block) in blocks.iter().enumerate() {
        if let ContentBlock::Image { data, media_type } = block {
            image_count += 1;

            // 1. media_type 白名单
            if !is_supported_image_media_type(media_type) {
                return Err(AppError::Validation(format!(
                    "第 {} 张图片格式不支持：{}（允许：png / jpeg / gif / webp）",
                    idx + 1,
                    media_type
                )));
            }

            // 2. base64 解码 + 尺寸校验
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| {
                    AppError::Validation(format!(
                        "第 {} 张图片 base64 解码失败：{}",
                        idx + 1,
                        e
                    ))
                })?;
            if decoded.len() > MAX_IMAGE_SIZE {
                let mb = decoded.len() as f64 / 1024.0 / 1024.0;
                return Err(AppError::Validation(format!(
                    "第 {} 张图片过大：{:.2} MB（最大 {} MB）",
                    idx + 1,
                    mb,
                    MAX_IMAGE_SIZE / 1024 / 1024
                )));
            }
        }
    }

    // 3. 张数上限
    if image_count > MAX_IMAGE_COUNT {
        return Err(AppError::Validation(format!(
            "单条消息最多 {} 张图片，当前 {} 张",
            MAX_IMAGE_COUNT, image_count
        )));
    }

    Ok(())
}

// =========================================================================
// 入参结构
// =========================================================================

/// `send_message` 入参中的模板部分（P2-4）
///
/// - `template_id`  选中的模板 ID
/// - `values`       变量值字典
#[derive(Debug, Clone, Deserialize)]
pub struct TemplateInput {
    pub template_id: String,
    #[serde(default)]
    pub values: std::collections::HashMap<String, String>,
}

/// `send_message` 入参
///
/// P2-2 双接口：
/// - `content: Option<String>` — 旧接口，纯文本（保持向后兼容）
/// - `content_blocks: Option<Vec<ContentBlock>>` — 新接口，支持图片等多模态块
///
/// 优先级：`content_blocks` 存在时优先使用；否则 fallback 到 `content`。
/// 两者都不提供 → 校验失败（与旧版「content 不能为空」一致）。
#[derive(Debug, Deserialize)]
pub struct SendMessageInput {
    pub conversation_id: String,
    /// 旧接口：纯文本（与 P2-1 之前一致）
    /// P2-2 后改为 `Option<String>`，与 `content_blocks` 二选一
    #[serde(default)]
    pub content: Option<String>,
    /// P2-2: 新接口：富文本块（含 Image 等多模态）
    #[serde(default)]
    pub content_blocks: Option<Vec<ContentBlock>>,
    /// 可选：附加的模板（应用后会被渲染并注入到 system_prompt / user_prompt_prefix）
    #[serde(default)]
    pub template: Option<TemplateInput>,
    /// P2-1: 是否启用工具调用
    #[serde(default)]
    pub tools_enabled: bool,
}

// =========================================================================
// 事件 Payload 结构
// =========================================================================

/// `chat:start` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatStartPayload {
    pub conversation_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
}

/// `chat:chunk` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatChunkPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub delta: String,
}

/// `chat:done` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatDonePayload {
    pub conversation_id: String,
    pub message_id: String,
    pub finish_reason: String,
    /// P2-3: Token 用量信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// `chat:error` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatErrorPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub kind: String,
    pub message: String,
}

/// `chat:round-state` 事件 payload — W2.4 可观测性
#[derive(Clone, Serialize, Debug)]
pub struct ChatRoundStatePayload {
    pub conversation_id: String,
    pub round: u32,
    pub elapsed_ms: u64,
    pub tokens_prompt: u32,
    pub tokens_completion: u32,
    pub cached_tokens: u32,
    pub retry_count: u32,
}

/// `chat:retrying` 事件 payload — 通知前端正在重试
#[derive(Clone, Serialize)]
pub struct ChatRetryingPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub attempt: u32,
    pub max_attempts: u32,
    /// W2.6: 重试原因（如 "network_error" / "server_error_5xx"）
    pub reason: String,
}

// === P2-1 工具调用事件 payload ===

/// `chat:tool-call-start` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolCallStartPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
    pub name: String,
}

/// `chat:tool-call-delta` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolCallDeltaPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
    pub delta: String,
}

/// `chat:tool-call-end` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolCallEndPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
}

/// `chat:tool-result` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatToolResultPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

/// `chat:thinking` 事件 payload
#[derive(Clone, Serialize)]
pub struct ChatThinkingPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub content: String,
}

// === A2-3 工具授权事件 payload ===

/// `chat:tool-auth-request` 事件 payload (Rust → Frontend)
///
/// 当工具调用需要用户确认授权（例如路径不在白名单）时，Rust 侧 emit 此事件，
/// 携带工具名 / 待访问路径 / 参数 / 唯一 request_id，前端弹窗后用同一
/// `request_id` 响应。
///
/// - `request_id`     唯一标识，前后端匹配响应用
/// - `tool_use_id`    LLM 端的工具调用 ID（用于工具结果回填）
/// - `tool_name`      工具名
/// - `file_path`      待访问的路径（可能为空，例如 list_directory 也适用）
/// - `arguments`      工具调用参数 JSON 字符串（前端展示用）
/// - `conversation_id` / `message_id` 与其它 chat:* 事件保持一致，便于前端过滤
/// - `reason`         触发原因（前端展示文案）
#[derive(Clone, Serialize)]
pub struct ToolAuthRequestPayload {
    pub request_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub file_path: String,
    pub arguments: String,
    pub conversation_id: String,
    pub message_id: String,
    pub reason: String,
}

/// `chat:tool-auth-response` 事件 payload (Frontend → Rust)
///
/// 前端弹窗后通过此事件把用户选择告诉 Rust 侧。
/// Rust 侧在 `tool_executor` 里用 `request_id` 匹配 oneshot 通道，
/// 据此决定执行工具还是把工具结果写为「拒绝授权」。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolAuthResponse {
    pub request_id: String,
    pub allowed: bool,
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造 N 字节原始数据 → base64 字符串
    fn make_b64_bytes(n: usize) -> String {
        base64::engine::general_purpose::STANDARD.encode(vec![0u8; n])
    }

    // --- validate_images ---

    #[test]
    fn validate_images_empty_blocks_ok() {
        // 无图片 → 直接通过
        assert!(validate_images(&[]).is_ok());
        let blocks = vec![ContentBlock::text("纯文本")];
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_small_image_ok() {
        let blocks = vec![ContentBlock::image(make_b64_bytes(1024), "image/png")];
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_too_large_rejected() {
        // 6 MiB > 5 MiB 上限
        let big = make_b64_bytes(6 * 1024 * 1024);
        let blocks = vec![ContentBlock::image(big, "image/png")];
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(msg.contains("过大"), "错误信息应提示过大，实际: {}", msg);
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_exactly_5mb_ok() {
        // 5 MiB 边界值应放行
        let exact = make_b64_bytes(MAX_IMAGE_SIZE);
        let blocks = vec![ContentBlock::image(exact, "image/png")];
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_5mb_plus_one_rejected() {
        let over = make_b64_bytes(MAX_IMAGE_SIZE + 1);
        let blocks = vec![ContentBlock::image(over, "image/png")];
        assert!(validate_images(&blocks).is_err());
    }

    #[test]
    fn validate_images_unsupported_media_type_rejected() {
        let blocks = vec![ContentBlock::image(make_b64_bytes(100), "image/bmp")];
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(msg.contains("不支持"), "错误信息应提示不支持，实际: {}", msg);
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_invalid_base64_rejected() {
        let blocks = vec![ContentBlock::image("not_base64!@#$%", "image/png")];
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(
                    msg.contains("base64"),
                    "错误信息应提到 base64，实际: {}",
                    msg
                );
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_count_limit() {
        // 21 张 1KB 图片 → 超过 MAX_IMAGE_COUNT=20
        let blocks: Vec<ContentBlock> = (0..21)
            .map(|_| ContentBlock::image(make_b64_bytes(1024), "image/png"))
            .collect();
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(msg.contains("最多"), "错误信息应提到最多，实际: {}", msg);
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_exactly_max_count_ok() {
        // 恰好 20 张 → 应放行
        let blocks: Vec<ContentBlock> = (0..MAX_IMAGE_COUNT)
            .map(|_| ContentBlock::image(make_b64_bytes(1024), "image/png"))
            .collect();

        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_mixed_text_and_images_ok() {
        // 文本 + 多张图片混合
        let mut blocks = vec![ContentBlock::text("看这些图")];
        for _ in 0..3 {
            blocks.push(ContentBlock::image(make_b64_bytes(1024), "image/png"));
        }
        blocks.push(ContentBlock::text("请描述"));
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_supports_all_four_types() {
        for mt in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
            let blocks = vec![ContentBlock::image(make_b64_bytes(100), mt)];
            assert!(validate_images(&blocks).is_ok(), "{} 应被允许", mt);
        }
    }

    // --- SendMessageInput 序列化（确认前端 JSON 格式） ---

    #[test]
    fn send_input_accepts_legacy_content() {
        // 旧版 JSON（仅 content）应能反序列化
        let json = r#"{"conversation_id":"c1","content":"hello"}"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.conversation_id, "c1");
        assert_eq!(input.content.as_deref(), Some("hello"));
        assert!(input.content_blocks.is_none());
        assert!(!input.tools_enabled);
    }

    #[test]
    fn send_input_accepts_content_blocks() {
        // 新版 JSON（含 content_blocks）
        let json = r#"{
            "conversation_id": "c1",
            "content_blocks": [
                {"type": "text", "text": "看图"},
                {"type": "image", "data": "AAAA", "media_type": "image/png"}
            ],
            "tools_enabled": true
        }"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.conversation_id, "c1");
        assert!(input.content.is_none());
        let blocks = input.content_blocks.unwrap();
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            ContentBlock::Text { text } => assert_eq!(text, "看图"),
            _ => panic!("第一个应为 Text"),
        }
        match &blocks[1] {
            ContentBlock::Image { data, media_type } => {
                assert_eq!(data, "AAAA");
                assert_eq!(media_type, "image/png");
            }
            _ => panic!("第二个应为 Image"),
        }
        assert!(input.tools_enabled);
    }

    #[test]
    fn send_input_accepts_both_legacy_and_new() {
        // 同时传 content 和 content_blocks → 都应能反序列化
        // （后端逻辑会优先使用 content_blocks）
        let json = r#"{
            "conversation_id": "c1",
            "content": "legacy text",
            "content_blocks": [
                {"type": "text", "text": "new text"}
            ]
        }"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.content.as_deref(), Some("legacy text"));
        assert!(input.content_blocks.is_some());
    }

    #[test]
    fn send_input_minimal_required_fields() {
        // 仅 conversation_id + content_blocks → 其它字段默认值正确
        let json = r#"{"conversation_id":"c1","content_blocks":[]}"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.conversation_id, "c1");
        assert!(input.content.is_none());
        // 空数组 → Some(vec![])，后续逻辑会 fallback 到 legacy_content 校验
        let blocks = input.content_blocks.unwrap();
        assert!(blocks.is_empty());
        assert!(!input.tools_enabled);
        assert!(input.template.is_none());
    }

    // --- ContentBlock / 白名单（从 llm/mod.rs 迁入）---

    #[test]
    fn image_block_serde_roundtrip() {
        let block = ContentBlock::image("iVBORw0KGgo=", "image/png");
        let json = serde_json::to_string(&block).unwrap();
        // tag = "type", rename_all = "snake_case"
        assert_eq!(
            json,
            r#"{"type":"image","data":"iVBORw0KGgo=","media_type":"image/png"}"#
        );
        // 反序列化回原值
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        match back {
            ContentBlock::Image { data, media_type } => {
                assert_eq!(data, "iVBORw0KGgo=");
                assert_eq!(media_type, "image/png");
            }
            _ => panic!("反序列化后类型不对：{:?}", back),
        }
    }

    #[test]
    fn image_block_helper() {
        let b = ContentBlock::image("abc", "image/jpeg");
        assert!(b.is_image());
        assert!(b.as_text().is_none());
    }

    #[test]
    fn text_block_not_image() {
        let b = ContentBlock::text("hello");
        assert!(!b.is_image());
        assert_eq!(b.as_text(), Some("hello"));
    }

    #[test]
    fn supported_media_types_whitelist() {
        for mt in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
            assert!(
                is_supported_image_media_type(mt),
                "{} 应在白名单内",
                mt
            );
        }
        for mt in ["image/bmp", "image/svg+xml", "application/pdf", "", "png"] {
            assert!(
                !is_supported_image_media_type(mt),
                "{} 不应在白名单内",
                mt
            );
        }
    }

    #[test]
    fn join_text_skips_images() {
        // P2-2: join_text 只拼接 Text 块，忽略 Image/ToolUse 等
        let blocks = vec![
            ContentBlock::text("hello "),
            ContentBlock::image("xxxx", "image/png"),
            ContentBlock::text("world"),
        ];
        assert_eq!(ContentBlock::join_text(&blocks), "hello world");
    }

    /// 混合消息（含图片）的 JSON 序列化结构对齐前端 types/index.ts
    #[test]
    fn mixed_message_json_shape() {
        let blocks = vec![
            ContentBlock::text("看这张图"),
            ContentBlock::image("AAAA", "image/png"),
        ];
        let json = serde_json::to_string(&blocks).unwrap();
        // 验证 JSON 数组中两个对象的结构
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);

        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "看这张图");

        assert_eq!(arr[1]["type"], "image");
        assert_eq!(arr[1]["data"], "AAAA");
        assert_eq!(arr[1]["media_type"], "image/png");
        // Image 没有其他字段
        assert_eq!(arr[1].as_object().unwrap().len(), 3);
    }

    // --- A2-3 ToolAuthRequestPayload / ToolAuthResponse ---

    #[test]
    fn tool_auth_request_payload_serde() {
        use super::ToolAuthRequestPayload;
        let p = ToolAuthRequestPayload {
            request_id: "req-1".into(),
            tool_use_id: "tc-1".into(),
            tool_name: "read_file".into(),
            file_path: "/etc/passwd".into(),
            arguments: r#"{"path":"/etc/passwd"}"#.into(),
            conversation_id: "c-1".into(),
            message_id: "m-1".into(),
            reason: "路径 '/etc/passwd' 不在白名单中".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["request_id"], "req-1");
        assert_eq!(parsed["tool_use_id"], "tc-1");
        assert_eq!(parsed["tool_name"], "read_file");
        assert_eq!(parsed["file_path"], "/etc/passwd");
        assert_eq!(parsed["conversation_id"], "c-1");
        assert_eq!(parsed["message_id"], "m-1");
        // 8 个字段
        assert_eq!(parsed.as_object().unwrap().len(), 8);
    }

    #[test]
    fn tool_auth_response_serde_roundtrip() {
        use super::ToolAuthResponse;
        let r = ToolAuthResponse {
            request_id: "req-2".into(),
            allowed: true,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#"{"request_id":"req-2","allowed":true}"#);

        let back: ToolAuthResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.request_id, "req-2");
        assert!(back.allowed);

        // false 路径
        let r2 = ToolAuthResponse {
            request_id: "req-3".into(),
            allowed: false,
        };
        let json2 = serde_json::to_string(&r2).unwrap();
        let back2: ToolAuthResponse = serde_json::from_str(&json2).unwrap();
        assert!(!back2.allowed);
    }
}
