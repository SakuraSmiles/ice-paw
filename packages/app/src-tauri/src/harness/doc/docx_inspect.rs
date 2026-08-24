//! `docx_inspect` —— inspect_docx 三级投影（word-capability-roadmap 步骤 2 / S0b）。
//!
//! 「调格式」的前提是「看得见格式」。read_file 对 docx 只给线性文本（结构全丢）；
//! 本模块在 S0a 类型树上出三档投影，按 token 预算分级：
//! - **outline**：每块一行的全局地图（块号 + 样式/层级 + 文本摘要）
//! - **format**：区间内 run 级有效格式（样式链合并后的值 = 「这段实际长什么样」）
//! - **text**：带块号前缀的正文（块号即行首地址）
//!
//! **块编址是步骤 3 edit_docx 的地址地基**：body 顺序 1-based（段落与表格混排统一
//! 编号），三档投影同一编址——outline 定位 → format 看格式 → text 引用，闭环。
//!
//! 纯函数（bytes 进报告出，无 IO）；工具薄壳在 mcp::docx_tool。

use crate::error::{AppError, AppResult};

use std::collections::HashMap;

use super::docx;
use super::docx_model::{self, Block, Revision};
use super::numbering::{self, NumberingCatalog};
use super::styles::{self, Stylesheet};
use super::xml_dom;

/// 投影档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectProjection {
    /// 大纲地图：每块一行（样式 + 层级 + 摘要），默认全量
    Outline,
    /// run 级格式：区间内段落/表格的有效格式明细，默认前 50 块
    Format,
    /// 带块号的正文，默认前 100 块
    Text,
    /// 页眉页脚：逐节列 header/footer 引用与部件内容（S3 首波④；按节组织，
    /// 不按块区间——start/end 不适用）
    HeadersFooters,
}

impl InspectProjection {
    /// 单次默认渲染的块数上限（token 分级：outline 行短可多、format 行长必须少）。
    fn default_span(self) -> usize {
        match self {
            InspectProjection::Outline => 400,
            InspectProjection::Format => 50,
            InspectProjection::Text => 100,
            // 按节组织不走块区间路径（inspect_document 特判，span 不参与）
            InspectProjection::HeadersFooters => 1,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            InspectProjection::Outline => "outline",
            InspectProjection::Format => "format",
            InspectProjection::Text => "text",
            InspectProjection::HeadersFooters => "headers_footers",
        }
    }
}

/// inspect 请求。
pub struct InspectRequest {
    pub projection: InspectProjection,
    /// 起始块号（1-based，含）；None = 1
    pub start: Option<usize>,
    /// 结束块号（1-based，含）；None = start + span - 1（按投影档）
    pub end: Option<usize>,
}

/// inspect 报告。
#[derive(Debug)]
pub struct InspectReport {
    pub total_blocks: usize,
    /// 实际渲染的块区间（1-based 含端点）
    pub range: (usize, usize),
    /// 区间之后还有块（提示 agent 用 start 续读）
    pub has_more: bool,
    /// 投影正文（含文档摘要头）
    pub content: String,
}

