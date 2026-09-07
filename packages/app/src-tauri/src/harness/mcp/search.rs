//! `search_files` 工具：在目录树里做正则内容搜索（对标 grep）
//!
//! `PathWhitelist` 授权（workspace 内自动放行）。递归遍历跳过常见噪音目录
//! （.git / node_modules / target / dist 等），按正则匹配文件行，返回命中清单。
//!
//! 错误契约：根目录不存在/是文件 → Err（挂 path_suggest 近似候选），**绝不
//! 静默返回空结果**——「0 命中」必须如实代表「搜过了、没搜到」，模型才能
//! 信任空结果并停止瞎试（2026-08-23 质量拍纪律：静默失败是最恶劣的失败形态）。

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
            if fs::metadata(&path)
                .map(|m| m.len() > MAX_FILE_SIZE)
                .unwrap_or(true)
            {
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
        "Recursively search file contents under a directory with a regex, like grep. \
Returns matching lines (file + line number). Skips .git/node_modules/target/dist and \
hidden dirs; files >2MB or non-UTF-8 are silently skipped. Empty results mean the \
pattern genuinely matched nothing — a wrong path is reported as an error, never as \
zero matches. Use include to filter by filename substring (e.g. \".rs\")."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Root directory to search (must exist and be a directory)." },
                "pattern": { "type": "string", "description": "Rust regex syntax, matched per line. Escape metacharacters to match literally (\"\\.rs\" for the literal \".rs\")." },
                "include": { "type": "string", "description": "Optional filename substring filter (e.g. \".rs\" only searches Rust files)." },
                "max_results": { "type": "integer", "description": "Maximum number of matching lines to return (default 100). Result may cap out before scanning everything — refine pattern/include or raise this if matches are cut short.", "default": 100 }
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

        let re = Regex::new(&parsed.pattern).map_err(|e| {
            // 三段式补「怎么办」：正则语法报错自带位置（为什么），再给正确示例
            AppError::Validation(format!(
                "search_files 正则无效: {e}。请用 Rust regex 语法并转义元字符：\
                 匹配字面句点写 \\.，匹配函数定义写 \"fn \\w+\"；不确定语法时可先用\
                 简单子串（不含元字符）试一次确认路径有命中"
            ))
        })?;

        let root = Path::new(&parsed.path);
        // 错误契约（见模块头）：根目录问题必须 Err，绝不静默返回空结果——
        // 否则模型把「路径错」读成「没搜到」，下一轮换个路径继续瞎试
        if !root.exists() {
            return Err(AppError::Validation(format!(
                "目录不存在: {}。{}",
                parsed.path,
                super::path_suggest::suggest_for_missing(root)
            )));
        }
        if !root.is_dir() {
            return Err(AppError::Validation(format!(
                "搜索根路径是文件不是目录: {}。如需读取单个文件请用 read_file。",
                parsed.path
            )));
        }

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

    /// 错误契约：根目录不存在 → Err 带 did-you-mean，绝不静默返回空结果
    /// （「0 命中」必须如实代表「搜过了没搜到」，否则模型把路径错当没搜到继续瞎试）
    #[tokio::test]
    async fn nonexistent_root_errors_instead_of_empty_matches() {
        let tool = SearchFilesTool;
        let err = tool
            .execute(r#"{"path": "/nonexistent_dir_qq7x", "pattern": "foo"}"#)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("目录不存在"), "{err}");
        assert!(err.contains("list_directory") || err.contains("近似候选"), "{err}");
    }

    /// 根路径是文件：指路 read_file，不静默空结果
    #[tokio::test]
    async fn file_root_errors_with_read_file_hint() {
        let tool = SearchFilesTool;
        let err = tool
            .execute(r#"{"path": "Cargo.toml", "pattern": "foo"}"#)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("不是目录"), "{err}");
        assert!(err.contains("read_file"), "{err}");
    }

    /// 正则无效：错误带「怎么办」（语法示例），不只报语法错误
    #[tokio::test]
    async fn invalid_regex_error_gives_syntax_hint() {
        let tool = SearchFilesTool;
        let err = tool
            .execute(r#"{"path": ".", "pattern": "[unclosed"}"#)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("正则无效"), "{err}");
        assert!(err.contains("regex 语法"), "{err}");
    }

    /// 快乐路径：仓库内搜索真实命中（CWD = src-tauri）
    #[tokio::test]
    async fn searches_repo_and_finds_pattern() {
        let tool = SearchFilesTool;
        let out = tool
            .execute(r#"{"path": ".", "pattern": "ice[-_]paw", "include": "Cargo.toml"}"#)
            .await
            .unwrap();
        assert!(out.contains("Cargo.toml"), "{out}");
        assert!(!out.contains("\"matches\":0"), "{out}");
    }
}
