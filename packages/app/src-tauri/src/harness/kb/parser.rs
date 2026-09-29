// parser.rs — KB 文档解析（RAG v2 → v3 检索质量批）
//!
//! v3（2026-09-29 RAG 地基批）：
//! - **结构感知分块**：markdown heading（#/##/###）是语义边界——标题行强制开新
//!   chunk（同一 section 的内容不被切到上一个 section 的尾巴里）；代码块（```)
//!   整块不切（切开 = 语法碎片，检索命中了也读不懂）。
//! - **15% 重叠**：相邻 chunk 间重叠 ~75 字符（CHUNK_TARGET_SIZE 的 15%）——
//!   跨段引用/表格被边界切断时，重叠区保证至少一个 chunk 持有完整语境。
//!   v2 无重叠的缺陷形态：表格头在 chunk A、数据在 chunk B——语义检索命中
//!   B 但 agent 拿到的片段缺表头，无法理解。

use serde::{Deserialize, Serialize};

/// 解析后的文档元数据 + 正文（分块由 [`split_into_chunks`] 独立做）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedDoc {
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
}

// =========================================================================
// markdown 解析
// =========================================================================

pub fn parse_markdown(content: &str) -> ParsedDoc {
    let (fm, body) = split_frontmatter(content);

    let title = fm
        .title
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| first_h1(body).map(|s| s.to_string()))
        .unwrap_or_default();

    let summary = fm
        .summary
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| first_paragraph(body));

    ParsedDoc {
        title,
        summary,
        tags: fm.tags.unwrap_or_default(),
    }
}

#[derive(Default, Deserialize)]
struct Frontmatter {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

fn split_frontmatter(content: &str) -> (Frontmatter, &str) {
    let trimmed = content.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            let fm: Frontmatter = serde_yaml_frontmatter(&rest[..end]);
            let body = &rest[end + 4..];
            return (fm, body);
        }
    }
    (Frontmatter::default(), content)
}

/// 极简 YAML frontmatter 解析（只取 title/summary/tags 三字段——完整 YAML
/// 解析器是杀鸡牛刀，且 frontmatter 格式由本应用 ensure/build_markdown 生成、
/// 形状可控）。tags 两种形态都支持：内联 `[a, b]` + 多行列表（`- a`）
fn serde_yaml_frontmatter(raw: &str) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let mut in_tags_list = false;
    let mut tags: Vec<String> = Vec::new();

    for line in raw.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("title:") {
            // YAML 序列化含冒号/井号的值会被引号包裹（单双两种）——剥掉
            fm.title = Some(v.trim().trim_matches(['"', '\'']).to_string());
            in_tags_list = false;
        } else if let Some(v) = line.strip_prefix("summary:") {
            fm.summary = Some(v.trim().trim_matches(['"', '\'']).to_string());
            in_tags_list = false;
        } else if let Some(v) = line.strip_prefix("tags:") {
            in_tags_list = true;
            let v = v.trim();
            if v.starts_with('[') {
                // 内联 [a, b] 形态
                tags = v
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(|s| s.trim().trim_matches(['"', '\'']).to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                in_tags_list = false; // 内联形态一行完结
            }
            // 空 `tags:` 后跟 `- item` 多行——由下方 in_tags_list 分支收集
        } else if in_tags_list {
            // 多行列表项：`- rust`
            if let Some(v) = line.strip_prefix("- ") {
                let tag = v.trim().trim_matches(['"', '\'']).to_string();
                if !tag.is_empty() {
                    tags.push(tag);
                }
            } else if !line.is_empty() {
                // 非列表项 → tags 块结束
                in_tags_list = false;
            }
        }
    }

    if !tags.is_empty() {
        fm.tags = Some(tags);
    }
    fm
}

fn first_h1(body: &str) -> Option<&str> {
    body.lines().find_map(|l| {
        let t = l.trim_start();
        t.strip_prefix("# ").map(|s| s.trim())
    })
}

pub fn first_paragraph(body: &str) -> String {
    body.split("\n\n")
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect()
}

pub fn content_hash(content: impl AsRef<[u8]>) -> String {
    blake2b_16hex(content.as_ref())
}

