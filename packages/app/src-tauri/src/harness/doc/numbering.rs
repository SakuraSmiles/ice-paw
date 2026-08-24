//! `numbering` —— `word/numbering.xml` 解析 + 自动编号实际值计算（S3 首波①）。
//!
//! **盲区治什么**：段落里的 `w:numPr`（numId+ilvl）只是引用，实际编号文本
//! （"3.2.1"、"一）、(a)"）是 Word 打开时按 numbering.xml 规则现场计算的。
//! 不接 numbering.xml，agent 就看不见列表/自动编号标题的真实编号值。
//!
//! 三件：
//! - [`parse_numbering`]：numbering.xml → [`NumberingCatalog`]（num→abstractNum
//!   映射 + 每级定义：numFmt / lvlText 模板 / start / pStyle 关联）
//! - [`compute_numbers`]：文档顺序计数模拟 → 顶层段落块号 → 编号文本
//!   （计数规则：遇 (numId, ilvl) 该级 +1，同 numId 更深层级清零——标准
//!   multilevel 行为；`lvlRestart` 覆盖属长尾，暂不支持）
//! - [`render_number`]：numFmt × lvlText 模板 → 编号文本（`%1..%9` 替换）
//!
//! **MVP 边界**（诚实记录）：只算段落**直接携带** numPr 的编号；样式关联
//! （标题样式经 abstractNum/lvl 的 pStyle 间接编号）解析并存储但**不参与计算**
//! ——等真实样本（语料标题均手打编号，无此形态）。表格内段落不参与计数
//! （表格内列表罕见且独立语义）。未知 numFmt 回退 decimal。

use std::collections::HashMap;

use super::docx_model::Block;
use super::xml_dom::Element;

/// 单个编号级别定义（abstractNum 内一个 w:lvl）。
#[derive(Debug, Clone)]
pub(super) struct LvlDef {
    /// w:numFmt val：decimal / bullet / lowerLetter / …（原样字符串，渲染时分发）
    pub num_fmt: String,
    /// w:lvlText val：如 "%1.%2" / "%1、" / "•"（%1..%9 为各级计数占位）
    pub lvl_text: String,
    /// w:start val（该级起始计数，通常 1）
    pub start: u32,
    /// w:pStyle val（样式关联编号：该级服务于哪个样式；MVP 只记录不参与计算，
    /// 等真实样本出现再接线——字段本身是解析产物，保留供下一波使用）
    #[allow(dead_code)] // 测试读它，非测试构建下无人读（解析产物先行保留）
    pub pstyle: Option<String>,
}

/// 一个 abstractNum（最多 9 级，ilvl 0-8）。
#[derive(Debug, Clone, Default)]
pub(super) struct AbstractNum {
    /// ilvl → 定义
    pub lvls: HashMap<u32, LvlDef>,
}

/// 整张编号目录。
#[derive(Debug, Clone, Default)]
pub(super) struct NumberingCatalog {
    /// numId → abstractNumId（w:num 实例化映射）
    nums: HashMap<u32, u32>,
    /// abstractNumId → 定义
    abstracts: HashMap<u32, AbstractNum>,
}

impl NumberingCatalog {
    pub(super) fn empty() -> Self {
        Self::default()
    }

    /// numId + ilvl → 级定义（num 缺失 / abstractNum 缺失 / 级缺失 → None，
    /// 调用方回退引用形式显示）。
    fn lvl_of(&self, num_id: u32, ilvl: u32) -> Option<&LvlDef> {
        let abs_id = self.nums.get(&num_id)?;
        self.abstracts.get(abs_id)?.lvls.get(&ilvl)
    }
}

