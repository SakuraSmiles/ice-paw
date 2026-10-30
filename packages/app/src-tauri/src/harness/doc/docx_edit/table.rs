//! 表格手术 + 容器重打包（自 `docx_edit` 拆出，保持纯函数契约：xml/bytes 进、新 xml/bytes 出）。

use std::io::{Cursor, Write};

use crate::error::{AppError, AppResult};

use super::*;

// =========================================================================
// 表格手术（insert_table_after / set_cell_text / insert_table_row_after）
// =========================================================================

/// 建表规模上限（防 agent 一次生成巨型表；超限走增行分批）。
const TABLE_MAX_ROWS: usize = 200;
const TABLE_MAX_COLS: usize = 30;

/// rows 矩阵校验：非空 / 行列上限 / 矩形一致。全挂「表格数据无效」家族前缀
/// （doom_loop 错误签名按家族前缀聚合，勿混入具体格内容）。
pub(super) fn validate_table_rows(rows: &[Vec<String>]) -> AppResult<()> {
    let Some(first) = rows.first() else {
        return Err(AppError::Validation(
            "表格数据无效: rows 为空，至少一行（首行默认表头）".into(),
        ));
    };
    if rows.len() > TABLE_MAX_ROWS {
        return Err(AppError::Validation(format!(
            "表格数据无效: 行数 {} 超上限 {TABLE_MAX_ROWS}——先建前 {TABLE_MAX_ROWS} 行，余下 insert_table_row_after 分批增补",
            rows.len()
        )));
    }
    let cols = first.len();
    if cols == 0 || cols > TABLE_MAX_COLS {
        return Err(AppError::Validation(format!(
            "表格数据无效: 列数 {cols} 越界（每行须 1..={TABLE_MAX_COLS} 格）"
        )));
    }
    for (i, r) in rows.iter().enumerate() {
        if r.len() != cols {
            return Err(AppError::Validation(format!(
                "表格数据无效: 第 {} 行 {} 格与首行 {cols} 格不一致——rows 须矩形矩阵（格内多段用 \\n 表达，勿多造行）",
                i + 1,
                r.len()
            )));
        }
    }
    Ok(())
}

/// 节内容宽度（twips）：取最后出现的页宽与左右边距（缺省 1 英寸 = 1440），
/// 页宽全缺省按 Letter 12240；结果下限钳 2000（防 0/负），再低回落 9026。
pub(super) fn content_width_twips(model: &docx_model::DocxDocument) -> u32 {
    let mut page_w: Option<u32> = None;
    let mut margin_l = 1440u32;
    let mut margin_r = 1440u32;
    for s in &model.sections {
        if let Some(w) = s.page_w {
            page_w = Some(w);
        }
        if let Some(m) = s.margin_left {
            margin_l = m;
        }
        if let Some(m) = s.margin_right {
            margin_r = m;
        }
    }
    let w = page_w
        .unwrap_or(12240)
        .saturating_sub(margin_l)
        .saturating_sub(margin_r);
    if w < 2000 {
        9026
    } else {
        w
    }
}

