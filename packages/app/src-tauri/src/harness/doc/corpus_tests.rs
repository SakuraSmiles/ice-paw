//! 真实语料冒烟测试（word-capability-roadmap 步骤 0 夹具地基）。
//!
//! 语料：用户提供的三份 Word 真机工程文档（SDP/SRS/INSTALL，含活动修订样本），
//! 特征盘点见 docs/word-capability-roadmap.md §四。
//!
//! 🚫 **语料严禁版本控制 / 上传**（用户拍板 2026-08-24）：文件只放本地
//! `tests/fixtures/docx/`（.gitignore 排除），测试**运行时读取**；文件缺失时
//! 本文件全部测试自动 skip（CI / 无语料机器不失败，本机放置后生效）。
//! 🚫 语料内容字符串一律不进代码（用户拍板 2026-08-24 二次收紧）：断言只用
//! 结构性锚点（规模/块号/首行派生关系/golden 逐字节对比），文档标题、正文
//! 词、样式名等任何来自语料的文本都不得出现在代码里。
//!
//! 本文件职责：锁「读侧吃得下真实复杂度」——整篇提取 / 分块分页 / 标题锚点 /
//! 零回归双闸 / 结构计数 / inspect 三级投影全过。

use super::{inspect_document, try_extract, try_extract_chunks, DocKind, InspectProjection, InspectRequest};
use super::xml_dom::{self, Element};

/// 运行时读取一份语料；缺失 → None（调用方 skip）。
fn corpus(name: &str) -> Option<Vec<u8>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/docx")
        .join(name);
    match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(_) => {
            eprintln!("skip：docx 语料 {name} 不存在（敏感文件不入库，仅本地 {}）", path.display());
            None
        }
    }
}

/// 三份全取（任一缺失整体 skip——多数断言是三份对比）。
fn all_corpus() -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    match (corpus("sdp.docx"), corpus("srs.docx"), corpus("install.docx")) {
        (Some(s), Some(r), Some(i)) => Some((s, r, i)),
        _ => None,
    }
}

#[test]
fn corpus_sdp_full_extract() {
    let Some(sdp) = corpus("sdp.docx") else { return };
    let doc = try_extract(&sdp, "docx").unwrap().expect("SDP 应识别为 docx");
    assert!(matches!(doc.kind, DocKind::Docx));
    // 🚫 语料内容字符串不进代码：锚点一律结构断言。「提取正确」由 golden 双闸
    // （corpus_model_text_equals_golden_scanner，逐字节 vs 扫描器）锁定，此处锁规模与派生关系。
    // 封面标题：非空短文本，且 == 提取文本首行（title 派生逻辑）
    let title = doc.title.clone().unwrap_or_default();
    assert!(!title.is_empty() && title.chars().count() <= 30, "SDP 标题异常: {title:?}");
    assert_eq!(doc.text.lines().next().map(str::trim), Some(title.as_str()));
    // 正文规模：30+ 页文档，提取文本应有几万字符量级
    assert!(doc.text.chars().count() > 20_000, "SDP 提取文本过短: {}", doc.text.chars().count());
}

#[test]
fn corpus_srs_full_extract() {
    let Some(srs) = corpus("srs.docx") else { return };
    let doc = try_extract(&srs, "docx").unwrap().expect("SRS 应识别为 docx");
    assert!(matches!(doc.kind, DocKind::Docx));
    // 封面标题同段双 run（跨 run 切分——run 级拼接的最小真实案例；拼接正确性由
    // golden 双闸逐字节锁定）。标题 == 提取文本首行。
    let title = doc.title.clone().unwrap_or_default();
    assert!(!title.is_empty() && title.chars().count() <= 30, "SRS 标题异常: {title:?}");
    assert_eq!(doc.text.lines().next().map(str::trim), Some(title.as_str()));
    // 正文规模：70 页文档
    assert!(doc.text.chars().count() > 10_000, "SRS 提取文本过短: {}", doc.text.chars().count());
    // 表格承重（20+ 表）：网格化建模正常（与 DOM 计数一致另由结构计数测试锁定）
    let dom = xml_dom::parse(&super::docx::read_document_xml(&srs).unwrap()).unwrap();
    let model = super::docx_model::build_document(&dom);
    assert!(count_model_tables(&model.body) >= 20, "SRS 表格数异常: {}", count_model_tables(&model.body));
}

#[test]
fn corpus_sdp_chunks_paginate() {
    let Some(sdp) = corpus("sdp.docx") else { return };
    let doc = try_extract(&sdp, "docx").unwrap().unwrap();
    let (kind, chunks) = try_extract_chunks(&sdp, "docx")
        .unwrap()
        .expect("SDP 分块应识别为 docx");
    assert!(matches!(kind, DocKind::Docx));
    // 35 页文档按 ~2000 token 装箱 → 几十块量级
    assert!(chunks.len() > 10, "SDP 分块数过少: {}", chunks.len());
    // 块标签连续、正文非空；首块以封面标题开头（分块从文本头开始、不丢首行）
    assert_eq!(chunks[0].label, "第1段");
    let title = doc.title.unwrap_or_default();
    assert!(chunks[0].text.starts_with(title.as_str()), "首块未含封面标题");
    for (i, c) in chunks.iter().enumerate() {
        assert!(!c.text.trim().is_empty(), "第 {} 块为空", i + 1);
        assert_eq!(c.label, format!("第{}段", i + 1));
    }
}

#[test]
fn corpus_srs_chunks_paginate() {
    let Some(srs) = corpus("srs.docx") else { return };
    let doc = try_extract(&srs, "docx").unwrap().unwrap();
    let (_, chunks) = try_extract_chunks(&srs, "docx")
        .unwrap()
        .expect("SRS 分块应识别为 docx");
    assert!(chunks.len() > 5, "SRS 分块数过少: {}", chunks.len());
    // 首块以封面标题开头（跨 run 拼接后的完整标题仍居首）
    let title = doc.title.unwrap_or_default();
    assert!(chunks[0].text.starts_with(title.as_str()), "首块未含封面标题");
}

// =========================================================================
// S0a 零回归硬闸 + 结构计数不变式（模型路径 vs golden 扫描器，真实语料三份）
// =========================================================================

/// 深度优先统计指定名字的元素总数（含任意嵌套深度）。
fn count_elements(el: &Element, name: &str) -> usize {
    let mut n = if el.name == name { 1 } else { 0 };
    for child in el.child_elements() {
        n += count_elements(child, name);
    }
    n
}

/// 递归统计模型中的段落数（含表格单元格内的段落）。
fn count_model_paragraphs(blocks: &[super::docx_model::Block]) -> usize {
    use super::docx_model::Block;
    blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(_) => 1,
            Block::Table(t) => t
                .rows
                .iter()
                .map(|r| r.cells.iter().map(|c| count_model_paragraphs(&c.blocks)).sum::<usize>())
                .sum(),
        })
        .sum()
}

/// 递归统计模型中的表格数（含嵌套表）。
fn count_model_tables(blocks: &[super::docx_model::Block]) -> usize {
    use super::docx_model::Block;
    blocks
        .iter()
        .map(|b| match b {
            Block::Table(t) => {
                1 + t.rows
                    .iter()
                    .map(|r| {
                        r.cells
                            .iter()
                            .map(|c| count_model_tables(&c.blocks))
                            .sum::<usize>()
                    })
                    .sum::<usize>()
            }
            Block::Paragraph(_) => 0,
        })
        .sum()
}

