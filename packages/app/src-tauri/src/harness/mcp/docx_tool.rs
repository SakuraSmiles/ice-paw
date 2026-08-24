//! `inspect_docx` / `edit_docx` 工具（word-capability-roadmap 步骤 2 / 步骤 3）。
//!
//! - **inspect_docx**（S0b）：read_file 对 docx 只给线性文本（结构全丢）；本工具
//!   在结构模型上出三档投影（outline / format / text，见 harness::doc::docx_inspect），
//!   块号 1-based 混排统一编号——这是 edit_docx 的地址地基。
//! - **edit_docx**（步骤 3，D3 批量事务）：在块编址上做块级手术，一批操作全有或
//!   全无；手术引擎在 harness::doc::docx_edit（纯函数）。
//!
//! 薄壳职责：读文件 + 扩展名守卫 + 参数解析 + 备份/原子写；全部业务逻辑在
//! harness::doc 纯函数层，独立单测。错误契约三段式：not-found 挂 did-you-mean；
//! 非 docx 指向 read_file / inspect_docx。

use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::harness::doc::{apply_edits_to_bytes, inspect_document, EditOp, InspectProjection, InspectRequest};

use super::client::McpClient;
use super::types::AuthorizationLevel;

pub struct InspectDocxTool;

#[derive(Deserialize)]
struct InspectDocxArgs {
    path: String,
    /// outline（默认，全图）/ format（run 级格式）/ text（带块号正文）/ headers_footers（页眉页脚）/ table（表格网格）
    #[serde(default)]
    projection: Option<String>,
    /// 起始块号（1-based，含）
    #[serde(default)]
    start: Option<usize>,
    /// 结束块号（1-based，含）
    #[serde(default)]
    end: Option<usize>,
}

#[derive(Serialize)]
struct InspectDocxResult {
    path: String,
    projection: &'static str,
    total_blocks: usize,
    /// [start, end]（1-based 含端点）
    range: (usize, usize),
    has_more: bool,
    /// has_more=true 时带续读提示的 next_start
    next_start: Option<usize>,
    content: String,
}

#[async_trait]
impl McpClient for InspectDocxTool {
    fn name(&self) -> &str {
        "inspect_docx"
    }

