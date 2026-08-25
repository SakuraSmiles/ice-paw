//! `def_edit` —— `word/styles.xml` / `word/numbering.xml` 定义部件手术（D12）。
//!
//! D9 的 set_ppr_element 证明了「封闭 schema 白名单 + 通用元素手术」对段落
//! 格式长尾的收敛力；本模块把同一哲学推到**样式与编号定义**——「一次定义、
//! 处处引用、可统一改」正是 Word 样式系统的本职：改一个标题样式的定义，
//! 全文引用处全部生效，这是逐段 set_format 永远给不了的。
//!
//! 三操作：
//! - `create_style`：最小出生（type/name/basedOn/qFormat）；细节同批
//!   set_style_element 补——寻址放应用期，create→set 天然可组合
//! - `set_style_element`：w:style 子树内元素手术。container 四档：
//!   style（直接子级）/ pPr / rPr / tblPr；xml=None 摘除 / Some 整元素
//!   替换或按 schema 位插入；容器缺则新建、摘空则清容器
//! - `set_numbering_element`：numbering.xml 里 abstractNum 的 w:lvl 级元素
//!   手术（numId 经 w:num 实例解析到 abstract；lvlOverride 不开刀）
//!
//! 不变式：
//! - **结构性寻位**：一切定位都基于 [`root_children`] 的直接子级偏移——
//!   w:style 内的 tblStylePr 也含 pPr/rPr/tblPr/trPr/tcPr，裸 find `<w:pPr`
//!   会撞进条件格式子树（D11 前缀碰撞纪律的结构版）
//! - 目标定义子树含任一 `*Change`（修订记录）拒改——先在 Word 接受修订
//! - latentStyles / docDefaults 永不碰；sectPr 类受保护于白名单缺席
//! - 产物必过 xml_dom::parse + w:style / w:lvl 计数守恒；只重打包目标部件
//!
//! 纯函数（bytes 进新 bytes 出，无 IO）；备份/原子写在 mcp::docx_tool 壳。

use std::collections::HashSet;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::error::{AppError, AppResult};

use super::docx::read_entry;
use super::docx_edit::{
    parse_attrs, repack_part, truncate, validate_fragment, AppliedOp, PPR_ELEMENTS,
    TBLPR_ELEMENTS,
};
use super::numbering::parse_numbering;
use super::styles::{parse_styles, Stylesheet};
use super::xml_dom;

// =========================================================================
// 白名单（ECMA-376 schema 序；双职 = 合法性 + 插入位序）
// =========================================================================

/// CT_Style 法定子元素，ECMA-376 schema 序。sectPr 不入 = 受保护于缺席
/// （分节符不归样式管）；name 拒摘除（样式身份，见 guard_style_element）。
const STYLE_ELEMENTS: [&str; 22] = [
    "name", "aliases", "basedOn", "next", "link", "autoRedefine", "hidden",
    "uiPriority", "semiHidden", "unhideWhenUsed", "qFormat", "locked",
    "personal", "personalCompose", "personalReply", "rsid",
    "pPr", "rPr", "tblPr", "trPr", "tcPr", "tblStylePr",
];

/// CT_RPr 法定子元素（style 的 rPr 容器内；docx_edit::apply_char_formats
/// 硬编码序的完整化）。
const RPR_ELEMENTS: [&str; 39] = [
    "rStyle", "rFonts", "b", "bCs", "i", "iCs", "caps", "smallCaps", "strike",
    "dstrike", "outline", "shadow", "emboss", "imprint", "noProof",
    "snapToGrid", "vanish", "webHidden", "color", "spacing", "w", "kern",
    "position", "sz", "szCs", "highlight", "u", "effect", "bdr", "shd",
    "fitText", "vertAlign", "rtl", "cs", "em", "lang", "eastAsianLayout",
    "specVanish", "oMath",
];

/// CT_Lvl 法定子元素（numbering 的 w:lvl 内）。pPr/rPr 整元素替换——片段从
/// projection=numbering 看到的原文复制。
const LVL_ELEMENTS: [&str; 12] = [
    "start", "numFmt", "lvlRestart", "pStyle", "isLgl", "suff", "lvlText",
    "lvlPicBulletId", "legacy", "lvlJc", "pPr", "rPr",
];

/// 修订记录元素：目标定义子树 / 片段中出现任一即拒改。
const CHANGE_ELEMENTS: [&str; 5] = [
    "pPrChange", "rPrChange", "tblPrChange", "trPrChange", "tcPrChange",
];

// =========================================================================
// 操作类型
// =========================================================================

/// w:style 的 @w:type 四值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleType {
    Paragraph,
    Character,
    Table,
    Numbering,
}

impl StyleType {
    fn as_str(self) -> &'static str {
        match self {
            StyleType::Paragraph => "paragraph",
            StyleType::Character => "character",
            StyleType::Table => "table",
            StyleType::Numbering => "numbering",
        }
    }
}

/// set_style_element 的容器四档：style 直接子级 / 段落格式 / 字符格式 / 表属性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleContainer {
    Style,
    PPr,
    RPr,
    TblPr,
}

impl StyleContainer {
    /// 容器标签名（不含 w: 前缀；style 档 = w:style 自身的直接子级）。
    fn tag(self) -> &'static str {
        match self {
            StyleContainer::Style => "style",
            StyleContainer::PPr => "pPr",
            StyleContainer::RPr => "rPr",
            StyleContainer::TblPr => "tblPr",
        }
    }

    /// 容器内合法子元素集（schema 序）。pPr/tblPr 复用文档级白名单。
    fn whitelist(self) -> &'static [&'static str] {
        match self {
            StyleContainer::Style => &STYLE_ELEMENTS,
            StyleContainer::PPr => &PPR_ELEMENTS,
            StyleContainer::RPr => &RPR_ELEMENTS,
            StyleContainer::TblPr => &TBLPR_ELEMENTS,
        }
    }
}

/// styles.xml 定义操作（edit_docx 的 style 族，全批或全无）。
#[derive(Debug, Clone)]
pub enum StyleEditOp {
    /// 新建样式（最小出生：type/name/basedOn/qFormat）。
    CreateStyle {
        style_type: StyleType,
        name: String,
        /// 缺省由显示名去空白派生。
        style_id: Option<String>,
        based_on: Option<String>,
    },
    /// w:style 子树内元素手术。
    SetStyleElement {
        /// 样式显示名或 ID（重名显示名拒，指路 ID）。
        style: String,
        container: StyleContainer,
        element: String,
        /// None=摘除；Some=整元素替换或按 schema 位插入。
        xml: Option<String>,
    },
}

/// numbering.xml 定义操作（edit_docx 的 numbering 族，全批或全无）。
#[derive(Debug, Clone)]
pub enum NumberingEditOp {
    /// abstractNum 的 w:lvl 级元素手术。
    SetNumberingElement {
        num_id: u32,
        /// ilvl（0-8）。
        level: u32,
        element: String,
        /// None=摘除；Some=整元素替换或按 schema 位插入。
        xml: Option<String>,
    },
}

// =========================================================================
// 结构性寻位（直接子级清单——免疫嵌套同名误配）
// =========================================================================

/// 根元素的直接子级（偏移相对入参切片）。
pub(super) struct Child {
    pub(super) start: usize,
    pub(super) end: usize,
    /// 全名（含 w: 前缀）。
    pub(super) name: String,
    /// 仅测试消费（语料选段跳过自闭合形态）；生产路径按需展开，不读此标记。
    #[allow(dead_code)]
    pub(super) self_closed: bool,
}

