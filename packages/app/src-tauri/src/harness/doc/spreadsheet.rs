//! `.xlsx` / `.xls` / `.xlsb` / `.ods` → markdown 表格文本提取（calamine）。
//!
//! calamine 是 Rust 读表格的事实标准（Polars 同款），`open_workbook_auto_from_rs`
//! 自动嗅探格式，老 `.xls` 也免费覆盖。
//!
//! 渲染策略：**每个 sheet 渲染成一张 GFM markdown 表**——首行作表头、紧跟合规分隔行
//! （`| --- | --- |`，每列至少一个 `-`，无需依赖前端 `healTableSeparators`），
//! 其余为数据行。比 CSV 更利于 LLM 理解列结构；单元格内的 `|` / 换行做转义以免破坏表格。
//!
//! 安全：单 sheet 数据行渲染上限 [`MAX_RENDER_ROWS`]，超出截断并标注（防止万行表
//! 生成数百 KB 文本撑爆 LLM 上下文 / 落库）。

use std::io::Cursor;

use calamine::{Data, Reader, Sheets};

use crate::error::{AppError, AppResult};

use super::{first_nonempty_line, DocKind, ExtractedDoc, TextChunk};

/// 单个 sheet 渲染的数据行上限（不含表头）。
const MAX_RENDER_ROWS: usize = 5000;

/// 从表格字节流提取文本（整篇，KB / read_file 路径用）。
///
/// 内部委托 [`render_sheets`]，把各 sheet 的 markdown 表用 `\n\n` 拼成一篇。
pub(super) fn extract(bytes: &[u8]) -> AppResult<ExtractedDoc> {
    let (sheet_names, sheets) = render_sheets(bytes)?;
    let mut out = String::new();
    let mut first_sheet: Option<String> = None;
    for (idx, (name, body)) in sheets.iter().enumerate() {
        if first_sheet.is_none() {
            first_sheet = Some(name.clone());
        }
        if idx > 0 {
            out.push_str("\n\n");
        }
        out.push_str("## ");
        out.push_str(name);
        out.push('\n');
        out.push_str(body);
    }

    let title = first_sheet.or_else(|| first_nonempty_line(&out));
    Ok(ExtractedDoc {
        text: out,
        kind: DocKind::Spreadsheet {
            sheets: sheet_names,
        },
        title,
    })
}

/// 按 sheet 分块提取（聊天附件分页路径用）：每个 sheet 一块。
///
/// 块文本 = `## {name}\n{body}`（与整篇路径的单 sheet 片段一致），label `Sheet:{name}`。
pub(super) fn extract_chunks(bytes: &[u8]) -> AppResult<(DocKind, Vec<TextChunk>)> {
    let (sheet_names, sheets) = render_sheets(bytes)?;
    let chunks = sheets
        .into_iter()
        .map(|(name, body)| TextChunk {
            text: format!("## {}\n{}", name, body),
            label: format!("Sheet:{}", name),
        })
        .collect();
    Ok((DocKind::Spreadsheet { sheets: sheet_names }, chunks))
}

/// 渲染每个 sheet 为 `(name, body)`，body 为该 sheet 的 GFM markdown 表（不含 `## name` 头）。
///
/// [`extract`] 与 [`extract_chunks`] 的共享脊柱——只解析一次工作簿，两条路径各自拼装。
/// sheet 读取失败记 warn 跳过（不中断）。
fn render_sheets(bytes: &[u8]) -> AppResult<(Vec<String>, Vec<(String, String)>)> {
    let cursor = Cursor::new(bytes.to_vec());
    let mut workbook: Sheets<_> = calamine::open_workbook_auto_from_rs(cursor)
        .map_err(|e| AppError::Internal(format!("表格解析失败: {e}")))?;
    let sheet_names = workbook.sheet_names();
    let mut sheets = Vec::with_capacity(sheet_names.len());
    for name in &sheet_names {
        let range = match workbook.worksheet_range(name) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    target: "ice_paw.doc",
                    "sheet 读取失败，跳过 sheet={} err={}", name, e
                );
                continue;
            }
        };
        let mut body = String::new();
        render_range(&range, &mut body);
        sheets.push((name.clone(), body));
    }
    Ok((sheet_names, sheets))
}