    fn description(&self) -> &str {
        "Inspect the structure of a Word .docx document at three levels of detail. \
         projection=outline (default): one line per block — block number, style/heading \
         level, text summary; the map of the whole document. projection=format: run-level \
         effective formatting (font size/weight/color, fonts, alignment, line spacing, \
         indents after style-chain resolution) plus table grids for a block range. \
         projection=ppr: raw paragraph-property (pPr) XML of each block — the editing \
         basis for edit_docx set_ppr_element (copy the element XML you see here, modify \
         it, send it back; also the verification view after that op). \
         projection=text: document text with block-number prefixes. \
         projection=headers_footers: per-section header/footer references with their \
         content (start/end do not apply — organized by sections, not block ranges). \
         projection=table: per-table cell grid for a block range — every cell on one row \
         line with merge/nested markers; the cell address scheme (row r × cell c, both \
         1-based) is exactly what edit_docx set_cell_text / insert_table_row_after use. \
         Blocks are numbered \
         1-based in document order (paragraphs and tables together); these numbers are \
         the addressing scheme used to reference locations for editing. Workflow: outline \
         first to locate blocks, then format/text/table with start/end for details. Defaults: \
         outline renders up to 400 blocks, format up to 50, ppr up to 20, text up to 100, \
         table up to 10."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the .docx file to inspect."
                },
                "projection": {
                    "type": "string",
                    "enum": ["outline", "format", "ppr", "text", "headers_footers", "table"],
                    "description": "Level of detail: outline (block map, default), format (run-level formatting), ppr (raw pPr XML per block — basis for set_ppr_element), text (block-numbered text), headers_footers (per-section header/footer content; no start/end), table (cell grid of table blocks — basis for set_cell_text / insert_table_row_after)."
                },
                "start": {
                    "type": "integer",
                    "description": "First block number to render (1-based, inclusive). Default 1. Not applicable to headers_footers."
                },
                "end": {
                    "type": "integer",
                    "description": "Last block number to render (1-based, inclusive). Default: start + span - 1 (span depends on projection). Not applicable to headers_footers."
                }
            },
            "required": ["path"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: InspectDocxArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("inspect_docx 参数解析失败: {e}")))?;

        let projection = match parsed.projection.as_deref() {
            None | Some("outline") => InspectProjection::Outline,
            Some("format") => InspectProjection::Format,
            Some("ppr") => InspectProjection::Ppr,
            Some("text") => InspectProjection::Text,
            Some("headers_footers") => InspectProjection::HeadersFooters,
            Some("table") => InspectProjection::Table,
            Some(other) => {
                return Err(AppError::Validation(format!(
                    "未知投影档位: {other}。支持 outline（块级地图，默认）/ format（run 级格式）/ text（带块号正文）/ headers_footers（页眉页脚）/ table（表格网格）。"
                )));
            }
        };

        let path = Path::new(&parsed.path);
        let canonical = match path.canonicalize() {
            Ok(c) => c,
            // 报错即行为契约：not-found 扫真实文件系统给近似候选（did-you-mean）
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AppError::Validation(format!(
                    "文件不存在: {}。{}",
                    parsed.path,
                    super::path_suggest::suggest_for_missing(path)
                )));
            }
            Err(e) => return Err(AppError::Validation(format!("文件路径无效: {e}"))),
        };

        let ext = canonical
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "docx" {
            return Err(AppError::Validation(format!(
                "不是 Word 文档: 扩展名 .{ext}。inspect_docx 只支持 .docx；其他文件请用 read_file。"
            )));
        }

        let bytes = tokio::fs::read(&canonical)
            .await
            .map_err(|e| AppError::Io(std::io::Error::other(format!("读取文件失败: {e}"))))?;

        let report = inspect_document(
            &bytes,
            &InspectRequest { projection, start: parsed.start, end: parsed.end },
        )?;

        let result = InspectDocxResult {
            path: parsed.path,
            projection: projection.as_str(),
            total_blocks: report.total_blocks,
            range: report.range,
            has_more: report.has_more,
            next_start: if report.has_more { Some(report.range.1 + 1) } else { None },
            content: report.content,
        };
        Ok(serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()))
    }
}

// =========================================================================
// edit_docx —— 块级手术工具（步骤 3，D3 批量事务）
// =========================================================================

pub struct EditDocxTool;

#[derive(Deserialize)]
struct EditDocxArgs {
    path: String,
    /// 操作批（全有或全无；段级操作每块限一个，表格操作（set_cell_text /
    /// insert_table_row_after）可同块多条按序组合，(行, 格) 去重）
    operations: Vec<OperationSpec>,
}

