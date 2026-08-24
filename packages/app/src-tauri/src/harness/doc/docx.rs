//! `.docx` → 文本提取（结构模型路径，word-capability-roadmap 步骤 1 / S0a）。
//!
//! docx 是 OOXML 的 ZIP 容器；正文在 `word/document.xml`。当前路径：
//! zip 直读 document.xml → [`super::xml_dom`] 极小 DOM → [`super::docx_model`]
//! 类型树（段落/run/表格网格/节属性/修订标记）→ `document_text` 投影 → normalize。
//!
//! 历史与零回归闸门：原实现是纯字符串扫描（`extract_text_from_xml`，保留为
//! **golden 参考**，仅测试编译）——模型投影必须与它逐字节相等，断言见本文件
//! `model_text_matches_golden_scanner` 与 corpus_tests 的真实语料双份比对。
//!
//! 已知微差（接受）：扫描器对**配对形式**的 `<w:tab></w:tab>` 不输出 `\t`（只认
//! 自闭合），模型统一输出——Word/WPS 恒发自闭合形式，真实文档无此差异。

use std::io::{Cursor, Read};

use crate::error::{AppError, AppResult};

use super::docx_model;
use super::{first_nonempty_line, normalize, DocKind, ExtractedDoc};

/// 从 docx 字节流提取文本（模型路径）。
pub(super) fn extract(bytes: &[u8]) -> AppResult<ExtractedDoc> {
    let xml = read_document_xml(bytes)?;
    let dom = super::xml_dom::parse(&xml)?;
    let model = docx_model::build_document(&dom);
    let mut text = docx_model::document_text(&model);
    normalize(&mut text);
    let title = first_nonempty_line(&text);
    Ok(ExtractedDoc {
        text,
        kind: DocKind::Docx,
        title,
    })
}

/// 从 docx (ZIP) 容器读出 `word/document.xml` 为字符串。
///
/// 容器非法 / 缺 document.xml → `Err`（绝不退回 lossy 让调用方误判成功）。
/// document.xml 约定为 UTF-8；对极少数编码损坏的文件用 lossy 兜底（仅此一处，
/// 不影响"非 office 走原文本解码"的总体约定——此处已确认是 docx）。
pub(super) fn read_document_xml(bytes: &[u8]) -> AppResult<String> {
    read_entry(bytes, "word/document.xml")?.ok_or_else(|| {
        AppError::Internal("docx 内缺少 word/document.xml".to_string())
    })
}

/// 从 docx (ZIP) 容器读出任意部件为字符串；部件不存在 → `Ok(None)`（styles.xml
/// 等可选部件）。容器非法 → `Err`。inspect_docx（S0b）与未来的手术引擎共用。
pub(super) fn read_entry(bytes: &[u8], name: &str) -> AppResult<Option<String>> {
    let cur = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cur)
        .map_err(|e| AppError::Internal(format!("docx 不是合法 ZIP 容器: {e}")))?;
    let mut buf = Vec::with_capacity(bytes.len() / 2);
    let mut entry = match archive.by_name(name) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => {
            return Err(AppError::Internal(format!("docx 内读取 {name} 失败: {e}")));
        }
    };
    entry.read_to_end(&mut buf).map_err(AppError::Io)?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

// =========================================================================
// golden 扫描器（旧实现原样保留，仅测试编译——零回归的对照面）
// =========================================================================

