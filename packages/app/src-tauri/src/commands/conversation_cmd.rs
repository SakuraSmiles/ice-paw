//! Conversation 相关 Tauri Commands
//!
//! Frontend 调用入口见 `icepaw-cleanup-plan.md` §2.3。

use tauri::State;
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::db::models::{Conversation, NewConversation};
use crate::db::repo;
use crate::error::AppResult;

/// 列出 agent 下的全部会话（pinned desc, updated_at desc）
#[tauri::command]
pub async fn list_conversations(
    state: State<'_, SqlitePool>,
    agent_id: String,
) -> AppResult<Vec<Conversation>> {
    let rows = repo::conversation::list_by_agent(state.inner(), &agent_id).await?;
    Ok(rows.into_iter().map(Conversation::from).collect())
}

/// 创建会话
#[tauri::command]
pub async fn create_conversation(
    state: State<'_, SqlitePool>,
    input: NewConversation,
) -> AppResult<Conversation> {
    let id = Uuid::new_v4().to_string();
    let row = repo::conversation::create(state.inner(), &id, &input).await?;
    Ok(Conversation::from(row))
}

/// 重命名
#[tauri::command]
pub async fn rename_conversation(
    state: State<'_, SqlitePool>,
    id: String,
    title: String,
) -> AppResult<()> {
    repo::conversation::rename(state.inner(), &id, &title).await
}

/// 置顶 / 取消置顶
#[tauri::command]
pub async fn pin_conversation(
    state: State<'_, SqlitePool>,
    id: String,
    pinned: bool,
) -> AppResult<()> {
    repo::conversation::set_pinned(state.inner(), &id, pinned).await
}

/// 删除会话（级联清理 messages）
#[tauri::command]
pub async fn delete_conversation(
    state: State<'_, SqlitePool>,
    id: String,
) -> AppResult<()> {
    repo::conversation::delete(state.inner(), &id).await
}
