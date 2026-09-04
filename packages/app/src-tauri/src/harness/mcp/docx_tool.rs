//! `inspect_docx` / `edit_docx` / `write_docx` 工具（word-capability-roadmap
//! 步骤 2 / 步骤 3 / 九波 D16）。
//!
//! - **inspect_docx**（S0b）：read_file 对 docx 只给线性文本（结构全丢）；本工具
//!   在结构模型上出三档投影（outline / format / text，见 harness::doc::docx_inspect），
//!   块号 1-based 混排统一编号——这是 edit_docx 的地址地基。
//! - **edit_docx**（步骤 3，D3 批量事务）：在块编址上做块级手术，一批操作全有或
//!   全无；手术引擎在 harness::doc::docx_edit（纯函数）。
//! - **write_docx**（D16 模板优先生成）：模板清空正文→按块序写入→生成自检→
//!   落盘，一次调用出整篇；引擎在 harness::doc::docx_write（纯函数）。
//!
//! 薄壳职责：读文件 + 扩展名守卫 + 参数解析 + 备份/原子写；全部业务逻辑在
//! harness::doc 纯函数层，独立单测。错误契约三段式：not-found 挂 did-you-mean；
//! 非 docx 指向 read_file / inspect_docx。

use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::harness::doc::{
    apply_edits_to_bytes_locked, apply_numbering_edits_to_bytes, apply_style_edits_to_bytes,
    build_builtin_template, generate_from_template, inspect_document, load_image,
    validate_document, AssertSpec, BUILTIN_TEMPLATES, EditOp, InspectProjection, InspectRequest,
    MAX_WRITE_BLOCKS, NumberingEditOp, StyleContainer, StyleEditOp, StyleType, ValidateReport,
    WriteBlock,
};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

pub struct InspectDocxTool;

#[derive(Deserialize)]
struct InspectDocxArgs {
    path: String,
    /// outline（默认，全图）/ format（run 级格式）/ text（带块号正文）/ headers_footers（页眉页脚）
    /// / table（表格网格）/ tblpr（表格属性原文，S3 四波①）/ ppr（段落属性原文）
    /// / styles·styledef·numbering（样式与编号定义三投影，D12）
    #[serde(default)]
    projection: Option<String>,
    /// 起始块号（1-based，含）
    #[serde(default)]
    start: Option<usize>,
    /// 结束块号（1-based，含）
    #[serde(default)]
    end: Option<usize>,
    /// 行号（仅 tblpr：level 下钻到行级 trPr）
    #[serde(default)]
    row: Option<usize>,
    /// 格号（仅 tblpr 且 row 给定：下钻到格级 tcPr）
    #[serde(default)]
    cell: Option<usize>,
    /// 样式显示名或 ID（仅 styledef：下钻该 w:style 原文）
    #[serde(default)]
    style: Option<String>,
    /// numId（仅 numbering：下钻该编号实例段）
    #[serde(default)]
    num_id: Option<u32>,
    /// ilvl 0-8（仅 numbering 且 num_id 给定：下钻该级 w:lvl 原文）
    #[serde(default)]
    level: Option<u32>,
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
         it, send it back; also the verification view after that op). For table blocks, \
         pass row+cell to list the pPr of every paragraph inside that cell (same \
         addressing as set_cell_format). \
         projection=text: document text with block-number prefixes. \
         projection=headers_footers: per-section header/footer references with their \
         content (start/end do not apply — organized by sections, not block ranges). \
         projection=table: per-table cell grid for a block range — every cell on one row \
         line with merge/nested markers plus a table-property summary line (style, \
         shading, borders, width) and per-cell format markers; the cell address scheme \
         (row r × cell c, both 1-based) is exactly what edit_docx set_cell_text / \
         set_cell_format use. projection=tblpr: raw table-property XML — tblPr per table \
         block by default, trPr with row, tcPr with row+cell; the editing basis for \
         edit_docx set_table_element (copy the element XML you see here, modify it, send \
         it back; also the verification view after that op). \
         projection=styles: one line per style — ID, display name, type \
         (paragraph/character/table/numbering), basedOn chain, own features; the \
         addressing map for edit_docx create_style / set_style_element (the style \
         parameter takes the display name or the ID; duplicate display names must use \
         the ID). projection=styledef: the raw XML of one w:style definition (style= \
         display name or ID) — the copy source for set_style_element and its \
         verification view; never write style XML from memory. \
         projection=numbering: one section per numId (shared-abstractNum disclosure, \
         body reference count, per-level summaries: numFmt / lvlText / start); num_id \
         drills into that instance, num_id+level into the raw w:lvl — the editing basis \
         for set_numbering_element (auto-numbering format, level text, indents all live \
         in numbering.xml, not in the paragraphs). \
         Blocks are numbered \
         1-based in document order (paragraphs and tables together); these numbers are \
         the addressing scheme used to reference locations for editing. Workflow: outline \
         first to locate blocks, then format/text/table/tblpr with start/end for details. \
         Defaults: outline renders up to 400 blocks, format up to 50, ppr up to 20, text \
         up to 100, table/tblpr up to 10, styles up to 200 lines (use style= to drill \
         down); styledef/numbering ignore start/end."
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
                    "enum": ["outline", "format", "ppr", "text", "headers_footers", "table", "tblpr", "styles", "styledef", "numbering"],
                    "description": "Level of detail: outline (block map, default), format (run-level formatting), ppr (raw pPr XML per block — basis for set_ppr_element), text (block-numbered text), headers_footers (per-section header/footer content; no start/end), table (cell grid of table blocks — basis for set_cell_text / set_cell_format), tblpr (raw tblPr/trPr/tcPr XML — basis for set_table_element), styles (style list — addressing map for create_style / set_style_element), styledef (raw w:style XML of one style — basis for set_style_element; requires style), numbering (numId catalog + per-level summaries; num_id+level drills to raw w:lvl — basis for set_numbering_element)."
                },
                "start": {
                    "type": "integer",
                    "description": "First block number to render (1-based, inclusive). Default 1. Not applicable to headers_footers."
                },
                "end": {
                    "type": "integer",
                    "description": "Last block number to render (1-based, inclusive). Default: start + span - 1 (span depends on projection). Not applicable to headers_footers."
                },
                "row": {
                    "type": "integer",
                    "description": "Row number r (1-based; projection=tblpr: that row's trPr; projection=ppr on a table block: that cell's paragraphs, requires cell). Must be used with the block range selecting one table block."
                },
                "cell": {
                    "type": "integer",
                    "description": "Cell number c (1-based, requires row; projection=tblpr: that cell's tcPr; projection=ppr on a table block: every paragraph's pPr inside the cell)."
                },
                "style": {
                    "type": "string",
                    "description": "Style display name or ID (projection=styledef only): drill down to that w:style's raw XML."
                },
                "num_id": {
                    "type": "integer",
                    "description": "numId (projection=numbering only): drill down to that numbering instance's section."
                },
                "level": {
                    "type": "integer",
                    "description": "ilvl 0-8 (projection=numbering only, requires num_id): drill down to that level's raw w:lvl XML."
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
            Some("tblpr") => InspectProjection::Tblpr,
            Some("styles") => InspectProjection::Styles,
            Some("styledef") => InspectProjection::Styledef,
            Some("numbering") => InspectProjection::Numbering,
            Some(other) => {
                return Err(AppError::Validation(format!(
                    "未知投影档位: {other}。支持 outline（块级地图，默认）/ format（run 级格式）/ text（带块号正文）/ headers_footers（页眉页脚）/ table（表格网格）/ tblpr（表格属性原文）/ ppr（段落属性原文）/ styles（样式清单）/ styledef（单样式定义原文）/ numbering（编号目录）。"
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
            &InspectRequest {
                projection,
                start: parsed.start,
                end: parsed.end,
                row: parsed.row,
                cell: parsed.cell,
                style: parsed.style,
                num_id: parsed.num_id,
                level: parsed.level,
            },
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
// validate_docx —— 断言验收工具（D15 八波①）
// =========================================================================

pub struct ValidateDocxTool;

#[derive(Deserialize)]
struct ValidateDocxArgs {
    path: String,
    /// 断言批（≤50 条；全部独立评估不短路，一次报告全量）
    assertions: Vec<AssertSpec>,
}

#[derive(Serialize)]
struct ValidateDocxResult {
    path: String,
    #[serde(flatten)]
    report: ValidateReport,
}

#[async_trait]
impl McpClient for ValidateDocxTool {
    fn name(&self) -> &str {
        "validate_docx"
    }

    fn description(&self) -> &str {
        "Executable acceptance check for a Word .docx document — turns 'what does done \
         look like' into machine-verifiable assertions instead of repeated inspect + \
         eyeballing. Takes a list of assertions (max 50), evaluates ALL of them \
         independently (no short-circuit), reports per-assertion pass/fail. \
         kind=block_count: total block count. kind=table_shape: the block must be a \
         table; optional rows / cols (cols = max gridSpan sum per row — the same \
         wording as the projection=table header) and style (table style display name \
         or ID). kind=block_text: paragraph full-text match — exactly one of equals / \
         contains / starts_with (full text, not the 60-char outline summary). \
         kind=block_style: paragraph style display name (outline meta wording; \
         unstyled paragraphs are '(无样式)'). kind=cell_text: cell full text \
         (multi-paragraph cells join with \\n; block/row/cell addressing identical to \
         projection=table; a vMerge continuation cell fails with a pointer to its \
         merge head). kind=cell_paragraph_count: paragraph count inside one cell. \
         kind=block_image: paragraph image count (omit equals for 'has ≥1 image'; \
         matches the [图片×N] projection suffix; paragraph blocks only). \
         kind=block_field: paragraph field instructions must contain this substring \
         (e.g. 'TOC', 'PAGE'; matches the [域:…] projection suffix; paragraph blocks \
         only). \
         An assertion failure is a NORMAL result (passed=false plus per-assertion \
         expected-vs-actual details), not a tool error; out-of-range block/row/cell \
         addresses are per-assertion failures showing the actual extent. Usage \
         pattern: when delegating document work, send the acceptance list along \
         (table N rows × M cols, r1c1 = ..., style = ...) so the executor can \
         self-check before claiming done — then re-run the SAME list to audit the \
         result without re-inspecting by eye."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the .docx file to validate."
                },
                "assertions": {
                    "type": "array",
                    "maxItems": 50,
                    "description": "Acceptance assertions; all evaluated independently, results reported per-assertion.",
                    "items": {
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "block_count" },
                                    "equals": { "type": "integer", "description": "Expected total block count." }
                                },
                                "required": ["kind", "equals"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "table_shape" },
                                    "block": { "type": "integer", "description": "Block number (1-based)." },
                                    "rows": { "type": "integer", "description": "Expected row count (optional)." },
                                    "cols": { "type": "integer", "description": "Expected column count = max gridSpan sum per row (optional)." },
                                    "style": { "type": "string", "description": "Expected table style display name or ID (optional)." }
                                },
                                "required": ["kind", "block"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "block_text" },
                                    "block": { "type": "integer", "description": "Paragraph block number (1-based)." },
                                    "equals": { "type": "string", "description": "Full text must equal this." },
                                    "contains": { "type": "string", "description": "Text must contain this." },
                                    "starts_with": { "type": "string", "description": "Text must start with this." }
                                },
                                "required": ["kind", "block"],
                                "description": "Exactly one of equals / contains / starts_with."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "block_style" },
                                    "block": { "type": "integer", "description": "Paragraph block number (1-based)." },
                                    "equals": { "type": "string", "description": "Expected style display name (outline meta wording)." }
                                },
                                "required": ["kind", "block", "equals"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "cell_text" },
                                    "block": { "type": "integer", "description": "Table block number (1-based)." },
                                    "row": { "type": "integer", "description": "Row r (1-based, projection=table addressing)." },
                                    "cell": { "type": "integer", "description": "Cell c (1-based)." },
                                    "equals": { "type": "string", "description": "Full cell text must equal this (multi-paragraph joined with \\n)." },
                                    "contains": { "type": "string", "description": "Cell text must contain this." },
                                    "starts_with": { "type": "string", "description": "Cell text must start with this." }
                                },
                                "required": ["kind", "block", "row", "cell"],
                                "description": "Exactly one of equals / contains / starts_with."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "cell_paragraph_count" },
                                    "block": { "type": "integer", "description": "Table block number (1-based)." },
                                    "row": { "type": "integer", "description": "Row r (1-based)." },
                                    "cell": { "type": "integer", "description": "Cell c (1-based)." },
                                    "equals": { "type": "integer", "description": "Expected paragraph count inside the cell." }
                                },
                                "required": ["kind", "block", "row", "cell", "equals"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "block_image" },
                                    "block": { "type": "integer", "description": "Paragraph block number (1-based)." },
                                    "equals": { "type": "integer", "description": "Expected image count in the paragraph; omit to assert 'has at least one image'." }
                                },
                                "required": ["kind", "block"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "block_field" },
                                    "block": { "type": "integer", "description": "Paragraph block number (1-based)." },
                                    "instr_contains": { "type": "string", "description": "Substring that must appear in the paragraph's field instructions (e.g. 'TOC')." }
                                },
                                "required": ["kind", "block", "instr_contains"]
                            }
                        ]
                    }
                }
            },
            "required": ["path", "assertions"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: ValidateDocxArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("validate_docx 参数解析失败: {e}")))?;

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
                "不是 Word 文档: 扩展名 .{ext}。validate_docx 只支持 .docx；其他文件请用 read_file。"
            )));
        }

        let bytes = tokio::fs::read(&canonical)
            .await
            .map_err(|e| AppError::Io(std::io::Error::other(format!("读取文件失败: {e}"))))?;

        let report = validate_document(&bytes, &parsed.assertions)?;
        let result = ValidateDocxResult { path: parsed.path, report };
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
    /// 操作批（全有或全无；段级操作每块限一个——例外：同锚块可挂多条
    /// insert_paragraph_after / insert_image_after / insert_toc_after 按序链式插入；
    /// 表格操作（set_cell_text /
    /// insert_table_row_after / set_cell_format / set_table_element）可同块多条
    /// 按序组合，且**跨表格块也可同批**（一次给多张表挂同一属性）——同格内
    /// set_table_element 按元素去重（不同元素可组合），内容/格式手术每格限一条；
    /// 结构重构（merge_cells / split_cell / delete_table_row）作用于**互不相交的
    /// 行区间**时可同批多条（如整列逐行横并一次发完；足迹相交或与内容操作同块
    /// 则拒，拆批）。
    /// 正文操作与定义操作（create_style / set_style_element /
    /// set_numbering_element / clear_body）不可混批（部件互斥），拆两批先后发
    operations: Vec<OperationSpec>,
    /// 范围锁（D15 八波②）：`[lo, hi]` 闭区间（1-based 块号，与 inspect 块号同口径）。
    /// 设了锁 → 批内任何操作的块地址越界整批拒（引擎硬约束，不靠 agent 自觉）；
    /// clear_body / 样式族 / 编号族与锁冲突拒（它们无块号概念或作用于全文）。
    #[serde(default)]
    allowed_blocks: Option<(usize, usize)>,
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
    /// 锚块后建新表：rows 矩形矩阵（首行默认表头——加粗 + 跨页重复）；列宽均分。
    /// table_style（D12 模板件）：表样式显示名或 ID——建表即挂模板样式（底纹/
    /// 边框随样式走）；缺省默认全边框直排。
    /// rows 与 rows_text 二选一（D15 八波③值优先）：rows=结构化矩阵（格内可多段，
    /// \n=多段）；rows_text=Markdown 表格或 TSV 纯文本，工具层解析成矩阵（弱构造
    /// 模型优先形态——纯文本是强项、嵌套 JSON 是翻车点；格内只支持单段文本）
    InsertTableAfter {
        block: usize,
        expect_prefix: String,
        #[serde(default)]
        rows: Option<Vec<Vec<String>>>,
        #[serde(default)]
        rows_text: Option<String>,
        #[serde(default)]
        header: Option<bool>,
        #[serde(default)]
        table_style: Option<String>,
    },
    /// 锚块后插入图片（D18 十波）：image_path = 图片文件路径（绝对，或相对
    /// agent workspace；png/jpg，≤10MiB）；width_mm 显式宽（毫米，钳版心；
    /// 缺省 min(原生像素宽, 版心) 不放大小图，高等比）。同锚可与
    /// insert_paragraph_after / insert_toc_after 链式同批（按输入序）
    InsertImageAfter {
        block: usize,
        expect_prefix: String,
        image_path: String,
        #[serde(default)]
        width_mm: Option<f64>,
    },
    /// 锚块后插入 TOC 目录域（D18 十波）：levels 1-9（目录收录的标题深度，缺省
    /// 3）；hyperlink 目录项超链接（缺省 true）。settings 自动置 updateFields——
    /// Word 打开即刷新目录；WPS 不保证（需全选后 F9，域缓存首刷前显示占位文案）
    InsertTocAfter {
        block: usize,
        expect_prefix: String,
        #[serde(default)]
        levels: Option<u32>,
        #[serde(default)]
        hyperlink: Option<bool>,
    },
    /// 改单元格文本：(row, cell) 双 1-based，与 projection=table 网格同口径；
    /// 保 tcPr；\n = 格内多段。格式保真：新段落数与原格相等 → 逐段按位继承
    /// 原各段格式；不等 → 回落首段格式（摘要会标注）
    SetCellText { block: usize, expect_prefix: String, row: usize, cell: usize, text: String },
    /// 克隆模板行增行（after_row 缺省 = 末行）：整结构克隆（tcPr/gridSpan/vMerge
    /// 原样），格文本替换为 cells（缺省全空；填充段与模板格段数相等时逐段
    /// 继承格式）——合并格表格唯一正确增行方式
    InsertTableRowAfter {
        block: usize,
        expect_prefix: String,
        #[serde(default)]
        after_row: Option<usize>,
        #[serde(default)]
        cells: Option<Vec<String>>,
    },
    /// 改格内文字格式：set_format 的格级版（段落格式作用于格内全部段落，字符格式
    /// 作用于格内全部 run）；(row, cell) 双 1-based。style=格内全段套段落样式
    /// （显示名或 ID）——表格格内段落脱离正文样式（如首行缩进透出）的正路
    SetCellFormat {
        block: usize,
        expect_prefix: String,
        row: usize,
        cell: usize,
        #[serde(default)]
        paragraph: Option<ParaFormatSpec>,
        #[serde(default)]
        character: Option<CharFormatSpec>,
        #[serde(default)]
        style: Option<String>,
    },
    /// 通用表格属性元素手术（set_ppr_element 的容器版）：level=table/row/cell
    /// 三档容器（tblPr/trPr/tcPr）；element 为容器子元素名（无 w: 前缀）；xml=null
    /// 移除整元素，xml=片段按 schema 序替换/插入。片段从 inspect_docx
    /// projection=tblpr 的原文复制修改，不凭记忆新写
    SetTableElement {
        block: usize,
        expect_prefix: String,
        level: TableLevelSpec,
        #[serde(default)]
        row: Option<usize>,
        #[serde(default)]
        cell: Option<usize>,
        element: String,
        #[serde(default)]
        xml: Option<String>,
    },
    /// 合并单元格（内容语义）：horizontal=同行 span 格并 1 格（gridSpan 求和、
    /// 内容按序拼接——**空段是结构占位不搬运**，段落数不膨胀，剔除数进摘要）；
    /// vertical=同列 span 行纵并（vMerge 头 restart 续 continue、内容留原格，
    /// split_cell 即恢复）。矩形区=direction/span 不传、改传 end_row+end_cell
    ///（区域右下角），(row,cell) 至 (end_row,end_cell) 一次合并。
    /// 结构重构：同表多条须作用于互不相交的行（如整列逐行横并可一批发完）
    MergeCells {
        block: usize,
        expect_prefix: String,
        /// 简单线并用（缺省 2）；矩形区（end_row+end_cell）不传
        #[serde(default)]
        direction: Option<MergeDirectionSpec>,
        row: usize,
        cell: usize,
        #[serde(default)]
        span: Option<usize>,
        #[serde(default)]
        end_row: Option<usize>,
        #[serde(default)]
        end_cell: Option<usize>,
    },
    /// 拆分单元格（merge_cells 的逆）：vertical=拆整条纵并链（各格内容恢复独立
    /// 显示）；horizontal=跨 N 列格拆回 N 个单格（内容留首格，其余空段）
    SplitCell {
        block: usize,
        expect_prefix: String,
        direction: MergeDirectionSpec,
        row: usize,
        cell: usize,
    },
    /// 删除表格一行（S3 七波·生产反馈 P0）：row 1-based 与 projection=table 同
    /// 口径。结构重构（该行下方行号整体前移，与同表其他操作的行区间相交即拒）；
    /// 行内含纵向合并头且下方有续格 → 拒（指路先 split_cell）；仅剩 1 行 → 拒
    ///（指路 delete_block 删整表）
    DeleteTableRow { block: usize, expect_prefix: String, row: usize },
    /// 清空正文（模板复用终件，D12）：删全部 body 块（含 sectPr 的块跳过——分节/
    /// 页面/页眉页脚结构保留）；expect_blocks = 当前块数指纹（防错删别人的文档）。
    /// 独占一批
    ClearBody { expect_blocks: usize },
    /// 新建样式（最小出生：type/name/basedOn/qFormat；细节同批 set_style_element
    /// 补——寻址放应用期，create→set 天然可组合）。name/ID 双撞拒
    CreateStyle {
        style_type: StyleTypeSpec,
        name: String,
        /// 缺省由显示名去空白派生
        #[serde(default)]
        style_id: Option<String>,
        /// 父样式（显示名或 ID）
        #[serde(default)]
        based_on: Option<String>,
    },
    /// 样式定义元素手术：style=显示名或 ID（重名显示名拒、指路 ID）；container
    /// 四档（style=直接子级 / pPr / rPr / tblPr）；element=容器内子元素名（无
    /// w: 前缀）；xml=null 摘除、片段按 schema 序整元素替换/插入。片段从
    /// projection=styledef 的原文复制修改，不凭记忆新写
    SetStyleElement {
        style: String,
        container: StyleContainerSpec,
        element: String,
        #[serde(default)]
        xml: Option<String>,
    },
    /// 编号级元素手术：numId 经 w:num 解析到 abstractNum 的第 level 级（0-8）；
    /// element 合法集 start/numFmt/lvlRestart/pStyle/isLgl/suff/lvlText/
    /// lvlPicBulletId/legacy/lvlJc/pPr/rPr。原文从 projection=numbering 的
    /// num_id+level 下钻视图复制修改
    SetNumberingElement {
        num_id: u32,
        level: u32,
        element: String,
        #[serde(default)]
        xml: Option<String>,
    },
}

