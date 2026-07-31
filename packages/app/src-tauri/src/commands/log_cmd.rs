//! 日志查看相关 Tauri Commands
//!
//! - `get_logs(line_count)`   tail 当前日志文件最近 N 行
//! - `get_data_dir()`         返回 app 数据目录路径
//! - `open_data_dir()`        用文件管理器打开数据目录

use tauri::AppHandle;

use crate::error::{AppError, AppResult};
use crate::logging;

/// 默认拉取行数 & 上限（防止前端误传超大值导致整文件读爆内存）
const DEFAULT_LINES: usize = 500;
const MAX_LINES: usize = 5000;

/// tail 当前日志文件最近 `line_count` 行。
///
/// 文件读取为阻塞 IO，丢到 `spawn_blocking` 避免占用异步运行时线程。
#[tauri::command]
pub async fn get_logs(app: AppHandle, line_count: Option<usize>) -> AppResult<Vec<String>> {
    let n = line_count.unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
    let dir = logging::log_dir(&app)?;
    let path = logging::current_log_path(&dir).ok_or_else(|| AppError::NotFound {
        resource: "log_file",
        id: dir.display().to_string(),
    })?;

    tauri::async_runtime::spawn_blocking(move || logging::tail_lines(&path, n))
        .await
        .map_err(|e| AppError::Internal(format!("读日志任务失败: {e}")))?
}

/// 返回 app 数据目录路径（前端「数据目录」行展示）。
#[tauri::command]
pub async fn get_data_dir(app: AppHandle) -> AppResult<String> {
    Ok(logging::data_dir(&app)?.display().to_string())
}

/// 用系统文件管理器打开数据目录。
///
/// 走后端 opener Rust API（不经前端 IPC 权限门控），打包后也能用。
#[tauri::command]
pub async fn open_data_dir(app: AppHandle) -> AppResult<()> {
    let dir = logging::data_dir(&app)?;
    tauri_plugin_opener::open_path(&dir, None::<&str>)
        .map_err(|e| AppError::Internal(format!("打开数据目录失败: {e}")))?;
    Ok(())
}
