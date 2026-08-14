//! Conversation 相关 Tauri Commands
//!
//! Frontend 调用入口见 `icepaw-cleanup-plan.md` §2.3。

use std::collections::HashMap;

use tauri::{Manager, State};
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::db::models::{Conversation, NewConversation};
use crate::db::repo;
use crate::error::AppResult;

/// 列出全部会话（不限 agent），按 pinned desc, updated_at desc
#[tauri::command]
pub async fn list_all_conversations(
    state: State<'_, SqlitePool>,
) -> AppResult<Vec<Conversation>> {
    let rows = repo::conversation::list_all(state.inner()).await?;
    Ok(rows.into_iter().map(Conversation::from).collect())
}

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

/// Task 3b: 更新对话级工具覆盖。
///
/// - `tools_override = None`：清除覆盖，恢复继承 Agent 配置。
/// - `tools_override = Some(map)`：写入 per-tool 勾选状态。
#[tauri::command]
pub async fn update_conversation_tools_override(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
    tools_override: Option<HashMap<String, bool>>,
) -> AppResult<()> {
    repo::conversation::update_tools_override(
        pool.inner(),
        &conversation_id,
        tools_override.as_ref(),
    )
    .await
}

/// 导出会话事件轨迹为 JSONL（session-event-log Phase 0 的最小只读出口）。
///
/// 每行一个事件对象（`session_events` 行，`payload` 内嵌为 JSON 对象而非转义
/// 字符串，便于肉眼核对），按 seq 正序——即权威回放序。文件名
/// `trajectory-{conversation_id}-{UTC时间戳}.jsonl`，写入用户下载目录
/// （Unix `$HOME/Downloads` / Windows `%USERPROFILE%\Downloads`，home 均缺失时
/// 回退 app 数据目录 `exports/`），返回写入的绝对路径。
///
/// 手验路径：发一条带工具调用的消息 → invoke 本命令 → 打开 JSONL 核对
/// turn_context → user_message → assistant_message → tool_execution →
/// tool_result_message → … → turn_ended 序列完整。
#[tauri::command]
pub async fn export_session_trajectory(
    app: tauri::AppHandle,
    pool: State<'_, SqlitePool>,
    conversation_id: String,
) -> AppResult<String> {
    // 会话存在性校验：不存在 → NotFound，而非导出空文件造成「无事件」误导
    repo::conversation::get_by_id(pool.inner(), &conversation_id).await?;
    let rows =
        repo::session_event::list_by_session(pool.inner(), &conversation_id, None).await?;

    let dir = exports_dir(&app)?;
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("trajectory-{conversation_id}-{ts}.jsonl"));

    // payload（TEXT 列）内嵌为 JSON 对象；万一存了非法 JSON 则原样降级为字符串
    let mut buf = String::new();
    for r in &rows {
        let payload_value: serde_json::Value =
            serde_json::from_str(&r.payload).unwrap_or(serde_json::Value::String(r.payload.clone()));
        let line = serde_json::json!({
            "id": r.id,
            "session_id": r.session_id,
            "seq": r.seq,
            "kind": r.kind,
            "actor": r.actor,
            "turn_id": r.turn_id,
            "message_id": r.message_id,
            "payload": payload_value,
            "created_at": r.created_at,
        });
        buf.push_str(&line.to_string());
        buf.push('\n');
    }

    // 文件写入为阻塞 IO，丢 spawn_blocking（同 log_cmd::get_logs 惯例）
    let path_str = path.display().to_string();
    tauri::async_runtime::spawn_blocking(move || std::fs::write(&path, buf))
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("导出任务失败: {e}")))?
        .map_err(|e| {
            crate::error::AppError::Internal(format!("写入轨迹文件失败: path={path_str} err={e}"))
        })?;
    Ok(path_str)
}

/// 解析导出目标目录：系统「下载」已知目录（Windows 走 SHGetKnownFolderPath，
/// 尊重 OneDrive 重定向——拼 `%USERPROFILE%\Downloads` 会在重定向机器上踩空建错目录），
/// 解析失败时回退 app 数据目录 `exports/`（与「数据目录」入口一致，用户可达）。
fn exports_dir(app: &tauri::AppHandle) -> AppResult<std::path::PathBuf> {
    let dir = match app.path().download_dir() {
        Ok(d) => d,
        Err(_) => crate::logging::data_dir(app)?.join("exports"),
    };
    std::fs::create_dir_all(&dir).map_err(|e| {
        crate::error::AppError::Internal(format!(
            "创建导出目录失败: dir={} err={e}",
            dir.display()
        ))
    })?;
    Ok(dir)
}