/// 参数态容器档位（tblPr / trPr / tcPr）。
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum TableLevelSpec {
    Table,
    Row,
    Cell,
}

/// 参数态合并方向。
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum MergeDirectionSpec {
    Horizontal,
    Vertical,
}

/// 参数态样式类型四值。
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum StyleTypeSpec {
    Paragraph,
    Character,
    Table,
    Numbering,
}

/// 参数态容器四档（与 XML 标签同形：pPr / rPr / tblPr）。
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum StyleContainerSpec {
    Style,
    #[serde(rename = "pPr")]
    PPr,
    #[serde(rename = "rPr")]
    RPr,
    #[serde(rename = "tblPr")]
    TblPr,
}

impl From<TableLevelSpec> for crate::harness::doc::TableLevel {
    fn from(s: TableLevelSpec) -> Self {
        match s {
            TableLevelSpec::Table => crate::harness::doc::TableLevel::Table,
            TableLevelSpec::Row => crate::harness::doc::TableLevel::Row,
            TableLevelSpec::Cell => crate::harness::doc::TableLevel::Cell,
        }
    }
}

impl From<MergeDirectionSpec> for crate::harness::doc::MergeDirection {
    fn from(s: MergeDirectionSpec) -> Self {
        match s {
            MergeDirectionSpec::Horizontal => crate::harness::doc::MergeDirection::Horizontal,
            MergeDirectionSpec::Vertical => crate::harness::doc::MergeDirection::Vertical,
        }
    }
}

impl From<StyleTypeSpec> for StyleType {
    fn from(s: StyleTypeSpec) -> Self {
        match s {
            StyleTypeSpec::Paragraph => StyleType::Paragraph,
            StyleTypeSpec::Character => StyleType::Character,
            StyleTypeSpec::Table => StyleType::Table,
            StyleTypeSpec::Numbering => StyleType::Numbering,
        }
    }
}