/// 旧实现的线性文本扫描：遇 `<` 解析一个 tag（到下一个 `>`），据 tag 名/闭合性
/// 推进状态机；在 `<w:t>` 内的纯文本做实体解码后追加；`</w:p>` → 换行。
#[cfg(test)]
pub(super) fn extract_text_from_xml(xml: &str) -> String {
    let n = xml.len();
    let mut out = String::with_capacity(n / 4);
    let mut i = 0usize;
    let mut in_t = false; // 是否在 <w:t>...</w:t> 文本区间内

    while i < n {
        let bytes = xml.as_bytes();
        if bytes[i] == b'<' {
            // 定位 tag 结束的 '>'
            let end = match xml[i..].find('>') {
                Some(j) => i + j,
                None => break, // 残缺 tag → 终止
            };
            // tag 内容（不含首尾的 < >）
            let raw = &xml[i + 1..end];
            let self_closing = raw.ends_with('/');
            // 去掉可能的尾随 '/' 与空白后取 tag 主体
            let body = raw.trim_end_matches('/').trim();
            let (closing, name_part) = if let Some(rest) = body.strip_prefix('/') {
                (true, rest.trim_start())
            } else {
                (false, body)
            };
            // tag 名 = 第一个空白或 '/' 之前的部分（丢弃属性）
            let name = name_part
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("");

            if !closing && !self_closing && name == "w:t" {
                in_t = true;
            } else if closing && name == "w:t" {
                in_t = false;
            } else if closing && name == "w:p" {
                out.push('\n'); // 段落结束 → 换行
            } else if !closing && self_closing {
                // 仅自闭合结构标签有意义；其余自闭合（如 w:rPr）忽略
                match name {
                    "w:tab" => out.push('\t'),
                    "w:br" | "w:cr" => out.push('\n'),
                    _ => {}
                }
            }
            i = end + 1;
        } else if in_t {
            // 捕获到下一个 '<' 为止的文本
            let next = xml[i..].find('<').map(|j| i + j).unwrap_or(n);
            docx_model::decode_entities_into(&xml[i..next], &mut out);
            i = next;
        } else {
            // tag 之间的空白 / 非 w:t 文本 → 跳过
            i = xml[i..].find('<').map(|j| i + j).unwrap_or(n);
        }
    }

    out
}