/// 剥离 pPr 内的 tab 停靠点定义块（`<w:tabs>…</w:tabs>`）。
///
/// 旧扫描器不识结构，把停靠点定义里的自闭合 `<w:tab/>` 误当**内容**输出 `\t`
/// （幻影制表符——TOC/自定义缩进段落每处停靠点定义多一个 `\t`）。模型正确地
/// 不把格式定义当文本（S0a 有意修复，见 roadmap §五-6）。golden 对比前先剥离
/// 定义块，使两侧语义对齐：剥离后扫描器输出 == 模型输出，逐字节。
fn strip_tab_stops(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<w:tabs>") {
        let after = &rest[start..];
        if let Some(end_rel) = after.find("</w:tabs>") {
            out.push_str(&rest[..start]);
            rest = &after[end_rel + "</w:tabs>".len()..];
        } else {
            break; // 残缺块原样保留
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn corpus_model_text_equals_golden_scanner() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    // S0a 零回归硬闸：真实语料（域/内容控件/表格承重/真机修订全形态）上，
    // 模型投影 == 扫描器（剥掉其幻影制表符缺陷后）逐字节相等。
    for (name, bytes) in [("SDP", &sdp), ("SRS", &srs), ("INSTALL", &install)] {
        let xml = super::docx::read_document_xml(bytes).unwrap();
        let mut scanner_text = super::docx::extract_text_from_xml(&strip_tab_stops(&xml));
        super::normalize(&mut scanner_text);
        let doc = try_extract(bytes, "docx").unwrap().unwrap();
        assert_eq!(doc.text, scanner_text, "{name} 模型投影与扫描器输出不一致");
    }
}

#[test]
fn corpus_model_structure_counts_match_dom() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    // 结构完整性：模型段落数/表格数 == DOM 中对应元素总数（无遗漏无重复建模）
    for (name, bytes) in [("SDP", &sdp), ("SRS", &srs), ("INSTALL", &install)] {
        let xml = super::docx::read_document_xml(bytes).unwrap();
        let dom = xml_dom::parse(&xml).unwrap();
        let model = super::docx_model::build_document(&dom);
        let dom_paras = count_elements(&dom, "w:p");
        let dom_tbls = count_elements(&dom, "w:tbl");
        let model_paras = count_model_paragraphs(&model.body);
        let model_tbls = count_model_tables(&model.body);
        assert_eq!(model_paras, dom_paras, "{name} 段落数不一致");
        assert_eq!(model_tbls, dom_tbls, "{name} 表格数不一致");
    }
}

// =========================================================================
// S0b inspect_docx 三级投影（真实语料：样式名解析 / 修订警告 / 编址一致性）
// =========================================================================

#[test]
fn corpus_inspect_three_projections() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    // (名称, 字节, 是否含修订): SDP/SRS 无修订；INSTALL 含真机修订（run 级 22/16）
    for (name, bytes, has_revisions) in
        [("SDP", &sdp, false), ("SRS", &srs, false), ("INSTALL", &install, true)]
    {
        // outline：默认上限 400（SDP 顶层块 ~1780，全量会爆 token——上限被语料正当化）
        let report = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Outline, start: None, end: None, row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert!(report.total_blocks > 80, "{name} 块数过少: {}", report.total_blocks);
        let outline_end = report.total_blocks.min(400);
        assert_eq!(report.range, (1, outline_end), "{name} outline 默认区间");
        assert_eq!(report.has_more, report.total_blocks > 400);
        // 修订警告：有修订必报（INSTALL），无修订不误报
        assert_eq!(
            report.content.contains("含修订标记"),
            has_revisions,
            "{name} 修订警告判定不符"
        );
        if has_revisions {
            // 真机修订锚点：计数锁定（模型/投影改动破坏修订语义时立刻抓到）
            assert!(
                report.content.contains("22 处插入 / 16 处删除"),
                "{name} 修订计数漂移: {}",
                report.content.lines().next().unwrap_or_default()
            );
        }
        // 国标模板样式表（SDP/SRS）：outlineLvl 定义的标题样式 → H 级标记出现
        if !has_revisions {
            assert!(report.content.contains("H1 "), "{name} 样式名/层级未解析");
        }
        // 块号连续性：渲染区间内每号恰好一行
        for n in 1..=outline_end {
            let prefix = format!("[{n}] ");
            let hits = report.content.lines().filter(|l| l.starts_with(&prefix)).count();
            assert_eq!(hits, 1, "{name} 块 {n} 行数 = {hits}（应恰好 1）");
        }

        // format：默认 span=50，has_more，含有效格式行
        let fmt = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Format, start: None, end: None, row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert_eq!(fmt.range, (1, 50), "{name} format 默认区间");
        assert!(fmt.has_more, "{name} format 应有 has_more");
        assert!(fmt.content.contains("段落格式:"), "{name} format 应有段落属性行");

        // text：带块号正文，区间续读
        let text = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Text, start: Some(2), end: Some(6), row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert_eq!(text.range, (2, 6));
        assert!(text.content.contains("[2]"), "{name} text 应带块号前缀");
        // 越界错误带总数（报错契约）
        let err = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Text, start: Some(report.total_blocks + 1), end: None, row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("块号越界"), "{name} 实际: {err}");
    }
}

#[test]
fn corpus_numbering_values_rendered() {
    let Some((sdp, srs, _install)) = all_corpus() else { return };
    // S3①：自动编号实际值进投影——语料锚点（全结构断言，不写具体编号文本进代码）
    // SDP：H1 自动编号标题（真实 Word 产物），编号从 1 起
    let report = inspect_document(
        &sdp,
        &InspectRequest { projection: InspectProjection::Outline, start: None, end: None, row: None, cell: None, style: None, num_id: None, level: None },
    )
    .unwrap();
    assert!(
        report.content.lines().any(|l| l.contains("H1 ") && l.contains("列表 1 ")),
        "SDP 应有自动编号 H1（列表 1 形态）"
    );
    assert!(
        report.content.lines().any(|l| l.contains("列表 2 ")),
        "SDP 自动编号应连续推进到 2"
    );
    // SRS：三级编号形态存在（N.N.N 计数模拟）
    let report = inspect_document(
        &srs,
        &InspectRequest { projection: InspectProjection::Outline, start: None, end: None, row: None, cell: None, style: None, num_id: None, level: None },
    )
    .unwrap();
    assert!(
        report
            .content
            .lines()
            .any(|l| l.contains("列表 ") && {
                let v = l.split("列表 ").nth(1).unwrap_or("").split(' ').next().unwrap_or("");
                v.split('.').count() == 3 && v.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            }),
        "SRS 应有多级编号形态 N.N.N"
    );
    // 手调 start 形态（作者用 lvl1 start=2 / lvl2 start=6 对齐手写编号）：首个 lvl2
    // 段的祖先级按其 start 渲染——回归 2026-08-24 真机发现（算成 1.1.6 误导 agent
    // 得出「章节号错位」误诊；Word 实渲染/目录静态文本/用户肉眼三方一致 1.2.6）
    assert!(
        report.content.contains("列表 1.2.6"),
        "SRS 手调 start 编号应渲染 1.2.6（而非 1.1.6）"
    );
    // 三份语料投影里不出现未解析引用形式（numPr 引用全部解析出值/或该段无值回退——
    // 语料 numbering.xml 完整，默认 400 块渲染面内引用形式应为零）
    for bytes in [&sdp, &srs] {
        let report = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Outline, start: None, end: None, row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert!(
            !report.content.contains("列表(num"),
            "渲染面内不应有未解析编号引用（numbering.xml 解析缺口）"
        );
    }
}

#[test]
fn corpus_headers_footers_projection() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    // S3④：页眉页脚投影（真实 Word 产物：SDP/SRS 有 even/default/first 全形态引用，
    // INSTALL 三节全 default；空部件与有内容部件并存——（空）标注 + 非空行都要出现）
    for (name, bytes) in [("SDP", &sdp), ("SRS", &srs), ("INSTALL", &install)] {
        let report = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::HeadersFooters, start: None, end: None, row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert!(report.content.contains("[节 1]"), "{name} 应有节行: {}", report.content);
        assert!(
            report.content.lines().any(|l| l.contains("页眉") && l.contains(':') && !l.contains("（空）")),
            "{name} 应有非空页眉行"
        );
        assert!(
            report.content.lines().any(|l| l.contains("页脚") && l.contains(':') && !l.contains("（空）")),
            "{name} 应有非空页脚行（页码域缓存值）"
        );
        assert!(report.content.contains("（空）"), "{name} 空部件应诚实标注");
        // 悬空引用（rels 有但 document 未引用的部件）不出现；已引用的全部解析
        assert!(!report.content.contains("悬空"), "{name} 不应有悬空引用");
    }
}

