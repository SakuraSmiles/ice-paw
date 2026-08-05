//! 内置文件写入/编辑/删除工具：`write_file` / `edit_file` / `delete_file`
//!
//! 让 agent 具备改代码能力。三者均 `PathWhitelist` 授权（agent workspace 内自动放行，
//! 路径在 workspace 外才弹窗确认）。统一用 `path` 字段名，便于 tool_executor 提取做白名单。
//!
//! **自动备份**：write_file / edit_file / delete_file 在修改/删除已存在文件前，
//! 自动将原文件拷贝到同目录的 `.icepaw-backup/` 下（带时间戳），每个文件最多保留 10 份。

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{AppError, AppResult};

use super::client::McpClient;
use super::types::AuthorizationLevel;

/// 每个文件最多保留的备份数
const MAX_BACKUPS: usize = 10;

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

/// 修改/删除文件前自动备份（如果文件已存在）。
///
/// 备份到 `<parent>/.icepaw-backup/<timestamp>_<filename>`，
/// 每个文件最多保留 MAX_BACKUPS 份旧备份。
/// 返回备份路径（None = 文件不存在，无需备份）。
fn backup_if_exists(path: &Path) -> AppResult<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }

    let parent = path.parent().unwrap_or(Path::new("."));
    let backup_dir = parent.join(".icepaw-backup");
    std::fs::create_dir_all(&backup_dir)
        .map_err(|e| AppError::Io(std::io::Error::other(format!("创建备份目录失败: {e}"))))?;

    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".into());

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
    let backup_name = format!("{timestamp}_{filename}");
    let backup_path = backup_dir.join(&backup_name);

    std::fs::copy(path, &backup_path)
        .map_err(|e| AppError::Io(std::io::Error::other(format!("备份文件失败: {e}"))))?;

    // 清理旧备份（只保留最近 MAX_BACKUPS 个）
    cleanup_old_backups(&backup_dir, &filename)?;

    Ok(Some(backup_path.to_string_lossy().to_string()))
}

/// 清理同一文件的旧备份，只保留最近 MAX_BACKUPS 个。
fn cleanup_old_backups(backup_dir: &Path, original_filename: &str) -> AppResult<()> {
    let suffix = format!("_{original_filename}");
    let mut backups: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(backup_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(&suffix) {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        backups.push((entry.path(), modified));
                    }
                }
            }
        }
    }

    if backups.len() <= MAX_BACKUPS {
        return Ok(());
    }

    // 按修改时间降序，保留最新的 MAX_BACKUPS 个
    backups.sort_by_key(|b| std::cmp::Reverse(b.1));
    for (path, _) in &backups[MAX_BACKUPS..] {
        let _ = std::fs::remove_file(path);
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

        // 修改前自动备份
        let backup = backup_if_exists(path)?;

        tokio::fs::write(path, &parsed.content)
            .await
            .map_err(AppError::Io)?;

        Ok(serde_json::json!({
            "path": parsed.path,
            "bytes_written": parsed.content.len(),
            "backup": backup,
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

        // 修改前自动备份
        let backup = backup_if_exists(path)?;

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
            "backup": backup,
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

        // 删除前自动备份（仅文件，目录不备份）
        let backup = if !path.is_dir() {
            backup_if_exists(path)?
        } else {
            None
        };

        let meta = tokio::fs::metadata(path).await.map_err(AppError::Io)?;
        if meta.is_dir() {
            tokio::fs::remove_dir(path).await.map_err(AppError::Io)?;
        } else {
            tokio::fs::remove_file(path).await.map_err(AppError::Io)?;
        }

        Ok(serde_json::json!({
            "path": parsed.path,
            "deleted": true,
            "backup": backup,
        })
        .to_string())
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
