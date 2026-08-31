//! `docx_pkg` —— docx 包级增补通道（word-capability-roadmap 十波 D18）。
//!
//! 既有手术引擎（docx_edit）只重打包 `word/document.xml` 一个部件；图片插入与
//! TOC 需要动**包结构**：新增 media entry、登记关系（rels）、补内容类型
//!（[Content_Types].xml）、置打开时刷新域（settings.xml）。本模块提供这层基建。
//!
//! 核心不变式（D17「CT/rels 永不重写只裁剪/增补」的推进版）：
//! - **只增补、绝不重编号**——既有 rId / media 文件 / CT 条目原字节不动；新资源
//!   名一律「扫原包现有编号 → max+1 递增」，撞名跳过取下一号；
//! - **未变更的部件不进 replacements**（整个 entry 经 raw_copy_file 逐字节保真，
//!   corpus untouched 闸的地基）——CT 已有 Default / settings 已有 updateFields
//!   时直接跳过，不产生等值重写；
//! - 替换件缺失**显式 Err**（repack_part 对缺失 part 静默跳过是陷阱，此处泛化版
//!   必须报「内部 bug」——扫描与重打包之间包不可能变，缺失即程序错误）。
//!
//! XML 层纯函数（rels/CT/settings 构造、drawing/TOC 段构造、EMU 数学）；显式 IO
//! 边界仅两处：`load_image`（读图片源文件，工具壳调）与 `repack_package`（zip
//! 重打包）。

use std::collections::HashSet;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use crate::error::{AppError, AppResult};

use super::xml_dom;

// =========================================================================
// 图片装载（工具壳调；读源文件 + 格式/尺寸/大小闸）
// =========================================================================

/// 图片源大小上限（10 MiB——内嵌大图显著撑大 docx，超限先压缩）。
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// 已装载待插入的图片（write_docx image 块 / edit_docx insert_image_after 的
/// 载荷）。`ext` 与 media 命名、CT Default 一体联动，只产 png/jpeg。
#[derive(Debug, Clone)]
pub struct ImagePayload {
    pub bytes: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
    /// "png" | "jpeg"（media 扩展名与 CT Extension 同值）
    pub ext: &'static str,
}

/// 读取并校验图片源：绝对路径直读；相对路径挂 workspace（无 workspace 报错——
/// 相对路径无锚点）。格式仅 PNG/JPEG，超 10 MiB 拒，全部错误挂 `图片无效:` 家族。
pub fn load_image(path: &str, workspace: Option<&str>) -> AppResult<ImagePayload> {
    let resolved: std::path::PathBuf = if Path::new(path).is_absolute() {
        path.into()
    } else {
        match workspace {
            Some(ws) => Path::new(ws).join(path),
            None => {
                return Err(AppError::Validation(format!(
                    "图片无效: 相对路径 {path:?} 需要会话 workspace 才能定位。\
                     怎么办：改用图片的绝对路径。"
                )))
            }
        }
    };
    let bytes = std::fs::read(&resolved).map_err(|e| {
        AppError::Validation(format!(
            "图片无效: 读取 {} 失败（{e}）。请确认路径存在且是 PNG/JPEG 图片文件。",
            resolved.display()
        ))
    })?;
    probe_image(bytes)
}

/// 图片字节校验/探测（纯）：magic 识别格式（PNG/JPEG）+ 读宽高 + 大小闸。
pub(super) fn probe_image(bytes: Vec<u8>) -> AppResult<ImagePayload> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AppError::Validation(format!(
            "图片无效: 图片 {:.1} MB 超上限 10 MB。\
             怎么办：先压缩或降分辨率再插入（内嵌大图会显著撑大文档）。",
            bytes.len() as f64 / 1048576.0
        )));
    }
    let reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| {
            AppError::Validation(format!(
                "图片无效: 无法识别图片格式（{e}）。仅支持 PNG / JPEG。"
            ))
        })?;
    let ext = match reader.format() {
        Some(image::ImageFormat::Png) => "png",
        Some(image::ImageFormat::Jpeg) => "jpeg",
        _ => {
            return Err(AppError::Validation(
                "图片无效: 不支持的图片格式（仅支持 PNG / JPEG）。\
                 怎么办：把图片另存为 PNG 或 JPEG 后重试。"
                    .into(),
            ))
        }
    };
    let (w, h) = reader.into_dimensions().map_err(|e| {
        AppError::Validation(format!(
            "图片无效: 图片数据解码失败（{e}）——文件可能已损坏。\
             怎么办：用看图软件重新导出该图片。"
        ))
    })?;
    if w == 0 || h == 0 {
        return Err(AppError::Validation(
            "图片无效: 图片宽或高为 0，不是可插入的图片。".into(),
        ));
    }
    Ok(ImagePayload { bytes, width_px: w, height_px: h, ext })
}

// =========================================================================
// 包级增补计划（扫原包定名 → 只增补）
// =========================================================================

/// 单图分配结果（zip 层编排注入回 EditOp 的 rId / media 名）。
#[derive(Debug)]
pub(super) struct ImageAlloc {
    pub rid: String,
    /// zip 内完整路径（如 `word/media/image4.png`）
    pub media_name: String,
}

