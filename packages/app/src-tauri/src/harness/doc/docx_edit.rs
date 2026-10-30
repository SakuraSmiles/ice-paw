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

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::Serialize;

use crate::error::{AppError, AppResult};

use super::docx_model::{self, Block};
use super::docx_pkg::{self, ImagePayload};
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
            Err(e) => return Err(AppError::Internal(format!("document.xml 扫描失败: {e}"))),
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
    ReplaceText {
        block: usize,
        expect_prefix: String,
        new_text: String,
    },
    /// 在锚块后插入新段：style（显示名，可选）指定样式；缺省继承锚块段落格式。
    /// 同锚块一批可多条链式（S3 七波：按输入序连续排列——写「标识行+描述行+
    /// 属性行」这类多段条目一批搞定，不再逐段拆批重寻址）。
    InsertParagraphAfter {
        block: usize,
        expect_prefix: String,
        text: String,
        style: Option<String>,
    },
    /// 删除整块（含段落标记）；块内含 sectPr（节属性载体）拒绝。
    DeleteBlock { block: usize, expect_prefix: String },
    /// 改段落样式（标题升降级等）：只动 pStyle 一个元素，正文 run 字节不动；
    /// 表格块拒绝。style 接受显示名或样式 ID（inspect_docx outline 样式列口径）。
    SetStyle {
        block: usize,
        expect_prefix: String,
        style: String,
    },
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
    /// 在锚块后插入图片（D18 十波）：`image` 由工具壳 load_image 装载（字节/宽高/
    /// 格式 png|jpeg）；`rid`/`cx_emu`/`cy_emu`/`docpr_id` 由 zip 层编排**注入**
    /// （扫包 max+1 分配 + 版心宽换算），工具层置零占位、xml 层不消费工具层值。
    /// width_mm 显式宽（毫米，钳版心）；缺省 min(原生像素宽, 版心)（不放大小图）。
    /// 图片源是读侧第二路径（同 template 先例：只读不授权 + 格式/大小闸在装载层）。
    InsertImageAfter {
        block: usize,
        expect_prefix: String,
        image: ImagePayload,
        width_mm: Option<f64>,
        /// 注入：关系 Id（如 rId7）——xml 层写 a:blip r:embed
        rid: String,
        /// 注入：目标宽/高（EMU）
        cx_emu: u64,
        cy_emu: u64,
        /// 注入：docPr 唯一 id（扫现有 max+1 + 批内序）
        docpr_id: u32,
    },
    /// 在锚块后插入 TOC 域段（D18 十波）：**裸 fldSimple 单段**（非 sdt 包裹——
    /// walk_blocks 会摊平 sdt 破坏「1 输入块 = 1 产物块」）；cached result 放
    /// 自愈文案，Word 带 updateFields（zip 层自动置）打开即刷新，WPS 不保证
    /// （F9 手动刷新兜底）。levels 1-9（目录深度）；hyperlink 目录项超链接。
    InsertTocAfter {
        block: usize,
        expect_prefix: String,
        levels: u32,
        hyperlink: bool,
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
    "pStyle",
    "keepNext",
    "keepLines",
    "pageBreakBefore",
    "framePr",
    "widowControl",
    "numPr",
    "suppressLineNumbers",
    "pBdr",
    "shd",
    "tabs",
    "suppressAutoHyphens",
    "kinsoku",
    "wordWrap",
    "overflowPunct",
    "topLinePunct",
    "autoSpaceDE",
    "autoSpaceDN",
    "bidi",
    "adjustRightInd",
    "snapToGrid",
    "spacing",
    "ind",
    "contextualSpacing",
    "mirrorIndents",
    "suppressOverlap",
    "jc",
    "textDirection",
    "textAlignment",
    "textboxTightWrap",
    "outlineLvl",
    "divId",
    "cnfStyle",
    "rPr",
];

/// CT_TblPrBase 法定子元素，ECMA-376 schema 序（set_table_element level=table；
/// def_edit 的表样式 tblPr 容器复用本表，pub(super)）。
pub(super) const TBLPR_ELEMENTS: [&str; 17] = [
    "tblStyle",
    "tblpPr",
    "tblOverlap",
    "bidiVisual",
    "tblStyleRowBandSize",
    "tblStyleColBandSize",
    "tblW",
    "jc",
    "tblCellSpacing",
    "tblInd",
    "tblBorders",
    "shd",
    "tblLayout",
    "tblCellMar",
    "tblLook",
    "tblCaption",
    "tblDescription",
];

/// CT_TrPr 法定子元素（set_table_element level=row）。
const TRPR_ELEMENTS: [&str; 12] = [
    "cnfStyle",
    "divId",
    "gridBefore",
    "gridAfter",
    "wBefore",
    "wAfter",
    "cantSplit",
    "trHeight",
    "tblHeader",
    "tblCellSpacing",
    "jc",
    "hidden",
];

