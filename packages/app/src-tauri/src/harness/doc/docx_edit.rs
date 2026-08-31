//! `docx_edit` —— 块级外科手术引擎（word-capability-roadmap 步骤 3）。
//!
//! 三层结构，全部纯函数（bytes/xml 进、新 xml/bytes 出）：
//! 1. **块定位器** [`locate_blocks`]：quick-xml 流式扫描 document.xml，产出每个
//!    编址块的**源码字节范围**（`[start, end)`）。遍历语义与 `docx_model::walk_blocks`
//!    严格一致（w:p/w:tbl 成块即止、sectPr/书签等跳过、未知容器透明递归摊平）——
//!    inspect_docx 的块号在这里一一对应到源码区间。对齐由测试锁定（语料逐块）。
//! 2. **手术** [`apply_edits`]：验证后变异——全批预检（块号/指纹/修订/节属性/
//!    重复块）通过才动刀；每个操作只替换目标块的字节区间，区间外**一个字节不动**
//!    （pPr/rPr 原样切片保留，「不敢懂的不碰」）。操作按位置降序 splice，原始
//!    偏移全程有效。
//! 3. **容器重打包** [`repack_part`]：只替换目标 entry（document.xml 或
//!    styles.xml / numbering.xml 定义部件），其余 zip
//!    entry 用 `raw_copy_file` **原样字节复制**（不解压重压，压缩参数/元数据不变）。
//!
//! MVP 操作词表（批量事务，D3 拍板 2026-08-24）：`replace_text` /
//! `insert_paragraph_after` / `delete_block` / `set_style` / `set_format`。
//! 表格内容操作后续批。

use std::collections::HashMap;
use std::io::{Cursor, Write};

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Serialize;

use crate::error::{AppError, AppResult};

use super::docx_model::{self, Block};
use super::styles::Stylesheet;

// =========================================================================
// 块定位器
// =========================================================================

/// 一个编址块在 document.xml 源码中的字节范围（`[start, end)`，UTF-8 字节偏移）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BlockSpan {
    pub start: usize,
    pub end: usize,
    /// w:tbl 块（true）；w:段落（false）
    pub is_table: bool,
}

/// 栈帧角色：BodySeq = 块级序列上下文（按 walk_blocks 语义找块）；Inside = 块 /
/// 跳过名单元素的内部（不再找块，等配对 End）。
#[derive(PartialEq, Clone, Copy)]
enum FrameRole {
    BodySeq,
    Inside,
}

/// 定位全部编址块。要求根为 `w:document` 且含 `w:body`（edit 路径只面对真实
/// docx 包，不做合成宽容——宽容会引入与模型编址的不一致面）。
pub(super) fn locate_blocks(xml: &str) -> AppResult<Vec<BlockSpan>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false); // 偏移必须与原始字节对齐
    // 不展开自闭合：Empty(w:p) 自成一块，范围即标签本身

    // (角色, 元素名, 块起点（成块帧才 Some）)
    let mut stack: Vec<(FrameRole, String, Option<usize>)> = Vec::new();
    let mut spans: Vec<BlockSpan> = Vec::new();
    let mut seen_body = false;
    let mut done = false;

    loop {
        // buffer_position = 上一事件结束处 = 本事件开始处
        let ev_start = reader.buffer_position() as usize;
        let ev = match reader.read_event() {
            Ok(Event::Eof) => break, // ⚠️ 必须在 Ok(e) 之前——通配臂会吞掉 Eof 致死循环
            Ok(e) => e,
            Err(e) => {
                return Err(AppError::Internal(format!("document.xml 扫描失败: {e}")))
            }
        };
        let ev_end = reader.buffer_position() as usize;

        if done {
            continue; // body 已闭合：剩余事件（如根闭合）无需处理
        }

        match ev {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                if !seen_body {
                    if name == "w:body" {
                        seen_body = true;
                        stack.push((FrameRole::BodySeq, name, None));
                    } else {
                        // w:document 根及其他前导元素内部：等 body
                        stack.push((FrameRole::Inside, name, None));
                    }
                    continue;
                }
                let in_body_seq = matches!(stack.last(), Some((FrameRole::BodySeq, _, _)));
                match name.as_str() {
                    "w:p" | "w:tbl" if in_body_seq => {
                        // 成块：起点 = 本 Start 标签首字节；内部整体跳过
                        stack.push((FrameRole::Inside, name.clone(), Some(ev_start)));
                    }
                    // 跳过名单（与 walk_blocks 一致）：sectPr / 书签 / 拼写标记
                    "w:sectPr" | "w:bookmarkStart" | "w:bookmarkEnd" | "w:proofErr"
                        if in_body_seq =>
                    {
                        stack.push((FrameRole::Inside, name, None));
                    }
                    _ if in_body_seq => {
                        // 未知容器（w:sdt 等）：透明递归，内部继续找块
                        stack.push((FrameRole::BodySeq, name, None));
                    }
                    _ => stack.push((FrameRole::Inside, name, None)),
                }
            }
            Event::Empty(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                let in_body_seq = matches!(stack.last(), Some((FrameRole::BodySeq, _, _)));
                if in_body_seq && (name == "w:p" || name == "w:tbl") {
                    // 自闭合块：范围即标签本身
                    spans.push(BlockSpan {
                        start: ev_start,
                        end: ev_end,
                        is_table: name == "w:tbl",
                    });
                }
                // 名单元素 / 其他自闭合：无块级语义
            }
            Event::End(_) => {
                if let Some((_, name, block_start)) = stack.pop() {
                    if let Some(start) = block_start {
                        spans.push(BlockSpan {
                            start,
                            end: ev_end,
                            is_table: name == "w:tbl",
                        });
                    }
                    if name == "w:body" {
                        done = true;
                    }
                }
            }
            _ => {} // Text / 实体等：不影响元素级定位
        }
    }

    if !seen_body {
        return Err(AppError::Internal(
            "document.xml 缺少 w:body（edit 路径要求标准 w:document/w:body 结构）".into(),
        ));
    }
    Ok(spans)
}

// =========================================================================
// 操作词表（批量事务）
// =========================================================================

/// 段落级格式（set_format 用；None 字段不动）。单位与 format 投影显示一致——
/// agent 读到什么单位就写什么单位（诚实回环）：行距=倍数 / 段前后=pt / 缩进=twips。
#[derive(Debug, Clone, Default)]
pub struct ParaFormat {
    /// w:jc val：left/center/right/both(两端)/distribute(分散)/start/end
    pub align: Option<String>,
    /// 倍数行距（1.5 → 1.5 倍行距；w:line = 倍×240，lineRule=auto）
    pub line_spacing: Option<f32>,
    /// 段前间距（pt；twips = pt×20）
    pub space_before_pt: Option<f32>,
    /// 段后间距（pt）
    pub space_after_pt: Option<f32>,
    /// 首行缩进（twips，非负；1 字符 ≈ 240tw）
    pub indent_first_line_tw: Option<i32>,
    /// 左缩进（twips，可负=悬挂出格）
    pub indent_left_tw: Option<i32>,
}

impl ParaFormat {
    fn is_empty(&self) -> bool {
        self.align.is_none()
            && self.line_spacing.is_none()
            && self.space_before_pt.is_none()
            && self.space_after_pt.is_none()
            && self.indent_first_line_tw.is_none()
            && self.indent_left_tw.is_none()
    }
}

/// 字符级格式（set_format 用；应用到段落内**每个 run**；None 字段不动）。
#[derive(Debug, Clone, Default)]
pub struct CharFormat {
    /// 加粗（true=加粗 false=显式不加粗——压过样式链）
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    /// 字号（pt；w:sz 半磅 = pt×2，sz/szCs 同步）
    pub font_size_pt: Option<f32>,
    /// 颜色（RRGGBB 六位十六进制）
    pub color: Option<String>,
    /// 字体族（同时写 eastAsia/ascii/hAnsi——中文文档常态是同名字体）
    pub font: Option<String>,
}

impl CharFormat {
    fn is_empty(&self) -> bool {
        self.bold.is_none()
            && self.italic.is_none()
            && self.font_size_pt.is_none()
            && self.color.is_none()
            && self.font.is_none()
    }
}

/// 一个编辑操作（块号 1-based，与 inspect_docx 编址一致）。
#[derive(Debug, Clone)]
pub enum EditOp {
    /// 替换段落文本：保留 pPr 与首个 run 的 rPr（周边格式不动）；表格块拒绝。
    ReplaceText { block: usize, expect_prefix: String, new_text: String },
    /// 在锚块后插入新段：style（显示名，可选）指定样式；缺省继承锚块段落格式。
    /// 同锚块一批可多条链式（S3 七波：按输入序连续排列——写「标识行+描述行+
    /// 属性行」这类多段条目一批搞定，不再逐段拆批重寻址）。
    InsertParagraphAfter { block: usize, expect_prefix: String, text: String, style: Option<String> },
    /// 删除整块（含段落标记）；块内含 sectPr（节属性载体）拒绝。
    DeleteBlock { block: usize, expect_prefix: String },
    /// 改段落样式（标题升降级等）：只动 pStyle 一个元素，正文 run 字节不动；
    /// 表格块拒绝。style 接受显示名或样式 ID（inspect_docx outline 样式列口径）。
    SetStyle { block: usize, expect_prefix: String, style: String },
    /// 改段落/字符格式：pPr 三元素（spacing/ind/jc）属性级合并 + 每个 run 的
    /// rPr（b/i/sz/color/rFonts）；未提及的属性原样保留；表格块拒绝。
    SetFormat {
        block: usize,
        expect_prefix: String,
        paragraph: Option<ParaFormat>,
        character: Option<CharFormat>,
    },
    /// 通用段落属性元素手术（S3 二波 D9：一个操作收编 pPr 全部法定子元素长尾）：
    /// element 无前缀名（numPr/keepNext/shd/…）；xml=None 移除整元素（pPr 摘空则
    /// 整体清理），Some=合法单根片段按 CT_PPr schema 序整元素替换/插入。片段须
    /// 从 inspect_docx（projection=ppr）看到的原文复制修改，不接受凭记忆新写。
    SetPprElement {
        block: usize,
        expect_prefix: String,
        element: String,
        xml: Option<String>,
    },
    /// 在锚块后插入整表（S3 三波·表格件）：rows 矩形字符串矩阵（\n = 格内多段）；
    /// header（缺省 true）首行表头——加粗 + 跨页重复；全边框 + 列宽按节宽均分。
    /// table_style（S3 五波·模板件）：表样式显示名或 ID——建表即挂用户模板的
    /// 表样式（边框/底纹/条纹带由样式定义，tblPr 首位 tblStyle 引用）。
    InsertTableAfter {
        block: usize,
        expect_prefix: String,
        rows: Vec<Vec<String>>,
        header: Option<bool>,
        table_style: Option<String>,
    },
    /// 改表格单元格文本（S3 三波·表格件）：(row, cell) 1-based，口径与
    /// inspect_docx projection=table 所见一致（跨列格占 1 个序号）；保 tcPr +
    /// 首段 pPr + 首 run rPr；\n = 格内多段；纵向合并续格/嵌套表拒绝。
    /// 同批同表允许多个本操作按序应用（(row, cell) 不得重复）。
    SetCellText {
        block: usize,
        expect_prefix: String,
        row: usize,
        cell: usize,
        text: String,
    },
    /// 表格增行（S3 三波·表格件）：克隆模板行（after_row 缺省 = 末行）的整行
    /// 结构（tcPr/gridSpan/vMerge 原样——含合并格的表增行的唯一正确姿势），
    /// 文本换 cells（缺省全空串）。同批同表可多个，按数组序应用。
    InsertTableRowAfter {
        block: usize,
        expect_prefix: String,
        after_row: Option<usize>,
        cells: Option<Vec<String>>,
    },
    /// 改格内文字格式（S3 四波·格式件）：set_format 的格级版本——同参数面，
    /// 地址 (row, cell) 与 projection=table 同口径。段落格式作用于格内全部
    /// 段落、字符格式作用于格内全部 run。同批同表可多条，(row, cell) 去重。
    /// style（六波·缺口 1）：格内全段套段落样式（显示名或 ID）——表格格内
    /// 段落脱离正文样式（如 Normal 首行缩进透出）的正路。
    SetCellFormat {
        block: usize,
        expect_prefix: String,
        row: usize,
        cell: usize,
        paragraph: Option<ParaFormat>,
        character: Option<CharFormat>,
        style: Option<String>,
    },
    /// 通用表格属性元素手术（S3 四波·格式件，set_ppr_element 的表格域镜像）：
    /// level=table/row/cell 三层，element 为对应容器（tblPr/trPr/tcPr）法定子
    /// 元素名（无 w: 前缀）；xml=None 移除整元素，Some=合法单根片段整元素替换/
    /// 按 schema 序插入。片段从 inspect_docx projection=tblpr 原文复制修改。
    /// gridSpan/hMerge/vMerge 是结构属性受保护（改了破坏网格对齐）——合并拆分
    /// 走 merge_cells / split_cell。同格不同 element 可同批组合（S3 七波：
    /// vAlign + tcBorders 一批，序无关）。
    SetTableElement {
        block: usize,
        expect_prefix: String,
        level: TableLevel,
        row: Option<usize>,
        cell: Option<usize>,
        element: String,
        xml: Option<String>,
    },
    /// 合并单元格（S3 四波·结构件，Word 原生语义；五波补矩形区）：两形态二选一——
    /// ① 简单线并（direction + span，span 缺省 2）：horizontal=同行连续格横并
    /// （gridSpan 求和、内容按序拼接到首格）；vertical=同列对齐格纵并（首格
    /// restart / 其余 continue，内容原样保留——拆分即恢复显示）。
    /// ② 矩形区（end_row + end_cell，与 direction/span 互斥）：(row,cell) 至
    /// (end_row,end_cell) 显示地址围成的区域一次合并（逐行横并 → 结果列纵并，
    /// Word 合并区域 UX 同款）。结构重构使地址重排，独占一批。
    MergeCells {
        block: usize,
        expect_prefix: String,
        direction: Option<MergeDirection>,
        row: usize,
        cell: usize,
        span: Option<usize>,
        end_row: Option<usize>,
        end_cell: Option<usize>,
    },
    /// 拆分单元格（S3 四波·结构件，merge_cells 的逆）：vertical 对 (合并头) 拆
    /// 整条纵并链（各格恢复自有内容显示）；horizontal 对 (跨N列) 格拆回 N 个
    /// 单格（内容留首格，其余空段继承首段格式）。独占一批。
    SplitCell {
        block: usize,
        expect_prefix: String,
        direction: MergeDirection,
        row: usize,
        cell: usize,
    },
    /// 删除表格一行（S3 七波·生产反馈 P0）：结构重构（行号重排），独占一批。
    /// 纵向合并守卫——行内含合并头且下方同网格列仍有续格 → 拒（内容在头格，
    /// 删 = 内容丢失 + 续格孤儿化），指路 split_cell 先拆；行内仅普通格/纯续格
    /// （头在上方）可删，链条缩短仍合法。仅剩 1 行 → 拒（空表非法），指路
    /// delete_block 删整表。
    DeleteTableRow {
        block: usize,
        expect_prefix: String,
        row: usize,
    },
    /// 清空正文全部块（S3 五波·模板件）：copy_file 复制模板 docx 后清掉旧内容、
    /// 保留页面设置与节结构，再写新正文。含 sectPr 的块（节属性载体）保留；
    /// body 末尾的直属 sectPr 本就不是块、天然保留。expect_blocks = 当前块数
    /// 指纹（验证后变异——清空是破坏性操作，块数不符 = 文档已变，防陈旧心智）。
    /// 独占一批（清空后一切块号失效，与其他操作组合必然寻址错乱）。
    ClearBody { expect_blocks: usize },
}

/// set_table_element 的三层作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableLevel {
    Table,
    Row,
    Cell,
}

/// merge_cells / split_cell 的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeDirection {
    Horizontal,
    Vertical,
}

/// CT_PPr 法定子元素，ECMA-376 schema 序。双职：合法性白名单 + 插入位序。
/// sectPr（分节符载体）/pPrChange（修订记录）受保护不开放，不在本表。
/// def_edit 的样式 pPr 容器复用本表（pub(super)）。
pub(super) const PPR_ELEMENTS: [&str; 34] = [
    "pStyle", "keepNext", "keepLines", "pageBreakBefore", "framePr", "widowControl",
    "numPr", "suppressLineNumbers", "pBdr", "shd", "tabs", "suppressAutoHyphens",
    "kinsoku", "wordWrap", "overflowPunct", "topLinePunct", "autoSpaceDE", "autoSpaceDN",
    "bidi", "adjustRightInd", "snapToGrid", "spacing", "ind", "contextualSpacing",
    "mirrorIndents", "suppressOverlap", "jc", "textDirection", "textAlignment",
    "textboxTightWrap", "outlineLvl", "divId", "cnfStyle", "rPr",
];

/// CT_TblPrBase 法定子元素，ECMA-376 schema 序（set_table_element level=table；
/// def_edit 的表样式 tblPr 容器复用本表，pub(super)）。
pub(super) const TBLPR_ELEMENTS: [&str; 17] = [
    "tblStyle", "tblpPr", "tblOverlap", "bidiVisual", "tblStyleRowBandSize",
    "tblStyleColBandSize", "tblW", "jc", "tblCellSpacing", "tblInd",
    "tblBorders", "shd", "tblLayout", "tblCellMar", "tblLook",
    "tblCaption", "tblDescription",
];

/// CT_TrPr 法定子元素（set_table_element level=row）。
const TRPR_ELEMENTS: [&str; 12] = [
    "cnfStyle", "divId", "gridBefore", "gridAfter", "wBefore", "wAfter",
    "cantSplit", "trHeight", "tblHeader", "tblCellSpacing", "jc", "hidden",
];

/// CT_TcPr 法定子元素（set_table_element level=cell）。
const TCPR_ELEMENTS: [&str; 13] = [
    "cnfStyle", "tcW", "gridSpan", "hMerge", "vMerge", "tcBorders", "shd",
    "noWrap", "tcMar", "textDirection", "tcFitText", "vAlign", "hideMark",
];

/// tcPr 结构属性（受保护）：改 = 破坏与 tblGrid/相邻行的对齐，Word 报文档
/// 损坏。合并/拆分走 merge_cells / split_cell（连内容语义一起处理）。
const TCPR_PROTECTED: [&str; 3] = ["gridSpan", "hMerge", "vMerge"];

