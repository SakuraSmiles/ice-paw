//! `write_docx` 生成引擎（word-capability-roadmap 九波 D16，模板优先）。
//!
//! 路线拍板（D16）：生成 = 模板起步而非从零拼 zip——模板是 Word/WPS 真机产物，
//! 样式质量白送、Word 必然能打开；生成完全复用已验证的手术链（clear_body /
//! insert_paragraph_after / insert_table_after），保真不变式全继承。纯创建
//!（自建默认样式表 + 从零构建包）降级延后项（见 roadmap §八）。
//!
//! 编排（**顺序前向、锚 = 当前末块**，零地址簿记）：
//! 1. 清空模板正文（ClearBody 独占批；块数指纹 = 刚解析的真实值——生成是单次
//!    原子内存链，不存在跨调用陈旧心智）；
//! 2. 清空后仍有块 → 多节模板拒（节尾 sectPr 嵌在段 pPr 内才残留；单节文档
//!    的 sectPr 恒为 body 直属子、不是块，清空后必然 0 块）；
//! 3. body 开头注入裸锚段 `<w:p/>`；
//! 4. 顺序遍历内容块：连续段落聚一批（同锚链式按输入序排列，D14 ⑳ 语义）、
//!    表格各自单独一批（与段落同锚互斥）；锚 = 当前文档末块，只增不减；
//! 5. 末批 DeleteBlock 摘除锚段；
//! 6. 生成自检（复用 validate 引擎的提取口径逐块逐格全量比对）——不过不落盘，
//!    「验证后变异」不变式移植到生成侧。
//!
//! 段落样式恒显式：heading → 候选链（标题 N / heading N / …）解析；普通段 →
//! 正文候选链（正文 / Normal）。显式 pStyle 使产物格式与锚类型完全无关——
//! build_inserted_paragraph 的继承分支（表锚会透出首格 pPr）在本链路不可达。
//!
//! 纯函数：模板字节 + 内容块 → 新文档字节；IO（模板读取/写盘/备份/授权）在
//! mcp::docx_tool 工具壳。

use std::io::Write;

use crate::error::{AppError, AppResult};

use super::docx_edit::{apply_edits_to_bytes, repack_part, EditOp};
use super::docx_pkg::ImagePayload;
use super::docx_validate::{validate_document, AssertSpec, MAX_ASSERTS};
use super::styles::Stylesheet;
use super::{docx, docx_model, styles, xml_dom};

/// 单次生成内容块上限（防滥用；表格内部另有 200×30 引擎上限）。
pub const MAX_WRITE_BLOCKS: usize = 300;

/// 生成内容块（引擎形态；工具层从 wire 格式转换，rows_text 解析在工具层）。
#[derive(Debug, Clone)]
pub enum WriteBlock {
    /// 标题段：level 1-based（1 = 最高级）；样式经候选链解析到模板实际标题样式
    Heading { level: u32, text: String },
    /// 普通段：style 显式样式名（显示名或 ID）；None → 正文样式候选链
    ///（正文 / Normal），都不中则不带样式（docDefaults 兜底）
    Paragraph { text: String, style: Option<String> },
    /// 表格：rows 矩阵（格内 \n=多段）；header 缺省 true（表头加粗+跨页重复）；
    /// table_style 显式表样式名（缺省默认全边框直排）
    Table { rows: Vec<Vec<String>>, header: Option<bool>, table_style: Option<String> },
    /// TOC 域段（D18 十波）：levels 1-9（目录收录的标题深度）；hyperlink 目录
    /// 项超链接。产物 = 裸 fldSimple 单段 + settings 自动置 updateFields——
    /// Word 打开即刷新；WPS 不保证（description 诚实边界：F9 手动刷新）
    Toc { levels: u32, hyperlink: bool },
    /// 图片段（D18 十波）：image 由工具壳 load_image 装载（字节/宽高/格式）；
    /// width_mm 显式宽（毫米，钳版心）；缺省 min(原生像素宽, 版心) 不放大小图
    Image { image: ImagePayload, width_mm: Option<f64> },
}

/// 生成结果摘要（工具壳转 JSON 用）。
#[derive(Debug)]
pub struct GeneratedDoc {
    pub bytes: Vec<u8>,
    pub paragraphs: usize,
    pub tables: usize,
    /// 图片块数（toc 同理独立计数——工具层结果摘要披露）
    pub images: usize,
    /// TOC 域块数
    pub tocs: usize,
}