/// CT_TcPr 法定子元素（set_table_element level=cell）。
const TCPR_ELEMENTS: [&str; 13] = [
    "cnfStyle",
    "tcW",
    "gridSpan",
    "hMerge",
    "vMerge",
    "tcBorders",
    "shd",
    "noWrap",
    "tcMar",
    "textDirection",
    "tcFitText",
    "vAlign",
    "hideMark",
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
///
/// D18 十波包级编排：批内含 insert_image_after / insert_toc_after 时，先扫包
/// 规划增补（rId/media 名/CT/settings，见 docx_pkg），把分配结果**注入** op 克隆
/// （xml 层保持纯函数——只消费注入后的 rid/extent/docPr）；产物重打包走多部件
/// 通道（document.xml + 包级增补件）。纯段落批次零开销早退（不扫包、原路径）。
pub fn apply_edits_to_bytes_locked(
    bytes: &[u8],
    ops: &[EditOp],
    allowed_blocks: Option<(usize, usize)>,
) -> AppResult<(Vec<u8>, Vec<AppliedOp>)> {
    let xml = super::docx::read_document_xml(bytes)?;
    let styles = match super::docx::read_entry(bytes, "word/styles.xml")? {
        Some(s) => super::styles::parse_styles(&super::xml_dom::parse(&s)?),
        None => Stylesheet::empty(),
    };

    // ---- 包级增补编排（仅批内有图/TOC 才走；二次解析 document.xml 拿版心宽与
    // docPr 基号——注入需要，正确性优先）----
    let has_pkg_image = ops
        .iter()
        .any(|o| matches!(o, EditOp::InsertImageAfter { .. }));
    let has_toc = ops
        .iter()
        .any(|o| matches!(o, EditOp::InsertTocAfter { .. }));
    let mut additions = docx_pkg::PkgAdditions::default();
    let prepared: std::borrow::Cow<[EditOp]> = if has_pkg_image || has_toc {
        let images: Vec<&ImagePayload> = ops
            .iter()
            .filter_map(|o| match o {
                EditOp::InsertImageAfter { image, .. } => Some(image),
                _ => None,
            })
            .collect();
        let (allocs, plan) = docx_pkg::plan_package_additions(bytes, &images, has_toc)?;
        additions = plan;
        let dom = super::xml_dom::parse(&xml)?;
        let model = docx_model::build_document(&dom);
        let width_tw = content_width_twips(&model);
        let docpr_base = docx_pkg::next_docpr_id(&xml);
        std::borrow::Cow::Owned(inject_package_resolved(ops, &allocs, width_tw, docpr_base))
    } else {
        std::borrow::Cow::Borrowed(ops)
    };

    let (new_xml, applied) = apply_edits_locked(&xml, &styles, &prepared, allowed_blocks)?;
    let out = if additions.replacements.is_empty() && additions.appends.is_empty() {
        repack_part(bytes, "word/document.xml", &new_xml)?
    } else {
        let mut replacements = vec![("word/document.xml".to_string(), new_xml)];
        replacements.extend(additions.replacements);
        docx_pkg::repack_package(bytes, &replacements, &additions.appends)?
    };
    Ok((out, applied))
}

/// 把包级分配结果注入 op 克隆：每图拿 allocs[i]（rId/media 对应）+ 版心宽换算的
/// extent + docPr 基号递增。工具层/测试直调的占位值一律被覆盖（唯一注入点，
/// 不存在陈旧 rid 路径）；其余 op 原样克隆。
fn inject_package_resolved(
    ops: &[EditOp],
    allocs: &[docx_pkg::ImageAlloc],
    width_tw: u32,
    docpr_base: u32,
) -> Vec<EditOp> {
    let mut alloc_i = 0usize;
    let mut pic_i = 0u32;
    ops.iter()
        .map(|op| match op {
            EditOp::InsertImageAfter {
                block,
                expect_prefix,
                image,
                width_mm,
                ..
            } => {
                let alloc = allocs
                    .get(alloc_i)
                    .expect("allocs 与批内图片数一致（plan_package_additions 同源产出）");
                alloc_i += 1;
                pic_i += 1;
                let (cx, cy) =
                    docx_pkg::compute_extent(image.width_px, image.height_px, *width_mm, width_tw);
                EditOp::InsertImageAfter {
                    block: *block,
                    expect_prefix: expect_prefix.clone(),
                    image: image.clone(),
                    width_mm: *width_mm,
                    rid: alloc.rid.clone(),
                    cx_emu: cx,
                    cy_emu: cy,
                    docpr_id: docpr_base + pic_i - 1,
                }
            }
            other => other.clone(),
        })
        .collect()
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
    // 可组合），内容/格式手术每格限一条。结构重构（merge_cells / split_cell /
    // delete_table_row）走**表内地址模拟器**（十一波，取代类型黑名单独占）：每个
    // 操作声明足迹 = 其改写地址布局所及的行区间，足迹不相交 ⇒ 各操作预检所读的行
    // 不被他人改写 ⇒ 原模型寻址校验依然成立（序无关，D14 组合判据机械化）；相交
    // 才拒并点名冲突对。结构操作与内容/段落操作同块仍互斥（保守边界：内容格寻址
    // 依赖格数账本，模拟器未覆盖其寻址前不组合）。
    let mut used_blocks: Vec<usize> = Vec::new();
    let mut table_modified: Vec<usize> = Vec::new();
    let mut used_cells: Vec<(usize, usize, usize, String)> = Vec::new(); // (block, row, cell, 目标键)
    let mut insert_anchors: Vec<usize> = Vec::new(); // 链式插入锚块（同块多条 insert_paragraph_after）
                                                     // 表内地址模拟器：block → 已声明足迹（操作序, 行区间, 标签）
    let mut structural_rows: HashMap<usize, Vec<(usize, std::ops::RangeInclusive<usize>, String)>> =
        HashMap::new();
    // 表格批内虚拟行状态（block → (每行格数, 虚拟行的原模型模板行)）：
    // 同块「先增行后填格」合法——寻址按批内输入序生效，结构与格数继承模板行
    let mut table_state: HashMap<usize, (Vec<usize>, Vec<Option<usize>>)> = HashMap::new();
    for (op_i, op) in ops.iter().enumerate() {
        // ClearBody 不寻址块——先于块号提取处理（独占一批 + 块数指纹）
        if let EditOp::ClearBody { expect_blocks } = op {
            if allowed_blocks.is_some() {
                // 锁检查够不到 continue 之后——在此特判：清空正文作用于全文，
                // 与任何区间语义冲突（D15 八波②）
                return Err(AppError::Validation(
                    "区间外块: clear_body 清空全部正文，与 allowed_blocks 区间锁冲突。\
                     带锁批次只允许区间内的块级操作。如确需清空正文，请去掉 \
                     allowed_blocks 重发。"
                        .into(),
                ));
            }
            if ops.len() != 1 {
                return Err(AppError::Validation(
                    "同一批多操作: clear_body 须独占一批（清空后一切块号失效，\
                     与其他操作组合必然寻址错乱）。请拆批：先 clear_body，\
                     再重新 inspect_docx 寻址写入。"
                        .into(),
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
            | EditOp::InsertImageAfter { block, .. }
            | EditOp::InsertTocAfter { block, .. }
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
            // 结构重构与内容/段落操作同块互斥（保守边界，见预检头注释）
            if used_blocks.contains(&block) || table_modified.contains(&block) {
                return Err(AppError::Validation(format!(
                    "同一块多操作: 块 {block} 已被其他操作引用。merge_cells / split_cell / \
                     delete_table_row 使行列地址重排，不能与内容/段落操作同批——请拆批，\
                     结构改完再寻址。"
                )));
            }
            // 表内地址模拟器：声明足迹，与本表已声明足迹求交（相交才拒）
            let (footprint, label) = structural_footprint_of(op, &model.body[block - 1]);
            for (j, prev, prev_label) in structural_rows
                .get(&block)
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                if ranges_overlap(prev, &footprint) {
                    return Err(AppError::Validation(format!(
                        "结构足迹冲突: 块 {block} 操作 {}（{label}）改写第 {}..={} 行布局，与\
                         操作 {}（{prev_label}，第 {}..={} 行）足迹相交。同批结构操作须作用于\
                         不相交的行区间——请拆批或调整行范围。网格视图用 inspect_docx \
                         projection=table。",
                        op_i + 1,
                        footprint.start(),
                        footprint.end(),
                        j + 1,
                        prev.start(),
                        prev.end()
                    )));
                }
            }
            structural_rows
                .entry(block)
                .or_default()
                .push((op_i, footprint, label));
        } else if structural_rows.contains_key(&block) {
            return Err(AppError::Validation(format!(
                "同一块多操作: 块 {block} 已挂 merge_cells / split_cell / delete_table_row\
                 （结构重构）。请拆批：结构改完重新 inspect_docx projection=table 寻址。"
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
            } else if matches!(
                op,
                EditOp::InsertParagraphAfter { .. }
                    | EditOp::InsertImageAfter { .. }
                    | EditOp::InsertTocAfter { .. }
            ) {
                // 链式插入（S3 七波；D18 扩图/域）：同锚块多条插入类操作按输入序
                // 连续排列（apply 侧聚合进一个插入 splice）；与其他段落/锚/表格操作仍互斥
                if used_blocks.contains(&block) || table_modified.contains(&block) {
                    return Err(AppError::Validation(format!(
                        "同一块多操作: 块 {block} 已被段落操作/锚操作/表格操作引用，\
                         不能再挂插入类操作。请拆批。"
                    )));
                }
                if !insert_anchors.contains(&block) {
                    insert_anchors.push(block);
                }
            } else {
                if used_blocks.contains(&block) {
                    return Err(AppError::Validation(format!(
                        "同一块多操作: 块 {block} 在本批中被多次引用。每块每批限一个操作\
                         （例外：同一锚块可挂多条 insert_paragraph_after / insert_image_after / \
                         insert_toc_after 按序链式插入；表格块可挂多个 set_cell_text / \
                         set_cell_format / set_table_element / insert_table_row_after）；请拆成多批。"
                    )));
                }
                if insert_anchors.contains(&block) {
                    return Err(AppError::Validation(format!(
                        "同一块多操作: 块 {block} 已挂链式插入类操作\
                         （insert_paragraph_after / insert_image_after / insert_toc_after），\
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
            EditOp::SetFormat {
                paragraph,
                character,
                ..
            } => {
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
            EditOp::InsertTableAfter {
                rows, table_style, ..
            } => {
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
            EditOp::InsertImageAfter { .. } => {
                // 图片字节/格式/大小闸在工具壳 load_image（docx_pkg）；注入字段
                // （rid/extent/docpr）由编排层统一分配——预检无可校验项。锚块带
                // 修订不拒绝：插入不触碰锚块内容（与 insert_paragraph_after 同判）
            }
            EditOp::InsertTocAfter { levels, .. } => {
                // 边界显式拒（勿静默 clamp——agent 传 0/10 多半是参数拼错，
                // 静默修成 1/9 会产出与意图不符的目录深度）。AppError::Validation
                // Display 全局包「参数校验失败:」前缀，勿手动重复
                if *levels == 0 || *levels > 9 {
                    return Err(AppError::Validation(format!(
                        "levels={levels} 越界（合法 1-9，目录收录的标题深度）。请复核后重试。"
                    )));
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
                    &model,
                    idx,
                    block,
                    *row,
                    *cell,
                    "set_cell_text",
                    "",
                    &mut table_state,
                    &mut used_cells,
                )?;
            }
            EditOp::InsertTableRowAfter {
                after_row, cells, ..
            } => {
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
            EditOp::SetCellFormat {
                row,
                cell,
                paragraph,
                character,
                style,
                ..
            } => {
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
                validate_formats(
                    paragraph.as_ref(),
                    character.as_ref(),
                    style.is_some(),
                    "set_cell_format",
                )?;
                precheck_cell_target(
                    &model,
                    idx,
                    block,
                    *row,
                    *cell,
                    "set_cell_format",
                    "",
                    &mut table_state,
                    &mut used_cells,
                )?;
            }
            EditOp::SetTableElement {
                level,
                row,
                cell,
                element,
                xml,
                ..
            } => {
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
                            &model,
                            idx,
                            block,
                            r,
                            c,
                            "set_table_element",
                            element.as_str(),
                            &mut table_state,
                            &mut used_cells,
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
                        element,
                        container,
                        whitelist.join(" ")
                    )));
                }
                if let Some(x) = xml {
                    validate_fragment(element, x, "tblpr")?;
                }
            }
            // clear_body 已在上文 continue——本 match 只覆盖寻址类操作
            EditOp::ClearBody { .. } => unreachable!("clear_body 已在上文 continue"),
            EditOp::MergeCells {
                direction,
                row,
                cell,
                span,
                end_row,
                end_cell,
                ..
            } => {
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
                             简单线并用 direction+span，矩形区用 end_row+end_cell，二选一。"
                                .into(),
                        ));
                    }
                    (true, None, None) => {
                        let (Some(er), Some(ec)) = (*end_row, *end_cell) else {
                            return Err(AppError::Validation(
                                "参数缺失: 矩形合并须同时给 end_row 与 end_cell（区域右下角，\
                                 与 (row,cell) 左上角配对）。"
                                    .into(),
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
                             + 可选 span）或 end_row+end_cell（矩形区域），二选一。"
                                .into(),
                        ));
                    }
                    (false, None, Some(_)) => {
                        return Err(AppError::Validation(
                            "参数冲突: span 须与 direction 同用（简单线并）。矩形区合并\
                             请改用 end_row+end_cell（不带 span）。"
                                .into(),
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
                                if head.cells[*cell - 1]
                                    .blocks
                                    .iter()
                                    .any(|b| matches!(b, Block::Table(_)))
                                {
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
                                            if c.blocks.iter().any(|b| matches!(b, Block::Table(_)))
                                            {
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
            EditOp::SplitCell {
                direction,
                row,
                cell,
                ..
            } => {
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
        ) && span.is_table
        {
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
    /// 同锚链式插入聚合（S3 七波 insert_paragraph_after；D18 扩图/域共用）：首条
    /// 建插入 splice（锚块末尾，区间外），后续条直接追加——整链一个插入 blob，
    /// 链序 = 输入序。不按「span.end + 累计长度」定位：那会伸进相邻块的 splice
    /// 区间（块间缝隙可为 0）。
    fn chain_into_insert_splice(
        plan: &mut Vec<Splice>,
        insert_plan_idx: &mut HashMap<usize, usize>,
        block: usize,
        anchor_end: usize,
    ) -> usize {
        if let Some(&i) = insert_plan_idx.get(&block) {
            return i;
        }
        plan.push(Splice {
            pos: anchor_end,
            remove_end: anchor_end,
            insert: String::new(),
            summaries: Vec::new(),
        });
        insert_plan_idx.insert(block, plan.len() - 1);
        plan.len() - 1
    }
    /// 同表多操作聚合（十一波起含结构操作链）：块号 → plan 索引——首触达建
    /// 替换 splice（以原块为底），后续操作按输入序改写 insert 文本。结构操作
    /// 与内容/段落操作经预检互斥，不会共享 entry；同族操作（结构足迹不相交 /
    /// 格去重）按输入序链式应用，互不越界。
    fn table_splice_entry(
        plan: &mut Vec<Splice>,
        table_plan_idx: &mut HashMap<usize, usize>,
        block: usize,
        span: BlockSpan,
        xml: &str,
    ) -> usize {
        if let Some(&i) = table_plan_idx.get(&block) {
            return i;
        }
        plan.push(Splice {
            pos: span.start,
            remove_end: span.end,
            insert: xml[span.start..span.end].to_string(),
            summaries: Vec::new(),
        });
        table_plan_idx.insert(block, plan.len() - 1);
        plan.len() - 1
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
            EditOp::ReplaceText {
                block, new_text, ..
            } => {
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
            EditOp::InsertParagraphAfter {
                block, text, style, ..
            } => {
                let span = spans[block - 1];
                let anchor_has_revision = has_revision(&model.body[block - 1]);
                let new_block = build_inserted_paragraph(
                    xml,
                    span,
                    &text,
                    style.as_deref(),
                    styles,
                    anchor_has_revision,
                );
                // 同锚链式（S3 七波）：聚合细节见 chain_into_insert_splice
                let entry =
                    chain_into_insert_splice(&mut plan, &mut insert_plan_idx, block, span.end);
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
            EditOp::InsertImageAfter {
                block,
                rid,
                cx_emu,
                cy_emu,
                docpr_id,
                ..
            } => {
                let span = spans[block - 1];
                // 图段不继承锚块 pPr/rPr（结构内容非文本，docDefaults 好默认；
                // 继承会把锚的缩进/字体带进图片段）
                let new_block = docx_pkg::build_drawing_paragraph(
                    &rid,
                    cx_emu,
                    cy_emu,
                    docpr_id,
                    &format!("图片 {docpr_id}"),
                );
                let entry =
                    chain_into_insert_splice(&mut plan, &mut insert_plan_idx, block, span.end);
                plan[entry].insert.push_str(&new_block);
                plan[entry].summaries.push(AppliedOp {
                    op: "insert_image_after",
                    block,
                    before: projected_of(&model, block),
                    after: format!("图片 {rid}（{}×{} EMU）", cx_emu, cy_emu),
                    style: None,
                    style_unchanged: None,
                    target: None,
                });
            }
            EditOp::InsertTocAfter {
                block,
                levels,
                hyperlink,
                ..
            } => {
                let span = spans[block - 1];
                let new_block = docx_pkg::build_toc_paragraph(levels, hyperlink);
                let entry =
                    chain_into_insert_splice(&mut plan, &mut insert_plan_idx, block, span.end);
                plan[entry].insert.push_str(&new_block);
                plan[entry].summaries.push(AppliedOp {
                    op: "insert_toc_after",
                    block,
                    before: projected_of(&model, block),
                    after: "TOC 域（打开文档自动刷新；WPS 需全选按 F9）".into(),
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
                let new_block = restyle_paragraph(&xml[span.start..span.end], style_id)
                    .ok_or_else(|| {
                        AppError::Internal(format!(
                            "段落改样式失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        ))
                    })?;
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
            EditOp::SetFormat {
                block,
                paragraph,
                character,
                ..
            } => {
                let span = spans[block - 1];
                let mut new_block = xml[span.start..span.end].to_string();
                if let Some(para) = &paragraph {
                    new_block = reformat_ppr(&new_block, para).ok_or_else(|| {
                        AppError::Internal(format!(
                            "段落格式手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        ))
                    })?;
                }
                if let Some(ch) = &character {
                    new_block = reformat_runs(&new_block, ch).ok_or_else(|| {
                        AppError::Internal(format!(
                            "字符格式手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        ))
                    })?;
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
                        after: truncate(
                            &describe_formats(paragraph.as_ref(), character.as_ref()),
                            60,
                        ),
                        style: None,
                        style_unchanged: None,
                        target: None,
                    }],
                });
            }
            EditOp::SetPprElement {
                block,
                element,
                xml: frag,
                ..
            } => {
                let span = spans[block - 1];
                let (new_block, changed) =
                    set_ppr_element(&xml[span.start..span.end], &element, frag.as_deref())
                        .ok_or_else(|| {
                            AppError::Internal(format!(
                                "pPr元素手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                            ))
                        })?;
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
            EditOp::InsertTableAfter {
                block,
                rows,
                header,
                table_style,
                ..
            } => {
                let span = spans[block - 1];
                // 预检已验存在 + 类型；此处取 ID 挂 tblStyle（批内样式表不可变，无 TOCTOU）
                let style_id = table_style
                    .as_deref()
                    .and_then(|s| styles.table_style_id(s));
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
            EditOp::SetCellText {
                block,
                row,
                cell,
                text,
                ..
            } => {
                let span = spans[block - 1];
                // 同表聚合：首次触达建 splice（以原块为底），后续操作改写 insert
                let entry = table_splice_entry(&mut plan, &mut table_plan_idx, block, span, xml);
                let cur = plan[entry].insert.clone();
                let (new_xml, aligned) = set_cell_text(&cur, row, cell, &text)
                    .ok_or_else(|| AppError::Internal(format!(
                        "单元格手术失败: 块 {block} 第 {row} 行第 {cell} 格 XML 形态异常（内部 bug，未写盘）"
                    )))?;
                plan[entry].insert = new_xml;
                // 摘要披露（重写保真到位置对齐粒度）：多段才有信息量
                let mut after = truncate(&text, 40);
                if text.contains('\n') {
                    after.push_str(if aligned {
                        "（格式按位继承）"
                    } else {
                        "（段数不对齐，格式回落首段模板）"
                    });
                }
                plan[entry].summaries.push(AppliedOp {
                    op: "set_cell_text",
                    block,
                    before: cell_projected_of(&model, block, row, cell),
                    after,
                    style: None,
                    style_unchanged: None,
                    target: None,
                });
            }
            EditOp::InsertTableRowAfter {
                block,
                after_row,
                cells,
                ..
            } => {
                let span = spans[block - 1];
                let entry = table_splice_entry(&mut plan, &mut table_plan_idx, block, span, xml);
                let cur = plan[entry].insert.clone();
                let (new_xml, any_fallback) =
                    insert_table_row_after(&cur, after_row, cells.as_deref()).ok_or_else(|| {
                        AppError::Internal(format!(
                            "表格增行失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        ))
                    })?;
                plan[entry].insert = new_xml;
                let filled = cells.as_ref().map(|c| c.len()).unwrap_or(0);
                let mut after = format!(
                    "克隆第 {} 行插入（{}）",
                    after_row
                        .map(|r| r.to_string())
                        .unwrap_or_else(|| "末行".into()),
                    if filled > 0 {
                        format!("{filled} 格已填")
                    } else {
                        "空行".to_string()
                    }
                );
                if any_fallback {
                    after.push_str("；部分格段数不对齐，格式回落首段模板");
                }
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
            EditOp::SetCellFormat {
                block,
                row,
                cell,
                paragraph,
                character,
                style,
                ..
            } => {
                let span = spans[block - 1];
                let entry = table_splice_entry(&mut plan, &mut table_plan_idx, block, span, xml);
                let cur = plan[entry].insert.clone();
                // 预检已校验样式存在；此处重解析拿 ID（批内样式表不可变，无 TOCTOU）
                let style_id = style
                    .as_deref()
                    .map(|name| styles.id_of(name).expect("预检已校验样式存在"));
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
            EditOp::SetTableElement {
                block,
                level,
                row,
                cell,
                element,
                xml: frag,
                ..
            } => {
                let span = spans[block - 1];
                let entry = table_splice_entry(&mut plan, &mut table_plan_idx, block, span, xml);
                let cur = plan[entry].insert.clone();
                let (new_xml, changed) =
                    set_table_element_xml(&cur, level, row, cell, &element, frag.as_deref())
                        .ok_or_else(|| {
                            AppError::Internal(format!(
                                "表格属性手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                            ))
                        })?;
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
            EditOp::MergeCells {
                block,
                direction,
                row,
                cell,
                span: mspan,
                end_row,
                end_cell,
                ..
            } => {
                let span = spans[block - 1];
                // 结构操作链（十一波）：同表多条经预检足迹判定后按输入序改写同一
                // splice（足迹不相交 ⇒ 互不越界）
                let entry = table_splice_entry(&mut plan, &mut table_plan_idx, block, span, xml);
                let cur = plan[entry].insert.clone();
                let (new_xml, summary) =
                    merge_cells_xml(&cur, direction, row, cell, mspan, end_row, end_cell)
                        .ok_or_else(|| {
                            AppError::Internal(format!(
                                "合并手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                            ))
                        })?;
                plan[entry].insert = new_xml;
                plan[entry].summaries.push(AppliedOp {
                    op: "merge_cells",
                    block,
                    before: cell_projected_of(&model, block, row, cell),
                    after: summary,
                    style: None,
                    style_unchanged: None,
                    target: None,
                });
            }
            EditOp::SplitCell {
                block,
                direction,
                row,
                cell,
                ..
            } => {
                let span = spans[block - 1];
                let entry = table_splice_entry(&mut plan, &mut table_plan_idx, block, span, xml);
                let cur = plan[entry].insert.clone();
                let (new_xml, summary) =
                    split_cell_xml(&cur, direction, row, cell).ok_or_else(|| {
                        AppError::Internal(format!(
                            "拆分手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                        ))
                    })?;
                plan[entry].insert = new_xml;
                plan[entry].summaries.push(AppliedOp {
                    op: "split_cell",
                    block,
                    before: cell_projected_of(&model, block, row, cell),
                    after: summary,
                    style: None,
                    style_unchanged: None,
                    target: None,
                });
            }
            EditOp::DeleteTableRow { block, row, .. } => {
                let span = spans[block - 1];
                let entry = table_splice_entry(&mut plan, &mut table_plan_idx, block, span, xml);
                let cur = plan[entry].insert.clone();
                let (new_xml, summary) = delete_table_row_xml(&cur, row).ok_or_else(|| {
                    AppError::Internal(format!(
                        "删行手术失败: 块 {block} XML 形态异常（内部 bug，未写盘）"
                    ))
                })?;
                plan[entry].insert = new_xml;
                plan[entry].summaries.push(AppliedOp {
                    op: "delete_table_row",
                    block,
                    before: projected_of(&model, block),
                    after: summary,
                    style: None,
                    style_unchanged: None,
                    target: None,
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
        .filter(|o| {
            matches!(
                o,
                EditOp::InsertParagraphAfter { .. }
                    | EditOp::InsertTableAfter { .. }
                    | EditOp::InsertImageAfter { .. }
                    | EditOp::InsertTocAfter { .. }
            )
        })
        .count();
    let deleted = ops
        .iter()
        .filter(|o| matches!(o, EditOp::DeleteBlock { .. }))
        .count();
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
        | EditOp::InsertImageAfter { expect_prefix, .. }
        | EditOp::InsertTocAfter { expect_prefix, .. }
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
            if target_key.is_empty() {
                String::new()
            } else {
                format!("的 {target_key}")
            }
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

/// 行区间相交判定（含端点）。
fn ranges_overlap(
    a: &std::ops::RangeInclusive<usize>,
    b: &std::ops::RangeInclusive<usize>,
) -> bool {
    a.start() <= b.end() && b.start() <= a.end()
}

/// 结构操作的表内足迹（表内地址模拟器，十一波）：其改写地址布局（格数/行序/
/// vMerge 标记）所及的行区间。soundness 不变式：**操作预检读过的每一行 ⊆ 自身
/// 足迹**——足迹不相交 ⇒ 他人不动我读过的行 ⇒ 原模型上的寻址校验在应用期依然
/// 成立（D14 组合判据机械化：不重叠行结构操作序无关，可同批）。删行足迹覆盖
/// 该行至末行（下方行号整体前移）；纵拆足迹精确到链尾（与 apply 链走同判据）。
fn structural_footprint_of(op: &EditOp, node: &Block) -> (std::ops::RangeInclusive<usize>, String) {
    match op {
        EditOp::MergeCells {
            row,
            cell,
            direction,
            span,
            end_row,
            ..
        } => {
            let (row, cell) = (*row, *cell);
            if let Some(er) = end_row {
                let last = (*er).max(row);
                (row..=last, format!("merge_cells r{row}..r{last} 矩形合并"))
            } else {
                match direction {
                    Some(MergeDirection::Vertical) => {
                        let s = span.unwrap_or(2);
                        let last = row
                            .checked_add(s)
                            .map(|e| e.saturating_sub(1))
                            .unwrap_or(row);
                        (
                            row..=last.max(row),
                            format!("merge_cells r{row} 起纵并 {s} 行"),
                        )
                    }
                    _ => (row..=row, format!("merge_cells r{row}c{cell} 横并")),
                }
            }
        }
        EditOp::SplitCell {
            direction,
            row,
            cell,
            ..
        } => match direction {
            MergeDirection::Vertical => {
                let chain_last = match node {
                    Block::Table(t) => split_v_chain_last(t, *row, *cell),
                    Block::Paragraph(_) => *row, // 非表格：预检稍后拒，足迹退化本行
                };
                (
                    *row..=chain_last,
                    format!("split_cell r{row}c{cell} 纵并链拆分"),
                )
            }
            MergeDirection::Horizontal => {
                (*row..=*row, format!("split_cell r{row}c{cell} 横并拆分"))
            }
        },
        EditOp::DeleteTableRow { row, .. } => {
            let last = match node {
                Block::Table(t) => t.rows.len(),
                Block::Paragraph(_) => *row,
            };
            (*row..=last.max(*row), format!("delete_table_row r{row}"))
        }
        _ => unreachable!("仅结构操作声明足迹"),
    }
}

/// 纵并链尾行号（模型级，与 split_cell_xml 的 XML 级链走同判据：同网格列区间
/// 向下遇 continue 即延伸，非 continue / 边界不对齐即止）。
fn split_v_chain_last(t: &docx_model::Table, row: usize, cell: usize) -> usize {
    let Some(head) = t.rows.get(row - 1) else {
        return row;
    };
    if cell == 0 || cell > head.cells.len() {
        return row; // 寻址残缺：预检稍后拒，足迹退化本行
    }
    let (g0, g1) = grid_range_of(head, cell);
    let mut last = row;
    for r2 in row + 1..=t.rows.len() {
        match cell_at_grid_range(&t.rows[r2 - 1], g0, g1) {
            Some(c) if c.v_merge.as_deref() == Some("continue") => last = r2,
            _ => break,
        }
    }
    last
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
        Block::Table(t) => t
            .rows
            .iter()
            .any(|row| row.cells.iter().any(|c| c.blocks.iter().any(has_revision))),
    }
}

// =========================================================================
// 块 XML 重建（pPr / rPr / 开标签属性全部原样字节切片保留）
// =========================================================================

/// 重建段落块：保留开标签属性 + pPr + 首个 run 的 rPr，正文换为 `new_text`。
/// `new_text = None` 表示删除整块（返回空串）。
fn rebuild_paragraph(xml: &str, span: BlockSpan, new_text: Option<&str>) -> String {
    let block_xml = &xml[span.start..span.end];
    let Some(text) = new_text else {
        return String::new();
    };

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
        return Err(AppError::Validation(format!(
            "空格式操作: {op_label} 未提供任何要修改的字段。\
                 paragraph（对齐/行距/段前后/缩进）与 character（粗斜/字号/颜色/字体）\
                 至少一项内有字段。"
        )));
    }
    const ALIGNS: [&str; 7] = [
        "left",
        "center",
        "right",
        "both",
        "distribute",
        "start",
        "end",
    ];
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
        let name = head
            .trim_end()
            .rsplit(char::is_whitespace)
            .next()
            .unwrap_or("")
            .to_string();
        let after = &rest[eq + 1..];
        let quote = after.chars().next();
        if quote != Some('"') {
            rest = &rest[eq + 1..];
            continue;
        }
        let Some(end_rel) = after[1..].find('"') else {
            break;
        };
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
pub(super) fn upsert_element(
    parent: &str,
    name: &str,
    new_tag: &str,
    later: &[&str],
    end_marker: &str,
) -> String {
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
            attr_set(
                &mut attrs,
                "w:line",
                &format!("{}", (v * 240.0).round() as i64),
            );
            attr_set(&mut attrs, "w:lineRule", "auto");
        }
        if let Some(v) = p.space_before_pt {
            attr_set(
                &mut attrs,
                "w:before",
                &format!("{}", (v * 20.0).round() as i64),
            );
            attr_remove(&mut attrs, "w:beforeLines");
            attr_remove(&mut attrs, "w:beforeAutospacing");
        }
        if let Some(v) = p.space_after_pt {
            attr_set(
                &mut attrs,
                "w:after",
                &format!("{}", (v * 20.0).round() as i64),
            );
            attr_remove(&mut attrs, "w:afterLines");
            attr_remove(&mut attrs, "w:afterAutospacing");
        }
        let tag = build_tag("spacing", &attrs);
        out = upsert_element(
            &out,
            "spacing",
            &tag,
            &["ind", "jc", "rPr", "sectPr", "pPrChange"],
            "</w:pPr>",
        );
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
        out = upsert_element(
            &out,
            "ind",
            &tag,
            &["jc", "rPr", "sectPr", "pPrChange"],
            "</w:pPr>",
        );
    }
    // jc（单属性，整元素替换/插入）
    if let Some(v) = &p.align {
        let tag = format!(r#"<w:jc w:val="{v}"/>"#);
        out = upsert_element(
            &out,
            "jc",
            &tag,
            &["textDirection", "outlineLvl", "rPr", "sectPr", "pPrChange"],
            "</w:pPr>",
        );
    }
    out
}

/// rPr 内容里应用字符格式（b/bCs、i/iCs、sz/szCs 成对；color 单元素；rFonts 合并）。
fn apply_char_formats(rpr_inner: &str, c: &CharFormat) -> String {
    let mut out = rpr_inner.to_string();
    if let Some(v) = c.bold {
        let tag = if v {
            "<w:b/>".to_owned()
        } else {
            r#"<w:b w:val="0"/>"#.to_owned()
        };
        out = upsert_element(&out, "b", &tag, &["i", "color", "sz", "u"], "</w:rPr>");
        let tag_cs = if v {
            "<w:bCs/>".to_owned()
        } else {
            r#"<w:bCs w:val="0"/>"#.to_owned()
        };
        out = upsert_element(&out, "bCs", &tag_cs, &["i", "color", "sz", "u"], "</w:rPr>");
    }
    if let Some(v) = c.italic {
        let tag = if v {
            "<w:i/>".to_owned()
        } else {
            r#"<w:i w:val="0"/>"#.to_owned()
        };
        out = upsert_element(&out, "i", &tag, &["color", "sz", "u"], "</w:rPr>");
        let tag_cs = if v {
            "<w:iCs/>".to_owned()
        } else {
            r#"<w:iCs w:val="0"/>"#.to_owned()
        };
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
        out = upsert_element(
            &out,
            "rFonts",
            &tag,
            &["b", "i", "color", "sz", "u"],
            "</w:rPr>",
        );
    }
    out
}

/// 全新 pPr 内容（无 pPr / 自闭合空 pPr 两分支共用；schema 序 spacing→ind→jc）。
fn fresh_ppr_inner(p: &ParaFormat) -> String {
    let mut inner = String::new();
    let mut attrs: Vec<(String, String)> = Vec::new();
    if let Some(v) = p.line_spacing {
        attr_set(
            &mut attrs,
            "w:line",
            &format!("{}", (v * 240.0).round() as i64),
        );
        attr_set(&mut attrs, "w:lineRule", "auto");
    }
    if let Some(v) = p.space_before_pt {
        attr_set(
            &mut attrs,
            "w:before",
            &format!("{}", (v * 20.0).round() as i64),
        );
    }
    if let Some(v) = p.space_after_pt {
        attr_set(
            &mut attrs,
            "w:after",
            &format!("{}", (v * 20.0).round() as i64),
        );
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
        return Some(format!(
            "{}{new_ppr}{}",
            &block_xml[..gt + 1],
            &block_xml[gt + 1..]
        ));
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
            format!(
                "{}<w:pPr>{frag}</w:pPr>{}",
                &block_xml[..s],
                &block_xml[open_end..]
            ),
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
            format!(
                "{}{}",
                &block_xml[..s],
                &block_xml[close_at + "</w:pPr>".len()..]
            ),
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
        let rel_a = block_xml[from..]
            .find("<w:r>")
            .map(|p| (from + p, "<w:r>".len()));
        let rel_b = block_xml[from..]
            .find("<w:r ")
            .map(|p| (from + p, "<w:r ".len()));
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


// 表格手术 + 容器重打包（自本文件拆出，保持纯函数契约：xml/bytes 进、新 xml/bytes 出）。
mod table;

use table::*;

// 供 doc 兄弟模块复用的共享件回导（保持原 docx_edit:: 路径不变）。
pub(in crate::harness::doc) use table::{direct_children_spans, repack_part};

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests;
