//! 内置文件写入/编辑/删除工具：`write_file` / `edit_file` / `delete_file`
//!
//! 让 agent 具备改代码能力。三者均 `PathWhitelist` 授权（agent workspace 内自动放行，
//! 路径在 workspace 外才弹窗确认）。统一用 `path` 字段名，便于 tool_executor 提取做白名单。

use std::path::Path;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{AppError, AppResult};

use super::client::McpClient;
use super::types::AuthorizationLevel;

/// 拒绝操作 Linux 虚拟文件系统等敏感路径（按原始串前缀判断，对新文件也生效）
fn reject_sensitive(path: &Path) -> AppResult<()> {
    let s = path.to_string_lossy();
    if s.starts_with("/proc/") || s.starts_with("/sys/") || s.starts_with("/dev/") {
        return Err(AppError::Validation(format!(
            "出于安全原因，不允许操作敏感路径: {s}"
        )));
    }
    Ok(())
}

// =========================================================================
// write_file
// =========================================================================

/// `write_file` 工具：写入文件（覆盖）
pub struct WriteFileTool;

#[derive(Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
    #[serde(default)]
    create_dirs: bool,
}

#[async_trait]
impl McpClient for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write text content to a local file (overwrites if it exists). \
Use create_dirs=true to create parent directories."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or workspace-relative path to the file to write."
                },
                "content": {
                    "type": "string",
                    "description": "Full text content to write to the file."
                },
                "create_dirs": {
                    "type": "boolean",
                    "default": false,
                    "description": "Create parent directories if they don't exist."
                }
            },
            "required": ["path", "content"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: WriteFileArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("write_file 参数解析失败: {e}")))?;

        let path = Path::new(&parsed.path);
        reject_sensitive(path)?;

        if parsed.create_dirs {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(AppError::Io)?;
            }
        }

        tokio::fs::write(path, &parsed.content)
            .await
            .map_err(AppError::Io)?;

        Ok(serde_json::json!({
            "path": parsed.path,
            "bytes_written": parsed.content.len(),
        })
        .to_string())
    }
}

// =========================================================================
// edit_file（精准字符串替换，对标 Claude Code 的 Edit）
// =========================================================================

/// `edit_file` 工具：精准字符串替换
pub struct EditFileTool;

#[derive(Deserialize)]
struct EditFileArgs {
    path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

#[async_trait]
impl McpClient for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Replace a unique string in a file. old_string must match exactly (including \
whitespace) and be unique unless replace_all=true. Fails if old_string is not found or not unique."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to edit." },
                "old_string": { "type": "string", "description": "The exact string to replace." },
                "new_string": { "type": "string", "description": "The replacement string." },
                "replace_all": { "type": "boolean", "default": false, "description": "Replace all occurrences." }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: EditFileArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("edit_file 参数解析失败: {e}")))?;

        let path = Path::new(&parsed.path);
        reject_sensitive(path)?;

        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(AppError::Io)?;

        let count = content.matches(&parsed.old_string).count();
        if count == 0 {
            return Err(AppError::Validation(format!(
                "edit_file: 未在 {} 中找到 old_string",
                parsed.path
            )));
        }
        if count > 1 && !parsed.replace_all {
            return Err(AppError::Validation(format!(
                "edit_file: old_string 在 {} 中出现 {count} 次，不唯一。请提供更长上下文或设 replace_all=true",
                parsed.path
            )));
        }

        let new_content = if parsed.replace_all {
            content.replace(&parsed.old_string, &parsed.new_string)
        } else {
            content.replacen(&parsed.old_string, &parsed.new_string, 1)
        };
        tokio::fs::write(path, &new_content)
            .await
            .map_err(AppError::Io)?;

        Ok(serde_json::json!({
            "path": parsed.path,
            "replacements": if parsed.replace_all { count } else { 1 },
        })
        .to_string())
    }
}

// =========================================================================
// delete_file（文件或空目录）
// =========================================================================

/// `delete_file` 工具：删除文件或空目录（非空目录拒绝，避免递归误删）
pub struct DeleteFileTool;

#[derive(Deserialize)]
struct DeleteFileArgs {
    path: String,
}

#[async_trait]
impl McpClient for DeleteFileTool {
    fn name(&self) -> &str {
        "delete_file"
    }

    fn description(&self) -> &str {
        "Delete a file or an EMPTY directory. Non-empty directories are rejected \
(use run_command for recursive removal after confirming)."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file or empty directory to delete." }
            },
            "required": ["path"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: DeleteFileArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("delete_file 参数解析失败: {e}")))?;

        let path = Path::new(&parsed.path);
        reject_sensitive(path)?;

        let meta = tokio::fs::metadata(path).await.map_err(AppError::Io)?;
        if meta.is_dir() {
            // 仅删空目录：remove_dir 对非空目录会报错（安全）
            tokio::fs::remove_dir(path).await.map_err(AppError::Io)?;
        } else {
            tokio::fs::remove_file(path).await.map_err(AppError::Io)?;
        }

        Ok(serde_json::json!({ "path": parsed.path, "deleted": true }).to_string())
    }
}

#[cfg(test)]
mod tests {
    // edit_file 的核心替换逻辑是纯字符串操作，这里通过 replacen 语义验证
    #[test]
    fn replacen_first_only() {
        let s = "a b a b";
        assert_eq!(s.replacen("a", "X", 1), "X b a b");
        assert_eq!(s.replace("a", "X"), "X b X b");
        assert_eq!(s.matches('a').count(), 2);
    }
}
