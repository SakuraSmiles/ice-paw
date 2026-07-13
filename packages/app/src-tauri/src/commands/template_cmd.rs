//! Template 相关 Tauri Commands
//!
//! 暴露给前端：
//! - `list_templates`      列出全部模板
//! - `get_template`        按 ID 取一条
//! - `create_template`     创建模板
//! - `update_template`     部分更新
//! - `delete_template`     删除
//!
//! 字段命名约定：与 `commands/agent_cmd.rs` 一致，
//! NewTemplate / TemplateUpdate 用 snake_case 字段名。

use tauri::State;
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::db::models::{NewTemplate, Template, TemplateUpdate};
use crate::db::repo;
use crate::error::{AppError, AppResult};

/// 列出全部模板
#[tauri::command]
pub async fn list_templates(state: State<'_, SqlitePool>) -> AppResult<Vec<Template>> {
    let rows = repo::template::list(state.inner()).await?;
    Ok(rows.into_iter().map(Template::from).collect())
}

/// 按 ID 取一条
#[tauri::command]
pub async fn get_template(
    state: State<'_, SqlitePool>,
    id: String,
) -> AppResult<Template> {
    let row = repo::template::get_by_id(state.inner(), &id).await?;
    Ok(Template::from(row))
}

/// 创建模板
#[tauri::command]
pub async fn create_template(
    state: State<'_, SqlitePool>,
    input: NewTemplate,
) -> AppResult<Template> {
    if input.name.trim().is_empty() {
        return Err(AppError::Validation("name 不能为空".into()));
    }

    let id = Uuid::new_v4().to_string();
    let row = repo::template::create(state.inner(), &input, &id).await?;
    Ok(Template::from(row))
}

/// 部分更新模板
#[tauri::command]
pub async fn update_template(
    state: State<'_, SqlitePool>,
    input: TemplateUpdate,
) -> AppResult<Template> {
    let row = repo::template::update(
        state.inner(),
        &input.id,
        input.name.as_deref(),
        input.description.as_deref(),
        input.system_prompt.as_deref(),
        input.user_prompt_prefix.as_deref(),
        input.variables.as_ref(),
        input.tools.as_ref(),
        input.sort_order,
    )
    .await?;
    Ok(Template::from(row))
}

/// 删除模板
#[tauri::command]
pub async fn delete_template(
    state: State<'_, SqlitePool>,
    id: String,
) -> AppResult<()> {
    repo::template::delete(state.inner(), &id).await
}
