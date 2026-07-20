//! `user_preferences` 表的 SQL 操作
//!
//! key-value 存储，每个偏好项独立一行。
//! - `get_all`  读取全部行，反序列化为 `UserPreferences` struct
//! - `set`      UPSERT 单个 key

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
];

/// 读取全部偏好设置，反序列化为 `UserPreferences`
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

    let prefs: UserPreferences = serde_json::from_value(serde_json::Value::Object(map))
        .unwrap_or_default();
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
