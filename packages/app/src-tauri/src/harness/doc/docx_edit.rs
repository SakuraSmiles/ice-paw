//! `docx_edit` —— 块级外科手术引擎（word-capability-roadmap 步骤 3）。
//!
//! 三层结构，全部纯函数（bytes/xml 进、新 xml/bytes 出）：
//! 1. **块定位器** [`locate_blocks`]：quick-xml 流式扫描 document.xml，产出每个
//!    编址块的**源码字节范围**（`[start, end)`）。遍历语义与 `docx_model::walk_blocks`
//!    严格一致（w:p/w:tbl 成块即止、sectPr/书签等跳过、未知容器透明递归摊平）——
//!    inspect_docx 的块号在这里一一对应到源码区间。对齐由测试锁定（语料逐块）。
//! 2. **手术** [`apply_edits`]：验证后变异——全批预检（块号/指纹/修订/节属性/
//!    重复块）通过才动刀；每个操作只替换目标块的字节区间，区间外**一个字节不动**
//!    （pPr/rPr 原样切片保留，「不敢懂的不碰」）。操作按位置降序 splice，原始
//!    偏移全程有效。
//! 3. **容器重打包** [`repack_document`]：只替换 `word/document.xml`，其余 zip
//!    entry 用 `raw_copy_file` **原样字节复制**（不解压重压，压缩参数/元数据不变）。
//!
//! MVP 操作词表（批量事务，D3 拍板 2026-08-24）：`replace_text` /
//! `insert_paragraph_after` / `delete_block`。格式/表格操作后续批。

use std::io::{Cursor, Write};

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Serialize;

use crate::error::{AppError, AppResult};

use super::docx_model::{self, Block};
use super::styles::Stylesheet;

// =========================================================================
// 块定位器
// =========================================================================

/// 一个编址块在 document.xml 源码中的字节范围（`[start, end)`，UTF-8 字节偏移）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BlockSpan {
    pub start: usize,
    pub end: usize,
    /// w:tbl 块（true）；w:段落（false）
    pub is_table: bool,
}

/// 栈帧角色：BodySeq = 块级序列上下文（按 walk_blocks 语义找块）；Inside = 块 /
/// 跳过名单元素的内部（不再找块，等配对 End）。
#[derive(PartialEq, Clone, Copy)]
enum FrameRole {
    BodySeq,
    Inside,
}

/// 定位全部编址块。要求根为 `w:document` 且含 `w:body`（edit 路径只面对真实
/// docx 包，不做合成宽容——宽容会引入与模型编址的不一致面）。
pub(super) fn locate_blocks(xml: &str) -> AppResult<Vec<BlockSpan>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false); // 偏移必须与原始字节对齐
    // 不展开自闭合：Empty(w:p) 自成一块，范围即标签本身

    // (角色, 元素名, 块起点（成块帧才 Some）)
    let mut stack: Vec<(FrameRole, String, Option<usize>)> = Vec::new();
    let mut spans: Vec<BlockSpan> = Vec::new();
    let mut seen_body = false;
    let mut done = false;

    loop {
        // buffer_position = 上一事件结束处 = 本事件开始处
        let ev_start = reader.buffer_position() as usize;
        let ev = match reader.read_event() {
            Ok(Event::Eof) => break, // ⚠️ 必须在 Ok(e) 之前——通配臂会吞掉 Eof 致死循环
            Ok(e) => e,
            Err(e) => {
                return Err(AppError::Internal(format!("document.xml 扫描失败: {e}")))
            }
        };
        let ev_end = reader.buffer_position() as usize;

        if done {
            continue; // body 已闭合：剩余事件（如根闭合）无需处理
        }

        match ev {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                if !seen_body {
                    if name == "w:body" {
                        seen_body = true;
                        stack.push((FrameRole::BodySeq, name, None));
                    } else {
                        // w:document 根及其他前导元素内部：等 body
                        stack.push((FrameRole::Inside, name, None));
                    }
                    continue;
                }
                let in_body_seq = matches!(stack.last(), Some((FrameRole::BodySeq, _, _)));
                match name.as_str() {
                    "w:p" | "w:tbl" if in_body_seq => {
                        // 成块：起点 = 本 Start 标签首字节；内部整体跳过
                        stack.push((FrameRole::Inside, name.clone(), Some(ev_start)));
                    }
                    // 跳过名单（与 walk_blocks 一致）：sectPr / 书签 / 拼写标记
                    "w:sectPr" | "w:bookmarkStart" | "w:bookmarkEnd" | "w:proofErr"
                        if in_body_seq =>
                    {
                        stack.push((FrameRole::Inside, name, None));
                    }
                    _ if in_body_seq => {
                        // 未知容器（w:sdt 等）：透明递归，内部继续找块
                        stack.push((FrameRole::BodySeq, name, None));
                    }
                    _ => stack.push((FrameRole::Inside, name, None)),
                }
            }
            Event::Empty(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let in_body_seq = matches!(stack.last(), Some((FrameRole::BodySeq, _, _)));
                if in_body_seq && (name == "w:p" || name == "w:tbl") {
                    // 自闭合块：范围即标签本身
                    spans.push(BlockSpan {
                        start: ev_start,
                        end: ev_end,
                        is_table: name == "w:tbl",
                    });
                }
                // 名单元素 / 其他自闭合：无块级语义
            }
            Event::End(_) => {
                if let Some((_, name, block_start)) = stack.pop() {
                    if let Some(start) = block_start {
                        spans.push(BlockSpan {
                            start,
                            end: ev_end,
                            is_table: name == "w:tbl",
                        });
                    }
                    if name == "w:body" {
                        done = true;
                    }
                }
            }
            _ => {} // Text / 实体等：不影响元素级定位
        }
    }

    if !seen_body {
        return Err(AppError::Internal(
            "document.xml 缺少 w:body（edit 路径要求标准 w:document/w:body 结构）".into(),
        ));
    }
    Ok(spans)
}