impl From<StyleContainerSpec> for StyleContainer {
    fn from(s: StyleContainerSpec) -> Self {
        match s {
            StyleContainerSpec::Style => StyleContainer::Style,
            StyleContainerSpec::PPr => StyleContainer::PPr,
            StyleContainerSpec::RPr => StyleContainer::RPr,
            StyleContainerSpec::TblPr => StyleContainer::TblPr,
        }
    }
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

/// 操作分族（D12）：一批只落一个 XML 部件——document 侧预检基于**原** styles.xml
/// 解析，同批改定义会校验/应用态分裂；拆批零成本（第二批 fresh parse 天然看到
/// 第一批的结果），故部件互斥直接拒。
enum FamilyOp {
    /// word/document.xml（正文块手术）
    Doc(EditOp),
    /// word/styles.xml（样式定义手术）
    Style(StyleEditOp),
    /// word/numbering.xml（编号定义手术）
    Numbering(NumberingEditOp),
}

/// 参数态 → 引擎态的族路由转换。固有方法而非 From：rows_text 解析可失败
/// （D15 八波③）、图片装载可失败（D18 十波），From 签名装不下 Err。
/// `workspace`（D18）：insert_image_after 相对图片路径的解析锚（绝对路径直读）。
impl OperationSpec {
    fn into_family(self, workspace: Option<&str>) -> AppResult<FamilyOp> {
        Ok(match self {
            // ---- document 族 ----
            OperationSpec::ReplaceText { block, expect_prefix, new_text } => {
                FamilyOp::Doc(EditOp::ReplaceText { block, expect_prefix, new_text })
            }
            OperationSpec::InsertParagraphAfter { block, expect_prefix, text, style } => {
                FamilyOp::Doc(EditOp::InsertParagraphAfter { block, expect_prefix, text, style })
            }
            OperationSpec::DeleteBlock { block, expect_prefix } => {
                FamilyOp::Doc(EditOp::DeleteBlock { block, expect_prefix })
            }
            OperationSpec::SetStyle { block, expect_prefix, style } => {
                FamilyOp::Doc(EditOp::SetStyle { block, expect_prefix, style })
            }
            OperationSpec::SetFormat { block, expect_prefix, paragraph, character } => {
                FamilyOp::Doc(EditOp::SetFormat {
                    block,
                    expect_prefix,
                    paragraph: paragraph.map(Into::into),
                    character: character.map(Into::into),
                })
            }
            OperationSpec::SetPprElement { block, expect_prefix, element, xml } => {
                FamilyOp::Doc(EditOp::SetPprElement { block, expect_prefix, element, xml })
            }
            OperationSpec::InsertTableAfter { block, expect_prefix, rows, rows_text, header, table_style } => {
                let rows = resolve_table_rows(rows, rows_text)?;
                FamilyOp::Doc(EditOp::InsertTableAfter { block, expect_prefix, rows, header, table_style })
            }
            OperationSpec::InsertImageAfter { block, expect_prefix, image_path, width_mm } => {
                // 图片装载（读侧第二路径：只读不授权 + 格式/大小闸在装载层）
                let image = load_image(&image_path, workspace)?;
                FamilyOp::Doc(EditOp::InsertImageAfter {
                    block,
                    expect_prefix,
                    image,
                    width_mm,
                    // 注入占位——zip 层编排统一分配覆盖（引擎唯一写入点）
                    rid: String::new(),
                    cx_emu: 0,
                    cy_emu: 0,
                    docpr_id: 0,
                })
            }
            OperationSpec::InsertTocAfter { block, expect_prefix, levels, hyperlink } => {
                FamilyOp::Doc(EditOp::InsertTocAfter {
                    block,
                    expect_prefix,
                    levels: levels.unwrap_or(3),
                    hyperlink: hyperlink.unwrap_or(true),
                })
            }
            OperationSpec::SetCellText { block, expect_prefix, row, cell, text } => {
                FamilyOp::Doc(EditOp::SetCellText { block, expect_prefix, row, cell, text })
            }
            OperationSpec::InsertTableRowAfter { block, expect_prefix, after_row, cells } => {
                FamilyOp::Doc(EditOp::InsertTableRowAfter { block, expect_prefix, after_row, cells })
            }
            OperationSpec::SetCellFormat {
                block,
                expect_prefix,
                row,
                cell,
                paragraph,
                character,
                style,
            } => FamilyOp::Doc(EditOp::SetCellFormat {
                block,
                expect_prefix,
                row,
                cell,
                paragraph: paragraph.map(Into::into),
                character: character.map(Into::into),
                style,
            }),
            OperationSpec::SetTableElement { block, expect_prefix, level, row, cell, element, xml } => {
                FamilyOp::Doc(EditOp::SetTableElement {
                    block,
                    expect_prefix,
                    level: level.into(),
                    row,
                    cell,
                    element,
                    xml,
                })
            }
            OperationSpec::MergeCells {
                block,
                expect_prefix,
                direction,
                row,
                cell,
                span,
                end_row,
                end_cell,
            } => FamilyOp::Doc(EditOp::MergeCells {
                block,
                expect_prefix,
                direction: direction.map(Into::into),
                row,
                cell,
                span,
                end_row,
                end_cell,
            }),
            OperationSpec::SplitCell { block, expect_prefix, direction, row, cell } => {
                FamilyOp::Doc(EditOp::SplitCell {
                    block,
                    expect_prefix,
                    direction: direction.into(),
                    row,
                    cell,
                })
            }
            OperationSpec::DeleteTableRow { block, expect_prefix, row } => {
                FamilyOp::Doc(EditOp::DeleteTableRow { block, expect_prefix, row })
            }
            // ---- styles 族 ----
            OperationSpec::CreateStyle { style_type, name, style_id, based_on } => {
                FamilyOp::Style(StyleEditOp::CreateStyle {
                    style_type: style_type.into(),
                    name,
                    style_id,
                    based_on,
                })
            }
            OperationSpec::SetStyleElement { style, container, element, xml } => {
                FamilyOp::Style(StyleEditOp::SetStyleElement {
                    style,
                    container: container.into(),
                    element,
                    xml,
                })
            }
            // ---- numbering 族 ----
            OperationSpec::SetNumberingElement { num_id, level, element, xml } => {
                FamilyOp::Numbering(NumberingEditOp::SetNumberingElement {
                    num_id,
                    level,
                    element,
                    xml,
                })
            }
            // ---- clear_body：document 族（走 apply_edits_to_bytes）----
            OperationSpec::ClearBody { expect_blocks } => {
                FamilyOp::Doc(EditOp::ClearBody { expect_blocks })
            }
        })
    }
}

/// insert_table_after 行矩阵二选一解析（D15 八波③）：rows（结构化，格内可多段）
/// / rows_text（值优先——Markdown 表格或 TSV，工具层解析成矩阵）。都给/都不给 →
/// `表格文本无效:` 家族拒。解析只管文本→矩阵；非矩形由引擎 validate_table_rows 兜底。
fn resolve_table_rows(
    rows: Option<Vec<Vec<String>>>,
    rows_text: Option<String>,
) -> AppResult<Vec<Vec<String>>> {
    match (rows, rows_text) {
        (Some(rows), None) => Ok(rows),
        (None, Some(text)) => parse_rows_text(&text),
        (Some(_), Some(_)) => Err(AppError::Validation(
            "表格文本无效: rows 与 rows_text 同时给出，二者互斥。rows=结构化矩阵\
             （格内可多段，\\n=多段）；rows_text=Markdown 表格或 TSV 纯文本（平台\
             解析）。请只保留一种。"
                .into(),
        )),
        (None, None) => Err(AppError::Validation(
            "表格文本无效: insert_table_after 需要 rows 或 rows_text 之一。rows=结构化\
             矩阵（格内可多段，\\n=多段）；rows_text=Markdown 表格或 TSV 纯文本\
             （弱构造模型优先用这个）。"
                .into(),
        )),
    }
}

/// 解析行式表格文本：含 `|` 的行按 Markdown 表格（剥离 `\|---\|` 分隔行，`\|`
/// 转义字面竖线）；含制表符的行按 TSV；两者都不含 → 拒（无法切列）。空行忽略。
fn parse_rows_text(text: &str) -> AppResult<Vec<Vec<String>>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cells: Vec<String> = if line.contains('|') {
            split_markdown_row(line)
        } else if line.contains('\t') {
            line.split('\t').map(|c| c.trim().to_string()).collect()
        } else {
            return Err(AppError::Validation(format!(
                "表格文本无效: 行 {:?} 既不含 | 也不含制表符，无法切列。rows_text 须是\
                 Markdown 表格（| 分列）或 TSV（制表符分列），示例：| 列1 | 列2 |",
                clip(line, 30)
            )));
        };
        // Markdown 对齐分隔行（--- / :---: 等）= 表头与正文边界，跳过
        let is_separator = cells.iter().all(|c| {
            let t = c.trim_matches(':');
            !t.is_empty() && t.chars().all(|ch| ch == '-')
        });
        if is_separator {
            continue;
        }
        if cells.iter().all(String::is_empty) {
            return Err(AppError::Validation(format!(
                "表格文本无效: 行 {:?} 解析出全空列。请检查该行的分隔符（| 或制表符）数量。",
                clip(line, 30)
            )));
        }
        rows.push(cells);
    }
    if rows.is_empty() {
        return Err(AppError::Validation(
            "表格文本无效: rows_text 解析后为空。须至少一行数据（Markdown 表格或 TSV）。".into(),
        ));
    }
    Ok(rows)
}

/// 剥 Markdown 行首尾包裹竖线 + 按未转义 `|` 切列 + `\|` 还原字面竖线。
fn split_markdown_row(line: &str) -> Vec<String> {
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    let mut cells: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cur.push('|');
            }
            '|' => {
                let cell = std::mem::take(&mut cur);
                cells.push(cell.trim().to_string());
            }
            _ => cur.push(ch),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

