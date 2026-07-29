//! 工具相关性打分 — 名称精确匹配 + 描述子串/整词匹配 + 调用历史权重
//!
//! Phase 1: 从 `tool_registry/scoring.rs` 迁出，位于 `harness/` 顶层。
//!
//! 1. [`score_tools`]：基于「query 名称精确匹配 + 描述子串/整词匹配 + 最近调用历史」对每个工具打 0..N 分
//! 2. [`apply_trim_markers`]：对超过 `trim_top_k` 的工具，在其 `description`
//!    末尾追加 ` [deprioritized]` 软标记
//!
//! 设计原则：
//! - 打分与裁剪分离
//! - 不读 DB：纯函数
//! - 不抛错：所有边界都有合理的回退行为

use std::collections::HashMap;

use crate::infra::protocol::ToolDef;

/// 每个工具的最终得分映射：`tool.name -> score (u32)`
pub type ScoreMap = HashMap<String, u32>;

const SCORE_NAME_EXACT: u32 = 3;
const SCORE_DESC_SUBSTR: u32 = 2;
const SCORE_DESC_WORD: u32 = 4;
const SCORE_HISTORY_PER_OCCURRENCE: u32 = 5;
const SCORE_HISTORY_CAP: u32 = 20;
const TRIM_MARKER: &str = " [deprioritized]";

/// 子串匹配打分：
/// - query 中每个 token 与 tool.name / description 不区分大小写匹配
/// - name 精确匹配 +3，description 子串匹配 +2，description 完整单词匹配 +4
/// - 调用历史权重：最近出现过的工具每次 +5，上限 20 分
pub fn score_tools(
    query: &str,
    defs: &[ToolDef],
    call_history: &[String],
) -> ScoreMap {
    let mut scores: ScoreMap = HashMap::new();

    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .collect();

    for def in defs {
        let name_lower = def.name.to_lowercase();
        let desc_lower = def.description.to_lowercase();

        let mut s: u32 = 0;

        if !tokens.is_empty() && query_lower == name_lower {
            s += SCORE_NAME_EXACT;
        }

        for tok in &tokens {
            if desc_lower.contains(tok) {
                s += SCORE_DESC_SUBSTR;
            }
            if desc_lower
                .split_whitespace()
                .any(|w| w == *tok)
            {
                s += SCORE_DESC_WORD;
            }
        }

        scores.insert(def.name.clone(), s);
    }

    for tool_name in call_history {
        if !scores.contains_key(tool_name) {
            continue;
        }
        let entry = scores.entry(tool_name.clone()).or_insert(0);
        let next = entry.saturating_add(SCORE_HISTORY_PER_OCCURRENCE);
        *entry = next.min(SCORE_HISTORY_CAP);
    }

    scores
}

/// 应用软裁剪标记
///
/// 把 `defs[..trim_top_k]` 保留原样，
/// 把 `defs[trim_top_k..]` 的每个 description 末尾追加 ` [deprioritized]`。
pub fn apply_trim_markers(defs: &mut [ToolDef], trim_top_k: usize) {
    if defs.len() <= trim_top_k {
        return;
    }
    for d in defs.iter_mut().skip(trim_top_k) {
        if !d.description.ends_with(TRIM_MARKER) {
            d.description.push_str(TRIM_MARKER);
        }
    }
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(name: &str, description: &str) -> ToolDef {
        ToolDef {
            name: name.into(),
            description: description.into(),
            parameters: json!({}),
        }
    }

    // ---- score_tools ----

    #[test]
    fn score_tools_substring_match_in_name() {
        let defs = vec![
            def("read_file", "Read a file from disk"),
            def("list_directory", "List files in a directory"),
        ];
        let scores = score_tools("read_file", &defs, &[]);
        assert_eq!(scores.get("read_file"), Some(&SCORE_NAME_EXACT));
        assert_eq!(scores.get("list_directory"), Some(&0));
    }

    #[test]
    fn score_tools_substring_match_in_desc() {
        let defs = vec![def("read_file", "Read a file from disk")];
        let scores = score_tools("file", &defs, &[]);
        assert_eq!(
            scores.get("read_file"),
            Some(&(SCORE_DESC_SUBSTR + SCORE_DESC_WORD))
        );
    }

    #[test]
    fn score_tools_history_weight() {
        let defs = vec![def("read_file", "x"), def("list_directory", "y")];

        let scores = score_tools("", &defs, &["read_file".into()]);
        assert_eq!(scores.get("read_file"), Some(&5));
        assert_eq!(scores.get("list_directory"), Some(&0));

        let history: Vec<String> = (0..5).map(|_| "read_file".into()).collect();
        let scores = score_tools("", &defs, &history);
        assert_eq!(scores.get("read_file"), Some(&SCORE_HISTORY_CAP));
    }

    #[test]
    fn score_tools_no_match_returns_zero() {
        let defs = vec![def("read_file", "Read a file")];
        let scores = score_tools("nothing-related-here", &defs, &[]);
        assert_eq!(scores.get("read_file"), Some(&0));
    }

    #[test]
    fn score_tools_case_insensitive() {
        let defs = vec![def("Read_File", "Reads A FILE from disk")];
        let scores = score_tools("read FILE", &defs, &[]);
        let s = scores.get("Read_File").copied().unwrap_or(0);
        assert!(
            s >= 8,
            "期望至少 +8（read 子串+2, file 子串+2, file 单词+4），实际 {s}"
        );
    }

    #[test]
    fn score_tools_empty_query() {
        let defs = vec![def("read_file", "x")];
        let scores = score_tools("", &defs, &[]);
        assert_eq!(scores.get("read_file"), Some(&0));

        let scores = score_tools("", &defs, &["read_file".into(), "read_file".into()]);
        assert_eq!(scores.get("read_file"), Some(&10));
    }

    // ---- apply_trim_markers ----

    #[test]
    fn apply_trim_markers_preserves_top_k() {
        let mut defs = vec![
            def("a", "alpha"),
            def("b", "beta"),
            def("c", "gamma"),
            def("d", "delta"),
        ];
        apply_trim_markers(&mut defs, 2);
        assert_eq!(defs[0].description, "alpha");
        assert_eq!(defs[1].description, "beta");
        assert!(defs[2].description.ends_with(TRIM_MARKER));
        assert!(defs[3].description.ends_with(TRIM_MARKER));
    }

    #[test]
    fn apply_trim_markers_marks_others() {
        let mut defs = vec![def("a", "alpha"), def("b", "beta")];
        apply_trim_markers(&mut defs, 0);
        assert!(defs[0].description.ends_with(TRIM_MARKER));
        assert!(defs[1].description.ends_with(TRIM_MARKER));
    }

    #[test]
    fn apply_trim_markers_no_op_when_under_threshold() {
        let mut defs = vec![def("a", "alpha")];
        apply_trim_markers(&mut defs, 5);
        assert_eq!(defs[0].description, "alpha");
    }

    #[test]
    fn apply_trim_markers_idempotent() {
        let mut defs = vec![def("a", "alpha [deprioritized]")];
        apply_trim_markers(&mut defs, 0);
        assert_eq!(defs[0].description.matches(TRIM_MARKER).count(), 1);
    }

    #[test]
    fn score_tools_ignores_unknown_history_entries() {
        let defs = vec![def("read_file", "x")];
        let history = vec!["nonexistent_tool".into(), "read_file".into()];
        let scores = score_tools("", &defs, &history);
        assert!(!scores.contains_key("nonexistent_tool"));
        assert_eq!(scores.get("read_file"), Some(&5));
    }
}