/// 建整表 XML：tblW pct 5000（100% 宽）+ 全边框 single sz=4 + 列宽均分；
/// header=true 时首行加粗 + tblHeader（跨页重复表头）。style_id 给定时 tblPr
/// 首位注 `<w:tblStyle>`（CT_TblPrBase schema 首子元素——用户模板表样式优先于
/// 默认边框，Word 按样式定义渲染条纹带/边框）。
pub(super) fn build_table_xml(
    rows: &[Vec<String>],
    header: bool,
    style_id: Option<&str>,
    width_tw: u32,
) -> String {
    let cols = rows.first().map(|r| r.len()).unwrap_or(1).max(1);
    let col_w = (width_tw / cols as u32).max(200);
    let mut s = String::with_capacity(64 + rows.len() * cols * 48);
    s.push_str(r#"<w:tbl><w:tblPr>"#);
    if let Some(sid) = style_id {
        s.push_str(&format!(r#"<w:tblStyle w:val="{sid}"/>"#));
    }
    s.push_str(r#"<w:tblW w:w="5000" w:type="pct"/><w:tblBorders>"#);
    for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
        s.push_str(&format!(
            r#"<w:{side} w:val="single" w:sz="4" w:space="0" w:color="auto"/>"#
        ));
    }
    s.push_str("</w:tblBorders></w:tblPr><w:tblGrid>");
    for _ in 0..cols {
        s.push_str(&format!(r#"<w:gridCol w:w="{col_w}"/>"#));
    }
    s.push_str("</w:tblGrid>");
    for (ri, row) in rows.iter().enumerate() {
        s.push_str("<w:tr>");
        let is_header = header && ri == 0;
        if is_header {
            s.push_str("<w:trPr><w:tblHeader/></w:trPr>");
        }
        for cell in row {
            s.push_str(&format!(
                r#"<w:tc><w:tcPr><w:tcW w:w="{col_w}" w:type="dxa"/></w:tcPr>{}"#,
                cell_paragraphs_xml(cell, is_header)
            ));
            s.push_str("</w:tc>");
        }
        s.push_str("</w:tr>");
    }
    s.push_str("</w:tbl>");
    s
}

/// 单元格文本 → 段落序列：空文本 = 单空段；`\n` 分段；表头行加粗。
pub(super) fn cell_paragraphs_xml(text: &str, bold: bool) -> String {
    let rpr = if bold { "<w:rPr><w:b/></w:rPr>" } else { "" };
    if text.is_empty() {
        return "<w:p/>".to_string();
    }
    let mut s = String::new();
    for line in text.split('\n') {
        s.push_str(&format!("<w:p>{}</w:p>", run_xml(rpr, line)));
    }
    s
}

/// `s` 内 `w:{name}` **直接子元素**的字节范围列表（相对 `s`；更深层同名忽略）。
/// 表格结构手术专用：行/格定位必须抗嵌套表（tc 内 w:tbl 含同名 w:tr/w:tc）。
/// 形态异常（扫描 Err / 未闭合）返回空表——调用方按手术失败处理，不写盘。
pub(in crate::harness::doc) fn direct_children_spans(s: &str, name: &str) -> Vec<(usize, usize)> {
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(false); // 偏移与原始字节对齐
    let target = format!("w:{name}");
    let mut spans = Vec::new();
    let mut depth = 0usize; // 当前嵌套在目标元素内的层数
    let mut direct_start: Option<usize> = None; // 最外层（直接子级）起点
    let mut balanced = true;
    loop {
        // buffer_position = 上一事件结束处 = 本事件开始处
        let ev_start = reader.buffer_position() as usize;
        let ev = match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };
        let ev_end = reader.buffer_position() as usize;
        match ev {
            Event::Start(e) if e.name().as_ref() == target.as_bytes() => {
                if depth == 0 {
                    direct_start = Some(ev_start);
                }
                depth += 1;
            }
            Event::End(e) if e.name().as_ref() == target.as_bytes() => {
                if depth == 0 {
                    balanced = false; // 多闭：形态异常
                    break;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = direct_start.take() {
                        spans.push((start, ev_end));
                    }
                }
            }
            Event::Empty(e) if e.name().as_ref() == target.as_bytes() && depth == 0 => {
                spans.push((ev_start, ev_end));
            }
            _ => {}
        }
    }
    if !balanced || depth != 0 || direct_start.is_some() {
        return Vec::new(); // 未闭合 / 悬空开标签：形态异常
    }
    spans
}

/// set_cell_text 手术：定位第 `row` 行第 `cell` 格（1-based，与 projection=table
/// 视图同口径——跨列格算 1 格），重建格文本（格式按位继承/回落见
/// rebuild_cell_content），其余字节原样。返回 (新块 XML, 是否按位继承)。
pub(super) fn set_cell_text(block_xml: &str, row: usize, cell: usize, text: &str) -> Option<(String, bool)> {
    let rows = direct_children_spans(block_xml, "tr");
    let (rs, re) = *rows.get(row.checked_sub(1)?)?;
    let row_xml = &block_xml[rs..re];
    let cells = direct_children_spans(row_xml, "tc");
    let (cs, ce) = *cells.get(cell.checked_sub(1)?)?;
    let (new_cell, aligned) = rebuild_cell_content(&row_xml[cs..ce], text)?;
    let mut out = String::with_capacity(block_xml.len() + 64);
    out.push_str(&block_xml[..rs]);
    out.push_str(&row_xml[..cs]);
    out.push_str(&new_cell);
    out.push_str(&row_xml[ce..]);
    out.push_str(&block_xml[re..]);
    Some((out, aligned))
}

/// 重建单元格：保开标签属性 + tcPr（含 gridSpan/vMerge/tcW），段落由 text 生成。
/// 格式保真策略（十一波规则：**重写保真到位置对齐粒度**）——新段落数与原格
/// 段落数相等 → 逐段按位继承原各段的 pPr 与首 run rPr；不等 → 回落首段模板
///（原行为），调用方在摘要披露。嵌套表保护：格内含 w:tbl 时返回 None（预检
/// 已拒，此处兜底防直调误伤）。返回 (新格 XML, 是否按位继承)。
pub(super) fn rebuild_cell_content(cell_xml: &str, text: &str) -> Option<(String, bool)> {
    if cell_xml.contains("<w:tbl>") || cell_xml.contains("<w:tbl ") {
        return None;
    }
    let open_end = cell_xml.find('>')? + 1; // 开标签止于 '>'（tc 无自闭合形态）
    let tcpr = find_element_span(cell_xml, "tcPr");
    let content_from = tcpr.map(|(_, e)| e).unwrap_or(open_end);
    let content = &cell_xml[content_from..cell_xml.len().saturating_sub("</w:tc>".len())];
    let old_ps = direct_children_spans(content, "p");
    if old_ps.is_empty() {
        return None; // tc 必有段——形态异常（沿旧首段切片的 None 语义）
    }
    let lines: Vec<&str> = if text.is_empty() {
        vec![""]
    } else {
        text.split('\n').collect()
    };
    let aligned = lines.len() == old_ps.len();
    let mut paras = String::new();
    if text.is_empty() {
        let p_xml = &content[old_ps[0].0..old_ps[0].1];
        paras.push_str(&format!("<w:p>{}</w:p>", slice_ppr(p_xml)));
    } else {
        for (i, line) in lines.iter().enumerate() {
            // 按位继承：第 i 段用原第 i 段格式；不对齐 → 回落首段模板
            let src = if aligned { old_ps[i] } else { old_ps[0] };
            let p_xml = &content[src.0..src.1];
            paras.push_str(&format!(
                "<w:p>{}{}</w:p>",
                slice_ppr(p_xml),
                run_xml(&slice_first_run_rpr(p_xml), line)
            ));
        }
    }
    let mut out = String::with_capacity(cell_xml.len() + paras.len() + 8);
    out.push_str(&cell_xml[..open_end]);
    if let Some((s, e)) = tcpr {
        out.push_str(&cell_xml[s..e]);
    }
    out.push_str(&paras);
    out.push_str("</w:tc>");
    Some((out, aligned))
}

/// 增行手术：克隆模板行（after_row 缺省 = 末行）整结构——tcPr/gridSpan/vMerge
/// 原样保留，格文本替换为 cells（缺省全空；填充段的格式按位继承模板行各段，
/// 见 rebuild_cell_content）。结构克隆是合并格表格唯一正确的增行方式（凭空
/// 造行会破坏 gridSpan/vMerge 与 tblGrid 的对应）。返回 (新块 XML, 是否有
/// 格发生多段回落)。
pub(super) fn insert_table_row_after(
    block_xml: &str,
    after_row: Option<usize>,
    cells: Option<&[String]>,
) -> Option<(String, bool)> {
    let rows = direct_children_spans(block_xml, "tr");
    if rows.is_empty() {
        return None;
    }
    let tpl_idx = match after_row {
        Some(r) => r.checked_sub(1)?,
        None => rows.len() - 1,
    };
    let (ts, te) = *rows.get(tpl_idx)?;
    let tpl = &block_xml[ts..te];
    let tcs = direct_children_spans(tpl, "tc");
    if tcs.is_empty() {
        return None;
    }
    let filled: Vec<&str> = match cells {
        Some(cs) => cs.iter().map(String::as_str).collect(),
        None => vec![""; tcs.len()],
    };
    let mut any_fallback = false;
    let mut new_row = String::with_capacity(tpl.len() + 32);
    new_row.push_str(&tpl[..tcs[0].0]); // <w:tr> + trPr（若有）原样
    for (i, (cs, ce)) in tcs.iter().enumerate() {
        let text = filled.get(i).copied().unwrap_or("");
        let (rebuilt, aligned) = rebuild_cell_content(&tpl[*cs..*ce], text)?;
        if !aligned && text.contains('\n') {
            any_fallback = true; // 多段填充未对齐 → 回落（单段填充回落无信息量）
        }
        new_row.push_str(&rebuilt);
    }
    new_row.push_str("</w:tr>");
    let mut out = String::with_capacity(block_xml.len() + new_row.len());
    out.push_str(&block_xml[..te]);
    out.push_str(&new_row);
    out.push_str(&block_xml[te..]);
    Some((out, any_fallback))
}

/// (row, cell) 单元格投影文本（前 60 字）——set_cell_text 摘要的 before 值。
pub(super) fn cell_projected_of(
    model: &docx_model::DocxDocument,
    block: usize,
    row: usize,
    cell: usize,
) -> String {
    let mut s = String::new();
    if let Block::Table(t) = &model.body[block - 1] {
        if let Some(r) = t.rows.get(row - 1) {
            if let Some(c) = r.cells.get(cell - 1) {
                docx_model::blocks_text(&c.blocks, &mut s);
            }
        }
    }
    truncate(s.trim_end_matches('\n'), 60)
}

// =========================================================================
// 表格格式件手术（S3 四波）：容器元素手术 / 格内格式 / 合并 / 拆分
// =========================================================================

/// 定位行 XML（1-based 行号 → 该行在 block_xml 内的字节范围）。
pub(super) fn row_span_of(block_xml: &str, row: usize) -> Option<(usize, usize)> {
    direct_children_spans(block_xml, "tr")
        .get(row.checked_sub(1)?)
        .copied()
}

/// 定位行内格 XML（1-based 格号，与 projection=table 显示序同口径）。
pub(super) fn cell_span_of(row_xml: &str, cell: usize) -> Option<(usize, usize)> {
    direct_children_spans(row_xml, "tc")
        .get(cell.checked_sub(1)?)
        .copied()
}

/// 对 (row, cell) 定位的格应用手术函数（输入格 XML，输出 (新格 XML, 附带值)），
/// 行内其余字节与表内其余行原样。嵌套表由手术函数自查（预检已拒，兜底防直调）。
pub(super) fn apply_to_cell<T>(
    block_xml: &str,
    row: usize,
    cell: usize,
    f: impl Fn(&str) -> Option<(String, T)>,
) -> Option<(String, T)> {
    let (rs, re) = row_span_of(block_xml, row)?;
    let row_xml = &block_xml[rs..re];
    let (cs, ce) = cell_span_of(row_xml, cell)?;
    let (new_cell, t) = f(&row_xml[cs..ce])?;
    let new_row = format!("{}{new_cell}{}", &row_xml[..cs], &row_xml[ce..]);
    Some((
        format!("{}{new_row}{}", &block_xml[..rs], &block_xml[re..]),
        t,
    ))
}

/// 属性容器子元素手术（tblPr / trPr / tcPr 通用，set_ppr_element 的容器版）：
/// - frag=None：移除 `<w:{element}>` 整元素；容器摘空则整体清理；元素不存在 →
///   返回 (原文, false) 空转
/// - frag=Some：整元素替换（已存在）或按 `whitelist` schema 序插入（不存在）；
///   容器不存在时新建于 scope 开标签后（容器是 tbl/tr/tc 的 schema 首子元素）
///
/// 返回 (新 scope XML, 是否实际改动)。None 仅防御形态异常（不写盘）。
pub(super) fn set_container_element_xml(
    scope_xml: &str,
    container: &str,
    element: &str,
    frag: Option<&str>,
    whitelist: &[&str],
) -> Option<(String, bool)> {
    let pat = format!("<w:{container}");
    let container_at = scope_xml.find(&pat).filter(|s| {
        matches!(
            scope_xml.as_bytes().get(s + pat.len()),
            Some(b'>') | Some(b'/') | Some(b' ')
        )
    });
    let Some(s) = container_at else {
        // 无容器：无元素可摘 → 空转；有片段 → 开标签后新建容器
        let Some(f) = frag else {
            return Some((scope_xml.to_string(), false));
        };
        let gt = scope_xml.find('>')?;
        if scope_xml.as_bytes()[gt - 1] == b'/' {
            return None; // tbl/tr/tc 不自闭合——形态异常
        }
        let new_container = format!("<w:{container}>{f}</w:{container}>");
        return Some((
            format!(
                "{}{new_container}{}",
                &scope_xml[..gt + 1],
                &scope_xml[gt + 1..]
            ),
            true,
        ));
    };
    let open_end = s + scope_xml[s..].find('>')? + 1;
    let close_tag = format!("</w:{container}>");
    if scope_xml.as_bytes()[open_end - 2] == b'/' {
        // 自闭合空容器（Word 偶出）：无子元素可摘 → 空转；有片段 → 展开为完整容器
        return match frag {
            None => Some((scope_xml.to_string(), false)),
            Some(f) => Some((
                format!(
                    "{}<w:{container}>{f}</w:{container}>{}",
                    &scope_xml[..s],
                    &scope_xml[open_end..]
                ),
                true,
            )),
        };
    }
    let close_at = open_end + scope_xml[open_end..].find(&close_tag)?;
    match frag {
        None => {
            let inner = &scope_xml[open_end..close_at];
            let Some((es, ee)) = find_element_span(inner, element) else {
                return Some((scope_xml.to_string(), false)); // 不存在 → 空转
            };
            let new_inner = format!("{}{}", &inner[..es], &inner[ee..]);
            if new_inner.trim().is_empty() {
                // 摘空 → 容器整体移除（Word 自身也这样清理）
                return Some((
                    format!(
                        "{}{}",
                        &scope_xml[..s],
                        &scope_xml[close_at + close_tag.len()..]
                    ),
                    true,
                ));
            }
            Some((
                format!(
                    "{}{}{}",
                    &scope_xml[..open_end],
                    new_inner,
                    &scope_xml[close_at..]
                ),
                true,
            ))
        }
        Some(f) => {
            let idx = whitelist.iter().position(|n| *n == element)?;
            let later: Vec<&str> = whitelist[idx + 1..].to_vec();
            let container_el = &scope_xml[s..close_at + close_tag.len()];
            let new_container = upsert_element(container_el, element, f, &later, &close_tag);
            Some((
                format!(
                    "{}{new_container}{}",
                    &scope_xml[..s],
                    &scope_xml[close_at + close_tag.len()..]
                ),
                true,
            ))
        }
    }
}

/// set_table_element 手术：定位 level 对应 scope（表=整块 / 行=tr / 格=tc），
/// 委托容器手术。row/cell 寻址按当前 XML 形态（同块前序操作已生效）。
pub(super) fn set_table_element_xml(
    block_xml: &str,
    level: TableLevel,
    row: Option<usize>,
    cell: Option<usize>,
    element: &str,
    frag: Option<&str>,
) -> Option<(String, bool)> {
    match level {
        TableLevel::Table => {
            set_container_element_xml(block_xml, "tblPr", element, frag, &TBLPR_ELEMENTS)
        }
        TableLevel::Row => {
            let (rs, re) = row_span_of(block_xml, row?)?;
            let (new_row, changed) = set_container_element_xml(
                &block_xml[rs..re],
                "trPr",
                element,
                frag,
                &TRPR_ELEMENTS,
            )?;
            Some((
                format!("{}{new_row}{}", &block_xml[..rs], &block_xml[re..]),
                changed,
            ))
        }
        TableLevel::Cell => apply_to_cell(block_xml, row?, cell?, |cell_xml| {
            set_container_element_xml(cell_xml, "tcPr", element, frag, &TCPR_ELEMENTS)
        }),
    }
}

/// set_cell_format 手术：段落格式作用于格内全部直接 `<w:p>`（reformat_ppr 逐段，
/// 含自闭合空段展开），字符格式作用于格内全部 run（reformat_runs 整格），
/// 样式（style_id 已反查）作用于格内全部段落（restyle_paragraph 逐段——与
/// 直接格式互不冲突，pStyle 与 spacing/ind 等 pPr 子元素各占各位）。
pub(super) fn set_cell_format_xml(
    block_xml: &str,
    row: usize,
    cell: usize,
    paragraph: Option<&ParaFormat>,
    character: Option<&CharFormat>,
    style_id: Option<&str>,
) -> Option<String> {
    apply_to_cell(block_xml, row, cell, |cell_xml| {
        if cell_xml.contains("<w:tbl>") || cell_xml.contains("<w:tbl ") {
            return None; // 嵌套表兜底（预检已拒）
        }
        let mut out = cell_xml.to_string();
        if let Some(p) = paragraph {
            let paras = direct_children_spans(&out, "p");
            if paras.is_empty() {
                return None; // Word 格必含 ≥1 段——形态异常
            }
            for (ps, pe) in paras.iter().rev() {
                let new_p = reformat_ppr(&out[*ps..*pe], p)?;
                out.replace_range(ps..pe, &new_p);
            }
        }
        if let Some(id) = style_id {
            let paras = direct_children_spans(&out, "p");
            if paras.is_empty() {
                return None; // Word 格必含 ≥1 段——形态异常
            }
            for (ps, pe) in paras.iter().rev() {
                let new_p = restyle_paragraph(&out[*ps..*pe], id)?;
                out.replace_range(ps..pe, &new_p);
            }
        }
        if let Some(c) = character {
            out = reformat_runs(&out, c)?;
        }
        Some((out, ()))
    })
    .map(|(s, _)| s)
}

/// XML 层的格 gridSpan（tcPr 内 w:gridSpan w:val；缺省 1）。
pub(super) fn xml_grid_span(cell_xml: &str) -> u32 {
    let Some((s, e)) = find_element_span(cell_xml, "tcPr") else {
        return 1;
    };
    let inner = &cell_xml[s..e]; // 偏移属本切片——勿拿去索引原串
    find_element_span(inner, "gridSpan")
        .map(|(gs, ge)| {
            parse_attrs(&inner[gs..ge])
                .into_iter()
                .find(|(k, _)| k == "w:val")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(1)
        })
        .unwrap_or(1)
}

/// XML 层的 vMerge 归一（None=无 / Some("restart") / Some("continue")，与模型口径同）。
pub(super) fn xml_v_merge(cell_xml: &str) -> Option<String> {
    let (s, e) = find_element_span(cell_xml, "tcPr")?;
    let inner = &cell_xml[s..e]; // 偏移属本切片——勿拿去索引原串
    let (vs, ve) = find_element_span(inner, "vMerge")?;
    Some(
        parse_attrs(&inner[vs..ve])
            .into_iter()
            .find(|(k, _)| k == "w:val")
            .map(|(_, v)| v)
            .unwrap_or_else(|| "continue".to_string()), // <w:vMerge/> 无 val = continue
    )
}

/// 行内占据恰好 [g0, g1) 网格列区间的格的字节范围（None = 边界不对齐）。
/// XML 层对齐判据（纵并用），与模型的 cell_at_grid_range 同语义。
pub(super) fn xml_cell_span_at_grid_range(row_xml: &str, g0: u32, g1: u32) -> Option<(usize, usize)> {
    let mut start = 0u32;
    for (s, e) in direct_children_spans(row_xml, "tc") {
        let span = xml_grid_span(&row_xml[s..e]);
        if start == g0 && start + span == g1 {
            return Some((s, e));
        }
        start += span;
    }
    None
}

/// 内容语义规则（十一波）：**占位段不是内容**。OOXML 要求 tc 至少含一个 w:p，
/// 空段常是结构性占位而非用户内容——内容聚合（横并拼接）时不参与搬运；段间
/// 夹带的其他节点（书签等）原样保留。图（drawing/pict/object）、域
///（fldSimple/instrText/fldChar）、非空白文本任一在场即算内容。
pub(super) fn is_placeholder_p(p_xml: &str) -> bool {
    const CONTENT_MARKERS: [&str; 6] = [
        "<w:drawing",
        "<w:pict",
        "<w:object",
        "<w:fldSimple",
        "<w:instrText",
        "w:fldChar",
    ];
    if CONTENT_MARKERS.iter().any(|m| p_xml.contains(m)) {
        return false;
    }
    // 非空白文本判定：逐个 <w:t 元素（验后随字符防 <w:tab 前缀碰撞）
    let mut hay = p_xml;
    while let Some(pos) = hay.find("<w:t") {
        let tail = &hay[pos + 4..]; // 跳过完整 4 字符 needle，tail[0] 即后随字符
        match tail.as_bytes().first() {
            Some(b'>') | Some(b' ') | Some(b'/') if !tail.starts_with("/>") => {
                // 正文 = start tag 的 '>' 后到 </w:t> 前
                if let (Some(open_end), Some(close)) = (tail.find('>'), tail.find("</w:t>")) {
                    if open_end < close && !tail[open_end + 1..close].trim().is_empty() {
                        return false;
                    }
                }
            }
            _ => {} // <w:tab 等前缀碰撞 / 自闭合空元素——跳过
        }
        hay = tail;
    }
    true
}

/// 全部直接子节点 span（任意标签）——占位段剔除须按「子节点逐个裁决」拷贝，
/// 粗粒度 cursor 拷贝会把被剔除的段一并带回（中段/尾段都中招）。
pub(super) fn all_direct_children_spans(s: &str) -> Vec<(usize, usize)> {
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(false); // 偏移与原始字节对齐
    let mut spans = Vec::new();
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    loop {
        let ev_start = reader.buffer_position() as usize;
        let ev = match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };
        let ev_end = reader.buffer_position() as usize;
        match ev {
            Event::Start(_) => {
                if depth == 0 {
                    start = Some(ev_start);
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(st) = start.take() {
                        spans.push((st, ev_end));
                    }
                }
            }
            Event::Empty(_) if depth == 0 => spans.push((ev_start, ev_end)),
            _ => {}
        }
    }
    spans
}

/// 格内容聚合（规则：占位段不是内容）：占位段（w:p 且无文字/图/域）剔除，
/// 其余子节点（含段间书签等非段节点与子节点间文本）按序逐个保留。
/// 返回 (聚合后内容, 剔除的占位段数)。
pub(super) fn aggregate_cell_content(inner: &str) -> (String, usize) {
    let mut out = String::new();
    let mut prev_end = 0usize;
    let mut dropped = 0usize;
    for (s, e) in all_direct_children_spans(inner) {
        out.push_str(&inner[prev_end..s]); // 子节点间文本（空白）
        let is_p = inner[s..].starts_with("<w:p>")
            || inner[s..].starts_with("<w:p ")
            || inner[s..].starts_with("<w:p/>");
        if is_p && is_placeholder_p(&inner[s..e]) {
            dropped += 1;
        } else {
            out.push_str(&inner[s..e]);
        }
        prev_end = e;
    }
    out.push_str(&inner[prev_end..]);
    (out, dropped)
}

/// 简单横并原语（merge_cells 线并形态与矩形区的逐行步骤共用）：
/// 第 row 行 display 格 cell..cell+span-1 并 1 格——首格 tcPr upsert
/// gridSpan=各格跨度之和；全部格内容（各自去 tcPr，**占位段剔除**——十一波
/// 规则「占位段不是内容」，摘要披露剔除数）按序拼进首格；其余格整段删除。
/// 返回 (新块 XML, 跨度合计, 剔除的占位段数)。
pub(super) fn merge_horizontal_xml(
    block_xml: &str,
    row: usize,
    cell: usize,
    span: usize,
) -> Option<(String, u32, usize)> {
    let (rs, re) = row_span_of(block_xml, row)?;
    let row_xml = &block_xml[rs..re];
    let cells = direct_children_spans(row_xml, "tc");
    let first = cell.checked_sub(1)?;
    let last = first.checked_add(span.checked_sub(1)?)?;
    let range = cells.get(first..=last)?.to_vec();
    let sum: u32 = range
        .iter()
        .map(|(s, e)| xml_grid_span(&row_xml[*s..*e]))
        .sum();
    // 首格容器手术 upsert gridSpan；内容 = 各格 inner（去 tcPr）按序聚合
    let head_xml = &row_xml[range[0].0..range[0].1];
    let (new_head, _) = set_container_element_xml(
        head_xml,
        "tcPr",
        "gridSpan",
        Some(&format!(r#"<w:gridSpan w:val="{sum}"/>"#)),
        &TCPR_ELEMENTS,
    )?;
    let mut content = String::new();
    let mut dropped: usize = 0;
    for (s, e) in &range {
        let cxml = &row_xml[*s..*e];
        let open_end = cxml.find('>')? + 1;
        let from = find_element_span(cxml, "tcPr")
            .map(|(_, te)| te)
            .unwrap_or(open_end);
        let (kept, d) =
            aggregate_cell_content(&cxml[from..cxml.len().saturating_sub("</w:tc>".len())]);
        dropped += d;
        content.push_str(&kept);
    }
    // 全占位合并（各格皆空）→ 聚合内容无段，补一个占位段满足「tc 至少含 1 段」
    if !content.contains("<w:p") {
        content.push_str("<w:p/>");
    }
    let open_end = head_xml.find('>')? + 1;
    let (ts, te) = find_element_span(&new_head, "tcPr")?; // upsert 后必在
    let merged = format!(
        "{}{}{}</w:tc>",
        &head_xml[..open_end],
        &new_head[ts..te],
        content
    );
    let new_row = format!(
        "{}{merged}{}",
        &row_xml[..range[0].0],
        &row_xml[range[last - first].1..]
    );
    Some((
        format!("{}{new_row}{}", &block_xml[..rs], &block_xml[re..]),
        sum,
        dropped,
    ))
}

/// 简单纵并原语：首格 (row,cell) 起同网格列区间 span 行——首格 upsert
/// `<w:vMerge w:val="restart"/>`，后续行同网格列区间的格 upsert `<w:vMerge/>`
/// （continue）；内容原样保留（拆分即恢复显示）。
pub(super) fn merge_vertical_xml(block_xml: &str, row: usize, cell: usize, span: usize) -> Option<String> {
    let (hrs, hre) = row_span_of(block_xml, row)?;
    let head_row_xml = &block_xml[hrs..hre];
    let head_cells = direct_children_spans(head_row_xml, "tc");
    let (hcs, hce) = *head_cells.get(cell.checked_sub(1)?)?;
    // 首格网格区间 [g0, g1)（对齐判据）
    let mut g0 = 0u32;
    for (s, e) in head_cells.iter().take(cell - 1) {
        g0 += xml_grid_span(&head_row_xml[*s..*e]);
    }
    let g1 = g0 + xml_grid_span(&head_row_xml[hcs..hce]);
    // 逐行容器手术（行 span 互不重叠，按原偏移重组）
    let mut fixed: Vec<(usize, usize, String)> = Vec::new();
    for i in 0..span {
        let r = row + i;
        let (rs2, re2) = row_span_of(block_xml, r)?;
        let row_xml = &block_xml[rs2..re2];
        let (cs2, ce2) = xml_cell_span_at_grid_range(row_xml, g0, g1)?;
        let frag = if i == 0 {
            r#"<w:vMerge w:val="restart"/>"#
        } else {
            "<w:vMerge/>"
        };
        let (new_cell, _) = set_container_element_xml(
            &row_xml[cs2..ce2],
            "tcPr",
            "vMerge",
            Some(frag),
            &TCPR_ELEMENTS,
        )?;
        fixed.push((
            rs2,
            re2,
            format!("{}{new_cell}{}", &row_xml[..cs2], &row_xml[ce2..]),
        ));
    }
    let mut out = String::with_capacity(block_xml.len() + 64);
    let mut cursor = 0usize;
    for (rs2, re2, nr) in fixed {
        out.push_str(&block_xml[cursor..rs2]);
        out.push_str(&nr);
        cursor = re2;
    }
    out.push_str(&block_xml[cursor..]);
    Some(out)
}

/// merge_cells 手术（Word 原生语义，两形态）：
/// - 简单线并（direction+span）：横并原语或纵并原语单发
/// - 矩形区（end_row+end_cell）：逐行横并（行互不重叠，倒序保原始偏移）→
///   结果列纵并——Word 合并区域 UX 同款，一次调用完成多行多列合并
pub(super) fn merge_cells_xml(
    block_xml: &str,
    direction: Option<MergeDirection>,
    row: usize,
    cell: usize,
    span: Option<usize>,
    end_row: Option<usize>,
    end_cell: Option<usize>,
) -> Option<(String, String)> {
    match (end_row, end_cell) {
        (Some(er), Some(ec)) => {
            let width = ec - cell + 1;
            let rows_n = er - row + 1;
            let mut out = block_xml.to_string();
            let mut dropped: usize = 0;
            if ec > cell {
                for r in (row..=er).rev() {
                    let (next, _, d) = merge_horizontal_xml(&out, r, cell, width)?;
                    out = next;
                    dropped += d;
                }
            }
            if er > row {
                out = merge_vertical_xml(&out, row, cell, rows_n)?;
            }
            let drop_note = if dropped > 0 {
                format!("，丢弃 {dropped} 个占位段")
            } else {
                String::new()
            };
            Some((
                out,
                format!(
                    "r{row}c{cell}..r{er}c{ec} 矩形合并（{width} 格宽 × {rows_n} 行{drop_note}）"
                ),
            ))
        }
        _ => {
            let dir = direction?; // 预检已挡缺参
            let s = span.unwrap_or(2);
            match dir {
                MergeDirection::Horizontal => merge_horizontal_xml(block_xml, row, cell, s)
                    .map(|(out, sum, dropped)| {
                        let drop_note = if dropped > 0 {
                            format!("，丢弃 {dropped} 个占位段")
                        } else {
                            String::new()
                        };
                        (
                            out,
                            format!("r{row} c{cell} 起横并 {s} 格（跨 {sum} 列，内容按序拼接{drop_note}）"),
                        )
                    }),
                MergeDirection::Vertical => merge_vertical_xml(block_xml, row, cell, s)
                    .map(|out| {
                        (
                            out,
                            format!("r{row}c{cell} 起纵并 {s} 行（内容保留，split_cell 即恢复）"),
                        )
                    }),
            }
        }
    }
}

/// split_cell 手术（merge_cells 的逆）：
/// - vertical：对 (合并头) 拆整条纵并链——首格摘 vMerge(restart)，沿同网格列
///   区间向下摘 vMerge(continue) 至链尾；各格内容原样恢复显示
/// - horizontal：对 (跨N列) 格拆回 N 个单格——首格保内容、tcPr 摘 gridSpan，
///   其余 N-1 格 = 同开标签 + 同 tcPr（无 gridSpan）+ 空段（继承首段 pPr）
pub(super) fn split_cell_xml(
    block_xml: &str,
    direction: MergeDirection,
    row: usize,
    cell: usize,
) -> Option<(String, String)> {
    match direction {
        MergeDirection::Vertical => {
            let rows = direct_children_spans(block_xml, "tr");
            let (hrs, hre) = *rows.get(row.checked_sub(1)?)?;
            let head_row_xml = &block_xml[hrs..hre];
            let head_cells = direct_children_spans(head_row_xml, "tc");
            let (hcs, hce) = *head_cells.get(cell.checked_sub(1)?)?;
            let mut g0 = 0u32;
            for (s, e) in head_cells.iter().take(cell - 1) {
                g0 += xml_grid_span(&head_row_xml[*s..*e]);
            }
            let g1 = g0 + xml_grid_span(&head_row_xml[hcs..hce]);
            // 沿链收集待摘格：首格必 restart（预检已验）；向下遇非 continue 即链尾
            let mut chain: Vec<(usize, usize, usize, usize)> = Vec::new(); // (行起, 行止, 格起, 格止)
            for i in 0..rows.len().saturating_sub(row - 1) {
                let r = row + i;
                let (rs2, re2) = *rows.get(r - 1)?;
                let row_xml = &block_xml[rs2..re2];
                let Some((cs2, ce2)) = xml_cell_span_at_grid_range(row_xml, g0, g1) else {
                    break; // 边界不对齐 → 链断
                };
                let vm = xml_v_merge(&row_xml[cs2..ce2]);
                if i == 0 {
                    if vm.as_deref() != Some("restart") {
                        return None; // 预检已挡——防御
                    }
                    chain.push((rs2, re2, cs2, ce2));
                } else {
                    match vm.as_deref() {
                        Some("continue") => chain.push((rs2, re2, cs2, ce2)),
                        _ => break, // 无合并 / 新链头 restart → 链尾
                    }
                }
            }
            let count = chain.len();
            let mut out = String::with_capacity(block_xml.len());
            let mut cursor = 0usize;
            for (rs2, re2, cs2, ce2) in chain {
                let row_xml = &block_xml[rs2..re2];
                let (new_cell, _) = set_container_element_xml(
                    &row_xml[cs2..ce2],
                    "tcPr",
                    "vMerge",
                    None,
                    &TCPR_ELEMENTS,
                )?;
                out.push_str(&block_xml[cursor..rs2]);
                out.push_str(&format!("{}{new_cell}{}", &row_xml[..cs2], &row_xml[ce2..]));
                cursor = re2;
            }
            out.push_str(&block_xml[cursor..]);
            Some((
                out,
                format!("r{row}c{cell} 纵并链拆分，{count} 格恢复独立显示"),
            ))
        }
        MergeDirection::Horizontal => {
            let (rs, re) = row_span_of(block_xml, row)?;
            let row_xml = &block_xml[rs..re];
            let (cs, ce) = cell_span_of(row_xml, cell)?;
            let cell_xml = &row_xml[cs..ce];
            let n = xml_grid_span(cell_xml);
            if n < 2 {
                return None; // 预检已挡——防御
            }
            // 摘 gridSpan（原 tcPr 只含 gridSpan 时容器整体清理）
            let (no_span_cell, _) =
                set_container_element_xml(cell_xml, "tcPr", "gridSpan", None, &TCPR_ELEMENTS)?;
            // 首段 pPr 模板（空格继承）
            let open_end = cell_xml.find('>')? + 1;
            let from = find_element_span(&no_span_cell, "tcPr")
                .map(|(_, e)| e)
                .unwrap_or(open_end);
            let content = &no_span_cell[from..no_span_cell.len().saturating_sub("</w:tc>".len())];
            let fp = find_element_span(content, "p")?;
            let ppr = slice_ppr(&content[fp.0..fp.1]);
            // N-1 个空格：同开标签 + 同 tcPr（若摘 gridSpan 后仍在）+ 空段
            let tcpr_left = find_element_span(&no_span_cell, "tcPr");
            let mut extras = String::new();
            for _ in 0..n - 1 {
                extras.push_str(&cell_xml[..open_end]);
                if let Some((ts, te)) = tcpr_left {
                    extras.push_str(&no_span_cell[ts..te]);
                }
                extras.push_str(&format!("<w:p>{ppr}</w:p>"));
                extras.push_str("</w:tc>");
            }
            let new_row = format!("{}{no_span_cell}{extras}{}", &row_xml[..cs], &row_xml[ce..]);
            let out = format!("{}{new_row}{}", &block_xml[..rs], &block_xml[re..]);
            Some((out, format!("r{row}c{cell} 横并拆回 {n} 格（内容留首格）")))
        }
    }
}

/// 删表格一行（S3 七波·生产反馈 P0）：direct_children_spans 定位直接子 tr
/// （嵌套表的内层 tr 不计），整段摘除。合并链与末行守卫在预检。
pub(super) fn delete_table_row_xml(table_xml: &str, row: usize) -> Option<(String, String)> {
    let rows = direct_children_spans(table_xml, "tr");
    let (rs, re) = *rows.get(row.checked_sub(1)?)?;
    let mut out = String::with_capacity(table_xml.len());
    out.push_str(&table_xml[..rs]);
    out.push_str(&table_xml[re..]);
    Some((out, format!("删第 {row} 行（剩 {} 行）", rows.len() - 1)))
}

// =========================================================================
// 容器重打包（只换 document.xml，其余 entry 原样字节复制）
// =========================================================================

/// 重打包 docx：`part` 指定的 entry 替换为 `new_xml`（document.xml / styles.xml /
/// numbering.xml 通用），其余 entry 经 `raw_copy_file` 原样复制（不解压重压——
/// 压缩参数与元数据不变）。
pub(in crate::harness::doc) fn repack_part(bytes: &[u8], part: &str, new_xml: &str) -> AppResult<Vec<u8>> {
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
            if name == part {
                // 逐 entry 借用冲突：目标部件先收名，循环外统一写
                drop(entry);
                w.start_file(part, zip::write::SimpleFileOptions::default())
                    .map_err(|e| AppError::Internal(format!("重打包 {part} 失败: {e}")))?;
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
