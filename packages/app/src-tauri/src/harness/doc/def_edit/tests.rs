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
    let (out, sums) =
        apply_style_edits_to_bytes(&zip_with(Some(before), None), ops).expect("styles 手术成功");
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
    assert!(
        !subs.iter().any(|c| c.name == "w:pPr"),
        "嵌套 pPr 不入直接子级"
    );
}

#[test]
fn root_children_rejects_malformed() {
    assert!(root_children("<w:styles><w:style>").is_none(), "未闭合");
    assert!(
        root_children("<w:styles/>").is_some(),
        "自闭合根合法（无子级）"
    );
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
    assert!(
        b_at < color_at && color_at < sz_at,
        "color 按 schema 序落位"
    );
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
    assert!(
        sp_at < jc_at && jc_at < ol_at,
        "jc 按 schema 序落 spacing 后 outlineLvl 前"
    );
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
    assert_eq!(
        sums[0].target.as_deref(),
        Some("style 'My Style'（ID MyStyle）")
    );

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
    assert!(
        after.contains("<w:basedOn w:val=\"2\"/>"),
        "basedOn 解析到 ID 2"
    );
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
    assert_eq!(
        extract_abstract(&after, "8"),
        extract_abstract(&before, "8")
    );
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
