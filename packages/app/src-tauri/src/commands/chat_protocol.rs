//! Chat 协议定义：入参结构 / 事件 Payload / 图片校验常量
//!
//! 纯数据层，不含业务逻辑。
//! - `SendMessageInput` / `TemplateInput`：前端 → 后端的入参
//! - 11 个 `ChatXxxPayload`：后端 → 前端的事件结构
//! - `validate_images`：图片校验（尺寸/张数/类型/base64）
//! - `MAX_IMAGE_SIZE` / `MAX_IMAGE_COUNT`：校验常量

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::llm::{is_supported_image_media_type, ContentBlock};

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

use crate::llm;

/// `chat:start` 事件 payload
#[derive(Clone, Serialize)]
pub(crate) struct ChatStartPayload {
    pub conversation_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
}

/// `chat:chunk` 事件 payload
#[derive(Clone, Serialize)]
pub(crate) struct ChatChunkPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub delta: String,
}

/// `chat:done` 事件 payload
#[derive(Clone, Serialize)]
pub(crate) struct ChatDonePayload {
    pub conversation_id: String,
    pub message_id: String,
    pub finish_reason: String,
    /// P2-3: Token 用量信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<llm::TokenUsage>,
}

/// `chat:error` 事件 payload
#[derive(Clone, Serialize)]
pub(crate) struct ChatErrorPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub kind: String,
    pub message: String,
}

/// `chat:retrying` 事件 payload — 通知前端正在重试
#[derive(Clone, Serialize)]
pub(crate) struct ChatRetryingPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub attempt: u32,
    pub max_attempts: u32,
}

// === P2-1 工具调用事件 payload ===

/// `chat:tool-call-start` 事件 payload
#[derive(Clone, Serialize)]
pub(crate) struct ChatToolCallStartPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
    pub name: String,
}

/// `chat:tool-call-delta` 事件 payload
#[derive(Clone, Serialize)]
pub(crate) struct ChatToolCallDeltaPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
    pub delta: String,
}

/// `chat:tool-call-end` 事件 payload
#[derive(Clone, Serialize)]
pub(crate) struct ChatToolCallEndPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub id: String,
}

/// `chat:tool-result` 事件 payload
#[derive(Clone, Serialize)]
pub(crate) struct ChatToolResultPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

/// `chat:thinking` 事件 payload
#[derive(Clone, Serialize)]
pub(crate) struct ChatThinkingPayload {
    pub conversation_id: String,
    pub message_id: String,
    pub content: String,
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ContentBlock;

    /// 构造 N 字节原始数据 → base64 字符串
    fn make_b64_bytes(n: usize) -> String {
        base64::engine::general_purpose::STANDARD.encode(vec![0u8; n])
    }

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
}