/// char 安全截断（错误文案展示行内容用；勿用 String::truncate——中文 panic）。
fn clip(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
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
         explicit style by display name) — several insert_paragraph_after ops on the \
         SAME anchor chain in order within one batch, so a multi-paragraph entry \
         (label + description + attributes) is one call; op=delete_block removes a whole block; \
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
         that Word falls back to the style's numbers). Media & TOC ops: \
         op=insert_image_after inserts a picture after the anchor block (image_path = \
         absolute path or path relative to the workspace, png/jpg up to 10 MiB; \
         width_mm optional explicit width in mm clamped to the page content width — \
         default is min(native pixel width, content width), never upscaling small \
         images, height keeps aspect ratio; the image bytes are embedded into the \
         package, the source file is only read); op=insert_toc_after inserts a \
         table-of-contents field after the anchor block (levels 1-9 = heading depth \
         collected, default 3; hyperlink=true links TOC entries; the document's \
         settings get updateFields=true so Word refreshes the TOC on open — WPS may \
         not: select-all + F9 there; until first refresh the field shows a \
         placeholder line). Both chain with insert_paragraph_after on the same \
         anchor in input order within one batch. Table operations: \
         op=insert_table_after creates a new table after an anchor block from a \
         rectangular rows matrix (first row is a bold repeating header by default; \
         100% width, all borders, evenly split columns) — or from rows_text, a \
         Markdown-table/TSV text the platform parses (value-first: prefer it when \
         building nested JSON arrays is error-prone; one paragraph per cell); \
         op=set_cell_text rewrites one \
         cell's text by (row, cell) address — the exact grid shown by inspect_docx \
         projection=table, keeping the cell's structure properties; when the new text \
         splits into as many paragraphs as the cell already has, each new paragraph \
         positionally inherits its counterpart's formatting, otherwise formatting \
         falls back to the first paragraph's (disclosed in the result summary); \
         op=insert_table_row_after appends a row by cloning a \
         template row (default: the last one) so merged cells keep working. Table ops \
         (set_cell_text / insert_table_row_after / set_cell_format / set_table_element) \
         compose freely within one batch: several ops on the same table in order, AND \
         ops on different table blocks in the same batch (e.g. apply the same \
         tblCellMar or three-line borders to every table of the document in one call). \
         Table formatting ops: op=set_cell_format changes \
         paragraph and/or character formatting of one cell and/or applies a paragraph \
         style to it (paragraph formatting hits every paragraph in the cell, character \
         formatting every run — same param shape as set_format; style re-styles every \
         paragraph — the fix for a body style's first-line indent bleeding into cells, \
         usable together with paragraph.indent_first_line_tw=0); \
         op=set_table_element is the generic escape hatch for any \
         table-property element (borders tblBorders, shading shd, width tblW, cell \
         margins tblCellMar, row height trHeight, vertical alignment vAlign, ...) at \
         three container levels: level=table (tblPr, no row/cell), level=row (trPr, \
         with row), level=cell (tcPr, with row+cell) — xml=null removes the element, \
         xml=<w:...> fragment replaces/inserts it at its schema position (copy the \
         current XML from inspect_docx projection=tblpr, never write from memory; \
         gridSpan/hMerge/vMerge are protected — use merge_cells / split_cell instead; \
         several set_table_element ops on the SAME cell compose in one batch as long \
         as element differs, e.g. vAlign + tcBorders together). \
         Structural ops: op=delete_table_row removes one table row (row 1-based, same \
         as projection=table; refuses when the row holds a vertical-merge head whose \
         chain continues below — split_cell vertical first — and when it is the table's \
         only row — delete_block the whole table instead). op=merge_cells merges cells \
         with content semantics — \
         horizontal merges span adjacent cells in one row (gridSpan sums, content \
         concatenates into the first cell; empty placeholder paragraphs are structural \
         padding, not content — they are dropped instead of concatenating, so paragraph \
         counts do not balloon, and the count is disclosed in the result summary); \
         vertical merges cells across rows at the \
         same grid columns (vMerge head restarts, content stays in place — split_cell \
         restores it). op=split_cell is the inverse — vertical splits the whole merge \
         chain, horizontal splits a spanning cell back into unit cells (content stays \
         in the first). merge_cells / split_cell / delete_table_row renumber row/cell \
         addresses, so structural ops follow a footprint rule: several of them on the \
         same table compose in one batch as long as their ROW RANGES are disjoint \
         (e.g. one horizontal merge per row down a whole column = one batch); \
         overlapping footprints reject with the conflicting pair — split the batch. \
         They still cannot mix with content/paragraph ops on the same table (finish \
         structure first, then re-inspect projection=table to address content). \
         Definition ops (styles.xml / numbering.xml — 'define once, reference \
         everywhere'): op=create_style creates a new style (minimal birth: \
         style_type/name/based_on; add detail with set_style_element in the same batch); \
         op=set_style_element operates on one style's definition — container=style \
         (direct children like basedOn/qFormat) / pPr / rPr / tblPr, element=child name, \
         xml=null removes / fragment replaces-or-inserts at schema position (copy from \
         inspect_docx projection=styledef, never from memory). Changing a style \
         definition updates every paragraph using it at once — that is the point of \
         styles, and the right tool for house typography (heading fonts, body sizes). \
         op=set_numbering_element operates on one level of an auto-numbering list \
         (num_id + level 0-8; element like numFmt/lvlText/start/lvlJc/pPr — copy raw \
         from projection=numbering drill-down). op=clear_body removes every body block \
         (fingerprint: expect_blocks = current block count; blocks holding sectPr are \
         kept — sections/pages/headers preserved) — the final step of template reuse. \
         Body ops and definition ops cannot mix in one batch (component exclusivity) — \
         send two batches. Recipes: header emphasis = set_table_element level=row \
         element=shd on the header row + set_cell_format character; banded rows = \
         set_table_element level=row element=shd on alternate rows; content-fitted \
         column widths = read projection=table, weight each column by its longest \
         text, set_table_element level=cell element=tcW on every cell proportionally \
         (sum ≈ table width); whole-document \
         typography = set_style_element on heading/body styles instead of per-paragraph \
         set_format. Creating a new document: use write_docx (template-first, one call — \
         workspace templates/ / shared templates folder ('formal-report.docx') / built-in \
         'report' / absolute path). The copy_file + \
         op=clear_body chain remains only for MULTI-SECTION templates that write_docx \
         rejects. Every \
         operation must carry expect_prefix, the current text prefix of its target block, \
         as a fingerprint guard — if any block no longer matches, the whole batch is \
         rejected and the file is left untouched. Blocks are addressed by inspect_docx \
         block numbers (1-based, paragraphs and tables in document order); text ops \
         (replace/delete/set_style/set_format/set_ppr_element) reject table and \
         revision-marked blocks. Optional allowed_blocks=[lo, hi] is a range lock for \
         delegated work: when set, any operation addressing a block outside lo..=hi \
         rejects the whole batch (engine-level hard constraint — use it when a task \
         must confine edits to a region, e.g. 'only the tables after section 3'). \
         The file is backed up \
         before writing. Workflow: inspect_docx (outline, then text/table/tblpr for \
         blocks; styles/styledef/numbering for definitions) to find targets, edit_docx, \
         then inspect_docx again to verify the result."
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
                     Each item is tagged with op; block numbers come from inspect_docx. \
                     Composition: table ops on different table blocks share one batch; \
                     several insert_paragraph_after / insert_image_after / \
                     insert_toc_after ops on the same anchor chain in input order; \
                     structural ops (merge_cells / split_cell / delete_table_row) compose \
                     on one table when their row ranges are disjoint (e.g. one horizontal \
                     merge per row down a column = one batch), but cannot mix with \
                     content/paragraph ops on the same table.",
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
                                "required": ["op", "block", "expect_prefix", "text"],
                                "description": "Insert a paragraph after the anchor. Several ops on the SAME anchor in one batch chain in order — a multi-paragraph entry (label + description + attributes) is one call."
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
                                        "description": "Structured rectangular matrix of cell texts (every row same length, 1-30 cells). \\n inside a cell = multiple paragraphs. Mutually exclusive with rows_text.",
                                        "items": { "type": "array", "minItems": 1, "maxItems": 30, "items": { "type": "string" } }
                                    },
                                    "rows_text": {
                                        "type": "string",
                                        "description": "Value-first alternative to rows (mutually exclusive — provide exactly one): paste the table as text and the platform parses it. A line containing | parses as a Markdown row (leading/trailing wrapping pipes optional, \\| escapes a literal pipe, the |---|---| separator line is skipped); otherwise a line with tabs parses as TSV. Blank lines are ignored. LIMITATION: one paragraph per cell — for multi-paragraph cells use the rows array. Preferred when constructing nested JSON is error-prone."
                                    },
                                    "header": { "type": "boolean", "description": "true (default): first row is bold and repeats across pages; false: plain data rows only." },
                                    "table_style": { "type": "string", "description": "Table style (display name or ID, @w:type=table — see projection=styles): attach the house template's table style so shading/borders follow it. Default: plain single-border table." }
                                },
                                "required": ["op", "block", "expect_prefix"],
                                "description": "Create a new table after the anchor block. Cell data comes from rows (structured matrix; supports multi-paragraph cells) OR rows_text (Markdown/TSV text the platform parses; single-paragraph cells — the value-first form when nested JSON is error-prone). For house-template look pass table_style (a @w:type=table style — pick from inspect_docx projection=styles) so borders/shading/row banding follow the style; without it the table is plain: 100% width, single borders, evenly split columns, bold repeating header row."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "insert_image_after" },
                                    "block": { "type": "integer", "description": "Anchor block (1-based); the picture paragraph becomes the next block." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the anchor block (fingerprint guard)." },
                                    "image_path": { "type": "string", "description": "Picture file path: absolute, or relative to the workspace. png/jpg, up to 10 MiB. The source file is only read (never modified); bytes are embedded into the docx package." },
                                    "width_mm": { "type": "number", "description": "Optional explicit width in millimeters, clamped to the page content width. Default: min(native pixel width, content width) — small images are never upscaled; height always keeps aspect ratio." }
                                },
                                "required": ["op", "block", "expect_prefix", "image_path"],
                                "description": "Insert a picture after the anchor block. Chains with insert_paragraph_after / insert_toc_after on the same anchor in input order within one batch. The new block shows as [图片×1] in inspect_docx outline/text projections."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "insert_toc_after" },
                                    "block": { "type": "integer", "description": "Anchor block (1-based) — usually the document title; the TOC field becomes the next block." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the anchor block (fingerprint guard)." },
                                    "levels": { "type": "integer", "minimum": 1, "maximum": 9, "description": "Heading depth collected into the TOC (default 3: headings 1-3)." },
                                    "hyperlink": { "type": "boolean", "description": "true (default): TOC entries hyperlink to sections." }
                                },
                                "required": ["op", "block", "expect_prefix"],
                                "description": "Insert a table-of-contents field after the anchor block. The document's settings get updateFields=true so Word refreshes the TOC automatically on open; WPS may not — select all and press F9 there. Until first refresh the field shows a placeholder line instead of page numbers. TOC entries come from heading styles (write_docx headings / 标题 N styles)."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "set_cell_text" },
                                    "block": { "type": "integer", "description": "Target table block (1-based)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the table block (fingerprint guard; any cell text works)." },
                                    "row": { "type": "integer", "description": "Row number r (1-based) — same as the rN lines of inspect_docx projection=table." },
                                    "cell": { "type": "integer", "description": "Cell number c within the row (1-based; a merged/spanning cell counts as one)." },
                                    "text": { "type": "string", "description": "New cell text. \\n = multiple paragraphs inside the cell; empty string = clear. Format fidelity: when paragraph count matches the cell's existing paragraphs, each new paragraph inherits its counterpart's formatting (positional); otherwise all fall back to the first paragraph's formatting." }
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
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "set_cell_format" },
                                    "block": { "type": "integer", "description": "Target table block (1-based)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the table block (fingerprint guard)." },
                                    "row": { "type": "integer", "description": "Row number r (1-based) — same as the rN lines of inspect_docx projection=table." },
                                    "cell": { "type": "integer", "description": "Cell number c within the row (1-based; a merged/spanning cell counts as one)." },
                                    "style": {
                                        "type": "string",
                                        "description": "Paragraph style (display name or ID, from inspect_docx outline or projection=styles) applied to EVERY paragraph in the cell. This is the clean way to escape a body style bleeding into table cells (e.g. Normal's first-line indent): point the cell at a style without that indent, or combine with paragraph.indent_first_line_tw=0."
                                    },
                                    "paragraph": {
                                        "type": "object",
                                        "description": "Paragraph formatting applied to EVERY paragraph in the cell; omitted fields unchanged. Units match set_format.",
                                        "properties": {
                                            "align": { "type": "string", "enum": ["left", "center", "right", "both", "distribute", "start", "end"] },
                                            "line_spacing": { "type": "number", "description": "Line spacing multiple, e.g. 1.5." },
                                            "space_before_pt": { "type": "number" },
                                            "space_after_pt": { "type": "number" },
                                            "indent_first_line_tw": { "type": "integer" },
                                            "indent_left_tw": { "type": "integer" }
                                        },
                                        "additionalProperties": false
                                    },
                                    "character": {
                                        "type": "object",
                                        "description": "Character formatting applied to EVERY run in the cell; omitted fields unchanged.",
                                        "properties": {
                                            "bold": { "type": "boolean" },
                                            "italic": { "type": "boolean" },
                                            "font_size_pt": { "type": "number" },
                                            "color": { "type": "string", "description": "Hex RGB without '#', e.g. FF0000." },
                                            "font": { "type": "string", "description": "Font family; sets eastAsia/ascii/hAnsi together." }
                                        },
                                        "additionalProperties": false
                                    }
                                },
                                "required": ["op", "block", "expect_prefix", "row", "cell"],
                                "description": "Change one cell's formatting (cell-level version of set_format): paragraph/character formats for every paragraph/run in the cell, and/or style to re-style every paragraph. At least one of style / a paragraph field / a character field must be set."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "set_table_element" },
                                    "block": { "type": "integer", "description": "Target table block (1-based)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the table block (fingerprint guard)." },
                                    "level": { "type": "string", "enum": ["table", "row", "cell"], "description": "Which property container to operate on: table → tblPr (no row/cell), row → trPr (with row), cell → tcPr (with row+cell)." },
                                    "row": { "type": "integer", "description": "Row number r (1-based); required for level=row/cell, rejected for level=table." },
                                    "cell": { "type": "integer", "description": "Cell number c (1-based); required for level=cell, rejected otherwise." },
                                    "element": {
                                        "type": "string",
                                        "description": "Container child element name without the w: prefix. tblPr (level=table): tblStyle tblpPr tblOverlap bidiVisual tblStyleRowBandSize tblStyleColBandSize tblW jc tblCellSpacing tblInd tblBorders shd tblLayout tblCellMar tblLook tblCaption tblDescription. trPr (level=row): cnfStyle divId gridBefore gridAfter wBefore wAfter cantSplit trHeight tblHeader tblCellSpacing jc hidden. tcPr (level=cell): cnfStyle tcW tcBorders shd noWrap tcMar textDirection tcFitText vAlign hideMark. gridSpan/hMerge/vMerge are protected (rejected — use merge_cells / split_cell)."
                                    },
                                    "xml": {
                                        "type": ["string", "null"],
                                        "description": "null (or omitted) = remove the whole element; a string = well-formed single-root fragment '<w:ELEMENT ...>...</w:ELEMENT>' replacing the existing element or inserted at its schema position. Copy the current XML from inspect_docx projection=tblpr and modify it — never write OOXML from memory. No xmlns declarations. Example: cell shading → level=cell, element=shd, xml='<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"DDEEFF\"/>'."
                                    }
                                },
                                "required": ["op", "block", "expect_prefix", "level", "element"],
                                "description": "Generic surgery on any table-property element — borders, shading, widths, margins, row heights, vertical alignment, repeating header (tblHeader), etc."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "merge_cells" },
                                    "block": { "type": "integer", "description": "Target table block (1-based). Structural op: composes with other structural ops on this table when their row ranges are disjoint." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the table block (fingerprint guard)." },
                                    "direction": { "type": "string", "enum": ["horizontal", "vertical"], "description": "Simple line merge; omit when using end_row+end_cell (rectangle region)." },
                                    "row": { "type": "integer", "description": "Row of the head (top-left) cell of the merged region (1-based)." },
                                    "cell": { "type": "integer", "description": "Head cell number within the row (1-based)." },
                                    "span": { "type": "integer", "minimum": 2, "description": "How many cells (horizontal) or rows (vertical) to merge; default 2. Simple mode only." },
                                    "end_row": { "type": "integer", "description": "Rectangle mode (with end_cell, no direction/span): bottom row of the region (1-based)." },
                                    "end_cell": { "type": "integer", "description": "Rectangle mode: rightmost cell of the bottom row (1-based)." }
                                },
                                "required": ["op", "block", "expect_prefix", "row", "cell"],
                                "description": "Merge cells with content semantics. Horizontal: merges span adjacent cells in one row — their texts concatenate into the head cell; empty placeholder paragraphs are structural padding, not content — they are dropped (paragraph counts do not balloon; drop count disclosed in the result). Vertical: merges cells across rows at the same grid columns — content stays in each cell, split_cell later restores independent display. Rectangle mode: omit direction/span, give end_row+end_cell — the whole (row,cell)..(end_row,end_cell) region merges in one op (row-wise horizontal merges, then the resulting column merges vertically)."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "split_cell" },
                                    "block": { "type": "integer", "description": "Target table block (1-based). Structural op: composes with other structural ops on this table when their row ranges are disjoint." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the table block (fingerprint guard)." },
                                    "direction": { "type": "string", "enum": ["horizontal", "vertical"] },
                                    "row": { "type": "integer", "description": "Row of the cell to split (1-based)." },
                                    "cell": { "type": "integer", "description": "Cell number within the row (1-based). vertical: the head (restart) cell of the merge chain. horizontal: the spanning (gridSpan > 1) cell." }
                                },
                                "required": ["op", "block", "expect_prefix", "direction", "row", "cell"],
                                "description": "Split a merged cell. Vertical: splits the whole vertical-merge chain under the head cell — each cell's content reappears. Horizontal: splits a spanning cell back into unit cells — content stays in the first, the rest get empty paragraphs with the same formatting."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "delete_table_row" },
                                    "block": { "type": "integer", "description": "Target table block (1-based). Structural op: claims this row through the table's last row (everything below shifts up)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the table block (fingerprint guard)." },
                                    "row": { "type": "integer", "description": "Row to delete (1-based, same numbering as inspect_docx projection=table)." }
                                },
                                "required": ["op", "block", "expect_prefix", "row"],
                                "description": "Delete one table row. Refuses when the row holds a vertical-merge HEAD whose chain continues below (split_cell vertical first, then delete), and when it is the table's only row (delete_block the whole table instead). Renumbers row addresses: rows below shift up, so its footprint runs to the table's last row — structural ops composing in one batch must not touch those rows."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "clear_body" },
                                    "expect_blocks": { "type": "integer", "description": "Fingerprint guard: the document's CURRENT total block count (from inspect_docx outline); mismatch rejects the batch." }
                                },
                                "required": ["op", "expect_blocks"],
                                "description": "Remove every body block (blocks holding sectPr are kept — sections/page setup/headers preserved). Template-reuse final step: copy a house template, clear_body, write fresh content. Must be the only op in the batch."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "create_style" },
                                    "style_type": { "type": "string", "enum": ["paragraph", "character", "table", "numbering"] },
                                    "name": { "type": "string", "description": "Style display name (what inspect_docx outline shows). Must be unique; the ID defaults to the name with whitespace removed." },
                                    "style_id": { "type": "string", "description": "Explicit styleId; default = name without whitespace." },
                                    "based_on": { "type": "string", "description": "Parent style (display name or ID) the new style inherits from." }
                                },
                                "required": ["op", "style_type", "name"],
                                "description": "Create a new style (minimal birth: type/name/basedOn/qFormat). Add formatting detail with set_style_element in the SAME batch — addressing happens at apply time, so create→set composes."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "set_style_element" },
                                    "style": { "type": "string", "description": "Target style — display name or styleId (duplicate display names must use the ID; see projection=styles)." },
                                    "container": { "type": "string", "enum": ["style", "pPr", "rPr", "tblPr"], "description": "Which property container: style = direct children of w:style, pPr = paragraph properties, rPr = character properties, tblPr = table properties (table styles only)." },
                                    "element": {
                                        "type": "string",
                                        "description": "Child element name without the w: prefix. container=style: name aliases basedOn next link autoRedefine hidden uiPriority semiHidden unhideWhenUsed qFormat locked personal personalCompose personalReply rsid pPr rPr tblPr trPr tcPr tblStylePr (name is protected from removal). container=pPr: the pPr whitelist (same as set_ppr_element). container=rPr: rStyle rFonts b bCs i iCs caps smallCaps strike dstrike outline shadow emboss imprint noProof snapToGrid vanish webHidden color spacing w kern position sz szCs highlight u effect bdr shd fitText vertAlign rtl cs em lang eastAsianLayout specVanish oMath. container=tblPr: the tblPr whitelist (same as set_table_element level=table)."
                                    },
                                    "xml": {
                                        "type": ["string", "null"],
                                        "description": "null (or omitted) = remove the whole element; a string = well-formed single-root fragment replacing the existing element or inserted at its schema position. Copy the current XML from inspect_docx projection=styledef and modify it — never write OOXML from memory. Example: heading color → style='heading 1', container=rPr, element=color, xml='<w:color w:val=\"1E4976\"/>'."
                                    }
                                },
                                "required": ["op", "style", "container", "element"],
                                "description": "Generic surgery on one STYLE DEFINITION (styles.xml). Changing a definition updates every paragraph using the style at once — the right tool for house typography (heading fonts/sizes/colors, body spacing). pPr-internal rPr is out of scope; use container=rPr for character formatting."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "set_numbering_element" },
                                    "num_id": { "type": "integer", "description": "numId of the list instance (see projection=numbering). numId=0 is Word's 'no numbering' marker and is rejected." },
                                    "level": { "type": "integer", "description": "ilvl 0-8 = list level 1-9.", "minimum": 0, "maximum": 8 },
                                    "element": {
                                        "type": "string",
                                        "description": "w:lvl child element name without the w: prefix. Legal elements (schema order): start numFmt lvlRestart pStyle isLgl suff lvlText lvlPicBulletId legacy lvlJc pPr rPr."
                                    },
                                    "xml": {
                                        "type": ["string", "null"],
                                        "description": "null = remove the element; a string = fragment replacing/inserting it (copy from projection=numbering num_id+level drill-down — never from memory). Examples: decimal→Chinese numbering numFmt xml='<w:numFmt w:val=\"chineseCounting\"/>'; level text lvlText xml='<w:lvlText w:val=\"%1、'/>'."
                                    }
                                },
                                "required": ["op", "num_id", "level", "element"],
                                "description": "Generic surgery on one LEVEL of an auto-numbering definition (numbering.xml). The op lands on the abstractNum level behind num_id; if several numIds share that abstractNum the summary discloses them (the change affects all). lvlOverride is out of scope."
                            }
                        ]
                    }
                },
                "allowed_blocks": {
                    "type": "array",
                    "items": { "type": "integer", "minimum": 1 },
                    "minItems": 2,
                    "maxItems": 2,
                    "description": "Range lock [lo, hi] (inclusive, 1-based block numbers) for delegated edits: when set, ANY operation whose block address falls outside the range rejects the whole batch (engine-level hard constraint — protects content the delegating agent must not touch). clear_body and style/numbering-family batches are incompatible with the lock. Omit for unrestricted edits."
                }
            },
            "required": ["path", "operations"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        // dispatch 统一走 execute_with_context（图片相对路径解析需要 workspace）；
        // 直调 execute 的旧路径（测试/无头）可用，图片相对路径会在装载层报
        // 「无 workspace」并指路绝对路径
        self.run(args, None).await
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        self.run(args, ctx.workspace.as_deref()).await
    }
}