fn blake2b_16hex(data: &[u8]) -> String {
    use blake2::{Blake2b512, Digest};
    let mut hasher = Blake2b512::new();
    hasher.update(data);
    let result = hasher.finalize();
    result.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

// =========================================================================
// chunk 切分（RAG v3：结构感知 + 重叠）
// =========================================================================

/// 目标 chunk 大小（字符数）
const CHUNK_TARGET_SIZE: usize = 500;
/// 单个 chunk 最大字符数（超过则强制切分）
const CHUNK_MAX_SIZE: usize = 800;
/// 相邻 chunk 重叠字符数（~15% 目标大小——跨边界语境保底）
const CHUNK_OVERLAP: usize = 75;

/// 把文档正文切分为 chunk 列表。
///
/// v3 策略（结构感知 + 重叠）：
/// 1. markdown heading（#/##/…）强制开新 chunk——同一 section 内容不被切到
///    上一个 section 尾巴（v2 纯段落累积会把「## 配置」的第一段粘到「## 简介」
///    的末段后面，语义检索命中后 agent 拿到的是跨 section 碎片）
/// 2. 代码块（``` 围栏）整块不切——切开 = 语法碎片
/// 3. 累积到 ~500 字符输出；超长段按行切分
/// 4. 输出 chunk 时携带尾部 `CHUNK_OVERLAP` 字符到下一个 chunk 的头部
///    （重叠区保证跨段引用/表格至少一个 chunk 持有完整语境）
pub fn split_into_chunks(content: &str) -> Vec<String> {
    // 先按结构边界分段（heading / 代码块围栏感知）
    let sections = split_by_structure(content);
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for section in sections {
        let section = section.trim();
        if section.is_empty() {
            continue;
        }

        // heading 行 → 强制开新 chunk（当前累积先输出）
        if section.starts_with('#') && !current.is_empty() {
            chunks.push(flush_with_overlap(&mut current));
        }

        // 超长段（非代码块）→ 按行切分累积
        if section.chars().count() > CHUNK_MAX_SIZE && !section.starts_with("```") {
            for line in section.lines() {
                if current.chars().count() + line.chars().count() + 1 > CHUNK_TARGET_SIZE
                    && !current.is_empty()
                {
                    chunks.push(flush_with_overlap(&mut current));
                }
                if !current.is_empty() {
                    current.push('\n');
                }
                current.push_str(line);
            }
            continue;
        }

        // 正常段落：累积到目标大小
        if !current.is_empty()
            && current.chars().count() + section.chars().count() > CHUNK_TARGET_SIZE
        {
            chunks.push(flush_with_overlap(&mut current));
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(section);
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    // 合并最后的小碎片（<100 字符）
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
        .into_iter()
        .filter(|c| !c.trim().is_empty())
        .collect()
}

/// 结构感知分段：按双换行分段，但 heading 行独立成段、代码块整块成段。
fn split_by_structure(content: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut in_code = false;
    let mut current = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // 代码块围栏切换
        if trimmed.starts_with("```") {
            in_code = !in_code;
            current.push('\n');
            current.push_str(line);
            continue;
        }

        // 代码块内：整块不切
        if in_code {
            current.push('\n');
            current.push_str(line);
            continue;
        }

        // heading 行 → 先输出当前累积，heading 独立成段
        if trimmed.starts_with('#') && !current.trim().is_empty() {
            sections.push(std::mem::take(&mut current));
        }

        // 空行 → 段落边界（代码块外的双换行）
        if trimmed.is_empty() && !current.trim().is_empty() {
            sections.push(std::mem::take(&mut current));
            continue;
        }

        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.trim().is_empty() {
        sections.push(current);
    }

    sections
}

/// 输出当前累积并给下一个 chunk 头部留重叠尾巴。
/// 从 `current` 尾部取 `CHUNK_OVERLAP` 字符作为 `next_overlap`，
/// 下一个 chunk 的开头会拼上它——跨边界语境保底。
fn flush_with_overlap(current: &mut String) -> String {
    let chunk = std::mem::take(current);
    // 尾部重叠：下个 chunk 头部会拼上这个尾巴
    let chars: Vec<char> = chunk.chars().collect();
    if chars.len() > CHUNK_OVERLAP {
        let tail: String = chars[chars.len() - CHUNK_OVERLAP..].iter().collect();
        *current = tail;
    }
    chunk
}