// =========================================================================
// 操作词表（批量事务）
// =========================================================================

/// 一个编辑操作（块号 1-based，与 inspect_docx 编址一致）。
#[derive(Debug, Clone)]
pub enum EditOp {
    /// 替换段落文本：保留 pPr 与首个 run 的 rPr（周边格式不动）；表格块拒绝。
    ReplaceText { block: usize, expect_prefix: String, new_text: String },
    /// 在锚块后插入新段：style（显示名，可选）指定样式；缺省继承锚块段落格式。
    InsertParagraphAfter { block: usize, expect_prefix: String, text: String, style: Option<String> },
    /// 删除整块（含段落标记）；块内含 sectPr（节属性载体）拒绝。
    DeleteBlock { block: usize, expect_prefix: String },
}

/// 单操作执行摘要（agent 读回验证用）。
#[derive(Debug, Serialize)]
pub struct AppliedOp {
    pub op: &'static str,
    pub block: usize,
    /// 原块投影文本（前 60 字）
    pub before: String,
    /// 新块投影文本（前 60 字；delete 为空）
    pub after: String,
}

/// 组合入口（工具层消费）：docx 字节 + 操作批 → 新 docx 字节 + 摘要。
/// 读 document.xml 与 styles.xml → 手术 → 重打包；IO/写盘/授权在调用方。
pub fn apply_edits_to_bytes(bytes: &[u8], ops: &[EditOp]) -> AppResult<(Vec<u8>, Vec<AppliedOp>)> {
    let xml = super::docx::read_document_xml(bytes)?;
    let styles = match super::docx::read_entry(bytes, "word/styles.xml")? {
        Some(s) => {
            super::styles::parse_styles(&super::xml_dom::parse(&s)?)
        }
        None => Stylesheet::empty(),
    };
    let (new_xml, applied) = apply_edits(&xml, &styles, ops)?;
    let out = repack_document(bytes, &new_xml)?;
    Ok((out, applied))
}

