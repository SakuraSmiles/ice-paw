use super::*;

/// 用 docx-rs 造真实包跑 inspect（zip + document.xml + styles.xml 全链路）。
fn docx_bytes() -> Vec<u8> {
    use docx_rs::{Document, Docx, Paragraph, Run};
    let document = Document::new()
        .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第一段正文")))
        .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第二段")));
    let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
    Docx::new()
        .document(document)
        .build()
        .pack(&mut cursor)
        .unwrap();
    cursor.into_inner()
}

#[test]
fn outline_text_format_on_real_package() {
    let bytes = docx_bytes();
    let report = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Outline,
            start: None,
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert_eq!(report.total_blocks, 2);
    assert_eq!(report.range, (1, 2));
    assert!(!report.has_more);
    assert!(
        report.content.contains("[1] ¶"),
        "outline 应有块 1: {}",
        report.content
    );
    assert!(report.content.contains("第一段正文"));

    let text = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Text,
            start: None,
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(text.content.contains("[2] 第二段"));

    let fmt = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Format,
            start: None,
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(
        fmt.content.contains("文本:"),
        "format 应有文本行: {}",
        fmt.content
    );
}

#[test]
fn range_validation_errors_and_clamps() {
    let bytes = docx_bytes();
    // start 越界 → 三段式报错
    let err = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Outline,
            start: Some(5),
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("块号越界"), "实际: {err}");
    // end < start → 报错
    let err = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Outline,
            start: Some(2),
            end: Some(1),
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("区间无效"), "实际: {err}");
    // end 超总 → clamp 到 2
    let report = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Outline,
            start: Some(1),
            end: Some(99),
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
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
            w.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(data.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }
    buf
}

#[test]
fn headers_footers_projection_renders_parts() {
    let bytes = hf_docx_bytes();
    let report = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::HeadersFooters,
            start: None,
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(
        report.content.contains("2 节"),
        "节计数: {}",
        report.content
    );
    assert_eq!(report.range, (1, 2), "range = 节区间");
    assert!(!report.has_more);
    // 部件内容 + 多行续行
    assert!(
        report
            .content
            .contains("页眉 default: 页眉甲行\n    页眉乙行"),
        "{}",
        report.content
    );
    assert!(
        report.content.contains("页脚 default: 页脚一行"),
        "{}",
        report.content
    );
    // 同部件多节引用（rIdH 两节）经缓存呈现一致
    assert_eq!(report.content.matches("页眉甲行").count(), 2);
    // rels 悬空引用诚实呈现（rIdMISSING 无对应 Relationship）
    assert!(report.content.contains("悬空"), "{}", report.content);
    // 图片关系（http://x/image）不进页眉脚表
    assert!(!report.content.contains("media/a.png"));

    // start/end 不适用 → 家族报错
    let err = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::HeadersFooters,
            start: Some(1),
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("不接受 start/end"), "实际: {err}");
}

/// 带合并格/空格的最小表格包。
fn tbl_docx_bytes() -> Vec<u8> {
    use std::io::Write;
    let doc_xml = concat!(
        r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>"#,
        r#"<w:p><w:r><w:t>引言段</w:t></w:r></w:p>"#,
        // 表：r1 整行跨 2 列；r2 = 续格 + 空格
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="100"/><w:gridCol w:w="100"/></w:tblGrid>"#,
        r#"<w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>总览</w:t></w:r></w:p></w:tc></w:tr>"#,
        r#"<w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc><w:tcPr/><w:p/></w:tc></w:tr>"#,
        r#"</w:tbl></w:body></w:document>"#,
    );
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        w.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        w.write_all(doc_xml.as_bytes()).unwrap();
        w.finish().unwrap();
    }
    buf
}

