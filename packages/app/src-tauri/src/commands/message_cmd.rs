//! Message 相关 Tauri Commands
//!
//! Frontend 调用入口见 `icepaw-cleanup-plan.md` §2.3。

use tauri::State;
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::db::models::{Message, NewMessage};
use crate::db::repo;
use crate::error::AppResult;

/// 列出会话内的消息
///
/// - `limit`：上限 1000，默认 100
/// - `before`：上一轮最末一条的 created_at，用于向前翻页
#[tauri::command]
pub async fn list_messages(
    state: State<'_, SqlitePool>,
    conversation_id: String,
    limit: Option<i64>,
    before: Option<String>,
) -> AppResult<Vec<Message>> {
    let rows = repo::message::list_by_conversation(
        state.inner(),
        &conversation_id,
        limit,
        before.as_deref(),
    )
    .await?;
    Ok(rows.into_iter().map(Message::from).collect())
}

/// 写入新消息
#[tauri::command]
pub async fn create_message(
    state: State<'_, SqlitePool>,
    input: NewMessage,
) -> AppResult<Message> {
    if input.conversation_id.trim().is_empty() {
        return Err(crate::error::AppError::Validation(
            "conversation_id 不能为空".into(),
        ));
    }
    if input.content.is_empty() {
        return Err(crate::error::AppError::Validation(
            "content 不能为空".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    let row = repo::message::create(state.inner(), &id, &input).await?;
    Ok(Message::from(row))
}