#[test]
fn corpus_install_revisions_visible_in_format() {
    let Some(install) = corpus("install.docx") else { return };
    // 含修订语料的 format 全量扫描：插入/删除 run 必须带标记（edit_docx 不触碰修订 run
    // 的前提 = agent 能看见它们在哪）
    let fmt = inspect_document(
        &install,
        &InspectRequest { projection: InspectProjection::Format, start: Some(1), end: Some(171), row: None, cell: None, style: None, num_id: None, level: None },
    )
    .unwrap();
    assert!(fmt.content.contains("〔插入修订〕"), "插入修订 run 未标记");
    assert!(fmt.content.contains("〔删除修订〕"), "删除修订 run 未标记");
    // text 投影剔除删除修订（接受修订后视图）——修订剔除不应清空正文（默认 span=100 块量级仍在）
    let text = inspect_document(
        &install,
        &InspectRequest { projection: InspectProjection::Text, start: Some(1), end: None, row: None, cell: None, style: None, num_id: None, level: None },
    )
    .unwrap();
    assert!(text.content.lines().count() > 100, "正文行数过少: {}", text.content.lines().count());
}

// =========================================================================
// 步骤 3 edit_docx 手术引擎（真实语料：定位器全量对齐 + 手术闭环 + untouched 保真）
// =========================================================================

/// 选一个「安全可编辑」块（1-based）：非表格、无修订 run、非 sectPr 载体、投影 ≥4 字。
/// 三操作各需不同块（每块每批限一操作），`skip` 排除已选块号。
/// 🚫 语料字符串不进代码：目标块与 expect_prefix 指纹全部运行时派生。
fn pick_editable_block(
    xml: &str,
    spans: &[super::docx_edit::BlockSpan],
    model: &super::docx_model::DocxDocument,
    skip: &[usize],
) -> Option<usize> {
    use super::docx_model::{blocks_text, Block};
    for (i, b) in model.body.iter().enumerate() {
        let n = i + 1;
        if skip.contains(&n) {
            continue;
        }
        if matches!(b, Block::Table(_)) {
            continue; // replace 拒表格
        }
        if let Block::Paragraph(p) = b {
            if p.runs.iter().any(|r| r.revision.is_some()) {
                continue; // replace/delete 拒修订块
            }
        }
        let block_xml = &xml[spans[i].start..spans[i].end];
        if block_xml.contains("<w:sectPr") {
            continue; // delete 拒节属性载体
        }
        let mut t = String::new();
        blocks_text(&model.body[i..i + 1], &mut t);
        if t.trim().chars().count() >= 4 {
            return Some(n); // 投影 ≥4 字：够派生指纹前缀
        }
    }
    None
}

/// 逐 entry 对比两个 docx 包：`except` 外的 entry 内容必须逐字节相等（保真不变式）。
fn assert_untouched_entries_identical(orig: &[u8], new: &[u8], except: &str) {
    use std::io::Read;
    let mut za = zip::ZipArchive::new(std::io::Cursor::new(orig)).unwrap();
    let mut zb = zip::ZipArchive::new(std::io::Cursor::new(new)).unwrap();
    assert_eq!(za.len(), zb.len(), "entry 数应一致");
    for i in 0..za.len() {
        let mut da = Vec::new();
        let mut ea = za.by_index(i).unwrap();
        let name = ea.name().to_string();
        ea.read_to_end(&mut da).unwrap();
        let mut db = Vec::new();
        let mut eb = zb.by_index(i).unwrap();
        eb.read_to_end(&mut db).unwrap();
        if name == except {
            assert_ne!(da, db, "{name} 应已替换");
        } else {
            assert_eq!(da, db, "{name} 应逐字节相等（untouched 保真）");
        }
    }
}

#[test]
fn corpus_edit_engine_roundtrip() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    // 手术替换/插入用自造短语（非语料来源——禁令）
    const REPLACED: &str = "测试替换后文本甲乙丙";
    const INSERTED: &str = "测试插入段XYZ";
    for (name, bytes) in [("SDP", &sdp), ("SRS", &srs), ("INSTALL", &install)] {
        let xml = super::docx::read_document_xml(bytes).unwrap();

        // ① 定位器全量对齐（真实 Word 16.0 产物：sdt/域/多节/表格混杂，千块级压测）
        let spans = super::docx_edit::locate_blocks(&xml).unwrap();
        let dom = xml_dom::parse(&xml).unwrap();
        let model = super::docx_model::build_document(&dom);
        assert_eq!(spans.len(), model.body.len(), "{name} 定位器/模型块数不一致");
        for (i, span) in spans.iter().enumerate() {
            let piece = &xml[span.start..span.end];
            let piece_model = super::docx_model::build_document(&xml_dom::parse(piece).unwrap());
            assert_eq!(piece_model.body.len(), 1, "{name} 块 {} span 子串非单块", i + 1);
        }

        // ② 手术闭环（纯函数不落盘；三操作三块，指纹 = 投影前 4 字运行时派生）
        let t1 = pick_editable_block(&xml, &spans, &model, &[]).expect("{name} 无可编辑块");
        let t2 = pick_editable_block(&xml, &spans, &model, &[t1]).expect("{name} 无第 2 可编辑块");
        let t3 = pick_editable_block(&xml, &spans, &model, &[t1, t2]).expect("{name} 无第 3 可编辑块");
        let prefix_of = |n: usize| -> String {
            let mut t = String::new();
            super::docx_model::blocks_text(&model.body[n - 1..n], &mut t);
            t.trim().chars().take(4).collect()
        };
        let (new_bytes, applied) = super::apply_edits_to_bytes(
            bytes,
            &[
                super::EditOp::ReplaceText {
                    block: t1,
                    expect_prefix: prefix_of(t1),
                    new_text: REPLACED.into(),
                },
                super::EditOp::InsertParagraphAfter {
                    block: t2,
                    expect_prefix: prefix_of(t2),
                    text: INSERTED.into(),
                    style: None,
                },
                super::EditOp::DeleteBlock { block: t3, expect_prefix: prefix_of(t3) },
            ],
        )
        .unwrap_or_else(|e| panic!("{name} 手术失败: {e}"));
        assert_eq!(applied.len(), 3);

        // ③ untouched 保真：除 document.xml 外逐 entry 字节相等
        assert_untouched_entries_identical(bytes, &new_bytes, "word/document.xml");

        // ④ 读回复核：t1 = REPLACED；t2 后紧跟 INSERTED；总块数 n+1-1 = n
        let seg = inspect_document(
            &new_bytes,
            &InspectRequest { projection: InspectProjection::Text, start: Some(t1), end: Some(t2 + 1), row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert!(seg.content.contains(REPLACED), "{name} t1={t1} 替换未生效:\n{}", seg.content);
        assert!(seg.content.contains(INSERTED), "{name} t2={t2} 后插入未生效:\n{}", seg.content);
        let full = inspect_document(
            &new_bytes,
            &InspectRequest { projection: InspectProjection::Outline, start: None, end: None, row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert_eq!(full.total_blocks, spans.len(), "{name} 块数守恒（+1 插 -1 删）");
    }
}

#[test]
fn corpus_set_style_surgery() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    // S3③：真实 Word 产物上换样式闭环。样式目标与指纹全运行时派生（语料字符串不进代码）：
    // 目标样式 = 文档 styles.xml 里第一个与目标块当前样式不同的段落样式 ID。
    for (name, bytes) in [("SDP", &sdp), ("SRS", &srs), ("INSTALL", &install)] {
        let xml = super::docx::read_document_xml(bytes).unwrap();
        let spans = super::docx_edit::locate_blocks(&xml).unwrap();
        let model = super::docx_model::build_document(&xml_dom::parse(&xml).unwrap());
        let target = pick_editable_block(&xml, &spans, &model, &[]).expect("{name} 无可编辑块");

        // 段落样式 ID 清单（来自 styles.xml；排除目标当前样式）
        let styles_xml = super::docx::read_entry(bytes, "word/styles.xml")
            .unwrap()
            .expect("{name} 应有 styles.xml");
        let styles_dom = xml_dom::parse(&styles_xml).unwrap();
        let current = match &model.body[target - 1] {
            super::docx_model::Block::Paragraph(p) => p.props.style.clone(),
            _ => None,
        };
        let mut candidates = Vec::new();
        for st in styles_dom.child_elements().filter(|e| e.name == "w:style") {
            if st.attr("w:type") != Some("paragraph") {
                continue;
            }
            if let Some(id) = st.attr("w:styleId") {
                if Some(id) != current.as_deref() {
                    candidates.push(id.to_string());
                }
            }
        }
        let want = candidates.first().expect("{name} 无可切换的段落样式").clone();

        let prefix_of = |n: usize| -> String {
            let mut t = String::new();
            super::docx_model::blocks_text(&model.body[n - 1..n], &mut t);
            t.trim().chars().take(4).collect()
        };
        let (new_bytes, applied) = super::apply_edits_to_bytes(
            bytes,
            &[super::EditOp::SetStyle {
                block: target,
                expect_prefix: prefix_of(target),
                style: want.clone(),
            }],
        )
        .unwrap_or_else(|e| panic!("{name} set_style 失败: {e}"));
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].style.as_deref(), Some(want.as_str()));

        // untouched 保真：只有 document.xml 变
        assert_untouched_entries_identical(bytes, &new_bytes, "word/document.xml");

        // 读回：目标块样式 == want，文本与块数不变
        let new_xml = super::docx::read_document_xml(&new_bytes).unwrap();
        let model2 = super::docx_model::build_document(&xml_dom::parse(&new_xml).unwrap());
        assert_eq!(model2.body.len(), model.body.len(), "{name} 块数应守恒");
        let super::docx_model::Block::Paragraph(p) = &model2.body[target - 1] else {
            panic!("{name} 目标块应仍是段落")
        };
        assert_eq!(p.props.style.as_deref(), Some(want.as_str()), "{name} 样式未生效");
        let (mut old_t, mut new_t) = (String::new(), String::new());
        super::docx_model::blocks_text(&model.body[target - 1..target], &mut old_t);
        super::docx_model::blocks_text(&model2.body[target - 1..target], &mut new_t);
        assert_eq!(old_t, new_t, "{name} set_style 不应改动文本");
    }
}