/// 走根元素的直接子级清单。形态异常（未闭合 / 多根 / 坏 XML）→ None。
/// （docx_inspect 的 styledef/numbering 原文投影共用——同一结构寻位，同一口径）
pub(super) fn root_children(s: &str) -> Option<Vec<Child>> {
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(false); // 偏移与原始字节对齐
    let mut out = Vec::new();
    let mut depth = 0usize; // 相对根：直接子级 = 0（其 Start 后变 1）
    let mut root_seen = false;
    let mut open: Option<(usize, String)> = None; // 匹配中的直接子级
    loop {
        let ev_start = reader.buffer_position() as usize;
        let ev = reader.read_event().ok()?;
        let ev_end = reader.buffer_position() as usize;
        match ev {
            Event::Start(e) => {
                if !root_seen {
                    root_seen = true; // 根自身不入子级
                } else {
                    if depth == 0 && open.is_none() {
                        open = Some((
                            ev_start,
                            String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                        ));
                    }
                    depth += 1;
                }
            }
            Event::End(_) => {
                if depth == 0 {
                    break; // 根闭合
                }
                depth -= 1;
                if depth == 0 {
                    if let Some((st, name)) = open.take() {
                        out.push(Child { start: st, end: ev_end, name, self_closed: false });
                    }
                }
            }
            Event::Empty(e) => {
                if !root_seen {
                    root_seen = true; // 自闭合根本身（合法，无子级）
                } else if depth == 0 {
                    out.push(Child {
                        start: ev_start,
                        end: ev_end,
                        name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                        self_closed: true,
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_seen || depth != 0 || open.is_some() {
        return None;
    }
    Some(out)
}

/// 子级开标签属性值（如 w:styleId；`s` 须从该子级起始处切片）。
pub(super) fn child_attr(s: &str, child: &Child, attr: &str) -> Option<String> {
    parse_attrs(&s[child.start..child.end])
        .into_iter()
        .find(|(k, _)| k == attr)
        .map(|(_, v)| v)
}

/// 子串里首个修订记录元素名（子树与片段共用探测）。
pub(super) fn change_marker(s: &str) -> Option<&'static str> {
    CHANGE_ELEMENTS
        .iter()
        .copied()
        .find(|n| s.contains(&format!("<w:{n}")))
}

// =========================================================================
// 元素手术原语
// =========================================================================

/// 父串直接子级里的替换 / schema 位插入 / 摘除。
/// 返回 (新父串, 是否实际改动, 摘除后父是否已无子元素——调用方决定是否清父)。
/// 「父已无子元素」只在摘除路径置位；None = 形态异常。
fn set_child_element(
    parent: &str,
    element: &str,
    frag: Option<&str>,
    whitelist: &[&str],
) -> Option<(String, bool, bool)> {
    let children = root_children(parent)?;
    let tag = format!("w:{element}");
    let Some(f) = frag else {
        let Some(hit) = children.iter().find(|c| c.name == tag) else {
            return Some((parent.to_string(), false, false)); // 不存在 → 空转
        };
        let new_parent = format!("{}{}", &parent[..hit.start], &parent[hit.end..]);
        let empty = root_children(&new_parent)?.is_empty();
        return Some((new_parent, true, empty));
    };
    if let Some(hit) = children.iter().find(|c| c.name == tag) {
        return Some((
            format!("{}{f}{}", &parent[..hit.start], &parent[hit.end..]),
            true,
            false,
        ));
    }
    // 插入位：schema 序中排在 element 之后的直接子级最早出现处之前（结构寻位，
    // 免疫嵌套同名误配——裸 find 会撞进 tblStylePr 内的 pPr 等）；一个都没有
    // 则落根内容末尾（闭合标签前——直接 append 串尾会掉到根元素外面）。
    // 自闭合根无内容区：先展开成对标签（仅插入路径展开——摘除/替换走不进
    // 这里，空转保持逐字节不变）。
    let idx = whitelist.iter().position(|n| *n == element)?;
    let later: HashSet<&str> = whitelist[idx + 1..].iter().copied().collect();
    let insert_at = children
        .iter()
        .filter(|c| later.contains(c.name.strip_prefix("w:").unwrap_or(&c.name)))
        .map(|c| c.start)
        .min();
    let inserted = match insert_at {
        Some(at) => format!("{}{f}{}", &parent[..at], &parent[at..]),
        None => {
            let base = if parent.ends_with("/>") {
                expand_self_closed_root(parent)?
            } else {
                parent.to_string()
            };
            let end = base.rfind("</")?;
            format!("{}{f}{}", &base[..end], &base[end..])
        }
    };
    Some((inserted, true, false))
}

/// 自闭合根展开成对标签（`<w:pPr a="b"/>` → `<w:pPr a="b"></w:pPr>`）。
/// 入参须是以 `/>` 收尾的单根元素串。
fn expand_self_closed_root(s: &str) -> Option<String> {
    let gt = s.find('>')?;
    let head = &s[..gt - 1]; // 去掉收尾 "/>"，留 "<w:pPr a=\"b\""
    let name = head
        .strip_prefix('<')?
        .split([' ', '\t', '\r', '\n'])
        .next()?; // 已含前缀（如 "w:style"）
    Some(format!("{head}></{name}>"))
}

/// 手术结果（摘要标注用）。
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Changed,
    /// 摘除目标本就不存在（容器缺 / 元素缺）——空转，显式报出防误读。
    Absent,
}

/// w:style 子串内的元素手术（container 三容器档 + style 直接子级档）。
/// 返回 (新 style 串, 结果, 摘要摘录)；None = 形态异常。自闭合的 style 根与
/// 自闭合空容器的展开统一由 set_child_element 插入路径处理（摘除路径不展开
/// ——空转保持逐字节不变）。
fn style_element_surgery(
    style_xml: &str,
    container: StyleContainer,
    element: &str,
    frag: Option<&str>,
) -> Option<(String, Outcome, String)> {
    if container == StyleContainer::Style {
        let (out, changed, _) = set_child_element(style_xml, element, frag, &STYLE_ELEMENTS)?;
        let outcome = if changed { Outcome::Changed } else { Outcome::Absent };
        return Some((out.clone(), outcome, out));
    }

    let tag = format!("w:{}", container.tag());
    let children = root_children(style_xml)?;
    let hit = children.iter().find(|c| c.name == tag);
    let Some(hit) = hit else {
        // 容器缺失：摘除 → 空转；替换 → 按 CT_Style 序新建容器（内含片段）
        return match frag {
            None => Some((style_xml.to_string(), Outcome::Absent, String::new())),
            Some(f) => {
                let new_container = format!("<w:{0}>{f}</w:{0}>", container.tag());
                let (out, _, _) = set_child_element(
                    style_xml,
                    container.tag(),
                    Some(&new_container),
                    &STYLE_ELEMENTS,
                )?;
                Some((out, Outcome::Changed, new_container))
            }
        };
    };

    let cxml = &style_xml[hit.start..hit.end];
    let (new_c, changed, now_empty) =
        set_child_element(cxml, element, frag, container.whitelist())?;
    if now_empty {
        // 摘空 → 整容器清理（Word 自身也这样清理）
        let out = format!("{}{}", &style_xml[..hit.start], &style_xml[hit.end..]);
        return Some((out, Outcome::Changed, "（容器已随摘空清理）".to_string()));
    }
    let outcome = if changed { Outcome::Changed } else { Outcome::Absent };
    Some((
        format!("{}{new_c}{}", &style_xml[..hit.start], &style_xml[hit.end..]),
        outcome,
        new_c,
    ))
}

// =========================================================================
// 校验 guard（首行 = 稳定家族前缀）
// =========================================================================

/// element 合法性 + 特殊保护（片段校验之前）。
fn guard_style_element(
    container: StyleContainer,
    element: &str,
    xml: Option<&str>,
) -> AppResult<()> {
    if container == StyleContainer::Style && element == "name" && xml.is_none() {
        return Err(AppError::Validation(
            "受保护: 样式显示名（w:name）不可摘除——它是样式身份。改名 = element=name 且 \
             xml 提供新的 <w:name w:val=\"新名\"/>（整元素替换，styleId 不动，全文\
             pStyle 引用不受影响）。"
                .into(),
        ));
    }
    if container == StyleContainer::PPr && element == "rPr" {
        return Err(AppError::Validation(
            "受保护: pPr 内的 rPr（段落标记字符格式）不在开刀范围。改样式的字符格式请用 \
             container=\"rPr\"（样式级字符格式，作用于全部文字）。"
                .into(),
        ));
    }
    let list = container.whitelist();
    if !list.contains(&element) {
        return Err(AppError::Validation(format!(
            "非法子元素: {} 容器内没有 <w:{element}> 子元素。合法元素（schema 序）: {}。\
             原文形态用 inspect_docx projection=styledef 查看，从看到的原文复制修改。",
            container.tag(),
            list.join(" ")
        )));
    }
    Ok(())
}

/// 样式引用（显示名或 ID）→ styleId。重名显示名拒（列全部 ID 指路 ID 寻址
/// ——HashMap 迭代序不确定，重名下显示名寻址不可用）；未知名挂候选。
fn resolve_style_id(sheet: &Stylesheet, style: &str, what: &str) -> AppResult<String> {
    let named = sheet.ids_named(style);
    if named.len() > 1 {
        return Err(AppError::Validation(format!(
            "样式名重复: {what} {style:?} 是重复的显示名，对应多个样式 ID（{}）。\
             请改用 styleId 寻址（style 参数传其中之一）。",
            named.join("、")
        )));
    }
    if let Some(id) = named.first() {
        return Ok((*id).to_string());
    }
    if let Some(id) = sheet.id_of(style) {
        return Ok(id.to_string());
    }
    Err(AppError::Validation(format!(
        "样式不存在: {what} {style:?} 不在本文档样式表。可用样式（前 20）: {}。\
         完整清单用 inspect_docx projection=styles。",
        sheet.display_names_joined(20)
    )))
}

/// 属性值转义（& < > "）。
fn escape_attr(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for ch in v.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

fn malformed_internal() -> AppError {
    AppError::Internal(
        "XML 形态异常（内部 bug，未写盘）: 定义部件结构解析失败，手术未落盘。".into(),
    )
}

/// 全文档 abstractNum 直属 w:lvl 计数（守恒闸；w:num 内的 lvlOverride 不计）。
fn count_lvls(xml: &str) -> Option<usize> {
    let children = root_children(xml)?;
    let mut n = 0;
    for c in children.iter().filter(|c| c.name == "w:abstractNum") {
        n += root_children(&xml[c.start..c.end])?
            .iter()
            .filter(|x| x.name == "w:lvl")
            .count();
    }
    Some(n)
}

// =========================================================================
// 入口（工具层消费）
// =========================================================================

/// styles.xml 定义手术：全批预检（逐操作即时校验）→ 顺序应用 → 守恒闸 →
/// 只重打包 styles.xml。
pub fn apply_style_edits_to_bytes(
    bytes: &[u8],
    ops: &[StyleEditOp],
) -> AppResult<(Vec<u8>, Vec<AppliedOp>)> {
    if ops.is_empty() {
        return Err(AppError::Validation(
            "操作列表为空: styles 批至少需要一个操作（create_style / set_style_element）。"
                .into(),
        ));
    }
    let Some(mut xml) = read_entry(bytes, "word/styles.xml")? else {
        return Err(AppError::Validation(
            "无样式部件: 本文档没有 word/styles.xml（极少见，文档可能由非常规工具生成），\
             无法做样式定义编辑。"
                .into(),
        ));
    };
    let orig_count = root_children(&xml)
        .map(|c| c.iter().filter(|x| x.name == "w:style").count())
        .ok_or_else(malformed_internal)?;

    // 批内去重：同一 (style, container, element) 两刀 = 意图不明（后写覆盖前写
    // 静默发生），显式报出
    let mut seen: HashSet<String> = HashSet::new();
    let mut created = 0usize;
    let mut summaries: Vec<AppliedOp> = Vec::new();

    for op in ops {
        match op {
            StyleEditOp::CreateStyle { style_type, name, style_id, based_on } => {
                let name_trim = name.trim();
                if name_trim.is_empty() {
                    return Err(AppError::Validation(
                        "非法参数: name（样式显示名）不能为空。".into(),
                    ));
                }
                // 寻址/撞名判定基于当前累积态（同批先 create 的样式可见）
                let sheet = parse_styles(&xml_dom::parse(&xml)?);
                let named = sheet.ids_named(name_trim);
                if !named.is_empty() {
                    return Err(AppError::Validation(format!(
                        "样式已存在: 显示名 {name_trim:?} 已被使用（ID: {}）。Word 允许重名\
                         但会混淆——请换名，或用 set_style_element 改现有样式。",
                        named.join("、")
                    )));
                }
                let id = match style_id {
                    Some(sid) => {
                        let t = sid.trim();
                        if t.is_empty() {
                            return Err(AppError::Validation(
                                "非法参数: style_id 不能为空白。".into(),
                            ));
                        }
                        t.to_string()
                    }
                    // 缺省 ID = 显示名去空白（"My Style" → "MyStyle"）
                    None => name_trim.chars().filter(|c| !c.is_whitespace()).collect(),
                };
                if sheet.id_of(&id).is_some() {
                    return Err(AppError::Validation(format!(
                        "样式已存在: styleId {id:?} 已被样式 {:?} 使用。请显式传一个\
                         未占用的 style_id。",
                        sheet.name_of(&id).unwrap_or("")
                    )));
                }
                let based_on_xml = match based_on {
                    Some(b) => {
                        let parent = resolve_style_id(&sheet, b, "based_on 父样式")?;
                        format!("<w:basedOn w:val=\"{}\"/>", escape_attr(&parent))
                    }
                    None => String::new(),
                };
                let el = format!(
                    "<w:style w:type=\"{}\" w:styleId=\"{}\"><w:name w:val=\"{}\"/>{}<w:qFormat/></w:style>",
                    style_type.as_str(),
                    escape_attr(&id),
                    escape_attr(name_trim),
                    based_on_xml,
                );
                // 追加在最后一个 w:style 之后（</w:styles> 前）——latentStyles/
                // docDefaults 永不碰，schema 序上 style 追加尾部天然正确
                let insert_at = xml
                    .rfind("</w:styles>")
                    .ok_or_else(malformed_internal)?;
                xml.insert_str(insert_at, &el);
                created += 1;
                summaries.push(AppliedOp {
                    op: "create_style",
                    block: 0,
                    before: String::new(),
                    after: truncate(&el, 60),
                    style: None,
                    style_unchanged: None,
                    target: Some(format!("style '{}'（ID {}）", name_trim, id)),
                });
            }
            StyleEditOp::SetStyleElement { style, container, element, xml: frag } => {
                guard_style_element(*container, element, frag.as_deref())?;
                if let Some(f) = frag {
                    if let Some(m) = change_marker(f) {
                        return Err(AppError::Validation(format!(
                            "受保护: 片段含修订记录元素 <w:{m}>。请去掉修订标记——修订\
                             历史与手术改动会互相踩踏。"
                        )));
                    }
                    validate_fragment(element, f, "styledef")?;
                }
                let sheet = parse_styles(&xml_dom::parse(&xml)?);
                let id = resolve_style_id(&sheet, style, "样式")?;
                // 批内去重按解析后的 styleId——显示名与 ID 寻址同一目标也命中
                // （同一 (style, container, element) 两刀 = 意图不明，显式报出）
                let key = format!("{id}\u{1}{:?}\u{1}{}", container, element);
                if !seen.insert(key) {
                    return Err(AppError::Validation(format!(
                        "重复操作: 同批对 style {style:?}（ID {id}）的 {}/{} 已有操作。\
                         请合并为一次操作，或拆成两批。",
                        container.tag(),
                        element
                    )));
                }
                // 结构寻位（sheet 解析自同一累积串，必中；不中即形态异常）
                let hit = root_children(&xml)
                    .ok_or_else(malformed_internal)?
                    .into_iter()
                    .find(|c| c.name == "w:style" && child_attr(&xml, c, "w:styleId").as_deref() == Some(id.as_str()))
                    .ok_or_else(malformed_internal)?;
                let style_xml = xml[hit.start..hit.end].to_string();
                if let Some(m) = change_marker(&style_xml) {
                    return Err(AppError::Validation(format!(
                        "受保护: 样式 {style:?} 的定义带修订记录（w:{m}）。请先在 Word 里\
                         接受/拒绝修订再编辑定义。"
                    )));
                }
                let (new_style, outcome, excerpt) =
                    style_element_surgery(&style_xml, *container, element, frag.as_deref())
                        .ok_or_else(malformed_internal)?;
                let display = sheet.name_of(&id).unwrap_or(id.as_str());
                let after = match outcome {
                    Outcome::Changed => truncate(&excerpt, 60),
                    Outcome::Absent => "（目标元素本就不存在，文档未变）".to_string(),
                };
                summaries.push(AppliedOp {
                    op: "set_style_element",
                    block: 0,
                    before: truncate(&style_xml, 60),
                    after,
                    style: None,
                    style_unchanged: None,
                    target: Some(format!(
                        "style '{}' {}/{}",
                        display,
                        container.tag(),
                        element
                    )),
                });
                xml.replace_range(hit.start..hit.end, &new_style);
            }
        }
    }

    // 产物闸：合法 XML + w:style 计数守恒（只增 create 的份数）
    xml_dom::parse(&xml)?;
    let new_count = root_children(&xml)
        .ok_or_else(malformed_internal)?
        .iter()
        .filter(|c| c.name == "w:style")
        .count();
    if new_count != orig_count + created {
        return Err(AppError::Internal(format!(
            "XML 形态异常（内部 bug，未写盘）: w:style 计数不守恒（期望 {}，实得 {}）。",
            orig_count + created,
            new_count
        )));
    }
    Ok((repack_part(bytes, "word/styles.xml", &xml)?, summaries))
}

/// numbering.xml 定义手术：numId 经 w:num 解析到 abstractNum 的 w:lvl；
/// lvlOverride 不开刀；共享同一 abstract 的 numId 披露进摘要。
pub fn apply_numbering_edits_to_bytes(
    bytes: &[u8],
    ops: &[NumberingEditOp],
) -> AppResult<(Vec<u8>, Vec<AppliedOp>)> {
    if ops.is_empty() {
        return Err(AppError::Validation(
            "操作列表为空: numbering 批至少需要一个操作（set_numbering_element）。".into(),
        ));
    }
    let Some(mut xml) = read_entry(bytes, "word/numbering.xml")? else {
        return Err(AppError::Validation(
            "无编号部件: 本文档没有 word/numbering.xml（还没有任何自动编号列表）。\
             请先在文档里建一个列表（如对段落 set_ppr_element 插入 numPr 后由 Word\
             补全定义），再回来调级。"
                .into(),
        ));
    };
    let orig_lvls = count_lvls(&xml).ok_or_else(malformed_internal)?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut summaries: Vec<AppliedOp> = Vec::new();

    for NumberingEditOp::SetNumberingElement { num_id, level, element, xml: frag } in ops {
        if *num_id == 0 {
            return Err(AppError::Validation(
                "编号引用或级别不存在: numId 0 是 Word 的「显式无编号」标记，不是列表。\
                 请用 inspect_docx projection=numbering 查可用 numId。"
                    .into(),
            ));
        }
        if *level > 8 {
            return Err(AppError::Validation(format!(
                "编号引用或级别不存在: level {level} 超界（ilvl 合法 0-8）。"
            )));
        }
        if !LVL_ELEMENTS.contains(&element.as_str()) {
            return Err(AppError::Validation(format!(
                "非法子元素: w:lvl 内没有 <w:{element}> 子元素。合法元素（schema 序）: {}。\
                 原文形态用 inspect_docx projection=numbering 查看，从看到的原文复制修改。",
                LVL_ELEMENTS.join(" ")
            )));
        }
        if let Some(f) = frag {
            if let Some(m) = change_marker(f) {
                return Err(AppError::Validation(format!(
                    "受保护: 片段含修订记录元素 <w:{m}>。请去掉修订标记——修订历史与\
                     手术改动会互相踩踏。"
                )));
            }
            validate_fragment(element, f, "numbering")?;
        }
        let key = format!("{num_id}\u{1}{level}\u{1}{element}");
        if !seen.insert(key) {
            return Err(AppError::Validation(format!(
                "重复操作: 同批对 numId {num_id} level {level} 的 {element} 已有操作。\
                 请合并为一次操作，或拆成两批。"
            )));
        }
        // numId → abstractNumId → w:lvl（基于当前累积态解析）
        let catalog = parse_numbering(&xml_dom::parse(&xml)?);
        let Some(abs_id) = catalog.abstract_of(*num_id) else {
            let ids: Vec<String> = catalog
                .num_entries()
                .iter()
                .map(|(n, _)| n.to_string())
                .collect();
            return Err(AppError::Validation(format!(
                "编号引用或级别不存在: numId {num_id} 不在编号目录。已有 numId: {}。\
                 完整清单用 inspect_docx projection=numbering 查看。",
                if ids.is_empty() { "（空）".to_string() } else { ids.join("、") }
            )));
        };
        if catalog.lvl_of(*num_id, *level).is_none() {
            let lvls: Vec<String> =
                catalog.ilvls_of_num(*num_id).iter().map(|l| l.to_string()).collect();
            return Err(AppError::Validation(format!(
                "编号引用或级别不存在: numId {num_id} 没有 level {level} 定义（已有级: {}）。",
                if lvls.is_empty() { "（空）".to_string() } else { lvls.join("、") }
            )));
        }
        // 结构寻位：abstractNum → 其直接子级 w:lvl（ilvl 匹配）
        let abs_id_str = abs_id.to_string();
        let abs_hit = root_children(&xml)
            .ok_or_else(malformed_internal)?
            .into_iter()
            .find(|c| {
                c.name == "w:abstractNum"
                    && child_attr(&xml, c, "w:abstractNumId").as_deref() == Some(abs_id_str.as_str())
            })
            .ok_or_else(malformed_internal)?;
        let abs_xml = xml[abs_hit.start..abs_hit.end].to_string();
        let level_str = level.to_string();
        let lvl_hit = root_children(&abs_xml)
            .ok_or_else(malformed_internal)?
            .into_iter()
            .find(|c| {
                c.name == "w:lvl"
                    && child_attr(&abs_xml, c, "w:ilvl").as_deref() == Some(level_str.as_str())
            })
            .ok_or_else(malformed_internal)?;
        let lvl_xml = abs_xml[lvl_hit.start..lvl_hit.end].to_string();
        if let Some(m) = change_marker(&lvl_xml) {
            return Err(AppError::Validation(format!(
                "受保护: numId {num_id} level {level} 的定义带修订记录（w:{m}）。请先在 \
                 Word 里接受/拒绝修订再编辑定义。"
            )));
        }
        // w:lvl 摘空不清理（级别本体保留——空级由 Word 忽略，删级 = 改结构，不做）
        let (new_lvl, changed) =
            match set_child_element(&lvl_xml, element, frag.as_deref(), &LVL_ELEMENTS) {
                Some((s, c, _)) => (s, c),
                None => return Err(malformed_internal()),
            };
        let new_abs = format!(
            "{}{new_lvl}{}",
            &abs_xml[..lvl_hit.start],
            &abs_xml[lvl_hit.end..]
        );
        let shared = catalog.num_ids_of_abstract(abs_id);
        let share_note = if shared.len() > 1 {
            let ids: Vec<String> = shared.iter().map(|n| n.to_string()).collect();
            format!("（影响 numId {}，共享 abstractNum {}）", ids.join("、"), abs_id)
        } else {
            String::new()
        };
        let after = if changed {
            // 摘要聚焦变更元素本身（lvl 头部属性不占 60 字预算）
            let tag = format!("<w:{element}");
            match new_lvl.find(&tag) {
                Some(at) => truncate(&new_lvl[at..], 60),
                None => truncate(&new_lvl, 60),
            }
        } else {
            "（目标元素本就不存在，文档未变）".to_string()
        };
        summaries.push(AppliedOp {
            op: "set_numbering_element",
            block: 0,
            before: truncate(&lvl_xml, 60),
            after,
            style: None,
            style_unchanged: None,
            target: Some(format!("numId {num_id} lvl {level} {element}{share_note}")),
        });
        xml.replace_range(abs_hit.start..abs_hit.end, &new_abs);
    }

    // 产物闸：合法 XML + w:lvl 计数守恒
    xml_dom::parse(&xml)?;
    if count_lvls(&xml) != Some(orig_lvls) {
        return Err(AppError::Internal(
            "XML 形态异常（内部 bug，未写盘）: w:lvl 计数不守恒。".into(),
        ));
    }
    Ok((repack_part(bytes, "word/numbering.xml", &xml)?, summaries))
}

// =========================================================================
// 单元测试（全部走真入口 apply_*_edits_to_bytes + zip 夹具——含 read_entry/
// 守恒闸/repack_part 完整路径；zip 级 untouched 不变式在此覆盖）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // 夹具
    // ------------------------------------------------------------------

    fn styles_xml(body: &str) -> String {
        format!(
            "<w:styles xmlns:w=\"w\"><w:docDefaults><w:rPrDefault><w:rPr/></w:rPrDefault></w:docDefaults>{body}</w:styles>"
        )
    }

    const H1: &str = "<w:style w:type=\"paragraph\" w:styleId=\"2\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"1\"/><w:pPr><w:spacing w:line=\"360\"/><w:outlineLvl w:val=\"0\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"32\"/></w:rPr></w:style>";

    fn one_style_doc() -> String {
        styles_xml(&format!(
            "<w:latentStyles w:count=\"1\"/><w:style w:type=\"paragraph\" w:styleId=\"1\"><w:name w:val=\"Normal\"/></w:style>{H1}"
        ))
    }

    fn numbering_doc() -> String {
        let lvl = |ilvl: usize, fmt: &str| {
            let ord = ilvl + 1; // Word 惯例：ilvl 0 的 lvlText 是 %1.
            format!(
                "<w:lvl w:ilvl=\"{ilvl}\" w:tentative=\"1\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"%{ord}.\"/><w:lvlJc w:val=\"left\"/></w:lvl>"
            )
        };
        format!(
            "<w:numbering xmlns:w=\"w\">\
             <w:abstractNum w:abstractNumId=\"7\">{}{}</w:abstractNum>\
             <w:abstractNum w:abstractNumId=\"8\"><w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"bullet\"/></w:lvl></w:abstractNum>\
             <w:num w:numId=\"21\"><w:abstractNumId w:val=\"7\"/></w:num>\
             <w:num w:numId=\"33\"><w:abstractNumId w:val=\"7\"/></w:num>\
             <w:num w:numId=\"40\"><w:abstractNumId w:val=\"8\"/></w:num>\
             <w:num w:numId=\"0\"><w:abstractNumId w:val=\"9\"/></w:num>\
             </w:numbering>",
            lvl(0, "decimal"),
            lvl(1, "lowerLetter"),
        )
    }

    /// 手造最小 docx（document + 可选 styles/numbering；docx-rs 不可控定义部件）。
    fn zip_with(styles: Option<&str>, numbering: Option<&str>) -> Vec<u8> {
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            let mut parts: Vec<(&str, String)> = vec![
                ("[Content_Types].xml", "<Types/>".to_string()),
                ("word/document.xml", "<w:document/>".to_string()),
            ];
            if let Some(s) = styles {
                parts.push(("word/styles.xml", s.to_string()));
            }
            if let Some(n) = numbering {
                parts.push(("word/numbering.xml", n.to_string()));
            }
            for (name, data) in parts {
                w.start_file(name, opts).unwrap();
                w.write_all(data.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    fn part_of(bytes: &[u8], name: &str) -> String {
        read_entry(bytes, name).unwrap().unwrap_or_default()
    }

    fn run_styles(before: &str, ops: &[StyleEditOp]) -> (Vec<u8>, String, Vec<AppliedOp>) {
        let (out, sums) = apply_style_edits_to_bytes(&zip_with(Some(before), None), ops)
            .expect("styles 手术成功");
        let styles = part_of(&out, "word/styles.xml");
        (out, styles, sums)
    }

    fn styles_err(before: &str, ops: &[StyleEditOp]) -> String {
        apply_style_edits_to_bytes(&zip_with(Some(before), None), ops)
            .unwrap_err()
            .to_string()
    }

    fn run_numbering(before: &str, ops: &[NumberingEditOp]) -> (Vec<u8>, String, Vec<AppliedOp>) {
        let (out, sums) = apply_numbering_edits_to_bytes(&zip_with(None, Some(before)), ops)
            .expect("numbering 手术成功");
        let numbering = part_of(&out, "word/numbering.xml");
        (out, numbering, sums)
    }

    fn numbering_err(before: &str, ops: &[NumberingEditOp]) -> String {
        apply_numbering_edits_to_bytes(&zip_with(None, Some(before)), ops)
            .unwrap_err()
            .to_string()
    }

    fn extract_style(xml: &str, id: &str) -> String {
        root_children(xml)
            .unwrap()
            .into_iter()
            .find(|c| c.name == "w:style" && child_attr(xml, c, "w:styleId").as_deref() == Some(id))
            .map(|c| xml[c.start..c.end].to_string())
            .unwrap()
    }

    fn extract_abstract(xml: &str, id: &str) -> String {
        root_children(xml)
            .unwrap()
            .into_iter()
            .find(|c| {
                c.name == "w:abstractNum"
                    && child_attr(xml, c, "w:abstractNumId").as_deref() == Some(id)
            })
            .map(|c| xml[c.start..c.end].to_string())
            .unwrap()
    }

    /// abstractNum 内按 ilvl 取 w:lvl 子串（断言收敛到目标级别——兄弟级别
    /// 的同名元素不背锅）。
    fn extract_lvl(xml: &str, abs_id: &str, ilvl: &str) -> String {
        let abs = extract_abstract(xml, abs_id);
        root_children(&abs)
            .unwrap()
            .into_iter()
            .find(|c| c.name == "w:lvl" && child_attr(&abs, c, "w:ilvl").as_deref() == Some(ilvl))
            .map(|c| abs[c.start..c.end].to_string())
            .unwrap()
    }

    // ------------------------------------------------------------------
    // 结构寻位原语
    // ------------------------------------------------------------------

    #[test]
    fn root_children_full_depth_not_fooled_by_nested() {
        // tblStylePr 内嵌 pPr/rPr——直接子级清单不得把它算进去
        let xml = styles_xml(&format!(
            "{H1}<w:style w:type=\"table\" w:styleId=\"t1\"><w:name w:val=\"tb\"/>\
             <w:tblStylePr w:type=\"firstRow\"><w:pPr><w:spacing w:line=\"240\"/></w:pPr>\
             <w:rPr><w:b/></w:rPr></w:tblStylePr></w:style>"
        ));
        let children = root_children(&xml).unwrap();
        let styles: Vec<&Child> = children.iter().filter(|c| c.name == "w:style").collect();
        assert_eq!(styles.len(), 2);
        let t1 = styles[1];
        let t1_xml = &xml[t1.start..t1.end];
        let subs = root_children(t1_xml).unwrap();
        assert!(subs.iter().any(|c| c.name == "w:tblStylePr"));
        assert!(!subs.iter().any(|c| c.name == "w:pPr"), "嵌套 pPr 不入直接子级");
    }

    #[test]
    fn root_children_rejects_malformed() {
        assert!(root_children("<w:styles><w:style>").is_none(), "未闭合");
        assert!(root_children("<w:styles/>").is_some(), "自闭合根合法（无子级）");
    }

    // ------------------------------------------------------------------
    // set_style_element
    // ------------------------------------------------------------------

    #[test]
    fn set_style_element_replaces_and_inserts_by_schema_order() {
        let before = one_style_doc();
        // 替换既有 rPr/sz
        let (_, after, sums) = run_styles(
            &before,
            &[StyleEditOp::SetStyleElement {
                style: "heading 1".into(),
                container: StyleContainer::RPr,
                element: "sz".into(),
                xml: Some("<w:sz w:val=\"44\"/>".into()),
            }],
        );
        assert!(after.contains("<w:sz w:val=\"44\"/>"));
        assert!(!after.contains("w:val=\"32\""));
        assert_eq!(sums[0].target.as_deref(), Some("style 'heading 1' rPr/sz"));

        // 插入既有容器的新元素：color 落在 b 后 sz 前（RPR 序 b(3) < color(19) < sz(24)）
        let (_, after, _) = run_styles(
            &before,
            &[StyleEditOp::SetStyleElement {
                style: "2".into(),
                container: StyleContainer::RPr,
                element: "color".into(),
                xml: Some("<w:color w:val=\"1E4976\"/>".into()),
            }],
        );
        let h1 = extract_style(&after, "2");
        let (b_at, color_at, sz_at) = (
            h1.find("<w:b/>").unwrap(),
            h1.find("<w:color").unwrap(),
            h1.find("<w:sz").unwrap(),
        );
        assert!(b_at < color_at && color_at < sz_at, "color 按 schema 序落位");
    }

    #[test]
    fn set_style_element_inserts_into_ppr_by_schema_order() {
        let (_, after, _) = run_styles(
            &one_style_doc(),
            &[StyleEditOp::SetStyleElement {
                style: "heading 1".into(),
                container: StyleContainer::PPr,
                element: "jc".into(),
                xml: Some("<w:jc w:val=\"center\"/>".into()),
            }],
        );
        let h1 = extract_style(&after, "2");
        let ppr = &h1[h1.find("<w:pPr>").unwrap()..h1.find("</w:pPr>").unwrap()];
        // PPR 序 spacing(22) < jc(26) < outlineLvl(31)：jc 须落两者之间
        let (sp_at, jc_at, ol_at) = (
            ppr.find("<w:spacing").unwrap(),
            ppr.find("<w:jc").unwrap(),
            ppr.find("<w:outlineLvl").unwrap(),
        );
        assert!(sp_at < jc_at && jc_at < ol_at, "jc 按 schema 序落 spacing 后 outlineLvl 前");
    }

    #[test]
    fn set_style_element_removal_paths() {
        let before = one_style_doc();
        // 摘 spacing（pPr 还剩 outlineLvl → 容器保留）
        let (_, after, sums) = run_styles(
            &before,
            &[StyleEditOp::SetStyleElement {
                style: "heading 1".into(),
                container: StyleContainer::PPr,
                element: "spacing".into(),
                xml: None,
            }],
        );
        let h1 = extract_style(&after, "2");
        assert!(!h1.contains("<w:spacing"));
        assert!(h1.contains("<w:pPr><w:outlineLvl w:val=\"0\"/></w:pPr>"));
        assert!(sums[0].after.contains("outlineLvl"), "摘要含新态");

        // 摘空容器：spacing + outlineLvl 都摘 → pPr 整体清理
        let (_, after, _) = run_styles(
            &before,
            &[
                StyleEditOp::SetStyleElement {
                    style: "heading 1".into(),
                    container: StyleContainer::PPr,
                    element: "spacing".into(),
                    xml: None,
                },
                StyleEditOp::SetStyleElement {
                    style: "heading 1".into(),
                    container: StyleContainer::PPr,
                    element: "outlineLvl".into(),
                    xml: None,
                },
            ],
        );
        let h1 = extract_style(&after, "2");
        assert!(!h1.contains("<w:pPr"), "摘空 → pPr 整体清理");
        assert!(h1.contains("<w:rPr>"), "兄弟容器保留");

        // 摘不存在的元素 → 空转（逐字节不变 + 摘要显式标注防误读）
        let (_, after, sums) = run_styles(
            &before,
            &[StyleEditOp::SetStyleElement {
                style: "heading 1".into(),
                container: StyleContainer::PPr,
                element: "shd".into(),
                xml: None,
            }],
        );
        assert_eq!(after, before, "空转 = styles.xml 逐字节不变");
        assert!(
            sums[0].after.starts_with("（目标元素本就不存在"),
            "{}",
            sums[0].after
        );
    }

    #[test]
    fn set_style_element_container_lifecycle() {
        let before = one_style_doc();
        // Normal 无 pPr：新建容器 + 元素
        let (_, after, _) = run_styles(
            &before,
            &[StyleEditOp::SetStyleElement {
                style: "Normal".into(),
                container: StyleContainer::PPr,
                element: "spacing".into(),
                xml: Some("<w:spacing w:line=\"276\"/>".into()),
            }],
        );
        let normal = extract_style(&after, "1");
        assert!(normal.contains("<w:pPr><w:spacing w:line=\"276\"/></w:pPr>"));

        // 摘除时容器不存在 → 空转
        let (_, after, sums) = run_styles(
            &before,
            &[StyleEditOp::SetStyleElement {
                style: "Normal".into(),
                container: StyleContainer::RPr,
                element: "b".into(),
                xml: None,
            }],
        );
        assert_eq!(after, before);
        assert!(sums[0].after.starts_with("（目标元素本就不存在"));
    }

    #[test]
    fn set_style_element_style_level_and_name_guard() {
        let before = one_style_doc();
        // style 直接子级：替换 basedOn + 同批插 qFormat（缺）→ 落 pPr 前
        // （STYLE 序 qFormat(10) < pPr(17)）
        let (_, after, _) = run_styles(
            &before,
            &[
                StyleEditOp::SetStyleElement {
                    style: "heading 1".into(),
                    container: StyleContainer::Style,
                    element: "basedOn".into(),
                    xml: Some("<w:basedOn w:val=\"9\"/>".into()),
                },
                StyleEditOp::SetStyleElement {
                    style: "heading 1".into(),
                    container: StyleContainer::Style,
                    element: "qFormat".into(),
                    xml: Some("<w:qFormat/>".into()),
                },
            ],
        );
        assert!(after.contains("<w:basedOn w:val=\"9\"/>"));
        // style 档插 qFormat（缺）→ 落在 pPr 前（STYLE 序 qFormat(10) < pPr(17)）
        let h1 = extract_style(&after, "2");
        let (q_at, p_at) = (h1.find("<w:qFormat").unwrap(), h1.find("<w:pPr").unwrap());
        assert!(q_at < p_at);

        // name 拒摘除；改名 = 整元素替换（styleId 不动）
        let err = styles_err(
            &before,
            &[StyleEditOp::SetStyleElement {
                style: "heading 1".into(),
                container: StyleContainer::Style,
                element: "name".into(),
                xml: None,
            }],
        );
        assert!(err.contains("受保护"), "{err}");
        let (_, after, _) = run_styles(
            &before,
            &[StyleEditOp::SetStyleElement {
                style: "heading 1".into(),
                container: StyleContainer::Style,
                element: "name".into(),
                xml: Some("<w:name w:val=\"heading 1 x\"/>".into()),
            }],
        );
        assert!(after.contains("<w:name w:val=\"heading 1 x\"/>"));
        assert!(after.contains("w:styleId=\"2\""), "styleId 不动");
    }

    #[test]
    fn set_style_element_guards() {
        let doc = one_style_doc();
        // pPr 内 rPr 指路
        let err = styles_err(
            &doc,
            &[StyleEditOp::SetStyleElement {
                style: "heading 1".into(),
                container: StyleContainer::PPr,
                element: "rPr".into(),
                xml: None,
            }],
        );
        assert!(err.contains("受保护"), "{err}");
        // 非法子元素（列 schema 清单）
        let err = styles_err(
            &doc,
            &[StyleEditOp::SetStyleElement {
                style: "heading 1".into(),
                container: StyleContainer::RPr,
                element: "outlineLvl".into(),
                xml: None,
            }],
        );
        assert!(err.contains("非法子元素"), "{err}");
        // 重名显示名拒（列全部 ID）
        let dup = styles_xml(&format!(
            "{H1}<w:style w:type=\"paragraph\" w:styleId=\"9\"><w:name w:val=\"heading 1\"/></w:style>"
        ));
        let err = styles_err(
            &dup,
            &[StyleEditOp::SetStyleElement {
                style: "heading 1".into(),
                container: StyleContainer::RPr,
                element: "b".into(),
                xml: None,
            }],
        );
        assert!(err.contains("样式名重复"), "{err}");
        assert!(err.contains("2、9"), "列全部同名 ID: {err}");
        // 不存在挂候选
        let err = styles_err(
            &doc,
            &[StyleEditOp::SetStyleElement {
                style: "nope".into(),
                container: StyleContainer::RPr,
                element: "b".into(),
                xml: None,
            }],
        );
        assert!(err.contains("样式不存在"), "{err}");
        assert!(err.contains("heading 1"), "挂候选: {err}");
        // 批内去重（显示名与 ID 寻址同一目标也命中）
        let mk = |s: &str, val: &str| StyleEditOp::SetStyleElement {
            style: s.into(),
            container: StyleContainer::RPr,
            element: "sz".into(),
            xml: Some(format!("<w:sz w:val=\"{val}\"/>")),
        };
        let err = styles_err(&doc, &[mk("heading 1", "40"), mk("2", "41")]);
        assert!(err.contains("重复操作"), "{err}");
        // 空批 / 无部件
        let err = styles_err(&doc, &[]);
        assert!(err.contains("操作列表为空"), "{err}");
        let err = apply_style_edits_to_bytes(&zip_with(None, None), &[mk("x", "1")])
            .unwrap_err()
            .to_string();
        assert!(err.contains("无样式部件"), "{err}");
        // 片段根名不符（复用 D9 校验层）
        let err = styles_err(
            &doc,
            &[StyleEditOp::SetStyleElement {
                style: "heading 1".into(),
                container: StyleContainer::RPr,
                element: "sz".into(),
                xml: Some("<w:color w:val=\"111111\"/>".into()),
            }],
        );
        assert!(err.contains("根元素必须是"), "{err}");
    }

    #[test]
    fn revision_marker_subtree_rejected() {
        let doc = styles_xml(&format!(
            "{H1}<w:style w:type=\"paragraph\" w:styleId=\"7\"><w:name w:val=\"dirty\"/>\
             <w:pPr><w:spacing w:line=\"240\"/><w:pPrChange w:id=\"1\"><w:pPr/></w:pPrChange></w:pPr></w:style>"
        ));
        let err = styles_err(
            &doc,
            &[StyleEditOp::SetStyleElement {
                style: "dirty".into(),
                container: StyleContainer::PPr,
                element: "jc".into(),
                xml: Some("<w:jc w:val=\"left\"/>".into()),
            }],
        );
        assert!(err.contains("受保护"), "{err}");
        assert!(err.contains("pPrChange"), "{err}");
        // 片段自带修订标记同样拒
        let err = styles_err(
            &one_style_doc(),
            &[StyleEditOp::SetStyleElement {
                style: "heading 1".into(),
                container: StyleContainer::RPr,
                element: "color".into(),
                xml: Some("<w:color w:val=\"1\"/><w:rPrChange/>".into()),
            }],
        );
        assert!(err.contains("受保护"), "{err}");
        assert!(err.contains("rPrChange"), "{err}");
    }

    #[test]
    fn create_style_birth_and_combo() {
        let before = one_style_doc();
        // 最小出生 + ID 派生（显示名去空白）
        let (_, after, sums) = run_styles(
            &before,
            &[StyleEditOp::CreateStyle {
                style_type: StyleType::Paragraph,
                name: "My Style".into(),
                style_id: None,
                based_on: None,
            }],
        );
        assert!(
            after.contains(
                "<w:style w:type=\"paragraph\" w:styleId=\"MyStyle\"><w:name w:val=\"My Style\"/><w:qFormat/></w:style>"
            ),
            "{after}"
        );
        assert_eq!(sums[0].target.as_deref(), Some("style 'My Style'（ID MyStyle）"));

        // 同批 create→set 组合（寻址放应用期）+ basedOn 解析到 ID + 显式 style_id
        let (_, after, _) = run_styles(
            &before,
            &[
                StyleEditOp::CreateStyle {
                    style_type: StyleType::Table,
                    name: "tb1".into(),
                    style_id: Some("tbl-x".into()),
                    based_on: Some("heading 1".into()),
                },
                StyleEditOp::SetStyleElement {
                    style: "tb1".into(),
                    container: StyleContainer::TblPr,
                    element: "tblBorders".into(),
                    xml: Some(
                        "<w:tblBorders><w:top w:val=\"single\" w:sz=\"4\"/></w:tblBorders>".into(),
                    ),
                },
            ],
        );
        assert!(after.contains("w:styleId=\"tbl-x\""));
        assert!(after.contains("<w:basedOn w:val=\"2\"/>"), "basedOn 解析到 ID 2");
        assert!(after.contains("<w:tblBorders>"));

        // 名/ID 双撞拒
        for dup in [
            StyleEditOp::CreateStyle {
                style_type: StyleType::Paragraph,
                name: "Normal".into(),
                style_id: Some("fresh".into()),
                based_on: None,
            },
            StyleEditOp::CreateStyle {
                style_type: StyleType::Paragraph,
                name: "brand-new".into(),
                style_id: Some("2".into()),
                based_on: None,
            },
        ] {
            let err = styles_err(&before, &[dup]);
            assert!(err.contains("样式已存在"), "{err}");
        }
        // based_on 不存在挂候选
        let err = styles_err(
            &before,
            &[StyleEditOp::CreateStyle {
                style_type: StyleType::Paragraph,
                name: "ok-name".into(),
                style_id: None,
                based_on: Some("ghost".into()),
            }],
        );
        assert!(err.contains("样式不存在"), "{err}");
    }

    #[test]
    fn self_closed_style_expands() {
        let before = styles_xml("<w:style w:type=\"paragraph\" w:styleId=\"5\" w:default=\"1\"/>");
        let (_, after, _) = run_styles(
            &before,
            &[StyleEditOp::SetStyleElement {
                style: "5".into(),
                container: StyleContainer::RPr,
                element: "sz".into(),
                xml: Some("<w:sz w:val=\"21\"/>".into()),
            }],
        );
        assert!(
            after.contains(
                "<w:style w:type=\"paragraph\" w:styleId=\"5\" w:default=\"1\"><w:rPr><w:sz w:val=\"21\"/></w:rPr></w:style>"
            ),
            "自闭合 style 展开成对标签再插容器: {after}"
        );
    }

    #[test]
    fn styles_surgery_leaves_other_entries_untouched() {
        // zip 级 untouched 不变式：styles 手术后 document.xml 内容不变 +
        // latentStyles 计数不变 + 非目标样式逐字节不变
        let before = one_style_doc();
        let (out, after, _) = run_styles(
            &before,
            &[
                StyleEditOp::SetStyleElement {
                    style: "heading 1".into(),
                    container: StyleContainer::RPr,
                    element: "sz".into(),
                    xml: Some("<w:sz w:val=\"44\"/>".into()),
                },
                StyleEditOp::CreateStyle {
                    style_type: StyleType::Paragraph,
                    name: "extra".into(),
                    style_id: None,
                    based_on: None,
                },
            ],
        );
        assert_eq!(part_of(&out, "word/document.xml"), "<w:document/>");
        assert_eq!(
            before.matches("w:latentStyles").count(),
            after.matches("w:latentStyles").count(),
            "latentStyles 永不碰"
        );
        assert!(after.contains(
            "<w:style w:type=\"paragraph\" w:styleId=\"1\"><w:name w:val=\"Normal\"/></w:style>"
        ));
    }

    // ------------------------------------------------------------------
    // numbering
    // ------------------------------------------------------------------

    #[test]
    fn numbering_lvl_surgery_and_shared_disclosure() {
        let before = numbering_doc();
        let (out, after, sums) = run_numbering(
            &before,
            &[NumberingEditOp::SetNumberingElement {
                num_id: 21,
                level: 0,
                element: "lvlText".into(),
                xml: Some("<w:lvlText w:val=\"%1、\"/>".into()),
            }],
        );
        assert!(after.contains("<w:lvlText w:val=\"%1、\"/>"));
        assert!(!after.contains("w:val=\"%1.\""));
        // 共享披露：numId 21/33 同挂 abstract 7
        assert_eq!(
            sums[0].target.as_deref(),
            Some("numId 21 lvl 0 lvlText（影响 numId 21、33，共享 abstractNum 7）")
        );
        // 非目标 abstractNum 8 逐字节不变 + document.xml 不变
        assert_eq!(extract_abstract(&after, "8"), extract_abstract(&before, "8"));
        assert_eq!(part_of(&out, "word/document.xml"), "<w:document/>");
    }

    #[test]
    fn numbering_schema_order_insert() {
        // 插 pStyle（LVL 序 numFmt(2) < pStyle(3) < lvlText(6)）：落 numFmt 之后 lvlText 前
        let (_, after, _) = run_numbering(
            &numbering_doc(),
            &[NumberingEditOp::SetNumberingElement {
                num_id: 33,
                level: 1,
                element: "pStyle".into(),
                xml: Some("<w:pStyle w:val=\"2\"/>".into()),
            }],
        );
        let abs = extract_abstract(&after, "7");
        let lvl1_at = abs.find("w:ilvl=\"1\"").unwrap();
        let lvl1 = &abs[lvl1_at - "<w:lvl ".len()..];
        let lvl1 = &lvl1[..lvl1.find("</w:lvl>").unwrap()];
        let (fmt_at, ps_at, text_at) = (
            lvl1.find("<w:numFmt").unwrap(),
            lvl1.find("<w:pStyle").unwrap(),
            lvl1.find("<w:lvlText").unwrap(),
        );
        assert!(fmt_at < ps_at && ps_at < text_at, "pStyle 按 schema 序落位");
    }

    #[test]
    fn numbering_removal_noop_and_level_kept() {
        let before = numbering_doc();
        // 摘不存在的 suff → 空转
        let (_, after, sums) = run_numbering(
            &before,
            &[NumberingEditOp::SetNumberingElement {
                num_id: 21,
                level: 0,
                element: "suff".into(),
                xml: None,
            }],
        );
        assert_eq!(after, before, "空转 = numbering.xml 逐字节不变");
        assert!(sums[0].after.starts_with("（目标元素本就不存在"));
        // 摘 lvlJc（w:lvl 还有子级 → 级本体保留；兄弟级别 lvl 1 的 lvlJc 不动）
        let (_, after, _) = run_numbering(
            &before,
            &[NumberingEditOp::SetNumberingElement {
                num_id: 21,
                level: 0,
                element: "lvlJc".into(),
                xml: None,
            }],
        );
        let lvl0 = extract_lvl(&after, "7", "0");
        let lvl1 = extract_lvl(&after, "7", "1");
        assert!(!lvl0.contains("lvlJc"), "目标级别的 lvlJc 已摘: {lvl0}");
        assert!(lvl1.contains("lvlJc"), "兄弟级别不动: {lvl1}");
        assert!(after.contains("w:abstractNumId=\"7\""), "级别本体保留");
    }

    #[test]
    fn numbering_rejects_bad_refs() {
        let doc = numbering_doc();
        let mk = |num_id: u32, level: u32, element: &str| NumberingEditOp::SetNumberingElement {
            num_id,
            level,
            element: element.into(),
            xml: None,
        };
        // numId 0
        let err = numbering_err(&doc, &[mk(0, 0, "start")]);
        assert!(err.contains("编号引用或级别不存在"), "{err}");
        // 未知 numId 列已有
        let err = numbering_err(&doc, &[mk(99, 0, "start")]);
        assert!(err.contains("编号引用或级别不存在"), "{err}");
        assert!(err.contains("21、33、40"), "列已有 numId: {err}");
        // 越界 level
        let err = numbering_err(&doc, &[mk(21, 9, "start")]);
        assert!(err.contains("编号引用或级别不存在"), "{err}");
        // abstract 8 只有 lvl 0：numId 40 level 1 报已有级
        let err = numbering_err(&doc, &[mk(40, 1, "start")]);
        assert!(err.contains("编号引用或级别不存在"), "{err}");
        assert!(err.contains("已有级: 0"), "{err}");
        // 非法子元素
        let err = numbering_err(&doc, &[mk(21, 0, "tblHeader")]);
        assert!(err.contains("非法子元素"), "{err}");
        // 空批 / 无部件
        let err = numbering_err(&doc, &[]);
        assert!(err.contains("操作列表为空"), "{err}");
        let err = apply_numbering_edits_to_bytes(&zip_with(None, None), &[mk(21, 0, "start")])
            .unwrap_err()
            .to_string();
        assert!(err.contains("无编号部件"), "{err}");
    }

    #[test]
    fn numbering_revision_marker_rejected() {
        let doc = String::from(
            "<w:numbering xmlns:w=\"w\"><w:abstractNum w:abstractNumId=\"7\">\
             <w:lvl w:ilvl=\"0\"><w:numFmt w:val=\"decimal\"/><w:pPr><w:pPrChange/></w:pPr></w:lvl>\
             </w:abstractNum><w:num w:numId=\"21\"><w:abstractNumId w:val=\"7\"/></w:num></w:numbering>",
        );
        let err = numbering_err(
            &doc,
            &[NumberingEditOp::SetNumberingElement {
                num_id: 21,
                level: 0,
                element: "lvlText".into(),
                xml: Some("<w:lvlText w:val=\"%1.\"/>".into()),
            }],
        );
        assert!(err.contains("受保护"), "{err}");
        assert!(err.contains("pPrChange"), "{err}");
    }
}


