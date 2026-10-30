use super::super::docx;
    use super::*;
    use std::io::{Cursor, Read};

    fn wrap(body: &str) -> String {
        format!(r#"<w:document xmlns:w="w" xmlns:r="r"><w:body>{body}</w:body></w:document>"#)
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
            assert_eq!(
                piece_model.body.len(),
                1,
                "块 {} 子串应恰为单块: {piece}",
                i + 1
            );
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
        assert!(
            out.contains(r#"<w:pPr><w:pStyle w:val="body"/></w:pPr><w:r><w:rPr><w:i/></w:rPr>"#)
        );
    }

    #[test]
    fn insert_with_style_name_resolves_id() {
        let styles_xml = r#"<w:styles><w:style w:type="paragraph" w:styleId="h1"><w:name w:val="heading 1"/></w:style></w:styles>"#;
        let sheet =
            super::super::styles::parse_styles(&super::super::xml_dom::parse(styles_xml).unwrap());
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
        assert!(
            out.contains(r#"<w:pPr><w:pStyle w:val="h1"/></w:pPr>"#),
            "显示名反查 ID"
        );
        // 未知样式报错
        let err = val_msg(
            apply_edits(
                &xml,
                &sheet,
                &[EditOp::InsertParagraphAfter {
                    block: 1,
                    expect_prefix: "正文".into(),
                    text: "x".into(),
                    style: Some("不存在的样式".into()),
                }],
            )
            .unwrap_err(),
        );
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
            &[EditOp::DeleteBlock {
                block: 2,
                expect_prefix: "二".into(),
            }],
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
                EditOp::DeleteBlock {
                    block: 5,
                    expect_prefix: "第5段".into(),
                },
                EditOp::ReplaceText {
                    block: 3,
                    expect_prefix: "第3段".into(),
                    new_text: "改后".into(),
                },
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
        let err = val_msg(
            apply_edits(
                &xml,
                &styles,
                &[EditOp::ReplaceText {
                    block: 1,
                    expect_prefix: "别的内容".into(),
                    new_text: "x".into(),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("指纹不符"), "实际: {err}");
        assert!(err.contains("inspect_docx"));
    }

    #[test]
    fn revision_and_sectpr_blocks_rejected() {
        let styles = Stylesheet::empty();
        // 修订块
        let xml = wrap(r#"<w:p><w:ins><w:r><w:t>修订段</w:t></w:r></w:ins></w:p>"#);
        let err = val_msg(
            apply_edits(
                &xml,
                &styles,
                &[EditOp::DeleteBlock {
                    block: 1,
                    expect_prefix: "修订段".into(),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("含修订标记"), "实际: {err}");
        // 节末段
        let xml = wrap(
            r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="1"/></w:sectPr></w:pPr><w:r><w:t>节末段</w:t></w:r></w:p>"#,
        );
        let err = val_msg(
            apply_edits(
                &xml,
                &styles,
                &[EditOp::DeleteBlock {
                    block: 1,
                    expect_prefix: "节末段".into(),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("节属性保护"), "实际: {err}");
    }

    #[test]
    fn duplicate_block_and_bounds_rejected() {
        let xml = wrap(r#"<w:p><w:r><w:t>一</w:t></w:r></w:p>"#);
        let styles = Stylesheet::empty();
        let err = val_msg(
            apply_edits(
                &xml,
                &styles,
                &[
                    EditOp::ReplaceText {
                        block: 1,
                        expect_prefix: "一".into(),
                        new_text: "x".into(),
                    },
                    EditOp::DeleteBlock {
                        block: 1,
                        expect_prefix: "x".into(),
                    },
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
        let err = val_msg(
            apply_edits(
                &xml,
                &styles,
                &[EditOp::DeleteBlock {
                    block: 7,
                    expect_prefix: "".into(),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("块号越界"), "实际: {err}");
        assert!(err.contains("1-1"), "应带有效范围: {err}");
    }

    #[test]
    fn table_block_replace_rejected() {
        let xml =
            wrap(r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
        let styles = Stylesheet::empty();
        let err = val_msg(
            apply_edits(
                &xml,
                &styles,
                &[EditOp::ReplaceText {
                    block: 1,
                    expect_prefix: "表".into(),
                    new_text: "x".into(),
                }],
            )
            .unwrap_err(),
        );
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
            &[EditOp::SetStyle {
                block: 1,
                expect_prefix: "正文段".into(),
                style: "heading 2".into(),
            }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "set_style");
        assert_eq!(applied[0].style.as_deref(), Some("h2"));
        assert!(out.contains(r#"<w:p w14:paraId="X1">"#), "开标签属性保留");
        assert!(out.contains(r#"<w:pPr><w:pStyle w:val="h2"/><w:jc w:val="center"/></w:pPr>"#));
        assert!(
            out.contains(r#"<w:rPr><w:b/></w:rPr><w:t>正文段</w:t></w:r>"#),
            "run 原样"
        );
        // 模型侧：样式生效 + 文本不变
        let m = model_of(&out);
        let super::docx_model::Block::Paragraph(p) = &m.body[0] else {
            panic!()
        };
        assert_eq!(p.props.style.as_deref(), Some("h2"));
        let mut t = String::new();
        docx_model::blocks_text(&m.body, &mut t);
        assert_eq!(t, "正文段\n");
    }

    #[test]
    fn set_style_noop_reports_style_unchanged() {
        // 目标段 pStyle 已是 h2，再 set 成 heading 2（显示名解析到同一 ID）→ 空转，
        // AppliedOp 显式报 style_unchanged=true；换到不同样式则 None（字段省略）
        let xml = wrap(
            r#"<w:p><w:pPr><w:pStyle w:val="h2"/></w:pPr><w:r><w:t>已是标题段</w:t></w:r></w:p>"#,
        );
        let (_, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetStyle {
                block: 1,
                expect_prefix: "已是标题段".into(),
                style: "heading 2".into(),
            }],
        )
        .unwrap();
        assert_eq!(applied[0].style_unchanged, Some(true));
        // agent 读回的 JSON 必须携带该信号（skip_serializing_if 只对 None 生效）
        let json = serde_json::to_string(&applied[0]).unwrap();
        assert!(json.contains("\"style_unchanged\":true"), "实际: {json}");

        let (out, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetStyle {
                block: 1,
                expect_prefix: "已是标题段".into(),
                style: "Normal".into(),
            }],
        )
        .unwrap();
        assert_eq!(applied[0].style_unchanged, None);
        let json = serde_json::to_string(&applied[0]).unwrap();
        assert!(
            !json.contains("style_unchanged"),
            "非空转不应带字段: {json}"
        );
        assert!(
            out.contains(r#"<w:pStyle w:val="body"/>"#),
            "真换样式仍生效"
        );
    }

    // ---- S3 二波（D9）：set_ppr_element 通用 pPr 元素手术 ----

    #[test]
    fn ppr_remove_keeps_sibling_elements() {
        // numPr 摘除，pStyle/spacing 兄弟字节原样，编号引用从模型消失
        let xml = wrap(concat!(
            r#"<w:p><w:pPr><w:pStyle w:val="3"/><w:numPr><w:ilvl w:val="2"/><w:numId w:val="2"/></w:numPr>"#,
            r#"<w:spacing w:before="163"/></w:pPr><w:r><w:t>1.2.6 标题段</w:t></w:r></w:p>"#,
        ));
        let (out, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetPprElement {
                block: 1,
                expect_prefix: "1.2.6".into(),
                element: "numPr".into(),
                xml: None,
            }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "set_ppr_element");
        assert!(
            applied[0].after.starts_with("removed numPr"),
            "实际: {}",
            applied[0].after
        );
        assert!(!out.contains("<w:numPr"), "numPr 应消失");
        assert!(
            out.contains(r#"<w:pPr><w:pStyle w:val="3"/><w:spacing w:before="163"/></w:pPr>"#),
            "兄弟元素原样: {out}"
        );
        let m = model_of(&out);
        let Block::Paragraph(p) = &m.body[0] else {
            panic!()
        };
        assert!(p.props.numbering.is_none(), "模型侧编号引用应消失");
    }

    #[test]
    fn ppr_remove_last_child_drops_empty_ppr() {
        // numPr 是唯一子元素 → 摘空后 pPr 整体清理
        let xml = wrap(
            r#"<w:p><w:pPr><w:numPr><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:t>仅编号段</w:t></w:r></w:p>"#,
        );
        let (out, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetPprElement {
                block: 1,
                expect_prefix: "仅编号".into(),
                element: "numPr".into(),
                xml: None,
            }],
        )
        .unwrap();
        assert!(applied[0].after.starts_with("removed numPr"));
        assert!(!out.contains("<w:pPr"), "空 pPr 应整体清理: {out}");
        let m = model_of(&out);
        assert_eq!(m.body.len(), 1, "块数守恒");
    }

    #[test]
    fn ppr_remove_absent_reports_noop() {
        // 段落无 numPr → 空转：文档逐字节不变 + 摘要明示
        let xml =
            wrap(r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>普通段</w:t></w:r></w:p>"#);
        let (out, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetPprElement {
                block: 1,
                expect_prefix: "普通".into(),
                element: "numPr".into(),
                xml: None,
            }],
        )
        .unwrap();
        assert!(
            applied[0].after.contains("空转"),
            "实际: {}",
            applied[0].after
        );
        assert_eq!(out, xml, "空转输出应与输入逐字节一致");
    }

    #[test]
    fn ppr_upsert_inserts_at_schema_position() {
        // keepNext（schema 序早于 numPr）插在 pStyle 与 numPr 之间；
        // outlineLvl（序晚于 spacing、早于 rPr）插在 spacing 与 rPr 之间
        let mk = |text: &str| {
            format!(
                concat!(
                    r#"<w:p><w:pPr><w:pStyle w:val="3"/><w:numPr><w:numId w:val="2"/></w:numPr>"#,
                    r#"<w:spacing w:before="120"/><w:rPr><w:b/></w:rPr></w:pPr>"#,
                    r#"<w:r><w:t>{}</w:t></w:r></w:p>"#,
                ),
                text
            )
        };
        let xml = wrap(&(mk("位序A") + &mk("位序B")));
        let (out, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[
                EditOp::SetPprElement {
                    block: 1,
                    expect_prefix: "位序A".into(),
                    element: "keepNext".into(),
                    xml: Some("<w:keepNext/>".into()),
                },
                EditOp::SetPprElement {
                    block: 2,
                    expect_prefix: "位序B".into(),
                    element: "outlineLvl".into(),
                    xml: Some(r#"<w:outlineLvl w:val="0"/>"#.into()),
                },
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 2);
        assert!(
            out.contains(r#"<w:pStyle w:val="3"/><w:keepNext/><w:numPr>"#),
            "keepNext 应插在 numPr 前: {out}"
        );
        assert!(
            out.contains(r#"<w:spacing w:before="120"/><w:outlineLvl w:val="0"/><w:rPr>"#),
            "outlineLvl 应插在 spacing 后 rPr 前: {out}"
        );
    }

    #[test]
    fn ppr_upsert_replaces_existing_whole_element() {
        // 已存在 jc → 整元素替换（属性值 + 属性集都以片段为准）
        let xml =
            wrap(r#"<w:p><w:pPr><w:jc w:val="left"/></w:pPr><w:r><w:t>替换段</w:t></w:r></w:p>"#);
        let (out, _) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetPprElement {
                block: 1,
                expect_prefix: "替换".into(),
                element: "jc".into(),
                xml: Some(r#"<w:jc w:val="right"/>"#.into()),
            }],
        )
        .unwrap();
        assert!(
            out.contains(r#"<w:pPr><w:jc w:val="right"/></w:pPr>"#),
            "整元素替换: {out}"
        );
    }

    #[test]
    fn ppr_upsert_creates_ppr_in_bare_paragraph() {
        // 无 pPr 裸段 → 开标签后新建；自闭合空段 → 顺势展开成对标签
        let xml = wrap(concat!(
            r#"<w:p><w:r><w:t>裸段</w:t></w:r></w:p>"#,
            r#"<w:p w14:paraId="E"/>"#,
        ));
        let (out, _) = apply_edits(
            &xml,
            &heading_styles(),
            &[
                EditOp::SetPprElement {
                    block: 1,
                    expect_prefix: "裸段".into(),
                    element: "keepNext".into(),
                    xml: Some("<w:keepNext/>".into()),
                },
                EditOp::SetPprElement {
                    block: 2,
                    expect_prefix: "".into(),
                    element: "keepLines".into(),
                    xml: Some("<w:keepLines/>".into()),
                },
            ],
        )
        .unwrap();
        assert!(
            out.contains(r#"<w:p><w:pPr><w:keepNext/></w:pPr><w:r><w:t>裸段</w:t></w:r></w:p>"#)
        );
        assert!(
            out.contains(r#"<w:p w14:paraId="E"><w:pPr><w:keepLines/></w:pPr></w:p>"#),
            "自闭合段展开: {out}"
        );
    }

    #[test]
    fn ppr_rejects_illegal_protected_and_table() {
        let styles = heading_styles();
        let para = wrap(r#"<w:p><w:r><w:t>一段</w:t></w:r></w:p>"#);
        // 非法元素名（numId 是 numPr 的子元素，不是 pPr 子元素）
        let err = val_msg(
            apply_edits(
                &para,
                &styles,
                &[EditOp::SetPprElement {
                    block: 1,
                    expect_prefix: "一段".into(),
                    element: "numId".into(),
                    xml: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("非法pPr子元素"), "实际: {err}");
        // 受保护：sectPr / pPrChange
        for protected in ["sectPr", "pPrChange"] {
            let err = val_msg(
                apply_edits(
                    &para,
                    &styles,
                    &[EditOp::SetPprElement {
                        block: 1,
                        expect_prefix: "一段".into(),
                        element: protected.into(),
                        xml: None,
                    }],
                )
                .unwrap_err(),
            );
            assert!(err.starts_with("受保护子元素"), "实际: {err}");
        }
        // 表格块
        let tbl =
            wrap(r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
        let err = val_msg(
            apply_edits(
                &tbl,
                &styles,
                &[EditOp::SetPprElement {
                    block: 1,
                    expect_prefix: "表".into(),
                    element: "keepNext".into(),
                    xml: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("表格块"), "实际: {err}");
    }

    #[test]
    fn ppr_fragment_validation_families() {
        let styles = heading_styles();
        let para = wrap(r#"<w:p><w:r><w:t>片段段</w:t></w:r></w:p>"#);
        let run = |xml: Option<&str>| {
            val_msg(
                apply_edits(
                    &para,
                    &styles,
                    &[EditOp::SetPprElement {
                        block: 1,
                        expect_prefix: "片段".into(),
                        element: "numPr".into(),
                        xml: xml.map(str::to_string),
                    }],
                )
                .unwrap_err(),
            )
        };
        // 根元素名与 element 不一致
        let err = run(Some(r#"<w:jc w:val="center"/>"#));
        assert!(err.starts_with("片段校验失败"), "实际: {err}");
        assert!(err.contains("根元素"));
        // 未闭合
        let err = run(Some(r#"<w:numPr><w:ilvl w:val="0"/>"#));
        assert!(err.starts_with("片段校验失败"), "实际: {err}");
        // xmlns 声明
        let err = run(Some(r#"<w:numPr xmlns:w="http://x"/>"#));
        assert!(
            err.starts_with("片段校验失败") && err.contains("xmlns"),
            "实际: {err}"
        );
        // 夹带受保护元素
        let err = run(Some(r#"<w:numPr><w:sectPr/></w:numPr>"#));
        assert!(err.starts_with("片段校验失败"), "实际: {err}");
        // 平铺双根
        let err = run(Some("<w:keepNext/><w:keepLines/>"));
        assert!(err.starts_with("片段校验失败"), "实际: {err}");
    }

    /// 样式定义自带编号（h2 直接带 / h3 经 basedOn 继承）——链检测 fixture
    fn numbered_heading_styles() -> Stylesheet {
        let styles_xml = r#"<w:styles>
            <w:style w:type="paragraph" w:styleId="h2"><w:name w:val="heading 2"/>
              <w:pPr><w:numPr><w:numId w:val="9"/></w:numPr></w:pPr></w:style>
            <w:style w:type="paragraph" w:styleId="h3"><w:name w:val="heading 3"/>
              <w:basedOn w:val="h2"/></w:style>
            <w:style w:type="paragraph" w:styleId="body"><w:name w:val="Normal"/></w:style>
        </w:styles>"#;
        super::super::styles::parse_styles(&super::super::xml_dom::parse(styles_xml).unwrap())
    }

    #[test]
    fn ppr_numpr_removal_warns_on_style_chain_numbering() {
        // 样式链（含 basedOn 继承）定义编号 → 段级 numPr 摘除后显式警告；
        // 样式无编号 → 无警告
        let mk = |style: &str| {
            format!(
                r#"<w:p><w:pPr><w:pStyle w:val="{style}"/><w:numPr><w:ilvl w:val="2"/><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:t>双源段</w:t></w:r></w:p>"#
            )
        };
        for style in ["h2", "h3"] {
            let xml = wrap(&mk(style));
            let (_, applied) = apply_edits(
                &xml,
                &numbered_heading_styles(),
                &[EditOp::SetPprElement {
                    block: 1,
                    expect_prefix: "双源".into(),
                    element: "numPr".into(),
                    xml: None,
                }],
            )
            .unwrap();
            assert!(
                applied[0].after.contains("样式链仍定义编号"),
                "{style} 应触发回退警告，实际: {}",
                applied[0].after
            );
        }
        let xml = wrap(&mk("h2"));
        let (_, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetPprElement {
                block: 1,
                expect_prefix: "双源".into(),
                element: "numPr".into(),
                xml: None,
            }],
        )
        .unwrap();
        assert!(
            !applied[0].after.contains("样式链仍定义编号"),
            "样式无编号不应警告"
        );
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
                EditOp::SetStyle {
                    block: 1,
                    expect_prefix: "列表项".into(),
                    style: "h2".into(),
                },
                EditOp::SetStyle {
                    block: 2,
                    expect_prefix: "裸段".into(),
                    style: "h2".into(),
                },
                EditOp::SetStyle {
                    block: 3,
                    expect_prefix: "".into(),
                    style: "body".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 3);
        assert!(
            out.contains(r#"<w:pPr><w:pStyle w:val="h2"/><w:numPr>"#),
            "pStyle 应插在 pPr 首位: {out}"
        );
        assert!(out.contains(
            r#"<w:p><w:pPr><w:pStyle w:val="h2"/></w:pPr><w:r><w:t>裸段</w:t></w:r></w:p>"#
        ));
        assert!(
            out.contains(r#"<w:p w14:paraId="E"><w:pPr><w:pStyle w:val="body"/></w:pPr></w:p>"#)
        );
        let m = model_of(&out);
        assert_eq!(m.body.len(), 3, "块数守恒");
    }

    #[test]
    fn set_style_unknown_and_table_rejected() {
        let styles = heading_styles();
        // 未知样式（家族前缀）
        let xml = wrap(r#"<w:p><w:r><w:t>一段</w:t></w:r></w:p>"#);
        let err = val_msg(
            apply_edits(
                &xml,
                &styles,
                &[EditOp::SetStyle {
                    block: 1,
                    expect_prefix: "一段".into(),
                    style: "没有这样式".into(),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("未知样式"), "实际: {err}");
        // 表格块
        let xml =
            wrap(r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
        let err = val_msg(
            apply_edits(
                &xml,
                &styles,
                &[EditOp::SetStyle {
                    block: 1,
                    expect_prefix: "表".into(),
                    style: "h2".into(),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("表格块"), "实际: {err}");
        // 段落属性修订（pPrChange）——换样式与其语义纠缠，拒
        let xml = wrap(
            r#"<w:p><w:pPr><w:pStyle w:val="body"/><w:pPrChange w:id="1"><w:pPr><w:pStyle w:val="h2"/></w:pPr></w:pPrChange></w:pPr><w:r><w:t>改格段</w:t></w:r></w:p>"#,
        );
        let err = val_msg(
            apply_edits(
                &xml,
                &styles,
                &[EditOp::SetStyle {
                    block: 1,
                    expect_prefix: "改格段".into(),
                    style: "h2".into(),
                }],
            )
            .unwrap_err(),
        );
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
        assert!(
            applied[0].after.contains("对齐=center"),
            "摘要: {}",
            applied[0].after
        );
        // 合并断言：before 未提及原样 120；line/lineRule/after 追加进同一 spacing；firstLine 覆盖 720
        assert!(
            out.contains(
                r#"<w:spacing w:before="120" w:line="360" w:lineRule="auto" w:after="120"/>"#
            ),
            "spacing 合并: {out}"
        );
        assert!(
            out.contains(r#"<w:ind w:firstLine="720" w:firstLineChars="0"/>"#),
            "ind 覆盖 + chars 变体压 0（跨层压制）: {out}"
        );
        assert!(out.contains(r#"<w:jc w:val="center"/>"#), "jc 插入: {out}");
        // schema 序：pStyle < spacing < ind < jc
        let ppr = out.find("<w:pPr>").unwrap();
        let end = out.find("</w:pPr>").unwrap();
        let seg = &out[ppr..end];
        let pos = |pat: &str| seg.find(pat).unwrap();
        assert!(
            pos("<w:pStyle") < pos("<w:spacing")
                && pos("<w:spacing") < pos("<w:ind")
                && pos("<w:ind") < pos("<w:jc")
        );
        // 模型读回
        let m = model_of(&out);
        let super::docx_model::Block::Paragraph(p) = &m.body[0] else {
            panic!()
        };
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
        assert!(
            out.contains(r#"<w:rFonts w:ascii="Times"/>"#),
            "rFonts 原样: {out}"
        );
        assert!(out.contains("<w:u/>"), "无关子元素保留: {out}");
        assert_eq!(out.matches("<w:b/>").count(), 2, "两个 run 都加粗: {out}");
        assert_eq!(
            out.matches(r#"<w:sz w:val="28"/>"#).count(),
            2,
            "14pt = 28 半磅: {out}"
        );
        assert_eq!(out.matches(r#"<w:color w:val="FF0000"/>"#).count(), 2);
        // run 2 无 rPr → 新建（rPr 是 w:r 的 schema 首子元素）
        assert!(
            out.contains(r#"<w:r><w:rPr>"#),
            "无 rPr 的 run 新建 rPr: {out}"
        );
        // 模型读回
        let m = model_of(&out);
        let super::docx_model::Block::Paragraph(p) = &m.body[0] else {
            panic!()
        };
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
                    character: Some(CharFormat {
                        bold: Some(true),
                        ..Default::default()
                    }),
                },
                EditOp::SetFormat {
                    block: 2,
                    expect_prefix: "".into(),
                    paragraph: Some(ParaFormat {
                        line_spacing: Some(2.0),
                        ..Default::default()
                    }),
                    character: None,
                },
            ],
        )
        .unwrap();
        assert!(
            out.contains(
                r#"<w:p><w:pPr><w:spacing w:line="480" w:lineRule="auto"/></w:pPr></w:p>"#
            ),
            "块 2 自闭合空段展开并新建 pPr: {out}"
        );
        let m = model_of(&out);
        assert_eq!(m.body.len(), 2);
        let super::docx_model::Block::Paragraph(p1) = &m.body[0] else {
            panic!()
        };
        assert_eq!(p1.props.alignment.as_deref(), Some("both"));
        assert_eq!(p1.props.indent_first_line, Some(480));
        assert_eq!(
            p1.props.indent_left, None,
            "负左缩进 i32 → 模型 u32 不收，XML 已写"
        );
        assert!(out.contains(r#"w:left="-240""#), "负左缩进写入: {out}");
        assert_eq!(p1.runs[0].props.bold, Some(true));
        let super::docx_model::Block::Paragraph(p2) = &m.body[1] else {
            panic!()
        };
        assert_eq!(p2.props.spacing_line, Some(480));
    }

    #[test]
    fn set_format_indent_zero_suppresses_chars_variants() {
        // 缺口 2（2026-08-26 生产反馈）：正文样式带 firstLineChars=200，工具只写
        // firstLine=0、删掉直接层 chars 变体 → 样式层 chars 透出、缩进仍在。
        // 修复后显式写 0（chars 单位优先于 twips，跨层压制唯一正解）
        let xml = concat!(
            r#"<w:p><w:pPr><w:pStyle w:val="body"/>"#,
            r#"<w:ind w:firstLine="480" w:firstLineChars="200"/></w:pPr><w:r><w:t>甲</w:t></w:r></w:p>"#,
        );
        // ① 直接层原带 chars 变体（生产真实形态）→ 四变体显式归零
        let (out, _) = apply_edits(
            &wrap(xml),
            &heading_styles(),
            &[EditOp::SetFormat {
                block: 1,
                expect_prefix: "甲".into(),
                paragraph: Some(ParaFormat {
                    indent_first_line_tw: Some(0),
                    ..Default::default()
                }),
                character: None,
            }],
        )
        .unwrap();
        assert!(
            out.contains(
                r#"<w:ind w:firstLine="0" w:firstLineChars="0" w:hanging="0" w:hangingChars="0"/>"#
            ),
            "四变体归零（任意元素内优先序下都渲染无缩进）: {out}"
        );
        // ② 无直接 ind → 新建即四零（压制不依赖直接层已有 ind）
        let xml2 = wrap(r#"<w:p><w:r><w:t>乙</w:t></w:r></w:p>"#);
        let (out2, _) = apply_edits(
            &xml2,
            &Stylesheet::empty(),
            &[EditOp::SetFormat {
                block: 1,
                expect_prefix: "乙".into(),
                paragraph: Some(ParaFormat {
                    indent_first_line_tw: Some(0),
                    ..Default::default()
                }),
                character: None,
            }],
        )
        .unwrap();
        assert!(
            out2.contains(
                r#"<w:ind w:firstLine="0" w:firstLineChars="0" w:hanging="0" w:hangingChars="0"/>"#
            ),
            "新建即四零: {out2}"
        );
        // ③ 非零值：chars 压 0、hanging 系移除（写 hanging=0 会反伤——元素内 hanging 优先 firstLine）
        let (out3, _) = apply_edits(
            &wrap(xml),
            &heading_styles(),
            &[EditOp::SetFormat {
                block: 1,
                expect_prefix: "甲".into(),
                paragraph: Some(ParaFormat {
                    indent_first_line_tw: Some(480),
                    ..Default::default()
                }),
                character: None,
            }],
        )
        .unwrap();
        assert!(
            out3.contains(r#"<w:ind w:firstLine="480" w:firstLineChars="0"/>"#),
            "非零：chars=0: {out3}"
        );
        assert!(
            !out3.contains("w:hanging"),
            "非零不写 hanging（反伤 firstLine）: {out3}"
        );
        // ④ 左缩进同律：leftChars=0 压样式层
        let (out4, _) = apply_edits(
            &xml2,
            &Stylesheet::empty(),
            &[EditOp::SetFormat {
                block: 1,
                expect_prefix: "乙".into(),
                paragraph: Some(ParaFormat {
                    indent_left_tw: Some(0),
                    ..Default::default()
                }),
                character: None,
            }],
        )
        .unwrap();
        assert!(
            out4.contains(r#"<w:ind w:left="0" w:leftChars="0"/>"#),
            "左缩进双零: {out4}"
        );
    }

    #[test]
    fn set_cell_format_style_restyles_cell_paragraphs() {
        // 缺口 1（2026-08-26 生产反馈）：格内段落无法套段落样式（set_style 拒表格块）。
        // set_cell_format 加 style：格内全段 pStyle 手术，名/ID 反查同 set_style
        let cell = concat!(
            r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr>"#,
            r#"<w:p><w:pPr><w:ind w:firstLine="480"/></w:pPr><w:r><w:t>上</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>下</w:t></w:r></w:p></w:tc>"#,
        );
        let xml = wrap(&format!(
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid><w:tr>{}{}</w:tr></w:tbl>"#,
            cell,
            cell_xml("邻")
        ));
        // ① 纯样式操作（paragraph/character 全空）合法 + 摘要携带样式
        let (out, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetCellFormat {
                block: 1,
                expect_prefix: "上".into(),
                row: 1,
                cell: 1,
                paragraph: None,
                character: None,
                style: Some("heading 2".into()),
            }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "set_cell_format");
        assert_eq!(
            applied[0].style.as_deref(),
            Some("h2"),
            "AppliedOp.style 携带解析后 ID"
        );
        assert!(
            applied[0].after.contains("样式=heading 2"),
            "摘要: {}",
            applied[0].after
        );
        assert_eq!(
            out.matches(r#"<w:pStyle w:val="h2"/>"#).count(),
            2,
            "格内两段都换样式: {out}"
        );
        assert!(
            out.contains(r#"<w:ind w:firstLine="480"/>"#),
            "既有 ind 保留: {out}"
        );
        let before_last_close = out.rsplit_once("</w:tc>").unwrap().0;
        let neighbor = &out[before_last_close.rfind("<w:tc>").unwrap()..];
        assert!(!neighbor.contains("w:pStyle"), "邻格未动: {neighbor}");
        // 模型读回：格内两段样式都生效
        let m = model_of(&out);
        let super::docx_model::Block::Table(t) = &m.body[0] else {
            panic!()
        };
        for b in &t.rows[0].cells[0].blocks {
            let super::docx_model::Block::Paragraph(p) = b else {
                continue;
            };
            assert_eq!(p.props.style.as_deref(), Some("h2"));
        }
        // ② 样式 + 直接格式同手：pStyle 首位 + 四零 ind（与缺口 2 修复叠加的生产配方）
        let (out2, _) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetCellFormat {
                block: 1,
                expect_prefix: "上".into(),
                row: 1,
                cell: 1,
                paragraph: Some(ParaFormat {
                    indent_first_line_tw: Some(0),
                    ..Default::default()
                }),
                character: None,
                style: Some("heading 2".into()),
            }],
        )
        .unwrap();
        assert!(
            out2.contains(
                r#"<w:pPr><w:pStyle w:val="h2"/><w:ind w:firstLine="0" w:firstLineChars="0" w:hanging="0" w:hangingChars="0"/></w:pPr>"#
            ),
            "pStyle 首位（schema 序）+ 四零 ind: {out2}"
        );
        // ③ 未知样式家族前缀（与 set_style 同判）
        let err = val_msg(
            apply_edits(
                &xml,
                &heading_styles(),
                &[EditOp::SetCellFormat {
                    block: 1,
                    expect_prefix: "上".into(),
                    row: 1,
                    cell: 1,
                    paragraph: None,
                    character: None,
                    style: Some("没有这样式".into()),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("未知样式"), "实际: {err}");
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
        let err =
            val_msg(apply_edits(&xml, &Stylesheet::empty(), &[empty_op(None, None)]).unwrap_err());
        assert!(err.starts_with("空格式操作"), "实际: {err}");
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[empty_op(Some(ParaFormat::default()), None)],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("空格式操作"), "实际: {err}");
        // 对齐白名单
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[empty_op(
                    Some(ParaFormat {
                        align: Some("middle".into()),
                        ..Default::default()
                    }),
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
                    Some(CharFormat {
                        color: Some("RED".into()),
                        ..Default::default()
                    }),
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
                    Some(CharFormat {
                        font_size_pt: Some(999.0),
                        ..Default::default()
                    }),
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
                    Some(ParaFormat {
                        indent_first_line_tw: Some(-10),
                        ..Default::default()
                    }),
                    None,
                )],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("缩进值无效"), "实际: {err}");
        // 表格块（前缀对准表内文字，确保落在表格拒绝而非指纹不符）
        let tbl =
            wrap(r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
        let err = val_msg(
            apply_edits(
                &tbl,
                &Stylesheet::empty(),
                &[EditOp::SetFormat {
                    block: 1,
                    expect_prefix: "表".into(),
                    paragraph: None,
                    character: Some(CharFormat {
                        bold: Some(true),
                        ..Default::default()
                    }),
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
        use docx_rs::{Document, Docx, Paragraph, Run};
        let document = Document::new()
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第一段")))
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第二段")));
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        Docx::new()
            .document(document)
            .build()
            .pack(&mut cursor)
            .unwrap();
        let original = cursor.into_inner();

        let xml = docx::read_document_xml(&original).unwrap();
        let repacked = repack_part(&original, "word/document.xml", &xml).unwrap();

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
        let repacked2 = repack_part(&original, "word/document.xml", &new_xml).unwrap();
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

    fn read_entry_at<R: std::io::Read + std::io::Seek>(
        zip: &mut zip::ZipArchive<R>,
        i: usize,
    ) -> (String, Vec<u8>) {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().to_string();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        (name, data)
    }

    // ---- 表格四件（S3 三波）----

    /// 两格行 / 两行两格表 fixture。
    fn cell_xml(text: &str) -> String {
        format!(
            r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#
        )
    }
    fn row_xml(a: &str, b: &str) -> String {
        format!("<w:tr>{}{}</w:tr>", cell_xml(a), cell_xml(b))
    }
    fn tbl_of(rows: &[String]) -> String {
        format!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid>{}</w:tbl>"#,
            rows.concat()
        )
    }
    fn two_row_table_doc() -> String {
        wrap(&tbl_of(&[row_xml("甲一", "乙一"), row_xml("甲二", "乙二")]))
    }

    #[test]
    fn insert_table_after_builds_grid() {
        let xml = wrap(r#"<w:p><w:r><w:t>锚段</w:t></w:r></w:p>"#);
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableAfter {
                block: 1,
                expect_prefix: "锚段".into(),
                rows: vec![
                    vec!["列一".into(), "列二".into()],
                    vec!["1".into(), "2".into()],
                ],
                header: None,
                table_style: None,
            }],
        )
        .unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].op, "insert_table_after");
        // 块数守恒：1 段 + 1 表
        let m = model_of(&out);
        assert_eq!(m.body.len(), 2);
        let Block::Table(t) = &m.body[1] else {
            panic!("块 2 应为表格")
        };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0].cells.len(), 2);
        // 默认表头：加粗 + 跨页重复 + 全边框 + 100% 宽
        assert!(out.contains("<w:tblHeader/>"), "表头行应带 tblHeader");
        assert!(out.contains("<w:b/>"), "表头应加粗");
        assert!(out.contains(r#"w:type="pct""#), "tblW 应为百分比宽");
        assert!(out.contains("<w:insideH"), "应有内框线");
        // header=false：无表头标记无加粗
        let (out2, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableAfter {
                block: 1,
                expect_prefix: "锚段".into(),
                rows: vec![vec!["a".into(), "b".into()]],
                header: Some(false),
                table_style: None,
            }],
        )
        .unwrap();
        assert!(!out2.contains("<w:tblHeader/>"));
        // \n = 格内多段（两个 <w:p>，非 br——表格格内软换行走真段落）
        let (out3, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableAfter {
                block: 1,
                expect_prefix: "锚段".into(),
                rows: vec![vec!["一\n二".into(), "b".into()]],
                header: Some(false),
                table_style: None,
            }],
        )
        .unwrap();
        assert!(!out3.contains("<w:br/>"), "格内 \\n 不应转 br（设计=分段）");
        let cell1 = out3
            .find("一</w:t>")
            .map(|p| out3[..p].matches("<w:p>").count())
            .unwrap_or(0);
        assert_eq!(
            cell1,
            2,
            "首格应含两个段落: {}",
            &out3[out3.find("<w:tbl>").unwrap()..out3.find("<w:tbl>").unwrap() + 300]
        );
    }

    #[test]
    fn insert_table_after_rejects_bad_rows() {
        let xml = wrap(r#"<w:p><w:r><w:t>锚</w:t></w:r></w:p>"#);
        // 空 rows
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::InsertTableAfter {
                    block: 1,
                    expect_prefix: "锚".into(),
                    rows: vec![],
                    header: None,
                    table_style: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("表格数据无效"), "实际: {err}");
        // 非矩形
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::InsertTableAfter {
                    block: 1,
                    expect_prefix: "锚".into(),
                    rows: vec![vec!["a".into(), "b".into()], vec!["c".into()]],
                    header: None,
                    table_style: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("表格数据无效"), "实际: {err}");
        assert!(err.contains("2 行"), "应指明行号: {err}");
        // 超行数上限
        let big: Vec<Vec<String>> = (0..201).map(|i| vec![i.to_string()]).collect();
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::InsertTableAfter {
                    block: 1,
                    expect_prefix: "锚".into(),
                    rows: big,
                    header: None,
                    table_style: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("表格数据无效"), "实际: {err}");
        assert!(err.contains("超上限"), "实际: {err}");
    }

    #[test]
    fn set_cell_text_preserves_structure_and_formats() {
        // 格 (1,1)：gridSpan=2 + 段 pPr 居中 + run 加粗——改文本全保留
        let span_cell = r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/><w:gridSpan w:val="2"/></w:tcPr><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>旧文</w:t></w:r></w:p></w:tc>"#;
        let xml = wrap(&format!(
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid><w:tr>{}{}</w:tr></w:tbl>"#,
            span_cell,
            cell_xml("乙")
        ));
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetCellText {
                block: 1,
                expect_prefix: "旧文".into(),
                row: 1,
                cell: 1,
                text: "新文\n二段".into(),
            }],
        )
        .unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].op, "set_cell_text");
        assert_eq!(applied[0].before, "旧文");
        // 原格 1 段 / 新文 2 段 → 不对齐，回落首段模板（摘要披露，十一波规则二）
        assert_eq!(
            applied[0].after,
            "新文\n二段（段数不对齐，格式回落首段模板）"
        );
        // tcPr 整体保留（tcW + gridSpan）
        assert!(
            out.contains(r#"<w:gridSpan w:val="2"/>"#),
            "gridSpan 应保留"
        );
        // 段/字符格式保留 + \n 成两段
        assert!(out.contains(r#"<w:jc w:val="center"/>"#), "段落格式应保留");
        assert!(out.contains("<w:b/>"), "字符格式应保留");
        assert!(out.contains("新文"), "新文本应写入");
        assert!(!out.contains("旧文"), "旧文本应替换");
        assert_eq!(out.matches("<w:p>").count(), 2 + 1, "格 1 两段 + 格 2 一段");
        // 产物可回读且结构不变
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows[0].cells.len(), 2);
        assert_eq!(t.rows[0].cells[0].grid_span, Some(2));
        // 空文本 = 清空（留一个带原格式的空段）
        let (out2, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetCellText {
                block: 1,
                expect_prefix: "旧文".into(),
                row: 1,
                cell: 1,
                text: String::new(),
            }],
        )
        .unwrap();
        assert!(
            out2.contains(r#"<w:jc w:val="center"/>"#),
            "清空仍保留段落格式"
        );
        assert!(!out2.contains("旧文"));
    }

    #[test]
    fn set_cell_text_error_families() {
        // 非表格块
        let xml = wrap(r#"<w:p><w:r><w:t>段</w:t></w:r></w:p>"#);
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetCellText {
                    block: 1,
                    expect_prefix: "段".into(),
                    row: 1,
                    cell: 1,
                    text: "x".into(),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("非表格块"), "实际: {err}");
        // 行越界 / 格越界
        let xml = two_row_table_doc();
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetCellText {
                    block: 1,
                    expect_prefix: "甲".into(),
                    row: 5,
                    cell: 1,
                    text: "x".into(),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("单元格越界"), "实际: {err}");
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetCellText {
                    block: 1,
                    expect_prefix: "甲".into(),
                    row: 1,
                    cell: 3,
                    text: "x".into(),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("单元格越界"), "实际: {err}");
        // 纵向合并续格（bare vMerge = continue）——续格放第 2 行，指纹取首行
        let cont_cell = r#"<w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc>"#;
        let cont_row = format!("<w:tr>{}{}</w:tr>", cont_cell, cell_xml("乙二"));
        let xml = wrap(&format!(
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid>{}{}</w:tbl>"#,
            row_xml("甲一", "乙一"),
            cont_row
        ));
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetCellText {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    row: 2,
                    cell: 1,
                    text: "x".into(),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("纵向合并续格"), "实际: {err}");
        assert!(err.contains("合并头"), "应指向合并头: {err}");
        // 同一格多操作
        let xml = two_row_table_doc();
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[
                    EditOp::SetCellText {
                        block: 1,
                        expect_prefix: "甲".into(),
                        row: 1,
                        cell: 1,
                        text: "x".into(),
                    },
                    EditOp::SetCellText {
                        block: 1,
                        expect_prefix: "甲".into(),
                        row: 1,
                        cell: 1,
                        text: "y".into(),
                    },
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("同一格多操作"), "实际: {err}");
        // 段落操作与表格操作同块冲突
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[
                    EditOp::SetCellText {
                        block: 1,
                        expect_prefix: "甲".into(),
                        row: 1,
                        cell: 1,
                        text: "x".into(),
                    },
                    EditOp::DeleteBlock {
                        block: 1,
                        expect_prefix: "甲".into(),
                    },
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
    }

    #[test]
    fn insert_table_row_after_clones_template() {
        // 模板行带 vMerge restart + gridSpan——克隆后结构原样、文本替换
        let merged_cell = r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/><w:vMerge w:val="restart"/><w:gridSpan w:val="1"/></w:tcPr><w:p><w:r><w:t>头</w:t></w:r></w:p></w:tc>"#;
        let merged_row = format!("<w:tr>{}{}</w:tr>", merged_cell, cell_xml("右"));
        let xml = wrap(&format!(
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid>{}{}</w:tbl>"#,
            merged_row,
            row_xml("甲二", "乙二")
        ));
        // 克隆第 1 行插到其后 + 填文本
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableRowAfter {
                block: 1,
                expect_prefix: "头".into(),
                after_row: Some(1),
                cells: Some(vec!["新甲".into(), "新乙".into()]),
            }],
        )
        .unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].op, "insert_table_row_after");
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows.len(), 3, "2 行 + 1 克隆");
        // 新行（第 2 行）结构继承：vMerge restart / 格数一致；文本已换
        assert_eq!(
            t.rows[1].cells[0].v_merge.as_deref(),
            Some("restart"),
            "vMerge 应随克隆保留"
        );
        assert!(out.contains("新甲"), "cells 文本应写入");
        // 缺省：克隆末行、全空格
        let (out2, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableRowAfter {
                block: 1,
                expect_prefix: "头".into(),
                after_row: None,
                cells: None,
            }],
        )
        .unwrap();
        let m2 = model_of(&out2);
        let Block::Table(t2) = &m2.body[0] else {
            panic!()
        };
        assert_eq!(t2.rows.len(), 3);
        // 新末行文本为空（甲二/乙二 不重复出现三次）
        assert_eq!(out2.matches("甲二").count(), 1, "克隆行文本应清空");
    }

    #[test]
    fn insert_table_row_after_error_families() {
        let xml = two_row_table_doc();
        // 行号越界
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::InsertTableRowAfter {
                    block: 1,
                    expect_prefix: "甲".into(),
                    after_row: Some(9),
                    cells: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("行号越界"), "实际: {err}");
        // 列数不符
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::InsertTableRowAfter {
                    block: 1,
                    expect_prefix: "甲".into(),
                    after_row: Some(1),
                    cells: Some(vec!["只给一格".into(), "两格".into(), "三格".into()]),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("列数不符"), "实际: {err}");
        // 非表格块
        let xml_p = wrap(r#"<w:p><w:r><w:t>段</w:t></w:r></w:p>"#);
        let err = val_msg(
            apply_edits(
                &xml_p,
                &Stylesheet::empty(),
                &[EditOp::InsertTableRowAfter {
                    block: 1,
                    expect_prefix: "段".into(),
                    after_row: None,
                    cells: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("非表格块"), "实际: {err}");
    }

    #[test]
    fn same_table_batch_insert_then_fill() {
        // 「增行 + 填新行格」一批完成：虚拟行寻址 + 按序生效
        let xml = two_row_table_doc();
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::InsertTableRowAfter {
                    block: 1,
                    expect_prefix: "甲".into(),
                    after_row: None,
                    cells: None,
                },
                EditOp::SetCellText {
                    block: 1,
                    expect_prefix: "甲".into(),
                    row: 3,
                    cell: 1,
                    text: "新行甲".into(),
                },
                EditOp::SetCellText {
                    block: 1,
                    expect_prefix: "甲".into(),
                    row: 3,
                    cell: 2,
                    text: "新行乙".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 3, "三操作同块按序组合");
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows.len(), 3);
        let mut text = String::new();
        for c in &t.rows[2].cells {
            docx_model::blocks_text(&c.blocks, &mut text);
        }
        assert_eq!(text, "新行甲\n新行乙\n", "新行两格已填: {text}");
        // 原行不动
        let mut text0 = String::new();
        for c in &t.rows[0].cells {
            docx_model::blocks_text(&c.blocks, &mut text0);
        }
        assert_eq!(text0, "甲一\n乙一\n");
    }

    #[test]
    fn text_op_on_table_points_to_table_ops() {
        // 段落操作命中表格块 → 家族前缀「表格块」+ 指路新三件
        let xml = two_row_table_doc();
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::ReplaceText {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    new_text: "x".into(),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("表格块"), "实际: {err}");
        assert!(err.contains("set_cell_text"), "应指路 set_cell_text: {err}");
        assert!(err.contains("insert_table_after"), "应指路建表: {err}");
    }

    #[test]
    fn direct_children_spans_skips_nested() {
        // 行内嵌套表（tc 里的 w:tbl 含 w:tr）不得计入直接子行
        let nested_row = format!(
            r#"<w:tr>{}<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr><w:tbl><w:tr>{}</w:tr></w:tbl><w:p/></w:tc></w:tr>"#,
            cell_xml("外"),
            row_xml("内甲", "内乙")
        );
        let tbl = format!(
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid>{nested_row}{}</w:tbl>"#,
            row_xml("二行", "二行乙")
        );
        let rows = super::direct_children_spans(&tbl, "tr");
        assert_eq!(rows.len(), 2, "嵌套 w:tr 不计入: {}", rows.len());
        for (s, e) in &rows {
            assert!(tbl[*s..*e].starts_with("<w:tr>"));
            assert!(tbl[*s..*e].ends_with("</w:tr>"));
        }
        // 格级同理：首行 2 格（嵌套表在格内，其 w:tc 不计入）
        let first_row = &tbl[rows[0].0..rows[0].1];
        let cells = super::direct_children_spans(first_row, "tc");
        assert_eq!(cells.len(), 2, "嵌套 w:tc 不计入");
    }

    // ---- 表格格式四件（S3 四波）----

    const SHD_FRAG: &str = r#"<w:shd w:val="clear" w:color="auto" w:fill="DDEEFF"/>"#;

    #[test]
    fn set_table_element_table_level_upsert_remove_noop() {
        let xml = two_row_table_doc(); // tblPr 内已有 tblW
                                       // 插入 shd（schema 序在 tblBorders 后、无 tblBorders 则 tblW 后）
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetTableElement {
                block: 1,
                expect_prefix: "甲一".into(),
                level: TableLevel::Table,
                row: None,
                cell: None,
                element: "shd".into(),
                xml: Some(SHD_FRAG.into()),
            }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "set_table_element");
        assert_eq!(applied[0].after, "set table:shd");
        let tblpr = out
            .split("<w:tblPr>")
            .nth(1)
            .unwrap()
            .split("</w:tblPr>")
            .next()
            .unwrap();
        assert!(
            tblpr.contains(r#"<w:tblW w:w="0" w:type="auto"/>"#),
            "tblW 原样保留"
        );
        assert!(tblpr.contains(SHD_FRAG), "shd 已插入");
        assert!(
            tblpr.find("<w:tblW").unwrap() < tblpr.find("<w:shd").unwrap(),
            "schema 序：tblW 在 shd 前"
        );
        // 替换：同元素再写不同值
        let frag2 = r#"<w:shd w:val="clear" w:color="auto" w:fill="FFFFFF"/>"#;
        let (out2, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetTableElement {
                block: 1,
                expect_prefix: "甲一".into(),
                level: TableLevel::Table,
                row: None,
                cell: None,
                element: "shd".into(),
                xml: Some(frag2.into()),
            }],
        )
        .unwrap();
        assert!(out2.contains(frag2), "整元素替换");
        // 移除 tblW → 摘空（tblPr 只剩 tblW 的 fixture）→ 容器整体清理
        let (out3, applied3) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetTableElement {
                block: 1,
                expect_prefix: "甲一".into(),
                level: TableLevel::Table,
                row: None,
                cell: None,
                element: "tblW".into(),
                xml: None,
            }],
        )
        .unwrap();
        assert_eq!(applied3[0].after, "removed table:tblW");
        assert!(!out3.contains("<w:tblPr"), "摘空容器整体清理");
        // 空转：元素不存在 + xml=null
        let (out4, applied4) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetTableElement {
                block: 1,
                expect_prefix: "甲一".into(),
                level: TableLevel::Table,
                row: None,
                cell: None,
                element: "tblLook".into(),
                xml: None,
            }],
        )
        .unwrap();
        assert_eq!(out4, xml, "空转不改字节");
        assert!(
            applied4[0].after.contains("不存在"),
            "空转信号: {}",
            applied4[0].after
        );
    }

    #[test]
    fn set_table_element_creates_container_when_absent() {
        // tblPr 自闭合空容器 → 有片段时展开为完整容器
        let xml = wrap(&format!(
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid>{}</w:tbl>"#,
            row_xml("甲", "乙")
        ));
        let (out, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetTableElement {
                block: 1,
                expect_prefix: "甲".into(),
                level: TableLevel::Table,
                row: None,
                cell: None,
                element: "tblBorders".into(),
                xml: Some(
                    r#"<w:tblBorders><w:insideH w:val="single" w:sz="4"/></w:tblBorders>"#.into(),
                ),
            }],
        )
        .unwrap();
        assert!(
            out.contains(r#"<w:tblPr><w:tblBorders>"#),
            "空容器展开: {out}"
        );
        // tblPr 完全缺失 → 开标签后新建
        let xml2 = wrap(&format!(
            r#"<w:tbl><w:tblGrid><w:gridCol w:w="4500"/></w:tblGrid><w:tr>{}</w:tr></w:tbl>"#,
            cell_xml("独")
        ));
        let (out2, _) = apply_edits(
            &xml2,
            &Stylesheet::empty(),
            &[EditOp::SetTableElement {
                block: 1,
                expect_prefix: "独".into(),
                level: TableLevel::Table,
                row: None,
                cell: None,
                element: "tblW".into(),
                xml: Some(r#"<w:tblW w:w="0" w:type="auto"/>"#.into()),
            }],
        )
        .unwrap();
        assert!(
            out2.contains(
                r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid>"#
            ),
            "容器新建于开标签后: {out2}"
        );
    }

    #[test]
    fn set_table_element_row_and_cell_levels() {
        let xml = two_row_table_doc();
        // 行级：trHeight（容器不存在 → 新建于 tr 开标签后）
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetTableElement {
                block: 1,
                expect_prefix: "甲一".into(),
                level: TableLevel::Row,
                row: Some(2),
                cell: None,
                element: "trHeight".into(),
                xml: Some(r#"<w:trHeight w:val="400" w:hRule="atLeast"/>"#.into()),
            }],
        )
        .unwrap();
        assert_eq!(applied[0].after, "set r2:trHeight");
        // 第二行（甲二/乙二）带 trPr，第一行不带
        let r2 = out.split("甲二").next().unwrap();
        assert!(
            r2.contains(r#"<w:trPr><w:trHeight w:val="400" w:hRule="atLeast"/></w:trPr>"#),
            "行级容器新建: {out}"
        );
        // 格级：shd + vAlign
        let (out2, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetTableElement {
                block: 1,
                expect_prefix: "甲一".into(),
                level: TableLevel::Cell,
                row: Some(1),
                cell: Some(2),
                element: "shd".into(),
                xml: Some(SHD_FRAG.into()),
            }],
        )
        .unwrap();
        assert!(
            out2.contains(&format!(r#"<w:tcW w:w="4500" w:type="dxa"/>{SHD_FRAG}"#)),
            "tcPr 内 schema 位插入"
        );
        // 模型读回：格特征上屏
        let m = model_of(&out2);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows[0].cells[1].shd_fill.as_deref(), Some("DDEEFF"));
        assert_eq!(t.rows[0].cells[0].shd_fill, None, "只动目标格");
    }

    #[test]
    fn set_table_element_error_families() {
        let xml = two_row_table_doc();
        // 受保护结构属性
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetTableElement {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    level: TableLevel::Cell,
                    row: Some(1),
                    cell: Some(1),
                    element: "gridSpan".into(),
                    xml: Some(r#"<w:gridSpan w:val="2"/>"#.into()),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("受保护子元素"), "实际: {err}");
        assert!(err.contains("merge_cells"), "应指路 merge_cells: {err}");
        // 白名单外
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetTableElement {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    level: TableLevel::Table,
                    row: None,
                    cell: None,
                    element: "tblGridX".into(),
                    xml: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("非法子元素"), "实际: {err}");
        assert!(err.contains("tblpr"), "应指路 tblpr 投影: {err}");
        // level 与 row/cell 组合校验
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetTableElement {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    level: TableLevel::Table,
                    row: Some(1),
                    cell: None,
                    element: "shd".into(),
                    xml: Some(SHD_FRAG.into()),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("参数校验失败"), "实际: {err}");
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetTableElement {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    level: TableLevel::Cell,
                    row: Some(1),
                    cell: None,
                    element: "shd".into(),
                    xml: Some(SHD_FRAG.into()),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("参数校验失败"), "实际: {err}");
        // 行越界
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetTableElement {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    level: TableLevel::Row,
                    row: Some(9),
                    cell: None,
                    element: "trHeight".into(),
                    xml: Some(r#"<w:trHeight w:val="400"/>"#.into()),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("行号越界"), "实际: {err}");
        // 片段校验（根名不符）
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetTableElement {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    level: TableLevel::Table,
                    row: None,
                    cell: None,
                    element: "shd".into(),
                    xml: Some(r#"<w:wrong val="1"/>"#.into()),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.contains("tblpr"), "片段校验错应指路 tblpr: {err}");
        // 非表格块
        let xml_p = wrap(r#"<w:p><w:r><w:t>段</w:t></w:r></w:p>"#);
        let err = val_msg(
            apply_edits(
                &xml_p,
                &Stylesheet::empty(),
                &[EditOp::SetTableElement {
                    block: 1,
                    expect_prefix: "段".into(),
                    level: TableLevel::Table,
                    row: None,
                    cell: None,
                    element: "shd".into(),
                    xml: Some(SHD_FRAG.into()),
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("非表格块"), "实际: {err}");
    }

    #[test]
    fn set_cell_format_hits_all_paras_and_runs() {
        // 格内两段（一段带文本、一段空）+ 全 run 加粗 + 段落居中
        let cell = r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>上</w:t></w:r></w:p><w:p><w:r><w:t>下</w:t></w:r></w:p></w:tc>"#;
        let xml = wrap(&format!(
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid><w:tr>{}{}</w:tr></w:tbl>"#,
            cell,
            cell_xml("邻")
        ));
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetCellFormat {
                block: 1,
                expect_prefix: "上".into(),
                row: 1,
                cell: 1,
                paragraph: Some(ParaFormat {
                    align: Some("center".into()),
                    ..Default::default()
                }),
                character: Some(CharFormat {
                    bold: Some(true),
                    ..Default::default()
                }),
                style: None,
            }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "set_cell_format");
        assert_eq!(
            out.matches(r#"<w:jc w:val="center"/>"#).count(),
            2,
            "两段都居中: {out}"
        );
        assert_eq!(
            out.matches("<w:b/>").count(),
            2,
            "两 run 都加粗（邻格不中枪）: {out}"
        );
        // 邻格（最后一个 tc）原样：无 jc 无 b
        let before_last_close = out.rsplit_once("</w:tc>").unwrap().0;
        let neighbor = &out[before_last_close.rfind("<w:tc>").unwrap()..];
        assert!(neighbor.contains("邻"), "提取到的确是邻格: {neighbor}");
        assert!(
            !neighbor.contains("w:jc") && !neighbor.contains("<w:b/>"),
            "邻格未动: {neighbor}"
        );
        // 只给 character（paragraph None）合法
        let (out2, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetCellFormat {
                block: 1,
                expect_prefix: "上".into(),
                row: 1,
                cell: 1,
                paragraph: None,
                character: Some(CharFormat {
                    font_size_pt: Some(14.0),
                    ..Default::default()
                }),
                style: None,
            }],
        )
        .unwrap();
        assert_eq!(
            out2.matches(r#"<w:sz w:val="28"/>"#).count(),
            2,
            "14pt → sz=28 半磅"
        );
        // 两个都 None → 空格式操作（与 set_format 同家族，文案点名 set_cell_format）
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetCellFormat {
                    block: 1,
                    expect_prefix: "上".into(),
                    row: 1,
                    cell: 1,
                    paragraph: None,
                    character: None,
                    style: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("空格式操作"), "实际: {err}");
        assert!(err.contains("set_cell_format"), "实际: {err}");
        // 越界家族
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SetCellFormat {
                    block: 1,
                    expect_prefix: "上".into(),
                    row: 1,
                    cell: 9,
                    paragraph: Some(ParaFormat {
                        align: Some("center".into()),
                        ..Default::default()
                    }),
                    character: None,
                    style: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("单元格越界"), "实际: {err}");
    }

    /// 纵并 3 行：r1 restart、r2/r3 continue；内容留原格。
    fn vmerged_doc() -> String {
        let head = r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>头</w:t></w:r></w:p></w:tc>"#;
        let cont = r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/><w:vMerge/></w:tcPr><w:p><w:r><w:t>藏一</w:t></w:r></w:p></w:tc>"#;
        let cont2 = r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/><w:vMerge/></w:tcPr><w:p><w:r><w:t>藏二</w:t></w:r></w:p></w:tc>"#;
        wrap(&format!(
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr><w:tr>{}{}</w:tr></w:tbl>"#,
            head,
            cell_xml("右上"),
            cont,
            cell_xml("右中"),
            cont2,
            cell_xml("右下"),
        ))
    }

    #[test]
    fn merge_vertical_sets_restart_continue_keeps_content() {
        let xml = vmerged_doc();
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::MergeCells {
                block: 1,
                expect_prefix: "头".into(),
                direction: Some(MergeDirection::Vertical),
                row: 1,
                cell: 1,
                span: Some(3),
                end_row: None,
                end_cell: None,
            }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "merge_cells");
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows[0].cells[0].v_merge.as_deref(), Some("restart"));
        assert_eq!(t.rows[1].cells[0].v_merge.as_deref(), Some("continue"));
        assert_eq!(t.rows[2].cells[0].v_merge.as_deref(), Some("continue"));
        // 内容留原格（拆分即恢复）：藏一/藏二 仍在 XML
        assert!(out.contains("藏一") && out.contains("藏二"), "纵并内容保留");
        // 右列不中枪
        assert_eq!(t.rows[1].cells[1].v_merge, None);
    }

    #[test]
    fn merge_vertical_misaligned_rejected() {
        // 第 2 行首格跨 2 列 → [0,1) 网格区间无整格对齐
        let span_row = r#"<w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>整行</w:t></w:r></w:p></w:tc></w:tr>"#.to_string();
        let xml = wrap(&format!(
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid>{}{}</w:tbl>"#,
            row_xml("甲", "乙"),
            span_row
        ));
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::MergeCells {
                    block: 1,
                    expect_prefix: "甲".into(),
                    direction: Some(MergeDirection::Vertical),
                    row: 1,
                    cell: 1,
                    span: Some(2),
                    end_row: None,
                    end_cell: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("合并结构冲突"), "实际: {err}");
        assert!(err.contains("对齐"), "应解释网格对齐: {err}");
    }

    #[test]
    fn merge_horizontal_concatenates_and_sums_span() {
        let xml = two_row_table_doc();
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::MergeCells {
                block: 1,
                expect_prefix: "甲一".into(),
                direction: Some(MergeDirection::Horizontal),
                row: 1,
                cell: 1,
                span: Some(2),
                end_row: None,
                end_cell: None,
            }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "merge_cells");
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows[0].cells.len(), 1, "两格并一格");
        assert_eq!(t.rows[0].cells[0].grid_span, Some(2), "gridSpan 求和");
        let mut text = String::new();
        docx_model::blocks_text(&t.rows[0].cells[0].blocks, &mut text);
        let joined: String = text.split('\n').collect::<Vec<_>>().join("");
        assert!(
            joined.contains("甲一") && joined.contains("乙一"),
            "内容按序拼接: {joined}"
        );
        // 第二行不中枪
        assert_eq!(t.rows[1].cells.len(), 2);
    }

    #[test]
    fn split_vertical_restores_whole_chain() {
        let xml = vmerged_doc();
        // 先纵并整链（r1 头 + 2 续），再拆 → vMerge 全部消失、内容恢复独立
        let (merged, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::MergeCells {
                block: 1,
                expect_prefix: "头".into(),
                direction: Some(MergeDirection::Vertical),
                row: 1,
                cell: 1,
                span: Some(3),
                end_row: None,
                end_cell: None,
            }],
        )
        .unwrap();
        let (split, applied) = apply_edits(
            &merged,
            &Stylesheet::empty(),
            &[EditOp::SplitCell {
                block: 1,
                expect_prefix: "头".into(),
                direction: MergeDirection::Vertical,
                row: 1,
                cell: 1,
            }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "split_cell");
        assert!(!split.contains("vMerge"), "vMerge 全链摘除: {split}");
        // 与原始模型等价（格数/文本）——纵并拆分 = 语义还原
        let (mo, ms) = (model_of(&xml), model_of(&split));
        let mut a = String::new();
        let mut b = String::new();
        docx_model::blocks_text(&mo.body, &mut a);
        docx_model::blocks_text(&ms.body, &mut b);
        assert_eq!(a, b, "拆分后文本投影与原doc一致");
    }

    #[test]
    fn split_horizontal_restores_unit_cells() {
        let xml = two_row_table_doc();
        let (merged, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::MergeCells {
                block: 1,
                expect_prefix: "甲一".into(),
                direction: Some(MergeDirection::Horizontal),
                row: 1,
                cell: 1,
                span: Some(2),
                end_row: None,
                end_cell: None,
            }],
        )
        .unwrap();
        let (split, applied) = apply_edits(
            &merged,
            &Stylesheet::empty(),
            &[EditOp::SplitCell {
                block: 1,
                expect_prefix: "甲一".into(),
                direction: MergeDirection::Horizontal,
                row: 1,
                cell: 1,
            }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "split_cell");
        let m = model_of(&split);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows[0].cells.len(), 2, "拆回 2 格");
        assert_eq!(t.rows[0].cells[0].grid_span, None, "首格回单格");
        assert_eq!(t.rows[0].cells[1].grid_span, None, "补格单格");
        // 内容留首格、补格空段（继承首段格式模板；空 ppr 展开为成对标签）
        assert!(
            split.contains("甲一") && split.contains("乙一"),
            "内容留首格"
        );
        assert!(split.contains("<w:p></w:p>"), "补格空段: {split}");
        // 补格结构：同开标签 + 空 tcPr 之外无 gridSpan
        let new_cells = split
            .split("<w:tr>")
            .nth(1)
            .unwrap()
            .split("</w:tr>")
            .next()
            .unwrap();
        assert_eq!(new_cells.matches("<w:tc>").count(), 2);
    }

    #[test]
    fn merge_split_error_families() {
        let xml = vmerged_doc();
        // span < 2
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::MergeCells {
                    block: 1,
                    expect_prefix: "头".into(),
                    direction: Some(MergeDirection::Vertical),
                    row: 1,
                    cell: 1,
                    span: Some(1),
                    end_row: None,
                    end_cell: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("合并跨度无效"), "实际: {err}");
        // 缺省 span=2 合法
        assert!(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::MergeCells {
                block: 1,
                expect_prefix: "头".into(),
                direction: Some(MergeDirection::Vertical),
                row: 1,
                cell: 1,
                span: None,
                end_row: None,
                end_cell: None,
            }],
        )
        .is_ok());
        // 纵并 onto 续格 → 指路合并头
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::MergeCells {
                    block: 1,
                    expect_prefix: "头".into(),
                    direction: Some(MergeDirection::Vertical),
                    row: 2,
                    cell: 1,
                    span: Some(2),
                    end_row: None,
                    end_cell: None,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("纵向合并续格"), "实际: {err}");
        // split vertical 指到续格 → 专属家族 + 指路合并头（比泛化的非合并格更可行动）
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SplitCell {
                    block: 1,
                    expect_prefix: "头".into(),
                    direction: MergeDirection::Vertical,
                    row: 2,
                    cell: 1,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("纵向合并续格"), "实际: {err}");
        assert!(err.contains("合并头"), "应指路合并头: {err}");
        // split horizontal 单格 → 非合并格
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::SplitCell {
                    block: 1,
                    expect_prefix: "头".into(),
                    direction: MergeDirection::Horizontal,
                    row: 1,
                    cell: 2,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("非合并格"), "实际: {err}");
        // 行越界
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[EditOp::MergeCells {
                    block: 1,
                    expect_prefix: "头".into(),
                    direction: Some(MergeDirection::Vertical),
                    row: 9,
                    cell: 1,
                    span: Some(2),
                    end_row: None,
                    end_cell: None,
                }],
            )
            .unwrap_err(),
        );
        // 行越界归并进跨度家族（首行前缀稳定 + 报行数事实）
        assert!(err.starts_with("合并跨度无效"), "实际: {err}");
        assert!(err.contains("越界"), "实际: {err}");
    }

    #[test]
    fn merge_split_exclusive_per_batch() {
        let xml = two_row_table_doc();
        // merge 同批挂 set_cell_text → 拒（结构重构独占）
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[
                    EditOp::SetCellText {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        row: 1,
                        cell: 1,
                        text: "x".into(),
                    },
                    EditOp::MergeCells {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        direction: Some(MergeDirection::Horizontal),
                        row: 1,
                        cell: 1,
                        span: Some(2),
                        end_row: None,
                        end_cell: None,
                    },
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
        assert!(
            err.contains("不能与内容/段落操作同批"),
            "应解释互斥边界: {err}"
        );
        // 反序：merge 在前，后挂表格操作 → 拒
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[
                    EditOp::MergeCells {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        direction: Some(MergeDirection::Horizontal),
                        row: 1,
                        cell: 1,
                        span: Some(2),
                        end_row: None,
                        end_cell: None,
                    },
                    EditOp::SetCellText {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        row: 1,
                        cell: 1,
                        text: "x".into(),
                    },
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
        assert!(err.contains("重新 inspect_docx"), "应指路重寻址: {err}");
    }

    // ---- 十一波·内容语义两条 + 表内地址模拟器（生产反馈坑①②③）----

    /// 格 XML：tcPr + 自定义段序列（构造多段/占位段形态）。
    fn cell_paras_xml(paras: &str) -> String {
        format!(r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr>{paras}</w:tc>"#)
    }

    /// 简单横并 op（同行 c1 起并 2 格）。
    fn hmerge_op(prefix: &str, row: usize) -> EditOp {
        EditOp::MergeCells {
            block: 1,
            expect_prefix: prefix.into(),
            direction: Some(MergeDirection::Horizontal),
            row,
            cell: 1,
            span: Some(2),
            end_row: None,
            end_cell: None,
        }
    }

    #[test]
    fn is_placeholder_p_classifies_content() {
        // 规则：占位段不是内容——无文字/图/域即占位
        assert!(is_placeholder_p("<w:p/>"));
        assert!(is_placeholder_p(
            r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr></w:p>"#
        ));
        assert!(
            is_placeholder_p(r#"<w:p><w:r><w:t>  </w:t></w:r></w:p>"#),
            "纯空白=占位"
        );
        assert!(
            is_placeholder_p(r#"<w:p><w:r><w:tab/></w:r></w:p>"#),
            "tab 前缀碰撞不误判"
        );
        assert!(!is_placeholder_p(r#"<w:p><w:r><w:t>字</w:t></w:r></w:p>"#));
        assert!(
            !is_placeholder_p(r#"<w:p><w:r><w:drawing/></w:r></w:p>"#),
            "图=内容"
        );
        assert!(
            !is_placeholder_p(r#"<w:p><w:fldSimple w:instr="TOC"/></w:p>"#),
            "域=内容"
        );
    }

    #[test]
    fn horizontal_merge_drops_placeholder_paragraphs() {
        // 坑①：横并搬运空段致段落膨胀（r5: 2→5）——占位段剔除 + 摘要披露
        let c1 = cell_paras_xml(r#"<w:p><w:r><w:t>甲一</w:t></w:r></w:p><w:p/>"#);
        let c2 = cell_paras_xml(
            r#"<w:p><w:r><w:t>乙一</w:t></w:r></w:p><w:p><w:pPr><w:jc w:val="center"/></w:pPr></w:p>"#,
        );
        let xml = wrap(&tbl_of(&[format!("<w:tr>{c1}{c2}</w:tr>")]));
        let (out, applied) =
            apply_edits(&xml, &Stylesheet::empty(), &[hmerge_op("甲一", 1)]).unwrap();
        assert_eq!(
            applied[0].after,
            "r1 c1 起横并 2 格（跨 2 列，内容按序拼接，丢弃 2 个占位段）"
        );
        // 2 内容段保留、2 占位段（含带格式的空段）不搬运
        assert_eq!(
            out.matches("<w:p>").count() + out.matches("<w:p/>").count(),
            2,
            "只留内容段"
        );
        assert!(out.contains("甲一") && out.contains("乙一"));
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows[0].cells.len(), 1);
        assert_eq!(t.rows[0].cells[0].grid_span, Some(2));
    }

    #[test]
    fn horizontal_merge_all_placeholder_falls_back_to_single_p() {
        // 全占位合并 → 聚合内容无段，补一个 <w:p/>（tc 至少 1 段合法性）
        let xml = wrap(&tbl_of(&[format!(
            "<w:tr>{}{}</w:tr>",
            cell_paras_xml("<w:p/>"),
            cell_paras_xml("<w:p/>")
        )]));
        let (out, applied) = apply_edits(&xml, &Stylesheet::empty(), &[hmerge_op("", 1)]).unwrap();
        assert!(applied[0].after.contains("丢弃 2 个占位段"));
        assert!(out.contains("<w:p/>"), "应补一个占位段");
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows[0].cells.len(), 1);
    }

    #[test]
    fn rect_merge_accumulates_drop_disclosure() {
        // 矩形区逐行横并的占位段剔除数累进披露
        let row = |a: &str, b: &str| {
            format!(
                "<w:tr>{}{}</w:tr>",
                cell_paras_xml(&format!(r#"<w:p><w:r><w:t>{a}</w:t></w:r></w:p><w:p/>"#)),
                cell_paras_xml(&format!(r#"<w:p><w:r><w:t>{b}</w:t></w:r></w:p><w:p/>"#))
            )
        };
        let xml = wrap(&tbl_of(&[row("甲一", "乙一"), row("甲二", "乙二")]));
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::MergeCells {
                block: 1,
                expect_prefix: "甲一".into(),
                direction: None,
                row: 1,
                cell: 1,
                span: None,
                end_row: Some(2),
                end_cell: Some(2),
            }],
        )
        .unwrap();
        assert_eq!(
            applied[0].after,
            "r1c1..r2c2 矩形合并（2 格宽 × 2 行，丢弃 4 个占位段）"
        );
        assert!(out.contains("甲二"), "纵并续格内容保留");
    }

    #[test]
    fn set_cell_text_positional_format_inheritance() {
        // 坑③：原格 3 段各异格式 + 新文 3 段 → 逐段按位继承，免跟批补 format
        let paras = concat!(
            r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>一</w:t></w:r></w:p>"#,
            r#"<w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t>二</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>三</w:t></w:r></w:p>"#,
        );
        let xml = wrap(&tbl_of(&[format!(
            "<w:tr>{}{}</w:tr>",
            cell_paras_xml(paras),
            cell_xml("乙")
        )]));
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetCellText {
                block: 1,
                expect_prefix: "一".into(),
                row: 1,
                cell: 1,
                text: "新A\n新B\n新C".into(),
            }],
        )
        .unwrap();
        assert_eq!(applied[0].after, "新A\n新B\n新C（格式按位继承）");
        assert!(out.contains(r#"<w:jc w:val="center"/>"#), "首段格式随位");
        assert!(
            out.contains(r#"<w:jc w:val="right"/>"#),
            "次段格式按位继承（回落模板则丢）"
        );
        assert!(out.contains("新A") && out.contains("新C"));
    }

    #[test]
    fn insert_table_row_after_inherits_formats_positionally() {
        // 增行填充段与模板格段数相等 → 按位继承；多段不对齐 → 行摘要披露回落
        let paras = concat!(
            r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>一</w:t></w:r></w:p>"#,
            r#"<w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t>二</w:t></w:r></w:p>"#,
        );
        let xml = wrap(&tbl_of(&[format!(
            "<w:tr>{}{}</w:tr>",
            cell_paras_xml(paras),
            cell_xml("乙")
        )]));
        // 对齐填充
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableRowAfter {
                block: 1,
                expect_prefix: "一".into(),
                after_row: Some(1),
                cells: Some(vec!["新A\n新B".into(), "丙".into()]),
            }],
        )
        .unwrap();
        assert_eq!(applied[0].after, "克隆第 1 行插入（2 格已填）");
        assert!(out.contains(r#"<w:jc w:val="right"/>"#), "克隆段按位继承");
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows.len(), 2);
        // 不对齐填充（3 段 vs 模板 2 段）→ 回落 + 披露
        let (_, applied2) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableRowAfter {
                block: 1,
                expect_prefix: "一".into(),
                after_row: Some(1),
                cells: Some(vec!["x\ny\nz".into(), "丙".into()]),
            }],
        )
        .unwrap();
        assert!(
            applied2[0]
                .after
                .contains("部分格段数不对齐，格式回落首段模板"),
            "实际: {}",
            applied2[0].after
        );
    }

    #[test]
    fn structural_ops_disjoint_rows_compose_in_one_batch() {
        // 坑②：13 行逐行横并被迫 13 次调用 → 不同行横并一批发完（足迹不相交）
        let xml = wrap(&tbl_of(&[
            row_xml("甲一", "乙一"),
            row_xml("甲二", "乙二"),
            row_xml("甲三", "乙三"),
        ]));
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[hmerge_op("甲一", 1), hmerge_op("甲一", 3)],
        )
        .unwrap();
        assert_eq!(applied.len(), 2, "两操作同批生效");
        assert_eq!(applied.iter().filter(|a| a.op == "merge_cells").count(), 2);
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.rows[0].cells.len(), 1, "r1 已并");
        assert_eq!(t.rows[0].cells[0].grid_span, Some(2));
        assert_eq!(t.rows[1].cells.len(), 2, "r2 不中枪");
        assert_eq!(t.rows[2].cells.len(), 1, "r3 已并");
    }

    #[test]
    fn structural_footprint_overlap_rejects_with_conflict_pair() {
        // 同行两横并 → 足迹相交，点名冲突对
        let xml = two_row_table_doc();
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[
                    hmerge_op("甲一", 1),
                    EditOp::MergeCells {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        direction: Some(MergeDirection::Horizontal),
                        row: 1,
                        cell: 2,
                        span: Some(2),
                        end_row: None,
                        end_cell: None,
                    },
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("结构足迹冲突"), "实际: {err}");
        assert!(
            err.contains("操作 1") && err.contains("操作 2"),
            "点名冲突对: {err}"
        );
    }

    #[test]
    fn structural_footprint_delete_row_semantics() {
        // 删行足迹 = 该行..末行（下方行号前移）：上方横并可组合，行内/下方拒
        let xml = wrap(&tbl_of(&[
            row_xml("甲一", "乙一"),
            row_xml("甲二", "乙二"),
            row_xml("甲三", "乙三"),
        ]));
        // 上方横并 + 删尾行：足迹 1 与 3..=3 不相交 → 过
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                hmerge_op("甲一", 1),
                EditOp::DeleteTableRow {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    row: 3,
                },
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 2);
        assert!(!out.contains("甲三"), "r3 已删");
        assert!(out.contains("甲二"), "r2 不中枪");
        // 删中间行 + 下方横并：足迹 2..=3 与 3 相交 → 拒
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[
                    EditOp::DeleteTableRow {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        row: 2,
                    },
                    hmerge_op("甲一", 3),
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("结构足迹冲突"), "实际: {err}");
        assert!(err.contains("delete_table_row"), "点名对方操作: {err}");
    }

    #[test]
    fn structural_footprint_split_chain_precise() {
        // 纵拆足迹精确到链尾：链外行可组合，链内行拒
        let head = r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>头</w:t></w:r></w:p></w:tc>"#;
        let cont = r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/><w:vMerge/></w:tcPr><w:p><w:r><w:t>藏一</w:t></w:r></w:p></w:tc>"#;
        let xml = wrap(&tbl_of(&[
            format!("<w:tr>{head}{}</w:tr>", cell_xml("右上")),
            format!("<w:tr>{cont}{}</w:tr>", cell_xml("右中")),
            row_xml("甲三", "乙三"),
        ]));
        let split = EditOp::SplitCell {
            block: 1,
            expect_prefix: "头".into(),
            direction: MergeDirection::Vertical,
            row: 1,
            cell: 1,
        };
        // 链外（r3）横并 + 拆链 → 足迹 1..=2 与 3 不相交 → 过
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[split.clone(), hmerge_op("头", 3)],
        )
        .unwrap();
        assert_eq!(applied.len(), 2);
        assert!(out.contains("藏一"), "续格内容恢复显示");
        // 链内（r2）横并 + 拆链 → 拒
        let err = val_msg(
            apply_edits(&xml, &Stylesheet::empty(), &[split, hmerge_op("头", 2)]).unwrap_err(),
        );
        assert!(err.starts_with("结构足迹冲突"), "实际: {err}");
        assert!(err.contains("纵并链拆分"), "点名链足迹: {err}");
    }

    #[test]
    fn table_format_ops_compose_in_one_batch() {
        // 同块：set_table_element（表级）+ set_cell_format + set_cell_text 按序组合
        let xml = two_row_table_doc();
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::SetTableElement {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    level: TableLevel::Table,
                    row: None,
                    cell: None,
                    element: "shd".into(),
                    xml: Some(SHD_FRAG.into()),
                },
                EditOp::SetCellFormat {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    row: 1,
                    cell: 1,
                    paragraph: None,
                    character: Some(CharFormat {
                        bold: Some(true),
                        ..Default::default()
                    }),
                    style: None,
                },
                EditOp::SetCellText {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    row: 2,
                    cell: 2,
                    text: "新乙二".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 3, "三操作同批生效");
        assert!(out.contains(SHD_FRAG), "表级 shd");
        assert!(out.contains("<w:b/>"), "格级加粗");
        assert!(out.contains("新乙二"), "改格文本");
        // 产物再解析健康（apply_edits 内部已有模型级 diff 校验，这里再锁块数）
        let m = model_of(&out);
        assert_eq!(m.body.len(), 1);
    }

    // ---- S3 七波·删行 + 批组合放宽（生产反馈 P0/P2）----

    /// 纵并表（显式 restart 头）：r1c1 头、r2c1 续、r2c2 普通——删守卫用。
    fn vchain_doc() -> String {
        let head = r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>头</w:t></w:r></w:p></w:tc>"#;
        let cont = r#"<w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/><w:vMerge/></w:tcPr><w:p><w:r><w:t>藏一</w:t></w:r></w:p></w:tc>"#;
        wrap(&tbl_of(&[
            format!("<w:tr>{}{}</w:tr>", head, cell_xml("右上")),
            format!("<w:tr>{}{}</w:tr>", cont, cell_xml("右中")),
            row_xml("甲三", "乙三"),
        ]))
    }

    #[test]
    fn delete_table_row_removes_row_keeps_rest() {
        let xml = wrap(&tbl_of(&[
            row_xml("甲一", "乙一"),
            row_xml("甲二", "乙二"),
            row_xml("甲三", "乙三"),
        ]));
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::DeleteTableRow {
                block: 1,
                expect_prefix: "甲一".into(),
                row: 2,
            }],
        )
        .unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].op, "delete_table_row");
        assert!(
            applied[0].after.contains("剩 2 行"),
            "摘要带剩余行数: {}",
            applied[0].after
        );
        assert!(!out.contains("甲二"), "目标行内容应删除");
        assert!(out.contains("甲一") && out.contains("甲三"), "其余行不中枪");
        // 块数不变（表仍是 1 块）、行数 3→2、tblGrid 原样
        let m = model_of(&out);
        assert_eq!(m.body.len(), 1);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(out.matches("<w:gridCol").count(), 2, "tblGrid 不动");
    }

    #[test]
    fn delete_table_row_guards() {
        // ① 末行保护：单行表删唯一行 → 拒、指路 delete_block
        let one = wrap(&tbl_of(&[row_xml("仅", "一")]));
        let err = val_msg(
            apply_edits(
                &one,
                &Stylesheet::empty(),
                &[EditOp::DeleteTableRow {
                    block: 1,
                    expect_prefix: "仅".into(),
                    row: 1,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("空表保护"), "实际: {err}");
        assert!(err.contains("delete_block"), "应指路整表删除: {err}");

        // ② 合并头行（下方有续格）→ 拒、指路 split_cell
        let err = val_msg(
            apply_edits(
                &vchain_doc(),
                &Stylesheet::empty(),
                &[EditOp::DeleteTableRow {
                    block: 1,
                    expect_prefix: "头".into(),
                    row: 1,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("合并结构冲突"), "实际: {err}");
        assert!(err.contains("split_cell"), "应指路先拆纵并: {err}");

        // ③ 纯续格行可删（头在上方，链条缩短仍合法）
        let (out, applied) = apply_edits(
            &vchain_doc(),
            &Stylesheet::empty(),
            &[EditOp::DeleteTableRow {
                block: 1,
                expect_prefix: "头".into(),
                row: 2,
            }],
        )
        .unwrap();
        assert_eq!(applied.len(), 1);
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows.len(), 2, "删续格行后剩 2 行");
        assert_eq!(
            t.rows[0].cells[0].v_merge.as_deref(),
            Some("restart"),
            "头保留"
        );
        assert!(!out.contains("藏一"), "续格内容随行删除");

        // ④ 独占一批：同批挂 set_cell_text → 拒
        let err = val_msg(
            apply_edits(
                &two_row_table_doc(),
                &Stylesheet::empty(),
                &[
                    EditOp::SetCellText {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        row: 1,
                        cell: 1,
                        text: "x".into(),
                    },
                    EditOp::DeleteTableRow {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        row: 2,
                    },
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
        assert!(
            err.contains("delete_table_row"),
            "应点名结构重构家族: {err}"
        );

        // ⑤ 行号越界
        let err = val_msg(
            apply_edits(
                &two_row_table_doc(),
                &Stylesheet::empty(),
                &[EditOp::DeleteTableRow {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    row: 5,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("行号越界"), "实际: {err}");
    }

    #[test]
    fn set_table_element_same_cell_different_elements_compose() {
        // 同格不同 element 一批组合（vAlign + tcBorders）——生产反馈「拆两批」修正
        let xml = two_row_table_doc();
        let borders =
            r#"<w:tcBorders><w:top w:val="single" w:sz="4" w:color="auto"/></w:tcBorders>"#;
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::SetTableElement {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    level: TableLevel::Cell,
                    row: Some(1),
                    cell: Some(1),
                    element: "vAlign".into(),
                    xml: Some(r#"<w:vAlign w:val="center"/>"#.into()),
                },
                EditOp::SetTableElement {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    level: TableLevel::Cell,
                    row: Some(1),
                    cell: Some(1),
                    element: "tcBorders".into(),
                    xml: Some(borders.into()),
                },
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 2);
        assert!(out.contains(r#"<w:vAlign w:val="center"/>"#), "vAlign 生效");
        assert!(out.contains("<w:tcBorders>"), "tcBorders 生效");
        // schema 序：tcBorders 应排在 vAlign 前（tcW → tcBorders → … → vAlign）
        let b = out.find("<w:tcBorders>").unwrap();
        let v = out.find(r#"<w:vAlign"#).unwrap();
        assert!(b < v, "schema 序 tcBorders < vAlign");
        // 邻格不中枪
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else {
            panic!()
        };
        assert_eq!(t.rows[0].cells[1].v_align, None, "右格不中枪");

        // 同元素两条 → 拒「同一格多操作」
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[
                    EditOp::SetTableElement {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        level: TableLevel::Cell,
                        row: Some(1),
                        cell: Some(1),
                        element: "vAlign".into(),
                        xml: Some(r#"<w:vAlign w:val="center"/>"#.into()),
                    },
                    EditOp::SetTableElement {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        level: TableLevel::Cell,
                        row: Some(1),
                        cell: Some(1),
                        element: "vAlign".into(),
                        xml: Some(r#"<w:vAlign w:val="bottom"/>"#.into()),
                    },
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("同一格多操作"), "实际: {err}");
        assert!(err.contains("vAlign"), "应点名冲突元素: {err}");

        // set_cell_text + set_table_element 同格（不同目标键）→ 合法组合
        let (out2, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::SetCellText {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    row: 1,
                    cell: 1,
                    text: "新甲一".into(),
                },
                EditOp::SetTableElement {
                    block: 1,
                    expect_prefix: "甲一".into(),
                    level: TableLevel::Cell,
                    row: Some(1),
                    cell: Some(1),
                    element: "vAlign".into(),
                    xml: Some(r#"<w:vAlign w:val="center"/>"#.into()),
                },
            ],
        )
        .unwrap();
        assert!(out2.contains("新甲一") && out2.contains(r#"<w:vAlign w:val="center"/>"#));

        // set_cell_text 同格两条仍拒（重写语义）
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[
                    EditOp::SetCellText {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        row: 1,
                        cell: 1,
                        text: "a".into(),
                    },
                    EditOp::SetCellText {
                        block: 1,
                        expect_prefix: "甲一".into(),
                        row: 1,
                        cell: 1,
                        text: "b".into(),
                    },
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("同一格多操作"), "实际: {err}");
    }

    #[test]
    fn insert_paragraph_after_same_anchor_chains_in_order() {
        let xml = wrap(r#"<w:p><w:r><w:t>锚</w:t></w:r></w:p>"#);
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::InsertParagraphAfter {
                    block: 1,
                    expect_prefix: "锚".into(),
                    text: "一".into(),
                    style: None,
                },
                EditOp::InsertParagraphAfter {
                    block: 1,
                    expect_prefix: "锚".into(),
                    text: "二".into(),
                    style: None,
                },
                EditOp::InsertParagraphAfter {
                    block: 1,
                    expect_prefix: "锚".into(),
                    text: "三".into(),
                    style: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 3, "逐操作摘要");
        assert_eq!(applied[0].op, "insert_paragraph_after");
        // 块数守恒：1 锚 + 3 新段
        let m = model_of(&out);
        assert_eq!(m.body.len(), 4);
        // 链序 = 输入序：锚 → 一 → 二 → 三
        let p0 = out.find("锚</w:t>").unwrap();
        let p1 = out.find("一</w:t>").unwrap();
        let p2 = out.find("二</w:t>").unwrap();
        let p3 = out.find("三</w:t>").unwrap();
        assert!(p0 < p1 && p1 < p2 && p2 < p3, "链序应为锚→一→二→三");

        // 异块组合仍合法：块 1 链式插 2 段 + 块 2 改文本，一批
        let xml2 =
            wrap(r#"<w:p><w:r><w:t>锚</w:t></w:r></w:p><w:p><w:r><w:t>他段</w:t></w:r></w:p>"#);
        let (out2, applied2) = apply_edits(
            &xml2,
            &Stylesheet::empty(),
            &[
                EditOp::InsertParagraphAfter {
                    block: 1,
                    expect_prefix: "锚".into(),
                    text: "甲".into(),
                    style: None,
                },
                EditOp::InsertParagraphAfter {
                    block: 1,
                    expect_prefix: "锚".into(),
                    text: "乙".into(),
                    style: None,
                },
                EditOp::ReplaceText {
                    block: 2,
                    expect_prefix: "他段".into(),
                    new_text: "改段".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(applied2.len(), 3);
        assert!(out2.contains("改段"));
        let m2 = model_of(&out2);
        assert_eq!(m2.body.len(), 4);

        // 同锚混其他段落操作仍拒（链式例外只对 insert_paragraph_after 开放）
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[
                    EditOp::InsertParagraphAfter {
                        block: 1,
                        expect_prefix: "锚".into(),
                        text: "一".into(),
                        style: None,
                    },
                    EditOp::ReplaceText {
                        block: 1,
                        expect_prefix: "锚".into(),
                        new_text: "改".into(),
                    },
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
        // 反序：replace 在前，insert 在后 → 同样拒
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[
                    EditOp::ReplaceText {
                        block: 1,
                        expect_prefix: "锚".into(),
                        new_text: "改".into(),
                    },
                    EditOp::InsertParagraphAfter {
                        block: 1,
                        expect_prefix: "锚".into(),
                        text: "一".into(),
                        style: None,
                    },
                ],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
    }

    // =====================================================================
    // D18 十波引擎 e2e——包级增补全链（apply_edits_to_bytes 直调；内置 report
    // 模板无 media / 无 settings.xml / document rels 仅 rId1 → 全部走「新建」路径）
    // =====================================================================

    /// zip 原字节读取（read_entry 是 lossy String，二进制 media 件不适用）
    fn zip_read(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
        let mut a = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
        let out = match a.by_name(name) {
            Ok(mut f) => {
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).ok()?;
                Some(buf)
            }
            Err(_) => None,
        };
        out
    }

    fn zip_entry_names(bytes: &[u8]) -> Vec<String> {
        let a = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        a.file_names().map(str::to_string).collect()
    }

    /// 图片载荷辅助：引擎不重新探测（probe 在工具壳 load_image），字节任意可
    fn img_payload(w: u32, h: u32) -> ImagePayload {
        ImagePayload {
            bytes: b"\x89PNG\r\n\x1a\n-test-bytes".to_vec(),
            width_px: w,
            height_px: h,
            ext: "png",
        }
    }

    /// 占位字段填零的插图 op（rid/extent/docpr 由编排层注入覆盖——唯一写入点）
    fn image_op(block: usize, expect: &str, w: u32, h: u32) -> EditOp {
        EditOp::InsertImageAfter {
            block,
            expect_prefix: expect.into(),
            image: img_payload(w, h),
            width_mm: None,
            rid: String::new(),
            cx_emu: 0,
            cy_emu: 0,
            docpr_id: 0,
        }
    }

    /// 两块测试文档（标题 + 正文）——write_docx 全链产物，块 1 = 标题甲
    fn two_block_doc() -> Vec<u8> {
        let tpl = super::super::docx_write::build_builtin_template("report").unwrap();
        super::super::docx_write::generate_from_template(
            &tpl,
            &[
                super::super::docx_write::WriteBlock::Heading {
                    level: 1,
                    text: "标题甲".into(),
                },
                super::super::docx_write::WriteBlock::Paragraph {
                    text: "正文乙".into(),
                    style: None,
                },
            ],
        )
        .unwrap()
        .bytes
    }

    fn document_xml_of(bytes: &[u8]) -> String {
        zip_read(bytes, "word/document.xml").unwrap().lossy_utf8()
    }

    /// Vec<u8> → String（测试辅助；document.xml 是 UTF-8）
    trait LossyUtf8 {
        fn lossy_utf8(&self) -> String;
    }
    impl LossyUtf8 for Vec<u8> {
        fn lossy_utf8(&self) -> String {
            String::from_utf8_lossy(self).into_owned()
        }
    }

    #[test]
    fn engine_insert_image_after_e2e() {
        let doc = two_block_doc();
        let before_names = zip_entry_names(&doc);
        let (out, applied) = apply_edits_to_bytes(&doc, &[image_op(1, "标题甲", 100, 50)]).unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].op, "insert_image_after");

        // media 新部件：字节与载荷全等
        let media = zip_read(&out, "word/media/image1.png").expect("media/image1.png 应存在");
        assert_eq!(media, b"\x89PNG\r\n\x1a\n-test-bytes");

        // rels：原 rId1（styles）条目字节不动 + 新增 image 关系 rId2
        let rels = zip_read(&out, "word/_rels/document.xml.rels")
            .unwrap()
            .lossy_utf8();
        assert!(rels.contains(r#"Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml""#));
        assert!(rels.contains(r#"Id="rId2""#), "新图应分到 rId2: {rels}");
        assert!(rels.contains(r#"Target="media/image1.png""#));

        // CT：png Default 补上；document.xml：drawing 引用 rId2
        let ct = zip_read(&out, "[Content_Types].xml").unwrap().lossy_utf8();
        assert!(
            ct.contains(r#"<Default Extension="png""#),
            "CT 应补 png Default: {ct}"
        );
        let xml = document_xml_of(&out);
        assert!(xml.contains(r#"r:embed="rId2""#), "a:blip 应引 rId2");
        // wp:extent 默认宽 = 100px×9525 EMU（原生 < 版心 → 不放大）
        assert!(xml.contains(r#"<wp:extent cx="952500" cy="476250"/>"#));

        // 块数 +1（引擎自检已断言；此处复核模型投影）
        let model2 = model_of(&xml);
        assert_eq!(model2.body.len(), 3);
        let Block::Paragraph(p2) = &model2.body[1] else {
            panic!("新块应为段落")
        };
        assert_eq!(p2.image_count, 1, "读侧投影应见图");
        assert_eq!(
            p2.runs.iter().map(|r| r.text.as_str()).collect::<String>(),
            "",
            "图段无文本"
        );

        // untouched entry 逐字节保真（变更白名单外全部原样）
        let changed = [
            "word/document.xml",
            "word/_rels/document.xml.rels",
            "[Content_Types].xml",
        ];
        let after_names = zip_entry_names(&out);
        for name in &before_names {
            if changed.contains(&name.as_str()) {
                continue;
            }
            assert_eq!(
                zip_read(&out, name),
                zip_read(&doc, name),
                "{name} 应逐字节不变"
            );
        }
        assert!(after_names.iter().any(|n| n == "word/media/image1.png"));
    }

    #[test]
    fn engine_insert_two_images_rid_increment() {
        let doc = two_block_doc();
        let (out, applied) = apply_edits_to_bytes(
            &doc,
            &[
                image_op(1, "标题甲", 80, 60),
                image_op(1, "标题甲", 200, 100),
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 2);

        // media 双部件 + rId 递增不撞
        assert!(zip_read(&out, "word/media/image1.png").is_some());
        assert!(zip_read(&out, "word/media/image2.png").is_some());
        let rels = zip_read(&out, "word/_rels/document.xml.rels")
            .unwrap()
            .lossy_utf8();
        assert!(rels.contains(r#"Target="media/image1.png""#));
        assert!(rels.contains(r#"Target="media/image2.png""#));
        let xml = document_xml_of(&out);
        assert!(xml.contains(r#"r:embed="rId2""#));
        assert!(xml.contains(r#"r:embed="rId3""#));

        // docPr id 唯一递增（模板无既有 drawing → 1 / 2）
        assert!(xml.contains(r#"<wp:docPr id="1""#));
        assert!(xml.contains(r#"<wp:docPr id="2""#));

        // 同锚链式：两图都在块 1 之后、按输入序（先 80×60 后 200×100）
        let pos1 = xml.find(r#"r:embed="rId2""#).unwrap();
        let pos2 = xml.find(r#"r:embed="rId3""#).unwrap();
        assert!(pos1 < pos2, "链序 = 输入序");
        let model2 = model_of(&xml);
        assert_eq!(model2.body.len(), 4);
    }

    #[test]
    fn engine_insert_toc_after_e2e() {
        let doc = two_block_doc();
        let (out, applied) = apply_edits_to_bytes(
            &doc,
            &[EditOp::InsertTocAfter {
                block: 1,
                expect_prefix: "标题甲".into(),
                levels: 3,
                hyperlink: true,
            }],
        )
        .unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].op, "insert_toc_after");

        // 裸 fldSimple 单段 + 指令 + 自愈文案
        let xml = document_xml_of(&out);
        assert!(xml.contains("<w:fldSimple"), "TOC 应是 fldSimple 形态");
        assert!(xml.contains("TOC \\o"), "指令应含 TOC 域开关");
        assert!(
            xml.contains("目录将在打开文档时自动生成"),
            "cached result 放自愈文案"
        );

        // 模板无 settings.xml → 新建最小件 + updateFields + CT Override
        let settings = zip_read(&out, "word/settings.xml").expect("settings.xml 应新建");
        let settings = settings.lossy_utf8();
        assert!(
            settings.contains(r#"<w:updateFields w:val="true"/>"#),
            "updateFields 应置位: {settings}"
        );
        let ct = zip_read(&out, "[Content_Types].xml").unwrap().lossy_utf8();
        assert!(
            ct.contains(r#"PartName="/word/settings.xml""#),
            "CT 应补 settings Override"
        );

        // 读侧：域指令可见 + 块数 +1
        let model2 = model_of(&xml);
        assert_eq!(model2.body.len(), 3);
        let Block::Paragraph(p2) = &model2.body[1] else {
            panic!()
        };
        assert!(
            p2.field_instrs.iter().any(|i| i.contains("TOC")),
            "投影应收域指令: {:?}",
            p2.field_instrs
        );

        // 再插一次 TOC（第二个会话常见）——settings 幂等（已置位不重写、不重复条目）
        let (out2, _) = apply_edits_to_bytes(
            &out,
            &[EditOp::InsertTocAfter {
                block: 1,
                expect_prefix: "标题甲".into(),
                levels: 2,
                hyperlink: false,
            }],
        )
        .unwrap();
        let s2 = zip_read(&out2, "word/settings.xml").unwrap().lossy_utf8();
        assert_eq!(s2.matches("<w:updateFields").count(), 1, "幂等不重复: {s2}");
    }

    #[test]
    fn engine_chain_paragraph_toc_image_order() {
        let doc = two_block_doc();
        // 同锚三连：段 → TOC → 图，一批按输入序
        let (out, applied) = apply_edits_to_bytes(
            &doc,
            &[
                EditOp::InsertParagraphAfter {
                    block: 1,
                    expect_prefix: "标题甲".into(),
                    text: "链一".into(),
                    style: None,
                },
                EditOp::InsertTocAfter {
                    block: 1,
                    expect_prefix: "标题甲".into(),
                    levels: 2,
                    hyperlink: false,
                },
                image_op(1, "标题甲", 50, 50),
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 3);
        let xml = document_xml_of(&out);
        let p_pos = xml.find("链一").expect("链一段应存在");
        let t_pos = xml.find("<w:fldSimple").expect("TOC 段应存在");
        let i_pos = xml.find("r:embed=").expect("图段应存在");
        assert!(
            p_pos < t_pos && t_pos < i_pos,
            "同锚链序 = 输入序（段→TOC→图）"
        );
        let model2 = model_of(&xml);
        assert_eq!(model2.body.len(), 5);
    }

    #[test]
    fn engine_toc_levels_out_of_range_rejected() {
        let doc = two_block_doc();
        let err = val_msg(
            apply_edits_to_bytes(
                &doc,
                &[EditOp::InsertTocAfter {
                    block: 1,
                    expect_prefix: "标题甲".into(),
                    levels: 0,
                    hyperlink: true,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("levels=0 越界"), "实际: {err}");
        let err = val_msg(
            apply_edits_to_bytes(
                &doc,
                &[EditOp::InsertTocAfter {
                    block: 1,
                    expect_prefix: "标题甲".into(),
                    levels: 10,
                    hyperlink: true,
                }],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("levels=10 越界"), "实际: {err}");
    }

    #[test]
    fn engine_image_width_mm_clamped_and_ratio_kept() {
        let doc = two_block_doc();
        // 版心宽 8306 twips = 5274130 EMU（A4 - 左右 1800×2 页边）。
        // 显式 200mm = 7,200,000 EMU > 版心 → 钳到版心；高等比 1:2 → 版心/2
        let (out, _) = apply_edits_to_bytes(
            &doc,
            &[EditOp::InsertImageAfter {
                block: 1,
                expect_prefix: "标题甲".into(),
                image: img_payload(1000, 500),
                width_mm: Some(200.0),
                rid: String::new(),
                cx_emu: 0,
                cy_emu: 0,
                docpr_id: 0,
            }],
        )
        .unwrap();
        let xml = document_xml_of(&out);
        // 版心 = (11906 - 1800 - 1800) twips × 635 = 8306 × 635 = 5274310 EMU
        assert!(
            xml.contains(&format!(r#"cx="{}""#, 8306 * 635)),
            "宽应钳到版心"
        );
        assert!(
            xml.contains(&format!(r#"cy="{}""#, 8306 * 635 / 2)),
            "高等比 1:2"
        );

        // 原生像素宽 100px（952500 EMU）< 版心 → 不放大，width_mm 缺省用原生
        let (out2, _) = apply_edits_to_bytes(&doc, &[image_op(1, "标题甲", 100, 200)]).unwrap();
        let xml2 = document_xml_of(&out2);
        assert!(xml2.contains(r#"cx="952500""#), "小图不放大");
        assert!(xml2.contains(r#"cy="1905000""#), "高等比 1:2");
    }
