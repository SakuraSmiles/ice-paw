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
            &InspectRequest { projection: InspectProjection::Outline, start: None, end: None },
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
            &InspectRequest { projection: InspectProjection::Format, start: None, end: None },
        )
        .unwrap();
        assert_eq!(fmt.range, (1, 50), "{name} format 默认区间");
        assert!(fmt.has_more, "{name} format 应有 has_more");
        assert!(fmt.content.contains("段落格式:"), "{name} format 应有段落属性行");

        // text：带块号正文，区间续读
        let text = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Text, start: Some(2), end: Some(6) },
        )
        .unwrap();
        assert_eq!(text.range, (2, 6));
        assert!(text.content.contains("[2]"), "{name} text 应带块号前缀");
        // 越界错误带总数（报错契约）
        let err = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Text, start: Some(report.total_blocks + 1), end: None },
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
        &InspectRequest { projection: InspectProjection::Outline, start: None, end: None },
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
        &InspectRequest { projection: InspectProjection::Outline, start: None, end: None },
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
    // 三份语料投影里不出现未解析引用形式（numPr 引用全部解析出值/或该段无值回退——
    // 语料 numbering.xml 完整，默认 400 块渲染面内引用形式应为零）
    for bytes in [&sdp, &srs] {
        let report = inspect_document(
            bytes,
            &InspectRequest { projection: InspectProjection::Outline, start: None, end: None },
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
            &InspectRequest { projection: InspectProjection::HeadersFooters, start: None, end: None },
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
        &InspectRequest { projection: InspectProjection::Format, start: Some(1), end: Some(171) },
    )
    .unwrap();
    assert!(fmt.content.contains("〔插入修订〕"), "插入修订 run 未标记");
    assert!(fmt.content.contains("〔删除修订〕"), "删除修订 run 未标记");
    // text 投影剔除删除修订（接受修订后视图）——修订剔除不应清空正文（默认 span=100 块量级仍在）
    let text = inspect_document(
        &install,
        &InspectRequest { projection: InspectProjection::Text, start: Some(1), end: None },
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
            &InspectRequest { projection: InspectProjection::Text, start: Some(t1), end: Some(t2 + 1) },
        )
        .unwrap();
        assert!(seg.content.contains(REPLACED), "{name} t1={t1} 替换未生效:\n{}", seg.content);
        assert!(seg.content.contains(INSERTED), "{name} t2={t2} 后插入未生效:\n{}", seg.content);
        let full = inspect_document(
            &new_bytes,
            &InspectRequest { projection: InspectProjection::Outline, start: None, end: None },
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
