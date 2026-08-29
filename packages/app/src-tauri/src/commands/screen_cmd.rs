//! 屏幕通道命令（批次④ 步骤 1+3）：`screen_channel_open` / `screen_channel_stop` /
//! `get_screen_channel_state` + 暂停/恢复/手动授予/脱离四件。
//!
//! 通道生命周期走 tracing（`ice_paw.screen_channel`）不进 session_events——
//! 开/关由用户命令触发、无 conv/turn 容器，伪造 turn_id 会毒化 reconcile 的
//! turn 锚点分组（§4.11 评审 A4）。

use sqlx::SqlitePool;
use tauri::{AppHandle, State};

use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::mcp::screen::channel::{self, ScreenChannelState};

/// 开启通道（聊天头开关主入口）：开启者会话即刻附着（免 Confirm，§4.1）。
/// 通道已 Active 时 = 把调用会话加入共享（附着），不重复「开启」。
#[tauri::command]
pub async fn screen_channel_open(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    conversation_id: String,
) -> AppResult<ScreenChannelState> {
    if conversation_id.trim().is_empty() {
        return Err(AppError::Validation(
            "screen 通道开启失败: 会话 ID 为空——请先选中一个会话再开启屏幕共享".into(),
        ));
    }
    let conv = repo::conversation::get_by_id(pool.inner(), &conversation_id)
        .await
        .map_err(|_| {
            AppError::Validation(format!(
                "screen 通道开启失败: 会话 {conversation_id} 不存在——\
                 会话可能已被删除，刷新会话列表后重试"
            ))
        })?;
    let agent = repo::agent::get_by_id(pool.inner(), &conv.agent_id).await.ok();
    let info = channel::AttachInfo {
        agent_name: agent
            .map(|a| a.name)
            .unwrap_or_else(|| "未知 agent".into()),
        conv_title: conv.title,
        purpose: String::new(),
    };
    let ch = channel::global();
    let newly = ch.open(&conversation_id, info.clone());
    tracing::info!(
        target: "ice_paw.screen_channel",
        conv = %conversation_id, agent = %info.agent_name, newly_opened = newly,
        "屏幕通道开启/附着（用户开关）"
    );
    channel::emit_state(&app);
    Ok(ch.snapshot())
}

/// 关闭通道（终止键的步骤 1 形态；HUD 常驻终止键在步骤 2）。
/// Off 状态调用幂等。park/排队中的会话在步骤 3 起会收到家族错误
/// `screen 通道已关闭`（本步无 gate，无挂起者）。
#[tauri::command]
pub async fn screen_channel_stop(app: AppHandle) -> AppResult<ScreenChannelState> {
    let ch = channel::global();
    if let Some(cleared) = ch.stop() {
        tracing::info!(
            target: "ice_paw.screen_channel",
            sessions = cleared.len(),
            "屏幕通道关闭（用户终止），附着会话已清空"
        );
        channel::emit_closed(&app, "user");
        channel::emit_state(&app);
    }
    Ok(ch.snapshot())
}

/// 通道状态拉取（开关/未来 HUD 初始化用；运行期更新走 screen:channel-state 事件）。
#[tauri::command]
pub async fn get_screen_channel_state() -> AppResult<ScreenChannelState> {
    Ok(channel::global().snapshot())
}

/// 暂停（§4.4：读写 gate 全部 park；通道/授权/附着保持——播放器语义）。
/// Off 状态幂等无操作。挂起中的会话被「停止生成」打断时收到家族错误
/// `screen 通道已暂停`（评审 B1 取消感知 park）。
#[tauri::command]
pub async fn screen_channel_pause(app: AppHandle) -> AppResult<ScreenChannelState> {
    let ch = channel::global();
    if ch.is_active() {
        ch.pause();
        tracing::info!(target: "ice_paw.screen_channel", "屏幕通道暂停（读写挂起）");
        channel::emit_state(&app);
    }
    Ok(ch.snapshot())
}

/// 恢复：park 中的读写 gate 被唤醒继续。Off 状态幂等无操作。
#[tauri::command]
pub async fn screen_channel_resume(app: AppHandle) -> AppResult<ScreenChannelState> {
    let ch = channel::global();
    if ch.is_active() {
        ch.resume();
        tracing::info!(target: "ice_paw.screen_channel", "屏幕通道恢复");
        channel::emit_state(&app);
    }
    Ok(ch.snapshot())
}

/// 手动切换写令牌（HUD 队列「授予」；步骤 2 接 HUD，先备命令面）：
/// 目标会话立即持有；原持有者入队尾（评审 B9——不入队即丢失唤醒），不中止其回合。
#[tauri::command]
pub async fn screen_channel_grant(
    app: AppHandle,
    conversation_id: String,
) -> AppResult<ScreenChannelState> {
    let ch = channel::global();
    if !ch.is_active() {
        return Err(AppError::Validation(
            "screen 通道未开启: 无法授予屏幕操作权——通道已关闭".into(),
        ));
    }
    if !ch.is_attached(&conversation_id) {
        return Err(AppError::Validation(format!(
            "screen 通道授予失败: 会话 {conversation_id} 未加入屏幕共享——\
             只有已加入的会话能获得操作权"
        )));
    }
    ch.grant(&conversation_id);
    tracing::info!(
        target: "ice_paw.screen_channel",
        conv = %conversation_id,
        "写令牌手动授予（用户切换）"
    );
    channel::emit_state(&app);
    Ok(ch.snapshot())
}

/// 会话脱离通道（连带归还令牌/摘除排队位；HUD「移除会话」/清理钩子用）。
#[tauri::command]
pub async fn screen_channel_detach(
    app: AppHandle,
    conversation_id: String,
) -> AppResult<ScreenChannelState> {
    let ch = channel::global();
    ch.detach(&conversation_id);
    channel::emit_state(&app);
    Ok(ch.snapshot())
}
