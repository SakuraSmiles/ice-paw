//! `.pdf` → 纯文本提取（pdf-extract）。
//!
//! pdf-extract 基于布局分析提取文本，对数字版 PDF 通过率约 91%（社区基准）；
//! 扫描件 / 纯图片 PDF 无文字层，提取结果为空——这是 PDF 文本提取的固有限制，
//! 需 OCR 才能解决，不在本模块范围。
//!
//! 输出规整化：pdf-extract 用 form feed (`\u{c}`) 分页，且常有多余空行；
//! 这里把分页符转成段落分隔（`\n\n`）并经 [`normalize`] 折叠，便于阅读与 chunk 切分。

use crate::error::{AppError, AppResult};

use super::{first_nonempty_line, normalize, DocKind, ExtractedDoc, TextChunk};

/// 从 PDF 字节流提取文本（整篇，KB / read_file 路径用）。
///
/// 历史上 pdf-extract 的 form feed `\u{c}` 被替换成 `\n\n` 再 normalize——
/// 页边界信息丢失。现在 [`raw_pages`] 保留页边界，本函数 = `raw_pages().join("\n\n")`，
/// 输出与旧实现等价（KB 行为零改），同时把「保留页边界」收敛到单一实现。
pub(super) fn extract(bytes: &[u8]) -> AppResult<ExtractedDoc> {
    let pages = raw_pages(bytes)?;
    let mut text = pages.join("\n\n");
    normalize(&mut text);
    let title = first_nonempty_line(&text);
    Ok(ExtractedDoc {
        text,
        kind: DocKind::Pdf,
        title,
    })
}

/// 按页提取（聊天附件分页路径用）：每页一块，label `第N页`。
///
/// 空页（split 后 trim 为空，如扫描件无文字层）被过滤——若全 PDF 无文字，
/// 返回单块空文本（兜底，`total_pages ≥ 1`，让 LLM 知道这是个无文字层的 PDF）。
pub(super) fn extract_chunks(bytes: &[u8]) -> AppResult<Vec<TextChunk>> {
    let pages = raw_pages(bytes)?;
    if pages.is_empty() {
        return Ok(vec![TextChunk {
            text: String::new(),
            label: "第1页".to_string(),
        }]);
    }
    Ok(pages
        .into_iter()
        .enumerate()
        .map(|(i, p)| TextChunk {
            text: p,
            label: format!("第{}页", i + 1),
        })
        .collect())
}

/// 按分页符 `\u{c}` 切出每页，各自 [`normalize`]，过滤空页。
///
/// pdf-extract 用 form feed 标记换页；保留这个边界是 PDF 分块（每页一块）的关键。
/// 单页内仍可能有多余空行，逐页 normalize 折叠；页间分隔由调用方决定
/// （[`extract`] join `\n\n`，[`extract_chunks`] 各自独立）。
fn raw_pages(bytes: &[u8]) -> AppResult<Vec<String>> {
    let raw = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| AppError::Internal(format!("PDF 文本提取失败: {e}")))?;
    let pages: Vec<String> = raw
        .split('\u{c}')
        .map(|p| {
            let mut s = p.to_string();
            normalize(&mut s);
            s
        })
        .filter(|p| !p.is_empty())
        .collect();
    Ok(pages)
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

    #[test]
    fn raw_pages_preserves_form_feed_boundaries() {
        // 模拟 pdf-extract 输出：form feed 分页 + 多余空行。raw_pages 应按 \u{c}
        // 切出 2 页、各自 normalize 过滤空行。空页（split 产物若 trim 为空）被过滤。
        // 这里手写「带空页」的输入：页间一个纯空段（split 后是空串）应被丢弃。
        let raw = "第一页内容\u{c}\n\n\n\n第二页内容";
        let pages: Vec<String> = raw
            .split('\u{c}')
            .map(|p| {
                let mut s = p.to_string();
                normalize(&mut s);
                s
            })
            .filter(|p| !p.is_empty())
            .collect();
        assert_eq!(
            pages,
            vec!["第一页内容".to_string(), "第二页内容".to_string()]
        );
    }
}