/// 模板优先生成主入口（纯函数，bytes → bytes）。
pub fn generate_from_template(template: &[u8], blocks: &[WriteBlock]) -> AppResult<GeneratedDoc> {
    if blocks.is_empty() {
        return Err(AppError::Validation(
            "生成块无效: blocks 为空。write_docx 至少一个内容块——纯复制模板请用 copy_file。".into(),
        ));
    }
    if blocks.len() > MAX_WRITE_BLOCKS {
        return Err(AppError::Validation(format!(
            "生成块无效: {} 块（上限 {MAX_WRITE_BLOCKS}）。请拆成多次生成（write_docx 续写\
             前先用 edit_docx 定位末块），或减少单次内容量。",
            blocks.len()
        )));
    }

    // 模板合法性（生成期所有「模板本身的问题」统一 模板无效/模板不支持 家族；
    // 内容问题（未知样式/坏表格）由引擎自然报错）
    let doc_xml = docx::read_document_xml(template).map_err(|e| {
        AppError::Validation(format!(
            "模板无效: 不是可用的 docx 模板（{e}）。请确认模板文件未损坏，或换用内置\
             模板（template=\"report\"）。"
        ))
    })?;
    let dom = xml_dom::parse(&doc_xml).map_err(|e| {
        AppError::Validation(format!("模板无效: 模板 document.xml 解析失败（{e}）。"))
    })?;
    let model = docx_model::build_document(&dom);
    let stylesheet = load_styles(template)?;

    let mut bytes = template.to_vec();
    // 1. 清空正文（n=0 的空模板跳过——ClearBody 对空 body 是 no-op，省一次解析）
    let n = model.body.len();
    if n > 0 {
        let (cleared, _) = apply_edits_to_bytes(&bytes, &[EditOp::ClearBody { expect_blocks: n }])?;
        bytes = cleared;
    }
    // 2. 多节守卫：清空后仍有块 = 节尾 sectPr 嵌在段内
    if block_count(&bytes)? > 0 {
        return Err(AppError::Validation(
            "模板不支持: 该模板是多节文档（分节符内嵌段落），write_docx v1 只支持单节\
             模板。怎么办：copy_file 复制模板后用 edit_docx clear_body + 逐块写入\
             （多节结构原样保留）。".into(),
        ));
    }
    // 3. 注入裸锚段
    bytes = inject_anchor(bytes)?;

    // 4. 顺序前向写入；锚 = 当前末块（只增不减，无偏移数学）
    let mut anchor = 1usize;
    let mut paragraphs = 0usize;
    let mut tables = 0usize;
    let mut images = 0usize;
    let mut tocs = 0usize;
    let mut i = 0usize;
    while i < blocks.len() {
        match &blocks[i] {
            WriteBlock::Table { rows, header, table_style } => {
                // 表格与段落同锚互斥 → 独占一批；expect_prefix 留空（生成链内
                // 的块都是本链刚写的，无跨调用陈旧心智可防）
                let op = EditOp::InsertTableAfter {
                    block: anchor,
                    expect_prefix: String::new(),
                    rows: rows.clone(),
                    header: *header,
                    table_style: table_style.clone(),
                };
                let (nb, _) = apply_edits_to_bytes(&bytes, &[op])?;
                bytes = nb;
                tables += 1;
                anchor += 1;
                i += 1;
            }
            WriteBlock::Toc { levels, hyperlink } => {
                // TOC / 图片 = run-breaker 独占一批（与段落链同锚互斥；单 op 批
                // 锚 +1 前进——正确性优先，全内存无性能焦虑）
                let op = EditOp::InsertTocAfter {
                    block: anchor,
                    expect_prefix: String::new(),
                    levels: *levels,
                    hyperlink: *hyperlink,
                };
                let (nb, _) = apply_edits_to_bytes(&bytes, &[op])?;
                bytes = nb;
                tocs += 1;
                anchor += 1;
                i += 1;
            }
            WriteBlock::Image { image, width_mm } => {
                let op = EditOp::InsertImageAfter {
                    block: anchor,
                    expect_prefix: String::new(),
                    image: image.clone(),
                    width_mm: *width_mm,
                    // 注入占位（zip 层编排覆盖——引擎唯一写入点）
                    rid: String::new(),
                    cx_emu: 0,
                    cy_emu: 0,
                    docpr_id: 0,
                };
                let (nb, _) = apply_edits_to_bytes(&bytes, &[op])?;
                bytes = nb;
                images += 1;
                anchor += 1;
                i += 1;
            }
            _ => {
                // 连续段落（含 heading）聚一批：同锚链式按输入序排列
                let run_len = blocks[i..]
                    .iter()
                    .take_while(|b| {
                        !matches!(
                            b,
                            WriteBlock::Table { .. } | WriteBlock::Toc { .. } | WriteBlock::Image { .. }
                        )
                    })
                    .count();
                let ops: Vec<EditOp> = blocks[i..i + run_len]
                    .iter()
                    .map(|b| {
                        let (text, style) = paragraph_style_of(b, &stylesheet);
                        EditOp::InsertParagraphAfter {
                            block: anchor,
                            expect_prefix: String::new(),
                            text,
                            style,
                        }
                    })
                    .collect();
                let (nb, _) = apply_edits_to_bytes(&bytes, &ops)?;
                bytes = nb;
                paragraphs += run_len;
                anchor += run_len;
                i += run_len;
            }
        }
    }

    // 5. 摘除锚段（我们注入的裸 <w:p/>，确定性安全；空指纹放行）
    let (nb, _) = apply_edits_to_bytes(
        &bytes,
        &[EditOp::DeleteBlock { block: 1, expect_prefix: String::new() }],
    )?;
    bytes = nb;

    // 6. 生成自检（不过 = 引擎 bug，Err 而非数据）
    self_check(&bytes, blocks)?;
    Ok(GeneratedDoc { bytes, paragraphs, tables, images, tocs })
}

