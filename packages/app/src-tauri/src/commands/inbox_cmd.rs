//! MA-3 收件箱命令——hold 扣住来件的用户出口（查看/批准/拒绝/三态切换）。
//!
//! 授权语义全在用户手势侧：批准（allow=true）= settled(consumed) + 消费回合
//! （`by="user-approval"`，**不占自动消费配额**——配额只护栏 accept 政策的
//! 自动行为）；拒绝 = settled(refused)。投递/自动消费路径（工具/引擎）不经
//! 本文件——见 `harness/inbox.rs`。

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, State};

use crate::db::repo::{self, session_event};
use crate::error::{AppError, AppResult};
use crate::harness::event_log::{
    self, CrossSessionMessagePayload, CrossSessionMessageSettledPayload, EventCtx,
};
use crate::harness::inbox;

/// 收件箱单条 pending 的投影（payload 解包 + 定位字段）。
#[derive(Debug, Serialize)]
pub struct InboxItem {
    pub message_id: String,
    pub source_conversation_id: String,
    pub source_conversation_title: String,
    pub source_agent_id: String,
    pub source_agent_name: String,
    pub content: String,
    pub expect_reply: bool,
    pub delivered_at_unix: u64,
}

/// 收件箱视图：pending 列表（投递序）+ 当前收件政策。
#[derive(Debug, Serialize)]
pub struct InboxView {
    pub policy: String,
    pub items: Vec<InboxItem>,
}

/// 收件政策合法值（conversations.inbox_policy 词表）。
const POLICIES: &[&str] = &["accept", "hold", "refuse"];

/// 列出一个会话的收件箱（pending 来件 + 当前政策）。
#[tauri::command]
pub async fn list_inbox(pool: State<'_, SqlitePool>, conversation_id: String) -> AppResult<InboxView> {
    let conv = repo::conversation::get_by_id(pool.inner(), &conversation_id).await?;
    let rows = session_event::list_pending_inbox(pool.inner(), &conversation_id).await?;
    let mut items = Vec::with_capacity(rows.len());
    for r in &rows {
        match serde_json::from_str::<CrossSessionMessagePayload>(&r.payload) {
            Ok(p) => items.push(InboxItem {
                message_id: p.message_id,
                source_conversation_id: p.source_conversation_id,
                source_conversation_title: p.source_conversation_title,
                source_agent_id: p.source_agent_id,
                source_agent_name: p.source_agent_name,
                content: p.content,
                expect_reply: p.expect_reply,
                delivered_at_unix: p.delivered_at_unix,
            }),
            // 单条坏行不挡整个收件箱（derive 同款「记 issue 不吞」——这里
            // 以日志披露；该行仍占 pending 位，settle 后自然出队）
            Err(e) => tracing::warn!(
                target: "ice_paw.inbox",
                conv = %conversation_id,
                seq = r.seq,
                "来件 payload 解析失败（跳过渲染）: {e}"
            ),
        }
    }
    Ok(InboxView {
        policy: conv.inbox_policy,
        items,
    })
}

/// 全部会话的 pending 来件计数（boot 批量拉取——侧栏 badge 数据源）。
///
/// 返回 `(conversation_id, count)` 对；只含有来件的会话（零计数的会话不在
/// 返回里，前端 Map 查不到 = 0）。
#[tauri::command]
pub async fn list_inbox_counts(pool: State<'_, SqlitePool>) -> AppResult<Vec<(String, i64)>> {
    session_event::count_pending_inbox_all(pool.inner()).await
}

/// 切换收件政策（accept / hold / refuse）。
#[tauri::command]
pub async fn set_inbox_policy(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
    policy: String,
) -> AppResult<()> {
    if !POLICIES.contains(&policy.as_str()) {
        return Err(AppError::Validation(format!(
            "无效收件政策 '{policy}'——合法值：accept / hold / refuse"
        )));
    }
    repo::conversation::update_inbox_policy(pool.inner(), &conversation_id, &policy).await
}

/// 处置一条 pending 来件：批准（消费回合）或拒绝（settled refused）。
///
/// 批准时会话忙 → Err「正在生成中」（消息留 pending 零丢失——避免
/// 「settled 已写但消费未发生」的丢消息窗口，消费引擎的 ChatState 仲裁
/// 与此一致）。`message_id` 不在 pending 集（已处置/未知）→ Err NotFound。
#[tauri::command]
pub async fn respond_inbox_item(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    conversation_id: String,
    message_id: String,
    allow: bool,
) -> AppResult<()> {
    let conv = repo::conversation::get_by_id(pool.inner(), &conversation_id).await?;
    let pending = session_event::list_pending_inbox(pool.inner(), &conversation_id).await?;
    let ev = pending
        .iter()
        .find(|r| r.message_id.as_deref() == Some(message_id.as_str()))
        .cloned()
        .ok_or_else(|| AppError::NotFound {
            resource: "inbox_item",
            id: message_id.clone(),
        })?;

    if allow {
        // by=user-approval 不占自动配额；会话忙时 consume_pending 返回 Ok(false)
        let title = conv.title.clone();
        match inbox::consume_pending(&app, pool.inner(), conv, ev, "user-approval").await {
            Ok(true) => Ok(()),
            Ok(false) => Err(AppError::Validation(format!(
                "会话「{title}」正在生成中——来件已保留在收件箱，请等本轮结束后再批准"
            ))),
            Err(e) => Err(e),
        }
    } else {
        // 拒收：settled(refused)；turn_id 复原 cross:{id}（与投递/消费侧同一
        // 归组键），agent_id 取源 agent（ctx 构造对称，actor 仍恒 user）
        let payload: CrossSessionMessagePayload = serde_json::from_str(&ev.payload)
            .map_err(|e| AppError::Internal(format!("来件 payload 解析失败: {e}")))?;
        let ctx = EventCtx::new(
            &conversation_id,
            &format!("cross:{message_id}"),
            &payload.source_agent_id,
        );
        event_log::log_cross_session_message_settled(
            pool.inner(),
            &ctx,
            &CrossSessionMessageSettledPayload {
                v: 1,
                message_id,
                action: "refused".into(),
                by: "user-refused".into(),
            },
        )
        .await;
        Ok(())
    }
}
