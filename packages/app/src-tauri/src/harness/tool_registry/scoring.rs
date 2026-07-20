//! 工具相关性打分 — 名称精确匹配 + 描述子串/整词匹配 + 调用历史权重
//!
//! M1.2 工具裁剪：发往 LLM 的工具 schema 太多时，会显著拉高 prompt tokens
//! 并稀释 LLM 注意力。本模块提供「打分 → 软裁剪」的两段式能力：
//!
//! 1. [`score_tools`]：基于「query 名称精确匹配 + 描述子串/整词匹配 + 最近调用历史」对每个工具打 0..N 分
//! 2. [`apply_trim_markers`]：对超过 `trim_top_k` 的工具，在其 `description`
//!    末尾追加 ` [deprioritized]` 软标记（不是删除 —— 工具仍然可用，
//!    但语义上让 LLM 知道它是次选）
//!
//! 设计原则：
//! - 打分与裁剪**分离**：方便测试 + 后续 A3-4 摘要阶段复用打分逻辑
//! - 不读 DB：纯函数，输入 `&[ToolDef]` + `query` + 历史，输出 score / 标记
//! - 不抛错：所有边界（空 query / 空历史 / 空 defs）都有合理的回退行为

use std::collections::HashMap;

use crate::infra::protocol::ToolDef;

/// 每个工具的最终得分映射：`tool.name -> score (u32)`
pub type ScoreMap = HashMap<String, u32>;

/// 子串匹配打分权重
///
/// - 名称精确匹配（不区分大小写）：+3
/// - 描述子串匹配（不区分大小写）：+2
/// - 描述完整单词匹配（不区分大小写、整词边界）：+4
const SCORE_NAME_EXACT: u32 = 3;
const SCORE_DESC_SUBSTR: u32 = 2;
const SCORE_DESC_WORD: u32 = 4;

/// 调用历史权重：每出现一次 +5，单个工具累计上限 20
const SCORE_HISTORY_PER_OCCURRENCE: u32 = 5;
const SCORE_HISTORY_CAP: u32 = 20;

/// 软裁剪标记：append 到 description 末尾，让 LLM 知道这是次选
const TRIM_MARKER: &str = " [deprioritized]";

/// 子串匹配打分：
/// - query 中每个 token 与 tool.name / description 不区分大小写匹配
/// - name 精确匹配 +3，description 子串匹配 +2，description 完整单词匹配 +4
/// - 调用历史权重：最近出现过的工具每次 +5，上限 20 分
///
/// # 入参
/// - `query`：当前用户消息的纯文本（PipelineContext.current_user_query）
/// - `defs`：候选工具定义列表（来自 `tool_registry.list_tool_defs()`）
/// - `call_history`：最近调用过的工具名称列表（顺序不限，按出现次数累计）
///
/// # 返回
/// - `ScoreMap`：`tool.name -> score`。**未出现的工具得分为 0**（默认行为）
///
/// # 边界
/// - `query.trim().is_empty()` → 所有工具得 0，仅靠调用历史加权
/// - `defs.is_empty()` → 返回空 map
/// - `call_history` 中不存在的工具名 → 直接跳过（不报错）
pub fn score_tools(
    query: &str,
    defs: &[ToolDef],
    call_history: &[String],
) -> ScoreMap {
    let mut scores: ScoreMap = HashMap::new();

    // 1) 子串匹配打分
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .collect();

    for def in defs {
        let name_lower = def.name.to_lowercase();
        let desc_lower = def.description.to_lowercase();

        let mut s: u32 = 0;

        // name 精确匹配：仅当 query 与 name 完全相等时 +3
        if !tokens.is_empty() {
            // 多 token query 时，要求整体完全匹配才加分
            if query_lower == name_lower {
                s += SCORE_NAME_EXACT;
            }
        }

        // description 子串匹配：任一 token 命中 description 时 +2
        for tok in &tokens {
            if desc_lower.contains(tok) {
                s += SCORE_DESC_SUBSTR;
            }
            // description 完整单词匹配：description 中存在与 token 等长的单词
            // 注意：这里仅检查「作为独立词出现」，而非子串
            if desc_lower
                .split_whitespace()
                .any(|w| w == *tok)
            {
                s += SCORE_DESC_WORD;
            }
        }

        scores.insert(def.name.clone(), s);
    }

    // 2) 调用历史权重：每个工具按出现次数累计，上限 20
    for tool_name in call_history {
        // 只对 defs 中存在的工具加权（防止历史脏数据影响打分）
        if !scores.contains_key(tool_name) {
            continue;
        }
        let entry = scores.entry(tool_name.clone()).or_insert(0);
        let next = entry.saturating_add(SCORE_HISTORY_PER_OCCURRENCE);
        *entry = next.min(SCORE_HISTORY_CAP);
    }

    scores
}

