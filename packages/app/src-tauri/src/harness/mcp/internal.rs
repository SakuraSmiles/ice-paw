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

/// 把已读入的字节解码为文本。
///
/// office/pdf（docx / xlsx / xls / xlsb / ods / pdf）走 [`crate::harness::doc::try_extract`]
/// 提取出文本/markdown；其余扩展名走原 [`crate::infra::decode::decode_bytes`]
/// （UTF-8 / GBK / lossy）文本解码。返回 `(content, encoding_label)`，office 路径
/// label 为 `extracted-{kind}`（如 `extracted-docx`）。
///
/// **设计**：office 解析失败显式返回 [`Err`]，**绝不静默退回 lossy 文本**——避免把
/// OOXML 二进制当 GBK 解出乱码、冒充"成功"读到内容（这正是 office 支持要根治的 bug）。
/// 单文件读（read_file）直接 `?` 上抛让 agent 看见；批量读（read_multiple_files）
/// 由调用方把 `Err` 转成单条 item 的 error 字段，不中断整批。
fn decode_bytes_or_extract(bytes: &[u8], ext: &str) -> AppResult<(String, String)> {
    if let Some(doc) = crate::harness::doc::try_extract(bytes, ext)? {
        Ok((doc.text, format!("extracted-{}", doc.kind.label())))
    } else {
        let decoded = crate::infra::decode::decode_bytes(bytes);
        Ok((decoded.text, decoded.encoding.to_string()))
    }
}

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
        "Read the contents of a local file. Supports plain text, code, and binary \
         office/PDF documents: .docx, .xlsx, .xls, .xlsb, .ods are extracted to \
         readable text/markdown (spreadsheets render as tables), .pdf is extracted \
         to text. Large files (>30KB) are automatically paginated by lines — use \
         the offset parameter to read subsequent pages. The response includes \
         total_lines, has_more, and next_offset for pagination."
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
        let parsed: ReadFileArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("read_file 参数解析失败: {e}")))?;

        let path = Path::new(&parsed.path);

        // 安全检查：拒绝读取特殊文件
        let canonical = path
            .canonicalize()
            .map_err(|e| AppError::Validation(format!("文件路径无效: {e}")))?;

        let path_str = canonical.to_string_lossy();
        if path_str.starts_with("/proc/")
            || path_str.starts_with("/sys/")
            || path_str.starts_with("/dev/")
        {
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
                file_size, parsed.max_bytes
            )));
        }

        let bytes = tokio::fs::read(&canonical).await.map_err(AppError::Io)?;

        // office/pdf 走文档提取（docx/xlsx/xls/xlsb/ods/pdf），其余走文本解码。
        // office 解析失败显式 Err（不退回乱码）。
        let ext = canonical.extension().and_then(|e| e.to_str()).unwrap_or("");
        let (content, encoding_label) = decode_bytes_or_extract(&bytes, ext)?;

        // 按行分页：大文件或显式指定 offset/limit 时分页返回
        let all_lines: Vec<&str> = content.lines().collect();
        let total_lines = all_lines.len();

        // 决定是否分页
        let need_pagination =
            file_size > LARGE_FILE_THRESHOLD || parsed.offset > 0 || parsed.limit.is_some();

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
                encoding: encoding_label.clone(),
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
            encoding: encoding_label.clone(),
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

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
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
        let parsed: ListDirectoryArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("list_directory 参数解析失败: {e}")))?;

        let path = Path::new(&parsed.path);

        if !path.exists() {
            return Err(AppError::Validation(format!("目录不存在: {}", parsed.path)));
        }

        if !path.is_dir() {
            return Err(AppError::Validation(format!(
                "路径不是目录: {}",
                parsed.path
            )));
        }

        let mut entries = Vec::new();

        let mut reader = tokio::fs::read_dir(path).await.map_err(AppError::Io)?;

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
// directory_tree
// =========================================================================

/// `directory_tree` 工具：递归输出目录树（JSON）
pub struct DirectoryTreeTool;

/// 递归最大深度（防止超深目录爆栈/爆输出）
const MAX_TREE_DEPTH: usize = 8;
/// 节点数硬上限（达到即截断，结果里 truncated=true）
const MAX_TREE_NODES: usize = 2000;

#[derive(Deserialize)]
struct DirectoryTreeArgs {
    path: String,
}

#[derive(Serialize)]
struct TreeNode {
    name: String,
    #[serde(rename = "type")]
    node_type: &'static str,
    size: Option<u64>,
    children: Option<Vec<TreeNode>>,
}

/// 应跳过的噪音目录（隐藏目录 + 常见构建/依赖目录），与 search.rs 的 is_skip_dir 同义
fn is_noise_dir(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules" | "target" | "dist" | "build" | "__pycache__" | "venv" | ".venv"
        )
}