/// 参数态操作（tag = op）；转成引擎的 [`EditOp`] 后全批进入手术。
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum OperationSpec {
    ReplaceText { block: usize, expect_prefix: String, new_text: String },
    InsertParagraphAfter {
        block: usize,
        expect_prefix: String,
        text: String,
        /// 样式显示名（inspect_docx outline 样式列口径）；缺省继承锚块格式
        #[serde(default)]
        style: Option<String>,
    },
    DeleteBlock { block: usize, expect_prefix: String },
    /// 改段落样式（标题升降级等）：style 接受显示名或样式 ID；正文 run 不动
    SetStyle { block: usize, expect_prefix: String, style: String },
    /// 改格式：paragraph（对齐/行距/段前后/缩进）与 character（粗斜/字号/颜色/字体）
    /// 至少一项内有字段；None 字段原样保留
    SetFormat {
        block: usize,
        expect_prefix: String,
        #[serde(default)]
        paragraph: Option<ParaFormatSpec>,
        #[serde(default)]
        character: Option<CharFormatSpec>,
    },
    /// 通用段落属性元素手术：element 为 pPr 子元素名（无 w: 前缀）；xml=null 移除
    /// 整元素，xml=片段按 schema 序整元素替换/插入。片段从 inspect_docx
    /// projection=ppr 的原文复制修改，不凭记忆新写
    SetPprElement {
        block: usize,
        expect_prefix: String,
        element: String,
        #[serde(default)]
        xml: Option<String>,
    },
    /// 锚块后建新表：rows 矩形矩阵（首行默认表头——加粗 + 跨页重复）；列宽均分
    InsertTableAfter {
        block: usize,
        expect_prefix: String,
        rows: Vec<Vec<String>>,
        #[serde(default)]
        header: Option<bool>,
    },
    /// 改单元格文本：(row, cell) 双 1-based，与 projection=table 网格同口径；
    /// 保 tcPr / 首段 pPr / 首 run rPr；\n = 格内多段
    SetCellText { block: usize, expect_prefix: String, row: usize, cell: usize, text: String },
    /// 克隆模板行增行（after_row 缺省 = 末行）：整结构克隆（tcPr/gridSpan/vMerge
    /// 原样），格文本替换为 cells（缺省全空）——合并格表格唯一正确增行方式
    InsertTableRowAfter {
        block: usize,
        expect_prefix: String,
        #[serde(default)]
        after_row: Option<usize>,
        #[serde(default)]
        cells: Option<Vec<String>>,
    },
}

/// 参数态段落格式（单位与 format 投影显示一致：行距=倍数 / 段前后=pt / 缩进=twips）。
#[derive(Deserialize)]
struct ParaFormatSpec {
    #[serde(default)]
    align: Option<String>,
    #[serde(default)]
    line_spacing: Option<f32>,
    #[serde(default)]
    space_before_pt: Option<f32>,
    #[serde(default)]
    space_after_pt: Option<f32>,
    #[serde(default)]
    indent_first_line_tw: Option<i32>,
    #[serde(default)]
    indent_left_tw: Option<i32>,
}

/// 参数态字符格式（应用到段落内每个 run）。
#[derive(Deserialize)]
struct CharFormatSpec {
    #[serde(default)]
    bold: Option<bool>,
    #[serde(default)]
    italic: Option<bool>,
    #[serde(default)]
    font_size_pt: Option<f32>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    font: Option<String>,
}

impl From<ParaFormatSpec> for crate::harness::doc::ParaFormat {
    fn from(s: ParaFormatSpec) -> Self {
        Self {
            align: s.align,
            line_spacing: s.line_spacing,
            space_before_pt: s.space_before_pt,
            space_after_pt: s.space_after_pt,
            indent_first_line_tw: s.indent_first_line_tw,
            indent_left_tw: s.indent_left_tw,
        }
    }
}

impl From<CharFormatSpec> for crate::harness::doc::CharFormat {
    fn from(s: CharFormatSpec) -> Self {
        Self {
            bold: s.bold,
            italic: s.italic,
            font_size_pt: s.font_size_pt,
            color: s.color,
            font: s.font,
        }
    }
}

