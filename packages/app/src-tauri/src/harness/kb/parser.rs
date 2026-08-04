//! markdown 文档解析 —— 单文件 → 可检索的结构化字段（RAG v1 摄入管道第 1 环）
//!
//! v1 范围：提取 title / summary / tags，供 `kb_document` 表索引。
//! 不做完整 AST 解析（不引入 pulldown-cmark），用字符串处理 + `serde_yaml`
//! 解析 frontmatter 即可满足关键词检索需求。向量/切块留 v2。
//!
//! 字段来源优先级：
//! - title：frontmatter.title → 首个 H1 → 空串（indexer 用文件名兜底）
//! - summary：frontmatter.summary → 正文首段（截断 ~200 字符）
//! - tags：frontmatter.tags（YAML 数组）→ JSON 数组字符串；无则 `[]`

use blake2::{Blake2b512, Digest};
use serde::Deserialize;

/// 解析出的文档字段（对应 `kb_document` 的 title/summary/tags 列）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDoc {
    pub title: String,
    pub summary: String,
    /// JSON 数组字符串，如 `["rust","tauri"]`；无 tags 则 `[]`。
    pub tags: String,
}

/// frontmatter 的已知字段（缺失字段走 `Default`，解析失败整体回退空）。
#[derive(Deserialize, Default)]
struct FrontMatter {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    tags: Option<serde_yaml::Value>,
}

/// summary 截断上限（字符数）。首段过长时截断，避免索引列膨胀。
const SUMMARY_MAX_CHARS: usize = 200;

/// 解析 markdown 内容，提取 title/summary/tags。
///
/// 纯函数，无 IO —— 调用方（indexer）负责读文件后传入内容。
pub fn parse_markdown(content: &str) -> ParsedDoc {
    let (fm, body) = split_frontmatter(content);

    let title = fm
        .title
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| first_h1(&body).map(|s| s.to_string()))
        .unwrap_or_default();

    let summary = fm
        .summary
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| first_paragraph(&body));

    ParsedDoc {
        title,
        summary: truncate(&summary, SUMMARY_MAX_CHARS),
        tags: serialize_tags(&fm.tags),
    }
}

/// 计算内容的 blake2b-512 哈希（hex），用于增量索引的变更检测。
///
/// 稳定确定 —— 跨进程/跨重启一致，可直接存 `kb_document.content_hash` 比对。
pub fn content_hash(content: &[u8]) -> String {
    let digest = Blake2b512::digest(content);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

// =========================================================================
// 内部辅助
// =========================================================================

/// 拆出 YAML frontmatter 与正文。
///
/// frontmatter 约定：文件首行为独占的 `---`，到下一个独占的 `---` 之间为 YAML。
/// 首行不是 `---`、或未闭合（找不到第二个 `---`）时，整体视为正文（无 frontmatter）。
/// 自动跳过可选的 UTF-8 BOM。
fn split_frontmatter(content: &str) -> (FrontMatter, String) {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let lines: Vec<&str> = content.lines().collect();

    if lines.first().map(|l| l.trim()) != Some("---") {
        return (FrontMatter::default(), content.to_string());
    }

    // 从第 2 行起找闭合的 `---`
    let Some(close_rel) = lines.iter().skip(1).position(|l| l.trim() == "---") else {
        return (FrontMatter::default(), content.to_string());
    };
    let close_idx = close_rel + 1;

    let fm_yaml = lines[1..close_idx].join("\n");
    let fm: FrontMatter = serde_yaml::from_str(&fm_yaml).unwrap_or_default();
    let body = lines[close_idx + 1..].join("\n");
    (fm, body)
}

/// 取正文中首个 H1 标题文本（跳过 H2 及更深）。
///
/// 兼容 `# 标题`、`#标题`（无空格，中文常见）；`## ` 被排除（H1 专有）。
fn first_h1(body: &str) -> Option<&str> {
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') && !trimmed.starts_with("##") {
            let title = trimmed.trim_start_matches('#').trim();
            if !title.is_empty() {
                return Some(title);
            }
        }
    }
    None
}

/// 取正文首个连续段落（跳过空行 / 标题 / 分隔符 / 图片 / 代码块起始）。
///
/// 段内多行用空格拼接成单行，便于关键词匹配与摘要展示。
fn first_paragraph(body: &str) -> String {
    let mut para: Vec<&str> = Vec::new();
    let mut in_para = false;
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            if in_para {
                break;
            }
            continue;
        }
        let is_prose = !(t.starts_with('#')
            || t == "---"
            || t.starts_with("![")
            || t.starts_with("```"));
        if !is_prose {
            if in_para {
                break;
            }
            continue;
        }
        in_para = true;
        para.push(t);
    }
    para.join(" ")
}

