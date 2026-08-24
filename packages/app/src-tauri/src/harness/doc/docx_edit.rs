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
//! `insert_paragraph_after` / `delete_block` / `set_style` / `set_format`。
//! 表格内容操作后续批。

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

/// 段落级格式（set_format 用；None 字段不动）。单位与 format 投影显示一致——
/// agent 读到什么单位就写什么单位（诚实回环）：行距=倍数 / 段前后=pt / 缩进=twips。
#[derive(Debug, Clone, Default)]
pub struct ParaFormat {
    /// w:jc val：left/center/right/both(两端)/distribute(分散)/start/end
    pub align: Option<String>,
    /// 倍数行距（1.5 → 1.5 倍行距；w:line = 倍×240，lineRule=auto）
    pub line_spacing: Option<f32>,
    /// 段前间距（pt；twips = pt×20）
    pub space_before_pt: Option<f32>,
    /// 段后间距（pt）
    pub space_after_pt: Option<f32>,
    /// 首行缩进（twips，非负；1 字符 ≈ 240tw）
    pub indent_first_line_tw: Option<i32>,
    /// 左缩进（twips，可负=悬挂出格）
    pub indent_left_tw: Option<i32>,
}

impl ParaFormat {
    fn is_empty(&self) -> bool {
        self.align.is_none()
            && self.line_spacing.is_none()
            && self.space_before_pt.is_none()
            && self.space_after_pt.is_none()
            && self.indent_first_line_tw.is_none()
            && self.indent_left_tw.is_none()
    }
}

/// 字符级格式（set_format 用；应用到段落内**每个 run**；None 字段不动）。
#[derive(Debug, Clone, Default)]
pub struct CharFormat {
    /// 加粗（true=加粗 false=显式不加粗——压过样式链）
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    /// 字号（pt；w:sz 半磅 = pt×2，sz/szCs 同步）
    pub font_size_pt: Option<f32>,
    /// 颜色（RRGGBB 六位十六进制）
    pub color: Option<String>,
    /// 字体族（同时写 eastAsia/ascii/hAnsi——中文文档常态是同名字体）
    pub font: Option<String>,
}

impl CharFormat {
    fn is_empty(&self) -> bool {
        self.bold.is_none()
            && self.italic.is_none()
            && self.font_size_pt.is_none()
            && self.color.is_none()
            && self.font.is_none()
    }
}