#[test]
fn corpus_set_format_surgery() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    // S3②：真实 Word 产物上段落+字符格式手术闭环。目标块与指纹全运行时派生
    // （语料字符串不进代码）；格式值用与语料无关的自造值。
    for (name, bytes) in [("SDP", &sdp), ("SRS", &srs), ("INSTALL", &install)] {
        let xml = super::docx::read_document_xml(bytes).unwrap();
        let spans = super::docx_edit::locate_blocks(&xml).unwrap();
        let model = super::docx_model::build_document(&xml_dom::parse(&xml).unwrap());
        let target = pick_editable_block(&xml, &spans, &model, &[]).expect("{name} 无可编辑块");
        let t2 = pick_editable_block(&xml, &spans, &model, &[target]).expect("{name} 无第 2 可编辑块");
        let prefix_of = |n: usize| -> String {
            let mut t = String::new();
            super::docx_model::blocks_text(&model.body[n - 1..n], &mut t);
            t.trim().chars().take(4).collect()
        };
        let (new_bytes, applied) = super::apply_edits_to_bytes(
            bytes,
            &[
                super::EditOp::SetFormat {
                    block: target,
                    expect_prefix: prefix_of(target),
                    paragraph: Some(super::ParaFormat {
                        align: Some("center".into()),
                        line_spacing: Some(1.5),
                        ..Default::default()
                    }),
                    character: Some(super::CharFormat {
                        bold: Some(true),
                        font_size_pt: Some(14.0),
                        color: Some("FF0000".into()),
                        ..Default::default()
                    }),
                },
                // 第二块独立验段落格式（合并分支：真实文档常有既有 spacing/ind）
                super::EditOp::SetFormat {
                    block: t2,
                    expect_prefix: prefix_of(t2),
                    paragraph: Some(super::ParaFormat {
                        space_after_pt: Some(6.0),
                        indent_first_line_tw: Some(480),
                        ..Default::default()
                    }),
                    character: None,
                },
            ],
        )
        .unwrap_or_else(|e| panic!("{name} set_format 失败: {e}"));
        assert_eq!(applied.len(), 2);
        assert_eq!(applied[0].op, "set_format");

        // untouched 保真：只有 document.xml 变
        assert_untouched_entries_identical(bytes, &new_bytes, "word/document.xml");

        // 读回：块数/文本不变；目标块格式生效（段落 + 每 run 字符格式）
        let new_xml = super::docx::read_document_xml(&new_bytes).unwrap();
        let model2 = super::docx_model::build_document(&xml_dom::parse(&new_xml).unwrap());
        assert_eq!(model2.body.len(), model.body.len(), "{name} 块数应守恒");
        let super::docx_model::Block::Paragraph(p) = &model2.body[target - 1] else {
            panic!("{name} 目标块应仍是段落")
        };
        assert_eq!(p.props.alignment.as_deref(), Some("center"), "{name} 对齐未生效");
        assert_eq!(p.props.spacing_line, Some(360), "{name} 行距未生效");
        for (i, r) in p.runs.iter().enumerate() {
            assert_eq!(r.props.bold, Some(true), "{name} run {i} 加粗未生效");
            assert_eq!(r.props.size_half_pt, Some(28), "{name} run {i} 字号未生效");
            assert_eq!(r.props.color.as_deref(), Some("FF0000"), "{name} run {i} 颜色未生效");
        }
        let (mut old_t, mut new_t) = (String::new(), String::new());
        super::docx_model::blocks_text(&model.body[target - 1..target], &mut old_t);
        super::docx_model::blocks_text(&model2.body[target - 1..target], &mut new_t);
        assert_eq!(old_t, new_t, "{name} set_format 不应改动文本");
    }
}