#[test]
fn table_projection_renders_grid_with_markers() {
    let bytes = tbl_docx_bytes();
    let report = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Table,
            start: None,
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    // 头行：真列数（跨列行 gridSpan 求和，非格数）
    assert!(
        report.content.contains("[2] ▦ 表 2行×2列"),
        "{}",
        report.content
    );
    // 跨列 / 续 / 空标注
    assert!(
        report.content.contains("r1: 总览(跨2列)"),
        "{}",
        report.content
    );
    assert!(report.content.contains("(续)"), "{}", report.content);
    assert!(report.content.contains("(空)"), "{}", report.content);
    // 段落块不展开，但计数诚实
    assert!(
        report.content.contains("1 个段落块未显示"),
        "{}",
        report.content
    );

    // 无表格区间 → 指路（勿盲试）
    let only_para = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Table,
            start: Some(1),
            end: Some(1),
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(
        only_para.content.contains("无表格块"),
        "{}",
        only_para.content
    );
    assert!(
        only_para.content.contains("outline"),
        "应指向 outline: {}",
        only_para.content
    );
}

/// ppr 下钻夹具（六波·缺口 3）：格内两段——首段带 ind chars 变体（生产形态：
/// 正文样式首行缩进透进表格格）、次段裸段；邻格自闭合空段。
fn ppr_tbl_docx_bytes() -> Vec<u8> {
    use std::io::Write;
    let doc_xml = concat!(
        r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>"#,
        r#"<w:p><w:r><w:t>引言段</w:t></w:r></w:p>"#,
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="100"/><w:gridCol w:w="100"/></w:tblGrid>"#,
        r#"<w:tr><w:tc><w:tcPr/>"#,
        r#"<w:p><w:pPr><w:ind w:firstLine="480" w:firstLineChars="200"/></w:pPr><w:r><w:t>甲</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>乙</w:t></w:r></w:p></w:tc>"#,
        r#"<w:tc><w:tcPr/><w:p/></w:tc></w:tr></w:tbl>"#,
        r#"</w:body></w:document>"#,
    );
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        w.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        w.write_all(doc_xml.as_bytes()).unwrap();
        w.finish().unwrap();
    }
    buf
}

#[test]
fn ppr_projection_drills_into_cell_paragraphs() {
    let bytes = ppr_tbl_docx_bytes();
    // 默认：段落块照常 + 表格块指路下钻（不再一句「无段落属性」了事）
    let report = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Ppr,
            start: None,
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(
        report.content.contains("#1\t(无 pPr)"),
        "{}",
        report.content
    );
    assert!(
        report
            .content
            .contains("表格块——格内段落 pPr 用 row+cell 下钻"),
        "{}",
        report.content
    );
    // row+cell：格内逐段 pPr 原文
    let drill = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Ppr,
            start: Some(2),
            end: Some(2),
            row: Some(1),
            cell: Some(1),
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(drill.content.contains("[2] r1c1:"), "{}", drill.content);
    assert!(
        drill
            .content
            .contains(r#"  p1: <w:pPr><w:ind w:firstLine="480" w:firstLineChars="200"/></w:pPr>"#),
        "首段 pPr 原文: {}",
        drill.content
    );
    assert!(drill.content.contains("p2: (无 pPr)"), "{}", drill.content);
    // 邻格：自闭合空段 → (无 pPr)
    let drill2 = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Ppr,
            start: Some(2),
            end: Some(2),
            row: Some(1),
            cell: Some(2),
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(drill2.content.contains("[2] r1c2:"), "{}", drill2.content);
    assert!(
        drill2.content.contains("p1: (无 pPr)"),
        "{}",
        drill2.content
    );
    // row 缺 cell：行级没有 pPr，家族报错
    let err = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Ppr,
            start: Some(2),
            end: Some(2),
            row: Some(1),
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("需搭配 cell"), "实际: {err}");
    // 格越界三段式
    let err = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Ppr,
            start: Some(2),
            end: Some(2),
            row: Some(1),
            cell: Some(9),
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("单元格越界"), "实际: {err}");
    // 区间混入段落块 + row 设定 → 与 tblpr 同判
    let err = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Ppr,
            start: None,
            end: None,
            row: Some(1),
            cell: Some(1),
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("非表格块"), "实际: {err}");
    // 其他投影带 row 仍拒（守卫没被放宽误伤）
    let err = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Outline,
            start: None,
            end: None,
            row: Some(1),
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("参数不适用"), "实际: {err}");
}

