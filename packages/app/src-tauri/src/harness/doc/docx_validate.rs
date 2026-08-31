//! `docx_validate` —— validate_docx 断言验收引擎（word-capability-roadmap D15 八波①）。
//!
//! 生产驱动：统筹 agent 委派弱模型批量产表后，被迫每张表 inspect 三连肉眼比对
//! ——「什么是成功」在人脑里，没变成机器可执行的断言。本模块把验收降维成
//! 断言列表一次跑完：全部独立评估**不短路**，逐条报告 pass/fail 明细。
//!
//! 语义边界（与 doom_loop 的分工）：**断言失败是正常输出（passed=false），不是
//! Err**——doom_detect 按 is_error 计错误签名，验收结果是数据不是执行错误；
//! 只有参数结构错（匹配器零/多选、断言数超限）才 Err 快失败。地址越界（块/行/
//! 格）也是**文档状态断言**而非参数错：该条 fail 附实际范围，不挡其余断言。
//!
//! 口径对齐：块号/行列格地址 = projection=table 网格同口径（列数 = 任一行
//! gridSpan 求和最大值）；block_style = outline meta 的样式显示名口径。
//!
//! 纯函数：bytes 进、报告出，无 IO；工具薄壳（读文件/授权）在 mcp::docx_tool。

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

use super::docx_model::{blocks_text, Block, Table, TableCell};
use super::styles::Stylesheet;
use super::{docx, docx_model, styles, xml_dom};

/// 单条断言上限（防滥用——验收是关键点抽查不是全量对账）。
pub const MAX_ASSERTS: usize = 50;

/// 文本比对摘要长度（与 inspect outline 的 60 字口径一致）。
const CLIP: usize = 60;

// =========================================================================
// 断言语法（wire 格式即引擎语言）
// =========================================================================
//
// 与 OperationSpec/EditOp 的两层不同：断言没有默认值归一/族路由，serde 形态
// 就是引擎形态，故直接在引擎层定义（工具层零换形态）。

/// 单条断言。
#[derive(Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssertSpec {
    /// 文档总块数（sdt 摊平口径，同 inspect_docx total_blocks）
    BlockCount { equals: usize },
    /// 块须是表 + 形状断言；rows/cols/style 全可选（只给想验的）。
    /// cols 口径 = 任一行 gridSpan 求和最大值（同 projection=table 头行）；
    /// style 比表样式（显示名或 ID 同口径）
    TableShape {
        block: usize,
        #[serde(default)]
        rows: Option<usize>,
        #[serde(default)]
        cols: Option<usize>,
        #[serde(default)]
        style: Option<String>,
    },
    /// 段落块文本断言（**全文口径**非 60 字摘要）；equals/contains/starts_with
    /// 三选一；表格块 → 该条 fail 指路 cell_text
    BlockText {
        block: usize,
        #[serde(default)]
        equals: Option<String>,
        #[serde(default)]
        contains: Option<String>,
        #[serde(default)]
        starts_with: Option<String>,
    },
    /// 段落有效样式显示名断言（outline meta 口径；无样式 → `(无样式)`）；
    /// 表格块 → 该条 fail 指路 table_shape 的 style 参数
    BlockStyle { block: usize, equals: String },
    /// 格文本断言（**全文口径**，格内多段以 `\n` 连接）；(row, cell) 双 1-based
    /// 与 projection=table 网格同口径；续格 → 该条 fail（内容在合并头）
    CellText {
        block: usize,
        row: usize,
        cell: usize,
        #[serde(default)]
        equals: Option<String>,
        #[serde(default)]
        contains: Option<String>,
        #[serde(default)]
        starts_with: Option<String>,
    },
    /// 格内段落数断言（嵌套表块不计段）
    CellParagraphCount { block: usize, row: usize, cell: usize, equals: usize },
}