/// 对 docx 字节流做结构投影。
pub fn inspect_document(bytes: &[u8], req: &InspectRequest) -> AppResult<InspectReport> {
    let doc_xml = docx::read_document_xml(bytes)?;
    let dom = xml_dom::parse(&doc_xml)?;
    let model = docx_model::build_document(&dom);
    // styles.xml 可选部件：缺失/损坏 → 空表（有效格式退化为直接值，不挡主路径）
    let styles = match docx::read_entry(bytes, "word/styles.xml")? {
        Some(xml) => xml_dom::parse(&xml)
            .map(|dom| styles::parse_styles(&dom))
            .unwrap_or_else(|_| Stylesheet::empty()),
        None => Stylesheet::empty(),
    };
    // numbering.xml 可选部件：解析编号定义 → 文档顺序计数模拟出实际编号值
    // （"3.2.1"/"一、"——Word 渲染时才算的值，不接此处 agent 全盲）
    let numbers: HashMap<usize, String> = match docx::read_entry(bytes, "word/numbering.xml")? {
        Some(xml) => match xml_dom::parse(&xml) {
            Ok(dom) => {
                let catalog: NumberingCatalog = numbering::parse_numbering(&dom);
                numbering::compute_numbers(&model.body, &catalog)
            }
            Err(_) => HashMap::new(), // 损坏 → 回退引用形式显示，不挡主路径
        },
        None => HashMap::new(),
    };

    let total = model.body.len();
    // headers_footers 按节组织：不接受块区间参数，range = 节区间
    let (start, end, has_more) = if req.projection == InspectProjection::HeadersFooters {
        if req.start.is_some() || req.end.is_some() {
            return Err(AppError::Validation(
                "参数不适用：projection=headers_footers 按节组织，不接受 start/end。\
                 三档块区间投影（outline/format/text）才支持分页。"
                    .into(),
            ));
        }
        (1, model.sections.len().max(1), false)
    } else {
        resolve_range(total, req)?
    };

    let mut content = String::new();
    render_header(&model, total, &mut content);
    let ctx = RenderCtx { styles: &styles, numbers: &numbers };
    match req.projection {
        InspectProjection::Outline => {
            for (i, block) in model.body.iter().enumerate().take(end).skip(start - 1) {
                render_outline_line(&ctx, i + 1, block, &mut content);
            }
        }
        InspectProjection::Format => {
            for (i, block) in model.body.iter().enumerate().take(end).skip(start - 1) {
                render_format_block(&ctx, i + 1, block, &mut content);
            }
        }
        InspectProjection::Text => {
            for (i, block) in model.body.iter().enumerate().take(end).skip(start - 1) {
                render_text_block(&ctx, i + 1, block, &mut content);
            }
        }
        InspectProjection::HeadersFooters => {
            render_headers_footers(bytes, &model, &mut content)?;
        }
    }
    Ok(InspectReport { total_blocks: total, range: (start, end), has_more, content })
}

/// 块区间解析：缺省按投影档给 span；越界报错（三段式）/clamp。
fn resolve_range(total: usize, req: &InspectRequest) -> AppResult<(usize, usize, bool)> {
    let span = req.projection.default_span();
    let start = match req.start {
        Some(s) => {
            if s < 1 || s > total {
                return Err(AppError::Validation(format!(
                    "块号越界：start={s}，文档共 {total} 块（块号 1..={total}，1-based）。请先看 outline 投影确认目标块号。"
                )));
            }
            s
        }
        None => 1,
    };
    let end = match req.end {
        Some(e) => {
            if e < start {
                return Err(AppError::Validation(format!(
                    "区间无效：end={e} < start={start}。区间是 1-based 含端点，如 start=5 end=12 表示第 5..12 块。"
                )));
            }
            e.min(total)
        }
        None => (start + span - 1).min(total),
    };
    Ok((start, end, end < total))
}

// =========================================================================
// 渲染
// =========================================================================

struct RenderCtx<'a> {
    styles: &'a Stylesheet,
    /// 顶层段落块号 → 自动编号实际值（numbering.xml 计数模拟；空 = 无编号部件）
    numbers: &'a HashMap<usize, String>,
}

/// 文档摘要头（三档共用）：规模 / 节与页面 / 修订警告。
fn render_header(model: &docx_model::DocxDocument, total: usize, out: &mut String) {
    out.push_str(&format!("共 {total} 块（1-based，段落 ¶ 与表格 ▦ 混排统一编号）"));
    if !model.sections.is_empty() {
        let s = &model.sections[0];
        let page = match (s.page_w, s.page_h) {
            (Some(w), Some(h)) => format!("{}×{}cm", fmt_cm(w), fmt_cm(h)),
            _ => "尺寸未声明".to_string(),
        };
        let orient = s.orientation.as_deref().unwrap_or("portrait");
        out.push_str(&format!(" | {} 节 | 页面 {page} {orient}", model.sections.len()));
    }
    // 修订警告（roadmap 不变式：编辑默认不触碰修订 run——先让 agent 看见它们存在）
    let (ins, del) = count_revisions(&model.body);
    if ins + del > 0 {
        out.push_str(&format!(
            "\n⚠ 含修订标记：{ins} 处插入 / {del} 处删除（投影已按接受修订后的视图呈现；编辑操作默认不触碰修订 run）"
        ));
    }
    out.push('\n');
}

