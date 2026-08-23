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

/// 是否指向 IcePaw agent 配置文件（布局 `<workspaces>/agents/<id>/agent.yaml`）。
///
/// 用 [`Path::components`] 匹配路径段，跨平台（Windows `\` / Unix `/` 均正确），
/// 避免字符串 `contains` 的分隔符与大小写陷阱。文件名按 ASCII 大小写不敏感。
fn is_agent_config(path: &Path) -> bool {
    let is_yaml = path
        .file_name()
        .map(|n| n.eq_ignore_ascii_case("agent.yaml"))
        .unwrap_or(false);
    is_yaml && path.components().any(|c| c.as_os_str() == "agents")
}

/// 拒绝操作敏感路径（按原始串前缀判断，对新文件也生效）：
/// - Linux 虚拟文件系统 `/proc` `/sys` `/dev`
/// - IcePaw agent 配置文件 `agent.yaml`（改 agent 配置必须走 `propose_config_change`）
///
/// 后者是安全要害：agent 若能用 write_file/edit_file 直接改 agent.yaml，就会绕过
/// 配置提案审批系统与 guardrail 红线。所有写工具（write/edit/delete/move/create_dir）
/// 均经此函数兜底。
fn reject_sensitive(path: &Path) -> AppResult<()> {
    let s = path.to_string_lossy();
    if s.starts_with("/proc/") || s.starts_with("/sys/") || s.starts_with("/dev/") {
        return Err(AppError::Validation(format!(
            "出于安全原因，不允许操作敏感路径: {s}"
        )));
    }
    if is_agent_config(path) {
        return Err(AppError::Validation(
            "出于安全原因，不允许直接修改 agent 配置文件 (agent.yaml)，请使用 propose_config_change 工具发起配置提案。".into(),
        ));
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
    /// 默认 true：自动创建父目录。生产样本（2026-08-22）：write_file 连写 8 个
    /// 不存在目录下的文件 8 连败——「写新文件」不该先失败一次再学会带 flag。
    /// 显式传 false 才要求父目录已存在。
    #[serde(default = "default_create_dirs")]
    create_dirs: bool,
}

fn default_create_dirs() -> bool {
    true
}

#[async_trait]
impl McpClient for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write text content to a local file (overwrites if it exists). Missing parent \
directories are created automatically (default create_dirs=true)."
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
                    "default": true,
                    "description": "Create missing parent directories (default true)."
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
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    AppError::Validation(format!(
                        "write_file 创建父目录失败: {}: {e}。请确认路径合法且在授权工作区内。",
                        parent.display()
                    ))
                })?;
            }
        }

        // 修改前自动备份
        let backup = backup_if_exists(path)?;

        tokio::fs::write(path, &parsed.content).await.map_err(|e| {
            AppError::Validation(format!(
                "write_file 写入失败: {}: {e}。请确认路径合法且在授权工作区内；\
                 磁盘/权限问题请如实告知用户，勿重试同样调用。",
                parsed.path
            ))
        })?;

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