/// 单条失败。
#[derive(Serialize, Debug)]
pub struct AssertFailure {
    /// 断言种类（snake_case，同 serde tag）
    pub kind: &'static str,
    /// 人类可读定位：「块12」/「块12 r3c2」
    pub target: String,
    /// 失败明细（期望 vs 实际，60 字摘要口径）
    pub detail: String,
}

/// 验收报告。
#[derive(Serialize, Debug)]
pub struct ValidateReport {
    pub passed: bool,
    pub total: usize,
    pub failed: usize,
    /// 逐条失败明细（全过时空）
    pub failures: Vec<AssertFailure>,
    /// 通过断言的种类摘要（首现序，计数聚合：「table_shape×2」）——token 经济：
    /// 通过的不回明细，失败才展开
    pub passed_kinds: Vec<String>,
}

// =========================================================================
// 结构性预检（文档无关，先于解析快失败）
// =========================================================================

/// 参数结构校验：非空、不超上限、文本匹配器三选一。
pub fn check_asserts(asserts: &[AssertSpec]) -> AppResult<()> {
    if asserts.is_empty() {
        return Err(AppError::Validation(
            "断言无效: 断言列表为空。validate_docx 至少一条断言——先 inspect_docx \
             projection=outline 拿结构，再写验收关键点（形状/首尾格/样式）。"
                .into(),
        ));
    }
    if asserts.len() > MAX_ASSERTS {
        return Err(AppError::Validation(format!(
            "断言数超限: {} 条（上限 {MAX_ASSERTS}）。验收是关键点抽查不是全量对账；\
             确需更多请拆多批跑。",
            asserts.len()
        )));
    }
    for a in asserts {
        let (equals, contains, starts_with, target) = match a {
            AssertSpec::BlockText { block, equals, contains, starts_with } => {
                (equals, contains, starts_with, format!("块{block}"))
            }
            AssertSpec::CellText { block, row, cell, equals, contains, starts_with } => {
                (equals, contains, starts_with, format!("块{block} r{row}c{cell}"))
            }
            _ => continue,
        };
        let given = equals.is_some() as u8 + contains.is_some() as u8 + starts_with.is_some() as u8;
        if given != 1 {
            return Err(AppError::Validation(format!(
                "断言无效: {target} 的 equals / contains / starts_with 须恰选一个\
                 （给了 {given} 个）。全文比对用 equals，局部用 contains，前缀用 starts_with。"
            )));
        }
    }
    Ok(())
}

// =========================================================================
// 评估主入口
// =========================================================================

/// 对 docx 字节流跑断言批（zip → 模型管线同 inspect_document）。
pub fn validate_document(bytes: &[u8], asserts: &[AssertSpec]) -> AppResult<ValidateReport> {
    check_asserts(asserts)?;
    let doc_xml = docx::read_document_xml(bytes)?;
    let dom = xml_dom::parse(&doc_xml)?;
    let model = docx_model::build_document(&dom);
    // styles.xml 可选部件：缺失/损坏 → 空表（样式断言退化为原始 ID，不挡主路径）
    let stylesheet = match docx::read_entry(bytes, "word/styles.xml")? {
        Some(xml) => xml_dom::parse(&xml)
            .map(|dom| styles::parse_styles(&dom))
            .unwrap_or_else(|_| Stylesheet::empty()),
        None => Stylesheet::empty(),
    };
    Ok(validate_body(&model.body, &stylesheet, asserts))
}