/// headers_footers 投影：逐节列页眉/页脚引用与部件文本（S3 首波④）。
///
/// Word 语义注（诚实呈现）：节未显式引用时沿用上一节（首节则无）；type
/// default=默认页 / first=首页（需 titlePg）/ even=偶数页（需 evenAndOddHeaders）。
/// 部件内的域（页码/日期等）按最后保存的缓存值显示（XML 里只有缓存结果）。
fn render_headers_footers(
    bytes: &[u8],
    model: &docx_model::DocxDocument,
    out: &mut String,
) -> AppResult<()> {
    let rels = docx::parse_header_footer_rels(bytes)?;
    out.push_str(&format!(
        "页眉页脚（{} 节；类型 default=默认页 first=首页 even=偶数页；未显式引用的节沿用上一节）\n",
        model.sections.len()
    ));
    if model.sections.is_empty() {
        out.push_str("（文档无分节信息——无页眉页脚）\n");
        return Ok(());
    }
    // 部件文本缓存：同一部件被多节引用只解析一次
    let mut cache: HashMap<String, String> = HashMap::new();
    for (i, sec) in model.sections.iter().enumerate() {
        out.push_str(&format!("\n[节 {}]\n", i + 1));
        let mut lines = 0usize;
        for (label, refs) in [("页眉", &sec.header_refs), ("页脚", &sec.footer_refs)] {
            for (typ, rid) in refs {
                let desc = match rels.get(rid) {
                    Some(part) => {
                        let text = part_text(bytes, part, &mut cache)?;
                        if text.is_empty() { "（空）".to_string() } else { indent_continuation(&text) }
                    }
                    None => format!("（引用 {rid} 在 rels 中悬空——部件缺失）"),
                };
                out.push_str(&format!("  {label} {typ}: {desc}\n"));
                lines += 1;
            }
        }
        if lines == 0 {
            out.push_str("  （本节未显式引用——Word 沿用上一节；首节则为空）\n");
        }
    }
    Ok(())
}

/// 读一个 header/footer 部件并投影为文本（缓存 miss 才解析）。
/// 部件根是 w:hdr/w:ftr——build_document 的「其他容器」分支直接按块遍历复用。
fn part_text(bytes: &[u8], part: &str, cache: &mut HashMap<String, String>) -> AppResult<String> {
    if let Some(t) = cache.get(part) {
        return Ok(t.clone());
    }
    let text = match docx::read_entry(bytes, part)? {
        Some(xml) => match xml_dom::parse(&xml) {
            Ok(dom) => {
                let m = docx_model::build_document(&dom);
                let mut t = String::new();
                docx_model::blocks_text(&m.body, &mut t);
                t.trim().to_string()
            }
            Err(_) => "（部件 XML 损坏）".to_string(),
        },
        None => format!("（部件 {part} 在包中缺失）"),
    };
    cache.insert(part.to_string(), text.clone());
    Ok(text)
}

/// 多行部件文本的续行缩进（投影行内层级对齐）。
fn indent_continuation(s: &str) -> String {
    s.replace('\n', "\n    ")
}

