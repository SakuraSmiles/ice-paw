//! 工具相关性打分 — 名称精确匹配 + 描述子串/整词匹配 + 调用历史权重
//!
//! Phase 1: 从 `tool_registry/scoring.rs` 迁出，位于 `harness/` 顶层。
//!
//! 1. [`score_tools`]：基于「query 名称精确匹配 + 描述子串/整词匹配 + 最近调用历史」
//!    对每个工具打 0..N 分。分数**仅用于排序**（相关工具靠前），不再做任何裁剪/降级——
//!    所有工具始终全量发给 LLM，避免 agent 误判降级工具不可用。
//! 2. [`DEFAULT_TOOL_SORT_THRESHOLD`]：工具数超过此值才触发打分排序，否则保持原序。
//!
//! 分词支持 CJK：连续中文段按相邻 bigram 切分（"联网搜索"→"联网"/"网搜"/"搜索"），
//! 使中文 query 能与中文描述（如远程工具的【server 名】前缀）子串匹配。
//!
//! 设计原则：
//! - 打分与排序分离，纯函数
//! - 不读 DB，不抛错：所有边界都有合理的回退行为

use std::collections::HashMap;

use crate::infra::protocol::ToolDef;

/// 每个工具的最终得分映射：`tool.name -> score (u32)`
pub type ScoreMap = HashMap<String, u32>;

const SCORE_NAME_EXACT: u32 = 3;
const SCORE_DESC_SUBSTR: u32 = 2;
const SCORE_DESC_WORD: u32 = 4;
const SCORE_HISTORY_PER_OCCURRENCE: u32 = 5;
const SCORE_HISTORY_CAP: u32 = 20;

/// 工具数超过此值时，[`crate::harness::mcp::McpRegistry::list_tool_defs_with_query`]
/// 才按 query 相关性打分排序；否则保持原序全量返回。
///
/// 取 15：当前默认全量装配（内置 ~19 + 远程 MCP）下 defs.len() 恒 > 15，排序总会触发；
/// 该阈值主要作为「未来 agent 工具白名单 UI 复活后，小工具集零开销原序返回」的防护栏。
/// 无论排序与否，所有工具都**全量发送**——排序只影响顺序，不影响可见性。
pub const DEFAULT_TOOL_SORT_THRESHOLD: usize = 15;

/// 判断字符是否为 CJK 表意文字（Unified / Ext A / 兼容表意）。
/// 仅用于 tokenize 的 bigram 分段——标点与全角符号另由 [`is_separator`] 当分隔符处理，
/// 避免它们被吸进 cjk_buf 产生垃圾 bigram（如「索，」「，结」）。
fn is_cjk_ideograph(ch: char) -> bool {
    matches!(
        ch,
        '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}'
    )
}

/// 判断字符是否为 token 分隔符：空白、CJK 标点（。、等）、全角符号（，！全角拉丁）。
/// 这些字符切断 token 边界（先 flush 两个 buf 再丢弃），不进入任何 buf。
fn is_separator(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '\u{3000}'..='\u{303F}' | '\u{FF00}'..='\u{FFEF}')
}

