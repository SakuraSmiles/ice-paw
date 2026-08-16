//! Conversation 相关 Tauri Commands
//!
//! Frontend 调用入口见 `icepaw-cleanup-plan.md` §2.3。

use std::collections::HashMap;

use tauri::{Manager, State};
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::db::models::{Conversation, NewConversation, SessionEvent};
use crate::db::repo;
use crate::error::AppResult;

/// 列出全部会话（不限 agent），按 pinned desc, updated_at desc
#[tauri::command]
pub async fn list_all_conversations(state: State<'_, SqlitePool>) -> AppResult<Vec<Conversation>> {
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
pub async fn delete_conversation(state: State<'_, SqlitePool>, id: String) -> AppResult<()> {
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
    let rows = repo::session_event::list_by_session(pool.inner(), &conversation_id, None).await?;

    let dir = exports_dir(&app)?;
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("trajectory-{conversation_id}-{ts}.jsonl"));

    // payload（TEXT 列）内嵌为 JSON 对象；万一存了非法 JSON 则原样降级为字符串
    let mut buf = String::new();
    for r in &rows {
        let payload_value: serde_json::Value = serde_json::from_str(&r.payload)
            .unwrap_or(serde_json::Value::String(r.payload.clone()));
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

/// 读取一个会话的完整事件流（session-events）作为结构化列表。
///
/// 与 [`export_session_trajectory`] 同源（都走 `repo::session_event::list_by_session`，
/// seq 正序），但**不写文件**——直接返回 `Vec<SessionEvent>`，`payload` 已在服务端
/// parse 为 JSON 对象（非法 JSON 降级为字符串值，与导出一致）。供前端「轨迹回放」
/// 视图消费，免逐行 `JSON.parse`。
///
/// `limit` 为 `None` 时全量；`Some(n)` 时尾部优先（最新 n 条，`before_seq` 作游标向前
/// 翻页）；`limit=Some(n)` + `after_seq` 时正向增量（seq 严格大于游标的最早 n 条——轨迹
/// live 追加轮询用，返回空 = 已追平）。
///
/// 手验路径：DevTools
/// `await window.__TAURI_INTERNALS__.invoke('list_session_events', { conversationId: '…' })`
#[tauri::command]
pub async fn list_session_events(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
    limit: Option<i64>,
    before_seq: Option<i64>,
    after_seq: Option<i64>,
) -> AppResult<Vec<SessionEvent>> {
    // 会话存在性校验：不存在 → NotFound，与 export 一致（不返回空数组造成「无事件」误导）
    repo::conversation::get_by_id(pool.inner(), &conversation_id).await?;
    // limit=Some + after_seq → 正向增量（轨迹 live 追加轮询）；limit=Some → 尾部优先
    // 分页（before_seq 游标向前翻页）；None → 全量正序
    let rows = if let Some(n) = limit {
        if after_seq.is_some() {
            repo::session_event::list_after(pool.inner(), &conversation_id, after_seq, n).await?
        } else {
            repo::session_event::list_tail(pool.inner(), &conversation_id, before_seq, n).await?
        }
    } else {
        repo::session_event::list_by_session(pool.inner(), &conversation_id, None).await?
    };
    Ok(rows.into_iter().map(SessionEvent::from).collect())
}

/// 窗口前（`seq < before_seq` 一侧）的全局轮次数——轨迹「尾部优先分页」的轮号
/// 全局偏移（M3）：窗口内首个 turn 桶的真实轮号 = 偏移 + 1，翻页/首屏截断时
/// 轮次编号不再相对错位。前端在首载有更多分页时与每次「加载更早」后调用。
#[tauri::command]
pub async fn trajectory_turn_offset(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
    before_seq: i64,
) -> AppResult<i64> {
    repo::conversation::get_by_id(pool.inner(), &conversation_id).await?;
    repo::session_event::count_turns_before(pool.inner(), &conversation_id, before_seq).await
}

/// 会话轮次锚点列表（聊天「轮次导航条」UX #5）：一轮 = 一条用户消息，
/// 返回轻量 `{message_id, preview, created_at}`（repo 侧不加载大字段、SQL 截预览）。
/// 轮号由前端按下标 +1。供导航条目录渲染与跳转（配合消息分页补页到位）。
#[tauri::command]
pub async fn list_turn_anchors(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
) -> AppResult<Vec<crate::db::repo::message::TurnAnchor>> {
    repo::conversation::get_by_id(pool.inner(), &conversation_id).await?;
    repo::message::list_turn_anchors(pool.inner(), &conversation_id).await
}

/// 会话当前计划快照（任务胶囊「计划段」+ 计划卡取数用，C4/C5）。
///
/// 计划是全量覆写快照（`update_plan` 工具）→ **当前计划 = 最后一条
/// `plan_updated` 事件**。返回 None = 无计划（从未建立 / 最后一条为空清单 =
/// 已清空），UI 隐藏计划段。payload 损坏降级 None + warn（不吞错误类型）。
#[derive(serde::Serialize, Clone)]
pub struct SessionPlanSnapshot {
    pub items: Vec<crate::harness::event_log::PlanItem>,
    pub updated_at: String,
}

#[tauri::command]
pub async fn get_session_plan(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
) -> AppResult<Option<SessionPlanSnapshot>> {
    repo::conversation::get_by_id(pool.inner(), &conversation_id).await?;
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT payload, created_at FROM session_events \
         WHERE session_id = ? AND kind = 'plan_updated' \
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(&conversation_id)
    .fetch_optional(pool.inner())
    .await?;
    let Some((payload_json, updated_at)) = row else {
        return Ok(None);
    };
    match serde_json::from_str::<crate::harness::event_log::PlanUpdatedPayload>(&payload_json) {
        Ok(p) if !p.items.is_empty() => Ok(Some(SessionPlanSnapshot {
            items: p.items,
            updated_at,
        })),
        Ok(_) => Ok(None), // 空清单 = agent 主动清空计划
        Err(e) => {
            tracing::warn!(target: "ice_paw.plan", "plan_updated payload 损坏（降级无计划）: conv={conversation_id} err={e}");
            Ok(None)
        }
    }
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
        crate::error::AppError::Internal(format!("创建导出目录失败: dir={} err={e}", dir.display()))
    })?;
    Ok(dir)
}