/// edit_file old_string 未命中时的近似诊断（报错即行为契约）。
///
/// 三档，每档指认差异所在 + 对应动作，杜绝模型「凭记忆再猜一次参数」：
/// ① 首行（trim 相等）在文件中出现 → 差异在细节（空白/换行/标点），报出行号；
/// ② 按空白归一后可匹配 → 差异纯在空白/缩进；
/// ③ 无相近内容 → old_string 出自记忆而非当前文件。
fn edit_mismatch_hint(content: &str, old_string: &str) -> String {
    let first_line = old_string.lines().next().unwrap_or("").trim();
    if !first_line.is_empty() {
        for (idx, line) in content.lines().enumerate() {
            if line.trim() == first_line {
                return format!(
                    "提示：old_string 首行出现在文件第 {} 行附近，但整体未精确匹配——\
                     差异可能在空白/换行/标点。请用 read_file 读取该区域，逐字符复制实际内容重写 old_string。",
                    idx + 1
                );
            }
        }
    }
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let normed_old = norm(old_string);
    if !normed_old.is_empty() && norm(content).contains(&normed_old) {
        return "提示：忽略空白后可匹配——差异在空白/缩进。请用 read_file 读取目标区域，按实际内容逐字符复制。".into();
    }
    "提示：文件中无相近内容——old_string 可能出自记忆而非当前文件。请先 read_file 读取文件实际内容。".into()
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

        if path.is_dir() {
            return Err(AppError::Validation(format!(
                "路径是目录不是文件: {}。edit_file 只能编辑文件。",
                parsed.path
            )));
        }

        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AppError::Validation(format!(
                    "文件不存在: {}。{}",
                    parsed.path,
                    super::path_suggest::suggest_for_missing(path)
                )));
            }
            Err(e) => {
                return Err(AppError::Validation(format!(
                    "edit_file 读取失败: {}: {e}",
                    parsed.path
                )));
            }
        };

        // 修改前自动备份
        let backup = backup_if_exists(path)?;

        let count = content.matches(&parsed.old_string).count();
        if count == 0 {
            return Err(AppError::Validation(format!(
                "edit_file: 未在 {} 中找到 old_string（要求逐字符精确匹配，含空白与缩进）。{}",
                parsed.path,
                edit_mismatch_hint(&content, &parsed.old_string)
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

// =========================================================================
// move_file（移动 / 重命名，源文件带备份）
// =========================================================================

/// `move_file` 工具：移动或重命名文件/目录（对标 filesystem server 的 move_file）
///
/// 同卷用 rename（原子）；跨卷 rename 失败时回退 copy + remove。源文件若存在，移动前
/// 自动备份（复用 [`backup_if_exists`]）。
///
/// **授权**：tool_executor 经 `source` 字段提取路径做白名单校验（见
/// `extract_path_from_args`），source 在 workspace 内则免授权；destination 由本工具的
/// `reject_sensitive` 兜底拦截敏感路径。
pub struct MoveFileTool;

#[derive(Deserialize)]
struct MoveFileArgs {
    source: String,
    destination: String,
}

/// 递归复制目录/文件（跨卷 move 回退用）。async 递归用 Box::pin 规避尺寸无限增长。
async fn copy_recursive(src: &Path, dst: &Path) -> AppResult<()> {
    let meta = tokio::fs::metadata(src).await.map_err(AppError::Io)?;
    if meta.is_dir() {
        tokio::fs::create_dir_all(dst).await.map_err(AppError::Io)?;
        let mut reader = tokio::fs::read_dir(src).await.map_err(AppError::Io)?;
        while let Some(entry) = reader.next_entry().await.map_err(AppError::Io)? {
            let child_src = entry.path();
            let child_dst = dst.join(entry.file_name());
            Box::pin(copy_recursive(&child_src, &child_dst)).await?;
        }
    } else {
        tokio::fs::copy(src, dst).await.map_err(AppError::Io)?;
    }
    Ok(())
}

#[async_trait]
impl McpClient for MoveFileTool {
    fn name(&self) -> &str {
        "move_file"
    }

    fn description(&self) -> &str {
        "Move or rename a file or directory. Same-volume moves are atomic; cross-volume \
falls back to copy+delete. The source file is backed up before moving."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "source": { "type": "string", "description": "Path to the file or directory to move." },
                "destination": { "type": "string", "description": "Target path." }
            },
            "required": ["source", "destination"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: MoveFileArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("move_file 参数解析失败: {e}")))?;

        let src = Path::new(&parsed.source);
        let dst = Path::new(&parsed.destination);
        reject_sensitive(src)?;
        reject_sensitive(dst)?;

        if !src.exists() {
            return Err(AppError::Validation(format!(
                "move_file: 源路径不存在: {}",
                parsed.source
            )));
        }

        // 源文件备份（目录不备份，与 delete_file 一致）
        let backup = if !src.is_dir() {
            backup_if_exists(src)?
        } else {
            None
        };

        // 目标父目录缺失则自动建（对标 server-filesystem move_file 的友好行为）
        if let Some(parent) = dst.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(AppError::Io)?;
        }

        // 同卷 rename 原子；跨卷失败（Linux EXDEV=18 / Win ERROR_NOT_SAME_DEVICE=17）回退 copy+remove。
        // 注：Windows 的 rename 带 MOVEFILE_COPY_ALLOWED，跨卷通常已自动复制成功，回退主要服务 Linux。
        match tokio::fs::rename(src, dst).await {
            Ok(()) => {}
            Err(e) if matches!(e.raw_os_error(), Some(18) | Some(17)) => {
                copy_recursive(src, dst).await?;
                if src.is_dir() {
                    tokio::fs::remove_dir_all(src).await.map_err(AppError::Io)?;
                } else {
                    tokio::fs::remove_file(src).await.map_err(AppError::Io)?;
                }
            }
            Err(e) => return Err(AppError::Io(e)),
        }

        Ok(serde_json::json!({
            "source": parsed.source,
            "destination": parsed.destination,
            "backup": backup,
        })
        .to_string())
    }
}

// =========================================================================
// create_directory
// =========================================================================

/// `create_directory` 工具：创建目录（含父目录，幂等——已存在不报错）
pub struct CreateDirectoryTool;

#[derive(Deserialize)]
struct CreateDirectoryArgs {
    path: String,
}

#[async_trait]
impl McpClient for CreateDirectoryTool {
    fn name(&self) -> &str {
        "create_directory"
    }

    fn description(&self) -> &str {
        "Create a directory, including any missing parent directories. Idempotent: \
succeeds if the directory already exists."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path of the directory to create." }
            },
            "required": ["path"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::PathWhitelist
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: CreateDirectoryArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("create_directory 参数解析失败: {e}")))?;

        let path = Path::new(&parsed.path);
        reject_sensitive(path)?;

        // create_dir_all 幂等：目录已存在时返回 Ok
        tokio::fs::create_dir_all(path)
            .await
            .map_err(AppError::Io)?;

        Ok(serde_json::json!({
            "path": parsed.path,
            "created": true,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // edit_file 的核心替换逻辑是纯字符串操作，这里通过 replacen 语义验证
    #[test]
    fn replacen_first_only() {
        let s = "a b a b";
        assert_eq!(s.replacen("a", "X", 1), "X b a b");
        assert_eq!(s.replace("a", "X"), "X b X b");
        assert_eq!(s.matches('a').count(), 2);
    }

    #[test]
    fn move_and_create_dir_auth_levels() {
        assert_eq!(
            MoveFileTool.authorization_level(),
            AuthorizationLevel::PathWhitelist
        );
        assert_eq!(
            CreateDirectoryTool.authorization_level(),
            AuthorizationLevel::PathWhitelist
        );
    }

    // ---- P0-B：agent.yaml 配置保护 ----

    #[test]
    fn is_agent_config_detects_agent_yaml() {
        // 标准布局：<workspaces>/agents/<id>/agent.yaml
        assert!(is_agent_config(std::path::Path::new(
            "/home/u/ws/agents/dev-2/agent.yaml"
        )));
        // Windows 盘符路径用正斜杠：Path 在 Windows/Unix 上都能正确解析。若硬编码
        // 反斜杠 "C:\\..."，Linux 上 '\' 非分隔符、整串变成单段文件名 → CI Linux 误报。
        assert!(is_agent_config(std::path::Path::new(
            "C:/Users/dabai/icepaw-workspaces/agents/dev-2/agent.yaml"
        )));
        assert!(is_agent_config(std::path::Path::new(
            "/x/agents/test-buddy/agent.yaml"
        )));
        // 真正的 Windows 反斜杠风格仅在 Windows 宿主上被 Path 正确解析——按平台门控
        // 保留该覆盖；生产中 Linux 永不收到反斜杠路径，不在 Linux CI 上跑。
        #[cfg(windows)]
        {
            assert!(is_agent_config(std::path::Path::new(
                "C:\\Users\\dabai\\icepaw-workspaces\\agents\\dev-2\\agent.yaml"
            )));
        }
        // 文件名 ASCII 大小写不敏感
        assert!(is_agent_config(std::path::Path::new(
            "/x/agents/dev-2/AGENT.YAML"
        )));
        assert!(is_agent_config(std::path::Path::new(
            "/x/agents/dev-2/Agent.Yaml"
        )));
    }

    #[test]
    fn is_agent_config_rejects_non_agent_files() {
        // 普通业务文件
        assert!(!is_agent_config(std::path::Path::new("/proj/src/foo.rs")));
        // 文件名是 agent.yaml 但无 agents 路径段 → 不算（避免误伤用户项目）
        assert!(!is_agent_config(std::path::Path::new("/proj/agent.yaml")));
        // agents 段但文件名非 agent.yaml
        assert!(!is_agent_config(std::path::Path::new(
            "/x/agents/dev-2/config.yaml"
        )));
    }

    #[test]
    fn reject_sensitive_blocks_agent_yaml() {
        let res = reject_sensitive(std::path::Path::new("/home/u/ws/agents/dev-2/agent.yaml"));
        assert!(res.is_err());
        let msg = res.unwrap_err().to_string();
        assert!(
            msg.contains("propose_config_change"),
            "错误应引导使用 propose_config_change，实际: {msg}"
        );
    }

    #[test]
    fn reject_sensitive_keeps_proc_guard() {
        // 回归：Linux 虚拟文件系统仍被拒
        assert!(reject_sensitive(std::path::Path::new("/proc/self/status")).is_err());
        assert!(reject_sensitive(std::path::Path::new("/sys/kernel/x")).is_err());
        assert!(reject_sensitive(std::path::Path::new("/dev/null")).is_err());
    }

    #[test]
    fn reject_sensitive_allows_normal_files() {
        assert!(reject_sensitive(std::path::Path::new("/proj/src/main.rs")).is_ok());
        assert!(reject_sensitive(std::path::Path::new("C:\\proj\\src\\main.rs")).is_ok());
    }

    // ---- Agent 质量拍（2026-08-23）：create_dirs 好默认 + 报错行为契约 ----

    /// 不带 create_dirs 的调用（模型最常见形态）应直接成功——父目录自动创建
    #[tokio::test]
    async fn write_file_creates_missing_parents_by_default() {
        let dir = std::env::temp_dir().join(format!("icepaw_wf_{}", uuid::Uuid::new_v4()));
        let target = dir.join("a/b/c.txt");
        let args =
            serde_json::json!({"path": target.to_string_lossy(), "content": "hi"}).to_string();
        let out = WriteFileTool
            .execute(&args)
            .await
            .expect("默认应自动建父目录并写入成功");
        assert!(out.contains("bytes_written"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hi");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 显式 create_dirs=false 失败时给契约文案（发生了什么 + 勿重试指令），非裸 io 错误
    #[tokio::test]
    async fn write_file_explicit_no_dirs_fails_with_contract() {
        let dir = std::env::temp_dir().join(format!("icepaw_wf_{}", uuid::Uuid::new_v4()));
        let target = dir.join("nope/x.txt");
        let args = serde_json::json!({
            "path": target.to_string_lossy(), "content": "x", "create_dirs": false
        })
        .to_string();
        let err = WriteFileTool.execute(&args).await.unwrap_err().to_string();
        assert!(err.contains("写入失败"), "报发生了什么: {err}");
        assert!(err.contains("勿重试同样调用"), "给行为指令: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 档①：首行能定位（trim 相等）→ 报行号，指认差异在细节
    #[test]
    fn edit_mismatch_hint_locates_first_line() {
        let content = "fn main() {\n    let x = 1;\n}\n";
        // 整体有一处空格数差异，首行完全相同
        let old = "fn main() {\n   let x = 1;\n}";
        let hint = edit_mismatch_hint(content, old);
        assert!(hint.contains("第 1 行"), "应报出首行行号: {hint}");
        assert!(hint.contains("read_file"), "应指向读取实际内容: {hint}");
    }

    /// 档②：首行 trim 不等但空白归一后匹配 → 差异纯在空白
    #[test]
    fn edit_mismatch_hint_whitespace_only() {
        let content = "alpha beta\ngamma\n";
        let old = "alpha  beta\ngamma"; // 首行双空格
        let hint = edit_mismatch_hint(content, old);
        assert!(hint.contains("空白"), "应指认空白差异: {hint}");
    }

    /// 档③：无相近内容 → old_string 出自记忆
    #[test]
    fn edit_mismatch_hint_nothing_close() {
        let hint = edit_mismatch_hint("print('hi')\n", "def totally_different():\n    pass");
        assert!(hint.contains("记忆"), "应指认凭记忆拼串: {hint}");
    }
}
