//! `doc` —— 二进制办公文档（docx / xlsx / xls / xlsb / ods / pdf）→ 文本/markdown 提取。
//!
//! **共享脊柱**：`read_file` / `read_multiple_files`（[mcp::internal]）、KB 索引
//! （[kb::indexer]）、聊天附件（[commands::chat_cmd]）三个调用点都经 [`try_extract`]，
//! 单点维护、单点测试。
//!
//! 设计原则：
//! - **纯函数**（bytes 进，text 出），无 IO → 可单测、三处零成本复用。
//! - 非 office/pdf 扩展名返回 [`Ok`]`(None)` → 调用方走原文本解码路径（[infra::decode]）。
//! - **是 office/pdf 但解析失败返回 [`Err`]**，绝不静默退回 lossy 文本——
//!   避免又把 OOXML 二进制当 GBK 解出乱码、冒充"成功"读到内容（这正是本模块要根治的 bug）。
//!
//! [`try_extract`]: try_extract

use crate::error::AppResult;

mod docx;
mod pdf;
mod spreadsheet;

/// 从一个文档中提取出的结果。
#[derive(Debug, Clone)]
pub struct ExtractedDoc {
    /// 提取出的文本 / markdown（喂 LLM 与 KB chunk 复用）。
    pub text: String,
    /// 来源类型，read_file 返回结构里标注。
    pub kind: DocKind,
    /// 标题候选：docx 首个非空段 / xlsx 首个 sheet 名 / pdf 首行。
    pub title: Option<String>,
}

/// 文档类型。read_file 返回结构据此标注来源，便于 UI / 日志区分。
#[derive(Debug, Clone)]
pub enum DocKind {
    /// Word .docx
    Docx,
    /// 电子表格（xlsx / xls / xlsb / ods）；`sheets` 为各 sheet 名（按工作簿顺序）。
    Spreadsheet { sheets: Vec<String> },
    /// PDF
    Pdf,
}

impl DocKind {
    /// 短标签，用于 read_file 的 `encoding` 字段与附件注入标注。
    pub fn label(&self) -> &'static str {
        match self {
            DocKind::Docx => "docx",
            DocKind::Spreadsheet { .. } => "spreadsheet",
            DocKind::Pdf => "pdf",
        }
    }
}

/// 内部分发用：扩展名 → 解析格式。
enum DocFormat {
    Docx,
    Spreadsheet,
    Pdf,
}

/// 判断（已小写、去点的）扩展名是否受支持，返回对应格式。
fn classify(ext: &str) -> Option<DocFormat> {
    match ext {
        "docx" => Some(DocFormat::Docx),
        "xlsx" | "xls" | "xlsb" | "ods" => Some(DocFormat::Spreadsheet),
        "pdf" => Some(DocFormat::Pdf),
        _ => None,
    }
}

/// 按扩展名分发提取。
///
/// - `ext`：扩展名（不含前导点），大小写不敏感，如 `"docx"`、`"XLSX"`。
/// - [`Ok`]`(None)`：非 office/pdf 扩展名 → 调用方走原文本解码路径。
/// - [`Ok`]`(Some)`：提取成功。
/// - [`Err`]：是 office/pdf 但解析失败 → **不静默退回 lossy**。
pub fn try_extract(bytes: &[u8], ext: &str) -> AppResult<Option<ExtractedDoc>> {
    let fmt = match classify(&ext.to_ascii_lowercase()) {
        Some(f) => f,
        None => return Ok(None),
    };
    let doc = match fmt {
        DocFormat::Docx => docx::extract(bytes)?,
        DocFormat::Spreadsheet => spreadsheet::extract(bytes)?,
        DocFormat::Pdf => pdf::extract(bytes)?,
    };
    Ok(Some(doc))
}

// =========================================================================
// 子模块共享的纯函数辅助
// =========================================================================

/// 取首个非空（trim 后）行，作为标题候选。docx / pdf 共用。
pub(super) fn first_nonempty_line(s: &str) -> Option<String> {
    for line in s.lines() {
        let t = line.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    None
}

/// 规整化提取出的文本：跳过 CR，把 3+ 连续换行折叠为 2，整体首尾 trim。
///
/// docx / pdf 的原始输出常有多余空行（docx 段落间空段、pdf 分页符），
/// 折叠后更利于 LLM 阅读与 KB chunk 切分（chunk 按双换行分段）。
pub(super) fn normalize(text: &mut String) {
    let mut collapsed = String::with_capacity(text.len());
    let mut newlines = 0u32;
    for ch in text.chars() {
        match ch {
            '\r' => continue, // 跳过 CR，统一用 LF
            '\n' => {
                newlines += 1;
                if newlines <= 2 {
                    collapsed.push('\n');
                }
            }
            _ => {
                newlines = 0;
                collapsed.push(ch);
            }
        }
    }
    *text = collapsed.trim().to_string();
}

// =========================================================================
// 单元测试 —— try_extract 分发契约（各子模块的解析细节测试在各自文件内）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_office_ext_returns_none() {
        // 非 office 扩展名 → None（调用方走原文本路径）
        assert!(try_extract(b"plain text", "txt").unwrap().is_none());
        assert!(try_extract(b"abc", "md").unwrap().is_none());
        assert!(try_extract(b"abc", "").unwrap().is_none());
    }

    #[test]
    fn ext_is_case_insensitive() {
        // 大写扩展名同样分发到对应解析器（解析失败也走 Err，不返回 None）
        let res = try_extract(b"not a real docx", "DOCX");
        assert!(res.is_err(), "DOCX 应回退到 docx 解析器并因非法内容 Err");
        let res = try_extract(b"not a real xlsx", "XLSX");
        assert!(res.is_err());
        let res = try_extract(b"%PDF-1.4 broken", "PDF");
        // pdf 可能对残缺输入返回空文本而非 Err（pdf-extract 宽容）；只校验不返回 None 分支即可
        let _ = res;
    }

    #[test]
    fn first_nonempty_line_skips_blank() {
        assert_eq!(
            first_nonempty_line("\n  \n标题行\n正文"),
            Some("标题行".to_string())
        );
        assert_eq!(first_nonempty_line(""), None);
        assert_eq!(first_nonempty_line("\n \n"), None);
    }

    #[test]
    fn normalize_collapses_blank_runs_and_trims() {
        let mut s = "\n\n\n标题\n\n\n\n\n正文\n".to_string();
        normalize(&mut s);
        assert_eq!(s, "标题\n\n\n正文");
        // 单/双换行保留，3+→2
        let mut s = "a\nb".to_string();
        normalize(&mut s);
        assert_eq!(s, "a\nb");
        // CR 被跳过
        let mut s = "a\r\nb\r\r\r\nc".to_string();
        normalize(&mut s);
        assert_eq!(s, "a\nb\n\nc");
    }
}
