//! `templates` 表的 SQL 操作（仅 TemplateStage 使用）

use sqlx::SqlitePool;

use crate::db::models::TemplateRow;
use crate::error::{AppError, AppResult};

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<TemplateRow> {
    let row = sqlx::query_as::<_, TemplateRow>(
        "SELECT id, name, description, system_prompt, user_prompt_prefix,
                variables, tools, sort_order, created_at, updated_at
           FROM templates WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "template",
        id: id.to_string(),
    })?;
    Ok(row)
}
