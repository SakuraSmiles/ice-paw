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

use crate::context::token::estimate_tokens;
use crate::error::AppResult;

mod docx;
mod docx_edit;
mod docx_inspect;
mod docx_model;
mod numbering;
mod pdf;
mod pdf_render;
mod spreadsheet;
mod styles;
mod xml_dom;

// 真实语料冒烟（word-capability-roadmap 步骤 0；语料仅本地 tests/fixtures/docx/，
// 🚫不入库（用户禁令，gitignore 排除）——运行时读取，缺失自动 skip）
#[cfg(test)]
mod corpus_tests;

// Phase B 视觉路径：PDF 页面 → PNG（扫描件/图片型 PDF 读图用）。
pub use pdf_render::{page_count, render_page_to_png};

// inspect_docx 三级投影（S0b）：mcp::docx_tool 薄壳消费。
pub use docx_inspect::{inspect_document, InspectProjection, InspectReport, InspectRequest};

// edit_docx 块级手术（步骤 3）：批量事务，纯函数；IO/授权/写盘在 mcp::docx_tool。
pub use docx_edit::{apply_edits_to_bytes, AppliedOp, EditOp};

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

/// 一个提取「块」——聊天附件分页的翻页单位。
///
/// - PDF：一页一块（按 pdf-extract 的 form feed `\u{c}` 边界切），label `第N页`
/// - 电子表格：一个 sheet 一块，label `Sheet:{name}`
/// - docx：按 token 预算切段（段落边界），label `第N段`
#[derive(Debug, Clone)]
pub struct TextChunk {
    /// 该块的正文
    pub text: String,
    /// 人类可读的块标签（注入 LLM 的分隔头 / read_attachment_page 返回里展示）
    pub label: String,
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

/// 按扩展名分发「分块提取」——聊天附件大文件分页路径专用。
///
/// 返回 `(DocKind, chunks)`；`chunks.len()` 即总页数（`read_attachment_page` 的
/// `total_pages`）。非 office/pdf 扩展名返回 [`Ok`]`(None)`（调用方不分页）。
///
/// 与 [`try_extract`] 的关系：`try_extract` 给整篇（KB 索引 / read_file 复用，
/// KB 行为零改）；`try_extract_chunks` 给分块（仅聊天附件大文件）。两者独立，
/// 不共享中间结果——附件路径要么分页要么不分页，不会同时走两条。
pub fn try_extract_chunks(bytes: &[u8], ext: &str) -> AppResult<Option<(DocKind, Vec<TextChunk>)>> {
    let fmt = match classify(&ext.to_ascii_lowercase()) {
        Some(f) => f,
        None => return Ok(None),
    };
    let (kind, chunks) = match fmt {
        DocFormat::Pdf => (DocKind::Pdf, pdf::extract_chunks(bytes)?),
        DocFormat::Spreadsheet => spreadsheet::extract_chunks(bytes)?,
        DocFormat::Docx => {
            let doc = docx::extract(bytes)?;
            (DocKind::Docx, split_text_into_chunks(&doc.text, "段"))
        }
    };
    Ok(Some((kind, chunks)))
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

/// docx 分块的目标 token 预算（每块约 ≤2000 token，足够 LLM 一屏精读又不浪费翻页）。
const DOCX_CHUNK_TARGET_TOKENS: usize = 2000;

/// 把整篇文本按 token 预算切成多块（docx 分块用）。
///
/// 在 `\n\n`（段落）边界贪心装箱：累加段落 [`estimate_tokens`]，超
/// [`DOCX_CHUNK_TARGET_TOKENS`] 即封块。单段本身超预算（normalize 后罕见）→
/// 独立成一块（不硬切字符，宁大勿碎）。空文本兜底返回 1 个空块。
///
/// `unit` 进 label：`第N{unit}`（docx 传 `"段"`）。
pub(super) fn split_text_into_chunks(text: &str, unit: &str) -> Vec<TextChunk> {
    let target = DOCX_CHUNK_TARGET_TOKENS;
    let mut chunks: Vec<TextChunk> = Vec::new();
    let mut buf = String::new();
    let mut buf_tokens: usize = 0;
    let mut idx = 0usize;

    let flush = |chunks: &mut Vec<TextChunk>, buf: &mut String, idx: &mut usize| {
        if buf.is_empty() {
            return;
        }
        *idx += 1;
        let t = std::mem::take(buf);
        chunks.push(TextChunk {
            text: t.trim().to_string(),
            label: format!("第{}{}", idx, unit),
        });
    };

    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        let pt = estimate_tokens(para);
        // 缓冲非空且加入本段会超预算 → 先封块（单段本身超预算则独立成块，不再硬切）
        if !buf.is_empty() && buf_tokens + pt > target {
            flush(&mut chunks, &mut buf, &mut idx);
            buf_tokens = 0;
        }
        if !buf.is_empty() {
            buf.push_str("\n\n");
        }
        buf.push_str(para);
        buf_tokens += pt;
    }
    flush(&mut chunks, &mut buf, &mut idx);

    if chunks.is_empty() {
        // 空文本兜底：一个空块，保证 total_pages ≥ 1
        chunks.push(TextChunk {
            text: String::new(),
            label: format!("第1{}", unit),
        });
    }
    chunks
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
        // 标题前 3 个 \n→2，正文前 5 个 \n→2，首尾 trim
        assert_eq!(s, "标题\n\n正文");
        // 单/双换行保留，3+→2
        let mut s = "a\nb".to_string();
        normalize(&mut s);
        assert_eq!(s, "a\nb");
        // CR 被跳过：\r\r\r\n 只含 1 个 \n → b/c 间单换行
        let mut s = "a\r\nb\r\r\r\nc".to_string();
        normalize(&mut s);
        assert_eq!(s, "a\nb\nc");
    }

    #[test]
    fn split_text_into_chunks_packs_by_token_budget() {
        // 短文本 → 单块（label 第1段）
        let chunks = split_text_into_chunks("短文本", "段");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].label, "第1段");
        assert_eq!(chunks[0].text, "短文本");

        // 多段：每段单独不超预算，但累加超 → 至少 2 块
        // CJK 1 token/字，target=2000；造 3 段各 1500 字 → 前两段各成块、第三段成块
        let para: String = "字".repeat(1500);
        let text = format!("{}\n\n{}\n\n{}", para, para, para);
        let chunks = split_text_into_chunks(&text, "段");
        assert_eq!(chunks.len(), 3, "3×1500字 段应切成 3 块");
        assert_eq!(chunks[0].label, "第1段");
        assert_eq!(chunks[2].label, "第3段");

        // 空文本 → 兜底 1 空块（total_pages ≥ 1）
        let chunks = split_text_into_chunks("", "段");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.is_empty());
    }
}
