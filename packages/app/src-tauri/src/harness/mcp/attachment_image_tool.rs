//! `view_attachment_image` 工具 —— 扫描件 / 图片型附件的视觉读取（Phase B）。
//!
//! [`crate::harness::mcp::read_attachment_tool`] 读的是**提取后的文本**；但当附件是
//! 扫描件 / 纯图片 PDF（文本提取为空，`total_tokens == 0`），文本路径无能为力。此时
//! [`crate::commands::chat_cmd::materialize_file_blocks`] 会把原始字节存进
//! `message_attachment_files` 表（B.1），本工具取字节 → pdfium 渲染指定页 → PNG →
//! [`ToolOutput::image_png`] → `tool_executor` 注入 `Image` 块给视觉模型读图。
//!
//! - **越权守卫**：与 `read_attachment_page` 同——`message_id` 必须属于当前会话
//!   （附件字节按消息存，不带会话维度；不校验则可读任意会话附件）。
//! - `authorization_level = Always`：读用户自己刚上传的附件渲染图，非任意文件系统路径。
//! - `page` 1-based。
//! - **仅 PDF**：当前只 pdfium 渲染 PDF。Office 文档（docx/xlsx）几乎总有文本层、
//!   极少落到"空提取"分支；且无等价的轻量"渲染成图"路径——遇到非 PDF 如实告知。
//! - 覆盖 `execute_with_output`（而非 `execute_with_context`）以回传图片字节。
//!
//! 线程模型：pdfium 的 `Pdfium` 是 `!Send`，独占于专用渲染线程（见
//! [`crate::harness::doc::pdf_render`]）；本工具用 `spawn_blocking` 把阻塞的通道往返
//! 挪出 async runtime，避免卡住其它异步任务。

use async_trait::async_trait;
use serde::Deserialize;

use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::doc::{page_count, render_page_to_png};

use super::client::{McpClient, ToolContext, ToolOutput};
use super::types::AuthorizationLevel;

/// `view_attachment_image` 工具：把扫描件 / 图片型 PDF 的指定页渲染成图，发给视觉模型。
pub struct ViewAttachmentImageTool;

#[derive(Deserialize)]
struct ViewAttachmentImageArgs {
    /// 目标用户消息 ID（来自附件注入提示 `view_attachment_image(message_id="...")`）。
    message_id: String,
    /// 1-based 页号。
    page: i64,
}

/// 在独立阻塞线程跑 pdfium 任务，统一 join 错误。
///
/// pdfium 通道往返是阻塞的（`recv` 等渲染线程），不能直接在 async 上下文里调；
/// `spawn_blocking` 挪到阻塞线程池，join 失败（panic/取消）归一为 `AppError::Internal`。
async fn run_pdf<F, T>(f: F) -> AppResult<T>
where
    F: FnOnce() -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(format!("pdfium 渲染任务 join 失败: {e}")))?
}

#[async_trait]
impl McpClient for ViewAttachmentImageTool {
    fn name(&self) -> &str {
        "view_attachment_image"
    }

    fn description(&self) -> &str {
        "Render a page of an attached SCANNED or image-only PDF as an image and pass it to your \
         vision capability, so you can actually SEE the page when text extraction returned nothing. \
         Use this ONLY when an attachment's inline note says it could not be read (empty extraction, \
         likely a scan/image-only PDF) AND the file is a PDF. page is 1-based. Returns a short JSON \
         summary (page/total_pages/name) plus the rendered page image attached alongside. Call \
         repeatedly with the next page to page through the document."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message_id": {
                    "type": "string",
                    "description": "The message_id from the attachment's inline 'could not be read' note."
                },
                "page": {
                    "type": "integer",
                    "description": "1-based page number to render."
                }
            },
            "required": ["message_id", "page"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        // 需 conv_id 上下文做越权守卫 + 回传图片，走 execute_with_output。
        Err(AppError::Internal(
            "view_attachment_image 必须通过 execute_with_output 调用（需要 conv_id + 回传图片）".into(),
        ))
    }

    /// 覆盖 rich 方法：越权守卫 → 取字节 → pdfium 渲染 → 图片 + JSON 摘要。
    async fn execute_with_output(
        &self,
        args: &str,
        ctx: &ToolContext,
    ) -> AppResult<ToolOutput> {
        let parsed: ViewAttachmentImageArgs = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("view_attachment_image 参数解析失败: {e}"))
        })?;
        if parsed.page < 1 {
            return Err(AppError::Validation(format!(
                "page 必须 ≥ 1（1-based），收到 {}",
                parsed.page
            )));
        }

        // 越权守卫：消息必须属于当前会话。message_attachment_files 不带会话维度，
        // 不校验则可读任意会话的附件字节。
        let msg_conv = repo::message::conversation_id(&ctx.pool, &parsed.message_id)
            .await?
            .ok_or_else(|| AppError::Validation("消息不存在或无可视化附件".into()))?;
        if msg_conv != ctx.conv_id {
            return Err(AppError::Validation(
                "无权读取其它会话的附件（message_id 不属于当前会话）".into(),
            ));
        }

        // 取该消息首个视觉候选文件的原始字节（B.1：空提取时存进 message_attachment_files）。
        // v1 默认单文件场景；多文件时取 idx 最小者（get_first_by_message）。
        let row = repo::message_attachment_file::get_first_by_message(&ctx.pool, &parsed.message_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(
                    "该消息无可视化附件字节（可能附件并非扫描件、或已随消息删除）".into(),
                )
            })?;

        let ext = row.ext.to_lowercase();
        if ext != "pdf" {
            return Err(AppError::Validation(format!(
                "view_attachment_image 当前仅支持 PDF，收到 .{ext}。\
                 Office 文档请让用户复制相关文本贴入对话。"
            )));
        }

        // 先取总页数（校验 page 上界 + 写进摘要让 agent 知道文档规模）。
        let pdf_bytes_for_count = row.bytes.clone();
        let total_pages = run_pdf(move || page_count(&pdf_bytes_for_count)).await?;

        if parsed.page as u16 > total_pages {
            return Err(AppError::Validation(format!(
                "第 {} 页不存在（共 {} 页）",
                parsed.page, total_pages
            )));
        }

        // 渲染指定页 → PNG。page 已校验，render 内仍会再查一次边界（防御）。
        let page = parsed.page as usize;
        let pdf_bytes_for_render = row.bytes.clone();
        let png: Vec<u8> =
            run_pdf(move || render_page_to_png(&pdf_bytes_for_render, page)).await?;

        tracing::info!(
            target: "ice_paw.attach",
            name = %row.name,
            page = parsed.page,
            total_pages,
            png_bytes = png.len(),
            "view_attachment_image 渲染成功"
        );

        let summary = serde_json::json!({
            "message_id": parsed.message_id,
            "page": parsed.page,
            "total_pages": total_pages,
            "name": row.name,
            "note": "Rendered page image attached alongside. Use your vision to read it. \
                     Call again with page+1 to continue if you need more pages."
        });
        Ok(ToolOutput::with_image(summary.to_string(), png))
    }
}