/// S3 二波（D9 通用元素手术）：真实语料上 set_ppr_element 闭环——段级 numPr
/// 摘除（去自动编号的正路）+ keepNext 插入。目标块与指纹全运行时派生。
#[test]
fn corpus_ppr_element_surgery() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    use super::docx_model::{blocks_text, Block};
    let mut ran = 0usize;
    for (name, bytes) in [("SDP", &sdp), ("SRS", &srs), ("INSTALL", &install)] {
        let xml = super::docx::read_document_xml(bytes).unwrap();
        let spans = super::docx_edit::locate_blocks(&xml).unwrap();
        let model = super::docx_model::build_document(&xml_dom::parse(&xml).unwrap());

        // 找一个带段级编号、无修订、非表格、非 pPrChange 的有字段落
        let numbered = 'outer: {
            for (i, b) in model.body.iter().enumerate() {
                let Block::Paragraph(p) = b else { continue };
                if p.props.numbering.is_none() || p.runs.iter().any(|r| r.revision.is_some()) {
                    continue;
                }
                let bx = &xml[spans[i].start..spans[i].end];
                if bx.contains("<w:pPrChange") || bx.contains("<w:sectPr") {
                    continue;
                }
                let mut t = String::new();
                blocks_text(&model.body[i..i + 1], &mut t);
                if t.trim().chars().count() >= 4 {
                    break 'outer Some(i + 1);
                }
            }
            None
        };
        let Some(numbered) = numbered else { continue };
        // 第二目标（keepNext upsert）：任意可编辑段
        let other = pick_editable_block(&xml, &spans, &model, &[numbered])
            .expect("{name} 无第 2 可编辑块");
        let prefix_of = |n: usize| -> String {
            let mut t = String::new();
            blocks_text(&model.body[n - 1..n], &mut t);
            t.trim().chars().take(4).collect()
        };

        let (new_bytes, applied) = super::apply_edits_to_bytes(
            bytes,
            &[
                super::EditOp::SetPprElement {
                    block: numbered,
                    expect_prefix: prefix_of(numbered),
                    element: "numPr".into(),
                    xml: None,
                },
                super::EditOp::SetPprElement {
                    block: other,
                    expect_prefix: prefix_of(other),
                    element: "keepNext".into(),
                    xml: Some("<w:keepNext/>".into()),
                },
            ],
        )
        .unwrap_or_else(|e| panic!("{name} set_ppr_element 失败: {e}"));
        assert_eq!(applied.len(), 2);
        assert!(applied.iter().all(|a| a.op == "set_ppr_element"));
        // 摘要序按 splice 位置（块序）非入参序——按内容断言
        assert!(
            applied.iter().any(|a| a.after.starts_with("removed numPr")),
            "{name} 应含 numPr 摘除摘要，实际: {:?}",
            applied.iter().map(|a| a.after.as_str()).collect::<Vec<_>>()
        );
        ran += 1;

        // untouched 保真：只有 document.xml 变
        assert_untouched_entries_identical(bytes, &new_bytes, "word/document.xml");

        // 读回：块数/文本不变；编号引用消失；keepNext 落位
        let new_xml = super::docx::read_document_xml(&new_bytes).unwrap();
        let model2 = super::docx_model::build_document(&xml_dom::parse(&new_xml).unwrap());
        assert_eq!(model2.body.len(), model.body.len(), "{name} 块数应守恒");
        let Block::Paragraph(p) = &model2.body[numbered - 1] else { panic!() };
        assert!(p.props.numbering.is_none(), "{name} 段级编号引用应消失");
        let new_spans = super::docx_edit::locate_blocks(&new_xml).unwrap();
        assert!(
            new_xml[new_spans[other - 1].start..new_spans[other - 1].end]
                .contains("<w:keepNext"),
            "{name} keepNext 未落位"
        );
        let (mut old_t, mut new_t) = (String::new(), String::new());
        blocks_text(&model.body[numbered - 1..numbered], &mut old_t);
        blocks_text(&model2.body[numbered - 1..numbered], &mut new_t);
        assert_eq!(old_t, new_t, "{name} 元素手术不应改动文本");
    }
    assert!(ran >= 2, "SDP/SRS 语料应均有段级编号块可跑（实际 {ran} 份）");
}

/// S3 三波·表格四件：真实语料上三写操作闭环（set_cell_text 保结构改格 /
/// insert_table_row_after 克隆增行 / insert_table_after 建新表）+ 网格投影。
/// 目标块、(行, 格) 地址与指纹全运行时派生（语料字符串不进代码）。
#[test]
fn corpus_table_surgery() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    use super::docx_model::{blocks_text, Block};
    // 自造短语（非语料来源——禁令）
    const FILL: &str = "表格格测试甲乙丙";
    const NEW_HEAD: &str = "新表头甲";

    // 干净表判定：非空、无嵌套表、全格无修订 run
    fn table_clean(t: &super::docx_model::Table) -> bool {
        fn blocks_clean(blocks: &[Block]) -> bool {
            blocks.iter().all(|b| match b {
                Block::Paragraph(p) => p.runs.iter().all(|r| r.revision.is_none()),
                Block::Table(_) => false, // 嵌套表整表跳过
            })
        }
        !t.rows.is_empty() && t.rows.iter().all(|r| r.cells.iter().all(|c| blocks_clean(&c.blocks)))
    }

    let mut ran = 0usize;
    for (name, bytes) in [("SDP", &sdp), ("SRS", &srs), ("INSTALL", &install)] {
        let xml = super::docx::read_document_xml(bytes).unwrap();
        let spans = super::docx_edit::locate_blocks(&xml).unwrap();
        let model = super::docx_model::build_document(&xml_dom::parse(&xml).unwrap());

        // 找干净表 + 可编辑格（非续格、有 ≥2 字文本）+ 整表投影首字符非空行
        let mut found: Option<(usize, usize, usize, usize)> = None; // (块, 行, 格, 原行数)
        'tables: for (i, b) in model.body.iter().enumerate() {
            let Block::Table(t) = b else { continue };
            if !table_clean(t) {
                continue;
            }
            let mut whole = String::new();
            blocks_text(&model.body[i..i + 1], &mut whole);
            let whole = whole.trim_end_matches('\n');
            if whole.starts_with('\n') || whole.trim().is_empty() {
                continue; // 首格空段会让前 4 字指纹对不上投影首字符
            }
            for (ri, row) in t.rows.iter().enumerate() {
                for (ci, c) in row.cells.iter().enumerate() {
                    if c.v_merge.as_deref() == Some("continue") {
                        continue;
                    }
                    let mut txt = String::new();
                    blocks_text(&c.blocks, &mut txt);
                    if txt.trim().chars().count() >= 2 {
                        found = Some((i + 1, ri + 1, ci + 1, t.rows.len()));
                        break 'tables;
                    }
                }
            }
        }
        let Some((tbl_block, row, cell, rows0)) = found else { continue };
        let anchor = pick_editable_block(&xml, &spans, &model, &[tbl_block])
            .unwrap_or_else(|| panic!("{name} 无独立锚段"));
        // 指纹口径与预检一致：只去尾换行，取前 4 字
        let prefix_of = |n: usize| -> String {
            let mut t = String::new();
            blocks_text(&model.body[n - 1..n], &mut t);
            t.trim_end_matches('\n').chars().take(4).collect()
        };

        let (new_bytes, applied) = super::apply_edits_to_bytes(
            bytes,
            &[
                super::EditOp::SetCellText {
                    block: tbl_block,
                    expect_prefix: prefix_of(tbl_block),
                    row,
                    cell,
                    text: FILL.into(),
                },
                super::EditOp::InsertTableRowAfter {
                    block: tbl_block,
                    expect_prefix: prefix_of(tbl_block),
                    after_row: None,
                    cells: None,
                },
                super::EditOp::InsertTableAfter {
                    block: anchor,
                    expect_prefix: prefix_of(anchor),
                    rows: vec![vec![NEW_HEAD.into(), "新表体乙".into()], vec!["行一".into(), "行二".into()]],
                    header: None,
                    table_style: None,
                },
            ],
        )
        .unwrap_or_else(|e| panic!("{name} 表格手术失败: {e}"));
        assert_eq!(applied.len(), 3, "{name} 三操作应全过");
        ran += 1;

        // untouched 保真：只有 document.xml 变
        assert_untouched_entries_identical(bytes, &new_bytes, "word/document.xml");

        // 读回：块数 +1（新建表）；目标表行数 +1；目标格文本已换。
        // 锚段在目标表之前时，插表使目标表块号 +1（位移感知）
        let new_xml = super::docx::read_document_xml(&new_bytes).unwrap();
        let model2 = super::docx_model::build_document(&xml_dom::parse(&new_xml).unwrap());
        assert_eq!(model2.body.len(), model.body.len() + 1, "{name} 块数应 +1");
        let at = if anchor < tbl_block { tbl_block + 1 } else { tbl_block };
        let Block::Table(t2) = &model2.body[at - 1] else { panic!("{name} 目标块应仍是表") };
        assert_eq!(t2.rows.len(), rows0 + 1, "{name} 增行未生效");
        let mut got = String::new();
        blocks_text(&t2.rows[row - 1].cells[cell - 1].blocks, &mut got);
        assert_eq!(got.trim(), FILL, "{name} 改格未生效");
        // 新表落位（首格文本 = NEW_HEAD 派生查找，不依赖块号偏移）
        let new_tbl_found = model2.body.iter().any(|b| {
            let Block::Table(t) = b else { return false };
            let mut s = String::new();
            if let Some(c) = t.rows.first().and_then(|r| r.cells.first()) {
                blocks_text(&c.blocks, &mut s);
            }
            s.trim() == NEW_HEAD
        });
        assert!(new_tbl_found, "{name} 新建表未落位");

        // 网格投影（真实 Word 产物）：目标表块号渲染出矩阵行
        let grid = inspect_document(
            &new_bytes,
            &InspectRequest { projection: InspectProjection::Table, start: Some(at), end: Some(at), row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert!(grid.content.contains(&format!("[{at}] ▦ 表")), "{name} 网格头行缺失");
        assert!(grid.content.contains("r1:"), "{name} 网格应有行线");
    }
    assert!(ran >= 2, "SDP/SRS 语料应均有干净表可跑（实际 {ran} 份）");
}