/// 对账一个会话：session_events 回放（derive）vs messages 表提取（legacy），
/// 差异即 bug 清单（session-event-log Phase 1）。
///
/// 只读、无副作用——既不改事件也不改行。返回 [`ReconcileReport`]：
/// `diffs` 为未分类差异（非空 = 有 bug 嫌疑待查），`skipped` 为已文档化的
/// 已知容忍（epoch 前 legacy 行 / 不完整 turn / 错误行等，各有 reason）。
///
/// 手验路径：DevTools
/// `await window.__TAURI_INTERNALS__.invoke('reconcile_session', { conversationId: '…' })`。
#[tauri::command]
pub async fn reconcile_session(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
) -> AppResult<crate::harness::reconcile::ReconcileReport> {
    // 会话存在性校验：不存在 → NotFound，而非空报告造成「零差异」误导
    repo::conversation::get_by_id(pool.inner(), &conversation_id).await?;
    crate::harness::reconcile::reconcile_session(pool.inner(), &conversation_id).await
}

/// 读路径路由诊断（session-events Phase 2A）。
///
/// 返回路由器缓存的所有会话条目（各自走 derive / legacy 及原因）；可选传
/// `conversation_id` 当场解析该会话的决策（覆盖缓存，用于探测尚未聊过的会话）。
///
/// 手验路径：DevTools
/// ```js
/// await window.__TAURI_INTERNALS__.invoke('get_read_route_status', { conversationId: '…' })
/// // 或不传 conversationId 看全局快照
/// ```
#[tauri::command]
pub async fn get_read_route_status(
    pool: State<'_, SqlitePool>,
    route_registry: State<'_, crate::harness::read_route::ReadRouteRegistry>,
    conversation_id: Option<String>,
) -> AppResult<crate::harness::read_route::ReadRouteStatus> {
    let resolved = if let Some(cid) = &conversation_id {
        repo::conversation::get_by_id(pool.inner(), cid).await?;
        Some(route_registry.resolve(pool.inner(), cid, false).await?)
    } else {
        None
    };
    Ok(crate::harness::read_route::ReadRouteStatus {
        entries: route_registry.snapshot(),
        resolved,
    })
}
