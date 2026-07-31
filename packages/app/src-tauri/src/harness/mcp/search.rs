//! `search_files` 工具：在目录树里做正则内容搜索（对标 grep）
//!
//! `PathWhitelist` 授权（workspace 内自动放行）。递归遍历跳过常见噪音目录
//! （.git / node_modules / target / dist 等），按正则匹配文件行，返回命中清单。

use std::fs;
use std::path::Path;

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

use super::client::McpClient;
use super::types::AuthorizationLevel;

const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024; // 跳过 >2MB 的文件
const DEFAULT_MAX_RESULTS: usize = 100;
const MAX_LINE_LEN: usize = 240; // 命中行截断长度

pub struct SearchFilesTool;

#[derive(Deserialize)]
struct SearchFilesArgs {
    path: String,
    /// 正则表达式
    pattern: String,
    /// 可选：文件名子串过滤（如 ".rs" 只搜 rust 文件）
    #[serde(default)]
    include: Option<String>,
    #[serde(default = "default_max_results")]
    max_results: usize,
}

fn default_max_results() -> usize {
    DEFAULT_MAX_RESULTS
}

#[derive(Serialize)]
struct SearchMatch {
    file: String,
    line_no: usize,
    line: String,
}

/// 是否为应跳过的目录（隐藏目录 + 常见构建/依赖目录）
fn is_skip_dir(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules" | "target" | "dist" | "build" | "__pycache__" | ".venum" | "venv"
        )
}

fn walk(
    root: &Path,
    re: &Regex,
    include: &Option<String>,
    results: &mut Vec<SearchMatch>,
    max: usize,
) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if results.len() >= max {
            return;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if is_skip_dir(name) {
                    continue;
                }
            }
            walk(&path, re, include, results, max);
        } else if ft.is_file() {
            // include 过滤（文件名子串）
            if let Some(inc) = include {
                let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !fname.contains(inc) {
                    continue;
                }
            }
            // 跳过大文件 + 非 UTF8
            if fs::metadata(&path).map(|m| m.len() > MAX_FILE_SIZE).unwrap_or(true) {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if results.len() >= max {
                    return;
                }
                if re.is_match(line) {
                    results.push(SearchMatch {
                        file: path.to_string_lossy().to_string(),
                        line_no: i + 1,
                        line: truncate_line(line),
                    });
                }
            }
        }
    }
}

fn truncate_line(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() > MAX_LINE_LEN {
        let mut out: String = t.chars().take(MAX_LINE_LEN).collect();
        out.push('…');
        out
    } else {
        t.to_string()
    }
}

#[async_trait]
impl McpClient for SearchFilesTool {
    fn name(&self) -> &str {
        "search_files"
    }

    fn description(&self) -> &str {
        "Recursively search file contents under a directory with a regex. Returns matching \
lines (file + line number). Skips .git/node_modules/target/dist. Use include to filter by filename."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Root directory to search." },
                "pattern": { "type": "string", "description": "Regex pattern to match." },
                "include": { "type": "string", "description": "Optional filename substring filter (e.g. \".rs\")." },
                "max_results": { "type": "integer", "default": 100 }
            },
            "required": ["path", "pattern"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: SearchFilesArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("search_files 参数解析失败: {e}")))?;

        let re = Regex::new(&parsed.pattern)
            .map_err(|e| AppError::Validation(format!("search_files 正则无效: {e}")))?;

        let root = Path::new(&parsed.path);
        let mut results: Vec<SearchMatch> = Vec::new();
        walk(root, &re, &parsed.include, &mut results, parsed.max_results);

        Ok(serde_json::json!({
            "path": parsed.path,
            "pattern": parsed.pattern,
            "matches": results.len(),
            "results": results,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_dir_logic() {
        assert!(is_skip_dir(".git"));
        assert!(is_skip_dir("node_modules"));
        assert!(is_skip_dir("target"));
        assert!(!is_skip_dir("src"));
    }

    #[test]
    fn regex_matches_line() {
        let re = Regex::new("fn \\w+").unwrap();
        assert!(re.is_match("fn main() {}"));
        assert!(!re.is_match("struct Foo"));
    }
}