fn count_revisions(blocks: &[Block]) -> (usize, usize) {
    let mut counts = (0, 0);
    fn walk(blocks: &[Block], counts: &mut (usize, usize)) {
        for b in blocks {
            match b {
                Block::Paragraph(p) => {
                    for r in &p.runs {
                        match r.revision {
                            Some(Revision::Inserted) => counts.0 += 1,
                            Some(Revision::Deleted) => counts.1 += 1,
                            None => {}
                        }
                    }
                }
                Block::Table(t) => {
                    for row in &t.rows {
                        for cell in &row.cells {
                            walk(&cell.blocks, counts);
                        }
                    }
                }
            }
        }
    }
    walk(blocks, &mut counts);
    counts
}

/// outline：每块一行。
fn render_outline_line(ctx: &RenderCtx, n: usize, block: &Block, out: &mut String) {
    match block {
        Block::Paragraph(p) => {
            let text = para_text(p);
            let meta = para_meta(ctx, n, &p.props);
            if text.trim().is_empty() {
                out.push_str(&format!("[{n}] ¶ {meta} (空)\n"));
            } else {
                out.push_str(&format!("[{n}] ¶ {meta} {}\n", summarize(&text, 60)));
            }
        }
        Block::Table(t) => {
            let cols = t.rows.first().map(|r| r.cells.len()).unwrap_or(0);
            out.push_str(&format!(
                "[{n}] ▦ 表 {}行×{cols}列 | {}\n",
                t.rows.len(),
                table_row_summary(t.rows.first()),
            ));
        }
    }
}

/// 段落元信息：样式（显示名 + 大纲层级）/ 列表（有编号值显示实际值，否则引用形式）。
fn para_meta(ctx: &RenderCtx, n: usize, props: &docx_model::ParaProps) -> String {
    let mut parts: Vec<String> = Vec::new();
    match &props.style {
        Some(id) => {
            let name = ctx.styles.name_of(id).unwrap_or(id);
            match ctx.styles.outline_lvl_of(id) {
                Some(lvl) => parts.push(format!("H{} {name}", lvl + 1)),
                None => parts.push(format!("样式={name}")),
            }
        }
        None => parts.push("(无样式)".to_string()),
    }
    if let Some(num) = &props.numbering {
        match ctx.numbers.get(&n) {
            // 实际编号值（Word 渲染口径）：「列表 一、」/「列表 2.1)」
            Some(text) => parts.push(format!("列表 {text}")),
            // 引用无法解析（numId 不在目录/部件缺失）→ 原样引用，agent 可感知异常
            None => parts.push(format!("列表(num{},lvl{})", num.num_id, num.ilvl)),
        }
    }
    parts.join(" ")
}

/// format：块级格式明细（有效格式 = 样式链合并值）。
fn render_format_block(ctx: &RenderCtx, n: usize, block: &Block, out: &mut String) {
    match block {
        Block::Paragraph(p) => {
            out.push_str(&format!("[{n}] ¶ {}\n", para_meta(ctx, n, &p.props)));
            let chain = p
                .props
                .style
                .as_deref()
                .map(|id| ctx.styles.resolve_chain(id))
                .unwrap_or_default();
            let eff = styles::effective_para(&p.props, &chain, &ctx.styles.doc_default_para);
            out.push_str(&format!("     段落格式: {}\n", fmt_para_props(&eff)));
            let text = para_text(p);
            if text.trim().is_empty() {
                out.push_str("     文本: (空)\n");
            } else {
                out.push_str(&format!("     文本: {}\n", summarize(&text, 80)));
            }
            for (i, run) in p.runs.iter().enumerate() {
                if run.text.is_empty() {
                    continue;
                }
                let eff_run = styles::effective_run(&run.props, &chain, &ctx.styles.doc_default_run);
                let mut line = format!(
                    "     run {} \"{}\" → {}",
                    i + 1,
                    summarize(&run.text, 40),
                    fmt_effective_run(&eff_run),
                );
                if let Some(direct) = fmt_direct_run(&run.props) {
                    line.push_str(&format!(" [直接: {direct}]"));
                }
                match run.revision {
                    Some(Revision::Inserted) => line.push_str(" 〔插入修订〕"),
                    Some(Revision::Deleted) => line.push_str(" 〔删除修订〕"),
                    None => {}
                }
                out.push_str(&line);
                out.push('\n');
            }
        }
        Block::Table(t) => {
            let cols = t.rows.first().map(|r| r.cells.len()).unwrap_or(0);
            out.push_str(&format!("[{n}] ▦ 表格 {}行×{cols}列\n", t.rows.len()));
            const MAX_ROWS: usize = 15;
            for (ri, row) in t.rows.iter().enumerate().take(MAX_ROWS) {
                out.push_str(&format!("     r{}: {}\n", ri + 1, row_cells_summary(row)));
            }
            if t.rows.len() > MAX_ROWS {
                out.push_str(&format!("     … 还有 {} 行（表格行编辑见 edit_docx）\n", t.rows.len() - MAX_ROWS));
            }
        }
    }
}

