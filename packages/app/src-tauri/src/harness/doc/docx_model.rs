//! `docx_model` —— docx 类型化结构模型（word-capability-roadmap 步骤 1 / S0a）。
//!
//! 旧路径（docx.rs 扫描器）只出线性文本流；本模块把 `word/document.xml` 解析成
//! 类型树（段落 / run / 表格网格 / 节属性 / 修订标记），供 inspect_docx 三级投影
//! 与 edit_docx 外科手术使用。**text 投影由模型派生**，且必须与旧扫描器逐字节
//! 相等（零回归硬闸，见 docx.rs golden 测试 + corpus_tests）。
//!
//! 覆盖面原则（与扫描器一致）：已知容器结构化（w:p / w:tbl / w:hyperlink / w:ins /
//! w:sdt / 嵌套表），**未知容器透明递归**——不因「不认识」而丢文本；绘图/文本框
//! 等奇异子树线性摊平进所属段落（保文本，结构留待后续阶段）。
//!
//! 修订语义：w:ins 内的 run 标 `Inserted`（文本计入投影）；w:del 内的 run 文本来自
//! w:delText，标 `Deleted`（模型记录、投影剔除）——「读侧 w:ins 计入、w:delText 剔除」。

use super::xml_dom::{Element, Node};

// =========================================================================
// 模型类型
// =========================================================================

/// 一份 docx 的正文结构（styles.xml / numbering.xml 合并在 S0a-2 接入）。
pub(super) struct DocxDocument {
    /// 正文块序列（段落 / 表格，按文档顺序；含 sdt 等透明容器摊平后的内容）
    pub body: Vec<Block>,
    /// 节属性（多节文档按出现序；最后节的 sectPr 挂在 body 尾，节中断在段内 pPr）
    pub sections: Vec<SectionProps>,
}

pub(super) enum Block {
    Paragraph(Paragraph),
    Table(Table),
}

pub(super) struct Paragraph {
    /// run 序列；tab/br 已内联为 \t / \n（与旧扫描器的文本语义一致）
    pub runs: Vec<Run>,
    /// 段落直接格式（S0b inspect_docx 投影消费；有效格式 = 样式链合并，见 styles.rs）
    pub props: ParaProps,
}