/// 对已构建的模型体跑断言批（全部独立评估不短路）。
fn validate_body(body: &[Block], stylesheet: &Stylesheet, asserts: &[AssertSpec]) -> ValidateReport {
    let mut failures: Vec<AssertFailure> = Vec::new();
    let mut passed_order: Vec<&'static str> = Vec::new();
    for a in asserts {
        match eval_assert(a, body, stylesheet) {
            Ok(kind) => passed_order.push(kind),
            Err(f) => failures.push(f),
        }
    }
    // 种类摘要：首现序 + 计数聚合
    let mut passed_kinds: Vec<String> = Vec::new();
    for kind in &passed_order {
        if let Some(entry) = passed_kinds.iter_mut().find(|s| s.starts_with(kind)) {
            // 「kind」或「kind×N」→ 计数 +1 重写
            let n: usize = entry
                .split_once("×")
                .map(|(_, n)| n.parse().unwrap_or(1))
                .unwrap_or(1);
            *entry = format!("{kind}×{}", n + 1);
        } else {
            passed_kinds.push((*kind).to_string());
        }
    }
    ValidateReport {
        passed: failures.is_empty(),
        total: asserts.len(),
        failed: failures.len(),
        failures,
        passed_kinds,
    }
}

/// 评估单条断言：Ok(kind) = 通过；Err(failure) = 该条 fail（不中断批）。
fn eval_assert(
    a: &AssertSpec,
    body: &[Block],
    stylesheet: &Stylesheet,
) -> Result<&'static str, AssertFailure> {
    let total = body.len();
    match a {
        AssertSpec::BlockCount { equals } => {
            if *equals == total {
                Ok("block_count")
            } else {
                Err(AssertFailure {
                    kind: "block_count",
                    target: "全文".into(),
                    detail: format!("期望 {equals} 块，实际 {total} 块"),
                })
            }
        }
        AssertSpec::TableShape { block, rows, cols, style } => {
            let block_ref = block_or_fail(body, *block, "table_shape")?;
            let Block::Table(t) = block_ref else {
                return Err(AssertFailure {
                    kind: "table_shape",
                    target: format!("块{block}"),
                    detail: "是段落不是表格。先 inspect_docx projection=outline 确认块类型".into(),
                });
            };
            if let Some(want) = rows {
                if *want != t.rows.len() {
                    return Err(AssertFailure {
                        kind: "table_shape",
                        target: format!("块{block}"),
                        detail: format!("期望 {want} 行，实际 {} 行", t.rows.len()),
                    });
                }
            }
            if let Some(want) = cols {
                let actual = grid_cols(t);
                if *want != actual {
                    return Err(AssertFailure {
                        kind: "table_shape",
                        target: format!("块{block}"),
                        detail: format!("期望 {want} 列，实际 {actual} 列（口径 = 任一行 gridSpan 求和最大值）"),
                    });
                }
            }
            if let Some(want) = style {
                let actual = match &t.style_id {
                    Some(id) => stylesheet.name_of(id).unwrap_or(id).to_string(),
                    None => "无样式".to_string(),
                };
                // 显示名或原始 ID 都认（与 set_table_element 的 style 参数同口径）
                let raw_id_matches =
                    t.style_id.as_deref().is_some_and(|id| id == want.as_str());
                if !raw_id_matches && actual != *want {
                    return Err(AssertFailure {
                        kind: "table_shape",
                        target: format!("块{block}"),
                        detail: format!("期望表样式「{want}」，实际「{actual}」"),
                    });
                }
            }
            Ok("table_shape")
        }
        AssertSpec::BlockText { block, equals, contains, starts_with } => {
            let block_ref = block_or_fail(body, *block, "block_text")?;
            let Block::Paragraph(_) = block_ref else {
                return Err(AssertFailure {
                    kind: "block_text",
                    target: format!("块{block}"),
                    detail: "是表格块，block_text 只断言段落。表格内容用 cell_text，形状用 table_shape".into(),
                });
            };
            let mut text = String::new();
            blocks_text(std::slice::from_ref(block_ref), &mut text);
            let text = text.trim_end_matches('\n');
            match eval_text_match(equals, contains, starts_with, text) {
                Some(detail) => Err(AssertFailure { kind: "block_text", target: format!("块{block}"), detail }),
                None => Ok("block_text"),
            }
        }
        AssertSpec::BlockStyle { block, equals } => {
            let block_ref = block_or_fail(body, *block, "block_style")?;
            let Block::Paragraph(p) = block_ref else {
                return Err(AssertFailure {
                    kind: "block_style",
                    target: format!("块{block}"),
                    detail: "是表格块，block_style 只断言段落样式。表样式用 table_shape 的 style 参数".into(),
                });
            };
            let actual = match &p.props.style {
                Some(id) => stylesheet.name_of(id).unwrap_or(id).to_string(),
                None => "(无样式)".to_string(),
            };
            if actual != *equals {
                return Err(AssertFailure {
                    kind: "block_style",
                    target: format!("块{block}"),
                    detail: format!("期望样式「{equals}」，实际「{actual}」"),
                });
            }
            Ok("block_style")
        }
        AssertSpec::CellText { block, row, cell, equals, contains, starts_with } => {
            let cell_ref = cell_or_fail(body, *block, *row, *cell, "cell_text")?;
            let mut text = String::new();
            blocks_text(&cell_ref.blocks, &mut text);
            let text = text.trim_end_matches('\n');
            match eval_text_match(equals, contains, starts_with, text) {
                Some(detail) => Err(AssertFailure {
                    kind: "cell_text",
                    target: format!("块{block} r{row}c{cell}"),
                    detail,
                }),
                None => Ok("cell_text"),
            }
        }
        AssertSpec::CellParagraphCount { block, row, cell, equals } => {
            let cell_ref = cell_or_fail(body, *block, *row, *cell, "cell_paragraph_count")?;
            let actual = cell_ref
                .blocks
                .iter()
                .filter(|b| matches!(b, Block::Paragraph(_)))
                .count();
            if *equals == actual {
                Ok("cell_paragraph_count")
            } else {
                Err(AssertFailure {
                    kind: "cell_paragraph_count",
                    target: format!("块{block} r{row}c{cell}"),
                    detail: format!("期望 {equals} 段，实际 {actual} 段（嵌套表块不计段）"),
                })
            }
        }
    }
}