/// 把一个 calamine `Range` 渲染成 GFM 表追加到 `out`。
fn render_range(range: &calamine::Range<Data>, out: &mut String) {
    let width = range.width();
    let mut rows = range.rows();

    // 表头（第一行）
    let header = match rows.next() {
        Some(h) => h,
        None => {
            out.push_str("_(空表)_\n");
            return;
        }
    };
    out.push('|');
    for c in header.iter() {
        out.push(' ');
        out.push_str(&cell_to_str(c));
        out.push_str(" |");
    }
    // 首行短于宽度（尾部空列被裁）→ 补空表头，保持列对齐
    for _ in header.len()..width {
        out.push_str("  |");
    }
    out.push('\n');

    // 合规分隔行：每列一个 `---`（GFM 要求每列至少一个 `-`）
    out.push('|');
    for _ in 0..width {
        out.push_str(" --- |");
    }
    out.push('\n');

    // 数据行
    let mut count = 0usize;
    let mut truncated = false;
    for row in rows {
        if count >= MAX_RENDER_ROWS {
            truncated = true;
            break;
        }
        out.push('|');
        for c in row.iter() {
            out.push(' ');
            out.push_str(&cell_to_str(c));
            out.push_str(" |");
        }
        for _ in row.len()..width {
            out.push_str("  |");
        }
        out.push('\n');
        count += 1;
    }
    if truncated {
        out.push_str(&format!(
            "_(已截断：单 sheet 最多渲染 {} 行数据)_\n",
            MAX_RENDER_ROWS
        ));
    }
}

/// 把一个单元格值渲染为字符串。
///
/// - 整数 float（如 3.0）→ 去掉 `.0` 显示为 `3`，避免把 Excel 数值污染成 `3.0`。
/// - 字符串里的 `|` 转义为 `\|`、换行→空格，避免破坏 markdown 表格行。
fn cell_to_str(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.replace('|', "\\|").replace(['\n', '\r'], " "),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => format_float(*f),
        Data::Bool(b) => b.to_string(),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::DateTime(dt) => dt.to_string(),
        Data::Error(e) => format!("#{:?}", e),
    }
}

/// 浮点数渲染：整数值去 `.0`，特殊值给字面量。
fn format_float(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    // 整数值且不溢出 i64 → 显示为整数
    if f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 {
        format!("{}", f as i64)
    } else {
        format!("{}", f)
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_float_strips_integral_zero() {
        assert_eq!(format_float(3.0), "3");
        assert_eq!(format_float(-5.0), "-5");
        assert_eq!(format_float(3.14), "3.14");
        assert_eq!(format_float(0.0), "0");
        assert_eq!(format_float(f64::NAN), "NaN");
        assert_eq!(format_float(f64::INFINITY), "inf");
        assert_eq!(format_float(f64::NEG_INFINITY), "-inf");
    }

    #[test]
    fn cell_to_str_escapes_pipe_and_newline() {
        assert_eq!(cell_to_str(&Data::String("a|b".into())), "a\\|b");
        assert_eq!(cell_to_str(&Data::String("a\nb".into())), "a b");
        assert_eq!(cell_to_str(&Data::String("a\r\nb".into())), "a b");
        assert_eq!(cell_to_str(&Data::Int(42)), "42");
        assert_eq!(cell_to_str(&Data::Float(7.0)), "7");
        assert_eq!(cell_to_str(&Data::Float(7.5)), "7.5");
        assert_eq!(cell_to_str(&Data::Bool(true)), "true");
        assert_eq!(cell_to_str(&Data::Empty), "");
    }

    #[test]
    fn render_range_empty() {
        let range: calamine::Range<Data> = calamine::Range::empty();
        let mut out = String::new();
        render_range(&range, &mut out);
        assert_eq!(out, "_(空表)_\n");
    }

    #[test]
    fn render_range_simple_table() {
        // 构造 2 列 × 3 行（含表头）的 Range
        let range = build_range(vec![
            vec![Data::String("姓名".into()), Data::String("年龄".into())],
            vec![Data::String("张三".into()), Data::Int(30)],
            vec![Data::String("李四".into()), Data::Int(25)],
        ]);
        let mut out = String::new();
        render_range(&range, &mut out);
        assert_eq!(
            out,
            "| 姓名 | 年龄 |\n| --- | --- |\n| 张三 | 30 |\n| 李四 | 25 |\n"
        );
    }

    /// 用 from_sparse 构造一个紧凑 Range（rows × cols，行优先）。
    fn build_range(rows: Vec<Vec<Data>>) -> calamine::Range<Data> {
        let height = rows.len() as u32;
        let width = rows.iter().map(|r| r.len()).max().unwrap_or(0) as u32;
        let mut range = calamine::Range::<Data>::new((0, 0), (
            height.saturating_sub(1),
            width.saturating_sub(1),
        ));
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                range.set_value((r as u32, c as u32), cell.clone());
            }
        }
        range
    }
}
