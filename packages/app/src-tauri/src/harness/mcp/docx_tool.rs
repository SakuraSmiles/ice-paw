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
use crate::harness::doc::{
    apply_edits_to_bytes, apply_numbering_edits_to_bytes, apply_style_edits_to_bytes,
    inspect_document, EditOp, InspectProjection, InspectRequest, NumberingEditOp, StyleContainer,
    StyleEditOp, StyleType,
};

use super::client::McpClient;
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
         it, send it back; also the verification view after that op). \
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
                    "description": "Row number r (1-based, projection=tblpr only): drill down to that row's trPr. Must be used with the block range selecting one table block."
                },
                "cell": {
                    "type": "integer",
                    "description": "Cell number c (1-based, projection=tblpr only, requires row): drill down to that cell's tcPr."
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
// edit_docx —— 块级手术工具（步骤 3，D3 批量事务）
// =========================================================================

pub struct EditDocxTool;

#[derive(Deserialize)]
struct EditDocxArgs {
    path: String,
    /// 操作批（全有或全无；段级操作每块限一个，表格操作（set_cell_text /
    /// insert_table_row_after / set_cell_format / set_table_element）可同块多条
    /// 按序组合，(行, 格) 去重；merge_cells / split_cell 须独占该表）。
    /// 正文操作与定义操作（create_style / set_style_element /
    /// set_numbering_element / clear_body）不可混批（部件互斥），拆两批先后发
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
    /// 锚块后建新表：rows 矩形矩阵（首行默认表头——加粗 + 跨页重复）；列宽均分。
    /// table_style（D12 模板件）：表样式显示名或 ID——建表即挂模板样式（底纹/
    /// 边框随样式走）；缺省默认全边框直排
    InsertTableAfter {
        block: usize,
        expect_prefix: String,
        rows: Vec<Vec<String>>,
        #[serde(default)]
        header: Option<bool>,
        #[serde(default)]
        table_style: Option<String>,
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
    /// 改格内文字格式：set_format 的格级版（段落格式作用于格内全部段落，字符格式
    /// 作用于格内全部 run）；(row, cell) 双 1-based
    SetCellFormat {
        block: usize,
        expect_prefix: String,
        row: usize,
        cell: usize,
        #[serde(default)]
        paragraph: Option<ParaFormatSpec>,
        #[serde(default)]
        character: Option<CharFormatSpec>,
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
    /// 合并单元格（Word 原生语义）：horizontal=同行 span 格并 1 格（gridSpan 求和、
    /// 内容按序拼接）；vertical=同列 span 行纵并（vMerge 头 restart 续 continue、
    /// 内容留原格，split_cell 即恢复）。矩形区=direction/span 不传、改传
    /// end_row+end_cell（区域右下角），(row,cell) 至 (end_row,end_cell) 一次合并。
    /// 结构重构须独占该表
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

impl From<OperationSpec> for FamilyOp {
    fn from(spec: OperationSpec) -> Self {
        match spec {
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
            OperationSpec::InsertTableAfter { block, expect_prefix, rows, header, table_style } => {
                FamilyOp::Doc(EditOp::InsertTableAfter { block, expect_prefix, rows, header, table_style })
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
            } => FamilyOp::Doc(EditOp::SetCellFormat {
                block,
                expect_prefix,
                row,
                cell,
                paragraph: paragraph.map(Into::into),
                character: character.map(Into::into),
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
         order within one batch. Table formatting ops: op=set_cell_format changes \
         paragraph and/or character formatting of one cell (paragraph formatting hits \
         every paragraph in the cell, character formatting every run — same param shape \
         as set_format); op=set_table_element is the generic escape hatch for any \
         table-property element (borders tblBorders, shading shd, width tblW, cell \
         margins tblCellMar, row height trHeight, vertical alignment vAlign, ...) at \
         three container levels: level=table (tblPr, no row/cell), level=row (trPr, \
         with row), level=cell (tcPr, with row+cell) — xml=null removes the element, \
         xml=<w:...> fragment replaces/inserts it at its schema position (copy the \
         current XML from inspect_docx projection=tblpr, never write from memory; \
         gridSpan/hMerge/vMerge are protected — use merge_cells / split_cell instead). \
         Structural ops: op=merge_cells merges cells with Word-native semantics — \
         horizontal merges span adjacent cells in one row (gridSpan sums, content \
         concatenates into the first cell); vertical merges cells across rows at the \
         same grid columns (vMerge head restarts, content stays in place — split_cell \
         restores it). op=split_cell is the inverse — vertical splits the whole merge \
         chain, horizontal splits a spanning cell back into unit cells (content stays \
         in the first). merge_cells / split_cell renumber row/cell addresses, so each \
         must be the only op on its table block in a batch (finish structure first, \
         then re-inspect projection=table to address content). \
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
         set_table_element level=row element=shd on alternate rows; whole-document \
         typography = set_style_element on heading/body styles instead of per-paragraph \
         set_format. New document from a house template: check the workspace templates/ \
         directory first (list_directory), copy_file to the target, edit_docx \
         op=clear_body, then write content — the template's styles/numbering/headers \
         are preserved verbatim. Every \
         operation must carry expect_prefix, the current text prefix of its target block, \
         as a fingerprint guard — if any block no longer matches, the whole batch is \
         rejected and the file is left untouched. Blocks are addressed by inspect_docx \
         block numbers (1-based, paragraphs and tables in document order); text ops \
         (replace/delete/set_style/set_format/set_ppr_element) reject table and \
         revision-marked blocks. The file is backed up \
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
                                    "header": { "type": "boolean", "description": "true (default): first row is bold and repeats across pages; false: plain data rows only." },
                                    "table_style": { "type": "string", "description": "Table style (display name or ID, @w:type=table — see projection=styles): attach the house template's table style so shading/borders follow it. Default: plain single-border table." }
                                },
                                "required": ["op", "block", "expect_prefix", "rows"],
                                "description": "Create a new table after the anchor block. Default styling: 100% width, single borders, evenly split columns, bold repeating header row. Pass table_style to bind a named table style instead."
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
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "set_cell_format" },
                                    "block": { "type": "integer", "description": "Target table block (1-based)." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the table block (fingerprint guard)." },
                                    "row": { "type": "integer", "description": "Row number r (1-based) — same as the rN lines of inspect_docx projection=table." },
                                    "cell": { "type": "integer", "description": "Cell number c within the row (1-based; a merged/spanning cell counts as one)." },
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
                                "description": "Change one cell's text formatting (cell-level version of set_format). At least one field inside paragraph or character must be set."
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
                                    "block": { "type": "integer", "description": "Target table block (1-based). Must be the only op on this table in the batch." },
                                    "expect_prefix": { "type": "string", "description": "Current text prefix of the table block (fingerprint guard)." },
                                    "direction": { "type": "string", "enum": ["horizontal", "vertical"], "description": "Simple line merge; omit when using end_row+end_cell (rectangle region)." },
                                    "row": { "type": "integer", "description": "Row of the head (top-left) cell of the merged region (1-based)." },
                                    "cell": { "type": "integer", "description": "Head cell number within the row (1-based)." },
                                    "span": { "type": "integer", "minimum": 2, "description": "How many cells (horizontal) or rows (vertical) to merge; default 2. Simple mode only." },
                                    "end_row": { "type": "integer", "description": "Rectangle mode (with end_cell, no direction/span): bottom row of the region (1-based)." },
                                    "end_cell": { "type": "integer", "description": "Rectangle mode: rightmost cell of the bottom row (1-based)." }
                                },
                                "required": ["op", "block", "expect_prefix", "row", "cell"],
                                "description": "Merge cells with Word-native semantics. Horizontal: merges span adjacent cells in one row — their texts concatenate into the head cell. Vertical: merges cells across rows at the same grid columns — content stays in each cell, split_cell later restores independent display. Rectangle mode: omit direction/span, give end_row+end_cell — the whole (row,cell)..(end_row,end_cell) region merges in one op (row-wise horizontal merges, then the resulting column merges vertically)."
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "op": { "const": "split_cell" },
                                    "block": { "type": "integer", "description": "Target table block (1-based). Must be the only op on this table in the batch." },
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

        // 分族路由：正文块手术 / 样式定义手术 / 编号定义手术，一批只落一个部件
        if parsed.operations.is_empty() {
            return Err(AppError::Validation(
                "操作列表为空: operations 至少需要一个操作。".into(),
            ));
        }
        let mut doc_ops: Vec<EditOp> = Vec::new();
        let mut style_ops: Vec<StyleEditOp> = Vec::new();
        let mut numbering_ops: Vec<NumberingEditOp> = Vec::new();
        for spec in parsed.operations {
            match FamilyOp::from(spec) {
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

        // 全有或全无：手术在内存完成（含整批预检 + 产物再解析校验），通过才落盘
        let (new_bytes, applied) = if !doc_ops.is_empty() {
            apply_edits_to_bytes(&bytes, &doc_ops)?
        } else if !style_ops.is_empty() {
            apply_style_edits_to_bytes(&bytes, &style_ops)?
        } else {
            apply_numbering_edits_to_bytes(&bytes, &numbering_ops)?
        };

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
}
