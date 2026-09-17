//! 应用身份信息 Commands（设置-关于页）

use serde::Serialize;
use tauri::AppHandle;

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
