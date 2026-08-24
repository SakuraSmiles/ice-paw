//! `inspect_docx` 工具（word-capability-roadmap 步骤 2 / S0b）。
//!
//! read_file 对 docx 只给线性文本（结构全丢）；本工具在结构模型上出三档投影
//! （outline / format / text，见 harness::doc::docx_inspect），块号 1-based 混排
//! 统一编号——这是后续 edit_docx 的地址地基。
//!
//! 薄壳职责：读文件 + 扩展名守卫 + 参数解析；投影逻辑全在 docx_inspect（纯函数，
//! 独立单测）。错误契约三段式：not-found 挂 did-you-mean；非 docx 指向 read_file。

use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::harness::doc::{inspect_document, InspectProjection, InspectRequest};

use super::client::McpClient;
use super::types::AuthorizationLevel;

pub struct InspectDocxTool;

#[derive(Deserialize)]
struct InspectDocxArgs {
    path: String,
    /// outline（默认，全图）/ format（run 级格式）/ text（带块号正文）
    #[serde(default)]
    projection: Option<String>,
    /// 起始块号（1-based，含）
    #[serde(default)]
    start: Option<usize>,
    /// 结束块号（1-based，含）
    #[serde(default)]
    end: Option<usize>,
}

#[derive(Serialize)]
struct InspectDocxResult {
    path: String,
    projection: &'static str,
    total_blocks: usize,
    /// [start, end]（1-based 含端点）
    range: (usize, usize),
    has_more: bool,
    /// has_more=true 时带续读提示的 next_start
    next_start: Option<usize>,
    content: String,
}

#[async_trait]
impl McpClient for InspectDocxTool {
    fn name(&self) -> &str {
        "inspect_docx"
    }

    fn description(&self) -> &str {
        "Inspect the structure of a Word .docx document at three levels of detail. \
         projection=outline (default): one line per block — block number, style/heading \
         level, text summary; the map of the whole document. projection=format: run-level \
         effective formatting (font size/weight/color, fonts, alignment, line spacing, \
         indents after style-chain resolution) plus table grids for a block range. \
         projection=text: document text with block-number prefixes. Blocks are numbered \
         1-based in document order (paragraphs and tables together); these numbers are \
         the addressing scheme used to reference locations for editing. Workflow: outline \
         first to locate blocks, then format/text with start/end for details. Defaults: \
         outline renders up to 400 blocks, format up to 50, text up to 100."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the .docx file to inspect."
                },
                "projection": {
                    "type": "string",
                    "enum": ["outline", "format", "text"],
                    "description": "Level of detail: outline (block map, default), format (run-level formatting), text (block-numbered text)."
                },
                "start": {
                    "type": "integer",
                    "description": "First block number to render (1-based, inclusive). Default 1."
                },
                "end": {
                    "type": "integer",
                    "description": "Last block number to render (1-based, inclusive). Default: start + span - 1 (span depends on projection)."
                }
            },
            "required": ["path"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: InspectDocxArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("inspect_docx 参数解析失败: {e}")))?;

        let projection = match parsed.projection.as_deref() {
            None | Some("outline") => InspectProjection::Outline,
            Some("format") => InspectProjection::Format,
            Some("text") => InspectProjection::Text,
            Some(other) => {
                return Err(AppError::Validation(format!(
                    "未知投影档位: {other}。支持 outline（块级地图，默认）/ format（run 级格式）/ text（带块号正文）。"
                )));
            }
        };

        let path = Path::new(&parsed.path);
        let canonical = match path.canonicalize() {
            Ok(c) => c,
            // 报错即行为契约：not-found 扫真实文件系统给近似候选（did-you-mean）
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AppError::Validation(format!(
                    "文件不存在: {}。{}",
                    parsed.path,
                    super::path_suggest::suggest_for_missing(path)
                )));
            }
            Err(e) => return Err(AppError::Validation(format!("文件路径无效: {e}"))),
        };

        let ext = canonical
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "docx" {
            return Err(AppError::Validation(format!(
                "不是 Word 文档: 扩展名 .{ext}。inspect_docx 只支持 .docx；其他文件请用 read_file。"
            )));
        }

        let bytes = tokio::fs::read(&canonical)
            .await
            .map_err(|e| AppError::Io(std::io::Error::other(format!("读取文件失败: {e}"))))?;

        let report = inspect_document(
            &bytes,
            &InspectRequest { projection, start: parsed.start, end: parsed.end },
        )?;

        let result = InspectDocxResult {
            path: parsed.path,
            projection: projection.as_str(),
            total_blocks: report.total_blocks,
            range: report.range,
            has_more: report.has_more,
            next_start: if report.has_more { Some(report.range.1 + 1) } else { None },
            content: report.content,
        };
        Ok(serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()))
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// docx-rs 造最小真实包（zip 容器 + document.xml）。
    fn docx_bytes() -> Vec<u8> {
        use docx_rs::{Docx, Document, Paragraph, Run};
        let document =
            Document::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text("正文段")));
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        Docx::new().document(document).build().pack(&mut cursor).unwrap();
        cursor.into_inner()
    }

    #[tokio::test]
    async fn happy_path_outline() {
        let dir = std::env::temp_dir().join("icepaw_inspect_docx_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("样本.docx");
        std::fs::write(&file, docx_bytes()).unwrap();

        let tool = InspectDocxTool;
        let args = serde_json::json!({ "path": file.to_string_lossy() }).to_string();
        let out = tool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["projection"], "outline");
        assert_eq!(v["total_blocks"], 1);
        assert!(v["content"].as_str().unwrap().contains("正文段"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn rejects_non_docx_and_bad_projection() {
        let tool = InspectDocxTool;
        // 非 docx 扩展名
        let dir = std::env::temp_dir().join("icepaw_inspect_docx_test2");
        std::fs::create_dir_all(&dir).unwrap();
        let txt = dir.join("note.txt");
        std::fs::write(&txt, b"hello").unwrap();
        let args = serde_json::json!({ "path": txt.to_string_lossy() }).to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("read_file"), "应指向 read_file: {err}");
        // 未知投影
        let args = serde_json::json!({
            "path": txt.to_string_lossy(),
            "projection": "deep"
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("未知投影档位"), "实际: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn missing_file_gives_suggestions() {
        let tool = InspectDocxTool;
        let args = serde_json::json!({ "path": "Z:/不存在的/文档.docx" }).to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("文件不存在"), "实际: {err}");
    }
}