/// 把 frontmatter 的 tags 字段归一为 JSON 数组字符串。
///
/// 支持 YAML 序列（`[a, b]` 或块状 `- a`）与逗号分隔字符串；其余形式 → `[]`。
fn serialize_tags(value: &Option<serde_yaml::Value>) -> String {
    let tags: Vec<String> = match value {
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        Some(serde_yaml::Value::String(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        _ => Vec::new(),
    };
    serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string())
}

/// 按【字符数】截断，超出加省略号；用于 summary。
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.trim().to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

// =========================================================================
// chunk 切分（RAG v2）
// =========================================================================

/// 目标 chunk 大小（字符数）
const CHUNK_TARGET_SIZE: usize = 500;
/// 单个 chunk 最大字符数（超过则强制切分）
const CHUNK_MAX_SIZE: usize = 800;

/// 把文档正文切分为 chunk 列表。
///
/// 策略：
/// 1. 按双换行分段
/// 2. 累积段落到 ~500 字符时输出一个 chunk
/// 3. 单段超过 800 字符则按行进一步切分
/// 4. 最后一个小段落（<100 字符）合并到前一个 chunk
pub fn split_into_chunks(content: &str) -> Vec<String> {
    let paragraphs: Vec<&str> = content.split("\n\n").collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for para in paragraphs {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }

        // 单段过长 → 按行切分后逐行累积
        if para.chars().count() > CHUNK_MAX_SIZE {
            // 先把当前累积的输出
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            // 按行切分超长段落
            for line in para.lines() {
                if current.chars().count() + line.chars().count() + 1 > CHUNK_TARGET_SIZE && !current.is_empty() {
                    chunks.push(std::mem::take(&mut current));
                }
                if !current.is_empty() {
                    current.push('\n');
                }
                current.push_str(line);
            }
            continue;
        }

        // 正常段落：累积到目标大小
        if !current.is_empty() && current.chars().count() + para.chars().count() > CHUNK_TARGET_SIZE {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);
    }

    // 剩余部分
    if !current.is_empty() {
        chunks.push(current);
    }

    // 合并最后的小碎片
    if chunks.len() >= 2 {
        let last = chunks.last().unwrap();
        if last.chars().count() < 100 {
            let small = chunks.pop().unwrap();
            if let Some(prev) = chunks.last_mut() {
                prev.push_str("\n\n");
                prev.push_str(&small);
            } else {
                chunks.push(small);
            }
        }
    }

    chunks
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_title_summary_tags() {
        let md = "---\ntitle: 我的笔记\nsummary: 一段简介\ntags: [rust, tauri]\n---\n# 正文标题\n正文内容\n";
        let d = parse_markdown(md);
        assert_eq!(d.title, "我的笔记");
        assert_eq!(d.summary, "一段简介");
        assert_eq!(d.tags, r#"["rust","tauri"]"#);
    }

    #[test]
    fn title_falls_back_to_h1_when_no_frontmatter() {
        let md = "# 来自 H1 的标题\n\n第一段正文。\n";
        let d = parse_markdown(md);
        assert_eq!(d.title, "来自 H1 的标题");
        assert_eq!(d.summary, "第一段正文。");
        assert_eq!(d.tags, "[]");
    }

    #[test]
    fn h2_is_not_used_as_title() {
        // 有 H2 但无 H1 → title 为空（交由 indexer 用文件名兜底）
        let md = "## 二级标题\n正文\n";
        let d = parse_markdown(md);
        assert_eq!(d.title, "");
    }

    #[test]
    fn summary_takes_first_paragraph_when_absent() {
        let md = "# 标题\n\n第一段第一句。\n第一段第二句。\n\n第二段不应出现。\n";
        let d = parse_markdown(md);
        assert_eq!(d.summary, "第一段第一句。 第一段第二句。");
    }

    #[test]
    fn summary_truncates_long_paragraph() {
        let long = "字".repeat(500);
        let md = format!("# t\n\n{long}\n");
        let d = parse_markdown(&md);
        assert_eq!(d.summary.chars().count(), SUMMARY_MAX_CHARS + 1); // 200 字 + 省略号
        assert!(d.summary.ends_with('…'));
    }

    #[test]
    fn tags_from_block_sequence() {
        let md = "---\ntags:\n  - a\n  - b\n---\nbody\n";
        let d = parse_markdown(md);
        assert_eq!(d.tags, r#"["a","b"]"#);
    }

    #[test]
    fn tags_from_comma_string() {
        let md = "---\ntags: rust, tauri ,sql\n---\nbody\n";
        let d = parse_markdown(md);
        assert_eq!(d.tags, r#"["rust","tauri","sql"]"#);
    }

    #[test]
    fn no_frontmatter_no_h1() {
        let md = "就是一段正文，没有标题也没有 frontmatter。\n";
        let d = parse_markdown(md);
        assert_eq!(d.title, "");
        assert_eq!(d.summary, "就是一段正文，没有标题也没有 frontmatter。");
        assert_eq!(d.tags, "[]");
    }

    #[test]
    fn unclosed_frontmatter_treated_as_body() {
        // 没有闭合 ---，整体当正文（不解析为 frontmatter）
        let md = "---\ntitle: 不应被采用\n这行让它无法闭合\n";
        let d = parse_markdown(md);
        assert_eq!(d.title, "");
        assert!(d.summary.contains("不应被采用"));
    }

    #[test]
    fn chinese_h1_without_space() {
        let md = "#中文标题\n正文\n";
        let d = parse_markdown(md);
        assert_eq!(d.title, "中文标题");
    }

    #[test]
    fn content_hash_is_stable_and_distinct() {
        let h1 = content_hash(b"hello");
        let h2 = content_hash(b"hello");
        let h3 = content_hash(b"world");
        assert_eq!(h1, h2, "相同内容哈希必须一致");
        assert_ne!(h1, h3, "不同内容哈希必须不同");
        assert!(!h1.is_empty());
        // hex 字符串
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn bom_is_stripped() {
        let md = "\u{feff}---\ntitle: 带 BOM\n---\n正文\n";
        let d = parse_markdown(md);
        assert_eq!(d.title, "带 BOM");
    }
}