/// 包级增补计划：replacements = 替换的文本部件（原 entry 必须存在）；appends =
/// 新增二进制部件（media）。**未变更的部件不进本表**（保 untouched 逐字节）。
#[derive(Default, Debug)]
pub(super) struct PkgAdditions {
    pub replacements: Vec<(String, String)>,
    pub appends: Vec<(String, Vec<u8>)>,
}

/// 规划包级增补：images 按批内操作序（allocs[i] 一一对应）；has_toc = 批内含
/// TOC 插入（settings 置 updateFields）。无图且无 TOC → 空计划（连 zip 都不打开，
/// 纯段落批次零开销零风险）。
pub(super) fn plan_package_additions(
    bytes: &[u8],
    images: &[ImagePayload],
    has_toc: bool,
) -> AppResult<(Vec<ImageAlloc>, PkgAdditions)> {
    if images.is_empty() && !has_toc {
        return Ok((Vec::new(), PkgAdditions::default()));
    }
    let cur = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cur)
        .map_err(|e| AppError::Internal(format!("docx 不是合法 ZIP 容器: {e}")))?;
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    // 家族前缀（错误文案用）：有图挂图片族（图片问题优先暴露），纯 TOC 挂 TOC 族
    let family = if images.is_empty() { "TOC 插入无效" } else { "图片插入无效" };

    let mut allocs = Vec::new();
    let mut additions = PkgAdditions::default();

    // ---- rels：登记图片关系（原条目字节不动，新条目插在 </Relationships> 前）----
    if !images.is_empty() {
        let rels = read_zip_entry(&mut archive, "word/_rels/document.xml.rels")?.ok_or_else(
            || {
                AppError::Validation(format!(
                    "{family}: 文档包缺少 word/_rels/document.xml.rels 关系部件，\
                     无法登记图片关系。怎么办：确认目标文件是 Word/WPS 正常保存的 .docx。"
                ))
            },
        )?;
        let dom = xml_dom::parse(&rels).map_err(|e| {
            AppError::Validation(format!(
                "{family}: 关系部件解析失败（{e}）。怎么办：确认目标文件未损坏。"
            ))
        })?;
        let mut max_rid = 0u32;
        let mut used_ids: HashSet<String> = HashSet::new();
        for rel in dom.child_elements().filter(|e| e.name == "Relationship") {
            if let Some(id) = rel.attr("Id") {
                used_ids.insert(id.to_string());
                if let Some(n) = id.strip_prefix("rId").and_then(|s| s.parse::<u32>().ok()) {
                    max_rid = max_rid.max(n);
                }
            }
        }
        // media 现有编号扫描（word/media/imageN.* 的 N 取 max；非 image 前缀的
        // media 文件不参与编号但占名——分配时撞名跳过）
        let mut max_k = 0u32;
        for name in &names {
            let Some(rest) = name.strip_prefix("word/media/image") else { continue };
            let digits_end = rest.find('.').unwrap_or(rest.len());
            if let Some(n) = rest[..digits_end].parse::<u32>().ok() {
                max_k = max_k.max(n);
            }
        }
        let mut next_rid = max_rid + 1;
        let mut next_k = max_k + 1;
        let mut new_rels = String::new();
        for img in images {
            // rId 取空号（防非数字形态 Id 占位）；media 名撞现有 entry 跳过
            let rid = loop {
                let cand = format!("rId{next_rid}");
                next_rid += 1;
                if !used_ids.contains(&cand) {
                    break cand;
                }
            };
            let media_name = loop {
                let cand = format!("word/media/image{next_k}.{}", img.ext);
                next_k += 1;
                if !names.iter().any(|n| *n == cand) {
                    break cand;
                }
            };
            new_rels.push_str(&format!(
                r#"<Relationship Id="{rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="{}"/>"#,
                media_name.strip_prefix("word/").unwrap_or(&media_name),
            ));
            additions.appends.push((media_name.clone(), img.bytes.clone()));
            allocs.push(ImageAlloc { rid, media_name });
        }
        let extended = append_before_close(&rels, "</Relationships>", &new_rels)?;
        additions
            .replacements
            .push(("word/_rels/document.xml.rels".into(), extended));
    }

    // ---- settings：TOC 打开自刷（upsert updateFields；已有则跳过不重写）----
    let settings = read_zip_entry(&mut archive, "word/settings.xml")?;
    if has_toc {
        let new_settings = match settings.as_deref() {
            Some(s) => upsert_update_fields(s)?, // None = 已置，不重写
            None => Some(MINIMAL_SETTINGS_XML.to_string()),
        };
        if let Some(s) = new_settings {
            additions.replacements.push(("word/settings.xml".into(), s));
        }
    }

    // ---- CT：缺才补（图片 Default / settings Override），一项不缺则不重写 ----
    if !images.is_empty() || has_toc {
        let ct = read_zip_entry(&mut archive, "[Content_Types].xml")?.ok_or_else(|| {
            AppError::Validation(format!(
                "{family}: 文档包缺少 [Content_Types].xml。\
                 怎么办：确认目标文件是 Word/WPS 正常保存的 .docx。"
            ))
        })?;
        let mut insertions = String::new();
        if !images.is_empty() {
            let existing = existing_default_exts(&ct)?;
            let mut added: HashSet<&str> = HashSet::new();
            for img in images {
                if added.contains(img.ext) {
                    continue;
                }
                added.insert(img.ext);
                if !existing.iter().any(|e| e.eq_ignore_ascii_case(img.ext)) {
                    insertions.push_str(&format!(
                        r#"<Default Extension="{}" ContentType="{}"/>"#,
                        img.ext,
                        content_type_of(img.ext)
                    ));
                }
            }
        }
        // settings Override：部件新建时必然缺；已有 settings 却无 Override 的
        // 畸形包也补上（替换件已在 replacements，无声明 Word 可能拒载）
        if has_toc && !ct.contains("PartName=\"/word/settings.xml\"") {
            insertions.push_str(
                r#"<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>"#,
            );
        }
        if !insertions.is_empty() {
            let out = append_before_close(&ct, "</Types>", &insertions)?;
            additions.replacements.push(("[Content_Types].xml".into(), out));
        }
    }

    Ok((allocs, additions))
}