// =========================================================================
// 单元测试（golden 扫描器语义锁 + 模型零回归比对；真实语料见 corpus_tests）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::xml_dom;

    /// golden 路径：扫描器 + normalize
    fn run(xml: &str) -> String {
        let mut t = extract_text_from_xml(xml);
        normalize(&mut t);
        t
    }

    /// 模型路径：DOM → 模型 → 投影 + normalize
    fn model_text(xml: &str) -> String {
        let dom = xml_dom::parse(xml).unwrap();
        let model = docx_model::build_document(&dom);
        let mut t = docx_model::document_text(&model);
        normalize(&mut t);
        t
    }

    /// 零回归核心断言：同一 XML，模型路径与 golden 扫描器输出相等
    fn assert_zero_regression(xml: &str) {
        assert_eq!(model_text(xml), run(xml), "模型投影 ≠ 扫描器: {xml}");
    }

    #[test]
    fn model_text_matches_golden_scanner() {
        // 基础形态
        assert_zero_regression(r#"<w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:p><w:r><w:t>World</w:t></w:r></w:p>"#);
        assert_zero_regression("<w:p><w:r><w:t xml:space=\"preserve\">保留空格</w:t></w:r></w:p>");
        // 实体（已知/数字/未知）
        assert_zero_regression(
            "<w:p><w:r><w:t>a &amp; b &lt; c &gt; d &quot;e&quot; &apos;f&apos;</w:t></w:r></w:p>",
        );
        assert_zero_regression("<w:p><w:r><w:t>&#65;&#x42;&#20013;</w:t></w:r></w:p>");
        assert_zero_regression("<w:p><w:r><w:t>a &foo; b</w:t></w:r></w:p>");
        // tab / br / 空段折叠
        assert_zero_regression("<w:p><w:r><w:t>A</w:t><w:tab/><w:t>B</w:t></w:r></w:p>");
        assert_zero_regression("<w:p><w:r><w:t>A</w:t><w:br/><w:t>B</w:t></w:r></w:p>");
        assert_zero_regression(
            "<w:p></w:p><w:p></w:p><w:p></w:p><w:p><w:r><w:t>正文</w:t></w:r></w:p>",
        );
        // 修订 / 超链接 / 内容控件 / 域
        assert_zero_regression(
            r#"<w:p><w:ins><w:r><w:t>新增</w:t></w:r></w:ins><w:del><w:r><w:delText>旧文</w:delText></w:r></w:del></w:p>"#,
        );
        assert_zero_regression(r#"<w:p><w:hyperlink><w:r><w:t>链接</w:t></w:r></w:hyperlink></w:p>"#);
        assert_zero_regression(
            r#"<w:sdt><w:sdtContent><w:p><w:r><w:t>目录</w:t></w:r></w:p></w:sdtContent></w:sdt>"#,
        );
        assert_zero_regression(
            r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>TOC</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>域结果</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        );
        // 表格（含嵌套）
        assert_zero_regression(
            r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B1</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        );
        assert_zero_regression(
            r#"<w:tbl><w:tr><w:tc><w:tbl><w:tr><w:tc><w:p><w:r><w:t>内层</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:tc></w:tr></w:tbl>"#,
        );
        // 文本框（奇异子树摊平）
        assert_zero_regression(
            r#"<w:p><w:r><w:drawing><w:pict><w:txbxContent><w:p><w:r><w:t>框内</w:t></w:r></w:p></w:txbxContent></w:pict></w:drawing></w:r></w:p>"#,
        );
    }

    #[test]
    fn roundtrip_real_docx() {
        // 用 docx-rs 写入器生成真实 docx 包 → 喂给本模块的 extract（模型路径），
        // 验证 zip 直读 + DOM + 模型对真实 OOXML 包工作（这是本模块风险最高的代码）。
        use docx_rs::{Document, Docx, Paragraph, Run};
        let document = Document::new()
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("你好世界")))
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第二段落")));
        // Vec<u8> 不 impl Seek，用 Cursor 承载（pack 需 Write + Seek）
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        Docx::new()
            .document(document)
            .build()
            .pack(&mut cursor)
            .expect("docx-rs 打包成功");
        let buf = cursor.into_inner();

        let extracted = super::extract(&buf).expect("我们的提取成功");
        assert!(
            matches!(extracted.kind, super::DocKind::Docx),
            "kind 应为 Docx"
        );
        assert!(
            extracted.text.contains("你好世界"),
            "应含首段文本，实际: {:?}",
            extracted.text
        );
        assert!(extracted.text.contains("第二段落"));
        assert_eq!(extracted.title.as_deref(), Some("你好世界"));
    }

    #[test]
    fn extracts_text_from_wt() {
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="w"><w:body>
<w:p><w:r><w:t>Hello</w:t></w:r></w:p>
<w:p><w:r><w:t>World</w:t></w:r></w:p>
</w:body></w:document>"#;
        assert_eq!(run(xml), "Hello\nWorld");
        assert_zero_regression(xml);
    }

    #[test]
    fn entities_decoded() {
        let xml =
            "<w:p><w:r><w:t>a &amp; b &lt; c &gt; d &quot;e&quot; &apos;f&apos;</w:t></w:r></w:p>";
        assert_eq!(run(xml), "a & b < c > d \"e\" 'f'");
    }

    #[test]
    fn numeric_entities_decoded() {
        let xml = "<w:p><w:r><w:t>&#65;&#x42;&#20013;</w:t></w:r></w:p>";
        assert_eq!(run(xml), "AB中");
    }

    #[test]
    fn unknown_entity_preserved() {
        let xml = "<w:p><w:r><w:t>a &foo; b</w:t></w:r></w:p>";
        assert_eq!(run(xml), "a &foo; b");
    }

    #[test]
    fn tab_and_break_handling() {
        let xml = "<w:p><w:r><w:t>A</w:t><w:tab/><w:t>B</w:t></w:r></w:p>";
        assert_eq!(run(xml), "A\tB");
        let xml = "<w:p><w:r><w:t>A</w:t><w:br/><w:t>B</w:t></w:r></w:p>";
        assert_eq!(run(xml), "A\nB");
    }

    #[test]
    fn blank_paragraphs_collapsed() {
        // 三个连续空段 → normalize 折叠
        let xml = "<w:p></w:p><w:p></w:p><w:p></w:p><w:p><w:r><w:t>正文</w:t></w:r></w:p>";
        assert_eq!(run(xml), "正文");
    }
}