/// S3 四波·表格格式件：真实语料上四操作闭环（每操作独立事务，互不借址）——
/// ① set_table_element 表级 shd upsert + 幂等（二次套用逐字节稳定）
/// ② set_cell_format 格级段落+字符格式（文本/块数不变）
/// ③ merge_cells + split_cell 横向往返（格数/跨度还原，文本集合守恒）
/// ④ merge_cells + split_cell 纵向往返（整表文本投影与原表逐字节相等）
/// 目标表/格地址与指纹全运行时派生（语料字符串不进代码）。
#[test]
fn corpus_table_format_surgery() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    use super::docx_model::{blocks_text, Block};
    // 自造片段与值（非语料来源——禁令）
    const SHD_FRAG: &str = r#"<w:shd w:val="clear" w:color="auto" w:fill="EEF3FA"/>"#;
    const FILL: &str = "EEF3FA";

    fn table_clean(t: &super::docx_model::Table) -> bool {
        fn blocks_clean(blocks: &[Block]) -> bool {
            blocks.iter().all(|b| match b {
                Block::Paragraph(p) => p.runs.iter().all(|r| r.revision.is_none()),
                Block::Table(_) => false, // 嵌套表整表跳过
            })
        }
        !t.rows.is_empty() && t.rows.iter().all(|r| r.cells.iter().all(|c| blocks_clean(&c.blocks)))
    }

    let mut ran = 0usize;
    for (name, bytes) in [("SDP", &sdp), ("SRS", &srs), ("INSTALL", &install)] {
        let xml = super::docx::read_document_xml(bytes).unwrap();
        let model = super::docx_model::build_document(&xml_dom::parse(&xml).unwrap());

        // 找干净表：≥2 行（纵并用）且存在某行两个相邻单跨非续格（横并用）
        let mut found: Option<usize> = None;
        'tables: for (i, b) in model.body.iter().enumerate() {
            let Block::Table(t) = b else { continue };
            if !table_clean(t) || t.rows.len() < 2 {
                continue;
            }
            for row in &t.rows {
                for w in row.cells.windows(2) {
                    let (a, b2) = (&w[0], &w[1]);
                    if a.grid_span.is_none() && b2.grid_span.is_none()
                        && a.v_merge.is_none() && b2.v_merge.is_none()
                    {
                        found = Some(i + 1);
                        break 'tables;
                    }
                }
            }
        }
        let Some(tbl) = found else { continue };
        let Block::Table(t0) = &model.body[tbl - 1] else { unreachable!() };
        // 纵并头候选：首行首个单跨非合并格（下行同网格区间自然对齐——单跨必齐）
        let v_head = t0
            .rows
            .first()
            .unwrap()
            .cells
            .iter()
            .position(|c| c.grid_span.is_none() && c.v_merge.is_none())
            .unwrap()
            + 1;
        // 横并候选行/格：运行时重找（与发现判据同）
        let (h_row, h_cell) = {
            let mut pick = (1usize, 1usize);
            'pick: for (ri, row) in t0.rows.iter().enumerate() {
                for w in row.cells.windows(2) {
                    if w[0].grid_span.is_none() && w[1].grid_span.is_none()
                        && w[0].v_merge.is_none() && w[1].v_merge.is_none()
                    {
                        pick = (ri + 1, row.cells.iter().position(|c| c.grid_span.is_none() && c.v_merge.is_none()).unwrap() + 1);
                        break 'pick;
                    }
                }
            }
            pick
        };
        let prefix = {
            let mut s = String::new();
            blocks_text(&model.body[tbl - 1..tbl], &mut s);
            s.trim_end_matches('\n').chars().take(4).collect::<String>()
        };

        // ---- ① set_table_element 表级 shd：upsert + 幂等 ----
        let op = super::EditOp::SetTableElement {
            block: tbl,
            expect_prefix: prefix.clone(),
            level: super::TableLevel::Table,
            row: None,
            cell: None,
            element: "shd".into(),
            xml: Some(SHD_FRAG.into()),
        };
        let (b1, applied) = super::apply_edits_to_bytes(bytes, std::slice::from_ref(&op))
            .unwrap_or_else(|e| panic!("{name} set_table_element 失败: {e}"));
        assert_eq!(applied.len(), 1);
        assert_untouched_entries_identical(bytes, &b1, "word/document.xml");
        let (b1b, _) = super::apply_edits_to_bytes(&b1, &[op]).unwrap();
        assert_eq!(
            super::docx::read_document_xml(&b1).unwrap(),
            super::docx::read_document_xml(&b1b).unwrap(),
            "{name} set_table_element 应幂等（二次套用逐字节稳定）"
        );
        // 读回：tblpr 投影可见 + table 投影表属性行
        let tblpr = inspect_document(
            &b1,
            &InspectRequest { projection: InspectProjection::Tblpr, start: Some(tbl), end: Some(tbl), row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert!(tblpr.content.contains(FILL), "{name} tblpr 投影应含新底纹");
        assert!(tblpr.content.contains("<w:tblPr"), "{name} tblpr 应渲染原文");

        // ---- ② set_cell_format：格级段落+字符格式，文本/块数不变 ----
        let (b2, applied) = super::apply_edits_to_bytes(
            bytes,
            &[super::EditOp::SetCellFormat {
                block: tbl,
                expect_prefix: prefix.clone(),
                row: h_row,
                cell: h_cell,
                paragraph: Some(super::ParaFormat { align: Some("center".into()), ..Default::default() }),
                character: Some(super::CharFormat { bold: Some(true), ..Default::default() }),
                style: None,
            }],
        )
        .unwrap_or_else(|e| panic!("{name} set_cell_format 失败: {e}"));
        assert_eq!(applied.len(), 1);
        assert_untouched_entries_identical(bytes, &b2, "word/document.xml");
        let model2 = super::docx_model::build_document(&xml_dom::parse(&super::docx::read_document_xml(&b2).unwrap()).unwrap());
        assert_eq!(model2.body.len(), model.body.len(), "{name} 块数应守恒");
        let (mut old_t, mut new_t) = (String::new(), String::new());
        blocks_text(&model.body[tbl - 1..tbl], &mut old_t);
        blocks_text(&model2.body[tbl - 1..tbl], &mut new_t);
        assert_eq!(old_t, new_t, "{name} set_cell_format 不应改动文本");
        let Block::Table(t2) = &model2.body[tbl - 1] else { panic!() };
        let cell2 = &t2.rows[h_row - 1].cells[h_cell - 1];
        for b in &cell2.blocks {
            let Block::Paragraph(p) = b else { continue };
            assert_eq!(p.props.alignment.as_deref(), Some("center"), "{name} 格内段落应居中");
            for r in &p.runs {
                assert_eq!(r.props.bold, Some(true), "{name} 格内 run 应加粗");
            }
        }

        // ---- ③ 横并 + 拆分往返：格数/跨度还原，行内非空文本集合守恒 ----
        let (m1, _) = super::apply_edits_to_bytes(
            bytes,
            &[super::EditOp::MergeCells {
                block: tbl,
                expect_prefix: prefix.clone(),
                direction: Some(super::MergeDirection::Horizontal),
                row: h_row,
                cell: h_cell,
                span: Some(2),
                end_row: None,
                end_cell: None,
            }],
        )
        .unwrap_or_else(|e| panic!("{name} merge(horizontal) 失败: {e}"));
        let model_m = super::docx_model::build_document(&xml_dom::parse(&super::docx::read_document_xml(&m1).unwrap()).unwrap());
        let Block::Table(tm) = &model_m.body[tbl - 1] else { panic!() };
        assert_eq!(tm.rows[h_row - 1].cells.len(), t0.rows[h_row - 1].cells.len() - 1, "{name} 横并应少一格");
        assert_eq!(tm.rows[h_row - 1].cells[h_cell - 1].grid_span, Some(2), "{name} 横并跨度应为 2");
        let (s1, _) = super::apply_edits_to_bytes(
            &m1,
            &[super::EditOp::SplitCell {
                block: tbl,
                expect_prefix: prefix.clone(),
                direction: super::MergeDirection::Horizontal,
                row: h_row,
                cell: h_cell,
            }],
        )
        .unwrap_or_else(|e| panic!("{name} split(horizontal) 失败: {e}"));
        let model_s = super::docx_model::build_document(&xml_dom::parse(&super::docx::read_document_xml(&s1).unwrap()).unwrap());
        let Block::Table(ts) = &model_s.body[tbl - 1] else { panic!() };
        let row0 = &t0.rows[h_row - 1];
        let row_s = &ts.rows[h_row - 1];
        assert_eq!(row_s.cells.len(), row0.cells.len(), "{name} 横拆后格数应还原");
        assert!(row_s.cells.iter().all(|c| c.grid_span.is_none()), "{name} 横拆后应无跨度");
        let texts_of = |row: &super::docx_model::TableRow| -> Vec<String> {
            let mut v = Vec::new();
            for c in &row.cells {
                let mut s = String::new();
                blocks_text(&c.blocks, &mut s);
                v.extend(s.split('\n').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()));
            }
            v.sort();
            v
        };
        assert_eq!(texts_of(row0), texts_of(row_s), "{name} 横并拆往返文本集合应守恒");

        // ---- ④ 纵并 + 拆分往返：整表文本投影逐字节相等（内容留原格的语义还原） ----
        let v_span = 2usize;
        let (m2, _) = super::apply_edits_to_bytes(
            bytes,
            &[super::EditOp::MergeCells {
                block: tbl,
                expect_prefix: prefix.clone(),
                direction: Some(super::MergeDirection::Vertical),
                row: 1,
                cell: v_head,
                span: Some(v_span),
                end_row: None,
                end_cell: None,
            }],
        )
        .unwrap_or_else(|e| panic!("{name} merge(vertical) 失败: {e}"));
        let model_v = super::docx_model::build_document(&xml_dom::parse(&super::docx::read_document_xml(&m2).unwrap()).unwrap());
        let Block::Table(tv) = &model_v.body[tbl - 1] else { panic!() };
        assert_eq!(tv.rows[0].cells[v_head - 1].v_merge.as_deref(), Some("restart"), "{name} 纵并头应为 restart");
        assert_eq!(tv.rows[1].cells[v_head - 1].v_merge.as_deref(), Some("continue"), "{name} 纵并续格应为 continue");
        let (s2, _) = super::apply_edits_to_bytes(
            &m2,
            &[super::EditOp::SplitCell {
                block: tbl,
                expect_prefix: prefix.clone(),
                direction: super::MergeDirection::Vertical,
                row: 1,
                cell: v_head,
            }],
        )
        .unwrap_or_else(|e| panic!("{name} split(vertical) 失败: {e}"));
        let model_v2 = super::docx_model::build_document(&xml_dom::parse(&super::docx::read_document_xml(&s2).unwrap()).unwrap());
        let Block::Table(tv2) = &model_v2.body[tbl - 1] else { panic!() };
        assert!(tv2.rows.iter().all(|r| r.cells.iter().all(|c| c.v_merge.is_none())), "{name} 纵拆后应无 vMerge");
        let (mut ta, mut tb) = (String::new(), String::new());
        blocks_text(&model.body[tbl - 1..tbl], &mut ta);
        blocks_text(&model_v2.body[tbl - 1..tbl], &mut tb);
        assert_eq!(ta, tb, "{name} 纵并拆往返整表文本应逐字节还原");
        assert_untouched_entries_identical(&m2, &s2, "word/document.xml");
        ran += 1;
    }
    assert!(ran >= 2, "SDP/SRS 语料应均有干净表可跑（实际 {ran} 份）");
}

