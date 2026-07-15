//! Chat 收尾工具：CancellationToken 注销 + 成功 DB 回写 + 事件 emit
//!
//! 提供两个 pub(crate) 函数，供 chat_cmd.rs / stream_loop 调用：
//! - `cleanup()` — 所有退出路径的公共收尾（注销 CancellationToken）
//! - `cleanup_after_success_with_blocks()` — 正常完成时的 DB 回写 + emit chat:done + 注销

use tauri::{AppHandle, Emitter, Manager};
use sqlx::SqlitePool;

use crate::db::repo;
use crate::harness::chat_state::ChatState;
use crate::infra::protocol::{ChatDonePayload, ContentBlock};

/// 成功完成后的收尾：回写 content + content_blocks + emit done + 注销 token
pub(crate) fn cleanup_after_success_with_blocks(
    app: &AppHandle,
    pool: &SqlitePool,
    conv_id: &str,
    asst_msg_id: &str,
    content: &str,
    content_blocks: &[ContentBlock],
    finish_reason: &str,
    usage: Option<crate::infra::protocol::TokenUsage>,
) {
    let pool_clone = pool.clone();
    let asst_msg_id_clone = asst_msg_id.to_string();
    let content_clone = content.to_string();
    let blocks_json = serde_json::to_string(content_blocks).unwrap_or_else(|_| "[]".to_string());

    tokio::spawn(async move {
        let _ = repo::message::update_content(&pool_clone, &asst_msg_id_clone, &content_clone).await;
        let _ = repo::message::update_content_blocks(&pool_clone, &asst_msg_id_clone, &blocks_json).await;
    });

    let _ = app.emit(
        "chat:done",
        ChatDonePayload {
            conversation_id: conv_id.to_string(),
            message_id: asst_msg_id.to_string(),
            finish_reason: finish_reason.to_string(),
            usage,
        },
    );
    cleanup(app, pool, conv_id);
}

/// 注销 CancellationToken（所有退出路径的公共收尾）
pub(crate) fn cleanup(app: &AppHandle, _pool: &SqlitePool, conv_id: &str) {
    let chat_state = app.state::<ChatState>();
    chat_state.unregister(conv_id);
}