/// 一个编辑操作（块号 1-based，与 inspect_docx 编址一致）。
#[derive(Debug, Clone)]
pub enum EditOp {
    /// 替换段落文本：保留 pPr 与首个 run 的 rPr（周边格式不动）；表格块拒绝。
    ReplaceText { block: usize, expect_prefix: String, new_text: String },
    /// 在锚块后插入新段：style（显示名，可选）指定样式；缺省继承锚块段落格式。
    InsertParagraphAfter { block: usize, expect_prefix: String, text: String, style: Option<String> },
    /// 删除整块（含段落标记）；块内含 sectPr（节属性载体）拒绝。
    DeleteBlock { block: usize, expect_prefix: String },
    /// 改段落样式（标题升降级等）：只动 pStyle 一个元素，正文 run 字节不动；
    /// 表格块拒绝。style 接受显示名或样式 ID（inspect_docx outline 样式列口径）。
    SetStyle { block: usize, expect_prefix: String, style: String },
    /// 改段落/字符格式：pPr 三元素（spacing/ind/jc）属性级合并 + 每个 run 的
    /// rPr（b/i/sz/color/rFonts）；未提及的属性原样保留；表格块拒绝。
    SetFormat {
        block: usize,
        expect_prefix: String,
        paragraph: Option<ParaFormat>,
        character: Option<CharFormat>,
    },
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
    /// set_style 专用：应用后的样式 ID（其余操作 None，序列化省略）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
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
            "操作列表为空: edit_docx 至少需要一个操作。请提供 replace_text / insert_paragraph_after / delete_block / set_style / set_format 操作。".into(),
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
            | EditOp::DeleteBlock { block, .. }
            | EditOp::SetStyle { block, .. }
            | EditOp::SetFormat { block, .. } => block,
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
            EditOp::SetStyle { style, .. } => {
                if styles.id_of(style).is_none() {
                    return Err(AppError::Validation(format!(
                        "未知样式: {:?} 不在本文档样式表中。可用样式（前 20）: {}。\
                         样式名接受显示名或 ID，来自 inspect_docx outline 的样式列。",
                        style,
                        styles.display_names_joined(20)
                    )));
                }
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                // pPrChange 是段落属性修订记录（内含旧 pStyle）——换样式会与其语义
                // 纠缠，MVP 与修订块同判拒绝
                if block_xml.contains("<w:pPrChange") {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带段落属性修订（pPrChange），默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                // sectPr 在 pPr 内被原样保留（手术只动 pStyle 元素），改样式不破坏分节——放行
            }
            EditOp::SetFormat { paragraph, character, .. } => {
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                if block_xml.contains("<w:pPrChange") {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带段落属性修订（pPrChange），默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                validate_formats(paragraph.as_ref(), character.as_ref())?;
            }
        }
        // Replace / SetStyle / SetFormat 目标必须是段落块
        if matches!(
            op,
            EditOp::ReplaceText { .. } | EditOp::SetStyle { .. } | EditOp::SetFormat { .. }
        ) && span.is_table {
            return Err(AppError::Validation(format!(
                "表格块: 块 {block} 是表格，该操作只支持段落。\
                 表格内容编辑暂不支持（后续批次）；可用 inspect_docx text 查看表格内容。"
            )));
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
                        style: None,
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
                        style: None,
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
                        style: None,
                    },
                });
            }
            EditOp::SetStyle { block, style, .. } => {
                let span = spans[block - 1];
                // 预检已校验样式存在；此处重解析拿 ID（批内样式表不可变，无 TOCTOU）
                let style_id = styles.id_of(&style).expect("预检已校验样式存在");
                let new_block =
                    restyle_paragraph(&xml[span.start..span.end], style_id)
                        .ok_or_else(|| AppError::Internal(format!(
                            "段落改样式失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        )))?;
                let projected = projected_of(&model, block);
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: new_block,
                    summary: AppliedOp {
                        op: "set_style",
                        block,
                        before: projected.clone(),
                        after: projected, // 文本不变；样式见 style 字段
                        style: Some(style_id.to_string()),
                    },
                });
            }
            EditOp::SetFormat { block, paragraph, character, .. } => {
                let span = spans[block - 1];
                let mut new_block = xml[span.start..span.end].to_string();
                if let Some(para) = &paragraph {
                    new_block = reformat_ppr(&new_block, para)
                        .ok_or_else(|| AppError::Internal(format!(
                            "段落格式手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        )))?;
                }
                if let Some(ch) = &character {
                    new_block = reformat_runs(&new_block, ch)
                        .ok_or_else(|| AppError::Internal(format!(
                            "字符格式手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        )))?;
                }
                let projected = projected_of(&model, block);
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: new_block,
                    summary: AppliedOp {
                        op: "set_format",
                        block,
                        before: projected.clone(),
                        // 文本不变；after 携带本次应用的格式摘要（agent 读回验证）
                        after: truncate(&describe_formats(paragraph.as_ref(), character.as_ref()), 60),
                        style: None,
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
        | EditOp::DeleteBlock { expect_prefix, .. }
        | EditOp::SetStyle { expect_prefix, .. }
        | EditOp::SetFormat { expect_prefix, .. } => expect_prefix,
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

/// 段落改样式手术：只动 pStyle 一个元素，pPr 其余子元素 / rPr / run 字节一概不碰。
/// 三形态：已有 pStyle（首个 = 生效样式，schema 序在 pPrChange 前）→ 整元素替换；
/// 有 pPr 无 pStyle → pStyle 是 pPr 的 schema 首子元素，插开标签后即正确位置；
/// 无 pPr → 开标签后新建（自闭合空段 `<w:p/>` 顺势展开成对标签）。
/// 返回 None 仅防御形态异常（调用方报内部错误）。
fn restyle_paragraph(block_xml: &str, style_id: &str) -> Option<String> {
    let tag = format!(r#"<w:pStyle w:val="{style_id}"/>"#);
    // 1) 已有 pStyle：定位首个（自闭合或配对形式均支持）
    if let Some(s) = block_xml.find("<w:pStyle") {
        let gt = s + block_xml[s..].find('>')? + 1; // '>' 后一位
        let e = if block_xml.as_bytes()[gt - 2] == b'/' {
            gt // 自闭合：元素止于 '/>'
        } else {
            gt + block_xml[gt..].find("</w:pStyle>")? + "</w:pStyle>".len()
        };
        return Some(format!("{}{tag}{}", &block_xml[..s], &block_xml[e..]));
    }
    // 2) 有 pPr：精确到 `<w:pPr` 后跟 '>' / '/' / 空格（排除 `<w:pPrChange`——字母 C 跟随）
    let ppr_at = block_xml.find("<w:pPr").filter(|s| {
        matches!(
            block_xml.as_bytes().get(s + "<w:pPr".len()),
            Some(b'>') | Some(b'/') | Some(b' ')
        )
    });
    if let Some(s) = ppr_at {
        let gt = s + block_xml[s..].find('>')? + 1;
        // 自闭合空 pPr（Word 不写，防御）：整体换成含 pStyle 的完整 pPr
        if block_xml.as_bytes()[gt - 2] == b'/' {
            return Some(format!(
                "{}<w:pPr>{tag}</w:pPr>{}",
                &block_xml[..s],
                &block_xml[gt..]
            ));
        }
        return Some(format!("{}{tag}{}", &block_xml[..gt], &block_xml[gt..]));
    }
    // 3) 无 pPr：开标签后新建；自闭合空段展开
    let gt = block_xml.find('>')?;
    if block_xml.as_bytes()[gt - 1] == b'/' {
        let head = &block_xml[..gt - 1]; // 去掉 '/'
        return Some(format!("{head}><w:pPr>{tag}</w:pPr></w:p>"));
    }
    Some(format!(
        "{}<w:pPr>{tag}</w:pPr>{}",
        &block_xml[..gt + 1],
        &block_xml[gt + 1..]
    ))
}

/// set_format 值域校验（家族前缀稳定：空格式操作/对齐值无效/颜色值无效/…）。
fn validate_formats(para: Option<&ParaFormat>, ch: Option<&CharFormat>) -> AppResult<()> {
    let empty_para = para.is_none_or(|p| p.is_empty());
    let empty_ch = ch.is_none_or(|c| c.is_empty());
    if empty_para && empty_ch {
        return Err(AppError::Validation(
            "空格式操作: set_format 未提供任何要修改的字段。\
             paragraph（对齐/行距/段前后/缩进）与 character（粗斜/字号/颜色/字体）\
             至少一项内有字段。"
                .into(),
        ));
    }
    const ALIGNS: [&str; 7] = ["left", "center", "right", "both", "distribute", "start", "end"];
    if let Some(p) = para {
        if let Some(v) = &p.align {
            if !ALIGNS.contains(&v.as_str()) {
                return Err(AppError::Validation(format!(
                    "对齐值无效: {v:?}。可用 left/center/right/both(两端对齐)/distribute(分散对齐)/start/end。"
                )));
            }
        }
        if let Some(v) = p.line_spacing {
            if !v.is_finite() || v <= 0.0 {
                return Err(AppError::Validation(format!(
                    "行距值无效: {v}。应为正数倍数（1.5 = 1.5 倍行距）。"
                )));
            }
        }
        for (label, v) in [("段前", p.space_before_pt), ("段后", p.space_after_pt)] {
            if let Some(v) = v {
                if !v.is_finite() || !(0.0..=1000.0).contains(&v) {
                    return Err(AppError::Validation(format!(
                        "间距值无效: {label}={v}pt。合法范围 0-1000pt。"
                    )));
                }
            }
        }
        if let Some(v) = p.indent_first_line_tw {
            if v < 0 {
                return Err(AppError::Validation(format!(
                    "缩进值无效: 首行缩进 first_line={v}tw。应为非负 twips（1 字符 ≈ 240tw；\
                     悬挂缩进请在 Word 中或经样式设置）。"
                )));
            }
        }
    }
    if let Some(c) = ch {
        if let Some(v) = c.font_size_pt {
            if !v.is_finite() || !(1.0..=400.0).contains(&v) {
                return Err(AppError::Validation(format!(
                    "字号超出范围: {v}pt。合法范围 1-400pt。"
                )));
            }
        }
        if let Some(v) = &c.color {
            let ok = v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit());
            if !ok {
                return Err(AppError::Validation(format!(
                    "颜色值无效: {v:?}。应为 6 位十六进制 RRGGBB（如 FF0000 红色）。"
                )));
            }
        }
    }
    Ok(())
}

/// applied 摘要：本次应用的格式清单（如「段落(对齐=center 行距=1.5倍) 字符(粗 14pt)」）。
fn describe_formats(para: Option<&ParaFormat>, ch: Option<&CharFormat>) -> String {
    let mut parts = Vec::new();
    if let Some(p) = para {
        let mut seg = Vec::new();
        if let Some(v) = &p.align {
            seg.push(format!("对齐={v}"));
        }
        if let Some(v) = p.line_spacing {
            seg.push(format!("行距={v}倍"));
        }
        if let Some(v) = p.space_before_pt {
            seg.push(format!("段前={v}pt"));
        }
        if let Some(v) = p.space_after_pt {
            seg.push(format!("段后={v}pt"));
        }
        if let Some(v) = p.indent_first_line_tw {
            seg.push(format!("首行缩进={v}tw"));
        }
        if let Some(v) = p.indent_left_tw {
            seg.push(format!("左缩进={v}tw"));
        }
        if !seg.is_empty() {
            parts.push(format!("段落({})", seg.join(" ")));
        }
    }
    if let Some(c) = ch {
        let mut seg = Vec::new();
        if let Some(v) = c.bold {
            seg.push(if v { "加粗" } else { "取消加粗" }.to_string());
        }
        if let Some(v) = c.italic {
            seg.push(if v { "斜体" } else { "取消斜体" }.to_string());
        }
        if let Some(v) = c.font_size_pt {
            seg.push(format!("字号={v}pt"));
        }
        if let Some(v) = &c.color {
            seg.push(format!("颜色=#{v}"));
        }
        if let Some(v) = &c.font {
            seg.push(format!("字体={v}"));
        }
        if !seg.is_empty() {
            parts.push(format!("字符({})", seg.join(" ")));
        }
    }
    if parts.is_empty() {
        "（无字段）".into()
    } else {
        parts.join(" ")
    }
}

// =========================================================================
// set_format 手术辅助（元素定位 / 属性合并 / schema 序插入）
// =========================================================================

/// 在 `s` 中找名为 `w:{name}` 的元素字节区间（`[start, end)`，自闭合或配对形式）。
/// 撞名前缀（找 w:b 会先撞 w:bCs / w:bdr）按「后随字符是字母则跳过继续找」排除。
/// None = 不存在。
fn find_element_span(s: &str, name: &str) -> Option<(usize, usize)> {
    let pat = format!("<w:{name}");
    let mut from = 0usize;
    loop {
        let rel = s[from..].find(&pat)?;
        let start = from + rel;
        let after = start + pat.len();
        let next_ch = s[after..].chars().next()?;
        if next_ch.is_ascii_alphanumeric() {
            from = after; // w:b 撞上 w:bCs——继续找下一个出现
            continue;
        }
        // 开标签止于 '>'（自闭合时前一字符是 '/'）
        let gt = after + s[after..].find('>')?;
        let end = if s.as_bytes()[gt - 1] == b'/' {
            gt + 1
        } else {
            let close = format!("</w:{name}>");
            gt + 1 + s[gt + 1..].find(&close)? + close.len()
        };
        return Some((start, end));
    }
}

/// 解析元素开标签的属性表（`name="value"` 对；仅到首个 '>'）。
fn parse_attrs(el: &str) -> Vec<(String, String)> {
    let gt = el.find('>').unwrap_or(el.len());
    let tag = &el[..gt];
    let mut out = Vec::new();
    let mut rest = tag;
    while let Some(eq) = rest.find('=') {
        let head = &rest[..eq];
        let name = head.trim_end().rsplit(char::is_whitespace).next().unwrap_or("").to_string();
        let after = &rest[eq + 1..];
        let quote = after.chars().next();
        if quote != Some('"') {
            rest = &rest[eq + 1..];
            continue;
        }
        let Some(end_rel) = after[1..].find('"') else { break };
        let value = after[1..1 + end_rel].to_string();
        if !name.is_empty() {
            out.push((name, value));
        }
        rest = &after[end_rel + 2..];
    }
    out
}

/// 由属性表构造标准自闭合标签（属性序 = 传入序）。
fn build_tag(name: &str, attrs: &[(String, String)]) -> String {
    if attrs.is_empty() {
        return format!("<w:{name}/>");
    }
    let inner: Vec<String> = attrs.iter().map(|(k, v)| format!(r#"{k}="{v}""#)).collect();
    format!("<w:{name} {}/>", inner.join(" "))
}

/// 属性表 set/remove（原地）。
fn attr_set(attrs: &mut Vec<(String, String)>, key: &str, val: &str) {
    if let Some(a) = attrs.iter_mut().find(|(k, _)| k == key) {
        a.1 = val.to_string();
    } else {
        attrs.push((key.to_string(), val.to_string()));
    }
}
fn attr_remove(attrs: &mut Vec<(String, String)>, key: &str) {
    attrs.retain(|(k, _)| k != key);
}

/// 在父元素内容里替换或按 schema 序插入一个属性元素。
/// - 已存在 → 整元素替换为 `new_tag`
/// - 不存在 → 插到 `later`（schema 序中排在后面的兄弟元素名）最早出现处之前，
///   或 `end_marker`（如 `</w:pPr>`）之前
fn upsert_element(parent: &str, name: &str, new_tag: &str, later: &[&str], end_marker: &str) -> String {
    if let Some((s, e)) = find_element_span(parent, name) {
        return format!("{}{new_tag}{}", &parent[..s], &parent[e..]);
    }
    let mut insert_at = parent.find(end_marker).unwrap_or(parent.len());
    for later_name in later {
        if let Some(p) = parent.find(&format!("<w:{later_name}")) {
            insert_at = insert_at.min(p);
        }
    }
    format!("{}{new_tag}{}", &parent[..insert_at], &parent[insert_at..])
}

/// pPr 内容里应用段落格式（spacing/ind 属性级合并——未提及的属性原样保留）。
fn apply_para_formats(ppr_inner: &str, p: &ParaFormat) -> String {
    let mut out = ppr_inner.to_string();
    // spacing（行距/段前后共用一个元素，必须合并不是覆盖）
    if p.line_spacing.is_some() || p.space_before_pt.is_some() || p.space_after_pt.is_some() {
        let mut attrs = match find_element_span(&out, "spacing") {
            Some((s, e)) => parse_attrs(&out[s..e]),
            None => Vec::new(),
        };
        if let Some(v) = p.line_spacing {
            attr_set(&mut attrs, "w:line", &format!("{}", (v * 240.0).round() as i64));
            attr_set(&mut attrs, "w:lineRule", "auto");
        }
        if let Some(v) = p.space_before_pt {
            attr_set(&mut attrs, "w:before", &format!("{}", (v * 20.0).round() as i64));
            attr_remove(&mut attrs, "w:beforeLines");
            attr_remove(&mut attrs, "w:beforeAutospacing");
        }
        if let Some(v) = p.space_after_pt {
            attr_set(&mut attrs, "w:after", &format!("{}", (v * 20.0).round() as i64));
            attr_remove(&mut attrs, "w:afterLines");
            attr_remove(&mut attrs, "w:afterAutospacing");
        }
        let tag = build_tag("spacing", &attrs);
        out = upsert_element(&out, "spacing", &tag, &["ind", "jc", "rPr", "sectPr", "pPrChange"], "</w:pPr>");
    }
    // ind（首行/左缩进合并；两变体互斥需清对方）
    if p.indent_first_line_tw.is_some() || p.indent_left_tw.is_some() {
        let mut attrs = match find_element_span(&out, "ind") {
            Some((s, e)) => parse_attrs(&out[s..e]),
            None => Vec::new(),
        };
        if let Some(v) = p.indent_first_line_tw {
            attr_set(&mut attrs, "w:firstLine", &v.to_string());
            for k in ["w:hanging", "w:firstLineChars", "w:hangingChars"] {
                attr_remove(&mut attrs, k);
            }
        }
        if let Some(v) = p.indent_left_tw {
            attr_set(&mut attrs, "w:left", &v.to_string());
            attr_remove(&mut attrs, "w:leftChars");
        }
        let tag = build_tag("ind", &attrs);
        out = upsert_element(&out, "ind", &tag, &["jc", "rPr", "sectPr", "pPrChange"], "</w:pPr>");
    }
    // jc（单属性，整元素替换/插入）
    if let Some(v) = &p.align {
        let tag = format!(r#"<w:jc w:val="{v}"/>"#);
        out = upsert_element(&out, "jc", &tag, &["textDirection", "outlineLvl", "rPr", "sectPr", "pPrChange"], "</w:pPr>");
    }
    out
}

/// rPr 内容里应用字符格式（b/bCs、i/iCs、sz/szCs 成对；color 单元素；rFonts 合并）。
fn apply_char_formats(rpr_inner: &str, c: &CharFormat) -> String {
    let mut out = rpr_inner.to_string();
    if let Some(v) = c.bold {
        let tag = if v { "<w:b/>".to_owned() } else { r#"<w:b w:val="0"/>"#.to_owned() };
        out = upsert_element(&out, "b", &tag, &["i", "color", "sz", "u"], "</w:rPr>");
        let tag_cs = if v { "<w:bCs/>".to_owned() } else { r#"<w:bCs w:val="0"/>"#.to_owned() };
        out = upsert_element(&out, "bCs", &tag_cs, &["i", "color", "sz", "u"], "</w:rPr>");
    }
    if let Some(v) = c.italic {
        let tag = if v { "<w:i/>".to_owned() } else { r#"<w:i w:val="0"/>"#.to_owned() };
        out = upsert_element(&out, "i", &tag, &["color", "sz", "u"], "</w:rPr>");
        let tag_cs = if v { "<w:iCs/>".to_owned() } else { r#"<w:iCs w:val="0"/>"#.to_owned() };
        out = upsert_element(&out, "iCs", &tag_cs, &["color", "sz", "u"], "</w:rPr>");
    }
    if let Some(v) = c.font_size_pt {
        let half = (v * 2.0).round() as i64;
        out = upsert_element(
            &out,
            "sz",
            &format!(r#"<w:sz w:val="{half}"/>"#),
            &["szCs", "highlight", "u"],
            "</w:rPr>",
        );
        out = upsert_element(
            &out,
            "szCs",
            &format!(r#"<w:szCs w:val="{half}"/>"#),
            &["highlight", "u"],
            "</w:rPr>",
        );
    }
    if let Some(v) = &c.color {
        out = upsert_element(
            &out,
            "color",
            &format!(r#"<w:color w:val="{v}"/>"#),
            &["sz", "u"],
            "</w:rPr>",
        );
    }
    if let Some(v) = &c.font {
        let mut attrs = match find_element_span(&out, "rFonts") {
            Some((s, e)) => parse_attrs(&out[s..e]),
            None => Vec::new(),
        };
        attr_set(&mut attrs, "w:ascii", v);
        attr_set(&mut attrs, "w:hAnsi", v);
        attr_set(&mut attrs, "w:eastAsia", v);
        let tag = build_tag("rFonts", &attrs);
        out = upsert_element(&out, "rFonts", &tag, &["b", "i", "color", "sz", "u"], "</w:rPr>");
    }
    out
}

/// 全新 pPr 内容（无 pPr / 自闭合空 pPr 两分支共用；schema 序 spacing→ind→jc）。
fn fresh_ppr_inner(p: &ParaFormat) -> String {
    let mut inner = String::new();
    let mut attrs: Vec<(String, String)> = Vec::new();
    if let Some(v) = p.line_spacing {
        attr_set(&mut attrs, "w:line", &format!("{}", (v * 240.0).round() as i64));
        attr_set(&mut attrs, "w:lineRule", "auto");
    }
    if let Some(v) = p.space_before_pt {
        attr_set(&mut attrs, "w:before", &format!("{}", (v * 20.0).round() as i64));
    }
    if let Some(v) = p.space_after_pt {
        attr_set(&mut attrs, "w:after", &format!("{}", (v * 20.0).round() as i64));
    }
    if !attrs.is_empty() {
        inner.push_str(&build_tag("spacing", &attrs));
    }
    let mut ind_attrs: Vec<(String, String)> = Vec::new();
    if let Some(v) = p.indent_first_line_tw {
        ind_attrs.push(("w:firstLine".into(), v.to_string()));
    }
    if let Some(v) = p.indent_left_tw {
        ind_attrs.push(("w:left".into(), v.to_string()));
    }
    if !ind_attrs.is_empty() {
        inner.push_str(&build_tag("ind", &ind_attrs));
    }
    if let Some(v) = &p.align {
        inner.push_str(&format!(r#"<w:jc w:val="{v}"/>"#));
    }
    inner
}

/// 段落格式手术：定位/新建 pPr，应用段落格式；其余字节不动。
/// None 仅防御形态异常。
fn reformat_ppr(block_xml: &str, p: &ParaFormat) -> Option<String> {
    // 定位 pPr（精确 `<w:pPr` + 非字母后随，排除 pPrChange）
    let ppr_at = block_xml.find("<w:pPr").filter(|s| {
        matches!(
            block_xml.as_bytes().get(s + "<w:pPr".len()),
            Some(b'>') | Some(b'/') | Some(b' ')
        )
    });
    let Some(s) = ppr_at else {
        // 无 pPr：开标签后新建（自闭合空段顺势展开成对标签）
        let gt = block_xml.find('>')?;
        let new_ppr = format!("<w:pPr>{}</w:pPr>", fresh_ppr_inner(p));
        if block_xml.as_bytes()[gt - 1] == b'/' {
            let head = &block_xml[..gt - 1];
            return Some(format!("{head}>{new_ppr}</w:p>"));
        }
        return Some(format!("{}{new_ppr}{}", &block_xml[..gt + 1], &block_xml[gt + 1..]));
    };
    // 有 pPr：内容区间内应用
    let open_end = s + block_xml[s..].find('>')? + 1;
    if block_xml.as_bytes()[open_end - 2] == b'/' {
        // 自闭合空 pPr（防御）：整体替换为含格式的完整 pPr
        return Some(format!(
            "{}<w:pPr>{}</w:pPr>{}",
            &block_xml[..s],
            fresh_ppr_inner(p),
            &block_xml[open_end..]
        ));
    }
    let close_rel = block_xml[open_end..].find("</w:pPr>")?;
    let close_at = open_end + close_rel;
    let inner = &block_xml[open_end..close_at];
    let new_inner = apply_para_formats(inner, p);
    Some(format!(
        "{}{}{}",
        &block_xml[..open_end],
        new_inner,
        &block_xml[close_at..]
    ))
}

/// 字符格式手术：块内每个 run（含 hyperlink 内）的 rPr 应用字符格式；
/// 无 rPr 的 run 在开标签后新建（rPr 是 w:r 的 schema 首子元素）。
/// None 仅防御形态异常。
fn reformat_runs(block_xml: &str, c: &CharFormat) -> Option<String> {
    // 收集 run 开标签区间（"<w:r>" / "<w:r "）——倒序应用使偏移始终有效。
    // "<w:rPr" / "<w:rFonts" 等因后随字母不会被 "<w:r " / "<w:r>" 误配。
    let mut run_starts: Vec<usize> = Vec::new();
    let mut from = 0usize;
    loop {
        let rel_a = block_xml[from..].find("<w:r>").map(|p| (from + p, "<w:r>".len()));
        let rel_b = block_xml[from..].find("<w:r ").map(|p| (from + p, "<w:r ".len()));
        let hit = match (rel_a, rel_b) {
            (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
            (a, b) => a.or(b),
        };
        let Some((pos, len)) = hit else { break };
        run_starts.push(pos);
        from = pos + len;
    }
    let mut out = block_xml.to_string();
    for &rs in run_starts.iter().rev() {
        // 本轮处理后 out 与 block_xml 等长前缀可能已变——倒序应用时后续（更小的）
        // 偏移仍未被触碰。但 rs 是 block_xml 偏移，out 只在更靠后的位置变过，
        // 因此 rs 在 out 中仍有效。
        let open_end = rs + out[rs..].find('>')? + 1;
        // 直接子 rPr：紧跟开标签
        if out[open_end..].starts_with("<w:rPr>") {
            let close = open_end + "<w:rPr>".len();
            let inner_end = close + out[close..].find("</w:rPr>")?;
            let new_inner = apply_char_formats(&out[close..inner_end], c);
            out.replace_range(close..inner_end, &new_inner);
        } else if out[open_end..].starts_with("<w:rPr ") || out[open_end..].starts_with("<w:rPr/") {
            // 带属性/自闭合的 rPr 罕见：保守跳过该 run（不动比错动好）
            continue;
        } else {
            // 无 rPr：新建（rPr 是 w:r 的 schema 首子元素；空内容起手 = 全新插入，
            // apply_char_formats 的追加路径天然按 schema 序生成）
            let inner = apply_char_formats("", c);
            if !inner.is_empty() {
                let fresh = format!("<w:rPr>{inner}</w:rPr>");
                out.insert_str(open_end, &fresh);
            }
        }
    }
    Some(out)
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

    fn heading_styles() -> Stylesheet {
        let styles_xml = r#"<w:styles>
            <w:style w:type="paragraph" w:styleId="h2"><w:name w:val="heading 2"/></w:style>
            <w:style w:type="paragraph" w:styleId="body"><w:name w:val="Normal"/></w:style>
        </w:styles>"#;
        super::super::styles::parse_styles(&super::super::xml_dom::parse(styles_xml).unwrap())
    }

    #[test]
    fn set_style_replaces_pstyle_val_only() {
        // 已有 pStyle：只换 val，pPr 其余子元素 / run / rPr 字节原样
        let xml = wrap(
            r#"<w:p w14:paraId="X1"><w:pPr><w:pStyle w:val="body"/><w:jc w:val="center"/></w:pPr>\
               <w:r><w:rPr><w:b/></w:rPr><w:t>正文段</w:t></w:r></w:p>"#,
        );
        let (out, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetStyle { block: 1, expect_prefix: "正文段".into(), style: "heading 2".into() }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "set_style");
        assert_eq!(applied[0].style.as_deref(), Some("h2"));
        assert!(out.contains(r#"<w:p w14:paraId="X1">"#), "开标签属性保留");
        assert!(out.contains(r#"<w:pPr><w:pStyle w:val="h2"/><w:jc w:val="center"/></w:pPr>"#));
        assert!(out.contains(r#"<w:rPr><w:b/></w:rPr><w:t>正文段</w:t></w:r>"#), "run 原样");
        // 模型侧：样式生效 + 文本不变
        let m = model_of(&out);
        let super::docx_model::Block::Paragraph(p) = &m.body[0] else { panic!() };
        assert_eq!(p.props.style.as_deref(), Some("h2"));
        let mut t = String::new();
        docx_model::blocks_text(&m.body, &mut t);
        assert_eq!(t, "正文段\n");
    }

    #[test]
    fn set_style_inserts_or_creates_ppr() {
        // 块 1：有 pPr 无 pStyle → 插在 pPr 开标签后（schema 首子元素位，先于 numPr）
        // 块 2：无 pPr → 开标签后新建；块 3：自闭合空段 → 展开成对标签
        let xml = wrap(concat!(
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="3"/></w:numPr></w:pPr><w:r><w:t>列表项</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>裸段</w:t></w:r></w:p>"#,
            r#"<w:p w14:paraId="E"/>"#,
        ));
        let (out, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[
                EditOp::SetStyle { block: 1, expect_prefix: "列表项".into(), style: "h2".into() },
                EditOp::SetStyle { block: 2, expect_prefix: "裸段".into(), style: "h2".into() },
                EditOp::SetStyle { block: 3, expect_prefix: "".into(), style: "body".into() },
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 3);
        assert!(
            out.contains(r#"<w:pPr><w:pStyle w:val="h2"/><w:numPr>"#),
            "pStyle 应插在 pPr 首位: {out}"
        );
        assert!(out.contains(r#"<w:p><w:pPr><w:pStyle w:val="h2"/></w:pPr><w:r><w:t>裸段</w:t></w:r></w:p>"#));
        assert!(out.contains(r#"<w:p w14:paraId="E"><w:pPr><w:pStyle w:val="body"/></w:pPr></w:p>"#));
        let m = model_of(&out);
        assert_eq!(m.body.len(), 3, "块数守恒");
    }

    #[test]
    fn set_style_unknown_and_table_rejected() {
        let styles = heading_styles();
        // 未知样式（家族前缀）
        let xml = wrap(r#"<w:p><w:r><w:t>一段</w:t></w:r></w:p>"#);
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::SetStyle { block: 1, expect_prefix: "一段".into(), style: "没有这样式".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("未知样式"), "实际: {err}");
        // 表格块
        let xml = wrap(r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::SetStyle { block: 1, expect_prefix: "表".into(), style: "h2".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("表格块"), "实际: {err}");
        // 段落属性修订（pPrChange）——换样式与其语义纠缠，拒
        let xml = wrap(
            r#"<w:p><w:pPr><w:pStyle w:val="body"/><w:pPrChange w:id="1"><w:pPr><w:pStyle w:val="h2"/></w:pPr></w:pPrChange></w:pPr><w:r><w:t>改格段</w:t></w:r></w:p>"#,
        );
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::SetStyle { block: 1, expect_prefix: "改格段".into(), style: "h2".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("含修订标记"), "实际: {err}");
        assert!(err.contains("pPrChange"));
    }

    #[test]
    fn set_format_paragraph_merges_existing_attrs() {
        // 已有 spacing(before)/ind(firstLine)：合并——未提及的 before 原样，新字段进同一元素
        let xml = wrap(concat!(
            r#"<w:p><w:pPr><w:pStyle w:val="body"/><w:spacing w:before="120"/>"#,
            r#"<w:ind w:firstLine="480"/></w:pPr><w:r><w:t>正文段</w:t></w:r></w:p>"#,
        ));
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetFormat {
                block: 1,
                expect_prefix: "正文段".into(),
                paragraph: Some(ParaFormat {
                    align: Some("center".into()),
                    line_spacing: Some(1.5),
                    space_after_pt: Some(6.0),
                    indent_first_line_tw: Some(720),
                    ..Default::default()
                }),
                character: None,
            }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "set_format");
        assert!(applied[0].after.contains("对齐=center"), "摘要: {}", applied[0].after);
        // 合并断言：before 未提及原样 120；line/lineRule/after 追加进同一 spacing；firstLine 覆盖 720
        assert!(
            out.contains(r#"<w:spacing w:before="120" w:line="360" w:lineRule="auto" w:after="120"/>"#),
            "spacing 合并: {out}"
        );
        assert!(out.contains(r#"<w:ind w:firstLine="720"/>"#), "ind 覆盖: {out}");
        assert!(out.contains(r#"<w:jc w:val="center"/>"#), "jc 插入: {out}");
        // schema 序：pStyle < spacing < ind < jc
        let ppr = out.find("<w:pPr>").unwrap();
        let end = out.find("</w:pPr>").unwrap();
        let seg = &out[ppr..end];
        let pos = |pat: &str| seg.find(pat).unwrap();
        assert!(pos("<w:pStyle") < pos("<w:spacing")
            && pos("<w:spacing") < pos("<w:ind")
            && pos("<w:ind") < pos("<w:jc"));
        // 模型读回
        let m = model_of(&out);
        let super::docx_model::Block::Paragraph(p) = &m.body[0] else { panic!() };
        assert_eq!(p.props.alignment.as_deref(), Some("center"));
        assert_eq!(p.props.spacing_line, Some(360));
        assert_eq!(p.props.line_rule.as_deref(), Some("auto"));
        assert_eq!(p.props.spacing_before, Some(120));
        assert_eq!(p.props.spacing_after, Some(120)); // 6pt × 20
        assert_eq!(p.props.indent_first_line, Some(720));
    }

    #[test]
    fn set_format_character_applies_to_all_runs() {
        // 两个 run：已有含 rFonts/u 的 rPr / 无 rPr——全量应用且无关子元素保留
        let xml = wrap(concat!(
            r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Times"/><w:u/></w:rPr><w:t>甲</w:t></w:r>"#,
            r#"<w:r><w:t>乙</w:t></w:r></w:p>"#,
        ));
        let (out, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetFormat {
                block: 1,
                expect_prefix: "甲".into(),
                paragraph: None,
                character: Some(CharFormat {
                    bold: Some(true),
                    font_size_pt: Some(14.0),
                    color: Some("FF0000".into()),
                    ..Default::default()
                }),
            }],
        )
        .unwrap();
        // run 1：rFonts 原样（本操作未设 font）+ u 保留；b/bCs/color/sz/szCs 按 schema 序插入
        assert!(out.contains(r#"<w:rFonts w:ascii="Times"/>"#), "rFonts 原样: {out}");
        assert!(out.contains("<w:u/>"), "无关子元素保留: {out}");
        assert_eq!(out.matches("<w:b/>").count(), 2, "两个 run 都加粗: {out}");
        assert_eq!(out.matches(r#"<w:sz w:val="28"/>"#).count(), 2, "14pt = 28 半磅: {out}");
        assert_eq!(out.matches(r#"<w:color w:val="FF0000"/>"#).count(), 2);
        // run 2 无 rPr → 新建（rPr 是 w:r 的 schema 首子元素）
        assert!(out.contains(r#"<w:r><w:rPr>"#), "无 rPr 的 run 新建 rPr: {out}");
        // 模型读回
        let m = model_of(&out);
        let super::docx_model::Block::Paragraph(p) = &m.body[0] else { panic!() };
        for r in &p.runs {
            assert_eq!(r.props.bold, Some(true));
            assert_eq!(r.props.size_half_pt, Some(28));
            assert_eq!(r.props.color.as_deref(), Some("FF0000"));
        }
    }

    #[test]
    fn set_format_creates_ppr_and_rpr_when_absent() {
        // 裸段：pPr/rPr 均新建；自闭合空段展开成对标签并新建 pPr
        let xml = wrap(concat!(
            r#"<w:p><w:r><w:t>裸段</w:t></w:r></w:p>"#,
            r#"<w:p/>"#,
        ));
        let (out, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::SetFormat {
                    block: 1,
                    expect_prefix: "裸段".into(),
                    paragraph: Some(ParaFormat {
                        align: Some("both".into()),
                        indent_first_line_tw: Some(480),
                        indent_left_tw: Some(-240),
                        ..Default::default()
                    }),
                    character: Some(CharFormat { bold: Some(true), ..Default::default() }),
                },
                EditOp::SetFormat {
                    block: 2,
                    expect_prefix: "".into(),
                    paragraph: Some(ParaFormat { line_spacing: Some(2.0), ..Default::default() }),
                    character: None,
                },
            ],
        )
        .unwrap();
        assert!(
            out.contains(r#"<w:p><w:pPr><w:spacing w:line="480" w:lineRule="auto"/></w:pPr></w:p>"#),
            "块 2 自闭合空段展开并新建 pPr: {out}"
        );
        let m = model_of(&out);
        assert_eq!(m.body.len(), 2);
        let super::docx_model::Block::Paragraph(p1) = &m.body[0] else { panic!() };
        assert_eq!(p1.props.alignment.as_deref(), Some("both"));
        assert_eq!(p1.props.indent_first_line, Some(480));
        assert_eq!(p1.props.indent_left, None, "负左缩进 i32 → 模型 u32 不收，XML 已写");
        assert!(out.contains(r#"w:left="-240""#), "负左缩进写入: {out}");
        assert_eq!(p1.runs[0].props.bold, Some(true));
        let super::docx_model::Block::Paragraph(p2) = &m.body[1] else { panic!() };
        assert_eq!(p2.props.spacing_line, Some(480));
    }

    #[test]
    fn set_format_validation_families() {
        let xml = wrap(r#"<w:p><w:r><w:t>一段</w:t></w:r></w:p>"#);
        let empty_op = |paragraph, character| EditOp::SetFormat {
            block: 1,
            expect_prefix: "一段".into(),
            paragraph,
            character,
        };
        // 空格式
        let err = val_msg(apply_edits(&xml, &Stylesheet::empty(), &[empty_op(None, None)]).unwrap_err());
        assert!(err.starts_with("空格式操作"), "实际: {err}");
        let err = val_msg(
            apply_edits(&xml, &Stylesheet::empty(), &[empty_op(Some(ParaFormat::default()), None)]).unwrap_err(),
        );
        assert!(err.starts_with("空格式操作"), "实际: {err}");
        // 对齐白名单
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[empty_op(
                    Some(ParaFormat { align: Some("middle".into()), ..Default::default() }),
                    None,
                )],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("对齐值无效"), "实际: {err}");
        // 颜色 hex
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[empty_op(
                    None,
                    Some(CharFormat { color: Some("RED".into()), ..Default::default() }),
                )],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("颜色值无效"), "实际: {err}");
        // 字号范围
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[empty_op(
                    None,
                    Some(CharFormat { font_size_pt: Some(999.0), ..Default::default() }),
                )],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("字号超出范围"), "实际: {err}");
        // 负首行缩进
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[empty_op(
                    Some(ParaFormat { indent_first_line_tw: Some(-10), ..Default::default() }),
                    None,
                )],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("缩进值无效"), "实际: {err}");
        // 表格块（前缀对准表内文字，确保落在表格拒绝而非指纹不符）
        let tbl = wrap(r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
        let err = val_msg(
            apply_edits(
                &tbl,
                &Stylesheet::empty(),
                &[EditOp::SetFormat {
                    block: 1,
                    expect_prefix: "表".into(),
                    paragraph: None,
                    character: Some(CharFormat { bold: Some(true), ..Default::default() }),
                }],
            )
            .unwrap_err(),
        );
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
