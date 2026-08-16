//! 用户偏好设置相关 Tauri Commands
//!
//! - `get_preferences`  读取全部偏好
//! - `set_preference`   更新单个偏好（key-value）

use sqlx::SqlitePool;
use tauri::State;

use crate::db::models::UserPreferences;
use crate::db::repo;
use crate::error::AppResult;

/// 读取全部用户偏好设置
#[tauri::command]
pub async fn get_preferences(pool: State<'_, SqlitePool>) -> AppResult<UserPreferences> {
    repo::preferences::get_all(pool.inner()).await
}

/// 更新单个偏好项
///
/// `value` 接收字符串，前端传 JSON.stringify 后的字符串。
#[tauri::command]
pub async fn set_preference(
    pool: State<'_, SqlitePool>,
    key: String,
    value: String,
) -> AppResult<()> {
    repo::preferences::set(pool.inner(), &key, &value).await
}
