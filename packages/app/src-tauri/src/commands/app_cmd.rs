//! 应用身份信息 Commands（设置-关于页）+ 通用 OS 工具（open_path）

use serde::Serialize;
use tauri::AppHandle;

use crate::error::{AppError, AppResult};

/// 关于页展示的只读应用信息（productName / version / identifier 单一真相源 =
/// tauri.conf.json，与 `lib.rs::prune_webview_cache_on_version_change` 同源读取）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub product_name: String,
    pub version: String,
    pub identifier: String,
}

/// 返回应用身份信息（纯只读、无失败路径）。
#[tauri::command]
pub fn get_app_info(app: AppHandle) -> AppInfo {
    let config = app.config();
    AppInfo {
        product_name: config
            .product_name
            .clone()
            .unwrap_or_else(|| "IcePaw".to_string()),
        version: config
            .version
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        identifier: config.identifier.clone(),
    }
}

/// 用系统文件管理器打开指定路径（通用 OS 工具——「默认工作空间」等目录打开入口）。
/// 走后端 opener Rust API（同 log_cmd::open_data_dir 先例），打包后也能用。
#[tauri::command]
pub async fn open_path(path: String) -> AppResult<()> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation("路径为空，无法打开".into()));
    }
    let p = std::path::PathBuf::from(trimmed);
    if !p.exists() {
        return Err(AppError::NotFound {
            resource: "path",
            id: p.display().to_string(),
        });
    }
    tauri_plugin_opener::open_path(&p, None::<&str>)
        .map_err(|e| AppError::Internal(format!("打开路径失败: {e}")))?;
    Ok(())
}
