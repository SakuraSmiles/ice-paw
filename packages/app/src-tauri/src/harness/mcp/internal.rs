//! 内置 MCP 工具客户端（read_file / list_directory）
//!
//! Phase 1: 从 `tool_registry/builtin.rs` 迁移，实现 `McpClient` trait。
//!
//! 两个工具均为安全只读操作，在 Rust 侧直接执行（不走外部进程）。

use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

use super::client::McpClient;
use super::types::AuthorizationLevel;

// =========================================================================
// read_file
// =========================================================================

/// `read_file` 工具：读取本地文件内容（大文件自动分页）
pub struct ReadFileTool;

/// 大文件阈值（超过则自动分页，按行返回）
const LARGE_FILE_THRESHOLD: usize = 30_000;

/// 自动分页时的默认行数
const DEFAULT_PAGE_LINES: usize = 200;

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
    #[serde(default = "default_max_read_bytes")]
    max_bytes: usize,
    /// 从第几行开始读取（0-based），用于分页读取大文件
    #[serde(default)]
    offset: usize,
    /// 最多读取多少行（None = 自动决定）
    #[serde(default)]
    limit: Option<usize>,
}

fn default_max_read_bytes() -> usize {
    1024 * 1024 // 1MB 默认上限
}

#[async_trait]
impl McpClient for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a local file. Large files (>30KB) are automatically \
         paginated by lines — use the offset parameter to read subsequent pages. \
         The response includes total_lines, has_more, and next_offset for pagination."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file to read."
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (0-based). Use for paginating large files.",
                    "default": 0
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read. If omitted, auto-determined by file size.",
                }
            },
            "required": ["path"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: ReadFileArgs =
            serde_json::from_str(args).map_err(|e| AppError::Validation(format!(
                "read_file 参数解析失败: {e}"
            )))?;

        let path = Path::new(&parsed.path);

        // 安全检查：拒绝读取特殊文件
        let canonical = path.canonicalize().map_err(|e| {
            AppError::Validation(format!("文件路径无效: {e}"))
        })?;

        let path_str = canonical.to_string_lossy();
        if path_str.starts_with("/proc/") || path_str.starts_with("/sys/") || path_str.starts_with("/dev/") {
            return Err(AppError::Validation(
                "出于安全原因，不允许读取系统虚拟文件系统".into(),
            ));
        }

        let metadata = tokio::fs::metadata(&canonical).await.map_err(|e| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("文件不存在或不可访问: {e}"),
            ))
        })?;

        let file_size = metadata.len() as usize;
        if file_size > parsed.max_bytes {
            return Err(AppError::Validation(format!(
                "文件过大: {} bytes > {} bytes 上限",
                file_size,
                parsed.max_bytes
            )));
        }

        let bytes = tokio::fs::read(&canonical)
            .await
            .map_err(AppError::Io)?;

        let decoded = crate::infra::decode::decode_bytes(&bytes);
        let content = decoded.text;

        // 按行分页：大文件或显式指定 offset/limit 时分页返回
        let all_lines: Vec<&str> = content.lines().collect();
        let total_lines = all_lines.len();

        // 决定是否分页
        let need_pagination = file_size > LARGE_FILE_THRESHOLD
            || parsed.offset > 0
            || parsed.limit.is_some();

        if !need_pagination {
            // 小文件 + 无分页参数 → 直接返回全部
            #[derive(Serialize)]
            struct ReadFileResult {
                path: String,
                size: usize,
                encoding: String,
                total_lines: usize,
                content: String,
            }
            let result = ReadFileResult {
                path: parsed.path,
                size: file_size,
                encoding: decoded.encoding.to_string(),
                total_lines,
                content,
            };
            return Ok(serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()));
        }

        // 分页逻辑
        let page_limit = parsed.limit.unwrap_or(DEFAULT_PAGE_LINES);
        let start = parsed.offset.min(total_lines);
        let end = (start + page_limit).min(total_lines);
        let page_lines = &all_lines[start..end];
        let page_content = page_lines.join("\n");
        let has_more = end < total_lines;

        #[derive(Serialize)]
        struct ReadFilePageResult {
            path: String,
            size: usize,
            encoding: String,
            total_lines: usize,
            offset: usize,
            lines_returned: usize,
            has_more: bool,
            next_offset: Option<usize>,
            content: String,
        }

        let result = ReadFilePageResult {
            path: parsed.path,
            size: file_size,
            encoding: decoded.encoding.to_string(),
            total_lines,
            offset: start,
            lines_returned: page_lines.len(),
            has_more,
            next_offset: if has_more { Some(end) } else { None },
            content: page_content,
        };

        Ok(serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()))
    }
}