/// 单操作执行摘要（agent 读回验证用）。
#[derive(Debug, Serialize)]
pub struct AppliedOp {
    pub op: &'static str,
    pub block: usize,
    /// 原块投影文本（前 60 字）
    pub before: String,
    /// 新块投影文本（前 60 字；delete 为空）
    pub after: String,
    /// set_style 专用：应用后的样式 ID（其余操作 None，序列化省略）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// set_style 专用：目标段当前已是该样式（空转）。显式报出而非让 agent 从
    /// 「成功但文档没变」里猜——生产样本（2026-08-24）：agent 对已带目标样式的段
    /// set_style 读到成功，误判为「工具没生效」，转而回滚已正确的文档
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_unchanged: Option<bool>,
    /// 定义部件操作（styles/numbering）专用：操作定位串（如
    /// `style 'heading 1' pPr/spacing`），块级操作 None（序列化省略）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// 组合入口（工具层消费）：docx 字节 + 操作批 → 新 docx 字节 + 摘要。
/// 读 document.xml 与 styles.xml → 手术 → 重打包；IO/写盘/授权在调用方。
pub fn apply_edits_to_bytes(bytes: &[u8], ops: &[EditOp]) -> AppResult<(Vec<u8>, Vec<AppliedOp>)> {
    apply_edits_to_bytes_locked(bytes, ops, None)
}

/// [`apply_edits_to_bytes`] 的带锁版（D15 八波②）：`allowed_blocks = Some((lo, hi))`
/// 区间锁（1-based 闭区间）——批内任何操作的块地址越界 → 整批拒；clear_body 与
/// 锁语义冲突拒。范围守护是引擎硬约束（与 expect_prefix 同族），不靠 agent 自觉。
pub fn apply_edits_to_bytes_locked(
    bytes: &[u8],
    ops: &[EditOp],
    allowed_blocks: Option<(usize, usize)>,
) -> AppResult<(Vec<u8>, Vec<AppliedOp>)> {
    let xml = super::docx::read_document_xml(bytes)?;
    let styles = match super::docx::read_entry(bytes, "word/styles.xml")? {
        Some(s) => {
            super::styles::parse_styles(&super::xml_dom::parse(&s)?)
        }
        None => Stylesheet::empty(),
    };
    let (new_xml, applied) = apply_edits_locked(&xml, &styles, ops, allowed_blocks)?;
    let out = repack_part(bytes, "word/document.xml", &new_xml)?;
    Ok((out, applied))
}

/// 应用一批操作，返回新 document.xml 与逐操作摘要。**全有或全无**：任一预检
/// 不通过 → Err，原 xml 不动。（生产路径走 [`apply_edits_to_bytes_locked`]；
/// 本无锁形态是测试直调入口——xml/styles 层单测百余处。）
#[cfg(test)]
pub(super) fn apply_edits(
    xml: &str,
    styles: &Stylesheet,
    ops: &[EditOp],
) -> AppResult<(String, Vec<AppliedOp>)> {
    apply_edits_locked(xml, styles, ops, None)
}

/// [`apply_edits`] 的实现（带区间锁参数；None = 不锁，测试直调 `apply_edits` 即可）。
pub(super) fn apply_edits_locked(
    xml: &str,
    styles: &Stylesheet,
    ops: &[EditOp],
    allowed_blocks: Option<(usize, usize)>,
) -> AppResult<(String, Vec<AppliedOp>)> {
    if ops.is_empty() {
        return Err(AppError::Validation(
            "操作列表为空: edit_docx 至少需要一个操作。请提供 replace_text / insert_paragraph_after / delete_block / set_style / set_format 操作。".into(),
        ));
    }

    let dom = super::xml_dom::parse(xml)?;
    let model = docx_model::build_document(&dom);
    let spans = locate_blocks(xml)?;
    if spans.len() != model.body.len() {
        return Err(AppError::Internal(format!(
            "定位器与模型编址不一致: 定位 {} 块 / 模型 {} 块（内部 bug，请反馈）",
            spans.len(),
            model.body.len()
        )));
    }

    // ---- 预检全批（验证后变异：全部通过才动刀）----
    // 占用规则：段落操作/锚操作（insert_*_after）每块每批限一个——例外：同锚块可挂
    // 多条 insert_paragraph_after 按序链式（S3 七波）；表格修改操作（set_cell_text /
    // set_cell_format / set_table_element / insert_table_row_after）同块可多个按序组合
    // （一次填整行/整表；跨表格块也各自可挂，一次批多张表），但与段落操作/锚互斥，
    // 同格去重键 = (row, cell, 目标键)——set_table_element 按元素去重（同格不同元素
    // 可组合），内容/格式手术每格限一条；结构重构（merge_cells / split_cell /
    // delete_table_row）改地址布局，独占一批。
    let mut used_blocks: Vec<usize> = Vec::new();
    let mut table_modified: Vec<usize> = Vec::new();
    let mut used_cells: Vec<(usize, usize, usize, String)> = Vec::new(); // (block, row, cell, 目标键)
    let mut insert_anchors: Vec<usize> = Vec::new(); // 链式插入锚块（同块多条 insert_paragraph_after）
    let mut table_structural: Vec<usize> = Vec::new(); // merge/split 独占标记
    // 表格批内虚拟行状态（block → (每行格数, 虚拟行的原模型模板行)）：
    // 同块「先增行后填格」合法——寻址按批内输入序生效，结构与格数继承模板行
    let mut table_state: HashMap<usize, (Vec<usize>, Vec<Option<usize>>)> = HashMap::new();
    for op in ops {
        // ClearBody 不寻址块——先于块号提取处理（独占一批 + 块数指纹）
        if let EditOp::ClearBody { expect_blocks } = op {
            if allowed_blocks.is_some() {
                // 锁检查够不到 continue 之后——在此特判：清空正文作用于全文，
                // 与任何区间语义冲突（D15 八波②）
                return Err(AppError::Validation(
                    "区间外块: clear_body 清空全部正文，与 allowed_blocks 区间锁冲突。\
                     带锁批次只允许区间内的块级操作。如确需清空正文，请去掉 \
                     allowed_blocks 重发。".into(),
                ));
            }
            if ops.len() != 1 {
                return Err(AppError::Validation(
                    "同一批多操作: clear_body 须独占一批（清空后一切块号失效，\
                     与其他操作组合必然寻址错乱）。请拆批：先 clear_body，\
                     再重新 inspect_docx 寻址写入。".into(),
                ));
            }
            if *expect_blocks != model.body.len() {
                return Err(AppError::Validation(format!(
                    "指纹不符: 文档当前共 {} 块，expect_blocks={expect_blocks}。\
                     文档可能已被其他编辑改动——请先用 inspect_docx outline \
                     复核块数再重试。",
                    model.body.len()
                )));
            }
            continue;
        }
        let block = *match op {
            EditOp::ReplaceText { block, .. }
            | EditOp::InsertParagraphAfter { block, .. }
            | EditOp::DeleteBlock { block, .. }
            | EditOp::SetStyle { block, .. }
            | EditOp::SetFormat { block, .. }
            | EditOp::SetPprElement { block, .. }
            | EditOp::InsertTableAfter { block, .. }
            | EditOp::SetCellText { block, .. }
            | EditOp::InsertTableRowAfter { block, .. }
            | EditOp::SetCellFormat { block, .. }
            | EditOp::SetTableElement { block, .. }
            | EditOp::MergeCells { block, .. }
            | EditOp::SplitCell { block, .. }
            | EditOp::DeleteTableRow { block, .. } => block,
            // clear_body 无块寻址（上文 if-let 已 continue，此 match 不会遇到）
            EditOp::ClearBody { .. } => unreachable!("clear_body 已在上文 continue"),
        };
        if block == 0 || block > spans.len() {
            return Err(AppError::Validation(format!(
                "块号越界: 块 {block} 不存在（全文共 {} 块，块号 1-{}）。请先用 inspect_docx text 复核块号。",
                spans.len(),
                spans.len()
            )));
        }
        // 区间锁（D15 八波②）：块提取+越界校验之后、占用判定之前——block 已得且
        // 未动任何账本；InsertParagraphAfter / DeleteBlock 锚=目标同址，天然全覆盖
        if let Some((lo, hi)) = allowed_blocks {
            if block < lo || block > hi {
                return Err(AppError::Validation(format!(
                    "区间外块: 块 {block} 不在允许编辑区间 {lo}..={hi}（本批带 \
                     allowed_blocks 范围锁，锁外块禁止改动）。为什么：范围锁由任务方\
                     设定，防止越界改动受保护内容。怎么办：复核任务范围只动区间内块；\
                     如任务确需改动锁外块，请与任务方确认后去掉 allowed_blocks 重发。"
                )));
            }
        }
        if matches!(
            op,
            EditOp::MergeCells { .. } | EditOp::SplitCell { .. } | EditOp::DeleteTableRow { .. }
        ) {
            // 结构重构独占：合并/拆分/删行使行列地址重排，与任何其他操作同块都歧义
            if used_blocks.contains(&block) || table_modified.contains(&block) {
                return Err(AppError::Validation(format!(
                    "同一块多操作: 块 {block} 已被其他操作引用。merge_cells / split_cell / \
                     delete_table_row 使行列地址重排，须独占该表——请拆批，结构改完再寻址。"
                )));
            }
            used_blocks.push(block);
            table_structural.push(block);
        } else if table_structural.contains(&block) {
            return Err(AppError::Validation(format!(
                "同一块多操作: 块 {block} 已挂 merge_cells / split_cell / delete_table_row\
                 （结构重构独占）。请拆批：结构改完重新 inspect_docx projection=table 寻址。"
            )));
        } else {
            let is_table_modify = matches!(
                op,
                EditOp::SetCellText { .. }
                    | EditOp::InsertTableRowAfter { .. }
                    | EditOp::SetCellFormat { .. }
                    | EditOp::SetTableElement { .. }
            );
            if is_table_modify {
                if used_blocks.contains(&block) {
                    return Err(AppError::Validation(format!(
                        "同一块多操作: 块 {block} 已被段落操作/锚操作引用，不能再挂表格操作。请拆批。"
                    )));
                }
                if !table_modified.contains(&block) {
                    table_modified.push(block);
                }
            } else if matches!(op, EditOp::InsertParagraphAfter { .. }) {
                // 链式插入（S3 七波）：同锚块多条 insert_paragraph_after 按输入序
                // 连续排列（apply 侧聚合进一个插入 splice）；与其他段落/锚/表格操作仍互斥
                if used_blocks.contains(&block) || table_modified.contains(&block) {
                    return Err(AppError::Validation(format!(
                        "同一块多操作: 块 {block} 已被段落操作/锚操作/表格操作引用，\
                         不能再挂 insert_paragraph_after。请拆批。"
                    )));
                }
                if !insert_anchors.contains(&block) {
                    insert_anchors.push(block);
                }
            } else {
                if used_blocks.contains(&block) {
                    return Err(AppError::Validation(format!(
                        "同一块多操作: 块 {block} 在本批中被多次引用。每块每批限一个操作\
                         （例外：同一锚块可挂多条 insert_paragraph_after 按序链式插入；\
                         表格块可挂多个 set_cell_text / set_cell_format / set_table_element / \
                         insert_table_row_after）；请拆成多批。"
                    )));
                }
                if insert_anchors.contains(&block) {
                    return Err(AppError::Validation(format!(
                        "同一块多操作: 块 {block} 已挂链式 insert_paragraph_after，\
                         不能再被其他段落操作/锚操作引用。请拆批。"
                    )));
                }
                if table_modified.contains(&block) {
                    return Err(AppError::Validation(format!(
                        "同一块多操作: 块 {block} 已挂表格操作，不能再被段落操作/锚操作引用。请拆批。"
                    )));
                }
                used_blocks.push(block);
            }
        }

        let idx = block - 1;
        let span = spans[idx];
        let block_xml = &xml[span.start..span.end];
        let expect = expect_prefix_of(op);
        let mut projected = String::new();
        docx_model::blocks_text(&model.body[idx..idx + 1], &mut projected);
        let projected = projected.trim_end_matches('\n');
        if !projected.starts_with(expect.trim_end()) {
            return Err(AppError::Validation(format!(
                "指纹不符: 块 {block} 当前内容不以预期文本开头。期望前缀 {:?}，实际开头 {:?}。\
                 文档可能已被其他编辑改动——请先用 inspect_docx text 查看块 {block} 当前内容再重试。",
                truncate(expect, 40),
                truncate(projected, 40)
            )));
        }

        match op {
            EditOp::DeleteBlock { .. } => {
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                if block_xml.contains("<w:sectPr") {
                    return Err(AppError::Validation(format!(
                        "节属性保护: 块 {block} 含节属性（分节符载体），删除会破坏分节。\
                         请选择其他块操作。"
                    )));
                }
            }
            EditOp::ReplaceText { .. } => {
                // sectPr 在 pPr 内被原样切片保留，改文本不破坏分节——放行
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
            }
            EditOp::InsertParagraphAfter { style, .. } => {
                if let Some(name) = style {
                    if styles.id_of(name).is_none() {
                        return Err(AppError::Validation(format!(
                            "未知样式: {:?} 不在本文档样式表中。可用样式（前 20）: {}。\
                             样式名来自 inspect_docx outline 的样式列；或省略 style 继承锚块格式。",
                            name,
                            styles.display_names_joined(20)
                        )));
                    }
                }
            }
            EditOp::SetStyle { style, .. } => {
                if styles.id_of(style).is_none() {
                    return Err(AppError::Validation(format!(
                        "未知样式: {:?} 不在本文档样式表中。可用样式（前 20）: {}。\
                         样式名接受显示名或 ID，来自 inspect_docx outline 的样式列。",
                        style,
                        styles.display_names_joined(20)
                    )));
                }
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                // pPrChange 是段落属性修订记录（内含旧 pStyle）——换样式会与其语义
                // 纠缠，MVP 与修订块同判拒绝
                if block_xml.contains("<w:pPrChange") {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带段落属性修订（pPrChange），默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                // sectPr 在 pPr 内被原样保留（手术只动 pStyle 元素），改样式不破坏分节——放行
            }
            EditOp::SetFormat { paragraph, character, .. } => {
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                if block_xml.contains("<w:pPrChange") {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带段落属性修订（pPrChange），默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                validate_formats(paragraph.as_ref(), character.as_ref(), false, "set_format")?;
            }
            EditOp::SetPprElement { element, xml, .. } => {
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                if block_xml.contains("<w:pPrChange") {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block} 带段落属性修订（pPrChange），默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该块。"
                    )));
                }
                if element == "sectPr" || element == "pPrChange" {
                    return Err(AppError::Validation(format!(
                        "受保护子元素: {element} 不开放通用元素操作（sectPr 是分节符载体，\
                         pPrChange 是修订记录）。修改分节结构不在支持范围；换段落样式请用 set_style。"
                    )));
                }
                if !PPR_ELEMENTS.contains(&element.as_str()) {
                    return Err(AppError::Validation(format!(
                        "非法pPr子元素: {:?} 不在段落属性（pPr）法定子元素清单。合法元素\
                         （schema 序）: {}。名称不带 w: 前缀；查现有元素原文用 inspect_docx \
                         projection=ppr。",
                        element,
                        PPR_ELEMENTS.join(" ")
                    )));
                }
                if let Some(x) = xml {
                    validate_fragment(element, x, "ppr")?;
                }
            }
            EditOp::InsertTableAfter { rows, table_style, .. } => {
                validate_table_rows(rows)?;
                if let Some(s) = table_style {
                    if styles.table_style_id(s).is_none() {
                        if styles.id_of(s).is_some() {
                            return Err(AppError::Validation(format!(
                                "类型不符: 样式 {:?} 存在但不是表样式（w:type=table 的样式才能\
                                 挂在表上）。表样式清单用 inspect_docx projection=styles 的 \
                                 type 列筛选。",
                                s
                            )));
                        }
                        return Err(AppError::Validation(format!(
                            "未知样式: {:?} 不在本文档样式表中。可用样式（前 20）: {}。\
                             表样式清单用 inspect_docx projection=styles 的 type 列筛选；\
                             或省略 table_style 用默认全边框样式。",
                            s,
                            styles.display_names_joined(20)
                        )));
                    }
                }
            }
            EditOp::SetCellText { row, cell, .. } => {
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block}（表格）带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该表。"
                    )));
                }
                precheck_cell_target(
                    &model, idx, block, *row, *cell, "set_cell_text", "",
                    &mut table_state, &mut used_cells,
                )?;
            }
            EditOp::InsertTableRowAfter { after_row, cells, .. } => {
                let Block::Table(t) = &model.body[idx] else {
                    return Err(AppError::Validation(format!(
                        "非表格块: 块 {block} 是段落，insert_table_row_after 只作用于表格块。\
                         建表用 insert_table_after。"
                    )));
                };
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block}（表格）带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该表。"
                    )));
                }
                let (counts, tpls) = table_state.entry(block).or_insert_with(|| {
                    (
                        t.rows.iter().map(|r| r.cells.len()).collect::<Vec<_>>(),
                        vec![None; t.rows.len()],
                    )
                });
                // 缺省模板 = 当前末行（含本批已增行——连续追加多条也成立）
                let tpl_virtual = after_row.unwrap_or(counts.len());
                if tpl_virtual == 0 || tpl_virtual > counts.len() {
                    return Err(AppError::Validation(format!(
                        "行号越界: after_row={tpl_virtual} 不存在（该表当前共 {} 行，1-based；\
                         缺省 = 末行后追加）。网格视图用 inspect_docx projection=table。",
                        counts.len()
                    )));
                }
                let real_tpl = tpls[tpl_virtual - 1].unwrap_or(tpl_virtual);
                let tpl = &t.rows[real_tpl - 1];
                if let Some(cs) = cells {
                    if cs.len() != tpl.cells.len() {
                        return Err(AppError::Validation(format!(
                            "列数不符: cells 给了 {} 格，模板行（第 {tpl_virtual} 行）共 {} 格\
                             （含跨列/合并格，各占 1 个序号）。请与网格视图对齐。",
                            cs.len(),
                            tpl.cells.len()
                        )));
                    }
                }
                if tpl
                    .cells
                    .iter()
                    .any(|c| c.blocks.iter().any(|b| matches!(b, Block::Table(_))))
                {
                    return Err(AppError::Validation(format!(
                        "嵌套表: 模板行（第 {tpl_virtual} 行）内有格含表格，克隆会复制嵌套结构，暂不支持。\
                         请选无嵌套表的模板行。"
                    )));
                }
                // 虚拟行入账：格数与结构继承模板
                counts.push(tpl.cells.len());
                tpls.push(Some(real_tpl));
            }
            EditOp::SetCellFormat { row, cell, paragraph, character, style, .. } => {
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block}（表格）带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该表。"
                    )));
                }
                // 格内段落属性修订与段落块同判拒绝（pPrChange 纠缠段落格式手术）
                if block_xml.contains("<w:pPrChange") {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block}（表格）带段落属性修订（pPrChange），默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该表。"
                    )));
                }
                if let Some(name) = style {
                    if styles.id_of(name).is_none() {
                        return Err(AppError::Validation(format!(
                            "未知样式: {:?} 不在本文档样式表中。可用样式（前 20）: {}。\
                             样式名接受显示名或 ID，来自 inspect_docx outline 的样式列。",
                            name,
                            styles.display_names_joined(20)
                        )));
                    }
                }
                validate_formats(paragraph.as_ref(), character.as_ref(), style.is_some(), "set_cell_format")?;
                precheck_cell_target(
                    &model, idx, block, *row, *cell, "set_cell_format", "",
                    &mut table_state, &mut used_cells,
                )?;
            }
            EditOp::SetTableElement { level, row, cell, element, xml, .. } => {
                let Block::Table(t) = &model.body[idx] else {
                    return Err(AppError::Validation(format!(
                        "非表格块: 块 {block} 是段落，set_table_element 只作用于表格块。\
                         表格属性编辑前请先用 inspect_docx projection=table 确认块号。"
                    )));
                };
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block}（表格）带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该表。"
                    )));
                }
                let (whitelist, container): (&[&str], &str) = match level {
                    TableLevel::Table => {
                        if row.is_some() || cell.is_some() {
                            return Err(AppError::Validation(
                                "参数校验失败: level=table 是表级属性，不接受 row/cell。\
                                 行级/格级属性改用 level=row / level=cell。"
                                    .into(),
                            ));
                        }
                        (&TBLPR_ELEMENTS, "tblPr")
                    }
                    TableLevel::Row => {
                        let Some(r) = *row else {
                            return Err(AppError::Validation(
                                "参数校验失败: level=row 需要 row（1-based 行号，与 projection=table 的 rN 同口径）。"
                                    .into(),
                            ));
                        };
                        if cell.is_some() {
                            return Err(AppError::Validation(
                                "参数校验失败: level=row 不接受 cell（格级属性用 level=cell + row + cell）。"
                                    .into(),
                            ));
                        }
                        let (counts, _) = table_state.entry(block).or_insert_with(|| {
                            (
                                t.rows.iter().map(|r| r.cells.len()).collect::<Vec<_>>(),
                                vec![None; t.rows.len()],
                            )
                        });
                        if r == 0 || r > counts.len() {
                            return Err(AppError::Validation(format!(
                                "行号越界: row={r} 不存在（该表当前共 {} 行，1-based，含本批已增行）。\
                                 网格视图用 inspect_docx projection=table。",
                                counts.len()
                            )));
                        }
                        (&TRPR_ELEMENTS, "trPr")
                    }
                    TableLevel::Cell => {
                        let (Some(r), Some(c)) = (*row, *cell) else {
                            return Err(AppError::Validation(
                                "参数校验失败: level=cell 需要 row + cell（双 1-based，与 projection=table \
                                 的 rN / 格序同口径）。"
                                    .into(),
                            ));
                        };
                        precheck_cell_target(
                            &model, idx, block, r, c, "set_table_element", element.as_str(),
                            &mut table_state, &mut used_cells,
                        )?;
                        (&TCPR_ELEMENTS, "tcPr")
                    }
                };
                if TCPR_PROTECTED.contains(&element.as_str()) && matches!(level, TableLevel::Cell) {
                    return Err(AppError::Validation(format!(
                        "受保护子元素: {element} 是单元格结构属性（与表格网格对齐耦合，硬改会产出\
                         Word 报损坏的文档）。合并/拆分单元格请用 merge_cells / split_cell。"
                    )));
                }
                if !whitelist.contains(&element.as_str()) {
                    return Err(AppError::Validation(format!(
                        "非法子元素: {:?} 不在{}法定子元素清单。合法元素（schema 序）: {}。\
                         名称不带 w: 前缀；查现有元素原文用 inspect_docx projection=tblpr。",
                        element, container, whitelist.join(" ")
                    )));
                }
                if let Some(x) = xml {
                    validate_fragment(element, x, "tblpr")?;
                }
            }
            // clear_body 已在上文 continue——本 match 只覆盖寻址类操作
            EditOp::ClearBody { .. } => unreachable!("clear_body 已在上文 continue"),
            EditOp::MergeCells { direction, row, cell, span, end_row, end_cell, .. } => {
                let Block::Table(t) = &model.body[idx] else {
                    return Err(AppError::Validation(format!(
                        "非表格块: 块 {block} 是段落，merge_cells 只作用于表格块。"
                    )));
                };
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block}（表格）带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该表。"
                    )));
                }
                let is_rect = end_row.is_some() || end_cell.is_some();
                match (is_rect, direction, span) {
                    // 矩形区形态：end_row+end_cell 同给，且不得带 direction/span
                    (true, Some(_), _) | (true, _, Some(_)) => {
                        return Err(AppError::Validation(
                            "参数冲突: 矩形合并（end_row+end_cell）不接受 direction/span。\
                             简单线并用 direction+span，矩形区用 end_row+end_cell，二选一。".into(),
                        ));
                    }
                    (true, None, None) => {
                        let (Some(er), Some(ec)) = (*end_row, *end_cell) else {
                            return Err(AppError::Validation(
                                "参数缺失: 矩形合并须同时给 end_row 与 end_cell（区域右下角，\
                                 与 (row,cell) 左上角配对）。".into(),
                            ));
                        };
                        if er < *row || ec < *cell || (er == *row && ec == *cell) {
                            return Err(AppError::Validation(format!(
                                "合并区域无效: ({row},{cell})..({er},{ec}) 不构成 ≥2 格的区域\
                                 （end_row/end_cell 不得小于左上角，且区域至少 2 格）。\
                                 网格视图用 inspect_docx projection=table。"
                            )));
                        }
                        if er > t.rows.len() {
                            return Err(AppError::Validation(format!(
                                "合并区域无效: end_row={er} 越界（该表共 {} 行，1-based）。",
                                t.rows.len()
                            )));
                        }
                        // 各行区域检查 + 网格列区间对齐（纵并判据 = 同网格列区间）
                        let mut anchor: Option<(u32, u32)> = None; // (前缀网格 g0, 区域网格宽)
                        for r in *row..=er {
                            let r_model = &t.rows[r - 1];
                            if ec > r_model.cells.len() {
                                return Err(AppError::Validation(format!(
                                    "合并区域无效: 第 {r} 行共 {} 格，end_cell={ec} 越界。\
                                     网格视图用 inspect_docx projection=table。",
                                    r_model.cells.len()
                                )));
                            }
                            for c in &r_model.cells[*cell - 1..ec] {
                                if c.v_merge.is_some() {
                                    return Err(AppError::Validation(format!(
                                        "合并结构冲突: 第 {r} 行区域内含纵向合并格（(合并头)/(续) 标记），\
                                         向其扩展会破坏下方网格对齐。先 split_cell 拆纵并，或换不含合并格的区域。"
                                    )));
                                }
                                if c.blocks.iter().any(|b| matches!(b, Block::Table(_))) {
                                    return Err(AppError::Validation(format!(
                                        "合并结构冲突: 第 {r} 行区域内含嵌套表，暂不支持。"
                                    )));
                                }
                            }
                            let (g0, _) = grid_range_of(r_model, *cell);
                            let w: u32 = r_model.cells[*cell - 1..ec]
                                .iter()
                                .map(|c| c.grid_span.unwrap_or(1))
                                .sum();
                            match anchor {
                                None => anchor = Some((g0, w)),
                                Some((h0, hw)) if h0 == g0 && hw == w => {}
                                Some(_) => {
                                    return Err(AppError::Validation(
                                        "合并结构冲突: 各行区域边界的网格列不对齐（跨列格布局不同），\
                                         无法合并成矩形。Word 同样拒绝这种合并。".into(),
                                    ));
                                }
                            }
                        }
                    }
                    (false, None, None) => {
                        return Err(AppError::Validation(
                            "参数缺失: merge_cells 须给 direction（简单线并：horizontal/vertical \
                             + 可选 span）或 end_row+end_cell（矩形区域），二选一。".into(),
                        ));
                    }
                    (false, None, Some(_)) => {
                        return Err(AppError::Validation(
                            "参数冲突: span 须与 direction 同用（简单线并）。矩形区合并\
                             请改用 end_row+end_cell（不带 span）。".into(),
                        ));
                    }
                    (false, Some(dir), sp) => {
                        let span = sp.unwrap_or(2);
                        if span < 2 {
                            return Err(AppError::Validation(format!(
                                "合并跨度无效: span={span}（≥2 才构成合并）。拆分已有合并用 split_cell。"
                            )));
                        }
                        match dir {
                            MergeDirection::Horizontal => {
                                let r = row_bounds(t, *row)?;
                                if *cell == 0 || cell + span - 1 > r.cells.len() {
                                    return Err(AppError::Validation(format!(
                                        "合并跨度无效: 第 {} 行从格 {} 起横并 {span} 格越界（该行共 {} 格，\
                                         1-based，跨列格占 1 个序号）。网格视图用 inspect_docx projection=table。",
                                        row, cell, r.cells.len()
                                    )));
                                }
                                for c in &r.cells[*cell - 1..*cell - 1 + span] {
                                    if c.v_merge.is_some() {
                                        return Err(AppError::Validation(
                                            "合并结构冲突: 范围内含纵向合并格（(合并头)/(续) 标记），\
                                             横并向其扩展会破坏下方网格对齐。先 split_cell 拆纵并，或换不含合并格的范围。"
                                                .into(),
                                        ));
                                    }
                                    if c.blocks.iter().any(|b| matches!(b, Block::Table(_))) {
                                        return Err(AppError::Validation(
                                            "合并结构冲突: 范围内含嵌套表，暂不支持。".into(),
                                        ));
                                    }
                                }
                            }
                            MergeDirection::Vertical => {
                                if *row == 0 || row + span - 1 > t.rows.len() {
                                    return Err(AppError::Validation(format!(
                                        "合并跨度无效: 第 {} 行起纵并 {span} 行越界（该表共 {} 行，1-based）。",
                                        row, t.rows.len()
                                    )));
                                }
                                let head = row_bounds(t, *row)?;
                                if *cell == 0 || *cell > head.cells.len() {
                                    return Err(AppError::Validation(format!(
                                        "单元格越界: 块 {block} 第 {row} 行第 {cell} 格不存在（该行共 {} 格）。\
                                         网格视图用 inspect_docx projection=table。",
                                        head.cells.len()
                                    )));
                                }
                                if head.cells[*cell - 1].v_merge.as_deref() == Some("continue") {
                                    return Err(AppError::Validation(format!(
                                        "纵向合并续格: 块 {block} 第 {row} 行第 {cell} 格是续格（(续)），\
                                         请对该列上方带「(合并头)」标记的格执行合并。"
                                    )));
                                }
                                if head.cells[*cell - 1].blocks.iter().any(|b| matches!(b, Block::Table(_))) {
                                    return Err(AppError::Validation(
                                        "合并结构冲突: 首格含嵌套表，暂不支持。".into(),
                                    ));
                                }
                                let (g0, g1) = grid_range_of(head, *cell);
                                for r2 in *row + 1..=*row + span - 1 {
                                    let r_model = &t.rows[r2 - 1];
                                    match cell_at_grid_range(r_model, g0, g1) {
                                        None => {
                                            return Err(AppError::Validation(format!(
                                                "合并结构冲突: 第 {r2} 行该列的格边界不对齐（跨列格布局不同），\
                                                 无法纵向合并。Word 同样拒绝这种合并。"
                                            )));
                                        }
                                        Some(c) => {
                                            if c.blocks.iter().any(|b| matches!(b, Block::Table(_))) {
                                                return Err(AppError::Validation(format!(
                                                    "合并结构冲突: 第 {r2} 行该列含嵌套表，暂不支持。"
                                                )));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            EditOp::SplitCell { direction, row, cell, .. } => {
                let Block::Table(t) = &model.body[idx] else {
                    return Err(AppError::Validation(format!(
                        "非表格块: 块 {block} 是段落，split_cell 只作用于表格块。"
                    )));
                };
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block}（表格）带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该表。"
                    )));
                }
                let r = row_bounds(t, *row)?;
                if *cell == 0 || *cell > r.cells.len() {
                    return Err(AppError::Validation(format!(
                        "单元格越界: 块 {block} 第 {row} 行第 {cell} 格不存在（该行共 {} 格）。\
                         网格视图用 inspect_docx projection=table。",
                        r.cells.len()
                    )));
                }
                let c = &r.cells[*cell - 1];
                match direction {
                    MergeDirection::Vertical => {
                        if c.v_merge.as_deref() == Some("continue") {
                            return Err(AppError::Validation(format!(
                                "纵向合并续格: 块 {block} 第 {row} 行第 {cell} 格是续格（(续)）。\
                                 纵向拆分请对该列上方带「(合并头)」标记的格执行。"
                            )));
                        }
                        if c.v_merge.as_deref() != Some("restart") {
                            return Err(AppError::Validation(format!(
                                "非合并格: 块 {block} 第 {row} 行第 {cell} 格不是纵向合并头\
                                 （无 (合并头) 标记），无可拆分的纵向合并。"
                            )));
                        }
                    }
                    MergeDirection::Horizontal => {
                        if c.grid_span.unwrap_or(1) < 2 {
                            return Err(AppError::Validation(format!(
                                "非合并格: 块 {block} 第 {row} 行第 {cell} 格无横向跨列\
                                 （无 (跨N列) 标记），无可拆分的横向合并。"
                            )));
                        }
                    }
                }
            }
            EditOp::DeleteTableRow { row, .. } => {
                let Block::Table(t) = &model.body[idx] else {
                    return Err(AppError::Validation(format!(
                        "非表格块: 块 {block} 是段落，delete_table_row 只作用于表格块。\
                         删段落块用 delete_block。"
                    )));
                };
                if has_revision(&model.body[idx]) {
                    return Err(AppError::Validation(format!(
                        "含修订标记: 块 {block}（表格）带插入/删除修订，默认不触碰修订内容。\
                         请先在 Word 中接受或拒绝修订后再编辑该表。"
                    )));
                }
                let r_model = row_bounds(t, *row)?;
                if t.rows.len() == 1 {
                    return Err(AppError::Validation(
                        "空表保护: 该表仅剩此 1 行——删掉后成空表（非法结构）。\
                         要移除整个表请用 delete_block 删该表格块。"
                            .into(),
                    ));
                }
                // 纵向合并守卫：本行含合并头且下方同网格列还有续格 → 删除该行会
                // 孤儿化续格（内容在头格，删 = 内容一并丢失）。纯续格行可删——
                // 头在上方，链条缩短仍是合法结构。
                for (ci, c) in r_model.cells.iter().enumerate() {
                    if c.v_merge.as_deref() != Some("restart") {
                        continue;
                    }
                    let (g0, g1) = grid_range_of(r_model, ci + 1);
                    let orphan = t.rows[*row..].iter().any(|below| {
                        below.cells.iter().enumerate().any(|(bi, bc)| {
                            bc.v_merge.as_deref() == Some("continue") && {
                                let (b0, b1) = grid_range_of(below, bi + 1);
                                b0 < g1 && b1 > g0
                            }
                        })
                    });
                    if orphan {
                        return Err(AppError::Validation(format!(
                            "合并结构冲突: 块 {block} 第 {row} 行第 {} 格是纵向合并头\
                             （(合并头) 标记）且合并区延伸到下方行——删除该行会丢失合并格\
                             内容并孤儿化下方续格。请先 split_cell vertical 拆掉纵并链再删行，\
                             或改删合并区下方不含合并头的行。",
                            ci + 1
                        )));
                    }
                }
            }
        }
        // Replace / SetStyle / SetFormat / SetPprElement 目标必须是段落块
        if matches!(
            op,
            EditOp::ReplaceText { .. }
                | EditOp::SetStyle { .. }
                | EditOp::SetFormat { .. }
                | EditOp::SetPprElement { .. }
        ) && span.is_table {
            return Err(AppError::Validation(format!(
                "表格块: 块 {block} 是表格，该操作只支持段落。\
                 表格编辑请用：set_cell_text（改格文本）/ set_cell_format（格内文字格式）/ \
                 set_table_element（边框·底纹等表格属性）/ insert_table_row_after（增行）/ \
                 insert_table_after（建新表）/ merge_cells·split_cell（合并/拆分）；\
                 网格视图用 inspect_docx projection=table。"
            )));
        }
    }

    // ---- 生成 splice 计划（原始偏移；互不重叠，见预检的占用规则）----
    struct Splice {
        pos: usize,
        remove_end: usize,
        insert: String,
        /// 该 splice 的逐操作摘要（同块表格批组合 = 多条；顺序 = 输入序）
        summaries: Vec<AppliedOp>,
    }
    let mut plan: Vec<Splice> = Vec::new();
    // 同表多操作聚合：块号 → plan 索引（共享一个 splice，按序改写 insert 文本）
    let mut table_plan_idx: HashMap<usize, usize> = HashMap::new();
    // 同锚多段聚合（S3 七波）：锚块号 → plan 索引——链式 insert_paragraph_after
    // 追加进同一个插入 splice（链序 = 输入序）。不按「span.end + 累计长度」定位：
    // 那会伸进相邻块的 splice 区间（块间缝隙可能为 0），聚合进单 splice 无重叠风险。
    let mut insert_plan_idx: HashMap<usize, usize> = HashMap::new();
    for op in ops {
        match op.clone() {
            EditOp::ReplaceText { block, new_text, .. } => {
                let span = spans[block - 1];
                let new_block = rebuild_paragraph(xml, span, Some(&new_text));
                let after = new_text.clone();
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: new_block,
                    summaries: vec![AppliedOp {
                        op: "replace_text",
                        block,
                        before: projected_of(&model, block),
                        after: truncate(&after, 60),
                        style: None,
                        style_unchanged: None,
                        target: None,
                    }],
                });
            }
            EditOp::InsertParagraphAfter { block, text, style, .. } => {
                let span = spans[block - 1];
                let anchor_has_revision = has_revision(&model.body[block - 1]);
                let new_block = build_inserted_paragraph(xml, span, &text, style.as_deref(), styles, anchor_has_revision);
                // 同锚链式（S3 七波）：首条建插入 splice（锚块末尾，区间外），后续
                // 条追加——整链是一个插入 blob，链序 = 输入序，无 splice 重叠
                let entry = match insert_plan_idx.get(&block) {
                    Some(&i) => i,
                    None => {
                        plan.push(Splice {
                            pos: span.end, // 锚块末尾插入（区间外，不与同批其他 splice 重叠）
                            remove_end: span.end,
                            insert: String::new(),
                            summaries: Vec::new(),
                        });
                        insert_plan_idx.insert(block, plan.len() - 1);
                        plan.len() - 1
                    }
                };
                plan[entry].insert.push_str(&new_block);
                plan[entry].summaries.push(AppliedOp {
                    op: "insert_paragraph_after",
                    block,
                    before: projected_of(&model, block),
                    after: truncate(&text, 60),
                    style: None,
                    style_unchanged: None,
                    target: None,
                });
            }
            EditOp::DeleteBlock { block, .. } => {
                let span = spans[block - 1];
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: String::new(),
                    summaries: vec![AppliedOp {
                        op: "delete_block",
                        block,
                        before: projected_of(&model, block),
                        after: String::new(),
                        style: None,
                        style_unchanged: None,
                        target: None,
                    }],
                });
            }
            EditOp::SetStyle { block, style, .. } => {
                let span = spans[block - 1];
                // 预检已校验样式存在；此处重解析拿 ID（批内样式表不可变，无 TOCTOU）
                let style_id = styles.id_of(&style).expect("预检已校验样式存在");
                // 空转检测：目标段 pStyle 已是该样式 ID → 显式报 style_unchanged，
                // 勿让 agent 把「成功」读成「生效了」
                let already = match &model.body[block - 1] {
                    Block::Paragraph(p) => p.props.style.as_deref() == Some(style_id),
                    Block::Table(_) => false, // 预检已拒表格块
                };
                let new_block =
                    restyle_paragraph(&xml[span.start..span.end], style_id)
                        .ok_or_else(|| AppError::Internal(format!(
                            "段落改样式失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        )))?;
                let projected = projected_of(&model, block);
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: new_block,
                    summaries: vec![AppliedOp {
                        op: "set_style",
                        block,
                        before: projected.clone(),
                        after: projected, // 文本不变；样式见 style 字段
                        style: Some(style_id.to_string()),
                        style_unchanged: already.then_some(true),
                        target: None,
                    }],
                });
            }
            EditOp::SetFormat { block, paragraph, character, .. } => {
                let span = spans[block - 1];
                let mut new_block = xml[span.start..span.end].to_string();
                if let Some(para) = &paragraph {
                    new_block = reformat_ppr(&new_block, para)
                        .ok_or_else(|| AppError::Internal(format!(
                            "段落格式手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        )))?;
                }
                if let Some(ch) = &character {
                    new_block = reformat_runs(&new_block, ch)
                        .ok_or_else(|| AppError::Internal(format!(
                            "字符格式手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        )))?;
                }
                let projected = projected_of(&model, block);
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: new_block,
                    summaries: vec![AppliedOp {
                        op: "set_format",
                        block,
                        before: projected.clone(),
                        // 文本不变；after 携带本次应用的格式摘要（agent 读回验证）
                        after: truncate(&describe_formats(paragraph.as_ref(), character.as_ref()), 60),
                        style: None,
                        style_unchanged: None,
                        target: None,
                    }],
                });
            }
            EditOp::SetPprElement { block, element, xml: frag, .. } => {
                let span = spans[block - 1];
                let (new_block, changed) =
                    set_ppr_element(&xml[span.start..span.end], &element, frag.as_deref())
                        .ok_or_else(|| AppError::Internal(format!(
                            "pPr元素手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        )))?;
                // 诚实边界：段级 numPr 移除后，若样式链仍定义编号，Word 会回退显示
                // 样式编号（直接格式覆盖样式 → 摘除直接格式 = 落回样式定义）——
                // 显式警告，勿让 agent 以为编号已消失
                let mut after = if changed {
                    match frag.as_deref() {
                        None => format!("removed {element}"),
                        Some(_) => format!("set {element}"),
                    }
                } else {
                    format!("{element} 不存在（空转，文档未变）")
                };
                if changed
                    && frag.is_none()
                    && element == "numPr"
                    && style_chain_defines_numbering(&model, block, styles)
                {
                    after.push_str("；警告：该段样式链仍定义编号，Word 将回退显示样式编号——需改样式定义才彻底去编号");
                }
                let projected = projected_of(&model, block);
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: new_block,
                    summaries: vec![AppliedOp {
                        op: "set_ppr_element",
                        block,
                        before: projected,
                        after,
                        style: None,
                        style_unchanged: None,
                        target: None,
                    }],
                });
            }
            EditOp::InsertTableAfter { block, rows, header, table_style, .. } => {
                let span = spans[block - 1];
                // 预检已验存在 + 类型；此处取 ID 挂 tblStyle（批内样式表不可变，无 TOCTOU）
                let style_id = table_style.as_deref().and_then(|s| styles.table_style_id(s));
                let new_tbl = build_table_xml(
                    &rows,
                    header.unwrap_or(true),
                    style_id,
                    content_width_twips(&model),
                );
                let mut after = format!(
                    "{}行×{}列 表",
                    rows.len(),
                    rows.first().map(|r| r.len()).unwrap_or(0)
                );
                if let Some(s) = &table_style {
                    after.push_str(&format!("（表样式 {s}）"));
                }
                plan.push(Splice {
                    pos: span.end, // 锚块末尾插入
                    remove_end: span.end,
                    insert: new_tbl,
                    summaries: vec![AppliedOp {
                        op: "insert_table_after",
                        block,
                        before: projected_of(&model, block),
                        after,
                        style: None,
                        style_unchanged: None,
                        target: None,
                    }],
                });
            }
            EditOp::SetCellText { block, row, cell, text, .. } => {
                let span = spans[block - 1];
                // 同表聚合：首次触达建 splice（以原块为底），后续操作改写 insert
                let entry = match table_plan_idx.get(&block) {
                    Some(&i) => i,
                    None => {
                        plan.push(Splice {
                            pos: span.start,
                            remove_end: span.end,
                            insert: xml[span.start..span.end].to_string(),
                            summaries: Vec::new(),
                        });
                        table_plan_idx.insert(block, plan.len() - 1);
                        plan.len() - 1
                    }
                };
                let cur = plan[entry].insert.clone();
                let new_xml = set_cell_text(&cur, row, cell, &text)
                    .ok_or_else(|| AppError::Internal(format!(
                        "单元格手术失败: 块 {block} 第 {row} 行第 {cell} 格 XML 形态异常（内部 bug，未写盘）"
                    )))?;
                plan[entry].insert = new_xml;
                plan[entry].summaries.push(AppliedOp {
                    op: "set_cell_text",
                    block,
                    before: cell_projected_of(&model, block, row, cell),
                    after: truncate(&text, 60),
                    style: None,
                    style_unchanged: None,
                    target: None,
                });
            }
            EditOp::InsertTableRowAfter { block, after_row, cells, .. } => {
                let span = spans[block - 1];
                let entry = match table_plan_idx.get(&block) {
                    Some(&i) => i,
                    None => {
                        plan.push(Splice {
                            pos: span.start,
                            remove_end: span.end,
                            insert: xml[span.start..span.end].to_string(),
                            summaries: Vec::new(),
                        });
                        table_plan_idx.insert(block, plan.len() - 1);
                        plan.len() - 1
                    }
                };
                let cur = plan[entry].insert.clone();
                let new_xml = insert_table_row_after(&cur, after_row, cells.as_deref())
                    .ok_or_else(|| AppError::Internal(format!(
                        "表格增行失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                    )))?;
                plan[entry].insert = new_xml;
                let filled = cells.as_ref().map(|c| c.len()).unwrap_or(0);
                let after = format!(
                    "克隆第 {} 行插入（{}）",
                    after_row.map(|r| r.to_string()).unwrap_or_else(|| "末行".into()),
                    if filled > 0 { format!("{filled} 格已填") } else { "空行".to_string() }
                );
                plan[entry].summaries.push(AppliedOp {
                    op: "insert_table_row_after",
                    block,
                    before: projected_of(&model, block),
                    after,
                    style: None,
                    style_unchanged: None,
                    target: None,
                });
            }
            EditOp::SetCellFormat { block, row, cell, paragraph, character, style, .. } => {
                let span = spans[block - 1];
                let entry = match table_plan_idx.get(&block) {
                    Some(&i) => i,
                    None => {
                        plan.push(Splice {
                            pos: span.start,
                            remove_end: span.end,
                            insert: xml[span.start..span.end].to_string(),
                            summaries: Vec::new(),
                        });
                        table_plan_idx.insert(block, plan.len() - 1);
                        plan.len() - 1
                    }
                };
                let cur = plan[entry].insert.clone();
                // 预检已校验样式存在；此处重解析拿 ID（批内样式表不可变，无 TOCTOU）
                let style_id = style.as_deref().map(|name| {
                    styles.id_of(name).expect("预检已校验样式存在")
                });
                let new_xml =
                    set_cell_format_xml(&cur, row, cell, paragraph.as_ref(), character.as_ref(), style_id)
                        .ok_or_else(|| AppError::Internal(format!(
                            "格内格式手术失败: 块 {block} 第 {row} 行第 {cell} 格 XML 形态异常（内部 bug，未写盘）"
                        )))?;
                plan[entry].insert = new_xml;
                let fmts = describe_formats(paragraph.as_ref(), character.as_ref());
                let after = match &style {
                    Some(name) if fmts == "（无字段）" => format!("样式={name}"),
                    Some(name) => format!("样式={name} {fmts}"),
                    None => fmts,
                };
                plan[entry].summaries.push(AppliedOp {
                    op: "set_cell_format",
                    block,
                    before: cell_projected_of(&model, block, row, cell),
                    after: truncate(&after, 60),
                    style: style_id.map(str::to_string),
                    style_unchanged: None,
                    target: None,
                });
            }
            EditOp::SetTableElement { block, level, row, cell, element, xml: frag, .. } => {
                let span = spans[block - 1];
                let entry = match table_plan_idx.get(&block) {
                    Some(&i) => i,
                    None => {
                        plan.push(Splice {
                            pos: span.start,
                            remove_end: span.end,
                            insert: xml[span.start..span.end].to_string(),
                            summaries: Vec::new(),
                        });
                        table_plan_idx.insert(block, plan.len() - 1);
                        plan.len() - 1
                    }
                };
                let cur = plan[entry].insert.clone();
                let (new_xml, changed) =
                    set_table_element_xml(&cur, level, row, cell, &element, frag.as_deref())
                        .ok_or_else(|| AppError::Internal(format!(
                            "表格属性手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        )))?;
                plan[entry].insert = new_xml;
                let where_at = match (level, row, cell) {
                    (TableLevel::Table, _, _) => "table".to_string(),
                    (TableLevel::Row, Some(r), _) => format!("r{r}"),
                    (TableLevel::Cell, Some(r), Some(c)) => format!("r{r}c{c}"),
                    _ => String::new(), // 预检已挡参数残缺
                };
                let after = if changed {
                    match frag.as_deref() {
                        None => format!("removed {where_at}:{element}"),
                        Some(_) => format!("set {where_at}:{element}"),
                    }
                } else {
                    format!("{element} 不存在（空转，文档未变）")
                };
                plan[entry].summaries.push(AppliedOp {
                    op: "set_table_element",
                    block,
                    before: projected_of(&model, block),
                    after,
                    style: None,
                    style_unchanged: None,
                    target: None,
                });
            }
            EditOp::MergeCells { block, direction, row, cell, span: mspan, end_row, end_cell, .. } => {
                let span = spans[block - 1];
                let block_xml = &xml[span.start..span.end];
                let (new_xml, summary) =
                    merge_cells_xml(block_xml, direction, row, cell, mspan, end_row, end_cell)
                        .ok_or_else(|| AppError::Internal(format!(
                            "合并手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        )))?;
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: new_xml,
                    summaries: vec![AppliedOp {
                        op: "merge_cells",
                        block,
                        before: cell_projected_of(&model, block, row, cell),
                        after: summary,
                        style: None,
                        style_unchanged: None,
                        target: None,
                    }],
                });
            }
            EditOp::SplitCell { block, direction, row, cell, .. } => {
                let span = spans[block - 1];
                let block_xml = &xml[span.start..span.end];
                let (new_xml, summary) = split_cell_xml(block_xml, direction, row, cell)
                    .ok_or_else(|| AppError::Internal(format!(
                        "拆分手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                    )))?;
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: new_xml,
                    summaries: vec![AppliedOp {
                        op: "split_cell",
                        block,
                        before: cell_projected_of(&model, block, row, cell),
                        after: summary,
                        style: None,
                        style_unchanged: None,
                        target: None,
                    }],
                });
            }
            EditOp::DeleteTableRow { block, row, .. } => {
                let span = spans[block - 1];
                let block_xml = &xml[span.start..span.end];
                let (new_xml, summary) = delete_table_row_xml(block_xml, row)
                    .ok_or_else(|| AppError::Internal(format!(
                        "删行手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                    )))?;
                plan.push(Splice {
                    pos: span.start,
                    remove_end: span.end,
                    insert: new_xml,
                    summaries: vec![AppliedOp {
                        op: "delete_table_row",
                        block,
                        before: projected_of(&model, block),
                        after: summary,
                        style: None,
                        style_unchanged: None,
                        target: None,
                    }],
                });
            }
            EditOp::ClearBody { .. } => {
                // 单 splice 覆盖 [首块起点, 末块终点)，保留块原字节回填——
                // 含 sectPr 的块（节属性载体）不动，其余全删。独占一批（预检已验）。
                let kept: String = spans
                    .iter()
                    .map(|s| &xml[s.start..s.end])
                    .filter(|b| b.contains("<w:sectPr"))
                    .collect();
                let kept_n = spans
                    .iter()
                    .filter(|s| xml[s.start..s.end].contains("<w:sectPr"))
                    .count();
                let deleted_n = spans.len() - kept_n;
                let (pos, remove_end) = match (spans.first(), spans.last()) {
                    (Some(f), Some(l)) => (f.start, l.end),
                    // 空 body：无块可清——no-op splice 仍出摘要（applied 计数一致）
                    _ => (xml.len(), xml.len()),
                };
                plan.push(Splice {
                    pos,
                    remove_end,
                    insert: kept,
                    summaries: vec![AppliedOp {
                        op: "clear_body",
                        block: 0,
                        before: format!("{} 块", spans.len()),
                        after: format!("清空（删 {deleted_n} 块，保留 {kept_n} 个节属性块）"),
                        style: None,
                        style_unchanged: None,
                        target: None,
                    }],
                });
            }
        }
    }

    // ---- splice（按位置升序单 pass；区间互不重叠，见预检的占用规则）----
    plan.sort_by(|a, b| a.pos.cmp(&b.pos).then(a.remove_end.cmp(&b.remove_end)));
    let mut out = String::with_capacity(xml.len() + 256);
    let mut cursor = 0usize;
    for s in &plan {
        debug_assert!(s.pos >= cursor, "splice 区间重叠（内部 bug）");
        out.push_str(&xml[cursor..s.pos]);
        out.push_str(&s.insert);
        cursor = s.remove_end;
    }
    out.push_str(&xml[cursor..]);

    // 产物必须仍是合法 XML 且块数守恒（删 N 块插 M 块 → 总数变化可推算）
    let dom2 = super::xml_dom::parse(&out)?;
    let model2 = docx_model::build_document(&dom2);
    let inserted = ops
        .iter()
        .filter(|o| matches!(o, EditOp::InsertParagraphAfter { .. } | EditOp::InsertTableAfter { .. }))
        .count();
    let deleted = ops.iter().filter(|o| matches!(o, EditOp::DeleteBlock { .. })).count();
    let expect_blocks = if ops.iter().any(|o| matches!(o, EditOp::ClearBody { .. })) {
        // 独占一批：清空后 = 含 sectPr 的保留块数
        spans
            .iter()
            .filter(|s| xml[s.start..s.end].contains("<w:sectPr"))
            .count()
    } else {
        model.body.len() + inserted - deleted
    };
    if model2.body.len() != expect_blocks {
        return Err(AppError::Internal(format!(
            "手术后块数校验失败: 期望 {expect_blocks} 块，实际 {} 块（内部 bug，未写盘）",
            model2.body.len()
        )));
    }

    let applied = plan.into_iter().flat_map(|s| s.summaries).collect();
    Ok((out, applied))
}

fn expect_prefix_of(op: &EditOp) -> &str {
    match op {
        EditOp::ReplaceText { expect_prefix, .. }
        | EditOp::InsertParagraphAfter { expect_prefix, .. }
        | EditOp::DeleteBlock { expect_prefix, .. }
        | EditOp::SetStyle { expect_prefix, .. }
        | EditOp::SetFormat { expect_prefix, .. }
        | EditOp::SetPprElement { expect_prefix, .. }
        | EditOp::InsertTableAfter { expect_prefix, .. }
        | EditOp::SetCellText { expect_prefix, .. }
        | EditOp::InsertTableRowAfter { expect_prefix, .. }
        | EditOp::SetCellFormat { expect_prefix, .. }
        | EditOp::SetTableElement { expect_prefix, .. }
        | EditOp::MergeCells { expect_prefix, .. }
        | EditOp::SplitCell { expect_prefix, .. }
        | EditOp::DeleteTableRow { expect_prefix, .. } => expect_prefix,
        // ClearBody 无文本前缀——指纹走块数（预检先于本函数处理并 continue）
        EditOp::ClearBody { .. } => "",
    }
}

/// 表格格地址预检（set_cell_text / set_cell_format / set_table_element
/// level=cell 共用）：表格块边界、批内虚拟行边界、结构检查（续格指路合并头 /
/// 嵌套表 / 空段落——按来源行落到模板行）、(block, row, cell, target_key) 去重。
/// `target_key`（S3 七波）：set_table_element 传元素名——同格**不同元素**可同批
/// 组合（vAlign + tcBorders 一批，序无关）；内容/格式手术传 ""（重写语义，每格
/// 每批限一条）。`op_label` 仅用于非表格块报错文案。修订检查在调用方。
#[allow(clippy::too_many_arguments)]
fn precheck_cell_target(
    model: &docx_model::DocxDocument,
    idx: usize,
    block: usize,
    row: usize,
    cell: usize,
    op_label: &str,
    target_key: &str,
    table_state: &mut HashMap<usize, (Vec<usize>, Vec<Option<usize>>)>,
    used_cells: &mut Vec<(usize, usize, usize, String)>,
) -> AppResult<()> {
    let Block::Table(t) = &model.body[idx] else {
        return Err(AppError::Validation(format!(
            "非表格块: 块 {block} 是段落，{op_label} 只作用于表格块。\
             改段落文字用 replace_text；建表用 insert_table_after。"
        )));
    };
    // 批内虚拟行：同块先增行后填格合法（寻址按序生效）
    let (counts, tpls) = table_state.entry(block).or_insert_with(|| {
        (
            t.rows.iter().map(|r| r.cells.len()).collect::<Vec<_>>(),
            vec![None; t.rows.len()],
        )
    });
    if row == 0 || row > counts.len() {
        return Err(AppError::Validation(format!(
            "单元格越界: 块 {block} 第 {row} 行不存在（该表当前共 {} 行，1-based，\
             含本批已增行）。网格视图用 inspect_docx projection=table 复核行列号。",
            counts.len()
        )));
    }
    if cell == 0 || cell > counts[row - 1] {
        return Err(AppError::Validation(format!(
            "单元格越界: 块 {block} 第 {row} 行第 {cell} 格不存在（该行共 {} 格，\
             1-based 按网格显示顺序数，跨列格占 1 个序号）。\
             网格视图用 inspect_docx projection=table。",
            counts[row - 1]
        )));
    }
    // 结构检查按「来源行」：原行直查；本批增行查其模板行（克隆继承结构）
    let real_row = tpls[row - 1].unwrap_or(row);
    let r = &t.rows[real_row - 1];
    let c = &r.cells[cell - 1];
    if c.v_merge.as_deref() == Some("continue") {
        return Err(AppError::Validation(format!(
            "纵向合并续格: 块 {block} 第 {row} 行第 {cell} 格是纵向合并的续格（显示「(续)」），\
             其内容由上方合并头格统一持有。请对该列上方带「(合并头)」标记的格执行 {op_label}。"
        )));
    }
    if c.blocks.iter().any(|b| matches!(b, Block::Table(_))) {
        return Err(AppError::Validation(format!(
            "嵌套表: 块 {block} 第 {row} 行第 {cell} 格内含表格，暂不支持嵌套表编辑。\
             可用 delete_block 删整表后 insert_table_after 重建。"
        )));
    }
    if tpls[row - 1].is_none() && c.blocks.is_empty() {
        return Err(AppError::Validation(format!(
            "单元格结构异常: 块 {block} 第 {row} 行第 {cell} 格无段落，套不了格式模板。\
             请用 insert_table_row_after 重建该行。"
        )));
    }
    if used_cells
        .iter()
        .any(|(b, r, c, k)| *b == block && *r == row && *c == cell && k == target_key)
    {
        return Err(AppError::Validation(format!(
            "同一格多操作: 块 {block} 第 {row} 行第 {cell} 格{}在本批中被多次引用。\
             同格同目标每批限一条；同格多项表格属性用不同 element 的多条 \
             set_table_element 组合（如 vAlign + tcBorders 一批）。",
            if target_key.is_empty() { String::new() } else { format!("的 {target_key}") }
        )));
    }
    used_cells.push((block, row, cell, target_key.to_string()));
    Ok(())
}

/// 行号边界检查（merge/split 用——独占批无虚拟行，直接对模型行）。
fn row_bounds(t: &docx_model::Table, row: usize) -> AppResult<&docx_model::TableRow> {
    if row == 0 || row > t.rows.len() {
        return Err(AppError::Validation(format!(
            "行号越界: row={row} 不存在（该表共 {} 行，1-based）。\
             网格视图用 inspect_docx projection=table。",
            t.rows.len()
        )));
    }
    Ok(&t.rows[row - 1])
}

/// 第 `cell` 格（1-based）占据的网格列区间 [start, end)（0-based 网格列）。
/// 纵向合并的对齐判据：各行同区间的格才能合并（Word 同规）。
fn grid_range_of(row: &docx_model::TableRow, cell: usize) -> (u32, u32) {
    let mut start = 0u32;
    for (i, c) in row.cells.iter().enumerate() {
        let span = c.grid_span.unwrap_or(1);
        if i + 1 == cell {
            return (start, start + span);
        }
        start += span;
    }
    (start, start)
}

/// 行内占据恰好 [g0, g1) 网格列区间的格（None = 该行在此区间边界不对齐）。
fn cell_at_grid_range(
    row: &docx_model::TableRow,
    g0: u32,
    g1: u32,
) -> Option<&docx_model::TableCell> {
    let mut start = 0u32;
    for c in &row.cells {
        let span = c.grid_span.unwrap_or(1);
        if start == g0 && start + span == g1 {
            return Some(c);
        }
        start += span;
    }
    None
}

/// 块投影文本（前 60 字；与 inspect text 投影同口径）。
fn projected_of(model: &docx_model::DocxDocument, block: usize) -> String {
    let mut s = String::new();
    docx_model::blocks_text(&model.body[block - 1..block], &mut s);
    truncate(s.trim_end_matches('\n'), 60)
}

/// 块内是否含修订 run（w:ins / w:del）。
fn has_revision(block: &Block) -> bool {
    fn para_has_revision(runs: &[docx_model::Run]) -> bool {
        runs.iter().any(|r| r.revision.is_some())
    }
    match block {
        Block::Paragraph(p) => para_has_revision(&p.runs),
        Block::Table(t) => t.rows.iter().any(|row| {
            row.cells.iter().any(|c| c.blocks.iter().any(has_revision))
        }),
    }
}

// =========================================================================
// 块 XML 重建（pPr / rPr / 开标签属性全部原样字节切片保留）
// =========================================================================

/// 重建段落块：保留开标签属性 + pPr + 首个 run 的 rPr，正文换为 `new_text`。
/// `new_text = None` 表示删除整块（返回空串）。
fn rebuild_paragraph(xml: &str, span: BlockSpan, new_text: Option<&str>) -> String {
    let block_xml = &xml[span.start..span.end];
    let Some(text) = new_text else { return String::new() };

    // 开标签原样（`<w:p>` / 带 w14:paraId 等属性的 `<w:p …>`）；自闭合（`<w:p/>` /
    // `<w:p w14:paraId="X"/>`——Word 空段常态）去掉 "/>" 补 ">"，属性保留
    let open_tag = if let Some(stripped) = block_xml.strip_suffix("/>") {
        format!("{stripped}>")
    } else {
        let gt = block_xml.find('>').expect("块必含 '>'");
        block_xml[..=gt].to_string()
    };
    let ppr = slice_ppr(block_xml);
    let rpr = slice_first_run_rpr(block_xml);
    format!("{open_tag}{ppr}{}</w:p>", run_xml(&rpr, text))
}

/// 段落改样式手术：只动 pStyle 一个元素，pPr 其余子元素 / rPr / run 字节一概不碰。
/// 三形态：已有 pStyle（首个 = 生效样式，schema 序在 pPrChange 前）→ 整元素替换；
/// 有 pPr 无 pStyle → pStyle 是 pPr 的 schema 首子元素，插开标签后即正确位置；
/// 无 pPr → 开标签后新建（自闭合空段 `<w:p/>` 顺势展开成对标签）。
/// 返回 None 仅防御形态异常（调用方报内部错误）。
fn restyle_paragraph(block_xml: &str, style_id: &str) -> Option<String> {
    let tag = format!(r#"<w:pStyle w:val="{style_id}"/>"#);
    // 1) 已有 pStyle：定位首个（自闭合或配对形式均支持）
    if let Some(s) = block_xml.find("<w:pStyle") {
        let gt = s + block_xml[s..].find('>')? + 1; // '>' 后一位
        let e = if block_xml.as_bytes()[gt - 2] == b'/' {
            gt // 自闭合：元素止于 '/>'
        } else {
            gt + block_xml[gt..].find("</w:pStyle>")? + "</w:pStyle>".len()
        };
        return Some(format!("{}{tag}{}", &block_xml[..s], &block_xml[e..]));
    }
    // 2) 有 pPr：精确到 `<w:pPr` 后跟 '>' / '/' / 空格（排除 `<w:pPrChange`——字母 C 跟随）
    let ppr_at = block_xml.find("<w:pPr").filter(|s| {
        matches!(
            block_xml.as_bytes().get(s + "<w:pPr".len()),
            Some(b'>') | Some(b'/') | Some(b' ')
        )
    });
    if let Some(s) = ppr_at {
        let gt = s + block_xml[s..].find('>')? + 1;
        // 自闭合空 pPr（Word 不写，防御）：整体换成含 pStyle 的完整 pPr
        if block_xml.as_bytes()[gt - 2] == b'/' {
            return Some(format!(
                "{}<w:pPr>{tag}</w:pPr>{}",
                &block_xml[..s],
                &block_xml[gt..]
            ));
        }
        return Some(format!("{}{tag}{}", &block_xml[..gt], &block_xml[gt..]));
    }
    // 3) 无 pPr：开标签后新建；自闭合空段展开
    let gt = block_xml.find('>')?;
    if block_xml.as_bytes()[gt - 1] == b'/' {
        let head = &block_xml[..gt - 1]; // 去掉 '/'
        return Some(format!("{head}><w:pPr>{tag}</w:pPr></w:p>"));
    }
    Some(format!(
        "{}<w:pPr>{tag}</w:pPr>{}",
        &block_xml[..gt + 1],
        &block_xml[gt + 1..]
    ))
}

/// set_format / set_cell_format 值域校验（家族前缀稳定：空格式操作/对齐值无效/颜色值无效/…）。
///
/// `op_label` 进文案（报错指向具体操作名）；`style_present` = 调用方带样式参数
/// （set_cell_format 专属），此时 paragraph/character 全空也合法（纯换样式操作）。
fn validate_formats(
    para: Option<&ParaFormat>,
    ch: Option<&CharFormat>,
    style_present: bool,
    op_label: &str,
) -> AppResult<()> {
    let empty_para = para.is_none_or(|p| p.is_empty());
    let empty_ch = ch.is_none_or(|c| c.is_empty());
    if empty_para && empty_ch && !style_present {
        return Err(AppError::Validation(
            format!(
                "空格式操作: {op_label} 未提供任何要修改的字段。\
                 paragraph（对齐/行距/段前后/缩进）与 character（粗斜/字号/颜色/字体）\
                 至少一项内有字段。"
            ),
        ));
    }
    const ALIGNS: [&str; 7] = ["left", "center", "right", "both", "distribute", "start", "end"];
    if let Some(p) = para {
        if let Some(v) = &p.align {
            if !ALIGNS.contains(&v.as_str()) {
                return Err(AppError::Validation(format!(
                    "对齐值无效: {v:?}。可用 left/center/right/both(两端对齐)/distribute(分散对齐)/start/end。"
                )));
            }
        }
        if let Some(v) = p.line_spacing {
            if !v.is_finite() || v <= 0.0 {
                return Err(AppError::Validation(format!(
                    "行距值无效: {v}。应为正数倍数（1.5 = 1.5 倍行距）。"
                )));
            }
        }
        for (label, v) in [("段前", p.space_before_pt), ("段后", p.space_after_pt)] {
            if let Some(v) = v {
                if !v.is_finite() || !(0.0..=1000.0).contains(&v) {
                    return Err(AppError::Validation(format!(
                        "间距值无效: {label}={v}pt。合法范围 0-1000pt。"
                    )));
                }
            }
        }
        if let Some(v) = p.indent_first_line_tw {
            if v < 0 {
                return Err(AppError::Validation(format!(
                    "缩进值无效: 首行缩进 first_line={v}tw。应为非负 twips（1 字符 ≈ 240tw；\
                     悬挂缩进请在 Word 中或经样式设置）。"
                )));
            }
        }
    }
    if let Some(c) = ch {
        if let Some(v) = c.font_size_pt {
            if !v.is_finite() || !(1.0..=400.0).contains(&v) {
                return Err(AppError::Validation(format!(
                    "字号超出范围: {v}pt。合法范围 1-400pt。"
                )));
            }
        }
        if let Some(v) = &c.color {
            let ok = v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit());
            if !ok {
                return Err(AppError::Validation(format!(
                    "颜色值无效: {v:?}。应为 6 位十六进制 RRGGBB（如 FF0000 红色）。"
                )));
            }
        }
    }
    Ok(())
}

/// applied 摘要：本次应用的格式清单（如「段落(对齐=center 行距=1.5倍) 字符(粗 14pt)」）。
fn describe_formats(para: Option<&ParaFormat>, ch: Option<&CharFormat>) -> String {
    let mut parts = Vec::new();
    if let Some(p) = para {
        let mut seg = Vec::new();
        if let Some(v) = &p.align {
            seg.push(format!("对齐={v}"));
        }
        if let Some(v) = p.line_spacing {
            seg.push(format!("行距={v}倍"));
        }
        if let Some(v) = p.space_before_pt {
            seg.push(format!("段前={v}pt"));
        }
        if let Some(v) = p.space_after_pt {
            seg.push(format!("段后={v}pt"));
        }
        if let Some(v) = p.indent_first_line_tw {
            seg.push(format!("首行缩进={v}tw"));
        }
        if let Some(v) = p.indent_left_tw {
            seg.push(format!("左缩进={v}tw"));
        }
        if !seg.is_empty() {
            parts.push(format!("段落({})", seg.join(" ")));
        }
    }
    if let Some(c) = ch {
        let mut seg = Vec::new();
        if let Some(v) = c.bold {
            seg.push(if v { "加粗" } else { "取消加粗" }.to_string());
        }
        if let Some(v) = c.italic {
            seg.push(if v { "斜体" } else { "取消斜体" }.to_string());
        }
        if let Some(v) = c.font_size_pt {
            seg.push(format!("字号={v}pt"));
        }
        if let Some(v) = &c.color {
            seg.push(format!("颜色=#{v}"));
        }
        if let Some(v) = &c.font {
            seg.push(format!("字体={v}"));
        }
        if !seg.is_empty() {
            parts.push(format!("字符({})", seg.join(" ")));
        }
    }
    if parts.is_empty() {
        "（无字段）".into()
    } else {
        parts.join(" ")
    }
}

// =========================================================================
// set_format 手术辅助（元素定位 / 属性合并 / schema 序插入）
// =========================================================================

/// 在 `s` 中找名为 `w:{name}` 的元素字节区间（`[start, end)`，自闭合或配对形式）。
/// 撞名前缀（找 w:b 会先撞 w:bCs / w:bdr）按「后随字符是字母则跳过继续找」排除。
/// None = 不存在。
pub(super) fn find_element_span(s: &str, name: &str) -> Option<(usize, usize)> {
    let pat = format!("<w:{name}");
    let mut from = 0usize;
    loop {
        let rel = s[from..].find(&pat)?;
        let start = from + rel;
        let after = start + pat.len();
        let next_ch = s[after..].chars().next()?;
        if next_ch.is_ascii_alphanumeric() {
            from = after; // w:b 撞上 w:bCs——继续找下一个出现
            continue;
        }
        // 开标签止于 '>'（自闭合时前一字符是 '/'）
        let gt = after + s[after..].find('>')?;
        let end = if s.as_bytes()[gt - 1] == b'/' {
            gt + 1
        } else {
            let close = format!("</w:{name}>");
            gt + 1 + s[gt + 1..].find(&close)? + close.len()
        };
        return Some((start, end));
    }
}

/// 解析元素开标签的属性表（`name="value"` 对；仅到首个 '>'）。
pub(super) fn parse_attrs(el: &str) -> Vec<(String, String)> {
    let gt = el.find('>').unwrap_or(el.len());
    let tag = &el[..gt];
    let mut out = Vec::new();
    let mut rest = tag;
    while let Some(eq) = rest.find('=') {
        let head = &rest[..eq];
        let name = head.trim_end().rsplit(char::is_whitespace).next().unwrap_or("").to_string();
        let after = &rest[eq + 1..];
        let quote = after.chars().next();
        if quote != Some('"') {
            rest = &rest[eq + 1..];
            continue;
        }
        let Some(end_rel) = after[1..].find('"') else { break };
        let value = after[1..1 + end_rel].to_string();
        if !name.is_empty() {
            out.push((name, value));
        }
        rest = &after[end_rel + 2..];
    }
    out
}

/// 由属性表构造标准自闭合标签（属性序 = 传入序）。
pub(super) fn build_tag(name: &str, attrs: &[(String, String)]) -> String {
    if attrs.is_empty() {
        return format!("<w:{name}/>");
    }
    let inner: Vec<String> = attrs.iter().map(|(k, v)| format!(r#"{k}="{v}""#)).collect();
    format!("<w:{name} {}/>", inner.join(" "))
}

/// 属性表 set/remove（原地）。
fn attr_set(attrs: &mut Vec<(String, String)>, key: &str, val: &str) {
    if let Some(a) = attrs.iter_mut().find(|(k, _)| k == key) {
        a.1 = val.to_string();
    } else {
        attrs.push((key.to_string(), val.to_string()));
    }
}
fn attr_remove(attrs: &mut Vec<(String, String)>, key: &str) {
    attrs.retain(|(k, _)| k != key);
}

/// 在父元素内容里替换或按 schema 序插入一个属性元素。
/// - 已存在 → 整元素替换为 `new_tag`
/// - 不存在 → 插到 `later`（schema 序中排在后面的兄弟元素名）最早出现处之前，
///   或 `end_marker`（如 `</w:pPr>`）之前
pub(super) fn upsert_element(parent: &str, name: &str, new_tag: &str, later: &[&str], end_marker: &str) -> String {
    if let Some((s, e)) = find_element_span(parent, name) {
        return format!("{}{new_tag}{}", &parent[..s], &parent[e..]);
    }
    let mut insert_at = parent.find(end_marker).unwrap_or(parent.len());
    for later_name in later {
        if let Some(p) = parent.find(&format!("<w:{later_name}")) {
            insert_at = insert_at.min(p);
        }
    }
    format!("{}{new_tag}{}", &parent[..insert_at], &parent[insert_at..])
}

/// pPr 内容里应用段落格式（spacing/ind 属性级合并——未提及的属性原样保留）。
fn apply_para_formats(ppr_inner: &str, p: &ParaFormat) -> String {
    let mut out = ppr_inner.to_string();
    // spacing（行距/段前后共用一个元素，必须合并不是覆盖）
    if p.line_spacing.is_some() || p.space_before_pt.is_some() || p.space_after_pt.is_some() {
        let mut attrs = match find_element_span(&out, "spacing") {
            Some((s, e)) => parse_attrs(&out[s..e]),
            None => Vec::new(),
        };
        if let Some(v) = p.line_spacing {
            attr_set(&mut attrs, "w:line", &format!("{}", (v * 240.0).round() as i64));
            attr_set(&mut attrs, "w:lineRule", "auto");
        }
        if let Some(v) = p.space_before_pt {
            attr_set(&mut attrs, "w:before", &format!("{}", (v * 20.0).round() as i64));
            attr_remove(&mut attrs, "w:beforeLines");
            attr_remove(&mut attrs, "w:beforeAutospacing");
        }
        if let Some(v) = p.space_after_pt {
            attr_set(&mut attrs, "w:after", &format!("{}", (v * 20.0).round() as i64));
            attr_remove(&mut attrs, "w:afterLines");
            attr_remove(&mut attrs, "w:afterAutospacing");
        }
        let tag = build_tag("spacing", &attrs);
        out = upsert_element(&out, "spacing", &tag, &["ind", "jc", "rPr", "sectPr", "pPrChange"], "</w:pPr>");
    }
    // ind（首行/左缩进合并；两变体互斥需清对方）
    if p.indent_first_line_tw.is_some() || p.indent_left_tw.is_some() {
        let mut attrs = match find_element_span(&out, "ind") {
            Some((s, e)) => parse_attrs(&out[s..e]),
            None => Vec::new(),
        };
        if let Some(v) = p.indent_first_line_tw {
            // 跨层压制（2026-08-26 生产反馈缺口 2）：chars 单位变体不能只删——直接层
            // 删掉后样式层（如正文样式的 firstLineChars=200）会透出，工具报成功但
            // 缩进仍在。显式写 0 才是「直接层覆盖样式层」的语义（chars 单位优先于
            // twips）。零值连 hanging 系一并压零——任意元素内优先序下都渲染无缩进；
            // 非零值不写 hanging=0（同元素内 hanging 会压掉 firstLine，反伤目标）
            attr_set(&mut attrs, "w:firstLine", &v.to_string());
            attr_set(&mut attrs, "w:firstLineChars", "0");
            if v == 0 {
                attr_set(&mut attrs, "w:hanging", "0");
                attr_set(&mut attrs, "w:hangingChars", "0");
            } else {
                for k in ["w:hanging", "w:hangingChars"] {
                    attr_remove(&mut attrs, k);
                }
            }
        }
        if let Some(v) = p.indent_left_tw {
            // leftChars 同理显式写 0 压样式层（left 无互斥变体，恒安全）
            attr_set(&mut attrs, "w:left", &v.to_string());
            attr_set(&mut attrs, "w:leftChars", "0");
        }
        let tag = build_tag("ind", &attrs);
        out = upsert_element(&out, "ind", &tag, &["jc", "rPr", "sectPr", "pPrChange"], "</w:pPr>");
    }
    // jc（单属性，整元素替换/插入）
    if let Some(v) = &p.align {
        let tag = format!(r#"<w:jc w:val="{v}"/>"#);
        out = upsert_element(&out, "jc", &tag, &["textDirection", "outlineLvl", "rPr", "sectPr", "pPrChange"], "</w:pPr>");
    }
    out
}

/// rPr 内容里应用字符格式（b/bCs、i/iCs、sz/szCs 成对；color 单元素；rFonts 合并）。
fn apply_char_formats(rpr_inner: &str, c: &CharFormat) -> String {
    let mut out = rpr_inner.to_string();
    if let Some(v) = c.bold {
        let tag = if v { "<w:b/>".to_owned() } else { r#"<w:b w:val="0"/>"#.to_owned() };
        out = upsert_element(&out, "b", &tag, &["i", "color", "sz", "u"], "</w:rPr>");
        let tag_cs = if v { "<w:bCs/>".to_owned() } else { r#"<w:bCs w:val="0"/>"#.to_owned() };
        out = upsert_element(&out, "bCs", &tag_cs, &["i", "color", "sz", "u"], "</w:rPr>");
    }
    if let Some(v) = c.italic {
        let tag = if v { "<w:i/>".to_owned() } else { r#"<w:i w:val="0"/>"#.to_owned() };
        out = upsert_element(&out, "i", &tag, &["color", "sz", "u"], "</w:rPr>");
        let tag_cs = if v { "<w:iCs/>".to_owned() } else { r#"<w:iCs w:val="0"/>"#.to_owned() };
        out = upsert_element(&out, "iCs", &tag_cs, &["color", "sz", "u"], "</w:rPr>");
    }
    if let Some(v) = c.font_size_pt {
        let half = (v * 2.0).round() as i64;
        out = upsert_element(
            &out,
            "sz",
            &format!(r#"<w:sz w:val="{half}"/>"#),
            &["szCs", "highlight", "u"],
            "</w:rPr>",
        );
        out = upsert_element(
            &out,
            "szCs",
            &format!(r#"<w:szCs w:val="{half}"/>"#),
            &["highlight", "u"],
            "</w:rPr>",
        );
    }
    if let Some(v) = &c.color {
        out = upsert_element(
            &out,
            "color",
            &format!(r#"<w:color w:val="{v}"/>"#),
            &["sz", "u"],
            "</w:rPr>",
        );
    }
    if let Some(v) = &c.font {
        let mut attrs = match find_element_span(&out, "rFonts") {
            Some((s, e)) => parse_attrs(&out[s..e]),
            None => Vec::new(),
        };
        attr_set(&mut attrs, "w:ascii", v);
        attr_set(&mut attrs, "w:hAnsi", v);
        attr_set(&mut attrs, "w:eastAsia", v);
        let tag = build_tag("rFonts", &attrs);
        out = upsert_element(&out, "rFonts", &tag, &["b", "i", "color", "sz", "u"], "</w:rPr>");
    }
    out
}

/// 全新 pPr 内容（无 pPr / 自闭合空 pPr 两分支共用；schema 序 spacing→ind→jc）。
fn fresh_ppr_inner(p: &ParaFormat) -> String {
    let mut inner = String::new();
    let mut attrs: Vec<(String, String)> = Vec::new();
    if let Some(v) = p.line_spacing {
        attr_set(&mut attrs, "w:line", &format!("{}", (v * 240.0).round() as i64));
        attr_set(&mut attrs, "w:lineRule", "auto");
    }
    if let Some(v) = p.space_before_pt {
        attr_set(&mut attrs, "w:before", &format!("{}", (v * 20.0).round() as i64));
    }
    if let Some(v) = p.space_after_pt {
        attr_set(&mut attrs, "w:after", &format!("{}", (v * 20.0).round() as i64));
    }
    if !attrs.is_empty() {
        inner.push_str(&build_tag("spacing", &attrs));
    }
    let mut ind_attrs: Vec<(String, String)> = Vec::new();
    if let Some(v) = p.indent_first_line_tw {
        // 跨层压制同 apply_para_formats（缺口 2）：chars 变体显式写 0——无 pPr 的
        // 格内段落正是生产踩空形态（样式层 firstLineChars 透出）；零值连 hanging
        // 系一并压零，非零只压 chars（元素内 hanging 优先 firstLine，写 0 反伤）
        ind_attrs.push(("w:firstLine".into(), v.to_string()));
        ind_attrs.push(("w:firstLineChars".into(), "0".into()));
        if v == 0 {
            ind_attrs.push(("w:hanging".into(), "0".into()));
            ind_attrs.push(("w:hangingChars".into(), "0".into()));
        }
    }
    if let Some(v) = p.indent_left_tw {
        ind_attrs.push(("w:left".into(), v.to_string()));
        ind_attrs.push(("w:leftChars".into(), "0".into()));
    }
    if !ind_attrs.is_empty() {
        inner.push_str(&build_tag("ind", &ind_attrs));
    }
    if let Some(v) = &p.align {
        inner.push_str(&format!(r#"<w:jc w:val="{v}"/>"#));
    }
    inner
}

/// 段落格式手术：定位/新建 pPr，应用段落格式；其余字节不动。
/// None 仅防御形态异常。
fn reformat_ppr(block_xml: &str, p: &ParaFormat) -> Option<String> {
    // 定位 pPr（精确 `<w:pPr` + 非字母后随，排除 pPrChange）
    let ppr_at = block_xml.find("<w:pPr").filter(|s| {
        matches!(
            block_xml.as_bytes().get(s + "<w:pPr".len()),
            Some(b'>') | Some(b'/') | Some(b' ')
        )
    });
    let Some(s) = ppr_at else {
        // 无 pPr：开标签后新建（自闭合空段顺势展开成对标签）
        let gt = block_xml.find('>')?;
        let new_ppr = format!("<w:pPr>{}</w:pPr>", fresh_ppr_inner(p));
        if block_xml.as_bytes()[gt - 1] == b'/' {
            let head = &block_xml[..gt - 1];
            return Some(format!("{head}>{new_ppr}</w:p>"));
        }
        return Some(format!("{}{new_ppr}{}", &block_xml[..gt + 1], &block_xml[gt + 1..]));
    };
    // 有 pPr：内容区间内应用
    let open_end = s + block_xml[s..].find('>')? + 1;
    if block_xml.as_bytes()[open_end - 2] == b'/' {
        // 自闭合空 pPr（防御）：整体替换为含格式的完整 pPr
        return Some(format!(
            "{}<w:pPr>{}</w:pPr>{}",
            &block_xml[..s],
            fresh_ppr_inner(p),
            &block_xml[open_end..]
        ));
    }
    let close_rel = block_xml[open_end..].find("</w:pPr>")?;
    let close_at = open_end + close_rel;
    let inner = &block_xml[open_end..close_at];
    let new_inner = apply_para_formats(inner, p);
    Some(format!(
        "{}{}{}",
        &block_xml[..open_end],
        new_inner,
        &block_xml[close_at..]
    ))
}

/// 校验属性容器子元素片段（set_ppr_element / set_table_element 的 xml 参数）：
/// - 单根：首个元素必须是 `<w:{element}`，闭合后只允许空白
/// - well-formed：quick-xml 全程无错、深度归零
/// - 不得携带 xmlns 声明（w 前缀已在文档根声明；新声明可能指向不同 URI）
/// - 不得内嵌 sectPr/pPrChange（受保护元素防夹带）
///
/// `source_hint` = 抄原文的投影档位名（报错指路用）。
pub(super) fn validate_fragment(element: &str, xml: &str, source_hint: &str) -> AppResult<()> {
    if xml.contains("xmlns") {
        return Err(AppError::Validation(
            "片段校验失败: 不得携带 xmlns 声明（w 前缀已在文档根声明）。请去掉 xmlns 属性。".into(),
        ));
    }
    if xml.contains("<w:sectPr") || xml.contains("<w:pPrChange") {
        return Err(AppError::Validation(
            "片段校验失败: 不得包含 sectPr/pPrChange（分节符载体/修订记录，受保护）。".into(),
        ));
    }
    let expected_root = format!("w:{element}");
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut depth = 0i32;
    let mut root_seen = false;
    let mut trailing = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if depth == 0 {
                    if trailing || root_seen {
                        return Err(AppError::Validation(
                            "片段校验失败: 必须恰好一个根元素（不得平铺多个）。".into(),
                        ));
                    }
                    root_seen = true;
                    if e.name().as_ref() != expected_root.as_bytes() {
                        return Err(AppError::Validation(format!(
                            "片段校验失败: 根元素必须是 <w:{element}>（与 element 参数一致），\
                             实际是 <{}>。请从 inspect_docx projection={source_hint} 看到的\
                             原文复制后修改。",
                            String::from_utf8_lossy(e.name().as_ref())
                        )));
                    }
                }
                depth += 1;
            }
            Ok(Event::Empty(e)) => {
                if depth == 0 {
                    if trailing || root_seen {
                        return Err(AppError::Validation(
                            "片段校验失败: 必须恰好一个根元素（不得平铺多个）。".into(),
                        ));
                    }
                    root_seen = true;
                    if e.name().as_ref() != expected_root.as_bytes() {
                        return Err(AppError::Validation(format!(
                            "片段校验失败: 根元素必须是 <w:{element}>（与 element 参数一致），\
                             实际是 <{}>。请从 inspect_docx projection={source_hint} 看到的\
                             原文复制后修改。",
                            String::from_utf8_lossy(e.name().as_ref())
                        )));
                    }
                }
                // 自闭合不入深度
            }
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    trailing = true; // 根已闭合，之后只允许空白
                }
            }
            Ok(Event::Text(t)) => {
                if !t.iter().all(|b| b.is_ascii_whitespace()) && (depth == 0 || trailing) {
                    return Err(AppError::Validation(
                        "片段校验失败: 根元素闭合后不得再有文本。".into(),
                    ));
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(AppError::Validation(format!(
                    "片段校验失败: XML 不合法（{e}）。请从 inspect_docx projection={source_hint} \
                     看到的原文复制后修改，不要凭记忆手写。"
                )));
            }
        }
    }
    if !root_seen {
        return Err(AppError::Validation(format!(
            "片段校验失败: 缺少根元素 <w:{element}>。"
        )));
    }
    if depth != 0 {
        return Err(AppError::Validation(
            "片段校验失败: 元素未闭合（标签不配对）。".into(),
        ));
    }
    Ok(())
}

/// 通用 pPr 子元素手术（纯函数）。
/// - xml=None：移除 `<w:{element}>` 整元素；pPr 摘空（无其它子元素）则 pPr 整体清理
/// - xml=Some：整元素替换（已存在）或按 CT_PPr schema 序插入（不存在）；
///   无 pPr 时新建（自闭合空段顺势展开成对标签）
///
/// 返回 (新块 XML, 是否实际改动)。None 仅防御形态异常。
fn set_ppr_element(block_xml: &str, element: &str, xml: Option<&str>) -> Option<(String, bool)> {
    let frag = match xml {
        Some(f) => f,
        None => return remove_ppr_element(block_xml, element),
    };
    // 定位 pPr（精确 `<w:pPr` + 非字母后随，排除 pPrChange）
    let ppr_at = block_xml.find("<w:pPr").filter(|s| {
        matches!(
            block_xml.as_bytes().get(s + "<w:pPr".len()),
            Some(b'>') | Some(b'/') | Some(b' ')
        )
    });
    let Some(s) = ppr_at else {
        // 无 pPr：开标签后新建（自闭合空段顺势展开）
        let gt = block_xml.find('>')?;
        let new_ppr = format!("<w:pPr>{frag}</w:pPr>");
        if block_xml.as_bytes()[gt - 1] == b'/' {
            let head = &block_xml[..gt - 1];
            return Some((format!("{head}>{new_ppr}</w:p>"), true));
        }
        return Some((
            format!("{}{new_ppr}{}", &block_xml[..gt + 1], &block_xml[gt + 1..]),
            true,
        ));
    };
    let open_end = s + block_xml[s..].find('>')? + 1;
    if block_xml.as_bytes()[open_end - 2] == b'/' {
        // 自闭合空 pPr：整体替换为含片段的完整 pPr
        return Some((
            format!("{}<w:pPr>{frag}</w:pPr>{}", &block_xml[..s], &block_xml[open_end..]),
            true,
        ));
    }
    // later = schema 序中排在 element 之后的兄弟名（upsert_element 的插入位依据）
    let idx = PPR_ELEMENTS.iter().position(|n| *n == element)?;
    let later: Vec<&str> = PPR_ELEMENTS[idx + 1..].to_vec();
    let ppr_end = open_end + block_xml[open_end..].find("</w:pPr>")? + "</w:pPr>".len();
    let ppr = &block_xml[s..ppr_end];
    let new_ppr = upsert_element(ppr, element, frag, &later, "</w:pPr>");
    Some((
        format!("{}{new_ppr}{}", &block_xml[..s], &block_xml[ppr_end..]),
        true,
    ))
}

/// 移除 pPr 子元素；pPr 摘空则整体清理。元素不存在 → 空转（文档不变）。
fn remove_ppr_element(block_xml: &str, element: &str) -> Option<(String, bool)> {
    let s = block_xml.find("<w:pPr").filter(|s| {
        matches!(
            block_xml.as_bytes().get(s + "<w:pPr".len()),
            Some(b'>') | Some(b'/') | Some(b' ')
        )
    })?;
    let open_end = s + block_xml[s..].find('>')? + 1;
    if block_xml.as_bytes()[open_end - 2] == b'/' {
        // 自闭合空 pPr：无任何子元素，必空转
        return Some((block_xml.to_string(), false));
    }
    let close_rel = block_xml[open_end..].find("</w:pPr>")?;
    let close_at = open_end + close_rel;
    let inner = &block_xml[open_end..close_at];
    let Some((es, ee)) = find_element_span(inner, element) else {
        return Some((block_xml.to_string(), false)); // 不存在 → 空转
    };
    let new_inner = format!("{}{}", &inner[..es], &inner[ee..]);
    if new_inner.trim().is_empty() {
        // 摘空 → pPr 整体移除（Word 自身也这样清理）
        return Some((
            format!("{}{}", &block_xml[..s], &block_xml[close_at + "</w:pPr>".len()..]),
            true,
        ));
    }
    Some((
        format!(
            "{}{}{}",
            &block_xml[..open_end],
            new_inner,
            &block_xml[close_at..]
        ),
        true,
    ))
}

/// 段落的样式链是否定义编号（w:style/w:pPr/w:numPr）。段级 numPr 摘除后
/// 编号回退到样式定义——警告判定用。
fn style_chain_defines_numbering(
    model: &docx_model::DocxDocument,
    block: usize,
    styles: &Stylesheet,
) -> bool {
    match &model.body[block - 1] {
        Block::Paragraph(p) => p
            .props
            .style
            .as_deref()
            .is_some_and(|id| styles.chain_defines_numbering(id)),
        Block::Table(_) => false,
    }
}

/// 字符格式手术：块内每个 run（含 hyperlink 内）的 rPr 应用字符格式；
/// 无 rPr 的 run 在开标签后新建（rPr 是 w:r 的 schema 首子元素）。
/// None 仅防御形态异常。
fn reformat_runs(block_xml: &str, c: &CharFormat) -> Option<String> {
    // 收集 run 开标签区间（"<w:r>" / "<w:r "）——倒序应用使偏移始终有效。
    // "<w:rPr" / "<w:rFonts" 等因后随字母不会被 "<w:r " / "<w:r>" 误配。
    let mut run_starts: Vec<usize> = Vec::new();
    let mut from = 0usize;
    loop {
        let rel_a = block_xml[from..].find("<w:r>").map(|p| (from + p, "<w:r>".len()));
        let rel_b = block_xml[from..].find("<w:r ").map(|p| (from + p, "<w:r ".len()));
        let hit = match (rel_a, rel_b) {
            (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
            (a, b) => a.or(b),
        };
        let Some((pos, len)) = hit else { break };
        run_starts.push(pos);
        from = pos + len;
    }
    let mut out = block_xml.to_string();
    for &rs in run_starts.iter().rev() {
        // 本轮处理后 out 与 block_xml 等长前缀可能已变——倒序应用时后续（更小的）
        // 偏移仍未被触碰。但 rs 是 block_xml 偏移，out 只在更靠后的位置变过，
        // 因此 rs 在 out 中仍有效。
        let open_end = rs + out[rs..].find('>')? + 1;
        // 直接子 rPr：紧跟开标签
        if out[open_end..].starts_with("<w:rPr>") {
            let close = open_end + "<w:rPr>".len();
            let inner_end = close + out[close..].find("</w:rPr>")?;
            let new_inner = apply_char_formats(&out[close..inner_end], c);
            out.replace_range(close..inner_end, &new_inner);
        } else if out[open_end..].starts_with("<w:rPr ") || out[open_end..].starts_with("<w:rPr/") {
            // 带属性/自闭合的 rPr 罕见：保守跳过该 run（不动比错动好）
            continue;
        } else {
            // 无 rPr：新建（rPr 是 w:r 的 schema 首子元素；空内容起手 = 全新插入，
            // apply_char_formats 的追加路径天然按 schema 序生成）
            let inner = apply_char_formats("", c);
            if !inner.is_empty() {
                let fresh = format!("<w:rPr>{inner}</w:rPr>");
                out.insert_str(open_end, &fresh);
            }
        }
    }
    Some(out)
}

/// 构造插入段：style 显示名 → pPr；缺省继承锚块 pPr。rPr 继承锚块首 run
/// （锚块含修订时不继承——修订 run 的格式不作为模板）。
fn build_inserted_paragraph(
    xml: &str,
    anchor: BlockSpan,
    text: &str,
    style: Option<&str>,
    styles: &Stylesheet,
    anchor_has_revision: bool,
) -> String {
    let anchor_xml = &xml[anchor.start..anchor.end];
    let (ppr, rpr) = match style.and_then(|n| styles.id_of(n)) {
        Some(style_id) => (
            format!(r#"<w:pPr><w:pStyle w:val="{style_id}"/></w:pPr>"#),
            String::new(),
        ),
        None => {
            let ppr = slice_ppr(anchor_xml).to_string();
            let rpr = if anchor_has_revision || anchor.is_table {
                String::new()
            } else {
                slice_first_run_rpr(anchor_xml)
            };
            (ppr, rpr)
        }
    };
    format!("<w:p>{ppr}{}</w:p>", run_xml(&rpr, text))
}

/// 切出块内的 `<w:pPr>…</w:pPr>` 原样字节（无则空串）。
fn slice_ppr(block_xml: &str) -> &str {
    match block_xml.find("<w:pPr>") {
        Some(s) => match block_xml[s..].find("</w:pPr>") {
            Some(e_rel) => &block_xml[s..s + e_rel + "</w:pPr>".len()],
            None => "",
        },
        None => "",
    }
}

/// 切出首个 run 的 `<w:rPr>…</w:rPr>`（在 pPr 之后找，避开段落标记字符格式）。
fn slice_first_run_rpr(block_xml: &str) -> String {
    let search_from = match block_xml.find("</w:pPr>") {
        Some(e) => e + "</w:pPr>".len(),
        // 无 pPr：从开标签后开始
        None => block_xml.find('>').map(|g| g + 1).unwrap_or(0),
    };
    let hay = &block_xml[search_from..];
    match hay.find("<w:rPr>") {
        Some(s) => match hay[s..].find("</w:rPr>") {
            Some(e_rel) => hay[s..s + e_rel + "</w:rPr>".len()].to_string(),
            None => String::new(),
        },
        None => String::new(),
    }
}

/// 构造 run XML：普通文本段 / 换行（\n → w:br）/ 制表符（\t → w:tab）交替。
fn run_xml(rpr: &str, text: &str) -> String {
    let mut s = String::from("<w:r>");
    s.push_str(rpr);
    let mut buf = String::new();
    let flush = |buf: &mut String, s: &mut String| {
        if !buf.is_empty() {
            s.push_str(r#"<w:t xml:space="preserve">"#);
            escape_into(buf, s);
            s.push_str("</w:t>");
            buf.clear();
        }
    };
    for ch in text.chars() {
        match ch {
            '\n' => {
                flush(&mut buf, &mut s);
                s.push_str("<w:br/>");
            }
            '\t' => {
                flush(&mut buf, &mut s);
                s.push_str("<w:tab/>");
            }
            _ => buf.push(ch),
        }
    }
    flush(&mut buf, &mut s);
    s.push_str("</w:r>");
    s
}

/// XML 文本节点转义（& < >；其余字符合法直接输出）。
fn escape_into(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

pub(super) fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        s.chars().take(max_chars).collect()
    }
}

// =========================================================================
// 表格手术（insert_table_after / set_cell_text / insert_table_row_after）
// =========================================================================

/// 建表规模上限（防 agent 一次生成巨型表；超限走增行分批）。
const TABLE_MAX_ROWS: usize = 200;
const TABLE_MAX_COLS: usize = 30;

/// rows 矩阵校验：非空 / 行列上限 / 矩形一致。全挂「表格数据无效」家族前缀
/// （doom_loop 错误签名按家族前缀聚合，勿混入具体格内容）。
fn validate_table_rows(rows: &[Vec<String>]) -> AppResult<()> {
    let Some(first) = rows.first() else {
        return Err(AppError::Validation(
            "表格数据无效: rows 为空，至少一行（首行默认表头）".into(),
        ));
    };
    if rows.len() > TABLE_MAX_ROWS {
        return Err(AppError::Validation(format!(
            "表格数据无效: 行数 {} 超上限 {TABLE_MAX_ROWS}——先建前 {TABLE_MAX_ROWS} 行，余下 insert_table_row_after 分批增补",
            rows.len()
        )));
    }
    let cols = first.len();
    if cols == 0 || cols > TABLE_MAX_COLS {
        return Err(AppError::Validation(format!(
            "表格数据无效: 列数 {cols} 越界（每行须 1..={TABLE_MAX_COLS} 格）"
        )));
    }
    for (i, r) in rows.iter().enumerate() {
        if r.len() != cols {
            return Err(AppError::Validation(format!(
                "表格数据无效: 第 {} 行 {} 格与首行 {cols} 格不一致——rows 须矩形矩阵（格内多段用 \\n 表达，勿多造行）",
                i + 1,
                r.len()
            )));
        }
    }
    Ok(())
}

/// 节内容宽度（twips）：取最后出现的页宽与左右边距（缺省 1 英寸 = 1440），
/// 页宽全缺省按 Letter 12240；结果下限钳 2000（防 0/负），再低回落 9026。
fn content_width_twips(model: &docx_model::DocxDocument) -> u32 {
    let mut page_w: Option<u32> = None;
    let mut margin_l = 1440u32;
    let mut margin_r = 1440u32;
    for s in &model.sections {
        if let Some(w) = s.page_w {
            page_w = Some(w);
        }
        if let Some(m) = s.margin_left {
            margin_l = m;
        }
        if let Some(m) = s.margin_right {
            margin_r = m;
        }
    }
    let w = page_w.unwrap_or(12240).saturating_sub(margin_l).saturating_sub(margin_r);
    if w < 2000 { 9026 } else { w }
}

/// 建整表 XML：tblW pct 5000（100% 宽）+ 全边框 single sz=4 + 列宽均分；
/// header=true 时首行加粗 + tblHeader（跨页重复表头）。style_id 给定时 tblPr
/// 首位注 `<w:tblStyle>`（CT_TblPrBase schema 首子元素——用户模板表样式优先于
/// 默认边框，Word 按样式定义渲染条纹带/边框）。
fn build_table_xml(rows: &[Vec<String>], header: bool, style_id: Option<&str>, width_tw: u32) -> String {
    let cols = rows.first().map(|r| r.len()).unwrap_or(1).max(1);
    let col_w = (width_tw / cols as u32).max(200);
    let mut s = String::with_capacity(64 + rows.len() * cols * 48);
    s.push_str(r#"<w:tbl><w:tblPr>"#);
    if let Some(sid) = style_id {
        s.push_str(&format!(r#"<w:tblStyle w:val="{sid}"/>"#));
    }
    s.push_str(r#"<w:tblW w:w="5000" w:type="pct"/><w:tblBorders>"#);
    for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
        s.push_str(&format!(
            r#"<w:{side} w:val="single" w:sz="4" w:space="0" w:color="auto"/>"#
        ));
    }
    s.push_str("</w:tblBorders></w:tblPr><w:tblGrid>");
    for _ in 0..cols {
        s.push_str(&format!(r#"<w:gridCol w:w="{col_w}"/>"#));
    }
    s.push_str("</w:tblGrid>");
    for (ri, row) in rows.iter().enumerate() {
        s.push_str("<w:tr>");
        let is_header = header && ri == 0;
        if is_header {
            s.push_str("<w:trPr><w:tblHeader/></w:trPr>");
        }
        for cell in row {
            s.push_str(&format!(
                r#"<w:tc><w:tcPr><w:tcW w:w="{col_w}" w:type="dxa"/></w:tcPr>{}"#,
                cell_paragraphs_xml(cell, is_header)
            ));
            s.push_str("</w:tc>");
        }
        s.push_str("</w:tr>");
    }
    s.push_str("</w:tbl>");
    s
}

/// 单元格文本 → 段落序列：空文本 = 单空段；`\n` 分段；表头行加粗。
fn cell_paragraphs_xml(text: &str, bold: bool) -> String {
    let rpr = if bold { "<w:rPr><w:b/></w:rPr>" } else { "" };
    if text.is_empty() {
        return "<w:p/>".to_string();
    }
    let mut s = String::new();
    for line in text.split('\n') {
        s.push_str(&format!("<w:p>{}</w:p>", run_xml(rpr, line)));
    }
    s
}

/// `s` 内 `w:{name}` **直接子元素**的字节范围列表（相对 `s`；更深层同名忽略）。
/// 表格结构手术专用：行/格定位必须抗嵌套表（tc 内 w:tbl 含同名 w:tr/w:tc）。
/// 形态异常（扫描 Err / 未闭合）返回空表——调用方按手术失败处理，不写盘。
pub(super) fn direct_children_spans(s: &str, name: &str) -> Vec<(usize, usize)> {
    let mut reader = Reader::from_str(s);
    reader.config_mut().trim_text(false); // 偏移与原始字节对齐
    let target = format!("w:{name}");
    let mut spans = Vec::new();
    let mut depth = 0usize; // 当前嵌套在目标元素内的层数
    let mut direct_start: Option<usize> = None; // 最外层（直接子级）起点
    let mut balanced = true;
    loop {
        // buffer_position = 上一事件结束处 = 本事件开始处
        let ev_start = reader.buffer_position() as usize;
        let ev = match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };
        let ev_end = reader.buffer_position() as usize;
        match ev {
            Event::Start(e) if e.name().as_ref() == target.as_bytes() => {
                if depth == 0 {
                    direct_start = Some(ev_start);
                }
                depth += 1;
            }
            Event::End(e) if e.name().as_ref() == target.as_bytes() => {
                if depth == 0 {
                    balanced = false; // 多闭：形态异常
                    break;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = direct_start.take() {
                        spans.push((start, ev_end));
                    }
                }
            }
            Event::Empty(e) if e.name().as_ref() == target.as_bytes() && depth == 0 => {
                spans.push((ev_start, ev_end));
            }
            _ => {}
        }
    }
    if !balanced || depth != 0 || direct_start.is_some() {
        return Vec::new(); // 未闭合 / 悬空开标签：形态异常
    }
    spans
}

/// set_cell_text 手术：定位第 `row` 行第 `cell` 格（1-based，与 projection=table
/// 视图同口径——跨列格算 1 格），重建格文本，其余字节原样。
fn set_cell_text(block_xml: &str, row: usize, cell: usize, text: &str) -> Option<String> {
    let rows = direct_children_spans(block_xml, "tr");
    let (rs, re) = *rows.get(row.checked_sub(1)?)?;
    let row_xml = &block_xml[rs..re];
    let cells = direct_children_spans(row_xml, "tc");
    let (cs, ce) = *cells.get(cell.checked_sub(1)?)?;
    let new_cell = rebuild_cell_content(&row_xml[cs..ce], text)?;
    let mut out = String::with_capacity(block_xml.len() + 64);
    out.push_str(&block_xml[..rs]);
    out.push_str(&row_xml[..cs]);
    out.push_str(&new_cell);
    out.push_str(&row_xml[ce..]);
    out.push_str(&block_xml[re..]);
    Some(out)
}

/// 重建单元格：保开标签属性 + tcPr（含 gridSpan/vMerge/tcW），段落由 text 生成。
/// 模板格式 = 原首段的 pPr + 首 run 的 rPr（与 replace_text 保真纪律同源）。
/// 嵌套表保护：格内含 w:tbl 时返回 None（预检已拒，此处兜底防直调误伤）。
fn rebuild_cell_content(cell_xml: &str, text: &str) -> Option<String> {
    if cell_xml.contains("<w:tbl>") || cell_xml.contains("<w:tbl ") {
        return None;
    }
    let open_end = cell_xml.find('>')? + 1; // 开标签止于 '>'（tc 无自闭合形态）
    let tcpr = find_element_span(cell_xml, "tcPr");
    let content_from = tcpr.map(|(_, e)| e).unwrap_or(open_end);
    let content = &cell_xml[content_from..cell_xml.len().saturating_sub("</w:tc>".len())];
    let first_p = find_element_span(content, "p")?;
    let p_xml = &content[first_p.0..first_p.1];
    let ppr = slice_ppr(p_xml);
    let rpr = slice_first_run_rpr(p_xml);
    let mut paras = String::new();
    if text.is_empty() {
        paras.push_str(&format!("<w:p>{ppr}</w:p>"));
    } else {
        for line in text.split('\n') {
            paras.push_str(&format!("<w:p>{ppr}{}</w:p>", run_xml(&rpr, line)));
        }
    }
    let mut out = String::with_capacity(cell_xml.len() + paras.len() + 8);
    out.push_str(&cell_xml[..open_end]);
    if let Some((s, e)) = tcpr {
        out.push_str(&cell_xml[s..e]);
    }
    out.push_str(&paras);
    out.push_str("</w:tc>");
    Some(out)
}

/// 增行手术：克隆模板行（after_row 缺省 = 末行）整结构——tcPr/gridSpan/vMerge
/// 原样保留，格文本替换为 cells（缺省全空）。结构克隆是合并格表格唯一正确的
/// 增行方式（凭空造行会破坏 gridSpan/vMerge 与 tblGrid 的对应）。
fn insert_table_row_after(
    block_xml: &str,
    after_row: Option<usize>,
    cells: Option<&[String]>,
) -> Option<String> {
    let rows = direct_children_spans(block_xml, "tr");
    if rows.is_empty() {
        return None;
    }
    let tpl_idx = match after_row {
        Some(r) => r.checked_sub(1)?,
        None => rows.len() - 1,
    };
    let (ts, te) = *rows.get(tpl_idx)?;
    let tpl = &block_xml[ts..te];
    let tcs = direct_children_spans(tpl, "tc");
    if tcs.is_empty() {
        return None;
    }
    let filled: Vec<&str> = match cells {
        Some(cs) => cs.iter().map(String::as_str).collect(),
        None => vec![""; tcs.len()],
    };
    let mut new_row = String::with_capacity(tpl.len() + 32);
    new_row.push_str(&tpl[..tcs[0].0]); // <w:tr> + trPr（若有）原样
    for (i, (cs, ce)) in tcs.iter().enumerate() {
        let rebuilt = rebuild_cell_content(&tpl[*cs..*ce], filled.get(i).copied().unwrap_or(""))?;
        new_row.push_str(&rebuilt);
    }
    new_row.push_str("</w:tr>");
    let mut out = String::with_capacity(block_xml.len() + new_row.len());
    out.push_str(&block_xml[..te]);
    out.push_str(&new_row);
    out.push_str(&block_xml[te..]);
    Some(out)
}

/// (row, cell) 单元格投影文本（前 60 字）——set_cell_text 摘要的 before 值。
fn cell_projected_of(model: &docx_model::DocxDocument, block: usize, row: usize, cell: usize) -> String {
    let mut s = String::new();
    if let Block::Table(t) = &model.body[block - 1] {
        if let Some(r) = t.rows.get(row - 1) {
            if let Some(c) = r.cells.get(cell - 1) {
                docx_model::blocks_text(&c.blocks, &mut s);
            }
        }
    }
    truncate(s.trim_end_matches('\n'), 60)
}

// =========================================================================
// 表格格式件手术（S3 四波）：容器元素手术 / 格内格式 / 合并 / 拆分
// =========================================================================

/// 定位行 XML（1-based 行号 → 该行在 block_xml 内的字节范围）。
fn row_span_of(block_xml: &str, row: usize) -> Option<(usize, usize)> {
    direct_children_spans(block_xml, "tr")
        .get(row.checked_sub(1)?)
        .copied()
}

/// 定位行内格 XML（1-based 格号，与 projection=table 显示序同口径）。
fn cell_span_of(row_xml: &str, cell: usize) -> Option<(usize, usize)> {
    direct_children_spans(row_xml, "tc")
        .get(cell.checked_sub(1)?)
        .copied()
}

/// 对 (row, cell) 定位的格应用手术函数（输入格 XML，输出 (新格 XML, 附带值)），
/// 行内其余字节与表内其余行原样。嵌套表由手术函数自查（预检已拒，兜底防直调）。
fn apply_to_cell<T>(
    block_xml: &str,
    row: usize,
    cell: usize,
    f: impl Fn(&str) -> Option<(String, T)>,
) -> Option<(String, T)> {
    let (rs, re) = row_span_of(block_xml, row)?;
    let row_xml = &block_xml[rs..re];
    let (cs, ce) = cell_span_of(row_xml, cell)?;
    let (new_cell, t) = f(&row_xml[cs..ce])?;
    let new_row = format!("{}{new_cell}{}", &row_xml[..cs], &row_xml[ce..]);
    Some((format!("{}{new_row}{}", &block_xml[..rs], &block_xml[re..]), t))
}

/// 属性容器子元素手术（tblPr / trPr / tcPr 通用，set_ppr_element 的容器版）：
/// - frag=None：移除 `<w:{element}>` 整元素；容器摘空则整体清理；元素不存在 →
///   返回 (原文, false) 空转
/// - frag=Some：整元素替换（已存在）或按 `whitelist` schema 序插入（不存在）；
///   容器不存在时新建于 scope 开标签后（容器是 tbl/tr/tc 的 schema 首子元素）
///
/// 返回 (新 scope XML, 是否实际改动)。None 仅防御形态异常（不写盘）。
fn set_container_element_xml(
    scope_xml: &str,
    container: &str,
    element: &str,
    frag: Option<&str>,
    whitelist: &[&str],
) -> Option<(String, bool)> {
    let pat = format!("<w:{container}");
    let container_at = scope_xml.find(&pat).filter(|s| {
        matches!(
            scope_xml.as_bytes().get(s + pat.len()),
            Some(b'>') | Some(b'/') | Some(b' ')
        )
    });
    let Some(s) = container_at else {
        // 无容器：无元素可摘 → 空转；有片段 → 开标签后新建容器
        let Some(f) = frag else {
            return Some((scope_xml.to_string(), false));
        };
        let gt = scope_xml.find('>')?;
        if scope_xml.as_bytes()[gt - 1] == b'/' {
            return None; // tbl/tr/tc 不自闭合——形态异常
        }
        let new_container = format!("<w:{container}>{f}</w:{container}>");
        return Some((
            format!("{}{new_container}{}", &scope_xml[..gt + 1], &scope_xml[gt + 1..]),
            true,
        ));
    };
    let open_end = s + scope_xml[s..].find('>')? + 1;
    let close_tag = format!("</w:{container}>");
    if scope_xml.as_bytes()[open_end - 2] == b'/' {
        // 自闭合空容器（Word 偶出）：无子元素可摘 → 空转；有片段 → 展开为完整容器
        return match frag {
            None => Some((scope_xml.to_string(), false)),
            Some(f) => Some((
                format!(
                    "{}<w:{container}>{f}</w:{container}>{}",
                    &scope_xml[..s],
                    &scope_xml[open_end..]
                ),
                true,
            )),
        };
    }
    let close_at = open_end + scope_xml[open_end..].find(&close_tag)?;
    match frag {
        None => {
            let inner = &scope_xml[open_end..close_at];
            let Some((es, ee)) = find_element_span(inner, element) else {
                return Some((scope_xml.to_string(), false)); // 不存在 → 空转
            };
            let new_inner = format!("{}{}", &inner[..es], &inner[ee..]);
            if new_inner.trim().is_empty() {
                // 摘空 → 容器整体移除（Word 自身也这样清理）
                return Some((
                    format!("{}{}", &scope_xml[..s], &scope_xml[close_at + close_tag.len()..]),
                    true,
                ));
            }
            Some((
                format!("{}{}{}", &scope_xml[..open_end], new_inner, &scope_xml[close_at..]),
                true,
            ))
        }
        Some(f) => {
            let idx = whitelist.iter().position(|n| *n == element)?;
            let later: Vec<&str> = whitelist[idx + 1..].to_vec();
            let container_el = &scope_xml[s..close_at + close_tag.len()];
            let new_container = upsert_element(container_el, element, f, &later, &close_tag);
            Some((
                format!("{}{new_container}{}", &scope_xml[..s], &scope_xml[close_at + close_tag.len()..]),
                true,
            ))
        }
    }
}

/// set_table_element 手术：定位 level 对应 scope（表=整块 / 行=tr / 格=tc），
/// 委托容器手术。row/cell 寻址按当前 XML 形态（同块前序操作已生效）。
fn set_table_element_xml(
    block_xml: &str,
    level: TableLevel,
    row: Option<usize>,
    cell: Option<usize>,
    element: &str,
    frag: Option<&str>,
) -> Option<(String, bool)> {
    match level {
        TableLevel::Table => {
            set_container_element_xml(block_xml, "tblPr", element, frag, &TBLPR_ELEMENTS)
        }
        TableLevel::Row => {
            let (rs, re) = row_span_of(block_xml, row?)?;
            let (new_row, changed) = set_container_element_xml(
                &block_xml[rs..re],
                "trPr",
                element,
                frag,
                &TRPR_ELEMENTS,
            )?;
            Some((
                format!("{}{new_row}{}", &block_xml[..rs], &block_xml[re..]),
                changed,
            ))
        }
        TableLevel::Cell => apply_to_cell(block_xml, row?, cell?, |cell_xml| {
            set_container_element_xml(cell_xml, "tcPr", element, frag, &TCPR_ELEMENTS)
        }),
    }
}

/// set_cell_format 手术：段落格式作用于格内全部直接 `<w:p>`（reformat_ppr 逐段，
/// 含自闭合空段展开），字符格式作用于格内全部 run（reformat_runs 整格），
/// 样式（style_id 已反查）作用于格内全部段落（restyle_paragraph 逐段——与
/// 直接格式互不冲突，pStyle 与 spacing/ind 等 pPr 子元素各占各位）。
fn set_cell_format_xml(
    block_xml: &str,
    row: usize,
    cell: usize,
    paragraph: Option<&ParaFormat>,
    character: Option<&CharFormat>,
    style_id: Option<&str>,
) -> Option<String> {
    apply_to_cell(block_xml, row, cell, |cell_xml| {
        if cell_xml.contains("<w:tbl>") || cell_xml.contains("<w:tbl ") {
            return None; // 嵌套表兜底（预检已拒）
        }
        let mut out = cell_xml.to_string();
        if let Some(p) = paragraph {
            let paras = direct_children_spans(&out, "p");
            if paras.is_empty() {
                return None; // Word 格必含 ≥1 段——形态异常
            }
            for (ps, pe) in paras.iter().rev() {
                let new_p = reformat_ppr(&out[*ps..*pe], p)?;
                out.replace_range(ps..pe, &new_p);
            }
        }
        if let Some(id) = style_id {
            let paras = direct_children_spans(&out, "p");
            if paras.is_empty() {
                return None; // Word 格必含 ≥1 段——形态异常
            }
            for (ps, pe) in paras.iter().rev() {
                let new_p = restyle_paragraph(&out[*ps..*pe], id)?;
                out.replace_range(ps..pe, &new_p);
            }
        }
        if let Some(c) = character {
            out = reformat_runs(&out, c)?;
        }
        Some((out, ()))
    })
    .map(|(s, _)| s)
}

/// XML 层的格 gridSpan（tcPr 内 w:gridSpan w:val；缺省 1）。
fn xml_grid_span(cell_xml: &str) -> u32 {
    let Some((s, e)) = find_element_span(cell_xml, "tcPr") else {
        return 1;
    };
    let inner = &cell_xml[s..e]; // 偏移属本切片——勿拿去索引原串
    find_element_span(inner, "gridSpan")
        .map(|(gs, ge)| {
            parse_attrs(&inner[gs..ge])
                .into_iter()
                .find(|(k, _)| k == "w:val")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(1)
        })
        .unwrap_or(1)
}

/// XML 层的 vMerge 归一（None=无 / Some("restart") / Some("continue")，与模型口径同）。
fn xml_v_merge(cell_xml: &str) -> Option<String> {
    let (s, e) = find_element_span(cell_xml, "tcPr")?;
    let inner = &cell_xml[s..e]; // 偏移属本切片——勿拿去索引原串
    let (vs, ve) = find_element_span(inner, "vMerge")?;
    Some(
        parse_attrs(&inner[vs..ve])
            .into_iter()
            .find(|(k, _)| k == "w:val")
            .map(|(_, v)| v)
            .unwrap_or_else(|| "continue".to_string()), // <w:vMerge/> 无 val = continue
    )
}

/// 行内占据恰好 [g0, g1) 网格列区间的格的字节范围（None = 边界不对齐）。
/// XML 层对齐判据（纵并用），与模型的 cell_at_grid_range 同语义。
fn xml_cell_span_at_grid_range(
    row_xml: &str,
    g0: u32,
    g1: u32,
) -> Option<(usize, usize)> {
    let mut start = 0u32;
    for (s, e) in direct_children_spans(row_xml, "tc") {
        let span = xml_grid_span(&row_xml[s..e]);
        if start == g0 && start + span == g1 {
            return Some((s, e));
        }
        start += span;
    }
    None
}

/// 简单横并原语（merge_cells 线并形态与矩形区的逐行步骤共用）：
/// 第 row 行 display 格 cell..cell+span-1 并 1 格——首格 tcPr upsert
/// gridSpan=各格跨度之和；全部格内容（各自去 tcPr）按序拼进首格；其余格整段删除。
fn merge_horizontal_xml(
    block_xml: &str,
    row: usize,
    cell: usize,
    span: usize,
) -> Option<(String, u32)> {
    let (rs, re) = row_span_of(block_xml, row)?;
    let row_xml = &block_xml[rs..re];
    let cells = direct_children_spans(row_xml, "tc");
    let first = cell.checked_sub(1)?;
    let last = first.checked_add(span.checked_sub(1)?)?;
    let range = cells.get(first..=last)?.to_vec();
    let sum: u32 = range
        .iter()
        .map(|(s, e)| xml_grid_span(&row_xml[*s..*e]))
        .sum();
    // 首格容器手术 upsert gridSpan；内容 = 各格 inner（去 tcPr）按序拼接
    let head_xml = &row_xml[range[0].0..range[0].1];
    let (new_head, _) = set_container_element_xml(
        head_xml,
        "tcPr",
        "gridSpan",
        Some(&format!(r#"<w:gridSpan w:val="{sum}"/>"#)),
        &TCPR_ELEMENTS,
    )?;
    let mut content = String::new();
    for (s, e) in &range {
        let cxml = &row_xml[*s..*e];
        let open_end = cxml.find('>')? + 1;
        let from = find_element_span(cxml, "tcPr")
            .map(|(_, te)| te)
            .unwrap_or(open_end);
        content.push_str(&cxml[from..cxml.len().saturating_sub("</w:tc>".len())]);
    }
    let open_end = head_xml.find('>')? + 1;
    let (ts, te) = find_element_span(&new_head, "tcPr")?; // upsert 后必在
    let merged = format!(
        "{}{}{}</w:tc>",
        &head_xml[..open_end],
        &new_head[ts..te],
        content
    );
    let new_row = format!(
        "{}{merged}{}",
        &row_xml[..range[0].0],
        &row_xml[range[last - first].1..]
    );
    Some((
        format!("{}{new_row}{}", &block_xml[..rs], &block_xml[re..]),
        sum,
    ))
}

/// 简单纵并原语：首格 (row,cell) 起同网格列区间 span 行——首格 upsert
/// `<w:vMerge w:val="restart"/>`，后续行同网格列区间的格 upsert `<w:vMerge/>`
/// （continue）；内容原样保留（拆分即恢复显示）。
fn merge_vertical_xml(block_xml: &str, row: usize, cell: usize, span: usize) -> Option<String> {
    let (hrs, hre) = row_span_of(block_xml, row)?;
    let head_row_xml = &block_xml[hrs..hre];
    let head_cells = direct_children_spans(head_row_xml, "tc");
    let (hcs, hce) = *head_cells.get(cell.checked_sub(1)?)?;
    // 首格网格区间 [g0, g1)（对齐判据）
    let mut g0 = 0u32;
    for (s, e) in head_cells.iter().take(cell - 1) {
        g0 += xml_grid_span(&head_row_xml[*s..*e]);
    }
    let g1 = g0 + xml_grid_span(&head_row_xml[hcs..hce]);
    // 逐行容器手术（行 span 互不重叠，按原偏移重组）
    let mut fixed: Vec<(usize, usize, String)> = Vec::new();
    for i in 0..span {
        let r = row + i;
        let (rs2, re2) = row_span_of(block_xml, r)?;
        let row_xml = &block_xml[rs2..re2];
        let (cs2, ce2) = xml_cell_span_at_grid_range(row_xml, g0, g1)?;
        let frag = if i == 0 {
            r#"<w:vMerge w:val="restart"/>"#
        } else {
            "<w:vMerge/>"
        };
        let (new_cell, _) = set_container_element_xml(
            &row_xml[cs2..ce2],
            "tcPr",
            "vMerge",
            Some(frag),
            &TCPR_ELEMENTS,
        )?;
        fixed.push((
            rs2,
            re2,
            format!("{}{new_cell}{}", &row_xml[..cs2], &row_xml[ce2..]),
        ));
    }
    let mut out = String::with_capacity(block_xml.len() + 64);
    let mut cursor = 0usize;
    for (rs2, re2, nr) in fixed {
        out.push_str(&block_xml[cursor..rs2]);
        out.push_str(&nr);
        cursor = re2;
    }
    out.push_str(&block_xml[cursor..]);
    Some(out)
}

/// merge_cells 手术（Word 原生语义，两形态）：
/// - 简单线并（direction+span）：横并原语或纵并原语单发
/// - 矩形区（end_row+end_cell）：逐行横并（行互不重叠，倒序保原始偏移）→
///   结果列纵并——Word 合并区域 UX 同款，一次调用完成多行多列合并
fn merge_cells_xml(
    block_xml: &str,
    direction: Option<MergeDirection>,
    row: usize,
    cell: usize,
    span: Option<usize>,
    end_row: Option<usize>,
    end_cell: Option<usize>,
) -> Option<(String, String)> {
    match (end_row, end_cell) {
        (Some(er), Some(ec)) => {
            let width = ec - cell + 1;
            let rows_n = er - row + 1;
            let mut out = block_xml.to_string();
            if ec > cell {
                for r in (row..=er).rev() {
                    let (next, _) = merge_horizontal_xml(&out, r, cell, width)?;
                    out = next;
                }
            }
            if er > row {
                out = merge_vertical_xml(&out, row, cell, rows_n)?;
            }
            Some((
                out,
                format!("r{row}c{cell}..r{er}c{ec} 矩形合并（{width} 格宽 × {rows_n} 行）"),
            ))
        }
        _ => {
            let dir = direction?; // 预检已挡缺参
            let s = span.unwrap_or(2);
            match dir {
                MergeDirection::Horizontal => merge_horizontal_xml(block_xml, row, cell, s)
                    .map(|(out, sum)| {
                        (
                            out,
                            format!("r{row} c{cell} 起横并 {s} 格（跨 {sum} 列，内容已按序拼接）"),
                        )
                    }),
                MergeDirection::Vertical => merge_vertical_xml(block_xml, row, cell, s)
                    .map(|out| {
                        (
                            out,
                            format!("r{row}c{cell} 起纵并 {s} 行（内容保留，split_cell 即恢复）"),
                        )
                    }),
            }
        }
    }
}

/// split_cell 手术（merge_cells 的逆）：
/// - vertical：对 (合并头) 拆整条纵并链——首格摘 vMerge(restart)，沿同网格列
///   区间向下摘 vMerge(continue) 至链尾；各格内容原样恢复显示
/// - horizontal：对 (跨N列) 格拆回 N 个单格——首格保内容、tcPr 摘 gridSpan，
///   其余 N-1 格 = 同开标签 + 同 tcPr（无 gridSpan）+ 空段（继承首段 pPr）
fn split_cell_xml(
    block_xml: &str,
    direction: MergeDirection,
    row: usize,
    cell: usize,
) -> Option<(String, String)> {
    match direction {
        MergeDirection::Vertical => {
            let rows = direct_children_spans(block_xml, "tr");
            let (hrs, hre) = *rows.get(row.checked_sub(1)?)?;
            let head_row_xml = &block_xml[hrs..hre];
            let head_cells = direct_children_spans(head_row_xml, "tc");
            let (hcs, hce) = *head_cells.get(cell.checked_sub(1)?)?;
            let mut g0 = 0u32;
            for (s, e) in head_cells.iter().take(cell - 1) {
                g0 += xml_grid_span(&head_row_xml[*s..*e]);
            }
            let g1 = g0 + xml_grid_span(&head_row_xml[hcs..hce]);
            // 沿链收集待摘格：首格必 restart（预检已验）；向下遇非 continue 即链尾
            let mut chain: Vec<(usize, usize, usize, usize)> = Vec::new(); // (行起, 行止, 格起, 格止)
            for i in 0..rows.len().saturating_sub(row - 1) {
                let r = row + i;
                let (rs2, re2) = *rows.get(r - 1)?;
                let row_xml = &block_xml[rs2..re2];
                let Some((cs2, ce2)) = xml_cell_span_at_grid_range(row_xml, g0, g1) else {
                    break; // 边界不对齐 → 链断
                };
                let vm = xml_v_merge(&row_xml[cs2..ce2]);
                if i == 0 {
                    if vm.as_deref() != Some("restart") {
                        return None; // 预检已挡——防御
                    }
                    chain.push((rs2, re2, cs2, ce2));
                } else {
                    match vm.as_deref() {
                        Some("continue") => chain.push((rs2, re2, cs2, ce2)),
                        _ => break, // 无合并 / 新链头 restart → 链尾
                    }
                }
            }
            let count = chain.len();
            let mut out = String::with_capacity(block_xml.len());
            let mut cursor = 0usize;
            for (rs2, re2, cs2, ce2) in chain {
                let row_xml = &block_xml[rs2..re2];
                let (new_cell, _) = set_container_element_xml(
                    &row_xml[cs2..ce2],
                    "tcPr",
                    "vMerge",
                    None,
                    &TCPR_ELEMENTS,
                )?;
                out.push_str(&block_xml[cursor..rs2]);
                out.push_str(&format!("{}{new_cell}{}", &row_xml[..cs2], &row_xml[ce2..]));
                cursor = re2;
            }
            out.push_str(&block_xml[cursor..]);
            Some((
                out,
                format!("r{row}c{cell} 纵并链拆分，{count} 格恢复独立显示"),
            ))
        }
        MergeDirection::Horizontal => {
            let (rs, re) = row_span_of(block_xml, row)?;
            let row_xml = &block_xml[rs..re];
            let (cs, ce) = cell_span_of(row_xml, cell)?;
            let cell_xml = &row_xml[cs..ce];
            let n = xml_grid_span(cell_xml);
            if n < 2 {
                return None; // 预检已挡——防御
            }
            // 摘 gridSpan（原 tcPr 只含 gridSpan 时容器整体清理）
            let (no_span_cell, _) =
                set_container_element_xml(cell_xml, "tcPr", "gridSpan", None, &TCPR_ELEMENTS)?;
            // 首段 pPr 模板（空格继承）
            let open_end = cell_xml.find('>')? + 1;
            let from = find_element_span(&no_span_cell, "tcPr")
                .map(|(_, e)| e)
                .unwrap_or(open_end);
            let content =
                &no_span_cell[from..no_span_cell.len().saturating_sub("</w:tc>".len())];
            let fp = find_element_span(content, "p")?;
            let ppr = slice_ppr(&content[fp.0..fp.1]);
            // N-1 个空格：同开标签 + 同 tcPr（若摘 gridSpan 后仍在）+ 空段
            let tcpr_left = find_element_span(&no_span_cell, "tcPr");
            let mut extras = String::new();
            for _ in 0..n - 1 {
                extras.push_str(&cell_xml[..open_end]);
                if let Some((ts, te)) = tcpr_left {
                    extras.push_str(&no_span_cell[ts..te]);
                }
                extras.push_str(&format!("<w:p>{ppr}</w:p>"));
                extras.push_str("</w:tc>");
            }
            let new_row = format!(
                "{}{no_span_cell}{extras}{}",
                &row_xml[..cs],
                &row_xml[ce..]
            );
            let out = format!("{}{new_row}{}", &block_xml[..rs], &block_xml[re..]);
            Some((
                out,
                format!("r{row}c{cell} 横并拆回 {n} 格（内容留首格）"),
            ))
        }
    }
}

/// 删表格一行（S3 七波·生产反馈 P0）：direct_children_spans 定位直接子 tr
/// （嵌套表的内层 tr 不计），整段摘除。合并链与末行守卫在预检。
fn delete_table_row_xml(table_xml: &str, row: usize) -> Option<(String, String)> {
    let rows = direct_children_spans(table_xml, "tr");
    let (rs, re) = *rows.get(row.checked_sub(1)?)?;
    let mut out = String::with_capacity(table_xml.len());
    out.push_str(&table_xml[..rs]);
    out.push_str(&table_xml[re..]);
    Some((out, format!("删第 {row} 行（剩 {} 行）", rows.len() - 1)))
}

// =========================================================================
// 容器重打包（只换 document.xml，其余 entry 原样字节复制）
// =========================================================================

/// 重打包 docx：`part` 指定的 entry 替换为 `new_xml`（document.xml / styles.xml /
/// numbering.xml 通用），其余 entry 经 `raw_copy_file` 原样复制（不解压重压——
/// 压缩参数与元数据不变）。
pub(super) fn repack_part(bytes: &[u8], part: &str, new_xml: &str) -> AppResult<Vec<u8>> {
    let cur = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cur)
        .map_err(|e| AppError::Internal(format!("docx 不是合法 ZIP 容器: {e}")))?;

    let writer = {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::<u8>::with_capacity(bytes.len())));
        for i in 0..archive.len() {
            let entry = archive
                .by_index_raw(i)
                .map_err(|e| AppError::Internal(format!("docx 内读取 entry 失败: {e}")))?;
            let name = entry.name().to_string();
            if name == part {
                // 逐 entry 借用冲突：目标部件先收名，循环外统一写
                drop(entry);
                w.start_file(part, zip::write::SimpleFileOptions::default())
                    .map_err(|e| AppError::Internal(format!("重打包 {part} 失败: {e}")))?;
                w.write_all(new_xml.as_bytes()).map_err(AppError::Io)?;
            } else {
                w.raw_copy_file(entry)
                    .map_err(|e| AppError::Internal(format!("重打包 entry {name} 失败: {e}")))?;
            }
        }
        w
    };

    let out = writer
        .finish()
        .map_err(|e| AppError::Internal(format!("docx 重打包收尾失败: {e}")))?;
    Ok(out.into_inner())
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::super::docx;
    use super::*;
    use std::io::Read;

    fn wrap(body: &str) -> String {
        format!(
            r#"<w:document xmlns:w="w" xmlns:r="r"><w:body>{body}</w:body></w:document>"#
        )
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
            assert_eq!(piece_model.body.len(), 1, "块 {} 子串应恰为单块: {piece}", i + 1);
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
        assert!(out.contains(r#"<w:pPr><w:pStyle w:val="body"/></w:pPr><w:r><w:rPr><w:i/></w:rPr>"#));
    }

    #[test]
    fn insert_with_style_name_resolves_id() {
        let styles_xml = r#"<w:styles><w:style w:type="paragraph" w:styleId="h1"><w:name w:val="heading 1"/></w:style></w:styles>"#;
        let sheet = super::super::styles::parse_styles(
            &super::super::xml_dom::parse(styles_xml).unwrap(),
        );
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
        assert!(out.contains(r#"<w:pPr><w:pStyle w:val="h1"/></w:pPr>"#), "显示名反查 ID");
        // 未知样式报错
        let err = val_msg(apply_edits(
            &xml,
            &sheet,
            &[EditOp::InsertParagraphAfter {
                block: 1,
                expect_prefix: "正文".into(),
                text: "x".into(),
                style: Some("不存在的样式".into()),
            }],
        )
        .unwrap_err());
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
            &[EditOp::DeleteBlock { block: 2, expect_prefix: "二".into() }],
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
                EditOp::DeleteBlock { block: 5, expect_prefix: "第5段".into() },
                EditOp::ReplaceText { block: 3, expect_prefix: "第3段".into(), new_text: "改后".into() },
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
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[
                EditOp::ReplaceText { block: 1, expect_prefix: "别的内容".into(), new_text: "x".into() },
            ],
        )
        .unwrap_err());
        assert!(err.starts_with("指纹不符"), "实际: {err}");
        assert!(err.contains("inspect_docx"));
    }

    #[test]
    fn revision_and_sectpr_blocks_rejected() {
        let styles = Stylesheet::empty();
        // 修订块
        let xml = wrap(r#"<w:p><w:ins><w:r><w:t>修订段</w:t></w:r></w:ins></w:p>"#);
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::DeleteBlock { block: 1, expect_prefix: "修订段".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("含修订标记"), "实际: {err}");
        // 节末段
        let xml = wrap(
            r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="1"/></w:sectPr></w:pPr><w:r><w:t>节末段</w:t></w:r></w:p>"#,
        );
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::DeleteBlock { block: 1, expect_prefix: "节末段".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("节属性保护"), "实际: {err}");
    }

    #[test]
    fn duplicate_block_and_bounds_rejected() {
        let xml = wrap(r#"<w:p><w:r><w:t>一</w:t></w:r></w:p>"#);
        let styles = Stylesheet::empty();
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[
                EditOp::ReplaceText { block: 1, expect_prefix: "一".into(), new_text: "x".into() },
                EditOp::DeleteBlock { block: 1, expect_prefix: "x".into() },
            ],
        )
        .unwrap_err());
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::DeleteBlock { block: 7, expect_prefix: "".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("块号越界"), "实际: {err}");
        assert!(err.contains("1-1"), "应带有效范围: {err}");
    }

    #[test]
    fn table_block_replace_rejected() {
        let xml = wrap(r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
        let styles = Stylesheet::empty();
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::ReplaceText { block: 1, expect_prefix: "表".into(), new_text: "x".into() }],
        )
        .unwrap_err());
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
            &[EditOp::SetStyle { block: 1, expect_prefix: "正文段".into(), style: "heading 2".into() }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "set_style");
        assert_eq!(applied[0].style.as_deref(), Some("h2"));
        assert!(out.contains(r#"<w:p w14:paraId="X1">"#), "开标签属性保留");
        assert!(out.contains(r#"<w:pPr><w:pStyle w:val="h2"/><w:jc w:val="center"/></w:pPr>"#));
        assert!(out.contains(r#"<w:rPr><w:b/></w:rPr><w:t>正文段</w:t></w:r>"#), "run 原样");
        // 模型侧：样式生效 + 文本不变
        let m = model_of(&out);
        let super::docx_model::Block::Paragraph(p) = &m.body[0] else { panic!() };
        assert_eq!(p.props.style.as_deref(), Some("h2"));
        let mut t = String::new();
        docx_model::blocks_text(&m.body, &mut t);
        assert_eq!(t, "正文段\n");
    }

    #[test]
    fn set_style_noop_reports_style_unchanged() {
        // 目标段 pStyle 已是 h2，再 set 成 heading 2（显示名解析到同一 ID）→ 空转，
        // AppliedOp 显式报 style_unchanged=true；换到不同样式则 None（字段省略）
        let xml = wrap(r#"<w:p><w:pPr><w:pStyle w:val="h2"/></w:pPr><w:r><w:t>已是标题段</w:t></w:r></w:p>"#);
        let (_, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetStyle { block: 1, expect_prefix: "已是标题段".into(), style: "heading 2".into() }],
        )
        .unwrap();
        assert_eq!(applied[0].style_unchanged, Some(true));
        // agent 读回的 JSON 必须携带该信号（skip_serializing_if 只对 None 生效）
        let json = serde_json::to_string(&applied[0]).unwrap();
        assert!(json.contains("\"style_unchanged\":true"), "实际: {json}");

        let (out, applied) = apply_edits(
            &xml,
            &heading_styles(),
            &[EditOp::SetStyle { block: 1, expect_prefix: "已是标题段".into(), style: "Normal".into() }],
        )
        .unwrap();
        assert_eq!(applied[0].style_unchanged, None);
        let json = serde_json::to_string(&applied[0]).unwrap();
        assert!(!json.contains("style_unchanged"), "非空转不应带字段: {json}");
        assert!(out.contains(r#"<w:pStyle w:val="body"/>"#), "真换样式仍生效");
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
        assert!(applied[0].after.starts_with("removed numPr"), "实际: {}", applied[0].after);
        assert!(!out.contains("<w:numPr"), "numPr 应消失");
        assert!(
            out.contains(r#"<w:pPr><w:pStyle w:val="3"/><w:spacing w:before="163"/></w:pPr>"#),
            "兄弟元素原样: {out}"
        );
        let m = model_of(&out);
        let Block::Paragraph(p) = &m.body[0] else { panic!() };
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
        let xml = wrap(
            r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>普通段</w:t></w:r></w:p>"#,
        );
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
        assert!(applied[0].after.contains("空转"), "实际: {}", applied[0].after);
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
        let xml = wrap(r#"<w:p><w:pPr><w:jc w:val="left"/></w:pPr><w:r><w:t>替换段</w:t></w:r></w:p>"#);
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
        assert!(out.contains(r#"<w:pPr><w:jc w:val="right"/></w:pPr>"#), "整元素替换: {out}");
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
        assert!(out.contains(r#"<w:p><w:pPr><w:keepNext/></w:pPr><w:r><w:t>裸段</w:t></w:r></w:p>"#));
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
        let err = val_msg(apply_edits(
            &para,
            &styles,
            &[EditOp::SetPprElement { block: 1, expect_prefix: "一段".into(), element: "numId".into(), xml: None }],
        ).unwrap_err());
        assert!(err.starts_with("非法pPr子元素"), "实际: {err}");
        // 受保护：sectPr / pPrChange
        for protected in ["sectPr", "pPrChange"] {
            let err = val_msg(apply_edits(
                &para,
                &styles,
                &[EditOp::SetPprElement {
                    block: 1,
                    expect_prefix: "一段".into(),
                    element: protected.into(),
                    xml: None,
                }],
            ).unwrap_err());
            assert!(err.starts_with("受保护子元素"), "实际: {err}");
        }
        // 表格块
        let tbl = wrap(r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
        let err = val_msg(apply_edits(
            &tbl,
            &styles,
            &[EditOp::SetPprElement { block: 1, expect_prefix: "表".into(), element: "keepNext".into(), xml: None }],
        ).unwrap_err());
        assert!(err.starts_with("表格块"), "实际: {err}");
    }

    #[test]
    fn ppr_fragment_validation_families() {
        let styles = heading_styles();
        let para = wrap(r#"<w:p><w:r><w:t>片段段</w:t></w:r></w:p>"#);
        let run = |xml: Option<&str>| {
            val_msg(apply_edits(
                &para,
                &styles,
                &[EditOp::SetPprElement {
                    block: 1,
                    expect_prefix: "片段".into(),
                    element: "numPr".into(),
                    xml: xml.map(str::to_string),
                }],
            ).unwrap_err())
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
        assert!(err.starts_with("片段校验失败") && err.contains("xmlns"), "实际: {err}");
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
        assert!(!applied[0].after.contains("样式链仍定义编号"), "样式无编号不应警告");
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
                EditOp::SetStyle { block: 1, expect_prefix: "列表项".into(), style: "h2".into() },
                EditOp::SetStyle { block: 2, expect_prefix: "裸段".into(), style: "h2".into() },
                EditOp::SetStyle { block: 3, expect_prefix: "".into(), style: "body".into() },
            ],
        )
        .unwrap();
        assert_eq!(applied.len(), 3);
        assert!(
            out.contains(r#"<w:pPr><w:pStyle w:val="h2"/><w:numPr>"#),
            "pStyle 应插在 pPr 首位: {out}"
        );
        assert!(out.contains(r#"<w:p><w:pPr><w:pStyle w:val="h2"/></w:pPr><w:r><w:t>裸段</w:t></w:r></w:p>"#));
        assert!(out.contains(r#"<w:p w14:paraId="E"><w:pPr><w:pStyle w:val="body"/></w:pPr></w:p>"#));
        let m = model_of(&out);
        assert_eq!(m.body.len(), 3, "块数守恒");
    }

    #[test]
    fn set_style_unknown_and_table_rejected() {
        let styles = heading_styles();
        // 未知样式（家族前缀）
        let xml = wrap(r#"<w:p><w:r><w:t>一段</w:t></w:r></w:p>"#);
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::SetStyle { block: 1, expect_prefix: "一段".into(), style: "没有这样式".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("未知样式"), "实际: {err}");
        // 表格块
        let xml = wrap(r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::SetStyle { block: 1, expect_prefix: "表".into(), style: "h2".into() }],
        )
        .unwrap_err());
        assert!(err.starts_with("表格块"), "实际: {err}");
        // 段落属性修订（pPrChange）——换样式与其语义纠缠，拒
        let xml = wrap(
            r#"<w:p><w:pPr><w:pStyle w:val="body"/><w:pPrChange w:id="1"><w:pPr><w:pStyle w:val="h2"/></w:pPr></w:pPrChange></w:pPr><w:r><w:t>改格段</w:t></w:r></w:p>"#,
        );
        let err = val_msg(apply_edits(
            &xml,
            &styles,
            &[EditOp::SetStyle { block: 1, expect_prefix: "改格段".into(), style: "h2".into() }],
        )
        .unwrap_err());
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
        assert!(applied[0].after.contains("对齐=center"), "摘要: {}", applied[0].after);
        // 合并断言：before 未提及原样 120；line/lineRule/after 追加进同一 spacing；firstLine 覆盖 720
        assert!(
            out.contains(r#"<w:spacing w:before="120" w:line="360" w:lineRule="auto" w:after="120"/>"#),
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
        assert!(pos("<w:pStyle") < pos("<w:spacing")
            && pos("<w:spacing") < pos("<w:ind")
            && pos("<w:ind") < pos("<w:jc"));
        // 模型读回
        let m = model_of(&out);
        let super::docx_model::Block::Paragraph(p) = &m.body[0] else { panic!() };
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
        assert!(out.contains(r#"<w:rFonts w:ascii="Times"/>"#), "rFonts 原样: {out}");
        assert!(out.contains("<w:u/>"), "无关子元素保留: {out}");
        assert_eq!(out.matches("<w:b/>").count(), 2, "两个 run 都加粗: {out}");
        assert_eq!(out.matches(r#"<w:sz w:val="28"/>"#).count(), 2, "14pt = 28 半磅: {out}");
        assert_eq!(out.matches(r#"<w:color w:val="FF0000"/>"#).count(), 2);
        // run 2 无 rPr → 新建（rPr 是 w:r 的 schema 首子元素）
        assert!(out.contains(r#"<w:r><w:rPr>"#), "无 rPr 的 run 新建 rPr: {out}");
        // 模型读回
        let m = model_of(&out);
        let super::docx_model::Block::Paragraph(p) = &m.body[0] else { panic!() };
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
                    character: Some(CharFormat { bold: Some(true), ..Default::default() }),
                },
                EditOp::SetFormat {
                    block: 2,
                    expect_prefix: "".into(),
                    paragraph: Some(ParaFormat { line_spacing: Some(2.0), ..Default::default() }),
                    character: None,
                },
            ],
        )
        .unwrap();
        assert!(
            out.contains(r#"<w:p><w:pPr><w:spacing w:line="480" w:lineRule="auto"/></w:pPr></w:p>"#),
            "块 2 自闭合空段展开并新建 pPr: {out}"
        );
        let m = model_of(&out);
        assert_eq!(m.body.len(), 2);
        let super::docx_model::Block::Paragraph(p1) = &m.body[0] else { panic!() };
        assert_eq!(p1.props.alignment.as_deref(), Some("both"));
        assert_eq!(p1.props.indent_first_line, Some(480));
        assert_eq!(p1.props.indent_left, None, "负左缩进 i32 → 模型 u32 不收，XML 已写");
        assert!(out.contains(r#"w:left="-240""#), "负左缩进写入: {out}");
        assert_eq!(p1.runs[0].props.bold, Some(true));
        let super::docx_model::Block::Paragraph(p2) = &m.body[1] else { panic!() };
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
                paragraph: Some(ParaFormat { indent_first_line_tw: Some(0), ..Default::default() }),
                character: None,
            }],
        )
        .unwrap();
        assert!(
            out.contains(r#"<w:ind w:firstLine="0" w:firstLineChars="0" w:hanging="0" w:hangingChars="0"/>"#),
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
                paragraph: Some(ParaFormat { indent_first_line_tw: Some(0), ..Default::default() }),
                character: None,
            }],
        )
        .unwrap();
        assert!(
            out2.contains(r#"<w:ind w:firstLine="0" w:firstLineChars="0" w:hanging="0" w:hangingChars="0"/>"#),
            "新建即四零: {out2}"
        );
        // ③ 非零值：chars 压 0、hanging 系移除（写 hanging=0 会反伤——元素内 hanging 优先 firstLine）
        let (out3, _) = apply_edits(
            &wrap(xml),
            &heading_styles(),
            &[EditOp::SetFormat {
                block: 1,
                expect_prefix: "甲".into(),
                paragraph: Some(ParaFormat { indent_first_line_tw: Some(480), ..Default::default() }),
                character: None,
            }],
        )
        .unwrap();
        assert!(
            out3.contains(r#"<w:ind w:firstLine="480" w:firstLineChars="0"/>"#),
            "非零：chars=0: {out3}"
        );
        assert!(!out3.contains("w:hanging"), "非零不写 hanging（反伤 firstLine）: {out3}");
        // ④ 左缩进同律：leftChars=0 压样式层
        let (out4, _) = apply_edits(
            &xml2,
            &Stylesheet::empty(),
            &[EditOp::SetFormat {
                block: 1,
                expect_prefix: "乙".into(),
                paragraph: Some(ParaFormat { indent_left_tw: Some(0), ..Default::default() }),
                character: None,
            }],
        )
        .unwrap();
        assert!(out4.contains(r#"<w:ind w:left="0" w:leftChars="0"/>"#), "左缩进双零: {out4}");
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
        assert_eq!(applied[0].style.as_deref(), Some("h2"), "AppliedOp.style 携带解析后 ID");
        assert!(applied[0].after.contains("样式=heading 2"), "摘要: {}", applied[0].after);
        assert_eq!(out.matches(r#"<w:pStyle w:val="h2"/>"#).count(), 2, "格内两段都换样式: {out}");
        assert!(out.contains(r#"<w:ind w:firstLine="480"/>"#), "既有 ind 保留: {out}");
        let before_last_close = out.rsplit_once("</w:tc>").unwrap().0;
        let neighbor = &out[before_last_close.rfind("<w:tc>").unwrap()..];
        assert!(!neighbor.contains("w:pStyle"), "邻格未动: {neighbor}");
        // 模型读回：格内两段样式都生效
        let m = model_of(&out);
        let super::docx_model::Block::Table(t) = &m.body[0] else { panic!() };
        for b in &t.rows[0].cells[0].blocks {
            let super::docx_model::Block::Paragraph(p) = b else { continue };
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
                paragraph: Some(ParaFormat { indent_first_line_tw: Some(0), ..Default::default() }),
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
        let err = val_msg(apply_edits(&xml, &Stylesheet::empty(), &[empty_op(None, None)]).unwrap_err());
        assert!(err.starts_with("空格式操作"), "实际: {err}");
        let err = val_msg(
            apply_edits(&xml, &Stylesheet::empty(), &[empty_op(Some(ParaFormat::default()), None)]).unwrap_err(),
        );
        assert!(err.starts_with("空格式操作"), "实际: {err}");
        // 对齐白名单
        let err = val_msg(
            apply_edits(
                &xml,
                &Stylesheet::empty(),
                &[empty_op(
                    Some(ParaFormat { align: Some("middle".into()), ..Default::default() }),
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
                    Some(CharFormat { color: Some("RED".into()), ..Default::default() }),
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
                    Some(CharFormat { font_size_pt: Some(999.0), ..Default::default() }),
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
                    Some(ParaFormat { indent_first_line_tw: Some(-10), ..Default::default() }),
                    None,
                )],
            )
            .unwrap_err(),
        );
        assert!(err.starts_with("缩进值无效"), "实际: {err}");
        // 表格块（前缀对准表内文字，确保落在表格拒绝而非指纹不符）
        let tbl = wrap(r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#);
        let err = val_msg(
            apply_edits(
                &tbl,
                &Stylesheet::empty(),
                &[EditOp::SetFormat {
                    block: 1,
                    expect_prefix: "表".into(),
                    paragraph: None,
                    character: Some(CharFormat { bold: Some(true), ..Default::default() }),
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
        use docx_rs::{Docx, Document, Paragraph, Run};
        let document = Document::new()
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第一段")))
            .add_paragraph(Paragraph::new().add_run(Run::new().add_text("第二段")));
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        Docx::new().document(document).build().pack(&mut cursor).unwrap();
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

    fn read_entry_at<R: std::io::Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, i: usize) -> (String, Vec<u8>) {
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
                rows: vec![vec!["列一".into(), "列二".into()], vec!["1".into(), "2".into()]],
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
        let Block::Table(t) = &m.body[1] else { panic!("块 2 应为表格") };
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
        let cell1 = out3.find("一</w:t>").map(|p| out3[..p].matches("<w:p>").count()).unwrap_or(0);
        assert_eq!(cell1, 2, "首格应含两个段落: {}", &out3[out3.find("<w:tbl>").unwrap()..out3.find("<w:tbl>").unwrap() + 300]);
    }

    #[test]
    fn insert_table_after_rejects_bad_rows() {
        let xml = wrap(r#"<w:p><w:r><w:t>锚</w:t></w:r></w:p>"#);
        // 空 rows
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableAfter { block: 1, expect_prefix: "锚".into(), rows: vec![], header: None, table_style: None }],
        ).unwrap_err());
        assert!(err.starts_with("表格数据无效"), "实际: {err}");
        // 非矩形
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableAfter {
                block: 1,
                expect_prefix: "锚".into(),
                rows: vec![vec!["a".into(), "b".into()], vec!["c".into()]],
                header: None,
                table_style: None,
            }],
        ).unwrap_err());
        assert!(err.starts_with("表格数据无效"), "实际: {err}");
        assert!(err.contains("2 行"), "应指明行号: {err}");
        // 超行数上限
        let big: Vec<Vec<String>> = (0..201).map(|i| vec![i.to_string()]).collect();
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableAfter { block: 1, expect_prefix: "锚".into(), rows: big, header: None, table_style: None }],
        ).unwrap_err());
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
        assert_eq!(applied[0].after, "新文\n二段");
        // tcPr 整体保留（tcW + gridSpan）
        assert!(out.contains(r#"<w:gridSpan w:val="2"/>"#), "gridSpan 应保留");
        // 段/字符格式保留 + \n 成两段
        assert!(out.contains(r#"<w:jc w:val="center"/>"#), "段落格式应保留");
        assert!(out.contains("<w:b/>"), "字符格式应保留");
        assert!(out.contains("新文"), "新文本应写入");
        assert!(!out.contains("旧文"), "旧文本应替换");
        assert_eq!(out.matches("<w:p>").count(), 2 + 1, "格 1 两段 + 格 2 一段");
        // 产物可回读且结构不变
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else { panic!() };
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
        assert!(out2.contains(r#"<w:jc w:val="center"/>"#), "清空仍保留段落格式");
        assert!(!out2.contains("旧文"));
    }

    #[test]
    fn set_cell_text_error_families() {
        // 非表格块
        let xml = wrap(r#"<w:p><w:r><w:t>段</w:t></w:r></w:p>"#);
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetCellText { block: 1, expect_prefix: "段".into(), row: 1, cell: 1, text: "x".into() }],
        ).unwrap_err());
        assert!(err.starts_with("非表格块"), "实际: {err}");
        // 行越界 / 格越界
        let xml = two_row_table_doc();
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetCellText { block: 1, expect_prefix: "甲".into(), row: 5, cell: 1, text: "x".into() }],
        ).unwrap_err());
        assert!(err.starts_with("单元格越界"), "实际: {err}");
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetCellText { block: 1, expect_prefix: "甲".into(), row: 1, cell: 3, text: "x".into() }],
        ).unwrap_err());
        assert!(err.starts_with("单元格越界"), "实际: {err}");
        // 纵向合并续格（bare vMerge = continue）——续格放第 2 行，指纹取首行
        let cont_cell = r#"<w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc>"#;
        let cont_row = format!("<w:tr>{}{}</w:tr>", cont_cell, cell_xml("乙二"));
        let xml = wrap(&format!(
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid>{}{}</w:tbl>"#,
            row_xml("甲一", "乙一"),
            cont_row
        ));
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetCellText { block: 1, expect_prefix: "甲一".into(), row: 2, cell: 1, text: "x".into() }],
        ).unwrap_err());
        assert!(err.starts_with("纵向合并续格"), "实际: {err}");
        assert!(err.contains("合并头"), "应指向合并头: {err}");
        // 同一格多操作
        let xml = two_row_table_doc();
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::SetCellText { block: 1, expect_prefix: "甲".into(), row: 1, cell: 1, text: "x".into() },
                EditOp::SetCellText { block: 1, expect_prefix: "甲".into(), row: 1, cell: 1, text: "y".into() },
            ],
        ).unwrap_err());
        assert!(err.starts_with("同一格多操作"), "实际: {err}");
        // 段落操作与表格操作同块冲突
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::SetCellText { block: 1, expect_prefix: "甲".into(), row: 1, cell: 1, text: "x".into() },
                EditOp::DeleteBlock { block: 1, expect_prefix: "甲".into() },
            ],
        ).unwrap_err());
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
        let Block::Table(t) = &m.body[0] else { panic!() };
        assert_eq!(t.rows.len(), 3, "2 行 + 1 克隆");
        // 新行（第 2 行）结构继承：vMerge restart / 格数一致；文本已换
        assert_eq!(t.rows[1].cells[0].v_merge.as_deref(), Some("restart"), "vMerge 应随克隆保留");
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
        let Block::Table(t2) = &m2.body[0] else { panic!() };
        assert_eq!(t2.rows.len(), 3);
        // 新末行文本为空（甲二/乙二 不重复出现三次）
        assert_eq!(out2.matches("甲二").count(), 1, "克隆行文本应清空");
    }

    #[test]
    fn insert_table_row_after_error_families() {
        let xml = two_row_table_doc();
        // 行号越界
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableRowAfter {
                block: 1,
                expect_prefix: "甲".into(),
                after_row: Some(9),
                cells: None,
            }],
        ).unwrap_err());
        assert!(err.starts_with("行号越界"), "实际: {err}");
        // 列数不符
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::InsertTableRowAfter {
                block: 1,
                expect_prefix: "甲".into(),
                after_row: Some(1),
                cells: Some(vec!["只给一格".into(), "两格".into(), "三格".into()]),
            }],
        ).unwrap_err());
        assert!(err.starts_with("列数不符"), "实际: {err}");
        // 非表格块
        let xml_p = wrap(r#"<w:p><w:r><w:t>段</w:t></w:r></w:p>"#);
        let err = val_msg(apply_edits(
            &xml_p,
            &Stylesheet::empty(),
            &[EditOp::InsertTableRowAfter {
                block: 1,
                expect_prefix: "段".into(),
                after_row: None,
                cells: None,
            }],
        ).unwrap_err());
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
        let Block::Table(t) = &m.body[0] else { panic!() };
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
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::ReplaceText { block: 1, expect_prefix: "甲一".into(), new_text: "x".into() }],
        ).unwrap_err());
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
        let tblpr = out.split("<w:tblPr>").nth(1).unwrap().split("</w:tblPr>").next().unwrap();
        assert!(tblpr.contains(r#"<w:tblW w:w="0" w:type="auto"/>"#), "tblW 原样保留");
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
        assert!(applied4[0].after.contains("不存在"), "空转信号: {}", applied4[0].after);
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
                xml: Some(r#"<w:tblBorders><w:insideH w:val="single" w:sz="4"/></w:tblBorders>"#.into()),
            }],
        )
        .unwrap();
        assert!(out.contains(r#"<w:tblPr><w:tblBorders>"#), "空容器展开: {out}");
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
        assert!(out2.contains(r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid>"#), "容器新建于开标签后: {out2}");
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
        assert!(r2.contains(r#"<w:trPr><w:trHeight w:val="400" w:hRule="atLeast"/></w:trPr>"#), "行级容器新建: {out}");
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
        assert!(out2.contains(&format!(r#"<w:tcW w:w="4500" w:type="dxa"/>{SHD_FRAG}"#)), "tcPr 内 schema 位插入");
        // 模型读回：格特征上屏
        let m = model_of(&out2);
        let Block::Table(t) = &m.body[0] else { panic!() };
        assert_eq!(t.rows[0].cells[1].shd_fill.as_deref(), Some("DDEEFF"));
        assert_eq!(t.rows[0].cells[0].shd_fill, None, "只动目标格");
    }

    #[test]
    fn set_table_element_error_families() {
        let xml = two_row_table_doc();
        // 受保护结构属性
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
        assert!(err.starts_with("受保护子元素"), "实际: {err}");
        assert!(err.contains("merge_cells"), "应指路 merge_cells: {err}");
        // 白名单外
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
        assert!(err.starts_with("非法子元素"), "实际: {err}");
        assert!(err.contains("tblpr"), "应指路 tblpr 投影: {err}");
        // level 与 row/cell 组合校验
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
        assert!(err.starts_with("参数校验失败"), "实际: {err}");
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
        assert!(err.starts_with("参数校验失败"), "实际: {err}");
        // 行越界
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
        assert!(err.starts_with("行号越界"), "实际: {err}");
        // 片段校验（根名不符）
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
        assert!(err.contains("tblpr"), "片段校验错应指路 tblpr: {err}");
        // 非表格块
        let xml_p = wrap(r#"<w:p><w:r><w:t>段</w:t></w:r></w:p>"#);
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
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
                paragraph: Some(ParaFormat { align: Some("center".into()), ..Default::default() }),
                character: Some(CharFormat { bold: Some(true), ..Default::default() }),
                style: None,
            }],
        )
        .unwrap();
        assert_eq!(applied[0].op, "set_cell_format");
        assert_eq!(out.matches(r#"<w:jc w:val="center"/>"#).count(), 2, "两段都居中: {out}");
        assert_eq!(out.matches("<w:b/>").count(), 2, "两 run 都加粗（邻格不中枪）: {out}");
        // 邻格（最后一个 tc）原样：无 jc 无 b
        let before_last_close = out.rsplit_once("</w:tc>").unwrap().0;
        let neighbor = &out[before_last_close.rfind("<w:tc>").unwrap()..];
        assert!(neighbor.contains("邻"), "提取到的确是邻格: {neighbor}");
        assert!(!neighbor.contains("w:jc") && !neighbor.contains("<w:b/>"), "邻格未动: {neighbor}");
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
                character: Some(CharFormat { font_size_pt: Some(14.0), ..Default::default() }),
                style: None,
            }],
        )
        .unwrap();
        assert_eq!(out2.matches(r#"<w:sz w:val="28"/>"#).count(), 2, "14pt → sz=28 半磅");
        // 两个都 None → 空格式操作（与 set_format 同家族，文案点名 set_cell_format）
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
        assert!(err.starts_with("空格式操作"), "实际: {err}");
        assert!(err.contains("set_cell_format"), "实际: {err}");
        // 越界家族
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SetCellFormat {
                block: 1,
                expect_prefix: "上".into(),
                row: 1,
                cell: 9,
                paragraph: Some(ParaFormat { align: Some("center".into()), ..Default::default() }),
                character: None,
                style: None,
            }],
        ).unwrap_err());
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
        let Block::Table(t) = &m.body[0] else { panic!() };
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
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
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
        let Block::Table(t) = &m.body[0] else { panic!() };
        assert_eq!(t.rows[0].cells.len(), 1, "两格并一格");
        assert_eq!(t.rows[0].cells[0].grid_span, Some(2), "gridSpan 求和");
        let mut text = String::new();
        docx_model::blocks_text(&t.rows[0].cells[0].blocks, &mut text);
        let joined: String = text.split('\n').collect::<Vec<_>>().join("");
        assert!(joined.contains("甲一") && joined.contains("乙一"), "内容按序拼接: {joined}");
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
        let Block::Table(t) = &m.body[0] else { panic!() };
        assert_eq!(t.rows[0].cells.len(), 2, "拆回 2 格");
        assert_eq!(t.rows[0].cells[0].grid_span, None, "首格回单格");
        assert_eq!(t.rows[0].cells[1].grid_span, None, "补格单格");
        // 内容留首格、补格空段（继承首段格式模板；空 ppr 展开为成对标签）
        assert!(split.contains("甲一") && split.contains("乙一"), "内容留首格");
        assert!(split.contains("<w:p></w:p>"), "补格空段: {split}");
        // 补格结构：同开标签 + 空 tcPr 之外无 gridSpan
        let new_cells = split.split("<w:tr>").nth(1).unwrap().split("</w:tr>").next().unwrap();
        assert_eq!(new_cells.matches("<w:tc>").count(), 2);
    }

    #[test]
    fn merge_split_error_families() {
        let xml = vmerged_doc();
        // span < 2
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
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
        ).is_ok());
        // 纵并 onto 续格 → 指路合并头
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
        assert!(err.starts_with("纵向合并续格"), "实际: {err}");
        // split vertical 指到续格 → 专属家族 + 指路合并头（比泛化的非合并格更可行动）
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SplitCell {
                block: 1,
                expect_prefix: "头".into(),
                direction: MergeDirection::Vertical,
                row: 2,
                cell: 1,
            }],
        ).unwrap_err());
        assert!(err.starts_with("纵向合并续格"), "实际: {err}");
        assert!(err.contains("合并头"), "应指路合并头: {err}");
        // split horizontal 单格 → 非合并格
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::SplitCell {
                block: 1,
                expect_prefix: "头".into(),
                direction: MergeDirection::Horizontal,
                row: 1,
                cell: 2,
            }],
        ).unwrap_err());
        assert!(err.starts_with("非合并格"), "实际: {err}");
        // 行越界
        let err = val_msg(apply_edits(
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
        ).unwrap_err());
        // 行越界归并进跨度家族（首行前缀稳定 + 报行数事实）
        assert!(err.starts_with("合并跨度无效"), "实际: {err}");
        assert!(err.contains("越界"), "实际: {err}");
    }

    #[test]
    fn merge_split_exclusive_per_batch() {
        let xml = two_row_table_doc();
        // merge 同批挂 set_cell_text → 拒（结构重构独占）
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::SetCellText { block: 1, expect_prefix: "甲一".into(), row: 1, cell: 1, text: "x".into() },
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
        ).unwrap_err());
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
        assert!(err.contains("独占"), "应解释独占: {err}");
        // 反序：merge 在前，后挂表格操作 → 拒
        let err = val_msg(apply_edits(
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
                EditOp::SetCellText { block: 1, expect_prefix: "甲一".into(), row: 1, cell: 1, text: "x".into() },
            ],
        ).unwrap_err());
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
        assert!(err.contains("重新 inspect_docx"), "应指路重寻址: {err}");
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
                    character: Some(CharFormat { bold: Some(true), ..Default::default() }),
                    style: None,
                },
                EditOp::SetCellText { block: 1, expect_prefix: "甲一".into(), row: 2, cell: 2, text: "新乙二".into() },
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
        let xml = wrap(&tbl_of(&[row_xml("甲一", "乙一"), row_xml("甲二", "乙二"), row_xml("甲三", "乙三")]));
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[EditOp::DeleteTableRow { block: 1, expect_prefix: "甲一".into(), row: 2 }],
        )
        .unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].op, "delete_table_row");
        assert!(applied[0].after.contains("剩 2 行"), "摘要带剩余行数: {}", applied[0].after);
        assert!(!out.contains("甲二"), "目标行内容应删除");
        assert!(out.contains("甲一") && out.contains("甲三"), "其余行不中枪");
        // 块数不变（表仍是 1 块）、行数 3→2、tblGrid 原样
        let m = model_of(&out);
        assert_eq!(m.body.len(), 1);
        let Block::Table(t) = &m.body[0] else { panic!() };
        assert_eq!(t.rows.len(), 2);
        assert_eq!(out.matches("<w:gridCol").count(), 2, "tblGrid 不动");
    }

    #[test]
    fn delete_table_row_guards() {
        // ① 末行保护：单行表删唯一行 → 拒、指路 delete_block
        let one = wrap(&tbl_of(&[row_xml("仅", "一")]));
        let err = val_msg(apply_edits(
            &one,
            &Stylesheet::empty(),
            &[EditOp::DeleteTableRow { block: 1, expect_prefix: "仅".into(), row: 1 }],
        ).unwrap_err());
        assert!(err.starts_with("空表保护"), "实际: {err}");
        assert!(err.contains("delete_block"), "应指路整表删除: {err}");

        // ② 合并头行（下方有续格）→ 拒、指路 split_cell
        let err = val_msg(apply_edits(
            &vchain_doc(),
            &Stylesheet::empty(),
            &[EditOp::DeleteTableRow { block: 1, expect_prefix: "头".into(), row: 1 }],
        ).unwrap_err());
        assert!(err.starts_with("合并结构冲突"), "实际: {err}");
        assert!(err.contains("split_cell"), "应指路先拆纵并: {err}");

        // ③ 纯续格行可删（头在上方，链条缩短仍合法）
        let (out, applied) = apply_edits(
            &vchain_doc(),
            &Stylesheet::empty(),
            &[EditOp::DeleteTableRow { block: 1, expect_prefix: "头".into(), row: 2 }],
        )
        .unwrap();
        assert_eq!(applied.len(), 1);
        let m = model_of(&out);
        let Block::Table(t) = &m.body[0] else { panic!() };
        assert_eq!(t.rows.len(), 2, "删续格行后剩 2 行");
        assert_eq!(t.rows[0].cells[0].v_merge.as_deref(), Some("restart"), "头保留");
        assert!(!out.contains("藏一"), "续格内容随行删除");

        // ④ 独占一批：同批挂 set_cell_text → 拒
        let err = val_msg(apply_edits(
            &two_row_table_doc(),
            &Stylesheet::empty(),
            &[
                EditOp::SetCellText { block: 1, expect_prefix: "甲一".into(), row: 1, cell: 1, text: "x".into() },
                EditOp::DeleteTableRow { block: 1, expect_prefix: "甲一".into(), row: 2 },
            ],
        ).unwrap_err());
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
        assert!(err.contains("delete_table_row"), "应点名结构重构家族: {err}");

        // ⑤ 行号越界
        let err = val_msg(apply_edits(
            &two_row_table_doc(),
            &Stylesheet::empty(),
            &[EditOp::DeleteTableRow { block: 1, expect_prefix: "甲一".into(), row: 5 }],
        ).unwrap_err());
        assert!(err.starts_with("行号越界"), "实际: {err}");
    }

    #[test]
    fn set_table_element_same_cell_different_elements_compose() {
        // 同格不同 element 一批组合（vAlign + tcBorders）——生产反馈「拆两批」修正
        let xml = two_row_table_doc();
        let borders = r#"<w:tcBorders><w:top w:val="single" w:sz="4" w:color="auto"/></w:tcBorders>"#;
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
        let Block::Table(t) = &m.body[0] else { panic!() };
        assert_eq!(t.rows[0].cells[1].v_align, None, "右格不中枪");

        // 同元素两条 → 拒「同一格多操作」
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::SetTableElement {
                    block: 1, expect_prefix: "甲一".into(), level: TableLevel::Cell,
                    row: Some(1), cell: Some(1), element: "vAlign".into(),
                    xml: Some(r#"<w:vAlign w:val="center"/>"#.into()),
                },
                EditOp::SetTableElement {
                    block: 1, expect_prefix: "甲一".into(), level: TableLevel::Cell,
                    row: Some(1), cell: Some(1), element: "vAlign".into(),
                    xml: Some(r#"<w:vAlign w:val="bottom"/>"#.into()),
                },
            ],
        ).unwrap_err());
        assert!(err.starts_with("同一格多操作"), "实际: {err}");
        assert!(err.contains("vAlign"), "应点名冲突元素: {err}");

        // set_cell_text + set_table_element 同格（不同目标键）→ 合法组合
        let (out2, _) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::SetCellText { block: 1, expect_prefix: "甲一".into(), row: 1, cell: 1, text: "新甲一".into() },
                EditOp::SetTableElement {
                    block: 1, expect_prefix: "甲一".into(), level: TableLevel::Cell,
                    row: Some(1), cell: Some(1), element: "vAlign".into(),
                    xml: Some(r#"<w:vAlign w:val="center"/>"#.into()),
                },
            ],
        )
        .unwrap();
        assert!(out2.contains("新甲一") && out2.contains(r#"<w:vAlign w:val="center"/>"#));

        // set_cell_text 同格两条仍拒（重写语义）
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::SetCellText { block: 1, expect_prefix: "甲一".into(), row: 1, cell: 1, text: "a".into() },
                EditOp::SetCellText { block: 1, expect_prefix: "甲一".into(), row: 1, cell: 1, text: "b".into() },
            ],
        ).unwrap_err());
        assert!(err.starts_with("同一格多操作"), "实际: {err}");
    }

    #[test]
    fn insert_paragraph_after_same_anchor_chains_in_order() {
        let xml = wrap(r#"<w:p><w:r><w:t>锚</w:t></w:r></w:p>"#);
        let (out, applied) = apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::InsertParagraphAfter { block: 1, expect_prefix: "锚".into(), text: "一".into(), style: None },
                EditOp::InsertParagraphAfter { block: 1, expect_prefix: "锚".into(), text: "二".into(), style: None },
                EditOp::InsertParagraphAfter { block: 1, expect_prefix: "锚".into(), text: "三".into(), style: None },
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
        let xml2 = wrap(r#"<w:p><w:r><w:t>锚</w:t></w:r></w:p><w:p><w:r><w:t>他段</w:t></w:r></w:p>"#);
        let (out2, applied2) = apply_edits(
            &xml2,
            &Stylesheet::empty(),
            &[
                EditOp::InsertParagraphAfter { block: 1, expect_prefix: "锚".into(), text: "甲".into(), style: None },
                EditOp::InsertParagraphAfter { block: 1, expect_prefix: "锚".into(), text: "乙".into(), style: None },
                EditOp::ReplaceText { block: 2, expect_prefix: "他段".into(), new_text: "改段".into() },
            ],
        )
        .unwrap();
        assert_eq!(applied2.len(), 3);
        assert!(out2.contains("改段"));
        let m2 = model_of(&out2);
        assert_eq!(m2.body.len(), 4);

        // 同锚混其他段落操作仍拒（链式例外只对 insert_paragraph_after 开放）
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::InsertParagraphAfter { block: 1, expect_prefix: "锚".into(), text: "一".into(), style: None },
                EditOp::ReplaceText { block: 1, expect_prefix: "锚".into(), new_text: "改".into() },
            ],
        ).unwrap_err());
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
        // 反序：replace 在前，insert 在后 → 同样拒
        let err = val_msg(apply_edits(
            &xml,
            &Stylesheet::empty(),
            &[
                EditOp::ReplaceText { block: 1, expect_prefix: "锚".into(), new_text: "改".into() },
                EditOp::InsertParagraphAfter { block: 1, expect_prefix: "锚".into(), text: "一".into(), style: None },
            ],
        ).unwrap_err());
        assert!(err.starts_with("同一块多操作"), "实际: {err}");
    }
}