impl EditDocxTool {
    /// 双入口共体：workspace 注入图片相对路径解析锚（None = 无头/测试直调）。
    async fn run(&self, args: &str, workspace: Option<&str>) -> AppResult<String> {
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

        // 分族路由：正文块手术 / 样式定义手术 / 编号定义手术，一批只落一个部件
        if parsed.operations.is_empty() {
            return Err(AppError::Validation(
                "操作列表为空: operations 至少需要一个操作。".into(),
            ));
        }
        // 区间锁参数合法性（D15 八波②）：1-based 闭区间，lo≥1 且 lo≤hi
        if let Some((lo, hi)) = parsed.allowed_blocks {
            if lo == 0 || lo > hi {
                return Err(AppError::Validation(format!(
                    "区间锁无效: allowed_blocks=[{lo}, {hi}]，须 lo≥1 且 lo≤hi\
                     （1-based 闭区间，与 inspect 块号同口径）。请用 inspect_docx \
                     outline 复核块号后重发。"
                )));
            }
        }
        let mut doc_ops: Vec<EditOp> = Vec::new();
        let mut style_ops: Vec<StyleEditOp> = Vec::new();
        let mut numbering_ops: Vec<NumberingEditOp> = Vec::new();
        for spec in parsed.operations {
            match spec.into_family(workspace)? {
                FamilyOp::Doc(op) => doc_ops.push(op),
                FamilyOp::Style(op) => style_ops.push(op),
                FamilyOp::Numbering(op) => numbering_ops.push(op),
            }
        }
        let family_count = usize::from(!doc_ops.is_empty())
            + usize::from(!style_ops.is_empty())
            + usize::from(!numbering_ops.is_empty());
        if family_count > 1 {
            return Err(AppError::Validation(
                "部件互斥: 一批操作只能落在同一个部件——正文块手术（replace_text 等）与\
                 样式/编号定义手术（create_style / set_style_element / \
                 set_numbering_element）不可混批。为什么：正文侧预检基于原 styles.xml 解析，\
                 同批改定义会校验/应用态分裂。怎么办：拆成两批先后发即可——第二批会\
                 看到第一批的结果，零额外成本。"
                    .into(),
            ));
        }
        // 锁只作用于正文块批：样式/编号定义按样式名/numId 寻址，无块号概念
        if parsed.allowed_blocks.is_some() && doc_ops.is_empty() {
            return Err(AppError::Validation(
                "区间锁无效: allowed_blocks 只作用于正文块手术批（replace_text / \
                 set_cell_text 等）。样式/编号定义手术按样式名或 numId 寻址，无块号\
                 概念。请去掉 allowed_blocks，或拆批后再带锁。"
                    .into(),
            ));
        }

        // 全有或全无：手术在内存完成（含整批预检 + 产物再解析校验 + 备份），通过才落盘。
        // 中段是同步重活（zip 解包/重打包 + 图片源读取），spawn_blocking 离开 async
        // worker（Q6）；文件读写两端（tokio::fs::read / write_and_rename）保持在 async 侧。
        let canonical_for_blocking = canonical.clone();
        let allowed_blocks = parsed.allowed_blocks;
        let (new_bytes, applied, backup) = tokio::task::spawn_blocking(move || {
            // 全有或全无：手术在内存完成（含整批预检 + 产物再解析校验），通过才落盘
            let (new_bytes, applied) = if !doc_ops.is_empty() {
                apply_edits_to_bytes_locked(&bytes, &doc_ops, allowed_blocks)?
            } else if !style_ops.is_empty() {
                apply_style_edits_to_bytes(&bytes, &style_ops)?
            } else {
                apply_numbering_edits_to_bytes(&bytes, &numbering_ops)?
            };
            // 修改前备份（与 write_file 同一通道）；tmp + rename 原子替换（崩溃不损坏原文件）
            let backup = super::file_tools::backup_if_exists(&canonical_for_blocking)?;
            Ok::<_, AppError>((new_bytes, applied, backup))
        })
        .await
        .map_err(|e| AppError::Internal(format!("edit_docx 手术任务失败: {e}")))??;
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
// write_docx —— 模板优先生成（word-capability-roadmap 九波 D16）
// =========================================================================

/// 新建 .docx：模板（相对名依次查 workspace templates/ → 软件共享目录 →
/// 内置档位兜底，或绝对路径直读）清空正文 → 按块序写入标题/段落/表格 →
/// 生成自检全过才落盘。生成引擎在 harness::doc::docx_write（纯函数）；
/// 薄壳职责 = 模板解析 + IO + 备份。
///
/// override `execute_with_context`：相对模板名需要 ctx.workspace 拼接、
/// 共享目录需要 ctx.app_handle 推导（search_kb 同款透传链路）。
pub struct WriteDocxTool;

#[derive(Deserialize)]
struct WriteDocxArgs {
    /// 目标 .docx 路径（不存在即新建；父目录缺失自动创建；已存在先备份再覆盖）
    path: String,
    /// 模板：相对名（依次查 workspace templates/ → 共享模板目录 → 内置档位兜底，
    /// 同名文件优先于内置档位）| 任意 .docx 绝对路径。缺省 "report"。
    #[serde(default)]
    template: Option<String>,
    /// 内容块序列（heading / paragraph / table / toc / image）
    blocks: Vec<WriteBlockSpec>,
}

/// blocks 元素（LLM 侧形态）。table 的 rows/rows_text 二选一互斥，与
/// insert_table_after 同一形状（值优先）。
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WriteBlockSpec {
    Heading {
        /// 标题级 1-9；样式按候选链反查模板（中文显示名/规范名/英文/裸 ID）
        level: u32,
        text: String,
    },
    Paragraph {
        /// 单段语义：\n 不拆段（多段=多 block，诚实边界）
        text: String,
        #[serde(default)]
        style: Option<String>,
    },
    Table {
        #[serde(default)]
        rows: Option<Vec<Vec<String>>>,
        /// 值优先：Markdown 表格或 TSV 纯文本
        #[serde(default)]
        rows_text: Option<String>,
        #[serde(default)]
        header: Option<bool>,
        #[serde(default)]
        style: Option<String>,
    },
    /// TOC 目录域（D18 十波）：levels/hyperlink 缺省 3/true
    Toc {
        #[serde(default)]
        levels: Option<u32>,
        #[serde(default)]
        hyperlink: Option<bool>,
    },
    /// 图片段（D18 十波）：path 相对 workspace 或绝对；width_mm 显式宽（缺省
    /// min(原生, 版心) 不放大，等比高）
    Image {
        path: String,
        #[serde(default)]
        width_mm: Option<f64>,
    },
}

#[derive(Serialize)]
struct WriteDocxResult {
    path: String,
    template: String,
    /// true = 新建；false = 已存在（旧文件已备份）
    created: bool,
    blocks: usize,
    paragraphs: usize,
    tables: usize,
    /// 图片块数（D18 十波）
    images: usize,
    /// TOC 域块数（D18 十波）
    tocs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    backup: Option<String>,
    bytes: usize,
    check: &'static str,
}

/// 模板解析 → 模板字节（相对名四层链）。
///
/// 绝对路径直读；相对名依次查 ① workspace `templates/`（更具体者优先）
/// ② 软件共享目录 `<app_data_dir>/templates/`（安装包模板落盘处，全 agent
/// 共享、用户可自行放模板）③ 内置档位名兜底（[`BUILTIN_TEMPLATES`]——
/// 同名文件可覆盖内置）。全 miss 报错列出两处目录现有模板 + 内置档位
/// （报错即行为契约）。
async fn resolve_template(
    spec: &str,
    workspace: Option<&str>,
    shared_dir: Option<&Path>,
) -> AppResult<Vec<u8>> {
    let p = Path::new(spec);
    let candidates: Vec<std::path::PathBuf> = if p.is_absolute() {
        vec![p.to_path_buf()]
    } else {
        // 无扩展名的相对名（如内置档位名形态的 "report"）补试 .docx 变体——
        // 「同名文件覆盖内置档位」的落地面（文件叫 report.docx，spec 写 report）。
        let variants: Vec<String> = if p.extension().is_none() {
            vec![spec.to_string(), format!("{spec}.docx")]
        } else {
            vec![spec.to_string()]
        };
        let mut v = Vec::new();
        for name in &variants {
            if let Some(ws) = workspace {
                v.push(Path::new(ws).join("templates").join(name));
            }
        }
        for name in &variants {
            if let Some(sd) = shared_dir {
                v.push(sd.join(name));
            }
        }
        v
    };
    for path in &candidates {
        match tokio::fs::read(path).await {
            Ok(bytes) => return Ok(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(AppError::Validation(format!(
                    "模板无效: 读取模板失败 {}: {e}。请确认路径与文件权限。",
                    path.display()
                )));
            }
        }
    }
    // 相对名文件全 miss → 内置档位兜底（绝对路径不兜底——路径是明确意图）
    if !p.is_absolute() && BUILTIN_TEMPLATES.iter().any(|(name, _)| *name == spec) {
        return build_builtin_template(spec);
    }
    if candidates.is_empty() {
        return Err(AppError::Validation(format!(
            "模板无效: 相对模板名 {:?} 无处可查（agent 未设 workspace，共享模板目录\
             也不可用）。{}",
            clip(spec, 40),
            available_templates_hint(workspace, shared_dir)
        )));
    }
    if p.is_absolute() {
        // 报错即行为契约：not-found 扫真实文件系统给近似候选
        return Err(AppError::Validation(format!(
            "模板无效: 模板文件不存在 {}。{}",
            p.display(),
            super::path_suggest::suggest_for_missing(p)
        )));
    }
    Err(AppError::Validation(format!(
        "模板无效: {:?} 不存在（已查 workspace templates/ 与共享模板目录）。{}",
        clip(spec, 40),
        available_templates_hint(workspace, shared_dir)
    )))
}

/// 模板 miss 时的可用清单：两处目录现有 .docx + 内置档位（错误文案展示用）。
fn available_templates_hint(workspace: Option<&str>, shared_dir: Option<&Path>) -> String {
    fn dir_section(label: &str, dir: &Path) -> String {
        let names: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.to_ascii_lowercase().ends_with(".docx"))
                    .collect()
            })
            .unwrap_or_default();
        let list = if names.is_empty() {
            "（空）".to_string()
        } else {
            names.join(" / ")
        };
        format!("{label} {}: {list}", dir.display())
    }
    let mut sections: Vec<String> = Vec::new();
    if let Some(ws) = workspace {
        sections.push(dir_section("workspace templates/", &Path::new(ws).join("templates")));
    }
    if let Some(sd) = shared_dir {
        sections.push(dir_section("共享模板目录", sd));
    }
    sections.push(format!("内置档位: {}", builtin_template_names()));
    format!(
        "可用模板——{}。取法：相对名依次查 workspace templates/ 与共享目录（同名\
         文件优先于内置档位），或任意 .docx 绝对路径。",
        sections.join("；")
    )
}

