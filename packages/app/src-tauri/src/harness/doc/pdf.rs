//! `.pdf` → 纯文本提取（pdf-extract）。
//!
//! pdf-extract 基于布局分析提取文本，对数字版 PDF 通过率约 91%（社区基准）；
//! 扫描件 / 纯图片 PDF 无文字层，提取结果为空——这是 PDF 文本提取的固有限制，
//! 需 OCR 才能解决，不在本模块范围。
//!
//! 输出规整化：pdf-extract 用 form feed (`\u{c}`) 分页，且常有多余空行；
//! 这里把分页符转成段落分隔（`\n\n`）并经 [`normalize`] 折叠，便于阅读与 chunk 切分。

use crate::error::{AppError, AppResult};

use super::{first_nonempty_line, normalize, DocKind, ExtractedDoc};

/// 从 PDF 字节流提取文本。
pub(super) fn extract(bytes: &[u8]) -> AppResult<ExtractedDoc> {
    let raw = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| AppError::Internal(format!("PDF 文本提取失败: {e}")))?;
    // 分页符 → 段落分隔
    let mut text = raw.replace('\u{c}', "\n\n");
    normalize(&mut text);
    let title = first_nonempty_line(&text);
    Ok(ExtractedDoc {
        text,
        kind: DocKind::Pdf,
        title,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_feed_normalized() {
        // 直接验证分页符 → 段落分隔 + 多余空行折叠
        let mut s = "第一页内容\u{c}\n\n\n\n第二页内容".to_string();
        s = s.replace('\u{c}', "\n\n");
        normalize(&mut s);
        assert_eq!(s, "第一页内容\n\n第二页内容");
    }
}
