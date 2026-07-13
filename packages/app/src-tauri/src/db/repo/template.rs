//! `templates` 表的 SQL 操作
//!
//! 约定：
//! - `variables` / `tools` 在 Rust 侧以 JSON 字符串存 SQLite，
//!   在 Model 层（`Template`）自动展开为强类型结构
//! - 列表按 `sort_order ASC, created_at ASC` 输出
//! - 上层负责生成 ID（UUID v4）

use sqlx::SqlitePool;

use crate::db::models::{NewTemplate, TemplateRow};
use crate::error::{AppError, AppResult};

/// 列出全部模板，按 sort_order asc, created_at asc
pub async fn list(pool: &SqlitePool) -> AppResult<Vec<TemplateRow>> {
    let rows = sqlx::query_as::<_, TemplateRow>(
        "SELECT id, name, description, system_prompt, user_prompt_prefix,
                variables, tools, sort_order, created_at, updated_at
           FROM templates
          ORDER BY sort_order ASC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 按 id 取一条；找不到返回 `AppError::NotFound`
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

/// 创建模板
pub async fn create(pool: &SqlitePool, new: &NewTemplate, id: &str) -> AppResult<TemplateRow> {
    let variables_json = serde_json::to_string(&new.variables.clone().unwrap_or_default())?;
    let tools_json = serde_json::to_string(&new.tools.clone().unwrap_or_default())?;

    sqlx::query(
        "INSERT INTO templates
            (id, name, description, system_prompt, user_prompt_prefix,
             variables, tools, sort_order)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&new.name)
    .bind(&new.description)
    .bind(&new.system_prompt)
    .bind(&new.user_prompt_prefix)
    .bind(&variables_json)
    .bind(&tools_json)
    .bind(new.sort_order)
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// 部分更新（partial update）：None 字段不动
///
/// - `variables` / `tools` 整体替换为新值（Some(vec![]) 表示清空）
/// - 字符串字段：传 Some("") 表示清空，传 Some("...") 表示覆盖
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
    system_prompt: Option<&str>,
    user_prompt_prefix: Option<&str>,
    variables: Option<&Vec<crate::db::models::TemplateVariable>>,
    tools: Option<&Vec<String>>,
    sort_order: Option<i32>,
) -> AppResult<TemplateRow> {
    // 先读出来再合并，避免拼接动态 SQL
    let mut current = get_by_id(pool, id).await?;

    if let Some(v) = name {
        current.name = v.to_string();
    }
    if let Some(v) = description {
        current.description = v.to_string();
    }
    if let Some(v) = system_prompt {
        current.system_prompt = v.to_string();
    }
    if let Some(v) = user_prompt_prefix {
        current.user_prompt_prefix = v.to_string();
    }
    if let Some(v) = variables {
        current.variables = serde_json::to_string(v)?;
    }
    if let Some(v) = tools {
        current.tools = serde_json::to_string(v)?;
    }
    if let Some(v) = sort_order {
        current.sort_order = v;
    }

    sqlx::query(
        "UPDATE templates
            SET name = ?, description = ?, system_prompt = ?, user_prompt_prefix = ?,
                variables = ?, tools = ?, sort_order = ?
          WHERE id = ?",
    )
    .bind(&current.name)
    .bind(&current.description)
    .bind(&current.system_prompt)
    .bind(&current.user_prompt_prefix)
    .bind(&current.variables)
    .bind(&current.tools)
    .bind(current.sort_order)
    .bind(id)
    .execute(pool)
    .await?;

    get_by_id(pool, id).await
}

/// 删除模板
pub async fn delete(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let affected = sqlx::query("DELETE FROM templates WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();

    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "template",
            id: id.to_string(),
        });
    }
    Ok(())
}