// =========================================================================
// S3 五波·定义部件手术（D12）：styles / numbering 族真实语料闭环
// =========================================================================

/// styles 族闭环：styles 投影全量 → 运行时选首个带 outlineLvl 的干净样式 →
/// styledef 原文投影（抄写源）→ pPr/shd 手术 → 幂等 + 其余 w:style 与
/// latentStyles/docDefaults 逐字节不变 + document.xml 逐字节不变（含修订样本
/// 的 INSTALL 不受影响）。目标样式全运行时派生（语料字符串不进代码）。
#[test]
fn corpus_def_style_surgery() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    use super::def_edit::{change_marker, child_attr, root_children};
    use super::styles::parse_styles;
    // 自造片段与值（非语料来源——禁令）
    const SHD_FRAG: &str = r#"<w:shd w:val="clear" w:color="auto" w:fill="EEF3FA"/>"#;
    const FILL: &str = "EEF3FA";

    let mut ran = 0usize;
    for (name, bytes) in [("SDP", &sdp), ("SRS", &srs), ("INSTALL", &install)] {
        // styles 投影全量：表头 + 逐样式行（真实 Word 样式表规模）
        let list = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Styles, start: None, end: None, row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert!(list.content.contains("样式表"), "{name} styles 表头缺失");
        assert!(
            list.content.lines().filter(|l| l.contains("ID=")).count() >= 5,
            "{name} 样式行过少"
        );

        // 运行时选目标：首个带 outlineLvl 且定义子树无修订记录的样式（按 ID 寻址）
        let styles_xml = super::docx::read_entry(bytes, "word/styles.xml")
            .unwrap()
            .unwrap_or_else(|| panic!("{name} 应有 styles.xml"));
        let sheet = parse_styles(&xml_dom::parse(&styles_xml).unwrap());
        let children = root_children(&styles_xml).unwrap();
        let all = sheet.all_styles();
        let target = all
            .iter()
            .filter_map(|s| {
                children
                    .iter()
                    .find(|c| {
                        c.name == "w:style"
                            && child_attr(&styles_xml, c, "w:styleId").as_deref() == Some(s.id.as_str())
                    })
                    .map(|c| (s, c))
            })
            .find(|(s, c)| {
                s.outline_lvl.is_some() && change_marker(&styles_xml[c.start..c.end]).is_none()
            });
        let Some((style_def, _)) = target else { continue };
        let sid = style_def.id.clone();

        // styledef 原文投影：抄写源可见
        let def = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Styledef, start: None, end: None, row: None, cell: None, style: Some(sid.clone()), num_id: None, level: None },
        )
        .unwrap();
        assert!(def.content.contains("<w:style"), "{name} styledef 应渲染原文");
        assert!(
            def.content.contains(&format!("w:styleId=\"{sid}\"")),
            "{name} styledef 应是目标样式"
        );

        // 手术：pPr/shd upsert（目标样式变；其余逐子级不变）
        let op = super::StyleEditOp::SetStyleElement {
            style: sid.clone(),
            container: super::StyleContainer::PPr,
            element: "shd".into(),
            xml: Some(SHD_FRAG.into()),
        };
        let (new_bytes, applied) = super::apply_style_edits_to_bytes(bytes, std::slice::from_ref(&op))
            .unwrap_or_else(|e| panic!("{name} set_style_element 失败: {e}"));
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].op, "set_style_element");
        assert!(
            applied[0].target.as_deref().is_some_and(|t| t.contains("pPr/shd")),
            "{name} 摘要应带 target 定位串"
        );

        // untouched 保真：只有 styles.xml 变（document.xml 逐字节不动）
        assert_untouched_entries_identical(bytes, &new_bytes, "word/styles.xml");

        // 幂等：二次套用逐字节稳定
        let (new_bytes2, _) =
            super::apply_style_edits_to_bytes(&new_bytes, std::slice::from_ref(&op)).unwrap();
        assert_eq!(
            super::docx::read_entry(&new_bytes, "word/styles.xml").unwrap().unwrap(),
            super::docx::read_entry(&new_bytes2, "word/styles.xml").unwrap().unwrap(),
            "{name} set_style_element 应幂等"
        );

        // 同部件逐子级对比：目标样式变、其余 w:style 与 latentStyles/docDefaults
        // 等非样式直接子级逐字节不变
        let new_xml = super::docx::read_entry(&new_bytes, "word/styles.xml").unwrap().unwrap();
        let new_children = root_children(&new_xml).unwrap();
        assert_eq!(children.len(), new_children.len(), "{name} styles 直接子级数应守恒");
        for (a, b) in children.iter().zip(new_children.iter()) {
            let (sa, sb) = (&styles_xml[a.start..a.end], &new_xml[b.start..b.end]);
            let is_target =
                a.name == "w:style" && child_attr(&styles_xml, a, "w:styleId").as_deref() == Some(sid.as_str());
            if is_target {
                assert_ne!(sa, sb, "{name} 目标样式应已改");
                assert!(sb.contains(FILL), "{name} 手术值未落位");
            } else {
                assert_eq!(
                    sa, sb,
                    "{name} 非目标子级应逐字节不变（latentStyles/docDefaults/其余样式）"
                );
            }
        }

        // 读回：styledef 投影见新值；重解析样式计数守恒
        let def2 = inspect_document(
            &new_bytes,
            &InspectRequest { projection: InspectProjection::Styledef, start: None, end: None, row: None, cell: None, style: Some(sid.clone()), num_id: None, level: None },
        )
        .unwrap();
        assert!(def2.content.contains(FILL), "{name} styledef 读回应含新底纹");
        let sheet2 = parse_styles(&xml_dom::parse(&new_xml).unwrap());
        assert_eq!(sheet2.all_styles().len(), sheet.all_styles().len(), "{name} 样式计数应守恒");
        ran += 1;
    }
    assert!(ran >= 2, "SDP/SRS 语料应均有 outline 样式可跑（实际 {ran} 份）");
}

