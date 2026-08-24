//! `styles` —— `word/styles.xml` 解析 + 样式链有效格式合并（S0b）。
//!
//! 文档里的 `w:pStyle w:val="2"` 是样式 **ID**，对 LLM 无意义；「这段什么格式」
//! 的正确答案是**有效格式**：直接格式 > 样式链（basedOn 逐级） > docDefaults。
//! 本模块提供三件：
//! - [`parse_styles`]：styles.xml → [`Stylesheet`]（id/显示名/basedOn/outlineLvl/
//!   样式内 rPr+pPr；docDefaults 兜底格式）
//! - [`Stylesheet::resolve_chain`]：styleId → basedOn 链（防环，≤10 级）
//! - [`effective_run`] / [`effective_para`]：三层数据源的逐字段合并
//!
//! 样式链合并只覆盖字符格式（RunProps）与段落格式（ParaProps）；linked style /
//! 表格样式 / 主题字体（majorTheme）等长尾不展开（显示名照抄，解析值缺就空）。

use std::collections::HashMap;

use super::docx_model::{parse_para_props, parse_run_props, ParaProps, RunProps};
use super::xml_dom::Element;

/// 单个样式定义（w:style）。
pub(super) struct StyleDef {
    pub id: String,
    /// w:name val（显示名，如 "heading 1"）
    pub name: String,
    /// w:basedOn val（父样式 ID）
    pub based_on: Option<String>,
    /// w:pPr/w:outlineLvl val（0 = 一级标题 … 8 = 九级；大纲层级来源）
    pub outline_lvl: Option<u32>,
    pub run_props: RunProps,
    pub para_props: ParaProps,
}

/// 整张样式表。
pub(super) struct Stylesheet {
    /// w:docDefaults/w:rPrDefault/w:rPr（文档级默认字符格式，合并链最底层）
    pub doc_default_run: RunProps,
    /// w:docDefaults/w:pPrDefault/w:pPr（文档级默认段落格式）
    pub doc_default_para: ParaProps,
    by_id: HashMap<String, StyleDef>,
}

impl Stylesheet {
    /// 空样式表（styles.xml 缺失 / 解析无果时——有效格式退化为「直接格式 + 空」）。
    pub(super) fn empty() -> Self {
        Stylesheet {
            doc_default_run: RunProps::default(),
            doc_default_para: ParaProps::default(),
            by_id: HashMap::new(),
        }
    }

    /// styleId → 显示名（未知 ID 返回 None，调用方回退显示原始 ID）。
    pub(super) fn name_of(&self, id: &str) -> Option<&str> {
        self.by_id.get(id).map(|s| s.name.as_str())
    }

    /// styleId → 大纲级别（未知样式 / 无 outlineLvl 返回 None）。
    pub(super) fn outline_lvl_of(&self, id: &str) -> Option<u32> {
        self.by_id.get(id).and_then(|s| s.outline_lvl)
    }

    /// styleId → basedOn 继承链（自身在前，逐级向父；防环、最多 10 级）。
    pub(super) fn resolve_chain<'a>(&'a self, id: &str) -> Vec<&'a StyleDef> {
        let mut chain = Vec::new();
        let mut current = id;
        let mut hops = 0;
        while let Some(style) = self.by_id.get(current) {
            chain.push(style);
            hops += 1;
            if hops > 10 {
                break; // 深链防御（正常样式表 ≤5 级）
            }
            match style.based_on.as_deref() {
                Some(parent) => {
                    // 环防御：父已入链则停（A→B→A）
                    if chain.iter().any(|s| s.id == parent) {
                        break;
                    }
                    current = parent;
                }
                None => break,
            }
        }
        chain
    }
}

/// 解析 styles.xml 根元素（`w:styles`；其他根宽容遍历，同 docx_model 策略）。
pub(super) fn parse_styles(root: &Element) -> Stylesheet {
    let mut sheet = Stylesheet::empty();
    for el in root.child_elements() {
        match el.name.as_str() {
            "w:docDefaults" => {
                if let Some(rpd) = el
                    .child_elements()
                    .find(|e| e.name == "w:rPrDefault")
                {
                    if let Some(rpr) = rpd.child_elements().find(|e| e.name == "w:rPr") {
                        sheet.doc_default_run = parse_run_props(rpr);
                    }
                }
                if let Some(ppd) = el
                    .child_elements()
                    .find(|e| e.name == "w:pPrDefault")
                {
                    if let Some(ppr) = ppd.child_elements().find(|e| e.name == "w:pPr") {
                        sheet.doc_default_para = parse_para_props(ppr);
                    }
                }
            }
            "w:style" => {
                if let Some(def) = parse_style(el) {
                    sheet.by_id.insert(def.id.clone(), def);
                }
            }
            // w:latentStyles / w:sym 等无消费价值，跳过
            _ => {}
        }
    }
    sheet
}