/// 递归构建目录树。达到深度/节点上限时截断（不再下钻或返回 None）。
fn build_tree(path: &Path, depth: usize, node_count: &mut usize) -> Option<TreeNode> {
    if *node_count >= MAX_TREE_NODES {
        return None;
    }
    *node_count += 1;

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return None,
    };

    if meta.is_dir() {
        let mut children = Vec::new();
        // 未达深度上限才下钻
        if depth < MAX_TREE_DEPTH {
            if let Ok(entries) = std::fs::read_dir(path) {
                let mut sub: Vec<_> = entries.flatten().collect();
                sub.sort_by_key(|e| e.file_name());
                for entry in sub {
                    if *node_count >= MAX_TREE_NODES {
                        break;
                    }
                    let child_path = entry.path();
                    if let Some(fname) = child_path.file_name().and_then(|n| n.to_str()) {
                        if is_noise_dir(fname) {
                            continue;
                        }
                    }
                    if let Some(child) = build_tree(&child_path, depth + 1, node_count) {
                        children.push(child);
                    }
                }
            }
        }
        Some(TreeNode {
            name,
            node_type: "directory",
            size: None,
            children: Some(children),
        })
    } else {
        Some(TreeNode {
            name,
            node_type: "file",
            size: Some(meta.len()),
            children: None,
        })
    }
}

#[async_trait]
impl McpClient for DirectoryTreeTool {
    fn name(&self) -> &str {
        "directory_tree"
    }

    fn description(&self) -> &str {
        "Recursively list a directory as a JSON tree of files and subdirectories. \
Skips .git/node_modules/target/dist and hidden dirs. Caps depth (8) and node count \
(2000) to avoid huge outputs — check `truncated` in the result."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the directory to traverse."
                }
            },
            "required": ["path"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: DirectoryTreeArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("directory_tree 参数解析失败: {e}")))?;

        let path = Path::new(&parsed.path);
        if !path.exists() {
            return Err(AppError::Validation(format!("路径不存在: {}", parsed.path)));
        }

        let mut node_count = 0usize;
        let tree = build_tree(path, 0, &mut node_count)
            .ok_or_else(|| AppError::Validation("directory_tree: 节点数超上限".into()))?;

        Ok(serde_json::json!({
            "path": parsed.path,
            "nodes": node_count,
            "truncated": node_count >= MAX_TREE_NODES,
            "tree": tree,
        })
        .to_string())
    }
}

// =========================================================================
// get_file_info
// =========================================================================

/// `get_file_info` 工具：返回文件/目录的元信息
pub struct GetFileInfoTool;

#[derive(Deserialize)]
struct GetFileInfoArgs {
    path: String,
}

#[derive(Serialize)]
struct FileInfo {
    path: String,
    size: u64,
    is_dir: bool,
    is_file: bool,
    is_symlink: bool,
    readonly: bool,
    modified: Option<String>,
    created: Option<String>,
    accessed: Option<String>,
}

/// SystemTime → RFC3339（UTC）字符串
fn system_time_to_rfc(st: std::io::Result<std::time::SystemTime>) -> Option<String> {
    st.ok()
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
}

#[async_trait]
impl McpClient for GetFileInfoTool {
    fn name(&self) -> &str {
        "get_file_info"
    }

    fn description(&self) -> &str {
        "Return metadata for a file or directory: size, type (file/dir/symlink), \
readonly flag, and modified/created/accessed timestamps (RFC3339, UTC)."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to inspect."
                }
            },
            "required": ["path"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: GetFileInfoArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("get_file_info 参数解析失败: {e}")))?;

        let path = Path::new(&parsed.path);
        // symlink_metadata：不跟随符号链接，能识别链接本身
        let meta = tokio::fs::symlink_metadata(path)
            .await
            .map_err(AppError::Io)?;

        let is_symlink = meta.file_type().is_symlink();
        // 符号链接的 dir/file 判定需跟随目标
        let (is_dir, is_file) = if is_symlink {
            match tokio::fs::metadata(path).await {
                Ok(m) => (m.is_dir(), m.is_file()),
                Err(_) => (false, false),
            }
        } else {
            (meta.is_dir(), meta.is_file())
        };

        let info = FileInfo {
            path: parsed.path,
            size: meta.len(),
            is_dir,
            is_file,
            is_symlink,
            readonly: meta.permissions().readonly(),
            modified: system_time_to_rfc(meta.modified()),
            created: system_time_to_rfc(meta.created()),
            accessed: system_time_to_rfc(meta.accessed()),
        };

        Ok(serde_json::to_string(&info).unwrap_or_else(|_| "{}".to_string()))
    }
}

// =========================================================================
// read_multiple_files
// =========================================================================

/// `read_multiple_files` 工具：一次性读取多个文件
///
/// 单文件上限 1MB（更大的请单独 `read_file` 分页）。单文件失败不影响其余，逐项返回
/// ok/error。**授权**：因参数是路径数组、无法逐条自动放行，每次调用都会弹窗确认。
pub struct ReadMultipleFilesTool;

const MAX_MULTIPLE_FILES: usize = 20;
const MULTIPLE_FILE_MAX_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
struct ReadMultipleFilesArgs {
    paths: Vec<String>,
}