/// 把文本切成小写 token 供打分匹配：
/// - 连续 CJK 段 → 相邻 bigram（"联网搜索"→"联网"/"网搜"/"搜索"）；单字 CJK 段退化为该单字。
/// - 连续非 CJK 段 → 按空白分词（与旧 `split_whitespace` 等价，兼容英文/工具名）。
///
/// 用 bigram 而非单字：常见单字（如"的""用"）噪声过大，会让大量不相关描述命中子串匹配；
/// bigram 显著降低误匹配，同时仍能在含中文描述（远程工具的 server 名前缀）上命中。
///
/// pub(crate)：references.rs 复用作 @引用 选轮分词（发送正文 ↔ 被引会话轮文本
/// 的相关性打分），与工具排序同一套 CJK 语义。
pub(crate) fn tokenize(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut tokens = Vec::new();
    let mut ascii_buf = String::new();
    let mut cjk_buf: Vec<char> = Vec::new();

    fn flush_ascii(buf: &str, out: &mut Vec<String>) {
        for w in buf.split_whitespace() {
            if !w.is_empty() {
                out.push(w.to_string());
            }
        }
    }
    fn flush_cjk(chars: &[char], out: &mut Vec<String>) {
        if chars.is_empty() {
            return;
        }
        if chars.len() == 1 {
            out.push(chars[0].to_string());
            return;
        }
        for pair in chars.windows(2) {
            out.push(pair.iter().collect::<String>());
        }
    }

    for ch in lower.chars() {
        if is_cjk_ideograph(ch) {
            if !ascii_buf.is_empty() {
                flush_ascii(&ascii_buf, &mut tokens);
                ascii_buf.clear();
            }
            cjk_buf.push(ch);
        } else if is_separator(ch) {
            // 空白 / CJK 标点 / 全角符号 → 切断 token 边界，flush 两个 buf 后丢弃该字符
            if !ascii_buf.is_empty() {
                flush_ascii(&ascii_buf, &mut tokens);
                ascii_buf.clear();
            }
            if !cjk_buf.is_empty() {
                flush_cjk(&cjk_buf, &mut tokens);
                cjk_buf.clear();
            }
        } else {
            if !cjk_buf.is_empty() {
                flush_cjk(&cjk_buf, &mut tokens);
                cjk_buf.clear();
            }
            ascii_buf.push(ch);
        }
    }
    flush_ascii(&ascii_buf, &mut tokens);
    flush_cjk(&cjk_buf, &mut tokens);
    tokens
}