/// 解析 numbering.xml 根元素（`w:numbering`；宽容遍历，同 docx_model 策略）。
pub(super) fn parse_numbering(root: &Element) -> NumberingCatalog {
    let mut cat = NumberingCatalog::empty();
    for el in root.child_elements() {
        match el.name.as_str() {
            "w:abstractNum" => {
                let id = el.attr("w:abstractNumId").and_then(|v| v.parse().ok());
                let mut abs = AbstractNum::default();
                for lvl in el.child_elements().filter(|e| e.name == "w:lvl") {
                    let ilvl = lvl.attr("w:ilvl").and_then(|v| v.parse().ok());
                    let Some(ilvl) = ilvl else { continue };
                    let def = LvlDef {
                        num_fmt: lvl
                            .child_elements()
                            .find(|e| e.name == "w:numFmt")
                            .and_then(|e| e.attr("w:val"))
                            .unwrap_or("decimal")
                            .to_string(),
                        lvl_text: lvl
                            .child_elements()
                            .find(|e| e.name == "w:lvlText")
                            .and_then(|e| e.attr("w:val"))
                            .unwrap_or("")
                            .to_string(),
                        start: lvl
                            .child_elements()
                            .find(|e| e.name == "w:start")
                            .and_then(|e| e.attr("w:val"))
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(1),
                        pstyle: lvl
                            .child_elements()
                            .find(|e| e.name == "w:pStyle")
                            .and_then(|e| e.attr("w:val"))
                            .map(str::to_string),
                    };
                    abs.lvls.insert(ilvl, def);
                }
                if let Some(id) = id {
                    cat.abstracts.insert(id, abs);
                }
            }
            "w:num" => {
                let num_id = el.attr("w:numId").and_then(|v| v.parse().ok());
                let abs_id = el
                    .child_elements()
                    .find(|e| e.name == "w:abstractNumId")
                    .and_then(|e| e.attr("w:val"))
                    .and_then(|v| v.parse().ok());
                if let (Some(num_id), Some(abs_id)) = (num_id, abs_id) {
                    if num_id != 0 {
                        cat.nums.insert(num_id, abs_id); // numId=0 是「显式无编号」
                    }
                }
            }
            _ => {}
        }
    }
    cat
}

/// 文档顺序计数模拟：顶层段落块号（1-based）→ 编号文本。
///
/// 只处理段落**直接** numPr；表格块整体跳过（不进计数序）。同一 numId 的更深
/// ilvl 出现即清零（标准多级列表行为：子级随父级推进重新从 start 计）。
pub(super) fn compute_numbers(
    body: &[Block],
    catalog: &NumberingCatalog,
) -> HashMap<usize, String> {
    // (numId, ilvl) → 当前计数值
    let mut counters: HashMap<(u32, u32), u32> = HashMap::new();
    let mut out: HashMap<usize, String> = HashMap::new();
    for (i, block) in body.iter().enumerate() {
        let Block::Paragraph(p) = block else { continue };
        let Some(num) = &p.props.numbering else { continue };
        let Some(def) = catalog.lvl_of(num.num_id, num.ilvl) else { continue };

        // 本级 +1（首个出现从 start 起算：先重置为 start-1 再自增）
        let entry = counters
            .entry((num.num_id, num.ilvl))
            .or_insert(def.start.saturating_sub(1));
        *entry += 1;
        // 同 numId 更深层级清零（子级随父级重计）
        let deeper: Vec<(u32, u32)> = counters
            .keys()
            .filter(|(nid, lv)| *nid == num.num_id && *lv > num.ilvl)
            .copied()
            .collect();
        for k in deeper {
            if let Some(d) = catalog.lvl_of(k.0, k.1) {
                counters.insert(k, d.start.saturating_sub(1));
            }
        }

        // lvlText 模板渲染：%1..%9 → 对应级计数值经 numFmt 格式化
        let text = sanitize_bullet_glyph(&render_lvl_text(def, num.num_id, &counters));
        if !text.trim().is_empty() {
            out.insert(i + 1, text);
        }
    }
    out
}

/// bullet 列表的 lvlText 常是 Wingdings 私有区符号（U+F0xx，对 LLM 是乱码/空白）。
/// 私有区字符统一替换为 •（语义：无序符号列表，具体符号形状无信息量）。
fn sanitize_bullet_glyph(s: &str) -> String {
    if !s.chars().any(|c| ('\u{E000}'..='\u{F8FF}').contains(&c)) {
        return s.to_string();
    }
    s.chars()
        .map(|c| if ('\u{E000}'..='\u{F8FF}').contains(&c) { '•' } else { c })
        .collect()
}