impl From<OperationSpec> for EditOp {
    fn from(spec: OperationSpec) -> Self {
        match spec {
            OperationSpec::ReplaceText { block, expect_prefix, new_text } => {
                EditOp::ReplaceText { block, expect_prefix, new_text }
            }
            OperationSpec::InsertParagraphAfter { block, expect_prefix, text, style } => {
                EditOp::InsertParagraphAfter { block, expect_prefix, text, style }
            }
            OperationSpec::DeleteBlock { block, expect_prefix } => {
                EditOp::DeleteBlock { block, expect_prefix }
            }
            OperationSpec::SetStyle { block, expect_prefix, style } => {
                EditOp::SetStyle { block, expect_prefix, style }
            }
            OperationSpec::SetFormat { block, expect_prefix, paragraph, character } => {
                EditOp::SetFormat {
                    block,
                    expect_prefix,
                    paragraph: paragraph.map(Into::into),
                    character: character.map(Into::into),
                }
            }
            OperationSpec::SetPprElement { block, expect_prefix, element, xml } => {
                EditOp::SetPprElement { block, expect_prefix, element, xml }
            }
            OperationSpec::InsertTableAfter { block, expect_prefix, rows, header } => {
                EditOp::InsertTableAfter { block, expect_prefix, rows, header }
            }
            OperationSpec::SetCellText { block, expect_prefix, row, cell, text } => {
                EditOp::SetCellText { block, expect_prefix, row, cell, text }
            }
            OperationSpec::InsertTableRowAfter { block, expect_prefix, after_row, cells } => {
                EditOp::InsertTableRowAfter { block, expect_prefix, after_row, cells }
            }
        }
    }
}

#[derive(Serialize)]
struct EditDocxResult {
    path: String,
    /// 实际生效的操作数（= operations.len()，全有或全无）
    applied: usize,
    /// 修改前备份（<parent>/.icepaw-backup/<时间戳>_<文件名>；文件不存在时无）
    backup: Option<String>,
    /// 逐操作摘要（before/after 各前 60 字；agent 读回验证用）
    operations: Vec<crate::harness::doc::AppliedOp>,
}

#[async_trait]
impl McpClient for EditDocxTool {
    fn name(&self) -> &str {
        "edit_docx"
    }

