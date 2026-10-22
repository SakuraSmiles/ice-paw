//! view_image / show_image —— 本地图片双向通道（2026-10-22）
//!
//! 治「桌面共享权重过高」（用户实案：agent 看图片/文件效果经常想用屏幕捕获）：
//! - `view_image`：agent 视觉查看本地图片文件——Image 块回传走既有视觉适配链
//!   （视觉 agent 直读 / 非视觉平台链代读）；前端工具行同步渲染（用户顺带
//!   可见 agent 在看什么）
//! - `show_image`：把图片作为**对话内容**呈现给用户（前端在该轮 assistant
//!   组内渲染图片卡，复用消息预览器）；agent 同样收到 Image 块
//!   （「汇报效果图」场景——渲染产物/生成图/UI 稿）
//!
//! 两工具共享读图核心：魔数嗅探解码（image crate）→ 最长边 1600px 降采样
//! （对齐屏幕捕获降档梯，控 base64/token 成本）→ 统一 PNG 回传（ToolOutput
//! 仅支持 PNG；JPEG/GIF/WebP 转码，GIF 取首帧）。
//!
//! 授权 = [`AuthorizationLevel::PathWhitelist`]（与 read_file 同级：workspace
//! 内免弹、外路径走审批卡）——读图是文件读取语义，不是屏幕监控，不沾
//! 桌面共享通道治理。`path` 参数已在 tool_executor 的路径提取集内。

use async_trait::async_trait;
use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::harness::mcp::client::{McpClient, ToolContext, ToolOutput};
use crate::harness::mcp::types::AuthorizationLevel;

/// 图片文件大小上限（对齐附件链 MAX_IMAGE_BYTES 档）
const MAX_IMAGE_FILE_BYTES: u64 = 10 * 1024 * 1024;
/// 最长边降采样阈值（超过即缩到该值——对齐屏幕捕获降档梯）
const MAX_DIMENSION: u32 = 1600;

#[derive(serde::Deserialize)]
struct ImagePathArgs {
    path: String,
}

/// 读本地图片 → PNG 字节（含降采样）。返回 (png, width, height, 原格式)。
///
/// 重活（解码/缩放/编码）走 `spawn_blocking`——图片解码是同步 CPU 密集，
/// 0.6.4 性能批同款纪律（attachments/indexer/docx 三件先例）。
async fn read_image_as_png(path: &str) -> AppResult<(Vec<u8>, u32, u32, String)> {
    let meta = tokio::fs::metadata(path).await.map_err(|e| {
        // not-found 契约：挂近似候选（报错即行为契约）
        let hint =
            crate::harness::mcp::path_suggest::suggest_for_missing(std::path::Path::new(path));
        AppError::Io(std::io::Error::other(format!(
            "读取图片失败: {path}（{e}）。{hint}"
        )))
    })?;
    if !meta.is_file() {
        return Err(AppError::Validation(format!(
            "目标不是文件: {path}。view_image/show_image 只接受图片文件路径。"
        )));
    }
    if meta.len() > MAX_IMAGE_FILE_BYTES {
        return Err(AppError::Validation(format!(
            "图片过大: {} MB，超过 10MB 上限（{}）。请先用工具生成更小的版本。",
            meta.len() / (1024 * 1024),
            path
        )));
    }
    let bytes = tokio::fs::read(path).await.map_err(AppError::Io)?;
    let format = match image::guess_format(&bytes) {
        Ok(f) => format!("{f:?}"), // Debug 名（Png/Jpeg/Gif/WebP）
        Err(_) => {
            return Err(AppError::Validation(format!(
                "不是可识别的图片文件: {path}（PNG/JPEG/GIF/WebP 以外或已损坏）。文本文件请用 read_file。"
            )))
        }
    };

    let png = tokio::task::spawn_blocking(move || -> AppResult<Vec<u8>> {
        let img = image::load_from_memory(&bytes)
            .map_err(|e| AppError::Validation(format!("图片解码失败: {e}")))?;
        let img = if img.width().max(img.height()) > MAX_DIMENSION {
            // resize 适配进界内保持比例；调用侧已判「超界才缩」，无放大路径
            img.resize(
                MAX_DIMENSION,
                MAX_DIMENSION,
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            img
        };
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .map_err(|e| AppError::Validation(format!("PNG 编码失败: {e}")))?;
        Ok(buf)
    })
    .await
    .map_err(|e| AppError::Internal(format!("图片处理任务失败: {e}")))??;

    // 宽高取自解码后（降采样后的实际回传尺寸）
    let dims = image::load_from_memory(&png)
        .map(|i| (i.width(), i.height()))
        .unwrap_or((0, 0));
    Ok((png, dims.0, dims.1, format))
}

/// 组装两工具共用的回传（图 + JSON 摘要）
async fn run_image_tool(
    args: &str,
    tool_name: &str,
    note: &str,
) -> AppResult<ToolOutput> {
    let parsed: ImagePathArgs = serde_json::from_str(args)
        .map_err(|e| AppError::Validation(format!("{tool_name} 参数解析失败: {e}")))?;
    if parsed.path.trim().is_empty() {
        return Err(AppError::Validation("path 不能为空".into()));
    }
    let (png, w, h, format) = read_image_as_png(parsed.path.trim()).await?;
    tracing::info!(
        target: "ice_paw.image_tools",
        tool = tool_name,
        path = %parsed.path,
        format = %format,
        dims = %format!("{w}x{h}"),
        png_bytes = png.len(),
        "图片读取成功"
    );
    let summary = json!({
        "path": parsed.path.trim(),
        "format": format,
        "width": w,
        "height": h,
        "note": note,
    })
    .to_string();
    Ok(ToolOutput::with_image(summary, png))
}

// =========================================================================
// view_image —— agent 视觉查看本地图片
// =========================================================================

pub struct ViewImageTool;

#[async_trait]
impl McpClient for ViewImageTool {
    fn name(&self) -> &str {
        "view_image"
    }

    fn description(&self) -> &str {
        "View a local image FILE (PNG/JPEG/GIF/WebP) by path and inspect it visually. \
         Prefer this over screen capture when the target is an image file: rendered \
         exports, downloaded pictures, generated images, saved screenshots. Returns \
         the image as an image block you can see directly (downscaled to max 1600px)."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Absolute path to the image file." }
            },
            "required": ["path"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "view_image 必须通过 execute_with_output 调用（回传图片）".into(),
        ))
    }

    async fn execute_with_output(&self, args: &str, _ctx: &ToolContext) -> AppResult<ToolOutput> {
        run_image_tool(
            args,
            "view_image",
            "图片已作为图像块回传，可直接查看",
        )
        .await
    }
}