/// lvlText 模板渲染（`%1..%9` → 各级格式化值；bullet/none 无占位则原样）。
fn render_lvl_text(
    def: &LvlDef,
    num_id: u32,
    counters: &HashMap<(u32, u32), u32>,
) -> String {
    let mut out = String::with_capacity(def.lvl_text.len() + 8);
    let mut rest = def.lvl_text.as_str();
    while let Some(pct) = rest.find('%') {
        let after = &rest[pct + 1..];
        let next_char = after.chars().next();
        let is_placeholder = matches!(next_char, Some('1'..='9'));
        if !is_placeholder {
            // 非 %N 占位（如字面 %）原样
            out.push_str(&rest[..pct + 1]);
            rest = after;
            continue;
        }
        let ilvl = next_char.unwrap().to_digit(10).unwrap() - 1;
        out.push_str(&rest[..pct]);
        // 该级当前值：未出现过（模板引用了未出现层级——如 lvl2 段先于 lvl0 出现）按 1
        let value = counters.get(&(num_id, ilvl)).copied().unwrap_or(1);
        out.push_str(&render_number(&def.num_fmt, value));
        rest = &after[1..];
    }
    out.push_str(rest);
    out
}

/// numFmt 渲染分发。未知格式回退 decimal（保显示，不静默失败——值正确、形态
/// 可能与 Word 有别；常见集全覆盖，长尾格式（法律编号等）渐进）。
fn render_number(num_fmt: &str, value: u32) -> String {
    match num_fmt {
        "decimal" | "decimalFullWidth" => value.to_string(),
        "lowerLetter" => to_letters(value, false),
        "upperLetter" => to_letters(value, true),
        "lowerRoman" => to_roman(value, false),
        "upperRoman" => to_roman(value, true),
        "chineseCounting" | "chineseCountingThousand" | "chineseLegalSimplified"
        | "ideographDigital" | "japaneseCounting" | "japaneseDigitalTenThousand" => {
            to_chinese(value)
        }
        // bullet / none：lvlText 通常无 %N 占位，占位出现时给空（符号列表无序数值）
        "bullet" | "none" => String::new(),
        _ => value.to_string(),
    }
}

/// 1-based 字母序（a..z, aa..zz…）：Excel 列名同款算法。
fn to_letters(mut n: u32, upper: bool) -> String {
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'a' + (n % 26) as u8);
        n /= 26;
    }
    let s: String = out.iter().rev().map(|&b| b as char).collect();
    if upper { s.to_uppercase() } else { s }
}