// =========================================================================
// 辅助
// =========================================================================

/// 网格列数（任一行 gridSpan 求和最大值——同 render_table_grid 的口径）。
fn grid_cols(t: &Table) -> usize {
    t.rows
        .iter()
        .map(|r| r.cells.iter().map(|c| c.grid_span.unwrap_or(1) as usize).sum::<usize>())
        .max()
        .unwrap_or(0)
}

/// 取块（1-based）；越界 → 该条 fail 附实际范围（不是工具 Err）。
fn block_or_fail<'a>(
    body: &'a [Block],
    block: usize,
    kind: &'static str,
) -> Result<&'a Block, AssertFailure> {
    if block == 0 || block > body.len() {
        Err(AssertFailure {
            kind,
            target: format!("块{block}"),
            detail: format!("块号超出范围（文档共 {} 块）——先 inspect_docx 确认最新块号", body.len()),
        })
    } else {
        Ok(&body[block - 1])
    }
}

/// 取格（块 1-based + 行/格 1-based，projection=table 网格同口径）；
/// 越界/续格 → 该条 fail（附实际范围/指路合并头）。
fn cell_or_fail<'a>(
    body: &'a [Block],
    block: usize,
    row: usize,
    cell: usize,
    kind: &'static str,
) -> Result<&'a TableCell, AssertFailure> {
    let block_ref = block_or_fail(body, block, kind)?;
    let Block::Table(t) = block_ref else {
        return Err(AssertFailure {
            kind,
            target: format!("块{block}"),
            detail: "是段落不是表格，格级断言只作用于表格块。段落用 block_text".into(),
        });
    };
    let fail = |detail: String| AssertFailure { kind, target: format!("块{block} r{row}c{cell}"), detail };
    let row_ref = t.rows.get(row.wrapping_sub(1)).ok_or_else(|| {
        fail(format!("行号超出范围（表共 {} 行）——地址与 projection=table 的 rN 同口径", t.rows.len()))
    })?;
    let cell_ref = row_ref.cells.get(cell.wrapping_sub(1)).ok_or_else(|| {
        fail(format!("格号超出范围（r{row} 共 {} 格）——地址与 projection=table 的 cN 同口径", row_ref.cells.len()))
    })?;
    if cell_ref.v_merge.as_deref() == Some("continue") {
        return Err(fail(
            "是纵向合并续格，内容在合并头格——断言头格地址（projection=table 里标 (合并头) 的格）".into(),
        ));
    }
    Ok(cell_ref)
}