/// CT 里已声明的 Default 扩展名集合（小写归一）。
fn existing_default_exts(ct: &str) -> AppResult<Vec<String>> {
    let dom = xml_dom::parse(ct)
        .map_err(|e| AppError::Internal(format!("内容类型部件解析失败: {e}")))?;
    Ok(dom
        .child_elements()
        .filter(|e| e.name == "Default")
        .filter_map(|e| e.attr("Extension"))
        .map(|ext| ext.to_ascii_lowercase())
        .collect())
}

fn content_type_of(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpeg" => "image/jpeg",
        _ => "application/octet-stream",
    }
}

/// 在 `close_tag`（如 `</Relationships>`）之前插入文本；找不到 = 内部 bug
/// （扫描刚读过该部件，结构不会变）。
fn append_before_close(xml: &str, close_tag: &str, insertion: &str) -> AppResult<String> {
    let pos = xml.rfind(close_tag).ok_or_else(|| {
        AppError::Internal(format!(
            "包部件手术失败: 找不到 {close_tag}（内部 bug，未写盘）"
        ))
    })?;
    let mut out = String::with_capacity(xml.len() + insertion.len());
    out.push_str(&xml[..pos]);
    out.push_str(insertion);
    out.push_str(&xml[pos..]);
    Ok(out)
}

// =========================================================================
// settings.xml 手术（TOC 打开自刷）
// =========================================================================

/// 最小合法 settings 部件（包内无 settings.xml 时新建 + CT 补 Override）。
const MINIMAL_SETTINGS_XML: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
    r#"<w:updateFields w:val="true"/>"#,
    r#"</w:settings>"#,
);

/// ECMA-376 CT_Settings 序中 updateFields **之后**的元素（首个出现者之前插入；
/// 取文档序最小位置，与 schema 序一致）。m:mathPr 跨前缀、w14/w15 是现代 Word
/// 尾部扩展——真实文件几乎必含其中之一，全无才退到 `</w:settings>` 前。
const SETTINGS_LATER_ANCHORS: [&str; 19] = [
    "<w:hdrShapeDefaults", "<w:footnotePr", "<w:endnotePr", "<w:compat", "<w:docVars",
    "<w:rsids", "<m:mathPr", "<w:attachedSchema", "<w:themeFontLang", "<w:clrSchemeMapping",
    "<w:doNotIncludeSubdocsInStats", "<w:doNotAutoCompressPictures", "<w:forceUpgrade",
    "<w:smartTagType", "<w:shapeDefaults", "<w:decimalSymbol", "<w:listSeparator",
    "<w14:docId", "<w15:docId",
];

/// 幂等 upsert `<w:updateFields w:val="true"/>`：已存在（任意 val）→ Ok(None)
/// 不动；缺失 → 插入到 schema 合法位。settings 结构异常 → Err（TOC 族）。
fn upsert_update_fields(settings: &str) -> AppResult<Option<String>> {
    if settings.contains("<w:updateFields") {
        return Ok(None);
    }
    let insertion = r#"<w:updateFields w:val="true"/>"#;
    let at = SETTINGS_LATER_ANCHORS
        .iter()
        .filter_map(|a| find_element_open(settings, a))
        .min()
        .or_else(|| settings.rfind("</w:settings>"));
    let Some(pos) = at else {
        return Err(AppError::Validation(
            "TOC 插入无效: word/settings.xml 结构异常（找不到 w:settings 闭合标签）。\
             怎么办：确认目标文件是 Word/WPS 正常保存的 .docx。"
                .into(),
        ));
    };
    let mut out = String::with_capacity(settings.len() + insertion.len());
    out.push_str(&settings[..pos]);
    out.push_str(insertion);
    out.push_str(&settings[pos..]);
    Ok(Some(out))
}

/// 找 `<prefix_tag` 的元素开标签位置（后随字符须是 `>` / 空白 / `/`——防
/// `<w:compat` 撞 `<w:compatSetting` 的前缀碰撞）。
fn find_element_open(xml: &str, prefix_tag: &str) -> Option<usize> {
    let mut from = 0usize;
    loop {
        let pos = from + xml[from..].find(prefix_tag)?;
        match xml[pos + prefix_tag.len()..].chars().next() {
            Some('>') | Some(' ') | Some('/') => return Some(pos),
            _ => from = pos + prefix_tag.len(),
        }
    }
}