/// text：带块号的正文（表格展开为格内段落行）；列表段带实际编号前缀（贴近 Word 视图）。
fn render_text_block(ctx: &RenderCtx, n: usize, block: &Block, out: &mut String) {
    match block {
        Block::Paragraph(p) => {
            let text = para_text(p);
            let num_prefix = ctx.numbers.get(&n).map(|t| format!("{t} ")).unwrap_or_default();
            if text.trim().is_empty() {
                out.push_str(&format!("[{n}] (空)\n"));
            } else {
                // 段内软换行（w:br）原样保留为多行
                for line in text.split('\n') {
                    out.push_str(&format!("[{n}] {num_prefix}{line}\n"));
                }
            }
        }
        Block::Table(t) => {
            let cols = t.rows.first().map(|r| r.cells.len()).unwrap_or(0);
            out.push_str(&format!("[{n}] ▦ 表格 {}行×{cols}列:\n", t.rows.len()));
            render_table_text(t, "     ", out);
        }
    }
}

/// 表格的 text 展开（含嵌套表递归——与 document_text 的全量语义对齐，不静默丢内容）。
fn render_table_text(t: &docx_model::Table, prefix: &str, out: &mut String) {
    for row in &t.rows {
        for cell in &row.cells {
            for b in &cell.blocks {
                match b {
                    Block::Paragraph(p) => {
                        let text = para_text(p);
                        if !text.trim().is_empty() {
                            out.push_str(&format!("{prefix}{}\n", summarize(&text, 80)));
                        }
                    }
                    Block::Table(nt) => {
                        out.push_str(&format!("{prefix}▦ 嵌套表:\n"));
                        render_table_text(nt, &format!("{prefix}  "), out);
                    }
                }
            }
        }
    }
}

// =========================================================================
// 格式化辅助（纯展示换算，单位见 OOXML：twips = 1/20 pt；半磅字号）
// =========================================================================

/// 段落可见文本（剔除删除修订——与 text 投影语义一致）。
fn para_text(p: &docx_model::Paragraph) -> String {
    let mut s = String::new();
    for r in &p.runs {
        if r.revision != Some(Revision::Deleted) {
            s.push_str(&r.text);
        }
    }
    s
}

fn fmt_cm(twips: u32) -> String {
    let cm = twips as f64 / 567.0;
    format!("{cm:.1}")
}