/// 子串/整词匹配打分：
/// - query 与 tool.name 精确（不区分大小写）匹配 +3
/// - query 的每个 token 在 description 中子串匹配 +2、整词匹配 +4（整词仅对空白分词的英文有效）
/// - 调用历史权重：最近出现过的工具每次 +5，上限 20 分
pub fn score_tools(query: &str, defs: &[ToolDef], call_history: &[String]) -> ScoreMap {
    let mut scores: ScoreMap = HashMap::new();

    let query_lower = query.to_lowercase();
    let tokens = tokenize(query);

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
            // 整词匹配只对空白分词的 token（英文/工具名）有意义；
            // CJK bigram 没有「词边界」概念，跳过整词判定避免漏判。
            if !tok.chars().any(is_cjk_ideograph)
                && desc_lower.split_whitespace().any(|w| w == *tok)
            {
                s += SCORE_DESC_WORD;
            }
        }

        scores.insert(def.name.clone(), s);
    }

    // 调用历史在 base 之上叠加 bonus；bonus 独立设上限，不侵蚀 base 分。
    // （旧实现把 .min(SCORE_HISTORY_CAP) 作用在 base+history 累计总分上，导致高相关
    // （base>20）且最近调用过的工具反被压到 20，低于无 history 的高相关 peer——历史加权逆变。）
    let mut history_bonus: ScoreMap = ScoreMap::new();
    for tool_name in call_history {
        if !scores.contains_key(tool_name) {
            continue;
        }
        let b = history_bonus.entry(tool_name.clone()).or_insert(0);
        *b = b
            .saturating_add(SCORE_HISTORY_PER_OCCURRENCE)
            .min(SCORE_HISTORY_CAP);
    }
    for (name, bonus) in history_bonus {
        if let Some(score) = scores.get_mut(&name) {
            *score = score.saturating_add(bonus);
        }
    }

    scores
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

    #[test]
    fn score_tools_ignores_unknown_history_entries() {
        let defs = vec![def("read_file", "x")];
        let history = vec!["nonexistent_tool".into(), "read_file".into()];
        let scores = score_tools("", &defs, &history);
        assert!(!scores.contains_key("nonexistent_tool"));
        assert_eq!(scores.get("read_file"), Some(&5));
    }

    // ---- CJK bigram 分词 ----

    #[test]
    fn tokenize_cjk_bigram() {
        let toks = tokenize("联网搜索");
        // bigram：联网 / 网搜 / 搜索
        assert!(toks.contains(&"联网".to_string()));
        assert!(toks.contains(&"网搜".to_string()));
        assert!(toks.contains(&"搜索".to_string()));
    }

    #[test]
    fn tokenize_single_cjk_char_falls_back_to_char() {
        // 单字 CJK 段退化为该单字
        let toks = tokenize("搜");
        assert_eq!(toks, vec!["搜".to_string()]);
    }

    #[test]
    fn tokenize_mixed_cjk_and_ascii() {
        let toks = tokenize("read 联网 search");
        assert!(toks.contains(&"read".to_string()));
        assert!(toks.contains(&"search".to_string()));
        assert!(toks.contains(&"联网".to_string()));
    }

    #[test]
    fn score_tools_cjk_query_matches_cjk_desc() {
        // 远程工具带【server 名】中文前缀后，中文 query 应能命中
        let defs = vec![
            def("t3_webSearchPrime", "【GLM 联网搜索】Search the web"),
            def("read_file", "Read a file from disk"),
        ];
        let scores = score_tools("联网搜索", &defs, &[]);
        let web = scores.get("t3_webSearchPrime").copied().unwrap_or(0);
        let file = scores.get("read_file").copied().unwrap_or(0);
        assert!(web > 0, "中文 query 应命中带中文前缀的远程工具，实际 {web}");
        assert_eq!(file, 0, "英文描述工具不应被中文 query 命中");
        assert!(web > file);
    }

    // ---- DEFAULT_TOOL_SORT_THRESHOLD ----

    #[test]
    fn score_tools_mixed_cjk_ascii_query() {
        // 混合 query：英文 token 走整词(+4)+子串(+2)，CJK bigram 仅走子串(+2)
        let defs = vec![def("t1", "read 联网 module")];
        let scores = score_tools("read 联网", &defs, &[]);
        // read: +2+4=6；联网(bigram): +2（整词判定对 CJK 跳过）→ 共 8
        assert_eq!(
            scores.get("t1"),
            Some(&(SCORE_DESC_SUBSTR + SCORE_DESC_WORD + SCORE_DESC_SUBSTR))
        );
    }

    #[test]
    fn score_tools_cjk_token_skips_word_match() {
        // desc 含独立空白分词的 CJK token「联网」，query bigram「联网」只得子串 +2，
        // 不得整词 +4（CJK token 跳过整词判定）——锁住 is_cjk_ideograph 守卫。
        let defs = vec![def("t1", "功能 联网 其他")];
        let scores = score_tools("联网", &defs, &[]);
        assert_eq!(scores.get("t1"), Some(&SCORE_DESC_SUBSTR));
    }

    #[test]
    fn score_tools_history_does_not_erode_base() {
        // 关键回归：base>0 + 调用历史 → 总分 = base + bonus，不被 CAP(20) 侵蚀。
        // 旧实现会把 base+history 累计压到 20，反而低于无 history 的高相关 peer。
        let defs = vec![def("a", "alpha beta gamma delta")];
        let base = (SCORE_DESC_SUBSTR + SCORE_DESC_WORD) * 4; // 4 个英文词各 +6 = 24
        let scores = score_tools("alpha beta gamma delta", &defs, &["a".into()]);
        assert_eq!(
            scores.get("a"),
            Some(&(base + SCORE_HISTORY_PER_OCCURRENCE)),
            "base({base}) + history 不应被 CAP 侵蚀"
        );
    }

    #[test]
    fn default_sort_threshold_sensible() {
        // 阈值应 > 0 且 < 典型工具数（~20 内置 + 远程），否则排序永不触发或总触发
        const {
            assert!(DEFAULT_TOOL_SORT_THRESHOLD > 0);
            assert!(DEFAULT_TOOL_SORT_THRESHOLD < 30);
        }
    }
}