/// 带格式特征的表（S3 四波①）：表级 shd/边框/宽度 + 格级 shd/vAlign/tcBorders。
fn fmt_tbl_docx_bytes() -> Vec<u8> {
    use std::io::Write;
    let doc_xml = concat!(
        r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>"#,
        r#"<w:tbl><w:tblPr><w:tblStyle w:val="T1"/><w:tblW w:w="5000" w:type="pct"/>"#,
        r#"<w:tblBorders><w:top w:val="single" w:sz="4"/></w:tblBorders>"#,
        r#"<w:shd w:val="clear" w:color="auto" w:fill="F2F2F2"/></w:tblPr>"#,
        r#"<w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid>"#,
        r#"<w:tr><w:tc><w:tcPr><w:shd w:val="clear" w:fill="DDEEFF"/><w:vAlign w:val="center"/>"#,
        r#"</w:tcPr><w:p><w:r><w:t>表头甲</w:t></w:r></w:p></w:tc>"#,
        r#"<w:tc><w:tcPr><w:tcBorders><w:top w:val="double" w:sz="4"/></w:tcBorders>"#,
        r#"</w:tcPr><w:p><w:r><w:t>表头乙</w:t></w:r></w:p></w:tc></w:tr>"#,
        r#"<w:tr><w:tc><w:tcPr/><w:p><w:r><w:t>数据</w:t></w:r></w:p></w:tc>"#,
        r#"<w:tc><w:tcPr/><w:p/></w:tc></w:tr></w:tbl>"#,
        r#"</w:body></w:document>"#,
    );
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        w.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        w.write_all(doc_xml.as_bytes()).unwrap();
        w.finish().unwrap();
    }
    buf
}

#[test]
fn table_projection_shows_format_markers() {
    let bytes = fmt_tbl_docx_bytes();
    let report = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Table,
            start: None,
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    // 表属性摘要行
    assert!(report.content.contains("表属性:"), "{}", report.content);
    assert!(report.content.contains("样式=T1"), "{}", report.content);
    assert!(report.content.contains("底纹#F2F2F2"), "{}", report.content);
    assert!(report.content.contains("边框=自定义"), "{}", report.content);
    assert!(report.content.contains("宽=5000 pct"), "{}", report.content);
    // 格级标注
    assert!(
        report.content.contains("表头甲(底纹#DDEEFF)(垂直=center)"),
        "{}",
        report.content
    );
    assert!(
        report.content.contains("表头乙(自定边框)"),
        "{}",
        report.content
    );
    // 无特征的格不挂标注（「数据」格无括号尾巴）
    assert!(
        report.content.contains("r2: 数据 | (空)\n"),
        "{}",
        report.content
    );
}

#[test]
fn tblpr_projection_drills_three_levels() {
    let bytes = fmt_tbl_docx_bytes();
    // 表级
    let report = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Tblpr,
            start: None,
            end: None,
            row: None,
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(
        report
            .content
            .contains("[1] tblPr: <w:tblPr><w:tblStyle w:val=\"T1\"/>"),
        "{}",
        report.content
    );
    assert!(
        report.content.contains("</w:tblPr>"),
        "原文完整: {}",
        report.content
    );
    // 行级
    let report = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Tblpr,
            start: None,
            end: None,
            row: Some(1),
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(
        report.content.contains("[1] r1 trPr: (无 trPr)"),
        "{}",
        report.content
    );
    // 格级
    let report = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Tblpr,
            start: None,
            end: None,
            row: Some(1),
            cell: Some(1),
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap();
    assert!(
        report
            .content
            .contains("[1] r1c1 tcPr: <w:tcPr><w:shd w:val=\"clear\" w:fill=\"DDEEFF\"/>"),
        "{}",
        report.content
    );
    assert!(
        report.content.contains("<w:vAlign w:val=\"center\"/>"),
        "{}",
        report.content
    );
}