/// heading 级别 → 样式名候选链（首中即用）：中文显示名 / 规范 w:name / 英文
/// 显示名 / 裸 ID / 中文无空格变体 / 中文 Word 本地化 styleId（zh Word 里标题
/// 样式 styleId 常是 "1"/"2"…）。全不中 → 传主候选进引擎，「未知样式」报错
/// 自带可用样式清单（零文案重复）。
fn heading_style_candidates(level: u32) -> [String; 6] {
    [
        format!("标题 {level}"),
        format!("heading {level}"),
        format!("Heading {level}"),
        format!("Heading{level}"),
        format!("标题{level}"),
        level.to_string(),
    ]
}

/// 普通段正文样式候选链：中文显示名（WPS 常见）/ 规范名（Word 全系）。
const NORMAL_STYLE_CANDIDATES: [&str; 2] = ["正文", "Normal"];

/// 段落块 → (文本, 显式样式名)。样式恒显式（见模块 doc：继承分支不可达）。
fn paragraph_style_of(block: &WriteBlock, stylesheet: &Stylesheet) -> (String, Option<String>) {
    match block {
        WriteBlock::Heading { level, text } => {
            let candidates = heading_style_candidates(*level);
            let hit = candidates
                .iter()
                .find(|c| stylesheet.id_of(c).is_some())
                .cloned()
                .unwrap_or_else(|| candidates[0].clone());
            (text.clone(), Some(hit))
        }
        WriteBlock::Paragraph { text, style } => {
            let resolved = match style {
                Some(explicit) => Some(explicit.clone()),
                None => NORMAL_STYLE_CANDIDATES
                    .iter()
                    .find(|c| stylesheet.id_of(c).is_some())
                    .map(|c| c.to_string()),
            };
            (text.clone(), resolved)
        }
        WriteBlock::Table { .. } | WriteBlock::Toc { .. } | WriteBlock::Image { .. } => {
            unreachable!("表格/TOC/图片走独立批，不入段落链")
        }
    }
}

/// 当前文档正文块数（独立解析口径，与引擎 locate/model 双闸同源）。
fn block_count(bytes: &[u8]) -> AppResult<usize> {
    let xml = docx::read_document_xml(bytes)?;
    let dom = xml_dom::parse(&xml)?;
    Ok(docx_model::build_document(&dom).body.len())
}

/// 读模板样式表（styles.xml 缺失 → 空表，样式候选链自然全不中）。
fn load_styles(bytes: &[u8]) -> AppResult<Stylesheet> {
    Ok(match docx::read_entry(bytes, "word/styles.xml")? {
        Some(xml) => styles::parse_styles(&xml_dom::parse(&xml)?),
        None => Stylesheet::empty(),
    })
}