fn parse_style(el: &Element) -> Option<StyleDef> {
    let id = el.attr("w:styleId")?.to_string();
    let mut def = StyleDef {
        id,
        name: String::new(),
        based_on: None,
        outline_lvl: None,
        run_props: RunProps::default(),
        para_props: ParaProps::default(),
    };
    for child in el.child_elements() {
        match child.name.as_str() {
            "w:name" => def.name = child.attr("w:val").unwrap_or_default().to_string(),
            "w:basedOn" => def.based_on = child.attr("w:val").map(str::to_string),
            "w:rPr" => def.run_props = parse_run_props(child),
            "w:pPr" => {
                def.para_props = parse_para_props(child);
                def.outline_lvl = child
                    .child_elements()
                    .find(|e| e.name == "w:outlineLvl")
                    .and_then(|e| e.attr("w:val"))
                    .and_then(|v| v.parse().ok());
            }
            _ => {}
        }
    }
    if def.name.is_empty() {
        def.name = def.id.clone(); // 无名样式回退显示 ID
    }
    Some(def)
}

// =========================================================================
// 有效格式合并（直接 > 样式链 > docDefaults，逐字段取第一个 Some）
// =========================================================================

/// 字段级合并辅助：直接指定优先，否则取链上第一个 Some，最后 docDefaults。
/// 字段路径形如 `run_props.bold`（链元素是 StyleDef，格式属性在子结构里）。
macro_rules! pick {
    ($direct:expr, $chain:expr, $default:expr, $container:ident . $field:ident) => {
        $direct
            .$field
            .clone()
            .or_else(|| $chain.iter().find_map(|s| s.$container.$field.clone()))
            .or_else(|| $default.$field.clone())
    };
}

/// run 有效字符格式。
pub(super) fn effective_run(
    direct: &RunProps,
    chain: &[&StyleDef],
    doc_default: &RunProps,
) -> RunProps {
    RunProps {
        bold: pick!(direct, chain, doc_default, run_props.bold),
        italic: pick!(direct, chain, doc_default, run_props.italic),
        underline: pick!(direct, chain, doc_default, run_props.underline),
        strike: pick!(direct, chain, doc_default, run_props.strike),
        size_half_pt: pick!(direct, chain, doc_default, run_props.size_half_pt),
        color: pick!(direct, chain, doc_default, run_props.color),
        highlight: pick!(direct, chain, doc_default, run_props.highlight),
        font_east_asia: pick!(direct, chain, doc_default, run_props.font_east_asia),
        font_ascii: pick!(direct, chain, doc_default, run_props.font_ascii),
    }
}