// =========================================================================
// 容器重打包（repack_part 泛化：多部件替换 + 二进制追加）
// =========================================================================

/// 重打包 docx：`replacements` 的每个部件替换为新内容（**必须已存在**，缺失 =
/// 内部 bug 显式 Err——repack_part 对缺失 part 静默跳过是陷阱）；`appends` 追加
/// 新 entry（**不得与现有重名**）；其余 entry 经 raw_copy_file 逐字节保真。
pub(super) fn repack_package(
    bytes: &[u8],
    replacements: &[(String, String)],
    appends: &[(String, Vec<u8>)],
) -> AppResult<Vec<u8>> {
    let cur = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cur)
        .map_err(|e| AppError::Internal(format!("docx 不是合法 ZIP 容器: {e}")))?;
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    for (part, _) in replacements {
        if !names.iter().any(|n| n == part) {
            return Err(AppError::Internal(format!(
                "重打包部件缺失: {part} 不在包内（内部 bug，未写盘）"
            )));
        }
    }
    for (name, _) in appends {
        if names.iter().any(|n| n == name) {
            return Err(AppError::Internal(format!(
                "重打包追加件已存在: {name}（内部 bug，未写盘）"
            )));
        }
    }

    let mut w = zip::ZipWriter::new(Cursor::new(Vec::<u8>::with_capacity(bytes.len())));
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| AppError::Internal(format!("docx 内读取 entry 失败: {e}")))?;
        let name = entry.name().to_string();
        if let Some((_, content)) = replacements.iter().find(|(p, _)| *p == name) {
            // 逐 entry 借用冲突：目标部件先收名，循环外统一写
            drop(entry);
            w.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
                .map_err(|e| AppError::Internal(format!("重打包 {name} 失败: {e}")))?;
            w.write_all(content.as_bytes()).map_err(AppError::Io)?;
        } else {
            w.raw_copy_file(entry)
                .map_err(|e| AppError::Internal(format!("重打包 entry {name} 失败: {e}")))?;
        }
    }
    for (name, content) in appends {
        w.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
            .map_err(|e| AppError::Internal(format!("重打包追加 {name} 失败: {e}")))?;
        w.write_all(content).map_err(AppError::Io)?;
    }
    let out = w
        .finish()
        .map_err(|e| AppError::Internal(format!("docx 重打包收尾失败: {e}")))?;
    Ok(out.into_inner())
}