/// 应用一批操作，返回新 document.xml 与逐操作摘要。**全有或全无**：任一预检
/// 不通过 → Err，原 xml 不动。
pub(super) fn apply_edits(
    xml: &str,
    styles: &Stylesheet,
    ops: &[EditOp],
) -> AppResult<(String, Vec<AppliedOp>)> {
    if ops.is_empty() {
        return Err(AppError::Validation(
            "操作列表为空: edit_docx 至少需要一个操作。请提供 replace_text / insert_paragraph_after / delete_block 操作。".into(),
        ));
    }

    let dom = super::xml_dom::parse(xml)?;
    let model = docx_model::build_document(&dom);
    let spans = locate_blocks(xml)?;
    if spans.len() != model.body.len() {
        return Err(AppError::Internal(format!(
            "定位器与模型编址不一致: 定位 {} 块 / 模型 {} 块（内部 bug，请反馈）",
            spans.len(),
            model.body.len()
        )));
    }

    // ---- 预检全批（验证后变异：全部通过才动刀）----
    let mut used_blocks: Vec<usize> = Vec::new();
    for op in ops {
        let block = *match op {
            EditOp::ReplaceText { block, .. }
            | EditOp::InsertParagraphAfter { block, .. }
            | EditOp::DeleteBlock { block, .. } => block,
        };
        if block == 0 || block > spans.len() {
            return Err(AppError::Validation(format!(
                "块号越界: 块 {block} 不存在（全文共 {} 块，块号 1-{}）。请先用 inspect_docx text 复核块号。",
                spans.len(),
                spans.len()
            )));
        }
        if used_blocks.contains(&block) {
            return Err(AppError::Validation(format!(
                "同一块多操作: 块 {block} 在本批中被多次引用。每块每批限一个操作；请拆成多批。"
            )));
        }
        used_blocks.push(block);

        let idx = block - 1;
        let span = spans[idx];
        let block_xml = &xml[span.start..span.end];
        let expect = expect_prefix_of(op);
        let mut projected = String::new();
        docx_model::blocks_text(&model.body[idx..idx + 1], &mut projected);
        let projected = projected.trim_end_matches('\n');
        if !projected.starts_with(expect.trim_end()) {
            return Err(AppError::Validation(format!(
                "指纹不符: 块 {block} 当前内容不以预期文本开头。期望前缀 {:?}，实际开头 {:?}。\
                 文档可能已被其他编辑改动——请先用 inspect_docx text 查看块 {block} 当前内容再重试。",
                truncate(expect, 40),
                truncate(projected, 40)
            )));
        }

        match op {
            EditOp::DeleteBlock { .. } => {
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                if block_xml.contains("<w:sectPr") {
                    return Err(AppError::Validation(format!(
                        "节属性保护: 块 {block} 含节属性（分节符载体），删除会破坏分节。\
                         请选择其他块操作。"
                    )));
                }
            }
            EditOp::ReplaceText { .. } => {
                // sectPr 在 pPr 内被原样切片保留，改文本不破坏分节——放行
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
            }
            EditOp::InsertParagraphAfter { style, .. } => {
                if let Some(name) = style {
                    if styles.id_of(name).is_none() {
                        return Err(AppError::Validation(format!(
                            "未知样式: {:?} 不在本文档样式表中。可用样式（前 20）: {}。\
                             样式名来自 inspect_docx outline 的样式列；或省略 style 继承锚块格式。",
                            name,
                            styles.display_names_joined(20)
                        )));
                    }
                }
            }
        }
        // Replace 目标必须是段落块
        if let EditOp::ReplaceText { .. } = op {
            if span.is_table {
                return Err(AppError::Validation(format!(
                    "表格块: 块 {block} 是表格，replace_text 只支持段落。\
                     表格内容编辑暂不支持（后续批次）；可用 inspect_docx text 查看表格内容。"
                )));
            }
        }
    }

    // ---- 生成 splice 计划（原始偏移；互不重叠，见预检的每块一操作）----
    struct Splice {
        pos: usize,
        remove_end: usize,
        insert: String,
        summary: AppliedOp,
    }
    let mut plan: Vec<Splice> = Vec::new();
    for op in ops {
        match op.clone() {
            EditOp::ReplaceText { block, new_text, .. } => {
                let span = spans[block - 1];
                let new_block = rebuild_paragraph(xml, span, Some(&new_text));
                let after = new_text.clone();
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: new_block,
                    summary: AppliedOp {
                        op: "replace_text",
                        block,
                        before: projected_of(&model, block),
                        after: truncate(&after, 60),
                    },
                });
            }
            EditOp::InsertParagraphAfter { block, text, style, .. } => {
                let span = spans[block - 1];
                let anchor_has_revision = has_revision(&model.body[block - 1]);
                let new_block = build_inserted_paragraph(xml, span, &text, style.as_deref(), styles, anchor_has_revision);
                plan.push(Splice {
                    pos: span.end, // 锚块末尾插入（区间外，不与同批其他 splice 重叠）
                    remove_end: span.end,
                    insert: new_block,
                    summary: AppliedOp {
                        op: "insert_paragraph_after",
                        block,
                        before: projected_of(&model, block),
                        after: truncate(&text, 60),
                    },
                });
            }
            EditOp::DeleteBlock { block, .. } => {
                let span = spans[block - 1];
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: String::new(),
                    summary: AppliedOp {
                        op: "delete_block",
                        block,
                        before: projected_of(&model, block),
                        after: String::new(),
                    },
                });
            }
        }
    }

    // ---- splice（按位置升序单 pass；区间互不重叠，见预检的每块一操作）----
    plan.sort_by(|a, b| a.pos.cmp(&b.pos).then(a.remove_end.cmp(&b.remove_end)));
    let mut out = String::with_capacity(xml.len() + 256);
    let mut cursor = 0usize;
    for s in &plan {
        debug_assert!(s.pos >= cursor, "splice 区间重叠（内部 bug）");
        out.push_str(&xml[cursor..s.pos]);
        out.push_str(&s.insert);
        cursor = s.remove_end;
    }
    out.push_str(&xml[cursor..]);

    // 产物必须仍是合法 XML 且块数守恒（删 N 块插 M 段 → 总数变化可推算）
    let dom2 = super::xml_dom::parse(&out)?;
    let model2 = docx_model::build_document(&dom2);
    let inserted = ops.iter().filter(|o| matches!(o, EditOp::InsertParagraphAfter { .. })).count();
    let deleted = ops.iter().filter(|o| matches!(o, EditOp::DeleteBlock { .. })).count();
    let expect_blocks = model.body.len() + inserted - deleted;
    if model2.body.len() != expect_blocks {
        return Err(AppError::Internal(format!(
            "手术后块数校验失败: 期望 {expect_blocks} 块，实际 {} 块（内部 bug，未写盘）",
            model2.body.len()
        )));
    }

    let applied = plan.into_iter().map(|s| s.summary).collect();
    Ok((out, applied))
}