/// 文本匹配评估：Some(detail) = fail；None = pass。
/// （三选一已在 [`check_asserts`] 快失败，此处按「第一个 Some 的模式」评估。）
fn eval_text_match(
    equals: &Option<String>,
    contains: &Option<String>,
    starts_with: &Option<String>,
    actual: &str,
) -> Option<String> {
    if let Some(want) = equals {
        if actual == want.as_str() {
            None
        } else {
            Some(format!("期望等于「{}」，实际「{}」", clip(want), clip(actual)))
        }
    } else if let Some(want) = contains {
        if actual.contains(want.as_str()) {
            None
        } else {
            Some(format!("期望包含「{}」，实际「{}」", clip(want), clip(actual)))
        }
    } else if let Some(want) = starts_with {
        if actual.starts_with(want.as_str()) {
            None
        } else {
            Some(format!("期望以「{}」开头，实际「{}」", clip(want), clip(actual)))
        }
    } else {
        // check_asserts 已拦，纯防御
        Some("未给出比对模式（equals/contains/starts_with 三选一）".into())
    }
}

/// 摘要到 CLIP 字（字符安全，不切字节）。
fn clip(s: &str) -> String {
    if s.chars().count() <= CLIP {
        s.to_string()
    } else {
        let cut: String = s.chars().take(CLIP).collect();
        format!("{cut}…")
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(body: &str) -> String {
        format!(r#"<w:document xmlns:w="w" xmlns:r="r"><w:body>{body}</w:body></w:document>"#)
    }

    /// 两行三列表（r1: gridSpan2 + 1 格；r2: 三格），前置两段。styles 空表。
    fn doc_with_table() -> Vec<Block> {
        let xml = wrap(
            r#"<w:p><w:pPr><w:pStyle w:val="2"/></w:pPr><w:r><w:t>标题段</w:t></w:r></w:p>\
               <w:p><w:r><w:t>正文</w:t></w:r></w:p>\
               <w:tbl>\
                 <w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr>\
                 <w:tr><w:tc><w:p><w:r><w:t>C</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>D1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>D2</w:t></w:r></w:p></w:tc></w:tr>\
               </w:tbl>"#,
        );
        docx_model::build_document(&xml_dom::parse(&xml).unwrap()).body
    }

    fn report(body: &[Block], asserts: &[AssertSpec]) -> ValidateReport {
        check_asserts(asserts).unwrap();
        validate_body(body, &Stylesheet::empty(), asserts)
    }

    fn bc(equals: usize) -> AssertSpec {
        AssertSpec::BlockCount { equals }
    }

    #[test]
    fn all_pass_reports_kinds_summary() {
        let body = doc_with_table();
        let r = report(
            &body,
            &[
                bc(3),
                AssertSpec::TableShape { block: 3, rows: Some(2), cols: Some(3), style: None },
                AssertSpec::CellText {
                    block: 3,
                    row: 2,
                    cell: 3,
                    equals: Some("D2".into()),
                    contains: None,
                    starts_with: None,
                },
                AssertSpec::CellParagraphCount { block: 3, row: 1, cell: 1, equals: 1 },
                AssertSpec::BlockText {
                    block: 2,
                    equals: Some("正文".into()),
                    contains: None,
                    starts_with: None,
                },
            ],
        );
        assert!(r.passed);
        assert_eq!(r.failed, 0);
        assert_eq!(r.total, 5);
        assert_eq!(
            r.passed_kinds,
            vec![
                "block_count",
                "table_shape",
                "cell_text",
                "cell_paragraph_count",
                "block_text"
            ]
        );
        assert!(r.failures.is_empty());
    }

    #[test]
    fn duplicate_kinds_aggregate_with_count() {
        let body = doc_with_table();
        let r = report(
            &body,
            &[
                AssertSpec::CellText {
                    block: 3,
                    row: 2,
                    cell: 1,
                    equals: Some("C".into()),
                    contains: None,
                    starts_with: None,
                },
                AssertSpec::CellText {
                    block: 3,
                    row: 2,
                    cell: 2,
                    equals: Some("D1".into()),
                    contains: None,
                    starts_with: None,
                },
            ],
        );
        assert!(r.passed);
        assert_eq!(r.passed_kinds, vec!["cell_text×2".to_string()]);
    }

    #[test]
    fn block_count_mismatch_fails_with_actual() {
        let body = doc_with_table();
        let r = report(&body, &[bc(99)]);
        assert!(!r.passed);
        assert_eq!(r.failures[0].detail, "期望 99 块，实际 3 块");
    }

    #[test]
    fn table_shape_on_paragraph_points_to_outline() {
        let body = doc_with_table();
        let r = report(
            &body,
            &[AssertSpec::TableShape { block: 1, rows: Some(2), cols: None, style: None }],
        );
        assert!(!r.passed);
        assert!(r.failures[0].detail.contains("是段落不是表格"));
    }

    #[test]
    fn table_shape_rows_and_cols_mismatch() {
        let body = doc_with_table();
        let r = report(
            &body,
            &[AssertSpec::TableShape { block: 3, rows: Some(3), cols: Some(4), style: None }],
        );
        // rows 先报（评估顺序即报告顺序）
        assert!(r.failures[0].detail.contains("期望 3 行，实际 2 行"));
        let r = report(
            &body,
            &[AssertSpec::TableShape { block: 3, rows: Some(2), cols: Some(4), style: None }],
        );
        // cols 口径 = gridSpan 求和（r1 = 2+1 = 3，非格数 2）
        assert!(r.failures[0].detail.contains("期望 4 列，实际 3 列"));
    }

    #[test]
    fn block_text_on_table_points_to_cell_text() {
        let body = doc_with_table();
        let r = report(
            &body,
            &[AssertSpec::BlockText {
                block: 3,
                equals: Some("A".into()),
                contains: None,
                starts_with: None,
            }],
        );
        assert!(r.failures[0].detail.contains("cell_text"));
    }

    #[test]
    fn block_style_falls_back_to_raw_id_without_styles() {
        let body = doc_with_table();
        // styles.xml 缺失 → 空表 → 显示名退化为原始 ID（与 outline 口径一致）
        let r = report(&body, &[AssertSpec::BlockStyle { block: 1, equals: "2".into() }]);
        assert!(r.passed);
        let r = report(&body, &[AssertSpec::BlockStyle { block: 2, equals: "2".into() }]);
        assert!(r.failures[0].detail.contains("(无样式)"));
    }

    #[test]
    fn block_address_out_of_range_is_failure_not_error() {
        let body = doc_with_table();
        // 越界是文档状态断言：该条 fail，不 Err
        let r = report(
            &body,
            &[
                bc(3),
                AssertSpec::CellText {
                    block: 99,
                    row: 1,
                    cell: 1,
                    equals: Some("x".into()),
                    contains: None,
                    starts_with: None,
                },
            ],
        );
        assert_eq!(r.failed, 1);
        assert!(r.failures[0].detail.contains("超出范围"));
        assert_eq!(r.passed_kinds, vec!["block_count".to_string()]);
    }

    #[test]
    fn cell_address_out_of_range_reports_actual_extent() {
        let body = doc_with_table();
        let r = report(
            &body,
            &[AssertSpec::CellText {
                block: 3,
                row: 9,
                cell: 1,
                equals: Some("x".into()),
                contains: None,
                starts_with: None,
            }],
        );
        assert!(r.failures[0].detail.contains("表共 2 行"));
        let r = report(
            &body,
            &[AssertSpec::CellText {
                block: 3,
                row: 2,
                cell: 9,
                equals: Some("x".into()),
                contains: None,
                starts_with: None,
            }],
        );
        assert!(r.failures[0].detail.contains("r2 共 3 格"));
    }

    #[test]
    fn continuation_cell_fails_with_merge_head_pointer() {
        let xml = wrap(
            r#"<w:tbl>\
                 <w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>头</w:t></w:r></w:p></w:tc></w:tr>\
                 <w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr>\
               </w:tbl>"#,
        );
        let body = docx_model::build_document(&xml_dom::parse(&xml).unwrap()).body;
        let r = report(
            &body,
            &[AssertSpec::CellText {
                block: 1,
                row: 2,
                cell: 1,
                equals: Some("".into()),
                contains: None,
                starts_with: None,
            }],
        );
        assert!(r.failures[0].detail.contains("合并头"));
    }

    #[test]
    fn multi_paragraph_cell_text_joins_with_newline() {
        // 格内两段：blocks_text 以 \n 连接（与 set_cell_text 的 \n=多段对称）
        let xml = wrap(
            r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>甲</w:t></w:r></w:p><w:p><w:r><w:t>乙</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        );
        let body = docx_model::build_document(&xml_dom::parse(&xml).unwrap()).body;
        let r = report(
            &body,
            &[
                AssertSpec::CellText {
                    block: 1,
                    row: 1,
                    cell: 1,
                    equals: Some("甲\n乙".into()),
                    contains: None,
                    starts_with: None,
                },
                AssertSpec::CellParagraphCount { block: 1, row: 1, cell: 1, equals: 2 },
            ],
        );
        assert!(r.passed, "failures: {:?}", r.failures);
    }

    #[test]
    fn matcher_zero_or_multiple_rejected_before_eval() {
        assert!(check_asserts(&[]).is_err());
        let zero = vec![AssertSpec::BlockText {
            block: 1,
            equals: None,
            contains: None,
            starts_with: None,
        }];
        let err = check_asserts(&zero).unwrap_err().to_string();
        assert!(err.contains("断言无效"), "{err}");
        let both = vec![AssertSpec::CellText {
            block: 1,
            row: 1,
            cell: 1,
            equals: Some("a".into()),
            contains: Some("b".into()),
            starts_with: None,
        }];
        assert!(check_asserts(&both).is_err());
    }

    #[test]
    fn assert_cap_enforced() {
        let many: Vec<AssertSpec> = (0..MAX_ASSERTS + 1).map(|_| bc(1)).collect();
        let err = check_asserts(&many).unwrap_err().to_string();
        assert!(err.contains("断言数超限"), "{err}");
        let at_cap: Vec<AssertSpec> = (0..MAX_ASSERTS).map(|_| bc(1)).collect();
        assert!(check_asserts(&at_cap).is_ok());
    }

    #[test]
    fn contains_and_starts_with_evaluate() {
        let body = doc_with_table();
        let r = report(
            &body,
            &[AssertSpec::BlockText {
                block: 1,
                equals: None,
                contains: Some("标题".into()),
                starts_with: None,
            }],
        );
        assert!(r.passed);
        let r = report(
            &body,
            &[AssertSpec::BlockText {
                block: 2,
                equals: None,
                contains: None,
                starts_with: Some("正文余".into()),
            }],
        );
        assert!(!r.passed);
        assert!(r.failures[0].detail.contains("期望以"));
    }
}
