//! 图片校验 — 从 `protocol.rs` 提取出的纯逻辑函数
//!
//! 包含支持格式白名单、尺寸/张数上限、base64 合法性校验。
//! 在 `send_message` 入口处调用，先于任何 DB 写入或 LLM 调用。

use base64::Engine as _;
use crate::error::{AppError, AppResult};
use crate::infra::protocol::ContentBlock;

/// 支持的图片 MIME 类型白名单。
/// 与前端 `ImagePicker.vue` 的 `accept` 属性保持一致。
/// Anthropic 支持 `image/jpeg | image/png | image/gif | image/webp`；
/// OpenAI Vision 支持同等集合。
pub const SUPPORTED_IMAGE_MEDIA_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
];

/// 校验 media_type 是否在白名单内
pub fn is_supported_image_media_type(mt: &str) -> bool {
    SUPPORTED_IMAGE_MEDIA_TYPES.contains(&mt)
}

/// 单张图片的最大字节数（base64 解码后的原始字节大小）
///
/// 5MB 限制与 OpenAI / Anthropic 官方建议接近：
/// - OpenAI Vision: 单图 base64 ≤ ~20MB，但实践中 5MB 内体验最佳
/// - Anthropic: 单图 ≤ 5MB（推荐），超过会被服务端拒绝
///
/// 用 base64 解码后的字节数校验（不是 base64 字符串长度）。
pub const MAX_IMAGE_SIZE: usize = 5 * 1024 * 1024; // 5 MiB

/// 单条消息最多图片张数。
/// OpenAI 文档建议 ≤ 20 张/请求，这里统一用 20。
pub const MAX_IMAGE_COUNT: usize = 20;

/// 校验 content_blocks 中的图片（含尺寸 / 张数 / 类型 / base64 合法性）。
///
/// 在 `send_message` 入口处调用，**先于**任何 DB 写入或 LLM 调用。
///
/// 错误信息直接返回给前端用于 toast 提示（使用 `AppError::Validation`
/// → 前端 kind=`"validation"`，可识别为业务级错误）。
pub fn validate_images(blocks: &[ContentBlock]) -> AppResult<()> {
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