/// 罗马数字（1..=3999 标准；超界回退阿拉伯）。
fn to_roman(n: u32, upper: bool) -> String {
    const PAIRS: [(u32, &str); 13] = [
        (1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"),
        (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i"),
    ];
    if n == 0 || n >= 4000 {
        return n.to_string();
    }
    let mut out = String::new();
    let mut rest = n;
    for (v, sym) in PAIRS {
        while rest >= v {
            out.push_str(sym);
            rest -= v;
        }
    }
    if upper { out.to_uppercase() } else { out }
}

/// 中文数字（chineseCountingThousand 进位式：二十一 / 一百零一 / 一千零一）。
/// 上限 9999（Word 编号实际不会超；超界回退阿拉伯）。
fn to_chinese(n: u32) -> String {
    const DIGITS: [char; 10] = ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
    // digits 顺序 = 千百十个 → 单位按位对应（个位无单位）
    const UNITS: [char; 4] = ['千', '百', '十', '\0'];
    if n == 0 || n > 9999 {
        return n.to_string();
    }
    // 逐位产段（段 = 数字+单位），零的规则：连续零压缩、尾零去除、
    // 「一十」在 10-19 时按习惯省「一」（十六 → 十六）
    let mut sections: Vec<String> = Vec::new();
    let mut digits = [0u32; 4];
    digits[0] = n / 1000;
    digits[1] = (n / 100) % 10;
    digits[2] = (n / 10) % 10;
    digits[3] = n % 10;
    let mut pending_zero = false;
    for (pos, &d) in digits.iter().enumerate() {
        let unit = UNITS[pos];
        if d == 0 {
            pending_zero = true;
            continue;
        }
        if pending_zero && !sections.is_empty() {
            sections.push('零'.to_string());
        }
        pending_zero = false;
        // 十位为 1 且是首个非零段 → 省略「一」（10-19：十一..十九）
        if !(pos == 2 && d == 1 && sections.is_empty()) {
            sections.push(DIGITS[d as usize].to_string());
        }
        if unit != '\0' {
            sections.push(unit.to_string());
        }
    }
    sections.concat()
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::super::docx_model;
    use super::super::xml_dom;
    use super::*;

    fn numbering_xml(body: &str) -> NumberingCatalog {
        let dom = xml_dom::parse(&format!(
            r#"<w:numbering xmlns:w="w">{body}</w:numbering>"#
        ))
        .unwrap();
        parse_numbering(&dom)
    }

    #[test]
    fn parses_num_map_and_lvl_defs() {
        let cat = numbering_xml(
            r#"<w:abstractNum w:abstractNumId="7">
                <w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="chineseCounting"/><w:lvlText w:val="%1、"/><w:pStyle w:val="appendix"/></w:lvl>
                <w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2)"/></w:lvl>
            </w:abstractNum>
            <w:num w:numId="21"><w:abstractNumId w:val="7"/></w:num>
            <w:num w:numId="0"><w:abstractNumId w:val="9"/></w:num>"#,
        );
        let lvl0 = cat.lvl_of(21, 0).unwrap();
        assert_eq!(lvl0.num_fmt, "chineseCounting");
        assert_eq!(lvl0.lvl_text, "%1、");
        assert_eq!(lvl0.pstyle.as_deref(), Some("appendix"));
        assert_eq!(cat.lvl_of(21, 1).unwrap().num_fmt, "lowerLetter");
        assert!(cat.lvl_of(0, 0).is_none(), "numId=0 不入映射");
        assert!(cat.lvl_of(99, 0).is_none());
    }

    fn body_with_nums(nums: &[(u32, u32)]) -> Vec<Block> {
        nums.iter()
            .map(|&(num_id, ilvl)| {
                let dom = xml_dom::parse(&format!(
                    r#"<w:document xmlns:w="w"><w:body><w:p><w:pPr><w:numPr>\
                       <w:ilvl w:val="{ilvl}"/><w:numId w:val="{num_id}"/></w:numPr></w:pPr>\
                       <w:r><w:t>项</w:t></w:r></w:p></w:body></w:document>"#
                ))
                .unwrap();
                docx_model::build_document(&dom).body.remove(0)
            })
            .collect()
    }

    fn catalog_two_level() -> NumberingCatalog {
        numbering_xml(
            r#"<w:abstractNum w:abstractNumId="1">
                <w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl>
                <w:lvl w:ilvl="1"><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2)"/></w:lvl>
            </w:abstractNum>
            <w:num w:numId="5"><w:abstractNumId w:val="1"/></w:num>"#,
        )
    }

    #[test]
    fn counters_advance_and_deeper_levels_reset() {
        // 序列：lvl0, lvl1, lvl1, lvl0, lvl1 → 1. a) b) 2. a)
        let body = body_with_nums(&[(5, 0), (5, 1), (5, 1), (5, 0), (5, 1)]);
        let nums = compute_numbers(&body, &catalog_two_level());
        assert_eq!(nums.get(&1), Some(&"1.".to_string()));
        assert_eq!(nums.get(&2), Some(&"a)".to_string()));
        assert_eq!(nums.get(&3), Some(&"b)".to_string()));
        assert_eq!(nums.get(&4), Some(&"2.".to_string()));
        assert_eq!(nums.get(&5), Some(&"a)".to_string()), "子级应随父级清零重计");
    }

    #[test]
    fn separate_numids_count_independently() {
        let cat = numbering_xml(
            r#"<w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum>
               <w:num w:numId="5"><w:abstractNumId w:val="1"/></w:num>
               <w:num w:numId="6"><w:abstractNumId w:val="1"/></w:num>"#,
        );
        let body = body_with_nums(&[(5, 0), (5, 0), (6, 0)]);
        let nums = compute_numbers(&body, &cat);
        assert_eq!(nums.get(&1), Some(&"1.".to_string()));
        assert_eq!(nums.get(&2), Some(&"2.".to_string()));
        // 同 abstractNum 不同 num 实例：计数独立（Word 复制列表语义）
        assert_eq!(nums.get(&3), Some(&"1.".to_string()));
    }

    #[test]
    fn unresolved_reference_yields_no_number() {
        // numId 不在目录 → 无编号值（投影回退引用形式）
        let body = body_with_nums(&[(999, 0)]);
        let nums = compute_numbers(&body, &catalog_two_level());
        assert!(nums.is_empty());
    }

    #[test]
    fn chinese_numbering_full_forms() {
        let cat = numbering_xml(
            r#"<w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="chineseCounting"/><w:lvlText w:val="%1、"/></w:lvl></w:abstractNum>
               <w:num w:numId="5"><w:abstractNumId w:val="1"/></w:num>"#,
        );
        let body = body_with_nums(&[(5, 0); 3]);
        let nums = compute_numbers(&body, &cat);
        assert_eq!(nums.get(&1), Some(&"一、".to_string()));
        assert_eq!(nums.get(&2), Some(&"二、".to_string()));
        assert_eq!(nums.get(&3), Some(&"三、".to_string()));
        // 大数形态（十/二十一/一百零一…）由 to_chinese 单测锁定
    }

    #[test]
    fn to_chinese_covers_thousand_forms() {
        assert_eq!(to_chinese(1), "一");
        assert_eq!(to_chinese(9), "九");
        assert_eq!(to_chinese(10), "十");
        assert_eq!(to_chinese(16), "十六");
        assert_eq!(to_chinese(20), "二十");
        assert_eq!(to_chinese(21), "二十一");
        assert_eq!(to_chinese(100), "一百");
        assert_eq!(to_chinese(101), "一百零一");
        assert_eq!(to_chinese(110), "一百一十");
        assert_eq!(to_chinese(111), "一百一十一");
        assert_eq!(to_chinese(200), "二百");
        assert_eq!(to_chinese(1000), "一千");
        assert_eq!(to_chinese(1001), "一千零一");
        assert_eq!(to_chinese(1010), "一千零一十");
        assert_eq!(to_chinese(9999), "九千九百九十九");
        assert_eq!(to_chinese(10000), "10000"); // 超界回退阿拉伯
    }

    #[test]
    fn letters_roman_and_unknown_fallback() {
        assert_eq!(to_letters(1, false), "a");
        assert_eq!(to_letters(26, false), "z");
        assert_eq!(to_letters(27, false), "aa");
        assert_eq!(to_letters(1, true), "A");
        assert_eq!(to_roman(4, false), "iv");
        assert_eq!(to_roman(9, true), "IX");
        assert_eq!(to_roman(1994, true), "MCMXCIV");
        assert_eq!(render_number("unknownFmt", 3), "3", "未知格式回退 decimal");
        assert_eq!(render_number("bullet", 1), "", "bullet 无序数值");
    }

    #[test]
    fn lvl_text_without_placeholder_passthrough() {
        // bullet 列表：lvlText 即符号本体（无 %N），原样输出
        let cat = numbering_xml(
            r#"<w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/></w:lvl></w:abstractNum>
               <w:num w:numId="5"><w:abstractNumId w:val="1"/></w:num>"#,
        );
        let body = body_with_nums(&[(5, 0), (5, 0)]);
        let nums = compute_numbers(&body, &cat);
        assert_eq!(nums.get(&1), Some(&"•".to_string()));
        assert_eq!(nums.get(&2), Some(&"•".to_string()));
    }
}