    fn description(&self) -> &str {
        "Edit a Word .docx document at block granularity. Takes a batch of operations \
         applied as one all-or-nothing transaction: op=replace_text rewrites a paragraph's \
         text while keeping its paragraph/character formatting; op=insert_paragraph_after \
         adds a new paragraph after an anchor block (inherits its formatting, or an \
         explicit style by display name); op=delete_block removes a whole block; \
         op=set_style changes a paragraph's style (e.g. promote to a heading) touching \
         only the style element while leaving text and character formatting intact — \
         if the paragraph already has that style the result carries style_unchanged=true \
         (nothing changed; do not retry the same op); \
         op=set_format changes paragraph formatting (alignment, line spacing, spacing \
         before/after, indents) and/or character formatting (bold/italic, font size, \
         color, font family applied to every run) with unspecified properties preserved; \
         op=set_ppr_element is the generic escape hatch for any other paragraph-property \
         element (numPr, keepNext, shd, tabs, outlineLvl, ...) — xml=null removes the \
         element, xml=<w:...> fragment replaces/inserts it at its schema position (copy \
         the current XML from inspect_docx projection=ppr, never write from memory; if \
         removing numPr while the style chain still defines numbering, the result warns \
         that Word falls back to the style's numbers). Table operations: \
         op=insert_table_after creates a new table after an anchor block from a \
         rectangular rows matrix (first row is a bold repeating header by default; \
         100% width, all borders, evenly split columns); op=set_cell_text rewrites one \
         cell's text by (row, cell) address — the exact grid shown by inspect_docx \
         projection=table, keeping the cell's structure properties and the first \
         paragraph's formatting; op=insert_table_row_after appends a row by cloning a \
         template row (default: the last one) so merged cells keep working — multiple \
         set_cell_text / insert_table_row_after ops on the same table are applied in \
         order within one batch. Every \
         operation must carry expect_prefix, the current text prefix of its target block, \
         as a fingerprint guard — if any block no longer matches, the whole batch is \
         rejected and the file is left untouched. Blocks are addressed by inspect_docx \
         block numbers (1-based, paragraphs and tables in document order); text ops \
         (replace/delete/set_style/set_format/set_ppr_element) reject table and \
         revision-marked blocks. The file is backed up \
         before writing. Workflow: inspect_docx (outline, then text/table) to find blocks, \
         edit_docx, then inspect_docx again to verify the result."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the .docx file to edit."
                },
                "operations": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Operations applied as one all-or-nothing batch. \
                     Each item is tagged with op; block numbers come from inspect_docx.",
                    "items": {
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "replace_text" },
                                    "block": { "type": "integer", "description": "Target paragraph block (1-based; tables rejected)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the block (fingerprint guard)." },
                                    "new_text": { "type": "string", "description": "Replacement text. \\n → line break, \\t → tab." }
                                },
                                "required": ["op", "block", "expect_prefix", "new_text"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "insert_paragraph_after" },
                                    "block": { "type": "integer", "description": "Anchor block (1-based)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the anchor block." },
                                    "text": { "type": "string", "description": "New paragraph text. \\n → line break, \\t → tab." },
                                    "style": { "type": "string", "description": "Optional style display name from inspect_docx outline; default inherits anchor formatting." }
                                },
                                "required": ["op", "block", "expect_prefix", "text"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "delete_block" },
                                    "block": { "type": "integer", "description": "Block to delete (1-based; revision-marked and section-break blocks rejected)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the block." }
                                },
                                "required": ["op", "block", "expect_prefix"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "set_style" },
                                    "block": { "type": "integer", "description": "Target paragraph block (1-based; tables rejected)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the block (fingerprint guard)." },
                                    "style": { "type": "string", "description": "Style display name or ID from inspect_docx outline style column; text and character formatting are left intact." }
                                },
                                "required": ["op", "block", "expect_prefix", "style"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "set_format" },
                                    "block": { "type": "integer", "description": "Target paragraph block (1-based; tables rejected)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the block (fingerprint guard)." },
                                    "paragraph": {
                                        "type": "object",
                                        "description": "Paragraph-level formatting; omitted fields are left unchanged. Units match the format projection: line_spacing as a multiple (1.5 = 1.5-line spacing), spacing in pt, indents in twips (1 CJK char = 240tw).",
                                        "properties": {
                                            "align": { "type": "string", "enum": ["left", "center", "right", "both", "distribute", "start", "end"], "description": "both = justified, distribute = spread." },
                                            "line_spacing": { "type": "number", "description": "Line spacing multiple, e.g. 1.5. Must be > 0." },
                                            "space_before_pt": { "type": "number", "description": "Space before paragraph in points." },
                                            "space_after_pt": { "type": "number", "description": "Space after paragraph in points." },
                                            "indent_first_line_tw": { "type": "integer", "description": "First-line indent in twips, >= 0. 2 CJK chars = 480tw." },
                                            "indent_left_tw": { "type": "integer", "description": "Left indent in twips (negative = outdent)." }
                                        },
                                        "additionalProperties": false
                                    },
                                    "character": {
                                        "type": "object",
                                        "description": "Character formatting applied to EVERY run in the paragraph; omitted fields are left unchanged.",
                                        "properties": {
                                            "bold": { "type": "boolean" },
                                            "italic": { "type": "boolean" },
                                            "font_size_pt": { "type": "number", "description": "Font size in points (1-400)." },
                                            "color": { "type": "string", "description": "Hex RGB without '#', e.g. FF0000." },
                                            "font": { "type": "string", "description": "Font family name; sets eastAsia/ascii/hAnsi together." }
                                        },
                                        "additionalProperties": false
                                    }
                                },
                                "required": ["op", "block", "expect_prefix"],
                                "description": "At least one field inside paragraph or character must be set."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "set_ppr_element" },
                                    "block": { "type": "integer", "description": "Target paragraph block (1-based; tables rejected)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the block (fingerprint guard)." },
                                    "element": {
                                        "type": "string",
                                        "description": "Paragraph-property (pPr) child element name without the w: prefix, e.g. numPr, keepNext, shd, tabs, outlineLvl, widowControl. Legal elements (schema order): pStyle keepNext keepLines pageBreakBefore framePr widowControl numPr suppressLineNumbers pBdr shd tabs suppressAutoHyphens kinsoku wordWrap overflowPunct topLinePunct autoSpaceDE autoSpaceDN bidi adjustRightInd snapToGrid spacing ind contextualSpacing mirrorIndents suppressOverlap jc textDirection textAlignment textboxTightWrap outlineLvl divId cnfStyle rPr. sectPr/pPrChange are protected (rejected)."
                                    },
                                    "xml": {
                                        "type": ["string", "null"],
                                        "description": "null (or omitted) = remove the whole element; a string = well-formed single-root fragment '<w:ELEMENT ...>...</w:pPr-level element>' replacing the existing element or inserted at its schema position. Copy the current XML from inspect_docx projection=ppr and modify it — never write OOXML from memory. No xmlns declarations allowed. Example: to strip auto-numbering use element=numPr, xml=null."
                                    }
                                },
                                "required": ["op", "block", "expect_prefix", "element"],
                                "description": "Generic surgery on any paragraph-property element. Removing numPr converts auto-numbered paragraphs to plain text numbering (compute the displayed numbers first via inspect_docx outline if you need to hardcode them into the text with replace_text)."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "insert_table_after" },
                                    "block": { "type": "integer", "description": "Anchor block (1-based); the new table becomes the next block." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the anchor block (fingerprint guard)." },
                                    "rows": {
                                        "type": "array",
                                        "minItems": 1,
                                        "maxItems": 200,
                                        "description": "Rectangular matrix of cell texts (every row same length, 1-30 cells). \\n inside a cell = multiple paragraphs.",
                                        "items": { "type": "array", "minItems": 1, "maxItems": 30, "items": { "type": "string" } }
                                    },
                                    "header": { "type": "boolean", "description": "true (default): first row is bold and repeats across pages; false: plain data rows only." }
                                },
                                "required": ["op", "block", "expect_prefix", "rows"],
                                "description": "Create a new table after the anchor block. Default styling: 100% width, single borders, evenly split columns, bold repeating header row."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "set_cell_text" },
                                    "block": { "type": "integer", "description": "Target table block (1-based)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the table block (fingerprint guard; any cell text works)." },
                                    "row": { "type": "integer", "description": "Row number r (1-based) — same as the rN lines of inspect_docx projection=table." },
                                    "cell": { "type": "integer", "description": "Cell number c within the row (1-based; a merged/spanning cell counts as one)." },
                                    "text": { "type": "string", "description": "New cell text. \\n = multiple paragraphs inside the cell; empty string = clear. Formatting of the cell's first paragraph is preserved." }
                                },
                                "required": ["op", "block", "expect_prefix", "row", "cell", "text"],
                                "description": "Rewrite one table cell's text. Cells marked (续) in projection=table are vertical-merge continuations and cannot be edited — edit their (合并头) cell instead."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "insert_table_row_after" },
                                    "block": { "type": "integer", "description": "Target table block (1-based)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the table block (fingerprint guard)." },
                                    "after_row": { "type": "integer", "description": "Template row number (1-based) to clone and insert after; default = last row. Structure (widths, spans, merges) is cloned verbatim — the only correct way to add rows to a merged-cell table." },
                                    "cells": { "type": "array", "description": "Cell texts for the new row; length must equal the template row's cell count. Default = all empty." }
                                },
                                "required": ["op", "block", "expect_prefix"],
                                "description": "Append a row to a table by cloning an existing row's structure and filling its text."
                            }
                        ]
                    }
                }
            },
            "required": ["path", "operations"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: EditDocxArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("edit_docx 参数解析失败: {e}")))?;

        let path = Path::new(&parsed.path);
        let canonical = match path.canonicalize() {
            Ok(c) => c,
            // 报错即行为契约：not-found 扫真实文件系统给近似候选（did-you-mean）
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AppError::Validation(format!(
                    "文件不存在: {}。{}",
                    parsed.path,
                    super::path_suggest::suggest_for_missing(path)
                )));
            }
            Err(e) => return Err(AppError::Validation(format!("文件路径无效: {e}"))),
        };

        let ext = canonical
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "docx" {
            return Err(AppError::Validation(format!(
                "不是 Word 文档: 扩展名 .{ext}。edit_docx 只支持 .docx；读取其他文件请用 read_file。"
            )));
        }

        let bytes = tokio::fs::read(&canonical)
            .await
            .map_err(|e| AppError::Io(std::io::Error::other(format!("读取文件失败: {e}"))))?;

        // 全有或全无：手术在内存完成（含整批预检 + 产物再解析校验），通过才落盘
        let ops: Vec<EditOp> = parsed.operations.into_iter().map(EditOp::from).collect();
        let (new_bytes, applied) = apply_edits_to_bytes(&bytes, &ops)?;

        // 修改前备份（与 write_file 同一通道）；tmp + rename 原子替换（崩溃不损坏原文件）
        let backup = super::file_tools::backup_if_exists(&canonical)?;
        let mut tmp_name = canonical
            .file_name()
            .unwrap_or_default()
            .to_os_string();
        tmp_name.push(".icepaw-tmp");
        let tmp = canonical.with_file_name(tmp_name);
        if let Err(e) = write_and_rename(&tmp, &canonical, &new_bytes).await {
            tokio::fs::remove_file(&tmp).await.ok();
            return Err(AppError::Validation(format!(
                "edit_docx 写入失败: {}: {e}。原文件未改动。请确认路径在授权工作区内；\
                 备份位于 .icepaw-backup/（若有）。",
                canonical.display()
            )));
        }

        let result = EditDocxResult {
            path: parsed.path,
            applied: applied.len(),
            backup,
            operations: applied,
        };
        Ok(serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()))
    }
}