/// body 为 0 块时在 body 开头注入裸锚段 `<w:p/>`（无 pPr/rPr，不参与任何格式
/// 继承——段落样式恒显式，锚只是链式插入的定位点）。只重打包 document.xml，
/// 其余 zip entry 逐字节原样（复用引擎 repack 通道）。
fn inject_anchor(bytes: Vec<u8>) -> AppResult<Vec<u8>> {
    if block_count(&bytes)? > 0 {
        return Ok(bytes); // 多节模板在生成主流程已拒；防御性放行
    }
    let xml = docx::read_document_xml(&bytes)?;
    let body_open = xml.find("<w:body").ok_or_else(|| {
        AppError::Validation("模板无效: document.xml 缺少 w:body（不是合法 Word 文档）。".into())
    })?;
    let gt = xml[body_open..]
        .find('>')
        .ok_or_else(|| AppError::Validation("模板无效: w:body 开标签不完整。".into()))?;
    let insert_at = body_open + gt + 1;
    let mut out = String::with_capacity(xml.len() + 8);
    out.push_str(&xml[..insert_at]);
    out.push_str("<w:p/>");
    out.push_str(&xml[insert_at..]);
    repack_part(&bytes, "word/document.xml", &out)
}

/// 生成自检：复用 validate 引擎的提取口径（blocks_text / 网格映射 / 合并头
/// 语义），逐块逐格全量比对「产物 = 请求」。validate 单批上限 50 是给 agent
/// 抽查用的，自检是全量——分块跑完。文本比对走 trim 口径（与断言引擎一致）。
/// 失败 → Err（引擎 bug 语义，不该发生；此时工具壳尚未写盘，无半成品文件）。
fn self_check(bytes: &[u8], blocks: &[WriteBlock]) -> AppResult<()> {
    let mut asserts: Vec<AssertSpec> = Vec::with_capacity(blocks.len() + 1);
    asserts.push(AssertSpec::BlockCount { equals: blocks.len() });
    for (idx, b) in blocks.iter().enumerate() {
        let block_no = idx + 1;
        match b {
            WriteBlock::Heading { text, .. } | WriteBlock::Paragraph { text, .. } => {
                asserts.push(AssertSpec::BlockText {
                    block: block_no,
                    equals: Some(text.trim().to_string()),
                    contains: None,
                    starts_with: None,
                });
            }
            WriteBlock::Table { rows, .. } => {
                asserts.push(AssertSpec::TableShape {
                    block: block_no,
                    rows: Some(rows.len()),
                    cols: Some(rows.first().map_or(0, Vec::len)),
                    style: None,
                });
                for (r, row) in rows.iter().enumerate() {
                    for (c, cell) in row.iter().enumerate() {
                        asserts.push(AssertSpec::CellText {
                            block: block_no,
                            row: r + 1,
                            cell: c + 1,
                            equals: Some(cell.trim().to_string()),
                            contains: None,
                            starts_with: None,
                        });
                    }
                }
            }
            // TOC / 图片：块存在 + 特征断言（fldSimple 指令含 TOC / 恰 1 张图）
            WriteBlock::Toc { .. } => {
                asserts.push(AssertSpec::BlockField {
                    block: block_no,
                    instr_contains: "TOC".into(),
                });
            }
            WriteBlock::Image { .. } => {
                asserts.push(AssertSpec::BlockImage { block: block_no, count: Some(1) });
            }
        }
    }
    let mut failures: Vec<String> = Vec::new();
    for chunk in asserts.chunks(MAX_ASSERTS) {
        let report = validate_document(bytes, chunk)?;
        for f in &report.failures {
            failures.push(format!("{} {}", f.target, f.detail));
        }
    }
    if !failures.is_empty() {
        // 只展开前 5 处防错误文案爆长，余处计数披露
        let preview: Vec<String> = failures.iter().take(5).cloned().collect();
        let more = failures.len() - preview.len();
        let shown = preview.join("；");
        let suffix = if more > 0 { format!("…另 {more} 处") } else { String::new() };
        return Err(AppError::Internal(format!(
            "生成自检失败: 产物与请求内容不一致（{} 处：{shown}{suffix}）。这是引擎缺陷\
             而非输入问题，请反馈；本次调用未写入任何文件。",
            failures.len()
        )));
    }
    Ok(())
}

// =========================================================================
// 内置模板（L1 档位）——代码内建占位（D16：用户用 Word 造正式模板后替换/扩充；
// 演进形态 = 文件表 include_bytes!，一处一行）
// =========================================================================

/// 内置模板档位表：name → 一句话说明（工具层未知档位报错与 description 用）。
pub const BUILTIN_TEMPLATES: &[(&str, &str)] = &[(
    "report",
    "中文报告风——标题黑体（三号/四号/小四）、正文宋体小四 1.5 倍行距、A4 单节",
)];