/// 内置档位名清单（错误文案展示用，斜杠分隔）。
fn builtin_template_names() -> String {
    BUILTIN_TEMPLATES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(" / ")
}

#[async_trait]
impl McpClient for WriteDocxTool {
    fn name(&self) -> &str {
        "write_docx"
    }

    fn description(&self) -> &str {
        "Create a new .docx document from a template in ONE call — the preferred way to \
         produce Word files. The template's styles / numbering / page setup are preserved \
         verbatim, its body is cleared, your blocks are written in order, and a built-in \
         self-check verifies every block (text, table shape, image/TOC fields) BEFORE \
         anything is written to disk. Template resolution: a relative name is looked up first under the agent \
         workspace templates/ directory (house templates), then in the app's SHARED \
         templates folder (ships with 'formal-report.docx' — formal report styles: \
         4-level headings, table/list styles, classified-mark header + page-number \
         footer; editable by the user, shared by all agents), then falls back to the \
         built-in name 'report' (Chinese report style: SimHei headings, SimSun body 12pt \
         with 1.5 line spacing, A4 single section) — a file with the same name overrides \
         the built-in. An absolute path to any .docx also works. Omit template for \
         'report'. House style tweaking: edit the template file itself, or adjust style \
         definitions in the generated file with edit_docx set_style_element. blocks is an \
         ordered list written top to bottom; each block is one of {type:'heading', level \
         1-9, text} (style resolved via the template's heading styles), \
         {type:'paragraph', text, style?} (ONE paragraph per block — \\n does NOT split \
         paragraphs, send multiple blocks; omit style for the template body default), \
         {type:'table', rows_text | rows, header=true, style?} (rows_text value-first: a \
         Markdown table or TSV; rows structured, where \\n inside a cell = multiple \
         paragraphs in that cell), {type:'toc', levels? 1-9 default 3, hyperlink? default \
         true} (a table-of-contents FIELD placed after the title heading — settings get \
         updateFields=true so Word refreshes it on open; WPS may need select-all + F9, a \
         placeholder line tells the reader), {type:'image', path, width_mm?} (embeds a \
         picture; png/jpg up to 10 MiB; path absolute or workspace-relative; width_mm \
         clamped to content width and never upscaled, height keeps aspect ratio). \
         Single-section templates only (multi-section templates \
         are rejected — for those use copy_file + edit_docx clear_body). The target's \
         parent directory is created if missing; an existing file at the target is backed \
         up to .icepaw-backup/ then replaced atomically. Result reports paragraphs/tables/\
         images/tocs counts and check:'passed' — inspect_docx the result only if you need structure \
         details."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Target .docx path to create (parent directories created if missing; existing file backed up then replaced)."
                },
                "template": {
                    "type": "string",
                    "description": "Template: a file name looked up in the agent workspace templates/ directory first, then the app shared templates folder ('formal-report.docx' lives there — formal report styles), then a built-in name ('report'); a same-named file overrides the built-in. An absolute .docx path also works. Omit for 'report'."
                },
                "blocks": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 300,
                    "description": "Ordered content blocks, written top to bottom.",
                    "items": {
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "type": { "const": "heading" },
                                    "level": { "type": "integer", "minimum": 1, "maximum": 9, "description": "Heading level; resolved against the template's heading styles." },
                                    "text": { "type": "string", "description": "Heading text (single line)." }
                                },
                                "required": ["type", "level", "text"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "type": { "const": "paragraph" },
                                    "text": { "type": "string", "description": "Paragraph text. ONE paragraph per block — \\n is not split; send multiple blocks for multiple paragraphs." },
                                    "style": { "type": "string", "description": "Optional style display name or ID from the template; omit for the body default (Normal)." }
                                },
                                "required": ["type", "text"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "type": { "const": "table" },
                                    "rows_text": { "type": "string", "description": "Value-first: Markdown table (| separated, --- separator row skipped) or TSV. Mutually exclusive with rows." },
                                    "rows": { "type": "array", "items": { "type": "array", "items": { "type": "string" } }, "description": "Structured rows; \\n inside a cell = multiple paragraphs in that cell. Mutually exclusive with rows_text." },
                                    "header": { "type": "boolean", "description": "true (default) = first row rendered bold as header." },
                                    "style": { "type": "string", "description": "Optional table style display name or ID from the template; omit for the default grid." }
                                },
                                "required": ["type"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "type": { "const": "toc" },
                                    "levels": { "type": "integer", "minimum": 1, "maximum": 9, "description": "Heading depth included in the TOC (1-9). Default 3." },
                                    "hyperlink": { "type": "boolean", "description": "TOC entries clickable. Default true." }
                                },
                                "required": ["type"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "type": { "const": "image" },
                                    "path": { "type": "string", "description": "Image file path: absolute, or relative to the agent workspace. png/jpg/jpeg up to 10 MiB. Read-only — the bytes are embedded into the document." },
                                    "width_mm": { "type": "number", "description": "Display width in millimeters. Clamped to the template content width; never upscaled. Omit for min(native, content width); height keeps aspect ratio." }
                                },
                                "required": ["type", "path"]
                            }
                        ]
                    }
                }
            },
            "required": ["path", "blocks"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        // write_docx 走 execute_with_context（L2 相对模板路径需要 workspace）；
        // dispatch 已统一调 execute_with_context，这里只是 trait 兜底。
        Err(AppError::Internal(
            "write_docx 必须通过 execute_with_context 调用（需要 workspace 上下文）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: WriteDocxArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("write_docx 参数解析失败: {e}")))?;

        // 目标守卫：.docx 扩展名（目标尚不存在，不 canonicalize——授权层
        // path_within_workspace 对不存在路径有词法归一回退，0.3.5 已修）
        let target = Path::new(&parsed.path);
        let ext = target
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "docx" {
            return Err(AppError::Validation(format!(
                "不是 Word 文档: 目标扩展名 .{ext}。write_docx 只生成 .docx；\
                 写纯文本请用 write_file。"
            )));
        }

        // blocks → 引擎模型（table 走 resolve_table_rows 值优先，与 insert_table_after 同形）
        if parsed.blocks.is_empty() {
            return Err(AppError::Validation(
                "生成块无效: blocks 为空。新建文档至少需要一个内容块（heading / \
                 paragraph / table）；只想复制模板不改内容请用 copy_file。"
                    .into(),
            ));
        }
        if parsed.blocks.len() > MAX_WRITE_BLOCKS {
            return Err(AppError::Validation(format!(
                "生成块无效: blocks 共 {} 条，超过上限 {}。请拆成 write_docx 建骨架 + \
                 edit_docx 续写两步。",
                parsed.blocks.len(),
                MAX_WRITE_BLOCKS
            )));
        }
        // 块模型构建（含 load_image 同步读 + 表格行解析）与生成段同属同步重活，
        // 各包 spawn_blocking 离开 async worker（Q6）。错误先后序保持原状：
        // 块构建错先于模板解析错（原代码块循环在前）。
        let specs = parsed.blocks;
        let workspace = ctx.workspace.clone();
        let blocks_spawned = tokio::task::spawn_blocking(move || {
            let mut blocks: Vec<WriteBlock> = Vec::with_capacity(specs.len());
            for spec in specs {
                match spec {
                    WriteBlockSpec::Heading { level, text } => {
                        blocks.push(WriteBlock::Heading { level, text });
                    }
                    WriteBlockSpec::Paragraph { text, style } => {
                        blocks.push(WriteBlock::Paragraph { text, style });
                    }
                    WriteBlockSpec::Table {
                        rows,
                        rows_text,
                        header,
                        style,
                    } => {
                        let rows = resolve_table_rows(rows, rows_text)?;
                        blocks.push(WriteBlock::Table {
                            rows,
                            header,
                            table_style: style,
                        });
                    }
                    WriteBlockSpec::Toc { levels, hyperlink } => {
                        blocks.push(WriteBlock::Toc {
                            levels: levels.unwrap_or(3),
                            hyperlink: hyperlink.unwrap_or(true),
                        });
                    }
                    WriteBlockSpec::Image { path, width_mm } => {
                        // 图片装载（读侧第二路径：同 template 先例只读不授权；
                        // 相对路径挂 workspace——run 侧与模板解析同锚）
                        let image = load_image(&path, workspace.as_deref())?;
                        blocks.push(WriteBlock::Image { image, width_mm });
                    }
                }
            }
            Ok::<_, AppError>(blocks)
        });
        let blocks = blocks_spawned
            .await
            .map_err(|e| AppError::Internal(format!("write_docx 块构建任务失败: {e}")))??;
        let block_count = blocks.len();

        // 模板解析（相对名四层链）→ 内存全链生成（清空→锚→顺序写→自检；自检不过不落盘）
        let template_spec = parsed.template.clone().unwrap_or_else(|| "report".into());
        // 共享模板目录从 AppHandle 推导（app_data_dir/templates/，boot 已 ensure
        // 落盘）；无 AppHandle（单测/无头环境）→ 仅 workspace + 内置档位两层。
        let shared_dir = ctx
            .app_handle
            .as_ref()
            .and_then(|h| crate::logging::data_dir(h).ok().map(|d| d.join("templates")));
        let template = resolve_template(
            &template_spec,
            ctx.workspace.as_deref(),
            shared_dir.as_deref(),
        )
        .await?;
        // 生成 = clear_body→锚→顺序写→validate 自检（zip 手术 + 整文档再解析），同步重活
        let generated = tokio::task::spawn_blocking(move || generate_from_template(&template, &blocks))
            .await
            .map_err(|e| AppError::Internal(format!("write_docx 生成任务失败: {e}")))??;

        // 父目录好默认：缺失自动创建（copy_file 同款）
        if let Some(parent) = target.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    AppError::Io(std::io::Error::other(format!("创建目标目录失败: {e}")))
                })?;
            }
        }

        // 已存在 → 备份后覆盖；tmp + rename 原子写（edit_docx 同通道）
        let backup = super::file_tools::backup_if_exists(target)?;
        let mut tmp_name = target.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(".icepaw-tmp");
        let tmp = target.with_file_name(tmp_name);
        if let Err(e) = write_and_rename(&tmp, target, &generated.bytes).await {
            tokio::fs::remove_file(&tmp).await.ok();
            return Err(AppError::Validation(format!(
                "write_docx 写入失败: {}: {e}。请确认路径在授权工作区内；备份位于 \
                 .icepaw-backup/（若有）。",
                target.display()
            )));
        }

        let result = WriteDocxResult {
            path: parsed.path,
            template: template_spec,
            created: backup.is_none(),
            blocks: block_count,
            paragraphs: generated.paragraphs,
            tables: generated.tables,
            images: generated.images,
            tocs: generated.tocs,
            backup,
            bytes: generated.bytes.len(),
            check: "passed",
        };
        Ok(serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()))
    }
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
            &InspectRequest { projection: InspectProjection::Text, start: None, end: None, row: None, cell: None, style: None, num_id: None, level: None },
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

    /// 定义族走工具真入口：create→set 同批组合 + styledef 投影读回 + 部件互斥拒。
    #[tokio::test]
    async fn style_family_edits_and_exclusive_batches() {
        let dir = std::env::temp_dir().join("icepaw_edit_docx_def_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("样本.docx");
        std::fs::write(&file, three_para_docx()).unwrap();

        // styles 族 happy path：create 最小出生 → 同批 set 补字符格式（寻址放应用期）
        let tool = EditDocxTool;
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "operations": [
                { "op": "create_style", "style_type": "paragraph", "name": "交付标题" },
                { "op": "set_style_element", "style": "交付标题", "container": "rPr",
                  "element": "sz", "xml": "<w:sz w:val=\"32\"/>" }
            ]
        })
        .to_string();
        let out = tool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["applied"], 2, "{out}");

        // styledef 投影经工具入口读回（定义手术的验证视图闭环）
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "projection": "styledef",
            "style": "交付标题"
        })
        .to_string();
        let out = InspectDocxTool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let content = v["content"].as_str().unwrap();
        assert!(content.contains("交付标题"), "{content}");
        assert!(content.contains(r#"<w:sz w:val="32"/>"#), "{content}");

        // 部件互斥：正文操作 + 定义操作混批拒，文件字节原样
        let before = std::fs::read(&file).unwrap();
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "operations": [
                { "op": "replace_text", "block": 1, "expect_prefix": "第一段", "new_text": "x" },
                { "op": "create_style", "style_type": "paragraph", "name": "另一个样式" }
            ]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("部件互斥"), "实际: {err}");
        assert_eq!(std::fs::read(&file).unwrap(), before);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- validate_docx（D15 八波①）----

    #[tokio::test]
    async fn validate_reports_failures_as_data_not_error() {
        let dir = std::env::temp_dir().join("icepaw_validate_docx_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("样本.docx");
        std::fs::write(&file, three_para_docx()).unwrap();

        let tool = ValidateDocxTool;
        // 混合断言：2 过 2 败——失败是正常输出（passed=false），不是工具 Err
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "assertions": [
                { "kind": "block_count", "equals": 3 },
                { "kind": "block_text", "block": 2, "contains": "第二" },
                { "kind": "block_text", "block": 1, "equals": "错文本" },
                { "kind": "cell_text", "block": 1, "row": 1, "cell": 1, "contains": "x" }
            ]
        })
        .to_string();
        let out = tool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["passed"], false, "{out}");
        assert_eq!(v["total"], 4);
        assert_eq!(v["failed"], 2);
        let kinds: Vec<&str> = v["failures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["kind"].as_str().unwrap())
            .collect();
        // 独立评估不短路：两条失败都报（段落块打 cell 断言 → fail 指路，非 Err）
        assert_eq!(kinds, ["block_text", "cell_text"], "{out}");

        // 全过形态：failures 空、passed_kinds 摘要在场
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "assertions": [
                { "kind": "block_count", "equals": 3 },
                { "kind": "block_style", "block": 1, "equals": "(无样式)" }
            ]
        })
        .to_string();
        let out = tool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["passed"], true, "{out}");
        assert_eq!(v["failed"], 0);
        assert!(v["failures"].as_array().unwrap().is_empty());
        assert!(!v["passed_kinds"].as_array().unwrap().is_empty(), "{out}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn validate_rejects_over_limit_and_missing_file() {
        let dir = std::env::temp_dir().join("icepaw_validate_docx_test2");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("样本.docx");
        std::fs::write(&file, three_para_docx()).unwrap();

        let tool = ValidateDocxTool;
        // 断言数超限 = 参数错 → Err（区别于断言失败）
        let assertions: Vec<_> = (0..51)
            .map(|i| serde_json::json!({ "kind": "block_count", "equals": i }))
            .collect();
        let args =
            serde_json::json!({ "path": file.to_string_lossy(), "assertions": assertions })
                .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("断言数超限"), "实际: {err}");

        // 文件不存在：did-you-mean 契约同 inspect/edit
        let args = serde_json::json!({
            "path": "Z:/不存在的/文档.docx",
            "assertions": [{ "kind": "block_count", "equals": 1 }]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("文件不存在"), "实际: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- allowed_blocks 区间锁（D15 八波②）----

    #[tokio::test]
    async fn range_lock_allows_in_range_and_rejects_out_of_range() {
        let dir = std::env::temp_dir().join("icepaw_edit_docx_lock_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("样本.docx");
        std::fs::write(&file, three_para_docx()).unwrap();

        let tool = EditDocxTool;
        // 区间内：lock [1,2] 改块 2 → 通过
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "allowed_blocks": [1, 2],
            "operations": [
                { "op": "replace_text", "block": 2, "expect_prefix": "第二段", "new_text": "改写段" }
            ]
        })
        .to_string();
        let out = tool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["applied"], 1, "{out}");

        // 区间外：lock [1,2] 动块 3 → 整批拒（家族前缀），文件字节原样
        let before = std::fs::read(&file).unwrap();
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "allowed_blocks": [1, 2],
            "operations": [
                { "op": "replace_text", "block": 3, "expect_prefix": "第三段", "new_text": "越界段" }
            ]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("区间外块"), "实际: {err}");
        assert!(err.contains("1..=2"), "区间要在文案里: {err}");
        assert_eq!(std::fs::read(&file).unwrap(), before, "拒批文件逐字节 untouched");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn range_lock_rejects_clear_body_and_bad_bounds() {
        let dir = std::env::temp_dir().join("icepaw_edit_docx_lock_test2");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("样本.docx");
        std::fs::write(&file, three_para_docx()).unwrap();
        let before = std::fs::read(&file).unwrap();

        let tool = EditDocxTool;
        // clear_body 清空全文，与任何区间语义冲突 → 拒
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "allowed_blocks": [1, 3],
            "operations": [ { "op": "clear_body", "expect_blocks": 3 } ]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("区间外块"), "实际: {err}");
        assert!(err.contains("clear_body"), "实际: {err}");
        assert_eq!(std::fs::read(&file).unwrap(), before);

        // 非法区间 lo>hi → 工具层拒
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "allowed_blocks": [3, 2],
            "operations": [ { "op": "delete_block", "block": 2, "expect_prefix": "第二段" } ]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("区间锁无效"), "实际: {err}");

        // lo=0（块号 1-based）→ 拒
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "allowed_blocks": [0, 2],
            "operations": [ { "op": "delete_block", "block": 2, "expect_prefix": "第二段" } ]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("区间锁无效"), "实际: {err}");

        // 样式族无块号概念，与锁互斥 → 拒
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "allowed_blocks": [1, 3],
            "operations": [ { "op": "create_style", "style_type": "paragraph", "name": "某样式" } ]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("区间锁无效"), "实际: {err}");
        assert_eq!(std::fs::read(&file).unwrap(), before);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- insert_table_after rows_text（D15 八波③）----

    /// 表格工具测试用锚段包。
    fn anchored_docx() -> Vec<u8> {
        use docx_rs::{Docx, Document, Paragraph, Run};
        let document =
            Document::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text("锚段")));
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        Docx::new().document(document).build().pack(&mut cursor).unwrap();
        cursor.into_inner()
    }

    #[tokio::test]
    async fn rows_text_parses_markdown_and_matches_rows_form() {
        let dir = std::env::temp_dir().join("icepaw_rows_text_test");
        std::fs::create_dir_all(&dir).unwrap();
        let tool = EditDocxTool;

        // 同一表格两种形态：rows_text（Markdown 含分隔行 + \| 转义）vs rows 矩阵。
        // 模板字节取一次写两份（docx-rs 可能嵌时间戳，两次 build 字节不保证相同）
        let template = anchored_docx();
        let md = "| 列1 | 列2 |\n| --- | :---: |\n| 甲 | a\\|b |\n\n";
        let cases: [(&str, serde_json::Value); 2] = [
            ("rows_text", serde_json::json!(md)),
            ("rows", serde_json::json!([["列1", "列2"], ["甲", "a|b"]])),
        ];
        let mut summaries = Vec::new();
        let mut file_bytes = Vec::new();
        for (form, data) in cases {
            let file = dir.join(format!("{form}.docx"));
            std::fs::write(&file, &template).unwrap();
            let mut op = serde_json::json!({
                "op": "insert_table_after", "block": 1, "expect_prefix": "锚段"
            });
            op[form] = data;
            let args =
                serde_json::json!({ "path": file.to_string_lossy(), "operations": [op] })
                    .to_string();
            let out = tool.execute(&args).await.unwrap();
            let v: serde_json::Value = serde_json::from_str(&out).unwrap();
            assert_eq!(v["applied"], 1, "{form}: {out}");
            summaries.push(v["operations"].to_string());
            file_bytes.push(std::fs::read(&file).unwrap());
        }
        // 值优先与结构化等价：applied 摘要相等 + 产物字节逐字节相等
        assert_eq!(summaries[0], summaries[1], "两种形态 applied 摘要须一致");
        assert_eq!(file_bytes[0], file_bytes[1], "两种形态产物须一致");

        // 读回：表格内容正确落格（含 \| 还原的字面竖线）
        let inspect = crate::harness::doc::inspect_document(
            &file_bytes[0],
            &InspectRequest { projection: InspectProjection::Table, start: None, end: None, row: None, cell: None, style: None, num_id: None, level: None },
        )
        .unwrap();
        assert!(inspect.content.contains("a|b"), "{}", inspect.content);
        assert!(inspect.content.contains("列1"), "{}", inspect.content);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn rows_text_tsv_and_error_paths() {
        let dir = std::env::temp_dir().join("icepaw_rows_text_test2");
        std::fs::create_dir_all(&dir).unwrap();
        let tool = EditDocxTool;
        let file = dir.join("t.docx");
        std::fs::write(&file, anchored_docx()).unwrap();

        // TSV：制表符分列
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "operations": [ {
                "op": "insert_table_after", "block": 1, "expect_prefix": "锚段",
                "rows_text": "甲\t乙\n丙\t丁"
            } ]
        })
        .to_string();
        let out = tool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["applied"], 1, "{out}");

        // rows 与 rows_text 同时给 → 互斥拒
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "operations": [ {
                "op": "insert_table_after", "block": 1, "expect_prefix": "锚段",
                "rows": [["x"]], "rows_text": "| x |"
            } ]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("表格文本无效"), "实际: {err}");

        // 都不给 → 拒（必填二选一）
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "operations": [ { "op": "insert_table_after", "block": 1, "expect_prefix": "锚段" } ]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("表格文本无效"), "实际: {err}");

        // 既无 | 也无制表符 → 无法切列
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "operations": [ {
                "op": "insert_table_after", "block": 1, "expect_prefix": "锚段",
                "rows_text": "就一行没有分隔符"
            } ]
        })
        .to_string();
        let err = tool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("无法切列"), "实际: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// parse_rows_text 边缘（同步直调）：首尾包裹竖线剥离 / 空行忽略 / 空文本拒。
    #[test]
    fn parse_rows_text_edges() {
        let rows = parse_rows_text("| a | b |\n| c | d |").unwrap();
        assert_eq!(rows, [["a", "b"], ["c", "d"]]);

        // 无包裹竖线的 Markdown 行同样合法；空行忽略
        let rows = parse_rows_text("a | b\n\nc | d").unwrap();
        assert_eq!(rows, [["a", "b"], ["c", "d"]]);

        // 只有分隔行 → 空，拒
        let err = parse_rows_text("| --- | --- |").unwrap_err().to_string();
        assert!(err.contains("解析后为空"), "实际: {err}");
        let err = parse_rows_text("").unwrap_err().to_string();
        assert!(err.contains("解析后为空"), "实际: {err}");
    }

    // =========================================================================
    // write_docx 工具壳（D16 九波）：三层模板 / 备份覆盖 / 参数守卫 / 读回
    // =========================================================================

    /// 造带 workspace 的 ToolContext（write_docx 的 L2 相对模板路径需要它）。
    async fn write_ctx(workspace: Option<String>) -> ToolContext {
        ToolContext {
            conv_id: "c1".into(),
            agent_id: "a1".into(),
            project_id: None,
            workspace,
            pool: sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap(),
            api_key: None,
            app_handle: None,
            proposal_registry: None,
            turn_id: None,
            cancel: None,
        }
    }

    /// text 投影读回产物（带块号正文，验内容与块序）。
    fn read_back_text(bytes: &[u8]) -> String {
        crate::harness::doc::inspect_document(
            bytes,
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
        .unwrap()
        .content
    }

    #[tokio::test]
    async fn write_docx_builtin_creates_ordered_document() {
        let dir = std::env::temp_dir().join("icepaw_write_docx_test");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("报告.docx");

        // L1 缺省模板（= report）+ 混合块序：heading / paragraph / table(rows_text)
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "blocks": [
                { "type": "heading", "level": 1, "text": "季度报告" },
                { "type": "paragraph", "text": "本季度进展顺利。" },
                { "type": "table", "rows_text": "| 指标 | 数值 |\n| --- | --- |\n| 交付 | 3 |" },
                { "type": "heading", "level": 2, "text": "下季度计划" }
            ]
        })
        .to_string();
        let ctx = write_ctx(None).await;
        let out = WriteDocxTool
            .execute_with_context(&args, &ctx)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["created"], true, "{out}");
        assert_eq!(v["template"], "report", "{out}");
        assert_eq!(v["paragraphs"], 3, "{out}"); // 两 heading + 一段
        assert_eq!(v["tables"], 1, "{out}");
        assert_eq!(v["check"], "passed", "{out}");

        // 读回：四类内容全在且块序正确（text 投影带块号顺序输出）
        let text = read_back_text(&std::fs::read(&file).unwrap());
        let pos = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("缺 {needle}: {text}"));
        assert!(pos("季度报告") < pos("本季度进展顺利。"));
        assert!(pos("本季度进展顺利。") < pos("交付"));
        assert!(pos("交付") < pos("下季度计划"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn write_docx_template_three_layers() {
        let dir = std::env::temp_dir().join("icepaw_write_docx_test2");
        std::fs::remove_dir_all(&dir).ok();
        let ws = dir.join("ws");
        std::fs::create_dir_all(ws.join("templates")).unwrap();
        // 「家模板」= 内置模板字节落盘占位（真模板用户后续用 Word 造）
        let house = ws.join("templates").join("memo.docx");
        std::fs::write(&house, crate::harness::doc::build_builtin_template("report").unwrap())
            .unwrap();

        let blocks = r#"[{"type":"paragraph","text":"内容"}]"#.to_string();
        let ctx_ws = write_ctx(Some(ws.to_string_lossy().to_string())).await;

        // L2 相对路径：workspace/templates/ 下命中
        let file = dir.join("l2.docx");
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "template": "memo.docx",
            "blocks": serde_json::from_str::<serde_json::Value>(&blocks).unwrap()
        })
        .to_string();
        let out = WriteDocxTool
            .execute_with_context(&args, &ctx_ws)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["template"], "memo.docx", "{out}");
        assert!(file.exists(), "目标应已生成");

        // L3 绝对路径：任意 docx
        let file = dir.join("l3.docx");
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "template": house.to_string_lossy(),
            "blocks": serde_json::from_str::<serde_json::Value>(&blocks).unwrap()
        })
        .to_string();
        let out = WriteDocxTool
            .execute_with_context(&args, &ctx_ws)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["created"], true, "{out}");

        // 相对路径但无 workspace → 拒 + 三层规则文案（含档位名）
        let ctx_none = write_ctx(None).await;
        let args = serde_json::json!({
            "path": dir.join("no.docx").to_string_lossy(),
            "template": "memo.docx",
            "blocks": serde_json::from_str::<serde_json::Value>(&blocks).unwrap()
        })
        .to_string();
        let err = WriteDocxTool
            .execute_with_context(&args, &ctx_none)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("模板无效"), "实际: {err}");
        assert!(err.contains("report"), "应列内置档位名: {err}");

        // workspace 有但模板文件缺失 → not-found + did-you-mean
        let args = serde_json::json!({
            "path": dir.join("no.docx").to_string_lossy(),
            "template": "memoo.docx",
            "blocks": serde_json::from_str::<serde_json::Value>(&blocks).unwrap()
        })
        .to_string();
        let err = WriteDocxTool
            .execute_with_context(&args, &ctx_ws)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("模板无效"), "实际: {err}");
        assert!(err.contains("不存在"), "实际: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    // =========================================================================
    // 模板四层解析（D17 共享模板目录）——顺序 / 内置兜底 / 可用清单提示
    // =========================================================================

    #[tokio::test]
    async fn resolve_template_shared_dir_fallback_and_order() {
        let dir = std::env::temp_dir().join("icepaw_resolve_tpl_test");
        std::fs::remove_dir_all(&dir).ok();
        let ws = dir.join("ws");
        let shared = dir.join("shared");
        std::fs::create_dir_all(ws.join("templates")).unwrap();
        std::fs::create_dir_all(&shared).unwrap();
        let (seed_name, seed_bytes) = crate::harness::doc::shared_templates::SHARED_TEMPLATE_SEEDS[0];

        // 共享目录命中：无 workspace 也能用（共享目录不依赖 workspace）
        std::fs::write(shared.join(seed_name), seed_bytes).unwrap();
        let bytes = resolve_template(seed_name, None, Some(&shared)).await.unwrap();
        assert_eq!(bytes, seed_bytes.to_vec());

        // workspace 优先于共享目录（更具体者赢）：同名时取 workspace 字节。
        // 合成 docx 只取一次样——docx_rs 每次打包带时间戳，两次调用字节必异。
        let synthetic = docx_bytes();
        std::fs::write(ws.join("templates").join(seed_name), &synthetic).unwrap();
        let bytes = resolve_template(seed_name, Some(ws.to_str().unwrap()), Some(&shared))
            .await
            .unwrap();
        assert_eq!(bytes, synthetic);

        // 内置档位兜底：两处目录都无该文件时回落 BUILTIN_TEMPLATES
        let bytes = resolve_template("report", Some(ws.to_str().unwrap()), Some(&shared))
            .await
            .unwrap();
        assert!(!bytes.is_empty(), "内置档位应兜底");

        // 同名文件可覆盖内置档位：workspace 放 report.docx → 取文件
        std::fs::write(ws.join("templates").join("report.docx"), &synthetic).unwrap();
        let bytes = resolve_template("report", Some(ws.to_str().unwrap()), Some(&shared))
            .await
            .unwrap();
        assert_eq!(bytes, synthetic);

        // miss 报错即行为契约：列两处目录清单 + 内置档位
        let err = resolve_template("zzz.docx", Some(ws.to_str().unwrap()), Some(&shared))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("模板无效"), "实际: {err}");
        assert!(err.contains("共享模板目录"), "应列共享目录: {err}");
        assert!(err.contains(seed_name), "应列共享目录现有模板: {err}");
        assert!(err.contains("report"), "应列内置档位: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn write_docx_overwrite_existing_backs_up() {
        let dir = std::env::temp_dir().join("icepaw_write_docx_test3");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("old.docx");
        std::fs::write(&file, docx_bytes()).unwrap(); // 旧内容：单段「正文段」

        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "blocks": [ { "type": "heading", "level": 1, "text": "新标题" } ]
        })
        .to_string();
        let ctx = write_ctx(None).await;
        let out = WriteDocxTool
            .execute_with_context(&args, &ctx)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["created"], false, "{out}");
        let backup = v["backup"].as_str().unwrap().to_string();
        assert!(!backup.is_empty() && Path::new(&backup).exists(), "{out}");

        // 覆盖后 = 新文档（新标题在、旧正文不在）；备份里是旧内容
        let text = read_back_text(&std::fs::read(&file).unwrap());
        assert!(text.contains("新标题"), "{text}");
        assert!(!text.contains("正文段"), "旧内容应已被替换: {text}");
        let backup_text = read_back_text(&std::fs::read(&backup).unwrap());
        assert!(backup_text.contains("正文段"), "备份应是旧文件: {backup_text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn write_docx_rejects_bad_input() {
        let dir = std::env::temp_dir().join("icepaw_write_docx_test4");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = write_ctx(None).await;

        // blocks 空 → 生成块无效
        let args = serde_json::json!({
            "path": dir.join("a.docx").to_string_lossy(),
            "blocks": []
        })
        .to_string();
        let err = WriteDocxTool
            .execute_with_context(&args, &ctx)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("生成块无效"), "实际: {err}");

        // blocks 超上限（MAX_WRITE_BLOCKS=300）→ 生成块无效
        let overflow: Vec<serde_json::Value> = (0..=300)
            .map(|i| serde_json::json!({ "type": "paragraph", "text": format!("段{i}") }))
            .collect();
        let args = serde_json::json!({
            "path": dir.join("b.docx").to_string_lossy(),
            "blocks": overflow
        })
        .to_string();
        let err = WriteDocxTool
            .execute_with_context(&args, &ctx)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("超过上限"), "实际: {err}");

        // 目标非 .docx → 指路 write_file
        let args = serde_json::json!({
            "path": dir.join("c.txt").to_string_lossy(),
            "blocks": [ { "type": "paragraph", "text": "x" } ]
        })
        .to_string();
        let err = WriteDocxTool
            .execute_with_context(&args, &ctx)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("write_file"), "实际: {err}");

        // 未知块型（serde tag 拒）→ 参数解析失败
        let args = serde_json::json!({
            "path": dir.join("d.docx").to_string_lossy(),
            "blocks": [ { "type": "video", "text": "x" } ]
        })
        .to_string();
        let err = WriteDocxTool
            .execute_with_context(&args, &ctx)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("参数解析失败"), "实际: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    // =========================================================================
    // toc/image 块 + insert 两操作（D18 十波③ 工具层）
    // =========================================================================

    /// 造一张真实 PNG（image crate 编码，10×6 纯色）——装载层格式/尺寸探测实弹。
    fn tiny_png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(10, 6, image::Rgba([180, 60, 60, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    /// 读 zip 内某文本部件（缺失返 None）。
    fn zip_part(bytes: &[u8], name: &str) -> Option<String> {
        use std::io::Read;
        let mut a = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut f = match a.by_name(name) {
            Ok(f) => f,
            Err(_) => return None,
        };
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        Some(s)
    }

    /// zip 内部件存在性（二进制件如 media 只探不读）。
    fn zip_has(bytes: &[u8], name: &str) -> bool {
        zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .unwrap()
            .by_name(name)
            .is_ok()
    }

    #[tokio::test]
    async fn write_docx_toc_and_image_blocks() {
        let dir = std::env::temp_dir().join("icepaw_write_docx_tocimg");
        std::fs::remove_dir_all(&dir).ok();
        let ws = dir.join("ws");
        std::fs::create_dir_all(ws.join("figs")).unwrap();
        let png = tiny_png();
        std::fs::write(ws.join("figs").join("图1.png"), &png).unwrap();
        let file = dir.join("报告.docx");

        // image 相对路径挂 workspace；toc 缺省 levels=3/hyperlink=true
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "blocks": [
                { "type": "heading", "level": 1, "text": "系统报告" },
                { "type": "toc", "levels": 3 },
                { "type": "image", "path": "figs/图1.png", "width_mm": 120 },
                { "type": "paragraph", "text": "部署完成。" }
            ]
        })
        .to_string();
        let ctx = write_ctx(Some(ws.to_string_lossy().to_string())).await;
        let out = WriteDocxTool
            .execute_with_context(&args, &ctx)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["images"], 1, "{out}");
        assert_eq!(v["tocs"], 1, "{out}");
        assert_eq!(v["check"], "passed", "{out}");

        // 产物：media 字节全等 + settings updateFields（内置 report 无 settings →
        // 新建路径）+ 读回投影域/图片标记可见（治「空段」盲区）
        let bytes = std::fs::read(&file).unwrap();
        let media = {
            use std::io::Read;
            let mut a = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
            let mut m = Vec::new();
            a.by_name("word/media/image1.png").unwrap().read_to_end(&mut m).unwrap();
            m
        };
        assert_eq!(media, png, "media 部件应与源文件字节全等");
        assert!(
            zip_part(&bytes, "word/settings.xml")
                .unwrap()
                .contains("updateFields"),
            "settings 应自动置 updateFields"
        );
        let text = read_back_text(&bytes);
        assert!(text.contains("[域:TOC]"), "{text}");
        assert!(text.contains("[图片×1]"), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn edit_docx_insert_image_and_toc() {
        let dir = std::env::temp_dir().join("icepaw_edit_tocimg");
        std::fs::remove_dir_all(&dir).ok();
        let ws = dir.join("ws");
        std::fs::create_dir_all(ws.join("figs")).unwrap();
        let png = tiny_png();
        std::fs::write(ws.join("figs").join("fig.png"), &png).unwrap();
        let file = dir.join("t.docx");
        std::fs::write(&file, anchored_docx()).unwrap();

        // 同批图 + TOC（doc 族混排合法）；image 相对路径挂 ctx.workspace
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "operations": [
                { "op": "insert_image_after", "block": 1, "expect_prefix": "锚段",
                  "image_path": "figs/fig.png" },
                { "op": "insert_toc_after", "block": 1, "expect_prefix": "锚段", "levels": 2 }
            ]
        })
        .to_string();
        let ctx = write_ctx(Some(ws.to_string_lossy().to_string())).await;
        let out = EditDocxTool
            .execute_with_context(&args, &ctx)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["applied"], 2, "{out}");

        let bytes = std::fs::read(&file).unwrap();
        assert!(zip_has(&bytes, "word/media/image1.png"), "media 应入包");
        assert!(
            zip_part(&bytes, "word/settings.xml")
                .unwrap()
                .contains("updateFields")
        );
        let text = read_back_text(&bytes);
        assert!(text.contains("[域:TOC]"), "{text}");
        assert!(text.contains("[图片×1]"), "{text}");

        // 直调 execute（无 workspace）+ 相对路径 → 图片无效 + 指路绝对路径
        let file2 = dir.join("t2.docx");
        std::fs::write(&file2, anchored_docx()).unwrap();
        let args = serde_json::json!({
            "path": file2.to_string_lossy(),
            "operations": [ { "op": "insert_image_after", "block": 1,
                              "expect_prefix": "锚段", "image_path": "figs/fig.png" } ]
        })
        .to_string();
        let err = EditDocxTool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("图片无效"), "实际: {err}");
        assert!(err.contains("绝对路径"), "应指路绝对路径: {err}");

        // schema 披露抽查：两操作进 edit schema、两块型进 write schema
        let p = EditDocxTool.parameters().to_string();
        assert!(p.contains("insert_image_after"), "edit schema 缺插图操作");
        assert!(p.contains("insert_toc_after"), "edit schema 缺 TOC 操作");
        let p = WriteDocxTool.parameters().to_string();
        assert!(p.contains(r#""const":"toc""#), "write schema 缺 toc 块");
        assert!(p.contains(r#""const":"image""#), "write schema 缺 image 块");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn validate_docx_image_and_field_assertions() {
        // write_docx 全链造带域带图的文档（heading=1 toc=2 image=3）
        let dir = std::env::temp_dir().join("icepaw_validate_tocimg");
        std::fs::remove_dir_all(&dir).ok();
        let ws = dir.join("ws");
        std::fs::create_dir_all(ws.join("figs")).unwrap();
        std::fs::write(ws.join("figs").join("f.png"), tiny_png()).unwrap();
        let file = dir.join("v.docx");
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "blocks": [
                { "type": "heading", "level": 1, "text": "标题" },
                { "type": "toc" },
                { "type": "image", "path": "figs/f.png" }
            ]
        })
        .to_string();
        let ctx = write_ctx(Some(ws.to_string_lossy().to_string())).await;
        WriteDocxTool
            .execute_with_context(&args, &ctx)
            .await
            .unwrap();

        let tool = ValidateDocxTool;
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "assertions": [
                { "kind": "block_image", "block": 3, "equals": 1 },
                { "kind": "block_field", "block": 2, "instr_contains": "TOC" },
                { "kind": "block_field", "block": 3, "instr_contains": "TOC" },
                { "kind": "block_image", "block": 1 },
                { "kind": "block_image", "block": 99 }
            ]
        })
        .to_string();
        let out = tool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        // 失败 = 正常数据：块 3 是图非域、块 1 标题段无图、块 99 越界
        assert_eq!(v["passed"], false, "{out}");
        assert_eq!(v["total"], 5);
        assert_eq!(v["failed"], 3, "{out}");
        let kinds: Vec<&str> = v["failures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["block_field", "block_image", "block_image"], "{out}");

        // 全过形态：存在性（省 equals）+ 子串两 kind 进摘要
        let args = serde_json::json!({
            "path": file.to_string_lossy(),
            "assertions": [
                { "kind": "block_image", "block": 3 },
                { "kind": "block_field", "block": 2, "instr_contains": "TOC" }
            ]
        })
        .to_string();
        let out = tool.execute(&args).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["passed"], true, "{out}");
        let kinds: Vec<&str> = v["passed_kinds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        let kinds = kinds.join(",");
        assert!(kinds.contains("block_image") && kinds.contains("block_field"), "{out}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