/// 摘要：压平换行/制表符并截断（按字符数）。
fn summarize(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in s.trim().chars() {
        if out.chars().count() >= max_chars {
            out.push('…');
            return out;
        }
        if ch == '\n' || ch == '\t' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

fn fmt_pt(half_pt: u32) -> String {
    let pt = half_pt as f64 / 2.0;
    if (pt - pt.round()).abs() < f64::EPSILON {
        format!("{}pt", pt.round() as u32)
    } else {
        format!("{pt:.1}pt")
    }
}

/// 有效 run 格式一行：如 `16pt 粗 斜体 #FF0000 黑体/Times`（None 字段省略）。
fn fmt_effective_run(p: &docx_model::RunProps) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(v) = p.size_half_pt {
        parts.push(fmt_pt(v));
    }
    if p.bold == Some(true) {
        parts.push("粗".into());
    }
    if p.italic == Some(true) {
        parts.push("斜".into());
    }
    if p.underline == Some(true) {
        parts.push("下划线".into());
    }
    if p.strike == Some(true) {
        parts.push("删除线".into());
    }
    if let Some(v) = &p.color {
        if v.eq_ignore_ascii_case("auto") {
            parts.push("自动色".into());
        } else {
            parts.push(format!("#{v}"));
        }
    }
    if let Some(v) = &p.highlight {
        parts.push(format!("高亮={v}"));
    }
    match (&p.font_east_asia, &p.font_ascii) {
        (Some(ea), Some(ascii)) => parts.push(format!("{ea}/{ascii}")),
        (Some(ea), None) => parts.push(ea.clone()),
        (None, Some(ascii)) => parts.push(ascii.clone()),
        (None, None) => {}
    }
    if parts.is_empty() {
        "(默认)".into()
    } else {
        parts.join(" ")
    }
}

/// 直接格式（w:rPr 原样值，字段短名）；全 None → None（纯继承）。
fn fmt_direct_run(p: &docx_model::RunProps) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(v) = p.bold {
        parts.push(if v { "b".into() } else { "b=off".into() });
    }
    if let Some(v) = p.italic {
        parts.push(if v { "i".into() } else { "i=off".into() });
    }
    if let Some(v) = p.underline {
        parts.push(if v { "u".into() } else { "u=off".into() });
    }
    if let Some(v) = p.strike {
        parts.push(if v { "strike".into() } else { "strike=off".into() });
    }
    if let Some(v) = p.size_half_pt {
        parts.push(format!("sz={v}"));
    }
    if let Some(v) = &p.color {
        parts.push(format!("color={v}"));
    }
    if let Some(v) = &p.highlight {
        parts.push(format!("hl={v}"));
    }
    if let Some(v) = &p.font_east_asia {
        parts.push(format!("eastAsia={v}"));
    }
    if let Some(v) = &p.font_ascii {
        parts.push(format!("ascii={v}"));
    }
    if parts.is_empty() { None } else { Some(parts.join(" ")) }
}

/// 有效段落格式一行。
fn fmt_para_props(p: &docx_model::ParaProps) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(v) = &p.alignment {
        parts.push(format!("对齐={v}"));
    }
    if let Some(line) = p.spacing_line {
        let rule = p.line_rule.as_deref().unwrap_or("auto");
        let desc = match rule {
            // auto：line/240 = 倍数行距（240=单倍 360=1.5 倍）
            "auto" | "" => {
                let mult = line as f64 / 240.0;
                if (mult - mult.round()).abs() < f64::EPSILON {
                    format!("{}倍行距", trim_float(mult))
                } else {
                    format!("line={line}(auto)")
                }
            }
            // at / exact：line/20 = 磅（fmt_pt 已含单位）
            _ => format!("{}行距({rule})", fmt_pt(line)),
        };
        parts.push(desc);
    }
    match (p.spacing_before, p.spacing_after) {
        (Some(b), Some(a)) => parts.push(format!("段前/后={}/{}", fmt_pt(b), fmt_pt(a))),
        (Some(b), None) => parts.push(format!("段前={}", fmt_pt(b))),
        (None, Some(a)) => parts.push(format!("段后={}", fmt_pt(a))),
        (None, None) => {}
    }
    if let Some(v) = p.indent_first_line {
        parts.push(format!("首行缩进={v}tw"));
    }
    if let Some(v) = p.indent_hanging {
        parts.push(format!("悬挂缩进={v}tw"));
    }
    if let Some(v) = p.indent_left {
        parts.push(format!("左缩进={v}tw"));
    }
    if parts.is_empty() {
        "(默认)".into()
    } else {
        parts.join(" ")
    }
}