/// 构建内置模板 docx 字节（OnceLock 缓存，重复调用零重打包）。
pub fn build_builtin_template(name: &str) -> AppResult<Vec<u8>> {
    if !BUILTIN_TEMPLATES.iter().any(|(n, _)| *n == name) {
        let names = BUILTIN_TEMPLATES
            .iter()
            .map(|(n, desc)| format!("{n:?}（{desc}）"))
            .collect::<Vec<_>>()
            .join("、");
        return Err(AppError::Validation(format!(
            "模板无效: 未知内置模板 {name:?}。可用档位: {names}。template 也接受\
             模板绝对路径，或相对 workspace templates/ 目录的路径。"
        )));
    }
    static CACHE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    Ok(CACHE
        .get_or_init(|| builtin_report_docx().expect("内置模板构建不可失败"))
        .clone())
}

/// zip 打包部件（生产路径——docx.rs 读侧已依赖 zip crate，此处写侧同源）。
fn zip_parts(parts: &[(&str, &str)]) -> AppResult<Vec<u8>> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in parts {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .map_err(|e| AppError::Internal(format!("内置模板 zip 写入 {name} 失败: {e}")))?;
        w.write_all(content.as_bytes())
            .map_err(|e| AppError::Internal(format!("内置模板 zip 写入 {name} 失败: {e}")))?;
    }
    let cursor = w
        .finish()
        .map_err(|e| AppError::Internal(format!("内置模板 zip 收尾失败: {e}")))?;
    Ok(cursor.into_inner())
}

/// report 档位：最小合法 docx 五部件（[Content_Types]/根 rels/正文/正文 rels/
/// styles）；正文仅一个空段 + body 直属 sectPr（单节——清空后 0 块的编排前提）。
fn builtin_report_docx() -> AppResult<Vec<u8>> {
    zip_parts(&[
        ("[Content_Types].xml", CONTENT_TYPES_XML),
        ("_rels/.rels", ROOT_RELS_XML),
        ("word/_rels/document.xml.rels", DOC_RELS_XML),
        ("word/document.xml", &report_document_xml()),
        ("word/styles.xml", REPORT_STYLES_XML),
    ])
}

const CONTENT_TYPES_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
    r#"<Default Extension="xml" ContentType="application/xml"/>"#,
    r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
    r#"<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>"#,
    r#"</Types>"#,
);

const ROOT_RELS_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>"#,
    r#"</Relationships>"#,
);

const DOC_RELS_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    r#"</Relationships>"#,
);

/// 正文：空锚段 + body 直属 sectPr（A4 纵向、Word 默认页边距 上下 1440 /
/// 左右 1800 twips）。w:r 声明与引擎/Word 双兼容（未用到的前缀不声明）。
fn report_document_xml() -> String {
    let sect_pr = concat!(
        r#"<w:sectPr>"#,
        r#"<w:pgSz w:w="11906" w:h="16838"/>"#,
        r#"<w:pgMar w:top="1440" w:right="1800" w:bottom="1440" w:left="1800" w:header="851" w:footer="992" w:gutter="0"/>"#,
        r#"<w:cols w:space="425"/>"#,
        r#"<w:docGrid w:type="lines" w:linePitch="312"/>"#,
        r#"</w:sectPr>"#,
    );
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#,
            r#" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
            r#"<w:body><w:p/>{sect_pr}</w:body></w:document>"#,
        ),
        sect_pr = sect_pr,
    )
}