pub(super) struct Run {
    pub text: String,
    /// run 直接字符格式（S0b inspect_docx 投影消费）
    pub props: RunProps,
    /// 修订标记：Some(Inserted) 在 w:ins 内；Some(Deleted) 在 w:del 内（文本来自 w:delText）
    pub revision: Option<Revision>,
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub(super) enum Revision {
    Inserted,
    Deleted,
}

/// 段落直接格式 + 引用（w:pPr）。有效格式（样式链合并）见 styles.rs。
#[derive(Default, Clone)]
pub(super) struct ParaProps {
    /// w:pStyle val（样式 ID，非显示名）
    pub style: Option<String>,
    /// w:numPr（numId + ilvl；numId=0 表示无编号）
    pub numbering: Option<NumRef>,
    /// w:jc val（left/center/right/both/...）
    pub alignment: Option<String>,
    /// w:spacing w:line + w:lineRule（auto: line/240=倍数；at/exact: line/20=pt）
    pub spacing_line: Option<u32>,
    pub line_rule: Option<String>,
    /// w:spacing w:before / w:after（twips，1/20 pt）
    pub spacing_before: Option<u32>,
    pub spacing_after: Option<u32>,
    /// w:ind w:firstLine / w:hanging / w:left（twips）
    pub indent_first_line: Option<u32>,
    pub indent_hanging: Option<u32>,
    pub indent_left: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NumRef {
    pub num_id: u32,
    pub ilvl: u32,
}

/// run 直接字符格式（w:rPr）。三值语义：None=继承（样式链/默认），Some=直接指定。
#[derive(Default, Clone)]
pub(super) struct RunProps {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    /// w:sz / w:szCs（半磅单位，如 21 = 10.5pt）
    pub size_half_pt: Option<u32>,
    /// w:color val（hex RGB 或 "auto"）
    pub color: Option<String>,
    /// w:highlight val
    pub highlight: Option<String>,
    /// w:rFonts w:eastAsia（中文字体）
    pub font_east_asia: Option<String>,
    /// w:rFonts w:ascii（西文字体）
    pub font_ascii: Option<String>,
}

pub(super) struct Table {
    pub rows: Vec<TableRow>,
}

pub(super) struct TableRow {
    pub cells: Vec<TableCell>,
}

pub(super) struct TableCell {
    /// 单元格内容是块级序列（段落 / 嵌套表）
    pub blocks: Vec<Block>,
    /// w:gridSpan（横向合并占的列数）
    pub grid_span: Option<u32>,
    /// w:vMerge：Some("restart")=纵向合并头，Some("continue")=被合并续格，None=不合并
    pub v_merge: Option<String>,
}

/// 节属性（w:sectPr）。尺寸单位 twips（1/20 pt）。
#[derive(Default)]
pub(super) struct SectionProps {
    pub page_w: Option<u32>,
    pub page_h: Option<u32>,
    /// w:pgSz w:orient（portrait/landscape）
    pub orientation: Option<String>,
    pub margin_top: Option<u32>,
    pub margin_bottom: Option<u32>,
    pub margin_left: Option<u32>,
    pub margin_right: Option<u32>,
    /// (类型, r:id)：类型 default/first/even。rId → 部件名的映射走 rels（后续阶段接）
    pub header_refs: Vec<(String, String)>,
    pub footer_refs: Vec<(String, String)>,
}

// =========================================================================
// DOM → 模型
// =========================================================================

/// 从 `w:document` 根元素构建模型。
///
/// 根形态宽容（测试便利）：`w:document` → 找 body；`w:body` / 单块根（`w:p` /
/// `w:tbl`）/ 其他容器 → 直接按块遍历。
pub(super) fn build_document(root: &Element) -> DocxDocument {
    let mut doc = DocxDocument {
        body: Vec::new(),
        sections: Vec::new(),
    };
    match root.name.as_str() {
        "w:document" => {
            if let Some(body) = root.child_elements().find(|e| e.name == "w:body") {
                walk_blocks(&body.children, &mut doc.body, &mut doc.sections);
            }
        }
        "w:p" => {
            doc.body.push(Block::Paragraph(parse_paragraph(root, &mut doc.sections)));
        }
        "w:tbl" => doc.body.push(Block::Table(parse_table(root))),
        _ => walk_blocks(&root.children, &mut doc.body, &mut doc.sections),
    }
    doc
}

/// 块级遍历：w:p → 段落；w:tbl → 表格；w:sectPr → 节属性；其余透明递归。
fn walk_blocks(nodes: &[Node], out: &mut Vec<Block>, sections: &mut Vec<SectionProps>) {
    for n in nodes {
        let Node::Element(el) = n else { continue };
        match el.name.as_str() {
            "w:p" => out.push(Block::Paragraph(parse_paragraph(el, sections))),
            "w:tbl" => out.push(Block::Table(parse_table(el))),
            "w:sectPr" => sections.push(parse_sect_pr(el)),
            // 无块级语义的标记（书签 / 拼写检查标记等）直接跳过
            "w:bookmarkStart" | "w:bookmarkEnd" | "w:proofErr" => {}
            // 未知容器（w:sdt / 内容控件等）透明递归——覆盖面与旧扫描器一致
            _ => walk_blocks(&el.children, out, sections),
        }
    }
}

fn parse_paragraph(el: &Element, sections: &mut Vec<SectionProps>) -> Paragraph {
    let mut runs = Vec::new();
    let mut props = ParaProps::default();
    for child in el.child_elements() {
        match child.name.as_str() {
            "w:pPr" => {
                props = parse_para_props(child);
                // 段内节中断（本段是节末段）：pPr 内的 sectPr 一样计入
                if let Some(s) = child.child_elements().find(|e| e.name == "w:sectPr") {
                    sections.push(parse_sect_pr(s));
                }
            }
            _ => collect_runs(child, RunCtx::default(), &mut runs),
        }
    }
    Paragraph { runs, props }
}

/// 解析 w:pPr（styles.rs 的样式内 pPr 复用同一解析）。
pub(super) fn parse_para_props(p_pr: &Element) -> ParaProps {
    let mut props = ParaProps::default();
    for child in p_pr.child_elements() {
        match child.name.as_str() {
            "w:pStyle" => props.style = child.attr("w:val").map(str::to_string),
            "w:jc" => props.alignment = child.attr("w:val").map(str::to_string),
            "w:spacing" => {
                props.spacing_line = child.attr("w:line").and_then(|v| v.parse().ok());
                props.line_rule = child.attr("w:lineRule").map(str::to_string);
                props.spacing_before = child.attr("w:before").and_then(|v| v.parse().ok());
                props.spacing_after = child.attr("w:after").and_then(|v| v.parse().ok());
            }
            "w:ind" => {
                props.indent_first_line = child.attr("w:firstLine").and_then(|v| v.parse().ok());
                props.indent_hanging = child.attr("w:hanging").and_then(|v| v.parse().ok());
                props.indent_left = child.attr("w:left").and_then(|v| v.parse().ok());
            }
            "w:numPr" => {
                // numId=0 是「显式无编号」，建模为 None
                let num_id = child
                    .child_elements()
                    .find(|e| e.name == "w:numId")
                    .and_then(|e| e.attr("w:val"))
                    .and_then(|v| v.parse::<u32>().ok());
                let ilvl = child
                    .child_elements()
                    .find(|e| e.name == "w:ilvl")
                    .and_then(|e| e.attr("w:val"))
                    .and_then(|v| v.parse::<u32>().ok());
                if let Some(num_id) = num_id {
                    if num_id != 0 {
                        props.numbering = Some(NumRef { num_id, ilvl: ilvl.unwrap_or(0) });
                    }
                }
            }
            _ => {}
        }
    }
    props
}

/// 内联收集上下文：随 w:ins / w:del 容器传递修订标记。
#[derive(Default, Clone, Copy)]
struct RunCtx {
    revision: Option<Revision>,
}

/// 段内内联遍历：w:r → run；w:ins/w:del → 带修订标记递归；超链接等容器透明递归。
fn collect_runs(el: &Element, ctx: RunCtx, runs: &mut Vec<Run>) {
    match el.name.as_str() {
        "w:r" => runs.push(parse_run(el, ctx)),
        "w:ins" => {
            let ctx = RunCtx { revision: Some(Revision::Inserted) };
            for child in el.child_elements() {
                collect_runs(child, ctx, runs);
            }
        }
        "w:del" => {
            let ctx = RunCtx { revision: Some(Revision::Deleted) };
            for child in el.child_elements() {
                collect_runs(child, ctx, runs);
            }
        }
        // 无内联语义的标记跳过
        "w:bookmarkStart" | "w:bookmarkEnd" | "w:proofErr" => {}
        // w:hyperlink / w:smartTag / w:customXml / w:sdt / w:fldSimple / 未知容器：透明递归
        _ => {
            for child in el.child_elements() {
                collect_runs(child, ctx, runs);
            }
        }
    }
}

fn parse_run(el: &Element, ctx: RunCtx) -> Run {
    let mut text = String::new();
    let mut props = RunProps::default();
    for child in el.child_elements() {
        match child.name.as_str() {
            "w:rPr" => props = parse_run_props(child),
            "w:t" => decode_entities_into(&child.raw_text(), &mut text),
            // 删除修订的文本载体是 w:delText（仅在 del 上下文收集）
            "w:delText" if ctx.revision == Some(Revision::Deleted) => {
                decode_entities_into(&child.raw_text(), &mut text)
            }
            "w:tab" => text.push('\t'),
            "w:br" | "w:cr" => text.push('\n'),
            // 绘图 / 文本框 / 嵌套对象等奇异子树：线性摊平保文本（w:p 边界 → \n）
            _ => flatten_exotic_inline(child, &mut text),
        }
    }
    Run { text, props, revision: ctx.revision }
}

/// 奇异子树（w:drawing / w:pict / mc:AlternateContent / 未知元素）的兜底文本收集：
/// 任何深度的 w:t / tab / br 照旧扫描器语义收集，内层段落边界补 `\n`。
/// 结构化（图片标记 / 文本框建模）留待后续阶段，文本保真优先。
fn flatten_exotic_inline(el: &Element, text: &mut String) {
    for child in el.child_elements() {
        match child.name.as_str() {
            "w:t" | "w:delText" => decode_entities_into(&child.raw_text(), text),
            "w:tab" => text.push('\t'),
            "w:br" | "w:cr" => text.push('\n'),
            "w:p" => {
                flatten_exotic_inline(child, text);
                text.push('\n'); // 内层段落结束 → 换行（与扫描器 </w:p> 语义一致）
            }
            _ => flatten_exotic_inline(child, text),
        }
    }
}

/// on/off 类属性：无 val=true；val="false"/"0"/"none"/"off"=false；其余=true。
fn on_off(el: &Element) -> Option<bool> {
    match el.attr("w:val") {
        Some("false") | Some("0") | Some("none") | Some("off") => Some(false),
        _ => Some(true),
    }
}

/// 解析 w:rPr（styles.rs 的样式内 rPr 复用同一解析）。
pub(super) fn parse_run_props(r_pr: &Element) -> RunProps {
    let mut props = RunProps::default();
    for child in r_pr.child_elements() {
        match child.name.as_str() {
            "w:b" => props.bold = on_off(child),
            "w:i" => props.italic = on_off(child),
            "w:u" => props.underline = on_off(child),
            "w:strike" => props.strike = on_off(child),
            "w:sz" => {
                if let Some(v) = child.attr("w:val").and_then(|v| v.parse::<u32>().ok()) {
                    props.size_half_pt = Some(v);
                }
            }
            "w:color" => props.color = child.attr("w:val").map(str::to_string),
            "w:highlight" => props.highlight = child.attr("w:val").map(str::to_string),
            "w:rFonts" => {
                props.font_east_asia = child.attr("w:eastAsia").map(str::to_string);
                props.font_ascii = child.attr("w:ascii").map(str::to_string);
            }
            _ => {}
        }
    }
    props
}

fn parse_table(el: &Element) -> Table {
    let rows = el
        .child_elements()
        .filter(|c| c.name == "w:tr")
        .map(|tr| {
            let cells = tr
                .child_elements()
                .filter(|c| c.name == "w:tc")
                .map(parse_cell)
                .collect();
            TableRow { cells }
        })
        .collect();
    Table { rows }
}

fn parse_cell(el: &Element) -> TableCell {
    let mut blocks = Vec::new();
    // 单元格内不会再有 sectPr；透明容器复用同一遍历
    let mut dummy_sections = Vec::new();
    walk_blocks(&el.children, &mut blocks, &mut dummy_sections);
    let (mut grid_span, mut v_merge) = (None, None);
    if let Some(tc_pr) = el.child_elements().find(|c| c.name == "w:tcPr") {
        grid_span = tc_pr
            .child_elements()
            .find(|c| c.name == "w:gridSpan")
            .and_then(|c| c.attr("w:val"))
            .and_then(|v| v.parse::<u32>().ok());
        v_merge = tc_pr
            .child_elements()
            .find(|c| c.name == "w:vMerge")
            .and_then(|c| c.attr("w:val").map(str::to_string))
            // <w:vMerge/> 无 val = continue（OOXML 规范）
            .or_else(|| {
                tc_pr
                    .child_elements()
                    .find(|c| c.name == "w:vMerge")
                    .map(|_| "continue".to_string())
            });
    }
    TableCell { blocks, grid_span, v_merge }
}

fn parse_sect_pr(el: &Element) -> SectionProps {
    let mut s = SectionProps::default();
    for child in el.child_elements() {
        match child.name.as_str() {
            "w:pgSz" => {
                s.page_w = child.attr("w:w").and_then(|v| v.parse().ok());
                s.page_h = child.attr("w:h").and_then(|v| v.parse().ok());
                s.orientation = child.attr("w:orient").map(str::to_string);
            }
            "w:pgMar" => {
                s.margin_top = child.attr("w:top").and_then(|v| v.parse().ok());
                s.margin_bottom = child.attr("w:bottom").and_then(|v| v.parse().ok());
                s.margin_left = child.attr("w:left").and_then(|v| v.parse().ok());
                s.margin_right = child.attr("w:right").and_then(|v| v.parse().ok());
            }
            "w:headerReference" => {
                if let (Some(kind), Some(rid)) = (child.attr("w:type"), child.attr("r:id")) {
                    s.header_refs.push((kind.to_string(), rid.to_string()));
                }
            }
            "w:footerReference" => {
                if let (Some(kind), Some(rid)) = (child.attr("w:type"), child.attr("r:id")) {
                    s.footer_refs.push((kind.to_string(), rid.to_string()));
                }
            }
            _ => {}
        }
    }
    s
}

// =========================================================================
// text 投影（模型派生；须与旧扫描器逐字节相等）
// =========================================================================

/// 整篇文本投影（**未 normalize**，调用方沿用现有管线）。
/// 段落各占一行（含表格单元格段落）；删除修订的文本剔除；表格无额外分隔。
pub(super) fn document_text(doc: &DocxDocument) -> String {
    let mut out = String::new();
    blocks_text(&doc.body, &mut out);
    out
}

fn blocks_text(blocks: &[Block], out: &mut String) {
    for b in blocks {
        match b {
            Block::Paragraph(p) => {
                for r in &p.runs {
                    if r.revision != Some(Revision::Deleted) {
                        out.push_str(&r.text);
                    }
                }
                out.push('\n');
            }
            Block::Table(t) => {
                for row in &t.rows {
                    for cell in &row.cells {
                        blocks_text(&cell.blocks, out);
                    }
                }
            }
        }
    }
}

// =========================================================================
// XML 实体解码（自 docx.rs 迁入：模型是生产路径，扫描器降为 golden 参考）
// =========================================================================

/// 把一段（不含 tag 的）纯文本中的 XML 实体解码后追加到 `out`。
///
/// 支持：`&amp; &lt; &gt; &quot; &apos;` 与 `&#DD;` / `&#xHH;`。
/// 不认识的实体原样保留 `&`。
pub(super) fn decode_entities_into(input: &str, out: &mut String) {
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
            // 未找到合法实体 → 原样输出 '&'
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
// 单元测试（结构语义；真实语料的零回归断言见 corpus_tests）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::super::xml_dom;
    use super::*;

    fn model(xml_body: &str) -> DocxDocument {
        let xml = format!(
            r#"<w:document xmlns:w="w" xmlns:r="r"><w:body>{}</w:body></w:document>"#,
            xml_body
        );
        let dom = xml_dom::parse(&xml).unwrap();
        build_document(&dom)
    }

    fn text(doc: &DocxDocument) -> String {
        document_text(doc)
    }

    #[test]
    fn paragraphs_and_props() {
        let doc = model(r#"<w:p><w:pPr><w:pStyle w:val="2"/><w:jc w:val="center"/><w:numPr><w:ilvl w:val="1"/><w:numId w:val="3"/></w:numPr></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="32"/><w:color w:val="FF0000"/><w:rFonts w:eastAsia="黑体" w:ascii="Times"/></w:rPr><w:t>标题</w:t></w:r></w:p>"#);
        let Block::Paragraph(p) = &doc.body[0] else { panic!("应为段落") };
        assert_eq!(p.props.style.as_deref(), Some("2"));
        assert_eq!(p.props.alignment.as_deref(), Some("center"));
        assert_eq!(p.props.numbering, Some(NumRef { num_id: 3, ilvl: 1 }));
        let r = &p.runs[0];
        assert_eq!(r.text, "标题");
        assert_eq!(r.props.bold, Some(true));
        assert_eq!(r.props.size_half_pt, Some(32));
        assert_eq!(r.props.color.as_deref(), Some("FF0000"));
        assert_eq!(r.props.font_east_asia.as_deref(), Some("黑体"));
        assert_eq!(r.props.font_ascii.as_deref(), Some("Times"));
        assert_eq!(text(&doc), "标题\n");
    }

    #[test]
    fn on_off_false_values() {
        let doc = model(r#"<w:p><w:r><w:rPr><w:b w:val="false"/><w:i w:val="0"/></w:rPr><w:t>x</w:t></w:r></w:p>"#);
        let Block::Paragraph(p) = &doc.body[0] else { panic!() };
        assert_eq!(p.runs[0].props.bold, Some(false));
        assert_eq!(p.runs[0].props.italic, Some(false));
    }

    #[test]
    fn revision_semantics() {
        // w:ins 文本计入；w:del 文本（w:delText）剔除但保留在模型
        let doc = model(r#"<w:p><w:r><w:t>保留</w:t></w:r><w:ins><w:r><w:t>新增</w:t></w:r></w:ins><w:del><w:r><w:delText>旧文</w:delText></w:r></w:del></w:p>"#);
        let Block::Paragraph(p) = &doc.body[0] else { panic!() };
        assert_eq!(p.runs.len(), 3);
        assert_eq!(p.runs[1].revision, Some(Revision::Inserted));
        assert_eq!(p.runs[2].revision, Some(Revision::Deleted));
        assert_eq!(p.runs[2].text, "旧文");
        assert_eq!(text(&doc), "保留新增\n");
    }

    #[test]
    fn table_grid_and_nesting() {
        let doc = model(
            r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>B1</w:t></w:r></w:p></w:tc></w:tr>\
               <w:tr><w:tc><w:p><w:r><w:t>A2</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p><w:r><w:t></w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        );
        let Block::Table(t) = &doc.body[0] else { panic!() };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0].cells[1].grid_span, Some(2));
        // vMerge：显式 restart / 无 val = continue（OOXML 规范）
        assert_eq!(t.rows[0].cells[1].v_merge.as_deref(), Some("restart"));
        assert_eq!(t.rows[1].cells[1].v_merge.as_deref(), Some("continue"));
        // 单元格段落逐行输出（旧扫描器语义：表格内容线性流）
        assert_eq!(text(&doc), "A1\nB1\nA2\n\n");
    }

    #[test]
    fn nested_table_inside_cell() {
        let doc = model(
            r#"<w:tbl><w:tr><w:tc><w:tbl><w:tr><w:tc><w:p><w:r><w:t>内层</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>外层</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        );
        let Block::Table(t) = &doc.body[0] else { panic!() };
        let Block::Table(inner) = &t.rows[0].cells[0].blocks[0] else { panic!() };
        assert_eq!(text(&doc), "内层\n外层\n");
        assert_eq!(inner.rows.len(), 1);
    }

    #[test]
    fn transparent_containers_flatten() {
        // sdt（内容控件，TOC 常用）/ hyperlink 透明摊平，文本不丢
        let doc = model(r#"<w:sdt><w:sdtContent><w:p><w:r><w:t>目录项</w:t></w:r></w:p></w:sdtContent></w:sdt><w:p><w:hyperlink><w:r><w:t>链接文字</w:t></w:r></w:hyperlink></w:p>"#);
        assert_eq!(doc.body.len(), 2);
        assert_eq!(text(&doc), "目录项\n链接文字\n");
    }

    #[test]
    fn exotic_drawing_text_flattened() {
        // 文本框藏在 w:drawing 内：文本保真，内层段落边界 → \n
        let doc = model(r#"<w:p><w:r><w:t>前</w:t></w:r><w:r><w:drawing><w:pict><w:txbxContent><w:p><w:r><w:t>框内</w:t></w:r></w:p></w:txbxContent></w:pict></w:drawing></w:r><w:r><w:t>后</w:t></w:r></w:p>"#);
        assert_eq!(text(&doc), "前框内\n后\n");
    }

    #[test]
    fn section_props_collected() {
        let doc = model(
            r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838" w:orient="portrait"/><w:headerReference w:type="default" r:id="rId4"/></w:sectPr></w:pPr><w:r><w:t>节1末段</w:t></w:r></w:p>\
               <w:sectPr><w:pgMar w:top="1440" w:bottom="1440" w:left="1800" w:right="1800"/><w:footerReference w:type="first" r:id="rId8"/></w:sectPr>"#,
        );
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[0].page_w, Some(11906));
        assert_eq!(doc.sections[0].header_refs, vec![("default".to_string(), "rId4".to_string())]);
        assert_eq!(doc.sections[1].margin_left, Some(1800));
        assert_eq!(doc.sections[1].footer_refs[0].0, "first");
    }

    #[test]
    fn tab_stop_definitions_not_text() {
        // pPr 里的 tab 停靠点定义（格式元数据）不产生文本——旧扫描器的幻影 \t
        // 缺陷在 S0a 有意修复（见 corpus_tests::strip_tab_stops）
        let doc = model(r#"<w:p><w:pPr><w:tabs><w:tab w:val="left" w:pos="864"/><w:tab w:val="right" w:pos="9350"/></w:tabs></w:pPr><w:r><w:t>A</w:t><w:tab/><w:t>B</w:t></w:r></w:p>"#);
        assert_eq!(text(&doc), "A\tB\n");
    }

    #[test]
    fn num_id_zero_is_none() {
        let doc = model(r#"<w:p><w:pPr><w:numPr><w:numId w:val="0"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
        let Block::Paragraph(p) = &doc.body[0] else { panic!() };
        assert!(p.props.numbering.is_none());
    }
}
