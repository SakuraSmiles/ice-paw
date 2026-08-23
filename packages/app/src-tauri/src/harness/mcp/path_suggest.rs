//! not-found 报错的近似路径建议（did-you-mean）
//!
//! 诊断结论（2026-08-22，826 次失败工具调用样本）：「猜测/过时路径」是高频失败族——
//! 模型凭记忆拼路径（如 `protocol.rs` 实际已拆成 `protocol/` 目录），失败后只拿到
//! 一句裸「文件不存在」，下一轮只能再猜一次。本模块在报错时扫真实文件系统给出
//! 近似候选，把「猜路径循环」变成「一次纠偏」——报错即行为契约（Codex A8 借鉴：
//! 错误信息 = 发生了什么 + 为什么 + 怎么办，禁止裸 Err）。

use std::path::Path;

/// 候选数量上限（报错信息保持简短，前 3 个足够定位）
const MAX_SUGGESTIONS: usize = 3;

/// 两个名字（不含扩展名、小写）的相似度：越大越像。0 = 不相关。
///
/// - 3：完全相等（`protocol.rs` ↔ `protocol/`，典型如「文件拆成了同名目录」）
/// - 2：一方包含另一方（≥3 字符，`config.rs` ↔ `config_v2.rs`）
/// - 1：公共前缀 ≥3 字符（`summary.rs` ↔ `summarize.rs`）
fn similarity(missing_stem: &str, entry_stem: &str) -> u32 {
    if missing_stem == entry_stem {
        return 3;
    }
    if missing_stem.len() >= 3
        && entry_stem.len() >= 3
        && (missing_stem.contains(entry_stem) || entry_stem.contains(missing_stem))
    {
        return 2;
    }
    let common = missing_stem
        .chars()
        .zip(entry_stem.chars())
        .take_while(|(a, b)| a == b)
        .count();
    if common >= 3 {
        1
    } else {
        0
    }
}

/// 不存在路径的报错建议尾句（永远非空——没找到候选时也给通用恢复动作）。
///
/// 父目录存在时扫其条目取 top-3 近似候选（目录带 `/` 后缀标示）；父目录不存在或
/// 无相近候选时给「先看目录结构」的通用指引。
pub(crate) fn suggest_for_missing(path: &Path) -> String {
    let fallback = "请核对实际路径；不确定目录结构时，先对父目录调用 list_directory 或 directory_tree 查看。";
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return fallback.to_string();
    };
    // 裸文件名（如 "Cargo.tom"）的 parent() 是空串而非 "."——归一到 "."，
    // 否则 read_dir("") 直接失败，永远落进无候选兜底
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    if stem.is_empty() || stem.starts_with('.') {
        return fallback.to_string();
    }

    let mut candidates: Vec<(u32, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let ename = entry.file_name().to_string_lossy().to_string();
            if ename == name || ename.starts_with('.') {
                continue; // 同名（大小写差异等）与隐藏条目不值得报
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let estem = ename.split('.').next().unwrap_or(&ename).to_ascii_lowercase();
            let score = similarity(&stem, &estem);
            if score > 0 {
                let display = if is_dir { format!("{ename}/") } else { ename };
                candidates.push((score, display));
            }
        }
    }
    if candidates.is_empty() {
        return fallback.to_string();
    }
    // 分数降序、同分按名字——输出确定（不随 read_dir 顺序抖动）
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let top: Vec<String> = candidates
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, d)| d)
        .collect();
    format!(
        "近似候选（{} 内）: {}。请改用实际存在的路径。",
        parent.display(),
        top.join("、")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 临时目录 + 指定条目（名字唯一前缀保证测试间互不污染）
    fn tempdir_with(prefix: &str, entries: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("icepaw_ps_{}_{}", prefix, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for e in entries {
            let p = dir.join(e);
            if e.ends_with('/') {
                std::fs::create_dir_all(&p).unwrap();
            } else {
                std::fs::write(&p, "x").unwrap();
            }
        }
        dir
    }

    #[test]
    fn similarity_ladder() {
        assert_eq!(similarity("protocol", "protocol"), 3);
        assert_eq!(similarity("config", "config_v2"), 2);
        assert_eq!(similarity("summary", "summarize"), 1);
        assert_eq!(similarity("abc", "xyz"), 0);
    }

    #[test]
    fn suggests_same_stem_directory() {
        // 生产案例：protocol.rs 已拆成 protocol/ 目录——应报出同名目录候选
        let dir = tempdir_with("stem", &["protocol/", "context.rs"]);
        let hint = suggest_for_missing(&dir.join("protocol.rs"));
        assert!(hint.contains("protocol/"), "应含同名目录候选: {hint}");
        assert!(hint.contains("近似候选"), "应标注近似候选: {hint}");
    }

    #[test]
    fn no_candidates_falls_back_to_guidance() {
        let dir = tempdir_with("empty", &[]);
        let hint = suggest_for_missing(&dir.join("zzz_unrelated.md"));
        assert!(hint.contains("list_directory"), "无候选时给通用指引: {hint}");
    }

    #[test]
    fn missing_parent_falls_back_to_guidance() {
        // 父目录不存在 → read_dir 失败 → 通用指引（不 panic、不空串）
        let hint = suggest_for_missing(Path::new("/nonexistent_parent_qq7x/file.rs"));
        assert!(hint.contains("list_directory"));
    }

    #[test]
    fn ranks_exact_over_prefix() {
        let dir = tempdir_with(
            "rank",
            &["summarize.rs", "summary/", "summary_old.rs"],
        );
        let hint = suggest_for_missing(&dir.join("summary.rs"));
        // 同名目录（3 分）应排在前缀相似（1 分）之前
        let summary_pos = hint.find("summary/").expect("应含 summary/ 候选");
        let summarize_pos = hint.find("summarize.rs").expect("应含 summarize.rs 候选");
        assert!(summary_pos < summarize_pos, "排序应分数优先: {hint}");
    }
}