fn expect_prefix_of(op: &EditOp) -> &str {
    match op {
        EditOp::ReplaceText { expect_prefix, .. }
        | EditOp::InsertParagraphAfter { expect_prefix, .. }
        | EditOp::DeleteBlock { expect_prefix, .. } => expect_prefix,
    }
}

/// 块投影文本（前 60 字；与 inspect text 投影同口径）。
fn projected_of(model: &docx_model::DocxDocument, block: usize) -> String {
    let mut s = String::new();
    docx_model::blocks_text(&model.body[block - 1..block], &mut s);
    truncate(s.trim_end_matches('\n'), 60)
}

/// 块内是否含修订 run（w:ins / w:del）。
fn has_revision(block: &Block) -> bool {
    fn para_has_revision(runs: &[docx_model::Run]) -> bool {
        runs.iter().any(|r| r.revision.is_some())
    }
    match block {
        Block::Paragraph(p) => para_has_revision(&p.runs),
        Block::Table(t) => t.rows.iter().any(|row| {
            row.cells.iter().any(|c| c.blocks.iter().any(has_revision))
        }),
    }
}

// =========================================================================
// 块 XML 重建（pPr / rPr / 开标签属性全部原样字节切片保留）
// =========================================================================

/// 重建段落块：保留开标签属性 + pPr + 首个 run 的 rPr，正文换为 `new_text`。
/// `new_text = None` 表示删除整块（返回空串）。
fn rebuild_paragraph(xml: &str, span: BlockSpan, new_text: Option<&str>) -> String {
    let block_xml = &xml[span.start..span.end];
    let Some(text) = new_text else { return String::new() };

    // 开标签原样（`<w:p>` / 带 w14:paraId 等属性的 `<w:p …>`）；自闭合（`<w:p/>` /
    // `<w:p w14:paraId="X"/>`——Word 空段常态）去掉 "/>" 补 ">"，属性保留
    let open_tag = if let Some(stripped) = block_xml.strip_suffix("/>") {
        format!("{stripped}>")
    } else {
        let gt = block_xml.find('>').expect("块必含 '>'");
        block_xml[..=gt].to_string()
    };
    let ppr = slice_ppr(block_xml);
    let rpr = slice_first_run_rpr(block_xml);
    format!("{open_tag}{ppr}{}</w:p>", run_xml(&rpr, text))
}

