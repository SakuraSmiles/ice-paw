//! `user_preferences` 表的 SQL 操作
//!
//! key-value 存储，每个偏好项独立一行。
//! - `get_all`  读取全部行，反序列化为 `UserPreferences` struct
//! - `set`      UPSERT 单个 key

use std::path::PathBuf;

use sqlx::SqlitePool;

use crate::db::models::UserPreferences;
use crate::error::AppResult;

/// 已知的偏好字段名
const KNOWN_KEYS: &[&str] = &[
    "default_agent_id",
    "default_template_id",
    "on_startup",
    "language",
    "theme",
    "code_theme",
    "font_size",
    "default_workspace_path",
    "timezone",
    "embedding_provider",
    "embedding_model",
    "embedding_api_key",
    "embedding_base_url",
];

/// 系统默认工作空间根路径（安装即用，自动创建）
fn default_workspace_path() -> String {
    // Windows: %USERPROFILE%\Documents\icepaw-workspaces
    // macOS/Linux: ~/Documents/icepaw-workspaces
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    let mut path = PathBuf::from(base);
    path.push("Documents");
    path.push("icepaw-workspaces");
    path.to_string_lossy().to_string()
}

/// 确保目录存在（不存在则自动创建）
fn ensure_dir(path: &str) {
    let p = std::path::Path::new(path);
    if !p.exists() {
        let _ = std::fs::create_dir_all(p);
    }
}

/// 读取全部偏好设置，反序列化为 `UserPreferences`
///
/// 首次启动时自动初始化默认工作空间路径并落库。
pub async fn get_all(pool: &SqlitePool) -> AppResult<UserPreferences> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM user_preferences",
    )
    .fetch_all(pool)
    .await?;

    // 构造 JSON object，再反序列化为 UserPreferences
    let mut map = serde_json::Map::new();
    for (key, value) in &rows {
        if KNOWN_KEYS.contains(&key.as_str()) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(value) {
                map.insert(key.clone(), v);
            } else {
                map.insert(key.clone(), serde_json::Value::String(value.clone()));
            }
        }
    }

    let mut prefs: UserPreferences = serde_json::from_value(serde_json::Value::Object(map))
        .unwrap_or_default();

    // 首次启动：用户没设置过 → 自动初始化合理的默认值
    let needs_init = prefs.default_workspace_path.as_ref().is_none_or(|s| s.is_empty());
    if needs_init {
        let path = default_workspace_path();
        ensure_dir(&path);
        prefs.default_workspace_path = Some(path.clone());
        // 持久化到 DB，下次不再走初始化
        let _ = set(pool, "default_workspace_path", &path).await;
    }

    Ok(prefs)
}

/// UPSERT 单个偏好项
pub async fn set(pool: &SqlitePool, key: &str, value: &str) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO user_preferences (key, value, updated_at)
         VALUES (?, ?, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}