/// 段落有效格式（style / numbering 不参与合并——它们本身是引用语义）。
pub(super) fn effective_para(
    direct: &ParaProps,
    chain: &[&StyleDef],
    doc_default: &ParaProps,
) -> ParaProps {
    ParaProps {
        style: direct.style.clone(),
        numbering: direct.numbering,
        alignment: pick!(direct, chain, doc_default, para_props.alignment),
        spacing_line: pick!(direct, chain, doc_default, para_props.spacing_line),
        line_rule: pick!(direct, chain, doc_default, para_props.line_rule),
        spacing_before: pick!(direct, chain, doc_default, para_props.spacing_before),
        spacing_after: pick!(direct, chain, doc_default, para_props.spacing_after),
        indent_first_line: pick!(direct, chain, doc_default, para_props.indent_first_line),
        indent_hanging: pick!(direct, chain, doc_default, para_props.indent_hanging),
        indent_left: pick!(direct, chain, doc_default, para_props.indent_left),
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::super::xml_dom;
    use super::*;

    fn sheet(xml: &str) -> Stylesheet {
        let dom = xml_dom::parse(xml).unwrap();
        parse_styles(&dom)
    }

    #[test]
    fn parses_style_defs_and_defaults() {
        let s = sheet(
            r#"<w:styles>
                <w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val="21"/><w:rFonts w:eastAsia="宋体"/></w:rPr></w:rPrDefault></w:docDefaults>
                <w:style w:type="paragraph" w:styleId="2">
                    <w:name w:val="heading 1"/><w:basedOn w:val="1"/>
                    <w:pPr><w:outlineLvl w:val="0"/><w:jc w:val="center"/></w:pPr>
                    <w:rPr><w:b/><w:sz w:val="32"/></w:rPr>
                </w:style>
                <w:style w:type="paragraph" w:styleId="1"><w:name w:val="Normal"/><w:rPr><w:color w:val="000000"/></w:rPr></w:style>
            </w:styles>"#,
        );
        assert_eq!(s.name_of("2"), Some("heading 1"));
        assert_eq!(s.outline_lvl_of("2"), Some(0));
        assert_eq!(s.doc_default_run.size_half_pt, Some(21));
        assert_eq!(s.doc_default_run.font_east_asia.as_deref(), Some("宋体"));
        assert_eq!(s.name_of("missing"), None);
    }

    #[test]
    fn chain_resolution_inheritance_order() {
        let s = sheet(
            r#"<w:styles>
                <w:style w:styleId="3"><w:name w:val="heading 2"/><w:basedOn w:val="2"/><w:rPr><w:i/></w:rPr></w:style>
                <w:style w:styleId="2"><w:name w:val="heading 1"/><w:basedOn w:val="1"/><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
                <w:style w:styleId="1"><w:name w:val="Normal"/><w:rPr><w:color w:val="000000"/></w:rPr></w:style>
            </w:styles>"#,
        );
        let chain = s.resolve_chain("3");
        assert_eq!(chain.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["3", "2", "1"]);
        // 近者胜：sz 取 "2" 的 32，color 取 "1" 的 000000
        let empty = RunProps::default();
        let eff = effective_run(&empty, &chain, &empty);
        assert_eq!(eff.size_half_pt, Some(32));
        assert_eq!(eff.color.as_deref(), Some("000000"));
        assert_eq!(eff.italic, Some(true));
        // 直接格式覆盖一切
        let direct = RunProps { size_half_pt: Some(28), ..Default::default() };
        let eff = effective_run(&direct, &chain, &empty);
        assert_eq!(eff.size_half_pt, Some(28));
        // docDefaults 兜底（链上没人给 underline）
        let dd = RunProps { underline: Some(true), ..Default::default() };
        let eff = effective_run(&empty, &chain, &dd);
        assert_eq!(eff.underline, Some(true));
    }

    #[test]
    fn cyclic_based_on_terminates() {
        let s = sheet(
            r#"<w:styles>
                <w:style w:styleId="a"><w:name w:val="A"/><w:basedOn w:val="b"/></w:style>
                <w:style w:styleId="b"><w:name w:val="B"/><w:basedOn w:val="a"/></w:style>
            </w:styles>"#,
        );
        let chain = s.resolve_chain("a");
        assert_eq!(chain.len(), 2); // a → b 停（a 已在链中）
    }

    #[test]
    fn para_effective_merge_and_direct_wins() {
        let s = sheet(
            r#"<w:styles>
                <w:style w:styleId="p1"><w:name w:val="Style1"/><w:pPr><w:spacing w:line="360" w:lineRule="auto" w:before="120"/><w:ind w:firstLine="480"/></w:pPr></w:style>
            </w:styles>"#,
        );
        let chain = s.resolve_chain("p1");
        let eff = effective_para(&ParaProps::default(), &chain, &ParaProps::default());
        assert_eq!(eff.spacing_line, Some(360));
        assert_eq!(eff.line_rule.as_deref(), Some("auto"));
        assert_eq!(eff.spacing_before, Some(120));
        assert_eq!(eff.indent_first_line, Some(480));
        // 直接对齐覆盖样式
        let direct = ParaProps { alignment: Some("right".into()), ..Default::default() };
        let eff = effective_para(&direct, &chain, &ParaProps::default());
        assert_eq!(eff.alignment.as_deref(), Some("right"));
    }

    #[test]
    fn unnamed_style_falls_back_to_id() {
        let s = sheet(
            r#"<w:styles><w:style w:styleId="42"><w:rPr><w:b/></w:rPr></w:style></w:styles>"#,
        );
        assert_eq!(s.name_of("42"), Some("42"));
    }
}