/// 构造插入段：style 显示名 → pPr；缺省继承锚块 pPr。rPr 继承锚块首 run
/// （锚块含修订时不继承——修订 run 的格式不作为模板）。
fn build_inserted_paragraph(
    xml: &str,
    anchor: BlockSpan,
    text: &str,
    style: Option<&str>,
    styles: &Stylesheet,
    anchor_has_revision: bool,
) -> String {
    let anchor_xml = &xml[anchor.start..anchor.end];
    let (ppr, rpr) = match style.and_then(|n| styles.id_of(n)) {
        Some(style_id) => (
            format!(r#"<w:pPr><w:pStyle w:val="{style_id}"/></w:pPr>"#),
            String::new(),
        ),
        None => {
            let ppr = slice_ppr(anchor_xml).to_string();
            let rpr = if anchor_has_revision || anchor.is_table {
                String::new()
            } else {
                slice_first_run_rpr(anchor_xml)
            };
            (ppr, rpr)
        }
    };
    format!("<w:p>{ppr}{}</w:p>", run_xml(&rpr, text))
}

/// 切出块内的 `<w:pPr>…</w:pPr>` 原样字节（无则空串）。
fn slice_ppr(block_xml: &str) -> &str {
    match block_xml.find("<w:pPr>") {
        Some(s) => match block_xml[s..].find("</w:pPr>") {
            Some(e_rel) => &block_xml[s..s + e_rel + "</w:pPr>".len()],
            None => "",
        },
        None => "",
    }
}

/// 切出首个 run 的 `<w:rPr>…</w:rPr>`（在 pPr 之后找，避开段落标记字符格式）。
fn slice_first_run_rpr(block_xml: &str) -> String {
    let search_from = match block_xml.find("</w:pPr>") {
        Some(e) => e + "</w:pPr>".len(),
        // 无 pPr：从开标签后开始
        None => block_xml.find('>').map(|g| g + 1).unwrap_or(0),
    };
    let hay = &block_xml[search_from..];
    match hay.find("<w:rPr>") {
        Some(s) => match hay[s..].find("</w:rPr>") {
            Some(e_rel) => hay[s..s + e_rel + "</w:rPr>".len()].to_string(),
            None => String::new(),
        },
        None => String::new(),
    }
}

/// 构造 run XML：普通文本段 / 换行（\n → w:br）/ 制表符（\t → w:tab）交替。
fn run_xml(rpr: &str, text: &str) -> String {
    let mut s = String::from("<w:r>");
    s.push_str(rpr);
    let mut buf = String::new();
    let flush = |buf: &mut String, s: &mut String| {
        if !buf.is_empty() {
            s.push_str(r#"<w:t xml:space="preserve">"#);
            escape_into(buf, s);
            s.push_str("</w:t>");
            buf.clear();
        }
    };
    for ch in text.chars() {
        match ch {
            '\n' => {
                flush(&mut buf, &mut s);
                s.push_str("<w:br/>");
            }
            '\t' => {
                flush(&mut buf, &mut s);
                s.push_str("<w:tab/>");
            }
            _ => buf.push(ch),
        }
    }
    flush(&mut buf, &mut s);
    s.push_str("</w:r>");
    s
}

/// XML 文本节点转义（& < >；其余字符合法直接输出）。
fn escape_into(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect()
    }
}

// =========================================================================
// 容器重打包（只换 document.xml，其余 entry 原样字节复制）
// =========================================================================

/// 重打包 docx：`word/document.xml` 替换为 `new_xml`，其余 entry 经
/// `raw_copy_file` 原样复制（不解压重压——压缩参数与元数据不变）。
pub(super) fn repack_document(bytes: &[u8], new_xml: &str) -> AppResult<Vec<u8>> {
    let cur = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cur)
        .map_err(|e| AppError::Internal(format!("docx 不是合法 ZIP 容器: {e}")))?;

    let writer = {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::<u8>::with_capacity(bytes.len())));
        for i in 0..archive.len() {
            let entry = archive
                .by_index_raw(i)
                .map_err(|e| AppError::Internal(format!("docx 内读取 entry 失败: {e}")))?;
            let name = entry.name().to_string();
            if name == "word/document.xml" {
                // 逐 entry 借用冲突：document.xml 先收名，循环外统一写
                drop(entry);
                w.start_file("word/document.xml", zip::write::SimpleFileOptions::default())
                    .map_err(|e| AppError::Internal(format!("重打包 document.xml 失败: {e}")))?;
                w.write_all(new_xml.as_bytes()).map_err(AppError::Io)?;
            } else {
                w.raw_copy_file(entry)
                    .map_err(|e| AppError::Internal(format!("重打包 entry {name} 失败: {e}")))?;
            }
        }
        w
    };

    let out = writer
        .finish()
        .map_err(|e| AppError::Internal(format!("docx 重打包收尾失败: {e}")))?;
    Ok(out.into_inner())
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::super::docx;
    use super::*;
    use std::io::Read;

    fn wrap(body: &str) -> String {
        format!(
            r#"<w:document xmlns:w="w" xmlns:r="r"><w:body>{body}</w:body></w:document>"#
        )
    }

    fn model_of(xml: &str) -> docx_model::DocxDocument {
        docx_model::build_document(&super::super::xml_dom::parse(xml).unwrap())
    }

    /// 错误断言辅助：剥掉 AppError 的类型前缀（如「参数校验失败: 」），
    /// 断言对准 message 本身——家族词在前。
    fn val_msg(e: crate::error::AppError) -> String {
        let s = e.to_string();
        s.strip_prefix("参数校验失败: ").unwrap_or(&s).to_string()
    }

    /// 定位器对齐断言：数量 == 模型 body；逐块 span 子串独立解析为单块。
    fn assert_locator_aligned(xml: &str) {
        let spans = locate_blocks(xml).unwrap();
        let model = model_of(xml);
        assert_eq!(spans.len(), model.body.len(), "定位器/模型块数不一致");
        for (i, span) in spans.iter().enumerate() {
            let piece = &xml[span.start..span.end];
            let piece_model = model_of(piece);
            assert_eq!(piece_model.body.len(), 1, "块 {} 子串应恰为单块: {piece}", i + 1);
            let mut expect = String::new();
            docx_model::blocks_text(&model.body[i..i + 1], &mut expect);
            let mut got = String::new();
            docx_model::blocks_text(&piece_model.body, &mut got);
            assert_eq!(expect, got, "块 {} 文本不一致", i + 1);
        }
    }

    #[test]
    fn locator_aligns_with_model_basic() {
        assert_locator_aligned(&wrap(
            r#"<w:p><w:r><w:t>第一段</w:t></w:r></w:p><w:p/><w:p><w:r><w:t>第三段</w:t></w:r></w:p>"#,
        ));
    }

    #[test]
    fn locator_aligns_with_sdt_tables_and_markers() {
        // sdt 摊平 / 表格成块 / sectPr+书签跳过 / 未知容器递归
        assert_locator_aligned(&wrap(
            r#"<w:sdt><w:sdtContent><w:p><w:r><w:t>目录项</w:t></w:r></w:p></w:sdtContent></w:sdt>\
               <w:bookmarkStart w:id="1" w:name="_Toc"/><w:bookmarkEnd w:id="1"/>\
               <w:tbl><w:tr><w:tc><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc></w:tr></w:tbl>\
               <w:proofErr w:type="spellStart"/><w:p><w:r><w:t>正文</w:t></w:r></w:p>\
               <w:p><w:pPr><w:sectPr><w:pgSz w:w="1"/></w:sectPr></w:pPr><w:r><w:t>节末</w:t></w:r></w:p>\
               <w:sectPr><w:pgSz w:w="2"/></w:sectPr>"#,
        ));
    }

    #[test]
    fn locator_rejects_bodyless_xml() {
        let err = locate_blocks(r#"<w:document xmlns:w="w"></w:document>"#).unwrap_err();
        assert!(err.to_string().contains("w:body"), "实际: {err}");
    }

    #[test]
    fn replace_text_preserves_ppr_and_rpr() {
        let xml = wrap(
            r#"<w:p w14:paraId="ABC"><w:pPr><w:pStyle w:val="2"/><w:jc w:val="center"/></w:pPr>\
               <w:r><w:rPr><w:b/><w:sz w:val="32"/></w:rPr><w:t>旧标题</w:t></w:r></w:p>"#,
        );
        let styles = Stylesheet::empty();
        let (out, applied) = apply_edits(
            &xml,
            &styles,
            &[EditOp::ReplaceText {
                block: 1,
                expect_prefix: "旧标题".into(),
                new_text: "新标题<x>&".into(),
            }],
        )
        .unwrap();
        assert!(out.contains(r#"<w:p w14:paraId="ABC">"#), "开标签属性保留");
        assert!(out.contains(r#"<w:pPr><w:pStyle w:val="2"/><w:jc w:val="center"/></w:pPr>"#));
        assert!(out.contains("<w:rPr><w:b/><w:sz w:val=\"32\"/></w:rPr>"));
        assert!(out.contains("新标题&lt;x&gt;&amp;"), "实体转义");
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].op, "replace_text");
        // 重解析：单块 + 新文本 + 格式仍可解析
        let m = model_of(&out);
        let mut t = String::new();
        docx_model::blocks_text(&m.body, &mut t);
        assert_eq!(t, "新标题<x>&\n");
    }

    #[test]
    fn insert_after_inherits_anchor_format() {
        let xml = wrap(
            r#"<w:p><w:pPr><w:pStyle w:val="body"/></w:pPr><w:r><w:rPr><w:i/></w:rPr><w:t>锚段</w:t></w:r></w:p><w:p><w:r><w:t>尾段</w:t></w:r></w:p>"#,
        );
        let styles = Stylesheet::empty();
        let (out, _) = apply_edits(
            &xml,
            &styles,
            &[EditOp::InsertParagraphAfter {
                block: 1,
                expect_prefix: "锚段".into(),
                text: "插入段".into(),
                style: None,
            }],
        )
        .unwrap();
        let m = model_of(&out);
        assert_eq!(m.body.len(), 3);
        let mut t = String::new();
        docx_model::blocks_text(&m.body, &mut t);
        assert_eq!(t, "锚段\n插入段\n尾段\n");
        // 继承锚块 pPr + rPr
        assert!(out.contains(r#"<w:pPr><w:pStyle w:val="body"/></w:pPr><w:r><w:rPr><w:i/></w:rPr>"#));
    }

    #[test]
    fn insert_with_style_name_resolves_id() {
        let styles_xml = r#"<w:styles><w:style w:type="paragraph" w:styleId="h1"><w:name w:val="heading 1"/></w:style></w:styles>"#;
        let sheet = super::super::styles::parse_styles(
            &super::super::xml_dom::parse(styles_xml).unwrap(),
        );
        let xml = wrap(r#"<w:p><w:r><w:t>正文</w:t></w:r></w:p>"#);
        let (out, _) = apply_edits(
            &xml,
            &sheet,
            &[EditOp::InsertParagraphAfter {
                block: 1,
                expect_prefix: "正文".into(),
                text: "新标题".into(),
                style: Some("heading 1".into()),
            }],
        )
        .unwrap();
        assert!(out.contains(r#"<w:pPr><w:pStyle w:val="h1"/></w:pPr>"#), "显示名反查 ID");
        // 未知样式报错
        let err = val_msg(apply_edits(
            &xml,
            &sheet,
            &[EditOp::InsertParagraphAfter {
                block: 1,
                expect_prefix: "正文".into(),
                text: "x".into(),
                style: Some("不存在的样式".into()),
            }],
        )
        .unwrap_err());
        assert!(err.starts_with("未知样式"), "实际: {err}");
    }

    #[test]
    fn delete_block_removes_span_only() {
        let xml = wrap(
            r#"<w:p><w:r><w:t>一</w:t></w:r></w:p><w:p><w:r><w:t>二</w:t></w:r></w:p><w:p><w:r><w:t>三</w:t></w:r></w:p>"#,
        );
        let styles = Stylesheet::empty();
        let (out, _) = apply_edits(
            &xml,
            &styles,
            &[EditOp::DeleteBlock { block: 2, expect_prefix: "二".into() }],
        )
        .unwrap();
        let m = model_of(&out);
        assert_eq!(m.body.len(), 2);
        let mut t = String::new();
        docx_model::blocks_text(&m.body, &mut t);
        assert_eq!(t, "一\n三\n");
        // 区间外字节原样：两侧块 XML 不变
        assert!(out.contains("<w:p><w:r><w:t>一</w:t></w:r></w:p>"));
        assert!(out.contains("<w:p><w:r><w:t>三</w:t></w:r></w:p>"));
    }

    #[test]
    fn multi_ops_descending_splice() {
        // 一批三操作：删 5 / 改 3 / 插 1 后——原始偏移经降序 splice 全部正确落点
        let body: String = (1..=5)
            .map(|i| format!("<w:p><w:r><w:t>第{i}段</w:t></w:r></w:p>"))
            .collect();
        let xml = wrap(&body);
        let styles = Stylesheet::empty();
        let (out, applied) = apply_edits(
            &xml,
            &styles,
            &[
                EditOp::DeleteBlock { block: 5, expect_prefix: "第5段".into() },
                EditOp::ReplaceText { block: 3, expect_prefix: "第3段".into(), new_text: "改后".into() },
                EditOp::InsertParagraphAfter {
                    block: 1,
                    expect_prefix: "第1段".into(),
                    text: "新插".into(),
                    style: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 3);
        let m = model_of(&out);
        let mut t = String::new();
        docx_model::blocks_text(&m.body, &mut t);
        assert_eq!(t, "第1段\n新插\n第2段\n改后\n第4段\n");
    }

    #[test]
    fn fingerprint_mismatch_rejects_whole_batch() {
        let xml = wrap(r#"<w:p><w:r><w:t>实际内容</w:t></w:r></w:p>"#);
        let styles = Stylesheet::empty();
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[
                EditOp::ReplaceText { block: 1, expect_prefix: "别的内容".into(), new_text: "x".into() },
            ],
        )
        .unwrap_err());
        assert!(err.starts_with("指纹不符"), "实际: {err}");
        assert!(err.contains("inspect_docx"));
    }

    #[test]
    fn revision_and_sectpr_blocks_rejected() {
        let styles = Stylesheet::empty();
        // 修订块
        let xml = wrap(r#"<w:p><w:ins><w:r><w:t>修订段</w:t></w:r></w:ins></w:p>"#);
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::DeleteBlock { block: 1, expect_prefix: "修订段".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("含修订标记"), "实际: {err}");
        // 节末段
        let xml = wrap(
            r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="1"/></w:sectPr></w:pPr><w:r><w:t>节末段</w:t></w:r></w:p>"#,
        );
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::DeleteBlock { block: 1, expect_prefix: "节末段".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("节属性保护"), "实际: {err}");
    }

    #[test]
    fn duplicate_block_and_bounds_rejected() {
        let xml = wrap(r#"<w:p><w:r><w:t>一</w:t></w:r></w:p>"#);
        let styles = Stylesheet::empty();
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[
                EditOp::ReplaceText { block: 1, expect_prefix: "一".into(), new_text: "x".into() },
                EditOp::DeleteBlock { block: 1, expect_prefix: "x".into() },
            ],
        )
        .unwrap_err());
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::DeleteBlock { block: 7, expect_prefix: "".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("块号越界"), "实际: {err}");
        assert!(err.contains("1-1"), "应带有效范围: {err}");
    }

    #[test]
    fn table_block_replace_rejected() {
        let xml = wrap(r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
        let styles = Stylesheet::empty();
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::ReplaceText { block: 1, expect_prefix: "表".into(), new_text: "x".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("表格块"), "实际: {err}");
    }

    #[test]
    fn newline_and_tab_in_inserted_text() {
        let xml = wrap(r#"<w:p><w:r><w:t>锚</w:t></w:r></w:p>"#);
        let styles = Stylesheet::empty();
        let (out, _) = apply_edits(
            &xml,
            &styles,
            &[EditOp::InsertParagraphAfter {
                block: 1,
                expect_prefix: "锚".into(),
                text: "甲\n乙\t丙".into(),
                style: None,
            }],
        )
        .unwrap();
        let m = model_of(&out);
        let mut t = String::new();
        docx_model::blocks_text(&m.body, &mut t);
        assert_eq!(t, "锚\n甲\n乙\t丙\n");
    }

    #[test]
    fn repack_keeps_untouched_entries_byte_identical() {
        // docx-rs 造真实包 → 重打包（document.xml 原文）→ 逐 entry 内容字节相等
        use docx_rs::{Docx, Document, Paragraph, Run};
        let document = Document::new()
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第一段")))
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第二段")));
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        Docx::new().document(document).build().pack(&mut cursor).unwrap();
        let original = cursor.into_inner();

        let xml = docx::read_document_xml(&original).unwrap();
        let repacked = repack_document(&original, &xml).unwrap();

        // 解包对比逐 entry 内容字节
        let mut orig_zip = zip::ZipArchive::new(Cursor::new(&original)).unwrap();
        let mut new_zip = zip::ZipArchive::new(Cursor::new(&repacked)).unwrap();
        assert_eq!(orig_zip.len(), new_zip.len(), "entry 数一致");
        for i in 0..orig_zip.len() {
            let (oname, odata) = read_entry_at(&mut orig_zip, i);
            let (nname, ndata) = read_entry_at(&mut new_zip, i);
            assert_eq!(oname, nname, "entry 名一致");
            assert_eq!(odata, ndata, "entry {oname} 内容应逐字节相等");
        }

        // document.xml 替换后其余 entry 仍相等
        let new_xml = xml.replace("第一段", "改后段");
        assert_ne!(new_xml, xml);
        let repacked2 = repack_document(&original, &new_xml).unwrap();
        let mut new_zip2 = zip::ZipArchive::new(Cursor::new(&repacked2)).unwrap();
        for i in 0..orig_zip.len() {
            let (oname, odata) = read_entry_at(&mut orig_zip, i);
            let (nname, ndata) = read_entry_at(&mut new_zip2, i);
            assert_eq!(oname, nname);
            if oname == "word/document.xml" {
                assert_ne!(odata, ndata, "document.xml 应已替换");
            } else {
                assert_eq!(odata, ndata, "untouched entry {oname} 应逐字节相等");
            }
        }
        // 替换后可正常提取
        let extracted = docx::read_document_xml(&repacked2).unwrap();
        assert!(extracted.contains("改后段"));
    }

    fn read_entry_at<R: std::io::Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, i: usize) -> (String, Vec<u8>) {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().to_string();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        (name, data)
    }
}
