//! 图片校验 — 从 `protocol.rs` 提取出的纯逻辑函数
//!
//! 包含支持格式白名单、尺寸/张数上限、base64 合法性校验。
//! 在 `send_message` 入口处调用，先于任何 DB 写入或 LLM 调用。

use crate::error::{AppError, AppResult};
use crate::infra::protocol::ContentBlock;
use base64::Engine as _;

/// 支持的图片 MIME 类型白名单。
/// 与前端 `ImagePicker.vue` 的 `accept` 属性保持一致。
/// Anthropic 支持 `image/jpeg | image/png | image/gif | image/webp`；
/// OpenAI Vision 支持同等集合。
pub const SUPPORTED_IMAGE_MEDIA_TYPES: &[&str] =
    &["image/png", "image/jpeg", "image/gif", "image/webp"];

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
                    AppError::Validation(format!("第 {} 张图片 base64 解码失败：{}", idx + 1, e))
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

/// 剥离空图片块（0 字节），替换为诚实提示文本块。
///
/// **为什么需要**：0 字节图片（`data` 为空字符串、或 base64 解码后为空）能通过
/// [`validate_images`]（尺寸 0 ≤ 5MB、类型/base64 合法性均过），但发给 LLM 会被以
/// 400「Invalid image」拒绝——与 0 字节文档附件同源的「单个坏附件拖死整条消息」问题。
///
/// **软失败策略**（与文档附件 `materialize_file_blocks` 的软失败一致）：
/// - 空图片块：移除 + 注入一条 `[系统提示]` 文本块如实告知「第 N 张为空（0 字节），已跳过」，
///   **绝不阻塞整条消息**（其余图片 / 文本 / 附件照常发送）；
/// - base64 非法的图片块：同样剥离（坏块，发送必失败）；
/// - 非空图片块：原样保留。
///
/// 仅作用于**当前用户消息**（`send_message` 的 `final_blocks`）；历史图片块由
/// `ModalCapabilityStage` 另行处理（视觉模型原样过 / 非视觉剥为 marker）。
pub fn strip_empty_image_blocks(blocks: Vec<ContentBlock>) -> Vec<ContentBlock> {
    let mut out: Vec<ContentBlock> = Vec::with_capacity(blocks.len());
    let mut image_idx = 0usize;
    let mut dropped: Vec<usize> = Vec::new();

    for b in blocks {
        if let ContentBlock::Image { data, .. } = &b {
            image_idx += 1;
            // 解码失败（非法 base64）也视作坏块 → 剥离（发送必失败）。
            let is_empty_or_bad = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map(|d| d.is_empty())
                .unwrap_or(true);
            if is_empty_or_bad {
                dropped.push(image_idx);
                continue;
            }
        }
        out.push(b);
    }

    if dropped.is_empty() {
        return out;
    }

    let hint = if dropped.len() == 1 {
        format!(
            "第 {} 张图片为空（0 字节），已自动跳过、未发送给模型。",
            dropped[0]
        )
    } else {
        let list = dropped
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("、");
        format!("第 {list} 张图片为空（0 字节），已自动跳过、未发送给模型。")
    };
    out.push(ContentBlock::text(format!(
        "[系统提示：{hint} 可能是文件损坏或为空，请检查后重新上传。]"
    )));
    tracing::warn!(
        target: "ice_paw.attach",
        which = ?dropped,
        "剥离空图片块（0 字节），替换为诚实提示（不阻塞整条消息）"
    );
    out
}

// =========================================================================
// 单元测试（从 protocol.rs 迁入——测的就是本模块的函数）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::protocol::ContentBlock;

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
                assert!(
                    msg.contains("不支持"),
                    "错误信息应提示不支持，实际: {}",
                    msg
                );
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

    // --- strip_empty_image_blocks（0 字节图片软剥离）---

    #[test]
    fn strip_keeps_nonempty_image() {
        // 非空图片（1KB）→ 原样保留，无提示注入
        let blocks = vec![
            ContentBlock::text("看图"),
            ContentBlock::image(make_b64_bytes(1024), "image/png"),
        ];
        let out = strip_empty_image_blocks(blocks);
        assert_eq!(out.len(), 2, "非空图片应保留，无额外提示");
        assert!(out[1].is_image());
    }

    #[test]
    fn strip_removes_empty_image_and_injects_hint() {
        // 0 字节图片（data 为空）→ 剥离 + 注入诚实提示
        let blocks = vec![
            ContentBlock::image(String::new(), "image/png"),
            ContentBlock::text("正文"),
        ];
        let out = strip_empty_image_blocks(blocks);
        // 期望：空图被移除，正文保留，末尾追加 1 条提示
        assert_eq!(out.len(), 2, "空图剥离后应为 正文 + 提示");
        assert_eq!(out[0].as_text(), Some("正文"));
        let hint = out[1].as_text().expect("末尾应追加提示");
        assert!(hint.contains("0 字节"), "提示应说明 0 字节，实际: {hint}");
        assert!(out.iter().all(|b| !b.is_image()), "不应残留任何图片块");
    }

    #[test]
    fn strip_keeps_valid_when_mixed_with_empty() {
        // 一空一有效：空图剥离、有效图保留、提示点名「第 1 张」
        let blocks = vec![
            ContentBlock::image(String::new(), "image/png"), // 第 1 张：空
            ContentBlock::image(make_b64_bytes(512), "image/jpeg"), // 第 2 张：有效
        ];
        let out = strip_empty_image_blocks(blocks);
        let images: Vec<_> = out.iter().filter(|b| b.is_image()).collect();
        assert_eq!(images.len(), 1, "仅保留 1 张有效图");
        let hint = out.iter().find_map(|b| b.as_text()).expect("应有提示");
        assert!(hint.contains("第 1 张"), "应点名第 1 张为空，实际: {hint}");
    }

    #[test]
    fn strip_no_image_unchanged() {
        // 纯文本 / 无图 → 原样返回、无提示
        let blocks = vec![ContentBlock::text("只有文字")];
        let out = strip_empty_image_blocks(blocks);
        assert_eq!(out.len(), 1);
        assert!(out[0].as_text().is_some());
    }

    #[test]
    fn strip_invalid_base64_treated_as_empty() {
        // 非法 base64（解码失败）→ 视作坏块剥离（发送必失败）
        let blocks = vec![ContentBlock::image("not_base64!@#$%", "image/png")];
        let out = strip_empty_image_blocks(blocks);
        assert!(
            out.iter().all(|b| !b.is_image()),
            "非法 base64 图片应被剥离"
        );
        assert!(out.iter().any(|b| b.as_text().is_some()), "应注入提示");
    }

    // --- 白名单 ---

    #[test]
    fn supported_media_types_whitelist() {
        for mt in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
            assert!(is_supported_image_media_type(mt), "{} 应在白名单内", mt);
        }
        for mt in ["image/bmp", "image/svg+xml", "application/pdf", "", "png"] {
            assert!(!is_supported_image_media_type(mt), "{} 不应在白名单内", mt);
        }
    }
}