// =========================================================================
// list_directory
// =========================================================================

/// `list_directory` 工具：列出目录内容
pub struct ListDirectoryTool;

#[derive(Deserialize)]
struct ListDirectoryArgs {
    path: String,
}

#[async_trait]
impl McpClient for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List the contents of a local directory. Returns a list of files and subdirectories."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the directory to list."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: ListDirectoryArgs =
            serde_json::from_str(args).map_err(|e| AppError::Validation(format!(
                "list_directory 参数解析失败: {e}"
            )))?;

        let path = Path::new(&parsed.path);

        if !path.exists() {
            return Err(AppError::Validation(format!(
                "目录不存在: {}",
                parsed.path
            )));
        }

        if !path.is_dir() {
            return Err(AppError::Validation(format!(
                "路径不是目录: {}",
                parsed.path
            )));
        }

        let mut entries = Vec::new();

        let mut reader = tokio::fs::read_dir(path).await.map_err(|e| {
            AppError::Io(e)
        })?;

        #[derive(Serialize)]
        struct DirEntry {
            name: String,
            is_dir: bool,
            size: Option<u64>,
        }

        while let Some(entry) = reader.next_entry().await.map_err(AppError::Io)? {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            let size = if is_dir {
                None
            } else {
                entry.metadata().await.ok().map(|m| m.len())
            };

            entries.push(DirEntry { name, is_dir, size });
        }

        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string()))
    }
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_file_tool_valid() {
        let tool = ReadFileTool;
        let args = serde_json::json!({
            "path": "Cargo.toml",
            "max_bytes": 10240
        })
        .to_string();

        let result = tool.execute(&args).await;
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("ice-paw"));
    }

    #[tokio::test]
    async fn read_file_tool_nonexistent() {
        let tool = ReadFileTool;
        let args = r#"{"path": "/nonexistent/file/that/should/not/exist"}"#;

        let result = tool.execute(args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_file_tool_rejects_proc() {
        let tool = ReadFileTool;
        let args = r#"{"path": "/proc/self/status"}"#;

        let result = tool.execute(args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("安全") || msg.contains("不存在") || msg.contains("无效"),
            "should reject proc: {msg}"
        );
    }

    #[tokio::test]
    async fn list_directory_tool_valid() {
        let tool = ListDirectoryTool;
        let args = r#"{"path": "."}"#;

        let result = tool.execute(args).await;
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.starts_with("["));
    }

    #[tokio::test]
    async fn list_directory_tool_on_file() {
        let tool = ListDirectoryTool;
        let args = r#"{"path": "Cargo.toml"}"#;

        let result = tool.execute(args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_directory_tool_nonexistent() {
        let tool = ListDirectoryTool;
        let args = r#"{"path": "/nonexistent/directory"}"#;

        let result = tool.execute(args).await;
        assert!(result.is_err());
    }

    #[test]
    fn tool_def_generation() {
        let tool = ReadFileTool;
        let params = tool.parameters();
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["path"].is_object());
        assert_eq!(params["required"][0], "path");
    }

    #[test]
    fn read_file_auth_level() {
        let tool = ReadFileTool;
        assert_eq!(tool.authorization_level(), AuthorizationLevel::PathWhitelist);
    }

    #[test]
    fn list_directory_auth_level() {
        let tool = ListDirectoryTool;
        assert_eq!(tool.authorization_level(), AuthorizationLevel::Always);
    }
}