#[test]
fn tblpr_projection_error_families() {
    let bytes = fmt_tbl_docx_bytes();
    // 行越界
    let err = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Tblpr,
            start: None,
            end: None,
            row: Some(9),
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("行号越界"), "实际: {err}");
    // 格越界
    let err = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Tblpr,
            start: None,
            end: None,
            row: Some(1),
            cell: Some(9),
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("单元格越界"), "实际: {err}");
    assert!(err.contains("projection=table"), "应指路网格视图: {err}");
    // cell 不带 row
    let err = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Tblpr,
            start: None,
            end: None,
            row: None,
            cell: Some(1),
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("参数校验失败"), "实际: {err}");
    // row/cell 误传给其他投影
    let err = inspect_document(
        &bytes,
        &InspectRequest {
            projection: InspectProjection::Text,
            start: None,
            end: None,
            row: Some(1),
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("参数不适用"), "实际: {err}");
    // row 指到段落块区间（fmt fixture 只有表——换带前导段的 tbl_docx_bytes，表=块 2）
    let err = inspect_document(
        &tbl_docx_bytes(),
        &InspectRequest {
            projection: InspectProjection::Tblpr,
            start: Some(1),
            end: Some(1),
            row: Some(1),
            cell: None,
            style: None,
            num_id: None,
            level: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("非表格块"), "实际: {err}");
}

// ------------------------------------------------------------------
// 定义三投影（D12）：styles / styledef / numbering
// ------------------------------------------------------------------

/// 带样式表 + 编号部件 + 正文编号引用的最小包（定义三投影共用）。
fn def_docx_bytes() -> Vec<u8> {
    use std::io::Write;
    let doc_xml = concat!(
        r#"<w:document xmlns:w="w"><w:body>"#,
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="21"/></w:numPr></w:pPr><w:r><w:t>条目一</w:t></w:r></w:p>"#,
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="21"/></w:numPr></w:pPr><w:r><w:t>子项甲</w:t></w:r></w:p>"#,
        r#"</w:body></w:document>"#,
    );
    let styles_xml = concat!(
        r#"<w:styles xmlns:w="w"><w:docDefaults><w:rPrDefault><w:rPr/></w:rPrDefault></w:docDefaults>"#,
        r#"<w:style w:type="paragraph" w:styleId="1"><w:name w:val="Normal"/></w:style>"#,
        r#"<w:style w:type="paragraph" w:styleId="2"><w:name w:val="heading 1"/><w:basedOn w:val="1"/>"#,
        r#"<w:pPr><w:spacing w:line="360"/><w:outlineLvl w:val="0"/></w:pPr>"#,
        r#"<w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>"#,
        r#"<w:style w:type="table" w:styleId="T1"><w:name w:val="Grid Table"/><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr></w:style>"#,
        r#"</w:styles>"#,
    );
    let bullet_glyph = "\u{F0B7}"; // Wingdings 私有区（bullet 净化断言用）
    let numbering_xml = format!(
        concat!(
            r#"<w:numbering xmlns:w="w">"#,
            r#"<w:abstractNum w:abstractNumId="7">"#,
            r#"<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/></w:lvl>"#,
            r#"<w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2)"/><w:lvlJc w:val="left"/></w:lvl>"#,
            r#"</w:abstractNum>"#,
            r#"<w:abstractNum w:abstractNumId="8">"#,
            r#"<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="{bullet}"/><w:lvlJc w:val="left"/></w:lvl>"#,
            r#"</w:abstractNum>"#,
            r#"<w:num w:numId="21"><w:abstractNumId w:val="7"/></w:num>"#,
            r#"<w:num w:numId="33"><w:abstractNumId w:val="7"/></w:num>"#,
            r#"<w:num w:numId="40"><w:abstractNumId w:val="8"/></w:num>"#,
            r#"</w:numbering>"#,
        ),
        bullet = bullet_glyph,
    );
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        for (name, data) in [
            ("word/document.xml", doc_xml),
            ("word/styles.xml", styles_xml),
            ("word/numbering.xml", numbering_xml.as_str()),
        ] {
            w.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(data.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }
    buf
}

fn def_req(projection: InspectProjection) -> InspectRequest {
    InspectRequest {
        projection,
        start: None,
        end: None,
        row: None,
        cell: None,
        style: None,
        num_id: None,
        level: None,
    }
}

#[test]
fn styles_projection_lists_chain_and_features() {
    let bytes = def_docx_bytes();
    let report = inspect_document(&bytes, &def_req(InspectProjection::Styles)).unwrap();
    assert!(report.content.contains("3 个样式"), "{}", report.content);
    assert_eq!(report.range, (1, 3));
    assert!(!report.has_more);
    // 继承链 + 大纲级 + 自带字符格式
    assert!(
        report
            .content
            .contains("ID=2 | heading 1 | paragraph | ←Normal | H1 b sz=32"),
        "{}",
        report.content
    );
    // 表样式的 type 与空特征
    assert!(
        report
            .content
            .contains("ID=T1 | Grid Table | table | (无父) | -"),
        "{}",
        report.content
    );
    // start/end 不适用
    let mut req = def_req(InspectProjection::Styles);
    req.start = Some(1);
    let err = inspect_document(&bytes, &req).unwrap_err().to_string();
    assert!(err.contains("不接受 start/end"), "实际: {err}");
}

#[test]
fn styledef_projection_returns_raw_and_rejects() {
    let bytes = def_docx_bytes();
    // 按显示名下钻 → 原文整段
    let mut req = def_req(InspectProjection::Styledef);
    req.style = Some("heading 1".into());
    let report = inspect_document(&bytes, &req).unwrap();
    assert!(
        report.content.contains("样式 heading 1（ID 2）定义原文"),
        "{}",
        report.content
    );
    assert!(
        report
            .content
            .contains(r#"<w:style w:type="paragraph" w:styleId="2">"#),
        "应含原文开标签: {}",
        report.content
    );
    assert!(
        report.content.contains(r#"<w:outlineLvl w:val="0"/>"#),
        "{}",
        report.content
    );
    assert!(report.content.contains("</w:style>"), "原文完整");
    // 按 ID 下钻同样命中
    let mut req = def_req(InspectProjection::Styledef);
    req.style = Some("T1".into());
    let report = inspect_document(&bytes, &req).unwrap();
    assert!(report.content.contains("Grid Table"), "{}", report.content);

    // 未知名 → 家族报错 + 指路清单
    let mut req = def_req(InspectProjection::Styledef);
    req.style = Some("不存在样式".into());
    let err = inspect_document(&bytes, &req).unwrap_err().to_string();
    assert!(err.contains("样式不存在"), "实际: {err}");
    assert!(err.contains("projection=styles"), "应指路清单: {err}");
    // 缺 style 参数
    let err = inspect_document(&bytes, &def_req(InspectProjection::Styledef))
        .unwrap_err()
        .to_string();
    assert!(err.contains("参数校验失败"), "实际: {err}");
    // 无样式部件（手造包无 styles.xml）→ 诚实提示非报错（style 参数照给——
    // 参数缺失是请求侧错误，须先于部件检查报出）
    let mut req = def_req(InspectProjection::Styledef);
    req.style = Some("任意".into());
    let report = inspect_document(&tbl_docx_bytes(), &req).unwrap();
    assert!(report.content.contains("无样式部件"), "{}", report.content);
}

#[test]
fn styledef_rejects_duplicate_display_names() {
    use std::io::Write;
    let styles_xml = concat!(
        r#"<w:styles xmlns:w="w">"#,
        r#"<w:style w:type="paragraph" w:styleId="a1"><w:name w:val="同名"/></w:style>"#,
        r#"<w:style w:type="paragraph" w:styleId="b2"><w:name w:val="同名"/></w:style>"#,
        r#"</w:styles>"#,
    );
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        w.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        w.write_all(br#"<w:document xmlns:w="w"/>"#).unwrap();
        w.start_file("word/styles.xml", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(styles_xml.as_bytes()).unwrap();
        w.finish().unwrap();
    }
    let mut req = def_req(InspectProjection::Styledef);
    req.style = Some("同名".into());
    let err = inspect_document(&buf, &req).unwrap_err().to_string();
    assert!(err.contains("样式名重复"), "实际: {err}");
    assert!(
        err.contains("a1") && err.contains("b2"),
        "应列全部 ID: {err}"
    );
    // ID 寻址不受重名影响
    let mut req = def_req(InspectProjection::Styledef);
    req.style = Some("b2".into());
    let report = inspect_document(&buf, &req).unwrap();
    assert!(
        report.content.contains(r#"w:styleId="b2""#),
        "{}",
        report.content
    );
}

#[test]
fn numbering_projection_sections_and_drill() {
    let bytes = def_docx_bytes();
    let report = inspect_document(&bytes, &def_req(InspectProjection::Numbering)).unwrap();
    assert!(report.content.contains("3 个 numId"), "{}", report.content);
    // 共享披露 + 正文引用数（两段引用 numId 21）
    assert!(
            report
                .content
                .contains("numId 21 → abstractNum 7，与 numId 33 共享 abstractNum 7（改定义同影响）｜正文引用 2 处"),
            "{}",
            report.content
        );
    // 级摘要：格式 / lvlText / start；Wingdings 私有区净化为 •
    assert!(
        report.content.contains(r#"lvl 0: decimal "%1." start=1"#),
        "{}",
        report.content
    );
    assert!(
        report.content.contains(r#"lvl 1: lowerLetter "%2)"#),
        "{}",
        report.content
    );
    assert!(
        report.content.contains(r#"lvl 0: bullet "•""#),
        "私有区应净化: {}",
        report.content
    );
    // numId=33 无正文引用 → 0 处
    assert!(
        report.content.contains("numId 33 → abstractNum 7"),
        "{}",
        report.content
    );

    // 下钻：numId+level → w:lvl 原文（抄写源，须含完整标签）
    let mut req = def_req(InspectProjection::Numbering);
    req.num_id = Some(21);
    req.level = Some(1);
    let report = inspect_document(&bytes, &req).unwrap();
    assert!(
        report
            .content
            .contains("numId 21 level 1 定义原文（影响 numId 21、33，共享 abstractNum 7）"),
        "{}",
        report.content
    );
    assert!(
        report
            .content
            .contains(r#"<w:numFmt w:val="lowerLetter"/>"#),
        "{}",
        report.content
    );
    assert!(report.content.contains("</w:lvl>"), "原文完整");

    // 只给 num_id → 该实例段展开
    let mut req = def_req(InspectProjection::Numbering);
    req.num_id = Some(40);
    let report = inspect_document(&bytes, &req).unwrap();
    assert!(
        report.content.contains("numId 40 → abstractNum 8"),
        "{}",
        report.content
    );

    // 错误族
    let mut req = def_req(InspectProjection::Numbering);
    req.num_id = Some(99);
    let err = inspect_document(&bytes, &req).unwrap_err().to_string();
    assert!(err.contains("编号引用或级别不存在"), "实际: {err}");
    assert!(err.contains("21、33、40"), "应列已有 numId: {err}");
    let mut req = def_req(InspectProjection::Numbering);
    req.num_id = Some(0);
    let err = inspect_document(&bytes, &req).unwrap_err().to_string();
    assert!(err.contains("显式无编号"), "实际: {err}");
    // level 不带 num_id
    let mut req = def_req(InspectProjection::Numbering);
    req.level = Some(0);
    let err = inspect_document(&bytes, &req).unwrap_err().to_string();
    assert!(err.contains("参数校验失败"), "实际: {err}");

    // 无编号部件：清单诚实提示；点名下钻 → 报错
    let report =
        inspect_document(&tbl_docx_bytes(), &def_req(InspectProjection::Numbering)).unwrap();
    assert!(report.content.contains("无编号部件"), "{}", report.content);
    let mut req = def_req(InspectProjection::Numbering);
    req.num_id = Some(1);
    let err = inspect_document(&tbl_docx_bytes(), &req)
        .unwrap_err()
        .to_string();
    assert!(err.contains("无编号部件"), "实际: {err}");
}

#[test]
fn def_params_rejected_by_block_projections() {
    let bytes = def_docx_bytes();
    // style/num_id/level 误传给块投影 → 参数不适用
    let mut req = def_req(InspectProjection::Text);
    req.num_id = Some(21);
    let err = inspect_document(&bytes, &req).unwrap_err().to_string();
    assert!(err.contains("参数不适用"), "实际: {err}");
    assert!(err.contains("style/num_id/level"), "实际: {err}");
    // styles 清单投影也不接受 style 参数（下钻是 styledef 的事）
    let mut req = def_req(InspectProjection::Styles);
    req.style = Some("Normal".into());
    let err = inspect_document(&bytes, &req).unwrap_err().to_string();
    assert!(err.contains("参数不适用"), "实际: {err}");
}
