//! 屏幕通道命令（批次④ 步骤 1+3）：`screen_channel_open` / `screen_channel_stop` /
//! `get_screen_channel_state` + 暂停/恢复/手动授予/脱离四件 + HUD 显示器切换。
//!
//! 通道生命周期走 tracing（`ice_paw.screen_channel`）不进 session_events——
//! 开/关由用户命令触发、无 conv/turn 容器，伪造 turn_id 会毒化 reconcile 的
//! turn 锚点分组（§4.11 评审 A4）。
//!
//! 步骤 2 起 `screen:channel-state` 由 channel 的 bump 自动广播（gate 路径的
//! 令牌/队列变化不经命令层），命令层不再手动 emit；窗口（HUD/红边框）随
//! Off↔Active 转换建/毁（`hud::ensure_windows` / `hud::destroy_windows`）。

use sqlx::SqlitePool;
use tauri::{AppHandle, State};

use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::mcp::screen::channel::{self, ScreenChannelState};
use crate::harness::mcp::screen::hud;

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
    if newly {
        hud::ensure_windows(&app);
    }
    Ok(ch.snapshot())
}

/// 关闭通道（HUD 终止键 / 聊天头开关）。Off 状态调用幂等。
/// park/排队中的会话被 bump 唤醒后收到家族错误 `screen 通道已关闭`。
#[tauri::command]
pub async fn screen_channel_stop(app: AppHandle) -> AppResult<ScreenChannelState> {
    let ch = channel::global();
    if let Some(cleared) = ch.stop() {
        tracing::info!(
            target: "ice_paw.screen_channel",
            sessions = cleared.len(),
            "屏幕通道关闭（用户终止），附着会话已清空"
        );
        hud::destroy_windows(&app);
        channel::emit_closed(&app, "user");
    }
    Ok(ch.snapshot())
}

/// 通道状态拉取（开关/HUD 初始化/轮询用；运行期更新走 screen:channel-state 事件）。
#[tauri::command]
pub async fn get_screen_channel_state() -> AppResult<ScreenChannelState> {
    Ok(channel::global().snapshot())
}

/// 暂停（§4.4：读写 gate 全部 park；通道/授权/附着保持——播放器语义）。
/// Off 状态幂等无操作。挂起中的会话被「停止生成」打断时收到家族错误
/// `screen 操作取消`（评审 B1 取消感知 park）。
#[tauri::command]
pub async fn screen_channel_pause() -> AppResult<ScreenChannelState> {
    let ch = channel::global();
    if ch.is_active() {
        ch.pause();
        tracing::info!(target: "ice_paw.screen_channel", "屏幕通道暂停（读写挂起）");
    }
    Ok(ch.snapshot())
}

/// 恢复：park 中的读写 gate 被唤醒继续。Off 状态幂等无操作。
#[tauri::command]
pub async fn screen_channel_resume() -> AppResult<ScreenChannelState> {
    let ch = channel::global();
    if ch.is_active() {
        ch.resume();
        tracing::info!(target: "ice_paw.screen_channel", "屏幕通道恢复");
    }
    Ok(ch.snapshot())
}

/// 手动切换写令牌（HUD 队列「授予」）：目标会话立即持有；原持有者入队尾
/// （评审 B9——不入队即丢失唤醒），不中止其回合。
#[tauri::command]
pub async fn screen_channel_grant(conversation_id: String) -> AppResult<ScreenChannelState> {
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
    Ok(ch.snapshot())
}

/// 会话脱离通道（连带归还令牌/摘除排队位；HUD「移除会话」/清理钩子用）。
#[tauri::command]
pub async fn screen_channel_detach(conversation_id: String) -> AppResult<ScreenChannelState> {
    let ch = channel::global();
    ch.detach(&conversation_id);
    Ok(ch.snapshot())
}

/// HUD 显示器切换（HUD ◀▶ 键）：delta ±1，按显示器总数取模环绕。
/// 显示器列表与索引属 tauri available_monitors（与 GDI backend.monitors 顺序
/// 无对应——HUD 定位是窗口层自治域）。
#[tauri::command]
pub async fn screen_channel_cycle_hud_monitor(
    app: AppHandle,
    delta: i32,
) -> AppResult<ScreenChannelState> {
    let ch = channel::global();
    if !ch.is_active() {
        return Ok(ch.snapshot());
    }
    let count = app.available_monitors().map(|m| m.len()).unwrap_or(1).max(1);
    let current = ch.hud_monitor() as isize;
    let next = (current + delta as isize).rem_euclid(count as isize) as usize;
    ch.set_hud_monitor(next);
    hud::move_hud(&app, next);
    tracing::info!(target: "ice_paw.screen_channel", from = current, to = next, "HUD 显示器切换");
    Ok(ch.snapshot())
}

/// HUD 窗形态切换（B7 写避让/手动收起；HUD 页按 writing/collapsed 态驱动）：
/// mini=收缩右上角微条，passthrough=点击穿透（仅写执行中的自动收缩）。
#[tauri::command]
pub async fn screen_hud_set_form(app: AppHandle, mini: bool, passthrough: bool) -> AppResult<()> {
    hud::set_form(&app, mini, passthrough);
    Ok(())
}