#[derive(Serialize)]
struct MultipleReadItem {
    path: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<usize>,
}

/// 读取单个文件（用于 read_multiple_files）。失败返回结构化 error，不抛异常。
async fn read_one_for_multiple(path_str: &str) -> MultipleReadItem {
    let path = Path::new(path_str);
    let canonical = match path.canonicalize() {
        Ok(c) => c,
        Err(e) => {
            return MultipleReadItem {
                path: path_str.into(),
                ok: false,
                error: Some(format!("路径无效或不存在: {e}")),
                content: None,
                bytes: None,
            };
        }
    };
    let s = canonical.to_string_lossy();
    if s.starts_with("/proc/") || s.starts_with("/sys/") || s.starts_with("/dev/") {
        return MultipleReadItem {
            path: path_str.into(),
            ok: false,
            error: Some("安全限制：不允许读取系统虚拟文件系统".into()),
            content: None,
            bytes: None,
        };
    }
    match tokio::fs::read(&canonical).await {
        Ok(bytes) => {
            if bytes.len() > MULTIPLE_FILE_MAX_BYTES {
                return MultipleReadItem {
                    path: path_str.into(),
                    ok: false,
                    error: Some(format!(
                        "文件过大（{} bytes > {} 上限），请用 read_file 分页读取",
                        bytes.len(),
                        MULTIPLE_FILE_MAX_BYTES
                    )),
                    content: None,
                    bytes: Some(bytes.len()),
                };
            }
            // office/pdf 走文档提取，其余走文本解码；office 解析失败 → 该项 error，不中断批量
            let ext = canonical.extension().and_then(|e| e.to_str()).unwrap_or("");
            match decode_bytes_or_extract(&bytes, ext) {
                Ok((text, _enc)) => MultipleReadItem {
                    path: path_str.into(),
                    ok: true,
                    error: None,
                    content: Some(text),
                    bytes: Some(bytes.len()),
                },
                Err(e) => MultipleReadItem {
                    path: path_str.into(),
                    ok: false,
                    error: Some(format!("文档解析失败: {e}")),
                    content: None,
                    bytes: Some(bytes.len()),
                },
            }
        }
        Err(e) => MultipleReadItem {
            path: path_str.into(),
            ok: false,
            error: Some(format!("读取失败: {e}")),
            content: None,
            bytes: None,
        },
    }
}

#[async_trait]
impl McpClient for ReadMultipleFilesTool {
    fn name(&self) -> &str {
        "read_multiple_files"
    }

    fn description(&self) -> &str {
        "Read up to 20 files in one call. Supports plain text/code and office/PDF \
(.docx/.xlsx/.xls/.xlsb/.ods/.pdf are extracted to text). Files >1MB are skipped \
(use read_file to paginate). Individual failures don't abort the batch — each \
result carries its own ok/error. Note: always prompts for confirmation since \
multiple paths can't be auto-whitelisted."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of file paths to read (max 20)."
                }
            },
            "required": ["paths"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: ReadMultipleFilesArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("read_multiple_files 参数解析失败: {e}")))?;

        if parsed.paths.is_empty() {
            return Err(AppError::Validation(
                "read_multiple_files: paths 不能为空".into(),
            ));
        }

        let requested = parsed.paths.len();
        let take = requested.min(MAX_MULTIPLE_FILES);
        let mut results = Vec::with_capacity(take);
        for path_str in parsed.paths.iter().take(take) {
            results.push(read_one_for_multiple(path_str).await);
        }

        Ok(serde_json::json!({
            "requested": requested,
            "returned": results.len(),
            "truncated": requested > MAX_MULTIPLE_FILES,
            "results": results,
        })
        .to_string())
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
        assert_eq!(
            tool.authorization_level(),
            AuthorizationLevel::PathWhitelist
        );
    }

    #[test]
    fn list_directory_auth_level() {
        let tool = ListDirectoryTool;
        assert_eq!(
            tool.authorization_level(),
            AuthorizationLevel::PathWhitelist
        );
    }

    #[test]
    fn is_noise_dir_skips_common() {
        // directory_tree 的过滤逻辑：隐藏目录 + 常见构建/依赖目录
        assert!(is_noise_dir(".git"));
        assert!(is_noise_dir("node_modules"));
        assert!(is_noise_dir("target"));
        assert!(is_noise_dir("dist"));
        assert!(is_noise_dir(".venv"));
        assert!(!is_noise_dir("src"));
        assert!(!is_noise_dir("main.rs"));
    }

    #[test]
    fn new_file_tools_auth_levels() {
        assert_eq!(
            DirectoryTreeTool.authorization_level(),
            AuthorizationLevel::PathWhitelist
        );
        assert_eq!(
            GetFileInfoTool.authorization_level(),
            AuthorizationLevel::PathWhitelist
        );
        assert_eq!(
            ReadMultipleFilesTool.authorization_level(),
            AuthorizationLevel::PathWhitelist
        );
    }
}