/// 从打开的 archive 读任意文本部件（缺失 → None；容器/编码问题 → Err）。
fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> AppResult<Option<String>> {
    let mut buf = Vec::new();
    let mut entry = match archive.by_name(name) {
        Ok(e) => e,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => {
            return Err(AppError::Internal(format!("docx 内读取 {name} 失败: {e}")));
        }
    };
    entry.read_to_end(&mut buf).map_err(AppError::Io)?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

// =========================================================================
// drawing / TOC 段构造（命名空间内联声明，不依赖文档根声明集）
// =========================================================================

/// 构造内联图片段（wp:inline；wp/a/pic 命名空间内联声明在各子树上，r 声明在
/// a:blip 上——插进任何 document.xml 都自洽）。`rid` 由包层分配注入；
/// `cx/cy` 为 EMU 尺寸；`docpr_id` 全文档唯一非零（next_docpr_id + 序号）；
/// `name` 是 docPr 显示名（调用方生成的安全字面量，不做转义）。
pub(super) fn build_drawing_paragraph(
    rid: &str,
    cx: u64,
    cy: u64,
    docpr_id: u32,
    name: &str,
) -> String {
    format!(
        concat!(
            r#"<w:p><w:r><w:drawing>"#,
            r#"<wp:inline distT="0" distB="0" distL="0" distR="0" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing">"#,
            r#"<wp:extent cx="{cx}" cy="{cy}"/>"#,
            r#"<wp:effectExtent l="0" t="0" r="0" b="0"/>"#,
            r#"<wp:docPr id="{id}" name="{name}"/>"#,
            r#"<wp:cNvGraphicFramePr><a:graphicFrameLocks xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" noChangeAspect="1"/></wp:cNvGraphicFramePr>"#,
            r#"<a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">"#,
            r#"<a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">"#,
            r#"<pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">"#,
            r#"<pic:nvPicPr><pic:cNvPr id="{id}" name="{name}"/><pic:cNvPicPr/></pic:nvPicPr>"#,
            r#"<pic:blipFill><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:embed="{rid}"/>"#,
            r#"<a:stretch><a:fillRect/></a:stretch></pic:blipFill>"#,
            r#"<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>"#,
            r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr>"#,
            r#"</pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
        ),
        cx = cx,
        cy = cy,
        id = docpr_id,
        name = name,
        rid = rid,
    )
}

/// TOC 域自愈提示文案（cached result——Word 带 updateFields 打开即自动刷新域，
/// 届时本文案被真实目录替换；WPS 不保证自动刷，F9 手动刷新兜底）。
const TOC_SELF_HEAL_TEXT: &str = "目录将在打开文档时自动生成（若未生成：全选后按 F9）";

/// 构造 TOC 域段：**裸 fldSimple 单段**（非 sdt 包裹——sdt 会被 walk_blocks
/// 摊平成多块，破坏「1 输入块 = 1 产物块」的生成自检口径）。instr 形如
/// ` TOC \o "1-3" \h \z \u `（\h = 目录项超链接；\z 隐藏 Web 视图页码；\u 用
/// 大纲级别）。levels 钳 1..=9。
pub(super) fn build_toc_paragraph(levels: u32, hyperlink: bool) -> String {
    let levels = levels.clamp(1, 9);
    let h = if hyperlink { r#" \h"# } else { "" };
    format!(
        r#"<w:p><w:fldSimple w:instr=" TOC \o &quot;1-{levels}&quot;{h} \z \u "><w:r><w:t>{TOC_SELF_HEAL_TEXT}</w:t></w:r></w:fldSimple></w:p>"#,
    )
}

/// 扫描 document.xml 现有 `wp:docPr id=` / `pic:cNvPr id=` 取 max+1（Word 要求
/// docPr id 全文档唯一非零；批内多图由调用方在此基础上继续 +1）。没有先例 → 1。
pub(super) fn next_docpr_id(doc_xml: &str) -> u32 {
    let mut max = 0u32;
    for pat in ["wp:docPr id=\"", "pic:cNvPr id=\""] {
        let mut from = 0usize;
        while let Some(rel) = doc_xml[from..].find(pat) {
            let start = from + rel + pat.len();
            let digits: String =
                doc_xml[start..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u32>() {
                max = max.max(n);
            }
            from = start;
        }
    }
    max + 1
}

// =========================================================================
// EMU 数学
// =========================================================================

/// 1 像素（96 dpi）= 9525 EMU；1 twip = 635 EMU（914400/1440）；1 mm = 36000 EMU。
pub(super) const PX_TO_EMU: u64 = 9525;
pub(super) const TWIPS_TO_EMU: u64 = 635;
const MM_TO_EMU: f64 = 36000.0;

/// 计算插入尺寸（EMU）：`width_mm` 显式给 → mm 换算（钳到版心宽——超版心图会
/// 溢出页面）；缺省 → min(原生像素宽, 版心宽)（不放大小图）。高按宽等比缩放。
pub(super) fn compute_extent(
    width_px: u32,
    height_px: u32,
    width_mm: Option<f64>,
    content_width_twips: u32,
) -> (u64, u64) {
    let content_cx = content_width_twips as u64 * TWIPS_TO_EMU;
    let native_cx = width_px as u64 * PX_TO_EMU;
    let target_cx = match width_mm {
        Some(mm) if mm.is_finite() && mm > 0.0 => {
            ((mm * MM_TO_EMU).round() as u64).clamp(1, content_cx)
        }
        _ => native_cx.min(content_cx).max(1),
    };
    let target_cy = (target_cx * height_px as u64 / width_px as u64).max(1);
    (target_cx, target_cy)
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试侧 zip 打包（二进制 entry 无障碍）。
    fn zip_of(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in parts {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(content).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn s(x: &str) -> &[u8] {
        x.as_bytes()
    }

    /// 合成最小 docx 骨架（CT + document + 可选 rels/settings/media）。
    fn fixture_doc(rels: Option<&str>, settings: Option<&str>, media: &[(&str, &[u8])]) -> Vec<u8> {
        let mut parts: Vec<(&str, &[u8])> = vec![
            ("[Content_Types].xml", s(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/></Types>"#)),
            ("word/document.xml", s(r#"<w:document xmlns:w="w"><w:body><w:p/></w:body></w:document>"#)),
        ];
        if let Some(r) = rels {
            parts.push(("word/_rels/document.xml.rels", s(r)));
        }
        if let Some(st) = settings {
            parts.push(("word/settings.xml", s(st)));
        }
        for (n, c) in media {
            parts.push((n, *c));
        }
        zip_of(&parts)
    }

    const RELS_R1_R5: &str = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
        r#"<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>"#,
        r#"</Relationships>"#,
    );

    fn tiny_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200u8, 30, 60]));
        let mut buf = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .expect("png 编码");
        buf.into_inner()
    }

    fn tiny_jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([30u8, 200, 60]));
        let mut buf = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .expect("jpeg 编码");
        buf.into_inner()
    }

    fn png_payload(w: u32, h: u32) -> ImagePayload {
        ImagePayload { bytes: tiny_png(w, h), width_px: w, height_px: h, ext: "png" }
    }

    /// 错误断言辅助：剥 AppError 类型前缀，对准家族前缀。
    fn val_msg(e: AppError) -> String {
        let s = e.to_string();
        s.strip_prefix("参数校验失败: ").unwrap_or(&s).to_string()
    }

    fn read_part(bytes: &[u8], name: &str) -> Vec<u8> {
        let mut a = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut buf = Vec::new();
        a.by_name(name).unwrap().read_to_end(&mut buf).unwrap();
        buf
    }

    fn part_exists(bytes: &[u8], name: &str) -> bool {
        let a = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let hit = a.file_names().any(|n| n == name);
        hit
    }

    // ---- 图片探测 ----

    #[test]
    fn probe_accepts_png_jpeg_rejects_other() {
        let p = probe_image(tiny_png(10, 4)).unwrap();
        assert_eq!((p.width_px, p.height_px, p.ext), (10, 4, "png"));
        let j = probe_image(tiny_jpeg(6, 3)).unwrap();
        assert_eq!((j.width_px, j.height_px, j.ext), (6, 3, "jpeg"));
        // 非 PNG/JPEG 字节 → 图片无效 族
        let e = val_msg(probe_image(b"not an image".to_vec()).unwrap_err());
        assert!(e.starts_with("图片无效:"), "实际: {e}");
        // 超 10 MiB（PNG magic + 填充）
        let mut big = tiny_png(1, 1);
        big.resize(MAX_IMAGE_BYTES + 1, 0x55);
        let e = val_msg(probe_image(big).unwrap_err());
        assert!(e.starts_with("图片无效:"), "实际: {e}");
        assert!(e.contains("超上限"));
    }

    // ---- 分配扫描 ----

    #[test]
    fn allocs_gap_scan_and_increment() {
        // rels 用 rId1/rId5（缺号），media 已有 image1/image3（缺 2）→ 两图分配
        // rId6/image4、rId7/image5；rels 原条目字节不动、新条目在 </Relationships> 前
        let doc = fixture_doc(
            Some(RELS_R1_R5),
            None,
            &[("word/media/image1.png", b"old1"), ("word/media/image3.png", b"old3")],
        );
        let (allocs, additions) =
            plan_package_additions(&doc, &[png_payload(2, 2), png_payload(2, 2)], false).unwrap();
        assert_eq!(allocs[0].rid, "rId6");
        assert_eq!(allocs[0].media_name, "word/media/image4.png");
        assert_eq!(allocs[1].rid, "rId7");
        assert_eq!(allocs[1].media_name, "word/media/image5.png");
        // rels 替换件：原两条 Relationship 原样 + 新两条 image 关系
        let rels_new = additions
            .replacements
            .iter()
            .find(|(p, _)| p == "word/_rels/document.xml.rels")
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert!(rels_new.contains(r#"Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml""#));
        assert!(rels_new.contains(r#"<Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image4.png"/>"#));
        assert!(rels_new.contains(r#"<Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image5.png"/>"#));
        // 新条目全部位于 </Relationships> 之前
        let close = rels_new.rfind("</Relationships>").unwrap();
        assert!(rels_new[..close].contains(r#"Id="rId7""#));
        // media 追加件
        assert_eq!(additions.appends.len(), 2);
        assert_eq!(additions.appends[0].0, "word/media/image4.png");
    }

    #[test]
    fn empty_plan_when_no_image_no_toc() {
        // 纯段落批次：连非法 zip 都不打开（早退零开销）
        let (allocs, additions) = plan_package_additions(b"not a zip", &[], false).unwrap();
        assert!(allocs.is_empty());
        assert!(additions.replacements.is_empty());
        assert!(additions.appends.is_empty());
    }

    #[test]
    fn rels_missing_rejected_for_images() {
        let doc = fixture_doc(None, None, &[]);
        let e = val_msg(
            plan_package_additions(&doc, &[png_payload(2, 2)], false).unwrap_err(),
        );
        assert!(e.starts_with("图片插入无效:"), "实际: {e}");
        assert!(e.contains("document.xml.rels"));
    }

    // ---- CT 增补幂等 ----

    #[test]
    fn content_types_default_idempotent() {
        // 包内无 png Default → 补；已有 → 不进 replacements（逐字节保真）
        let doc = fixture_doc(Some(RELS_R1_R5), None, &[]);
        let (_, additions) =
            plan_package_additions(&doc, &[png_payload(2, 2)], false).unwrap();
        let ct = additions
            .replacements
            .iter()
            .find(|(p, _)| p == "[Content_Types].xml")
            .map(|(_, v)| v.as_str())
            .expect("无 png Default 时应补 CT");
        assert!(ct.contains(r#"<Default Extension="png" ContentType="image/png"/>"#));
        assert!(ct.ends_with("</Types>"));

        // 把上一步产物当新包（已含 png Default）→ 再规划一图：CT 不重写
        let repacked = repack_package(&doc, &additions.replacements, &additions.appends)
            .unwrap();
        let (_, additions2) =
            plan_package_additions(&repacked, &[png_payload(2, 2)], false).unwrap();
        assert!(
            !additions2.replacements.iter().any(|(p, _)| p == "[Content_Types].xml"),
            "CT 已有 png Default 不应重写"
        );
        // jpeg 图仍缺 Default → 补 jpeg 项
        let jpeg = ImagePayload { bytes: tiny_jpeg(2, 2), width_px: 2, height_px: 2, ext: "jpeg" };
        let (_, additions3) =
            plan_package_additions(&repacked, &[jpeg], false).unwrap();
        let ct3 = additions3
            .replacements
            .iter()
            .find(|(p, _)| p == "[Content_Types].xml")
            .map(|(_, v)| v.as_str())
            .expect("jpeg Default 缺失应补");
        assert!(ct3.contains(r#"<Default Extension="jpeg" ContentType="image/jpeg"/>"#));
    }

    // ---- settings upsert ----

    #[test]
    fn settings_upsert_insertion_points() {
        // (a) 有 compat/rsids → 插在 <w:compat 前（首个后继锚）
        let s_a = r#"<w:settings xmlns:w="w"><w:zoom w:percent="100"/><w:compat><w:compatSetting w:name="a"/></w:compat><w:rsids><w:rsid w:val="001"/></w:rsids></w:settings>"#;
        let out = upsert_update_fields(s_a).unwrap().unwrap();
        let at = out.find(r#"<w:updateFields w:val="true"/>"#).unwrap();
        let compat = out.find("<w:compat>").unwrap();
        assert!(at < compat, "应插在 compat 之前");
        assert!(out.ends_with("</w:settings>"));
        // (b) 无任何锚 → 退到 </w:settings> 前
        let s_b = r#"<w:settings xmlns:w="w"><w:zoom w:percent="100"/></w:settings>"#;
        let out = upsert_update_fields(s_b).unwrap().unwrap();
        assert!(out.contains(r#"<w:zoom w:percent="100"/><w:updateFields w:val="true"/></w:settings>"#));
        // (c) 已有 → Ok(None) 不动
        let s_c = r#"<w:settings xmlns:w="w"><w:updateFields w:val="true"/><w:compat/></w:settings>"#;
        assert!(upsert_update_fields(s_c).unwrap().is_none());
        // (d) 前缀碰撞防御：<w:compatSetting 撞 <w:compat——无 compat 元素但串里
        // 出现 compatSetting 前缀时不得误插（此处构造 rsids 锚承接）
        let s_d = r#"<w:settings xmlns:w="w"><w:rsids><w:rsid w:val="1"/></w:rsids></w:settings>"#;
        let out = upsert_update_fields(s_d).unwrap().unwrap();
        assert!(out.contains(r#"<w:updateFields w:val="true"/><w:rsids>"#));
        // (e) 结构异常 → TOC 族 Err
        let e = val_msg(upsert_update_fields("<w:settings xmlns:w=\"w\">").unwrap_err());
        assert!(e.starts_with("TOC 插入无效:"), "实际: {e}");
    }

    #[test]
    fn settings_missing_created_with_ct_override() {
        // 包内无 settings.xml + has_toc → 新建最小件 + CT 补 Override
        let doc = fixture_doc(Some(RELS_R1_R5), None, &[]);
        let (_, additions) = plan_package_additions(&doc, &[], true).unwrap();
        let settings = additions
            .replacements
            .iter()
            .find(|(p, _)| p == "word/settings.xml")
            .map(|(_, v)| v.as_str())
            .expect("settings 缺失应新建");
        assert!(settings.contains(r#"<w:updateFields w:val="true"/>"#));
        let ct = additions
            .replacements
            .iter()
            .find(|(p, _)| p == "[Content_Types].xml")
            .map(|(_, v)| v.as_str())
            .expect("settings 新建应补 CT Override");
        assert!(ct.contains(r#"<Override PartName="/word/settings.xml""#));
    }

    #[test]
    fn settings_present_left_untouched_when_updatefields_exists() {
        // 已带 updateFields 的 settings → 不进 replacements（保逐字节）
        let s = r#"<w:settings xmlns:w="w"><w:updateFields w:val="true"/><w:compat/></w:settings>"#;
        let doc = fixture_doc(Some(RELS_R1_R5), Some(s), &[]);
        let (_, additions) = plan_package_additions(&doc, &[], true).unwrap();
        assert!(
            !additions.replacements.iter().any(|(p, _)| p == "word/settings.xml"),
            "已有 updateFields 不应重写"
        );
    }

    // ---- 重打包 ----

    #[test]
    fn repack_rejects_missing_replacement_and_append_collision() {
        let doc = fixture_doc(Some(RELS_R1_R5), None, &[]);
        let e = repack_package(
            &doc,
            &[("word/nope.xml".into(), "<x/>".into())],
            &[],
        )
        .unwrap_err();
        assert!(e.to_string().contains("重打包部件缺失"), "实际: {e:?}");
        let e = repack_package(
            &doc,
            &[],
            &[("word/document.xml".into(), b"<x/>".to_vec())],
        )
        .unwrap_err();
        assert!(e.to_string().contains("追加件已存在"), "实际: {e:?}");
    }

    #[test]
    fn repack_preserves_untouched_bytes() {
        let media1 = tiny_png(3, 3);
        let doc = fixture_doc(
            Some(RELS_R1_R5),
            Some("<w:settings xmlns:w=\"w\"><w:compat/></w:settings>"),
            &[("word/media/image1.png", &media1), ("word/styles.xml", b"<w:styles/>")],
        );
        let out = repack_package(
            &doc,
            &[
                ("word/document.xml".into(), r#"<w:document/>"#.into()),
                ("word/settings.xml".into(), "<w:settings/>".into()),
            ],
            &[("word/media/image2.png".into(), b"NEWBINARY".to_vec())],
        )
        .unwrap();
        // 未列部件逐字节相等
        assert_eq!(read_part(&out, "word/styles.xml"), b"<w:styles/>");
        assert_eq!(read_part(&out, "word/media/image1.png"), media1);
        // 替换件生效
        assert_eq!(read_part(&out, "word/document.xml"), b"<w:document/>");
        assert_eq!(read_part(&out, "word/settings.xml"), b"<w:settings/>");
        // 追加件生效
        assert_eq!(read_part(&out, "word/media/image2.png"), b"NEWBINARY");
    }

    // ---- XML 构造合法性 ----

    #[test]
    fn drawing_paragraph_parses_with_embed_and_extent() {
        let xml = build_drawing_paragraph("rId9", 952500, 476250, 3, "图片 3");
        let dom = xml_dom::parse(&xml).unwrap();
        assert_eq!(dom.name, "w:p");
        // 深度找 wp:docPr 与 a:blip（内联命名空间解析无碍）
        fn find<'a>(el: &'a xml_dom::Element, name: &str) -> Option<&'a xml_dom::Element> {
            for c in el.child_elements() {
                if c.name == name {
                    return Some(c);
                }
                if let Some(hit) = find(c, name) {
                    return Some(hit);
                }
            }
            None
        }
        let doc_pr = find(&dom, "wp:docPr").expect("wp:docPr");
        assert_eq!(doc_pr.attr("id"), Some("3"));
        assert_eq!(doc_pr.attr("name"), Some("图片 3"));
        let blip = find(&dom, "a:blip").expect("a:blip");
        assert_eq!(blip.attr("r:embed"), Some("rId9"));
        let extent = find(&dom, "wp:extent").expect("wp:extent");
        assert_eq!(extent.attr("cx"), Some("952500"));
        assert_eq!(extent.attr("cy"), Some("476250"));
    }

    #[test]
    fn toc_paragraph_parses_and_instr_roundtrips() {
        let xml = build_toc_paragraph(3, true);
        let dom = xml_dom::parse(&xml).unwrap();
        assert_eq!(dom.name, "w:p");
        let fld = dom
            .child_elements()
            .find(|e| e.name == "w:fldSimple")
            .expect("fldSimple");
        // 属性实体解码后应得到规范 instr（TOC 断言引擎的 instr_contains 口径）
        assert_eq!(fld.attr("w:instr"), Some(r#" TOC \o "1-3" \h \z \u "#));
        assert!(fld.raw_text().contains("目录将在打开文档时自动生成"));
        // hyperlink=false → 无 \h
        let no_h = build_toc_paragraph(2, false);
        let dom2 = xml_dom::parse(&no_h).unwrap();
        let fld2 = dom2
            .child_elements()
            .find(|e| e.name == "w:fldSimple")
            .unwrap();
        assert_eq!(fld2.attr("w:instr"), Some(r#" TOC \o "1-2" \z \u "#));
        // levels 钳位
        let clamped = build_toc_paragraph(99, true);
        let dom3 = xml_dom::parse(&clamped).unwrap();
        let fld3 = dom3
            .child_elements()
            .find(|e| e.name == "w:fldSimple")
            .unwrap();
        assert!(fld3.attr("w:instr").unwrap().contains(r#"\o "1-9""#));
    }

    #[test]
    fn docpr_id_scans_max() {
        let xml = r#"<w:body><w:drawing><wp:docPr id="7" name="a"/><pic:pic><pic:cNvPr id="12" name="b"/></pic:pic></w:drawing></w:body>"#;
        assert_eq!(next_docpr_id(xml), 13);
        assert_eq!(next_docpr_id("<w:body><w:p/></w:body>"), 1);
    }

    // ---- EMU 数学 ----

    #[test]
    fn extent_math() {
        // A4 版心 11906-1800-1800 = 8306 twips → ×635 = 5,274,310 EMU
        let cw = 8306u32;
        let content_cx = 8306u64 * 635;
        // 小图原生（100px×9525=952,500）不放大
        assert_eq!(compute_extent(100, 50, None, cw), (952_500, 476_250));
        // 大图钳到版心宽，高等比
        let (cx, cy) = compute_extent(2000, 1000, None, cw);
        assert_eq!((cx, cy), (content_cx, content_cx / 2));
        // 显式 mm（100mm=3,600,000 < 版心）
        assert_eq!(compute_extent(100, 50, Some(100.0), cw), (3_600_000, 1_800_000));
        // 超版心 mm 钳到版心
        let (cx, _) = compute_extent(100, 50, Some(300.0), cw);
        assert_eq!(cx, content_cx);
    }

    // ---- 端到端规划→重打包（连插两图 rId/K 递增）----

    #[test]
    fn two_round_allocations_increment() {
        let doc = fixture_doc(Some(RELS_R1_R5), None, &[]);
        let img1 = png_payload(4, 4);
        let bytes1 = img1.bytes.clone();
        let (allocs1, add1) = plan_package_additions(&doc, &[img1], false).unwrap();
        let out1 = repack_package(&doc, &add1.replacements, &add1.appends).unwrap();
        assert_eq!(allocs1[0].media_name, "word/media/image1.png");
        assert_eq!(read_part(&out1, "word/media/image1.png"), bytes1);
        assert!(part_exists(&out1, "word/media/image1.png"));

        // 第二轮（对产物再插）：rId6 已占 → rId7；image1 已占 → image2
        let img2 = png_payload(4, 4);
        let bytes2 = img2.bytes.clone();
        let (allocs2, add2) = plan_package_additions(&out1, &[img2], false).unwrap();
        assert_eq!(allocs2[0].rid, "rId7");
        assert_eq!(allocs2[0].media_name, "word/media/image2.png");
        let out2 = repack_package(&out1, &add2.replacements, &add2.appends).unwrap();
        assert_eq!(read_part(&out2, "word/media/image1.png"), bytes1, "第一张图字节不动");
        assert_eq!(read_part(&out2, "word/media/image2.png"), bytes2);
    }
}