fn trim_float(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// 表首行摘要（outline 表格行）。
fn table_row_summary(row: Option<&docx_model::TableRow>) -> String {
    let Some(row) = row else { return "(空表)".into() };
    summarize(&row_cells_summary(row), 50)
}

/// 一行的格摘要：合并格标注，vMerge continue 显示 ⋯。
fn row_cells_summary(row: &docx_model::TableRow) -> String {
    let mut cells: Vec<String> = Vec::new();
    for c in &row.cells {
        if c.v_merge.as_deref() == Some("continue") {
            cells.push("⋯".into());
            continue;
        }
        let mut text = String::new();
        for b in &c.blocks {
            if let Block::Paragraph(p) = b {
                text.push_str(&para_text(p));
            }
        }
        let mut cell = summarize(&text, 20);
        if let Some(span) = c.grid_span {
            if span > 1 {
                cell.push_str(&format!("↔{span}"));
            }
        }
        cells.push(cell);
    }
    cells.join(" | ")
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 用 docx-rs 造真实包跑 inspect（zip + document.xml + styles.xml 全链路）。
    fn docx_bytes() -> Vec<u8> {
        use docx_rs::{Docx, Document, Paragraph, Run};
        let document = Document::new()
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第一段正文")))
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第二段")));
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        Docx::new().document(document).build().pack(&mut cursor).unwrap();
        cursor.into_inner()
    }

    #[test]
    fn outline_text_format_on_real_package() {
        let bytes = docx_bytes();
        let report = inspect_document(&bytes, &InspectRequest {
            projection: InspectProjection::Outline,
            start: None,
            end: None,
        })
        .unwrap();
        assert_eq!(report.total_blocks, 2);
        assert_eq!(report.range, (1, 2));
        assert!(!report.has_more);
        assert!(report.content.contains("[1] ¶"), "outline 应有块 1: {}", report.content);
        assert!(report.content.contains("第一段正文"));

        let text = inspect_document(&bytes, &InspectRequest {
            projection: InspectProjection::Text,
            start: None,
            end: None,
        })
        .unwrap();
        assert!(text.content.contains("[2] 第二段"));

        let fmt = inspect_document(&bytes, &InspectRequest {
            projection: InspectProjection::Format,
            start: None,
            end: None,
        })
        .unwrap();
        assert!(fmt.content.contains("文本:"), "format 应有文本行: {}", fmt.content);
    }

    #[test]
    fn range_validation_errors_and_clamps() {
        let bytes = docx_bytes();
        // start 越界 → 三段式报错
        let err = inspect_document(&bytes, &InspectRequest {
            projection: InspectProjection::Outline,
            start: Some(5),
            end: None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("块号越界"), "实际: {err}");
        // end < start → 报错
        let err = inspect_document(&bytes, &InspectRequest {
            projection: InspectProjection::Outline,
            start: Some(2),
            end: Some(1),
        })
        .unwrap_err();
        assert!(err.to_string().contains("区间无效"), "实际: {err}");
        // end 超总 → clamp 到 2
        let report = inspect_document(&bytes, &InspectRequest {
            projection: InspectProjection::Outline,
            start: Some(1),
            end: Some(99),
        })
        .unwrap();
        assert_eq!(report.range, (1, 2));
        assert!(!report.has_more);
    }

    #[test]
    fn format_span_default_is_50() {
        // token 分级锁定：outline 行短可多，format 行长必须少
        assert_eq!(InspectProjection::Format.default_span(), 50);
        assert_eq!(InspectProjection::Outline.default_span(), 400);
        assert_eq!(InspectProjection::Text.default_span(), 100);
    }

    #[test]
    fn unit_formatters() {
        assert_eq!(fmt_pt(32), "16pt");
        assert_eq!(fmt_pt(21), "10.5pt");
        assert_eq!(fmt_pt(24), "12pt");
        assert_eq!(fmt_cm(11906), "21.0");
        assert_eq!(summarize("abc\ndef", 10), "abc def");
        let long = "字".repeat(100);
        assert_eq!(summarize(&long, 60).chars().count(), 61); // 60 字 + …
        assert_eq!(trim_float(1.5), "1.5");
        assert_eq!(trim_float(2.0), "2");
    }

    /// 手工拼带页眉页脚的最小 zip 包（docx-rs 不便造 rels/部件）。
    fn hf_docx_bytes() -> Vec<u8> {
        use std::io::Write;
        let doc_xml = concat!(
            r#"<?xml version="1.0"?><w:document xmlns:w="w" xmlns:r="r"><w:body>"#,
            r#"<w:p><w:r><w:t>正文一</w:t></w:r></w:p>"#,
            r#"<w:p><w:pPr><w:sectPr>"#,
            r#"<w:headerReference w:type="default" r:id="rIdH"/>"#,
            r#"<w:footerReference w:type="default" r:id="rIdF"/>"#,
            r#"</w:sectPr></w:pPr><w:r><w:t>节一末</w:t></w:r></w:p>"#,
            r#"<w:sectPr><w:headerReference w:type="default" r:id="rIdH"/>"#,
            r#"<w:footerReference w:type="first" r:id="rIdMISSING"/>"#,
            r#"</w:sectPr></w:body></w:document>"#,
        );
        let rels_xml = concat!(
            r#"<?xml version="1.0"?><Relationships>"#,
            r#"<Relationship Id="rIdH" Type="http://x/header" Target="header1.xml"/>"#,
            r#"<Relationship Id="rIdF" Type="http://x/footer" Target="footer1.xml"/>"#,
            r#"<Relationship Id="rIdIMG" Type="http://x/image" Target="media/a.png"/>"#,
            r#"</Relationships>"#,
        );
        let hdr_xml = r#"<?xml version="1.0"?><w:hdr xmlns:w="w"><w:p><w:r><w:t>页眉甲行</w:t></w:r></w:p><w:p><w:r><w:t>页眉乙行</w:t></w:r></w:p></w:hdr>"#;
        let ftr_xml = r#"<?xml version="1.0"?><w:ftr xmlns:w="w"><w:p><w:r><w:t>页脚一行</w:t></w:r></w:p></w:ftr>"#;
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            for (name, data) in [
                ("word/document.xml", doc_xml),
                ("word/_rels/document.xml.rels", rels_xml),
                ("word/header1.xml", hdr_xml),
                ("word/footer1.xml", ftr_xml),
            ] {
                w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                w.write_all(data.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    #[test]
    fn headers_footers_projection_renders_parts() {
        let bytes = hf_docx_bytes();
        let report = inspect_document(&bytes, &InspectRequest {
            projection: InspectProjection::HeadersFooters,
            start: None,
            end: None,
        })
        .unwrap();
        assert!(report.content.contains("2 节"), "节计数: {}", report.content);
        assert_eq!(report.range, (1, 2), "range = 节区间");
        assert!(!report.has_more);
        // 部件内容 + 多行续行
        assert!(report.content.contains("页眉 default: 页眉甲行\n    页眉乙行"), "{}", report.content);
        assert!(report.content.contains("页脚 default: 页脚一行"), "{}", report.content);
        // 同部件多节引用（rIdH 两节）经缓存呈现一致
        assert_eq!(report.content.matches("页眉甲行").count(), 2);
        // rels 悬空引用诚实呈现（rIdMISSING 无对应 Relationship）
        assert!(report.content.contains("悬空"), "{}", report.content);
        // 图片关系（http://x/image）不进页眉脚表
        assert!(!report.content.contains("media/a.png"));

        // start/end 不适用 → 家族报错
        let err = inspect_document(&bytes, &InspectRequest {
            projection: InspectProjection::HeadersFooters,
            start: Some(1),
            end: None,
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("不接受 start/end"), "实际: {err}");
    }
}