/// numbering 族闭环：numbering 目录投影 → numId+level 下钻原文 →
/// lvl0/lvlRestart 手术（compute_numbers 不读 lvlRestart → 编号值不变）→
/// 幂等 + document.xml 逐字节不变。目标 numId 全运行时派生。
#[test]
fn corpus_def_numbering_surgery() {
    let Some((sdp, srs, install)) = all_corpus() else { return };
    use super::numbering::{compute_numbers, parse_numbering};
    // 自造片段（非语料来源——禁令）
    const RESTART_FRAG: &str = r#"<w:lvlRestart w:val="4"/>"#;

    let mut ran = 0usize;
    for (name, bytes) in [("SDP", &sdp), ("SRS", &srs), ("INSTALL", &install)] {
        let Some(numbering_xml) = super::docx::read_entry(bytes, "word/numbering.xml").unwrap() else {
            continue;
        };
        let catalog = parse_numbering(&xml_dom::parse(&numbering_xml).unwrap());
        let Some((num_id, _)) = catalog.num_entries().into_iter().min_by_key(|(n, _)| *n) else {
            continue;
        };

        // 目录投影：表头 + 目标 numId 段
        let list = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Numbering, start: None, end: None, row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert!(list.content.contains("编号目录"), "{name} numbering 表头缺失");
        assert!(
            list.content.contains(&format!("numId {num_id} ")),
            "{name} 目录应含目标 numId"
        );

        // 级下钻：lvl 0 原文（抄写源）
        let drill = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Numbering, start: None, end: None, row: None, cell: None, style: None, num_id: Some(num_id), level: Some(0) },
        )
        .unwrap();
        assert!(drill.content.contains("<w:lvl"), "{name} 下钻应渲染 lvl 原文");

        // 手术 + 幂等 + untouched
        let op = super::NumberingEditOp::SetNumberingElement {
            num_id,
            level: 0,
            element: "lvlRestart".into(),
            xml: Some(RESTART_FRAG.into()),
        };
        let (new_bytes, applied) = super::apply_numbering_edits_to_bytes(bytes, std::slice::from_ref(&op))
            .unwrap_or_else(|e| panic!("{name} set_numbering_element 失败: {e}"));
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].op, "set_numbering_element");
        assert_untouched_entries_identical(bytes, &new_bytes, "word/numbering.xml");
        let (new_bytes2, _) =
            super::apply_numbering_edits_to_bytes(&new_bytes, std::slice::from_ref(&op)).unwrap();
        assert_eq!(
            super::docx::read_entry(&new_bytes, "word/numbering.xml").unwrap().unwrap(),
            super::docx::read_entry(&new_bytes2, "word/numbering.xml").unwrap().unwrap(),
            "{name} set_numbering_element 应幂等"
        );

        // 编号值不变：同一正文，新旧 catalog 算出的自动编号全表相等
        let new_numbering = super::docx::read_entry(&new_bytes, "word/numbering.xml").unwrap().unwrap();
        let catalog2 = parse_numbering(&xml_dom::parse(&new_numbering).unwrap());
        let doc_xml = super::docx::read_document_xml(bytes).unwrap();
        let model = super::docx_model::build_document(&xml_dom::parse(&doc_xml).unwrap());
        assert_eq!(
            compute_numbers(&model.body, &catalog),
            compute_numbers(&model.body, &catalog2),
            "{name} lvlRestart 手术不应改变自动编号值"
        );

        // 读回：下钻可见新元素
        let drill2 = inspect_document(
            &new_bytes,
            &InspectRequest { projection: InspectProjection::Numbering, start: None, end: None, row: None, cell: None, style: None, num_id: Some(num_id), level: Some(0) },
        )
        .unwrap();
        assert!(drill2.content.contains("lvlRestart"), "{name} 下钻读回应含新元素");
        ran += 1;
    }
    assert!(ran >= 2, "SDP/SRS 语料应均有编号定义可跑（实际 {ran} 份）");
}

/// 合成 *Change 拒改：往真实 styles.xml 的目标样式子树注入自造修订记录元素，
/// set_style_element 必须拒（指路先在 Word 接受/拒绝修订）。纯函数无盘上
/// 副作用——拒改即原字节原样。
#[test]
fn corpus_def_change_guard() {
    let Some((sdp, _srs, _install)) = all_corpus() else { return };
    use super::def_edit::{child_attr, root_children};
    let styles_xml = super::docx::read_entry(&sdp, "word/styles.xml").unwrap().unwrap();
    let children = root_children(&styles_xml).unwrap();
    // 任选一个非自闭合样式，注入自造 rPrChange 到其子树尾部（well-formed）
    let hit = children
        .iter()
        .find(|c| c.name == "w:style" && !c.self_closed && child_attr(&styles_xml, c, "w:styleId").is_some())
        .expect("SDP 应有常规样式");
    let sid = child_attr(&styles_xml, hit, "w:styleId").unwrap();
    let close = "</w:style>";
    let span = &styles_xml[hit.start..hit.end];
    assert!(span.ends_with(close), "非自闭合样式应以 </w:style> 收尾");
    let inner_end = hit.end - close.len();
    let poisoned_xml = format!(
        "{}{}{}{}",
        &styles_xml[..inner_end],
        r#"<w:rPrChange w:id="1"><w:rPr/></w:rPrChange>"#,
        close,
        &styles_xml[hit.end..]
    );
    let poisoned = super::docx_edit::repack_part(&sdp, "word/styles.xml", &poisoned_xml).unwrap();

    let err = super::apply_style_edits_to_bytes(
        &poisoned,
        &[super::StyleEditOp::SetStyleElement {
            style: sid,
            container: super::StyleContainer::RPr,
            element: "b".into(),
            xml: Some("<w:b/>".into()),
        }],
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("受保护"), "实际: {err}");
    assert!(err.contains("修订记录"), "实际: {err}");
}
