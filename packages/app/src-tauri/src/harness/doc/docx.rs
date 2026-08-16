//! `.docx` → 文本提取。
//!
//! docx 是 OOXML 的 ZIP 容器；正文在 `word/document.xml`，可见文字位于 `<w:t>` 元素内。
//!
//! 为什么不走 docx-rs 的类型树：docx-rs 主要面向**写入**，读取的节点树嵌套极深
//! （`DocumentChild`(9 变体) → `ParagraphChild`(12) → `RunChild` → `Text`，表格还要
//! `Table`→`TableRow`→`TableCell`→`Paragraph` 四层递归），枚举变体多、且 hyperlink /
//! 修订（Insert/Delete）等也各自包 runs——容易漏取文本。直接从 ZIP 读 `document.xml`
//! 扫描 `<w:t>` 能**无视任何包裹元素**地拿到全部可见文字，更鲁棒、代码更短。
//!
//! 提取规则（轻量 tag 扫描，不引入 XML 解析器）：
//! - `<w:t ...>` 开 → 进入文本捕获；其后纯文本经 XML 实体解码后追加；`</w:t>` 关。
//! - `</w:p>`（段落结束）→ 换行。
//! - 自闭合 `<w:tab/>` → 制表符；`<w:br/>` / `<w:cr/>` → 换行。
//! - 表格结构当前**不单独保形为网格**（单元格内容按段落顺序成行）——LLM 仍可读懂，
//!   KB chunk 仍按段落切；网格化留作后续增强。

use std::io::{Cursor, Read};

use crate::error::{AppError, AppResult};

use super::{first_nonempty_line, normalize, DocKind, ExtractedDoc};

/// 从 docx 字节流提取文本。
pub(super) fn extract(bytes: &[u8]) -> AppResult<ExtractedDoc> {
    let xml = read_document_xml(bytes)?;
    let mut text = extract_text_from_xml(&xml);
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
fn read_document_xml(bytes: &[u8]) -> AppResult<String> {
    let cur = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cur)
        .map_err(|e| AppError::Internal(format!("docx 不是合法 ZIP 容器: {e}")))?;
    let mut buf = Vec::with_capacity(bytes.len() / 2);
    let mut entry = archive
        .by_name("word/document.xml")
        .map_err(|e| AppError::Internal(format!("docx 内缺少 word/document.xml: {e}")))?;
    entry.read_to_end(&mut buf).map_err(AppError::Io)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// 扫描 document.xml，提取可见文本。
///
/// 纯字符串扫描：遇 `<` 解析一个 tag（到下一个 `>`），据 tag 名/闭合性推进状态机；
/// 在 `<w:t>` 内的纯文本做实体解码后追加。
fn extract_text_from_xml(xml: &str) -> String {
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
            decode_entities_into(&xml[i..next], &mut out);
            i = next;
        } else {
            // tag 之间的空白 / 非 w:t 文本 → 跳过
            i = xml[i..].find('<').map(|j| i + j).unwrap_or(n);
        }
    }

    out
}

/// 把一段（不含 tag 的）纯文本中的 XML 实体解码后追加到 `out`。
///
/// 支持：`&amp; &lt; &gt; &quot; &apos;` 与 `&#DD;` / `&#xHH;`。
/// 不认识的实体原样保留 `&`。
fn decode_entities_into(input: &str, out: &mut String) {
    let bytes = input.as_bytes();
    let len = input.len();
    let mut i = 0usize;
    while i < len {
        if bytes[i] == b'&' {
            if let Some(semi_rel) = input[i..].find(';') {
                let ent = &input[i + 1..i + semi_rel];
                if let Some(ch) = decode_entity(ent) {
                    out.push(ch);
                    i += semi_rel + 1;
                    continue;
                }
            }
            // 末找到合法实体 → 原样输出 '&'
            out.push('&');
            i += 1;
        } else {
            // 复制到下一个 '&'
            let next = input[i..].find('&').map(|j| i + j).unwrap_or(len);
            out.push_str(&input[i..next]);
            i = next;
        }
    }
}

/// 解析单个实体名（不含首尾的 `&` `;`）为字符。
fn decode_entity(ent: &str) -> Option<char> {
    match ent {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = ent.strip_prefix('#') {
                dec.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                None
            }
        }
    }
}

// =========================================================================
// 单元测试（XML 扫描逻辑；真实 docx 字节的端到端测试见 tests/ 集成测）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn run(xml: &str) -> String {
        let mut t = extract_text_from_xml(xml);
        normalize(&mut t);
        t
    }

    #[test]
    fn roundtrip_real_docx() {
        // 用 docx-rs 写入器生成真实 docx 包 → 喂给本模块的 extract，
        // 验证 zip 直读 + <w:t> 扫描对真实 OOXML 包工作（这是本模块风险最高的代码）。
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
    }

    #[test]
    fn wt_with_attrs_still_captured() {
        // <w:t xml:space="preserve"> 形式
        let xml = "<w:p><w:r><w:t xml:space=\"preserve\">保留空格</w:t></w:r></w:p>";
        assert_eq!(run(xml), "保留空格");
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

    #[test]
    fn decode_entity_table() {
        assert_eq!(decode_entity("amp"), Some('&'));
        assert_eq!(decode_entity("lt"), Some('<'));
        assert_eq!(decode_entity("gt"), Some('>'));
        assert_eq!(decode_entity("quot"), Some('"'));
        assert_eq!(decode_entity("apos"), Some('\''));
        assert_eq!(decode_entity("#65"), Some('A'));
        assert_eq!(decode_entity("#x42"), Some('B'));
        assert_eq!(decode_entity("#X4E2D"), Some('中'));
        assert_eq!(decode_entity("nbsp"), None); // 未实现 → None（保留原样）
    }

    #[test]
    fn decode_entities_into_copies_plain() {
        let mut out = String::new();
        decode_entities_into("纯文本 no entities", &mut out);
        assert_eq!(out, "纯文本 no entities");
    }
}
