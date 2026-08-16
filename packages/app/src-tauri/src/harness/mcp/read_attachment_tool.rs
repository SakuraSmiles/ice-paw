//! `read_attachment_page` 工具 —— 聊天附件分页按页读取（Phase A）。
//!
//! 配合 [`crate::commands::chat_cmd::materialize_file_blocks`] 的大附件分页：当附件提取
//! 正文总 token 超阈值时，只把首页注入 LLM，其余各块存 `message_attachments` 表，由本工具
//! 按 `(message_id, page)` 按需读取。治本大 PDF（>1M）整篇灌进单个 Text block 撑爆窗口。
//!
//! - **越权守卫**：`message_id` 必须属于当前会话（`ctx.conv_id`）。附件块按消息存，
//!   不带会话维度；不校验的话 agent 可读其它会话的附件正文。
//! - `authorization_level = Always`：读自己刚发的附件正文无需用户授权（内容来源是用户
//!   自己上传的文件，非任意文件系统路径）。
//! - `page` 1-based，对应 `message_attachments.idx + 1`。

use async_trait::async_trait;
use serde::Deserialize;

use crate::db::repo;
use crate::error::{AppError, AppResult};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

/// `read_attachment_page` 工具：读取某条用户消息已分页附件的指定页正文。
pub struct ReadAttachmentPageTool;

#[derive(Deserialize)]
struct ReadAttachmentPageArgs {
    /// 目标用户消息 ID（来自附件注入提示 `read_attachment_page(message_id="...")`）
    message_id: String,
    /// 1-based 页号（= message_attachments.idx + 1）
    page: i64,
}

#[async_trait]
impl McpClient for ReadAttachmentPageTool {
    fn name(&self) -> &str {
        "read_attachment_page"
    }

    fn description(&self) -> &str {
        "Read a specific page of a large attached document that was paginated on upload. \
         When a user attaches a large PDF/spreadsheet/docx, only the first pages are shown \
         inline and the rest are stored paginated; use this to fetch subsequent pages. \
         The inline note tells you the message_id and the valid page range. \
         page is 1-based. Returns the page's text, its label (e.g. '第3页' / 'Sheet:销售'), \
         total_pages, and has_next."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message_id": {
                    "type": "string",
                    "description": "The message_id given in the attachment's inline pagination note."
                },
                "page": {
                    "type": "integer",
                    "description": "1-based page number to read (within 1..=total_pages)."
                }
            },
            "required": ["message_id", "page"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        // 需 conv_id 上下文做越权守卫，走 execute_with_context。
        Err(AppError::Internal(
            "read_attachment_page 必须通过 execute_with_context 调用（需要 conv_id 上下文）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: ReadAttachmentPageArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("read_attachment_page 参数解析失败: {e}")))?;

        if parsed.page < 1 {
            return Err(AppError::Validation(format!(
                "page 必须 ≥ 1（1-based），收到 {}",
                parsed.page
            )));
        }

        // 越权守卫：消息必须属于当前会话。message_attachments 表不带会话维度，
        // 不校验则可读任意会话的附件正文。
        let msg_conv = repo::message::conversation_id(&ctx.pool, &parsed.message_id)
            .await?
            .ok_or_else(|| AppError::Validation("消息不存在或无分页附件".into()))?;
        if msg_conv != ctx.conv_id {
            return Err(AppError::Validation(
                "无权读取其它会话的附件（message_id 不属于当前会话）".into(),
            ));
        }

        let total =
            repo::message_attachment::count_by_message(&ctx.pool, &parsed.message_id).await?;
        if total == 0 {
            return Err(AppError::Validation(
                "该消息无分页附件（可能附件较小未分页，或已随消息删除）".into(),
            ));
        }
        let idx = parsed.page - 1; // 0-based
        let row = repo::message_attachment::get_page(&ctx.pool, &parsed.message_id, idx)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!("第 {} 页不存在（共 {} 页）", parsed.page, total))
            })?;

        Ok(serde_json::json!({
            "message_id": parsed.message_id,
            "page": parsed.page,
            "total_pages": total,
            "name": row.name,
            "label": row.label,
            "content": row.text,
            "has_next": parsed.page < total,
        })
        .to_string())
    }
}