/// 样式表：docDefaults（宋体/Times 五号 + 1.5 倍行距）+ Normal（小四）+
/// Heading1-3（黑体加粗 16/14/12pt，规范 styleId/heading N 名）+ TableGrid
///（单线全边框，canonical 名 "Table Grid"，中文 Word 显示「网格型」）。
const REPORT_STYLES_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
    r#"<w:docDefaults><w:rPrDefault><w:rPr>"#,
    r#"<w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体" w:cs="Times New Roman"/>"#,
    r#"<w:sz w:val="21"/><w:szCs w:val="21"/>"#,
    r#"<w:lang w:val="en-US" w:eastAsia="zh-CN" w:bidi="ar-SA"/>"#,
    r#"</w:rPr></w:rPrDefault>"#,
    r#"<w:pPrDefault><w:pPr><w:spacing w:line="360" w:lineRule="auto"/></w:pPr></w:pPrDefault>"#,
    r#"</w:docDefaults>"#,
    // Normal（正文）：宋体小四（sz 24 = 12pt）、1.5 倍行距——报告风正文基调
    r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal">"#,
    r#"<w:name w:val="Normal"/><w:qFormat/>"#,
    r#"<w:pPr><w:spacing w:line="360" w:lineRule="auto"/></w:pPr>"#,
    r#"<w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:eastAsia="宋体"/>"#,
    r#"<w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr>"#,
    r#"</w:style>"#,
    // Heading1：黑体三号（32 半点 = 16pt）
    r#"<w:style w:type="paragraph" w:styleId="Heading1">"#,
    r#"<w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>"#,
    r#"<w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr>"#,
    r#"<w:rPr><w:rFonts w:eastAsia="黑体"/><w:b/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr>"#,
    r#"</w:style>"#,
    // Heading2：黑体四号（28 半点 = 14pt）
    r#"<w:style w:type="paragraph" w:styleId="Heading2">"#,
    r#"<w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>"#,
    r#"<w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="200" w:after="100"/><w:outlineLvl w:val="1"/></w:pPr>"#,
    r#"<w:rPr><w:rFonts w:eastAsia="黑体"/><w:b/><w:sz w:val="28"/><w:szCs w:val="28"/></w:rPr>"#,
    r#"</w:style>"#,
    // Heading3：黑体小四（24 半点 = 12pt）
    r#"<w:style w:type="paragraph" w:styleId="Heading3">"#,
    r#"<w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/>"#,
    r#"<w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="160" w:after="80"/><w:outlineLvl w:val="2"/></w:pPr>"#,
    r#"<w:rPr><w:rFonts w:eastAsia="黑体"/><w:b/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr>"#,
    r#"</w:style>"#,
    // TableGrid：单线全边框（sz 4 = 0.5pt）
    r#"<w:style w:type="table" w:styleId="TableGrid">"#,
    r#"<w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:qFormat/>"#,
    r#"<w:tblPr><w:tblBorders>"#,
    r#"<w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/>"#,
    r#"<w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/>"#,
    r#"<w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/>"#,
    r#"<w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/>"#,
    r#"<w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/>"#,
    r#"<w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/>"#,
    r#"</w:tblBorders></w:tblPr>"#,
    r#"</w:style>"#,
    // TableNormal：表样式基链（TableGrid basedOn 目标；Word 内建同款最小形态）
    r#"<w:style w:type="table" w:default="1" w:styleId="TableNormal">"#,
    r#"<w:name w:val="Normal Table"/>"#,
    r#"<w:tblPr><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>"#,
    r#"<w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr>"#,
    r#"</w:style>"#,
    r#"</w:styles>"#,
);

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成单节模板（带样式表）：body = 标题段 + 正文段 + body 直属 sectPr。
    fn synth_template(body_blocks: &str, styles_xml: &str) -> Vec<u8> {
        let document = format!(
            concat!(
                r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
                r#"<w:body>{body_blocks}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr></w:body></w:document>"#,
            ),
            body_blocks = body_blocks,
        );
        zip_parts_for_test(&[
            ("[Content_Types].xml", "<Types/>"),
            ("word/document.xml", &document),
            ("word/styles.xml", styles_xml),
        ])
    }

    /// 测试侧 zip 打包（生产 build_builtin_template 的 zip_parts 复用）。
    fn zip_parts_for_test(parts: &[(&str, &str)]) -> Vec<u8> {
        zip_parts(parts).expect("zip 打包不可失败")
    }

    const SYNTH_STYLES: &str = concat!(
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>"#,
        r#"<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:qFormat/></w:style>"#,
        r#"<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:qFormat/></w:style>"#,
        r#"</w:styles>"#,
    );

    fn heading(level: u32, text: &str) -> WriteBlock {
        WriteBlock::Heading { level, text: text.into() }
    }
    fn para(text: &str) -> WriteBlock {
        WriteBlock::Paragraph { text: text.into(), style: None }
    }
    fn table(rows: &[&[&str]]) -> WriteBlock {
        WriteBlock::Table {
            rows: rows.iter().map(|r| r.iter().map(|c| c.to_string()).collect()).collect(),
            header: None,
            table_style: None,
        }
    }

    /// blocks_text 对表格块的投影 = 逐格段落文本拼接（格间无分隔、段尾 \n）。
    fn table_text(rows: &[&[&str]]) -> String {
        let mut s = String::new();
        for row in rows {
            for c in *row {
                s.push_str(c);
                s.push('\n');
            }
        }
        s.trim().to_string()
    }

    /// 产物逐块断言辅助：(块号, 文本, 是否表)。全量 block_text/cell_text 由
    /// self_check 在生产路径兜住；这里只做产物的轻量结构复核。
    fn outline_of(bytes: &[u8]) -> Vec<(String, bool)> {
        let xml = docx::read_document_xml(bytes).unwrap();
        let dom = xml_dom::parse(&xml).unwrap();
        let model = docx_model::build_document(&dom);
        model
            .body
            .iter()
            .map(|b| {
                let mut text = String::new();
                docx_model::blocks_text(std::slice::from_ref(b), &mut text);
                let is_table = matches!(b, docx_model::Block::Table { .. });
                (text.trim().to_string(), is_table)
            })
            .collect()
    }

    #[test]
    fn generates_mixed_sequence_in_order() {
        let tpl = synth_template(
            r#"<w:p><w:r><w:t>旧标题</w:t></w:r></w:p><w:p><w:r><w:t>旧正文</w:t></w:r></w:p>"#,
            SYNTH_STYLES,
        );
        let blocks = vec![
            heading(1, "总标题"),
            para("引言段。"),
            table(&[&["列A", "列B"], &["1", "2"]]),
            para("表后段。"),
            heading(2, "小结"),
            table(&[&["x"], &["y"], &["z"]]),
            para("尾段。"),
        ];
        let out = generate_from_template(&tpl, &blocks).unwrap();
        assert_eq!(out.paragraphs, 5); // 2 heading + 3 段（headings 计入 paragraphs）
        assert_eq!(out.tables, 2);
        let outline = outline_of(&out.bytes);
        assert_eq!(
            outline,
            vec![
                ("总标题".into(), false),
                ("引言段。".into(), false),
                (table_text(&[&["列A", "列B"], &["1", "2"]]), true),
                ("表后段。".into(), false),
                ("小结".into(), false),
                (table_text(&[&["x"], &["y"], &["z"]]), true),
                ("尾段。".into(), false),
            ]
        );
        // 自检已在 generate 内跑过（逐格文本全量）——到达这里即全对
    }

    #[test]
    fn builtin_template_generates_and_carries_heading_styles() {
        let tpl = build_builtin_template("report").unwrap();
        let blocks = vec![
            heading(1, "季度报告"),
            para("正文第一段。"),
            table(&[&["项", "值"], &["收入", "100"]]),
        ];
        let out = generate_from_template(&tpl, &blocks).unwrap();
        // 产物 document.xml 里标题段挂 Heading1、正文段挂 Normal（显式样式链）
        let xml = docx::read_document_xml(&out.bytes).unwrap();
        assert!(xml.contains(r#"<w:pStyle w:val="Heading1"/>"#), "标题段应挂 Heading1");
        assert!(xml.contains(r#"<w:pStyle w:val="Normal"/>"#), "正文段应显式挂 Normal");
        // 保留 body 直属 sectPr（页面设置不丢）
        assert!(xml.contains("<w:sectPr>"));
    }

    #[test]
    fn empty_body_template_also_works() {
        // 无内容块的模板（body 只有 sectPr）——n=0 跳过 clear，锚注入路径相同
        let tpl = synth_template("", SYNTH_STYLES);
        let out = generate_from_template(&tpl, &[para("唯一段")]).unwrap();
        let outline = outline_of(&out.bytes);
        assert_eq!(outline, vec![("唯一段".into(), false)]);
    }

    #[test]
    fn anchor_removed_from_output() {
        let tpl = synth_template(r#"<w:p><w:r><w:t>旧</w:t></w:r></w:p>"#, SYNTH_STYLES);
        let out = generate_from_template(&tpl, &[para("首段")]).unwrap();
        let xml = docx::read_document_xml(&out.bytes).unwrap();
        // 锚段已删：body 内不再有裸 <w:p/>（生成的段都带 pPr/text）
        assert!(!xml.contains("<w:p/>"), "锚段应已删除");
    }

    #[test]
    fn multi_section_template_rejected() {
        // 节尾 sectPr 嵌在段 pPr 内 = 多节 → 清空后残留块 → 拒
        let tpl = synth_template(
            r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr></w:pPr></w:p><w:p><w:r><w:t>尾节段</w:t></w:r></w:p>"#,
            SYNTH_STYLES,
        );
        let err = generate_from_template(&tpl, &[para("x")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("参数校验失败: 模板不支持:"), "实际: {msg}");
    }

    #[test]
    fn empty_and_overflow_blocks_rejected() {
        let tpl = build_builtin_template("report").unwrap();
        let err = generate_from_template(&tpl, &[]).unwrap_err();
        assert!(err.to_string().starts_with("参数校验失败: 生成块无效:"));
        let many: Vec<WriteBlock> = (0..MAX_WRITE_BLOCKS + 1).map(|i| para(&format!("p{i}"))).collect();
        let err = generate_from_template(&tpl, &many).unwrap_err();
        assert!(err.to_string().starts_with("参数校验失败: 生成块无效:"));
    }

    #[test]
    fn heading_style_missing_falls_to_engine_error() {
        // 模板样式表缺 Heading3 → 候选链全不中 → 引擎「未知样式」报错（带清单）
        let tpl = synth_template(r#"<w:p/>"#, SYNTH_STYLES);
        let err = generate_from_template(&tpl, &[heading(3, "三级标题")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("未知样式"), "实际: {msg}");
        assert!(msg.contains("标题 3"), "报错应指名主候选: {msg}");
    }

    #[test]
    fn zh_display_name_heading_resolves() {
        // 候选链首项「标题 2」（中文显示名）命中——styleId/英文名模板也能落
        let styles = concat!(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
            r#"<w:style w:type="paragraph" w:default="1" w:styleId="a0"><w:name w:val="Normal"/></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="2"><w:name w:val="heading 2"/></w:style>"#,
            r#"</w:styles>"#,
        );
        let tpl = synth_template(r#"<w:p/>"#, styles);
        let out = generate_from_template(&tpl, &[heading(2, "二级")]).unwrap();
        let xml = docx::read_document_xml(&out.bytes).unwrap();
        assert!(xml.contains(r#"<w:pStyle w:val="2"/>"#), "应解析到实际 styleId=2");
    }

    #[test]
    fn self_check_catches_mismatch() {
        // 直测自检：产物块数与请求不符 → 生成自检失败
        let tpl = build_builtin_template("report").unwrap();
        let out = generate_from_template(&tpl, &[para("甲"), para("乙")]).unwrap();
        assert!(self_check(&out.bytes, &[para("甲")]).is_err());
        assert!(self_check(&out.bytes, &[para("甲"), para("乙")]).is_ok());
    }

    #[test]
    fn more_than_fifty_asserts_chunked_in_self_check() {
        // 12×12=144 格表 + 首尾段 → 自检断言远超 50，分块口径下仍全过
        let tpl = build_builtin_template("report").unwrap();
        let rows: Vec<Vec<String>> =
            (0..12).map(|r| (0..12).map(|c| format!("r{r}c{c}")).collect()).collect();
        let blocks = vec![para("前"), WriteBlock::Table { rows, header: None, table_style: None }, para("后")];
        let out = generate_from_template(&tpl, &blocks).unwrap();
        assert_eq!(out.tables, 1);
    }

    #[test]
    fn builtin_template_is_valid_single_section_docx() {
        let tpl = build_builtin_template("report").unwrap();
        // 0 内容块（body 仅空段 + 直属 sectPr）——编排前提成立
        assert_eq!(block_count(&tpl).unwrap(), 1);
        // 样式反查命中：标题三级 + 正文 + 表样式。内置模板用规范 w:name
        //（heading 1 等——真机 Word 同形，中文 UI 自行本地化显示「标题 1」；
        // 写中文名反而可能被 Word 当自定义样式）。「标题 N」中文别名由
        // 候选链兜底（zh_display_name_heading_resolves 测）。
        let styles = load_styles(&tpl).unwrap();
        for name in ["heading 1", "heading 2", "heading 3", "Heading3", "Normal"] {
            assert!(styles.id_of(name).is_some(), "{name} 应可反查");
        }
        assert!(styles.table_style_id("Table Grid").is_some());
        // 未知档位名报错列可用清单
        let err = build_builtin_template("nope").unwrap_err();
        assert!(err.to_string().starts_with("参数校验失败: 模板无效:"));
        assert!(err.to_string().contains("report"));
    }

    #[test]
    fn not_a_docx_template_rejected() {
        let err = generate_from_template(b"not a zip", &[para("x")]).unwrap_err();
        assert!(err.to_string().starts_with("参数校验失败: 模板无效:"));
    }
}