/// 先写 tmp 再 rename 覆盖目标（std rename 语义：目标存在则替换）。
async fn write_and_rename(tmp: &Path, target: &Path, bytes: &[u8]) -> AppResult<()> {
    tokio::fs::write(tmp, bytes)
        .await
        .map_err(|e| AppError::Io(std::io::Error::other(format!("临时文件写入失败: {e}"))))?;
    tokio::fs::rename(tmp, target)
        .await
        .map_err(|e| AppError::Io(std::io::Error::other(format!("原子替换失败: {e}"))))?;
    Ok(())
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// docx-rs 造最小真实包（zip 容器 + document.xml）。
    fn docx_bytes() -> Vec<u8> {
        use docx_rs::{Docx, Document, Paragraph, Run};
        let document =
            Document::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text("正文段")));
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        Docx::new().document(document).build().pack(&mut cursor).unwrap();
        cursor.into_inner()
    }

    #[tokio::test]
    async fn happy_path_outline() {
        let dir = std::env::temp_dir().join("icepaw_inspect_docx_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("样本.docx");
        std::fs::write(&file, docx_bytes()).unwrap();

        let tool = InspectDocxTool;
        let args = serde_json::json!({ "path": file.to_string_lossy() }).to_string();
        let out = tool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["projection"], "outline");
        assert_eq!(v["total_blocks"], 1);
        assert!(v["content"].as_str().unwrap().contains("正文段"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn rejects_non_docx_and_bad_projection() {
        let tool = InspectDocxTool;
        // 非 docx 扩展名
        let dir = std::env::temp_dir().join("icepaw_inspect_docx_test2");
        std::fs::create_dir_all(&dir).unwrap();
        let txt = dir.join("note.txt");
        std::fs::write(&txt, b"hello").unwrap();
        let args = serde_json::json!({ "path": txt.to_string_lossy() }).to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("read_file"), "应指向 read_file: {err}");
        // 未知投影
        let args = serde_json::json!({
            "path": txt.to_string_lossy(),
            "projection": "deep"
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("未知投影档位"), "实际: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn missing_file_gives_suggestions() {
        let tool = InspectDocxTool;
        let args = serde_json::json!({ "path": "Z:/不存在的/文档.docx" }).to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("文件不存在"), "实际: {err}");
    }

    // ---- edit_docx ----

    /// 三段正文的最小真实包。
    fn three_para_docx() -> Vec<u8> {
        use docx_rs::{Docx, Document, Paragraph, Run};
        let document = Document::new()
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第一段")))
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第二段")))
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第三段")));
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        Docx::new().document(document).build().pack(&mut cursor).unwrap();
        cursor.into_inner()
    }

    #[tokio::test]
    async fn edit_batch_applies_and_backs_up() {
        let dir = std::env::temp_dir().join("icepaw_edit_docx_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("样本.docx");
        std::fs::write(&file, three_para_docx()).unwrap();

        let tool = EditDocxTool;
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "operations": [
                { "op": "replace_text", "block": 2, "expect_prefix": "第二段", "new_text": "改写段" },
                { "op": "insert_paragraph_after", "block": 1, "expect_prefix": "第一段", "text": "新插段" },
                { "op": "delete_block", "block": 3, "expect_prefix": "第三段" }
            ]
        })
        .to_string();
        let out = tool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["applied"], 3);
        assert!(v["backup"].as_str().is_some(), "应已备份: {out}");
        assert_eq!(v["operations"].as_array().unwrap().len(), 3);

        // 读回验证：全文 = 第一段/新插段/改写段（第三段已删）
        let bytes = std::fs::read(&file).unwrap();
        let inspect = crate::harness::doc::inspect_document(
            &bytes,
            &InspectRequest { projection: InspectProjection::Text, start: None, end: None },
        )
        .unwrap();
        assert!(inspect.content.contains("新插段"));
        assert!(inspect.content.contains("改写段"));
        assert!(!inspect.content.contains("第三段"));
        assert_eq!(inspect.total_blocks, 3, "3 块 +1 插 -1 删");

        // 备份内容 = 原始字节
        let backup_path = v["backup"].as_str().unwrap();
        assert_eq!(std::fs::read(backup_path).unwrap(), three_para_docx());
        // tmp 不残留
        assert!(!file.with_file_name("样本.docx.icepaw-tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn fingerprint_mismatch_leaves_file_untouched() {
        let dir = std::env::temp_dir().join("icepaw_edit_docx_test2");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("样本.docx");
        std::fs::write(&file, three_para_docx()).unwrap();
        let before = std::fs::read(&file).unwrap();

        let tool = EditDocxTool;
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "operations": [
                { "op": "replace_text", "block": 1, "expect_prefix": "第一段", "new_text": "x" },
                { "op": "delete_block", "block": 2, "expect_prefix": "过时指纹" }
            ]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("指纹不符"), "实际: {err}");
        // 全有或全无：合法的第 1 条也未生效，文件字节原样
        assert_eq!(std::fs::read(&file).unwrap(), before);
        // 无备份产生（未动刀）
        assert!(!dir.join(".icepaw-backup").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn edit_rejects_non_docx_and_missing() {
        let tool = EditDocxTool;
        let dir = std::env::temp_dir().join("icepaw_edit_docx_test3");
        std::fs::create_dir_all(&dir).unwrap();
        let txt = dir.join("note.txt");
        std::fs::write(&txt, b"hello").unwrap();
        let args = serde_json::json!({
            "path": txt.to_string_lossy(),
            "operations": [{ "op": "delete_block", "block": 1, "expect_prefix": "" }]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("read_file"), "应指向 read_file: {err}");

        let args = serde_json::json!({
            "path": "Z:/不存在的/文档.docx",
            "operations": [{ "op": "delete_block", "block": 1, "expect_prefix": "" }]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("文件不存在"), "实际: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edit_auth_level_is_path_whitelist() {
        assert_eq!(EditDocxTool.authorization_level(), AuthorizationLevel::PathWhitelist);
    }
}