/// 应用软裁剪标记（排序与裁剪分离）
///
/// **打分与裁剪分离的设计要点**：本函数**不读打分结果**，
/// 排序与裁剪的逻辑应该由调用方（如 `list_tool_defs_with_query`）编排。
///
/// 本函数只做一件事：把 `defs[..trim_top_k]` 保留原样，
/// 把 `defs[trim_top_k..]` 的每个 description 末尾追加 ` [deprioritized]`。
///
/// # 入参
/// - `defs`：已**按相关性降序排列**的工具列表
/// - `trim_top_k`：保留原样的工具数量（>=1）；超过此数的标记为次选
///
/// # 行为
/// - `defs.len() <= trim_top_k` → 不动任何工具（全部保留）
/// - `trim_top_k == 0` → 全部标记为次选（防御性逻辑，正常不会触发）
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
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 构造一个 ToolDef 帮手（参数 schema 简单填一个空对象即可）
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
        // query 与 name 完全相等时 +3
        let defs = vec![
            def("read_file", "Read a file from disk"),
            def("list_directory", "List files in a directory"),
        ];
        let scores = score_tools("read_file", &defs, &[]);
        assert_eq!(scores.get("read_file"), Some(&SCORE_NAME_EXACT));
        // list_directory 不应拿到 name 精确分
        assert_eq!(scores.get("list_directory"), Some(&0));
    }

    #[test]
    fn score_tools_substring_match_in_desc() {
        // query 命中 description 子串时 +2，命中单词时再 +4 = +6
        let defs = vec![def("read_file", "Read a file from disk")];
        let scores = score_tools("file", &defs, &[]);
        // "file" 在 description 中既作为子串（+2）也作为单词（+4）出现
        assert_eq!(
            scores.get("read_file"),
            Some(&(SCORE_DESC_SUBSTR + SCORE_DESC_WORD))
        );
    }

    #[test]
    fn score_tools_history_weight() {
        // 每次历史 +5，上限 20
        let defs = vec![def("read_file", "x"), def("list_directory", "y")];

        // 单次出现 → +5
        let scores = score_tools("", &defs, &["read_file".into()]);
        assert_eq!(scores.get("read_file"), Some(&5));
        assert_eq!(scores.get("list_directory"), Some(&0));

        // 5 次出现 → 25，被 cap 到 20
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
        // 大小写不影响匹配
        let defs = vec![def("Read_File", "Reads A FILE from disk")];
        let scores = score_tools("read FILE", &defs, &[]);
        // 分析：
        // - tokens = ["read", "file"]
        // - name 精确匹配不命中（query_lower="read file" != name_lower="read_file"）
        // - "read": description 子串命中（"reads a file from disk".contains("read") → true）→ +2
        //         description 单词匹配（split_whitespace 含 "reads" 不含 "read"）→ 0
        // - "file": description 子串命中 → +2
        //         description 单词匹配（split_whitespace 含 "file"）→ +4
        // 合计：2 + 2 + 4 = 8
        // 关键：query "FILE" 在 description 中以小写形式出现，说明大小写不敏感生效
        let s = scores.get("Read_File").copied().unwrap_or(0);
        assert!(
            s >= 8,
            "期望至少 +8（read 子串+2, file 子串+2, file 单词+4），实际 {s}"
        );
    }

    #[test]
    fn score_tools_empty_query() {
        // 空 query → 子串匹配为 0，仅靠历史
        let defs = vec![def("read_file", "x")];
        let scores = score_tools("", &defs, &[]);
        assert_eq!(scores.get("read_file"), Some(&0));

        // 空 query + 历史 → 历史权重仍然生效
        let scores = score_tools("", &defs, &["read_file".into(), "read_file".into()]);
        assert_eq!(scores.get("read_file"), Some(&10));
    }

    // ---- apply_trim_markers ----

    #[test]
    fn apply_trim_markers_preserves_top_k() {
        // 前 top_k 个不动
        let mut defs = vec![
            def("a", "alpha"),
            def("b", "beta"),
            def("c", "gamma"),
            def("d", "delta"),
        ];
        apply_trim_markers(&mut defs, 2);
        assert_eq!(defs[0].description, "alpha");
        assert_eq!(defs[1].description, "beta");
        // 后两个被标记
        assert!(defs[2].description.ends_with(TRIM_MARKER));
        assert!(defs[3].description.ends_with(TRIM_MARKER));
    }

    #[test]
    fn apply_trim_markers_marks_others() {
        // 全部标记（trim_top_k=0 是防御性边界）
        let mut defs = vec![def("a", "alpha"), def("b", "beta")];
        apply_trim_markers(&mut defs, 0);
        assert!(defs[0].description.ends_with(TRIM_MARKER));
        assert!(defs[1].description.ends_with(TRIM_MARKER));
    }

    #[test]
    fn apply_trim_markers_no_op_when_under_threshold() {
        // len <= top_k → 不动
        let mut defs = vec![def("a", "alpha")];
        apply_trim_markers(&mut defs, 5);
        assert_eq!(defs[0].description, "alpha");
    }

    #[test]
    fn apply_trim_markers_idempotent() {
        // 已标记的不重复追加
        let mut defs = vec![def("a", "alpha [deprioritized]")];
        apply_trim_markers(&mut defs, 0);
        // 描述不应出现两次标记
        assert_eq!(defs[0].description.matches(TRIM_MARKER).count(), 1);
    }

    /// 历史中出现的工具不在 defs 列表里 → 不报错，直接跳过
    #[test]
    fn score_tools_ignores_unknown_history_entries() {
        let defs = vec![def("read_file", "x")];
        let history = vec!["nonexistent_tool".into(), "read_file".into()];
        let scores = score_tools("", &defs, &history);
        // nonexistent_tool 不应被插入到 scores（因为 defs 里没有）
        assert!(!scores.contains_key("nonexistent_tool"));
        assert_eq!(scores.get("read_file"), Some(&5));
    }
}