// =========================================================================
// show_image —— 把图片作为对话内容呈现给用户
// =========================================================================

pub struct ShowImageTool;

#[async_trait]
impl McpClient for ShowImageTool {
    fn name(&self) -> &str {
        "show_image"
    }

    fn description(&self) -> &str {
        "Display a local image file to the user DIRECTLY IN THE CHAT as an image card \
         in your message. Use when reporting visual results — rendered documents, \
         generated images, UI mockups, effect previews — so the user can view them \
         without opening files. The image is also returned to you as an image block."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Absolute path to the image file to show." }
            },
            "required": ["path"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "show_image 必须通过 execute_with_output 调用（回传图片）".into(),
        ))
    }

    async fn execute_with_output(&self, args: &str, _ctx: &ToolContext) -> AppResult<ToolOutput> {
        run_image_tool(
            args,
            "show_image",
            "图片已展示给用户（对话内容内图片卡），同时回传给你",
        )
        .await
    }
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 1×1 红 PNG（最小合法图）
    fn tiny_png() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(3, 3, image::Rgb([200, 40, 40]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    #[tokio::test]
    async fn view_image_roundtrip_png() {
        let dir = std::env::temp_dir().join(format!("icepaw_vi_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("dot.png");
        std::fs::write(&p, tiny_png()).unwrap();

        let args = json!({ "path": p.to_string_lossy() }).to_string();
        let out = run_image_tool(&args, "view_image", "test")
            .await
            .unwrap();
        assert!(out.image_png.is_some(), "PNG 原样回传");
        let v: serde_json::Value = serde_json::from_str(&out.text).unwrap();
        assert_eq!(v["width"], 3);
        assert_eq!(v["format"], "Png");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn rejects_non_image_with_family_error() {
        let dir = std::env::temp_dir().join(format!("icepaw_vi2_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("note.txt");
        std::fs::write(&p, "plain text").unwrap();

        let args = json!({ "path": p.to_string_lossy() }).to_string();
        let err = run_image_tool(&args, "view_image", "test")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("不是可识别的图片文件"), "实际: {err}");
        assert!(err.contains("read_file"), "指路 read_file: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn missing_file_error_has_hint() {
        let args = json!({ "path": "D:/definitely/not/exist/pic.png" }).to_string();
        let err = run_image_tool(&args, "show_image", "test")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("读取图片失败"), "家族前缀稳定: {err}");
    }
}
