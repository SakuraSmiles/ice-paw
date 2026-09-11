//! 频道 v1 命令层——前端侧栏懒建 / 频道视图 / 统筹者治理出口（design §C1/C2/C5）。
//!
//! 引擎行为（路由/选举/接力/护栏）在 `harness/channel.rs`；本文件只做：
//! - `ensure_channel`：侧栏入口懒建（幂等；创建**不**触发选举——选举按设计在
//!   首次广播时触发，避免打开频道就产生 LLM 调用与无人发起的投票发言）；
//! - `get_channel`：频道视图（频道行 + 成员档案 + 统筹者出身）；
//! - `set_channel_coordinator`：用户治理（任命 / 罢免——C5 指定档入口）；
//! - `reelect_channel_coordinator`：一键让系统补选（指定档故障降级的出口）。

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, State};

use crate::db::models::Conversation;
use crate::db::repo::{self};
use crate::error::{AppError, AppResult};
use crate::harness::channel;
use crate::harness::event_log::{self, ChannelCoordinatorPayload, EventCtx};

/// 频道成员档案（前端成员列表 / @ 弹层候选共用）。
#[derive(Debug, Serialize)]
pub struct ChannelMemberInfo {
    pub agent_id: String,
    pub name: String,
    /// 'coordinator' | 'lead' | 'member'
    pub role: String,
}

/// 频道视图：频道行（可能已归档）+ 成员 + 统筹者治理位。
#[derive(Debug, Serialize)]
pub struct ChannelView {
    pub channel: Option<Conversation>,
    pub members: Vec<ChannelMemberInfo>,
    pub coordinator_agent_id: Option<String>,
    /// 统筹者是否用户手动指定（C5 两档：指定档故障不自动换帅，频道头部据此
    /// 展示「让系统补选」入口）
    pub coordinator_appointed: bool,
}

async fn build_view(pool: &SqlitePool, conv: Option<Conversation>) -> AppResult<ChannelView> {
    let Some(conv) = conv else {
        return Ok(ChannelView {
            channel: None,
            members: Vec::new(),
            coordinator_agent_id: None,
            coordinator_appointed: false,
        });
    };
    let Some(pid) = conv.project_id.clone() else {
        return Err(AppError::Validation(
            "频道未挂载项目（数据异常）——请反馈问题".into(),
        ));
    };
    let profiles = repo::project::list_member_profiles(pool, &pid).await?;
    let coordinator_agent_id = profiles
        .iter()
        .find(|m| m.role == "coordinator")
        .map(|m| m.agent_id.clone());
    let coordinator_appointed = match coordinator_agent_id.as_deref() {
        Some(cid) => channel::coordinator_is_user_appointed(pool, &conv.id, cid).await,
        None => false,
    };
    Ok(ChannelView {
        members: profiles
            .into_iter()
            .map(|m| ChannelMemberInfo {
                agent_id: m.agent_id,
                name: m.name,
                role: m.role,
            })
            .collect(),
        channel: Some(conv),
        coordinator_agent_id,
        coordinator_appointed,
    })
}

/// 幂等确保项目频道存在（侧栏入口懒建；项目创建时不预建——老项目零迁移、
/// 空项目不产生空会话）。统筹投影 = 现任 coordinator，无则 joined_at 最早成员
///（临时投影不阻塞使用；正式统筹由首次广播触发的自选举产生）。
#[tauri::command]
pub async fn ensure_channel(
    pool: State<'_, SqlitePool>,
    project_id: String,
) -> AppResult<ChannelView> {
    let pool = pool.inner();
    let proj = repo::project::get_by_id(pool, &project_id).await?;
    let members = repo::project::list_member_profiles(pool, &project_id).await?;
    if members.is_empty() {
        return Err(AppError::Validation(format!(
            "项目「{}」还没有成员，无法开启频道——先在项目设置里添加成员",
            proj.name
        )));
    }
    let projection = members
        .iter()
        .find(|m| m.role == "coordinator")
        .or_else(|| members.first())
        .map(|m| m.agent_id.clone())
        .unwrap_or_default();
    // 标题 = 项目名本体（2026-09-11 拍板：频道身份改由头部 tag 徽章呈现，
    // 不再拼进标题字符串）；存量带「 · 频道」后缀的行由 migration 55 剥除。
    let title = proj.name.clone();
    let conv = repo::conversation::ensure_channel(pool, &project_id, &title, &projection).await?;
    build_view(pool, Some(conv.into())).await
}

/// 频道视图（含归档频道——前端归档入口只读展示用）。
#[tauri::command]
pub async fn get_channel(pool: State<'_, SqlitePool>, project_id: String) -> AppResult<ChannelView> {
    let pool = pool.inner();
    let conv = repo::conversation::active_channel_for_project(pool, &project_id).await?;
    match conv {
        Some(c) => build_view(pool, Some(c.into())).await,
        // 活频道不存在 → 查归档频道（归档入口展示）；两者皆无 = 未开启
        None => {
            let archived = repo::conversation::archived_channel_for_project(pool, &project_id).await?;
            build_view(pool, archived.map(Into::into)).await
        }
    }
}

/// 用户治理：任命（Some）/ 罢免（None）统筹者——C5 指定档的唯一入口。
///
/// 任命：目标须是项目成员；旧统筹降回 member、目标升 coordinator、投影更新、
/// streak 清零、appointed 事件。罢免：现任降 member、投影回落 joined_at 最早
/// 成员、removed 事件——统筹空缺，下次广播触发自选举。
#[tauri::command]
pub async fn set_channel_coordinator(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
    agent_id: Option<String>,
) -> AppResult<()> {
    let pool = pool.inner();
    let conv = repo::conversation::get_by_id(pool, &conversation_id).await?;
    if conv.kind != "channel" {
        return Err(AppError::Validation(
            "目标会话不是频道——统筹者治理只作用于频道".into(),
        ));
    }
    let Some(pid) = conv.project_id.clone() else {
        return Err(AppError::Validation(
            "频道未挂载项目（数据异常）——请反馈问题".into(),
        ));
    };
    let members = repo::project::list_member_profiles(pool, &pid).await?;
    let current = members.iter().find(|m| m.role == "coordinator");

    match agent_id {
        Some(target) => {
            let Some(m) = members.iter().find(|m| m.agent_id == target) else {
                return Err(AppError::Validation(
                    "目标 agent 不是该项目成员——先在项目设置里添加为成员".into(),
                ));
            };
            if let Some(cur) = current.filter(|c| c.agent_id != target) {
                repo::project::set_member_role(pool, &pid, &cur.agent_id, "member").await?;
            }
            if current.map(|c| c.agent_id.as_str()) != Some(target.as_str()) {
                repo::project::set_member_role(pool, &pid, &target, "coordinator").await?;
                repo::conversation::set_conversation_agent(pool, &conv.id, &target).await?;
                event_log::log_channel_coordinator(
                    pool,
                    &EventCtx::new(&conv.id, "", &target),
                    &ChannelCoordinatorPayload {
                        v: 1,
                        action: "appointed".into(),
                        agent_id: Some(m.agent_id.clone()),
                        reason: Some("用户手动指定".into()),
                    },
                )
                .await;
            }
            Ok(())
        }
        None => {
            let Some(cur) = current else {
                return Ok(()); // 本就空缺：幂等成功
            };
            repo::project::set_member_role(pool, &pid, &cur.agent_id, "member").await?;
            // 投影回落 joined_at 最早成员（ensure 同款语义；零成员由 FK 不可能）
            if let Some(earliest) = members.iter().find(|m| m.agent_id != cur.agent_id) {
                let _ = repo::conversation::set_conversation_agent(pool, &conv.id, &earliest.agent_id)
                    .await;
            }
            event_log::log_channel_coordinator(
                pool,
                &EventCtx::new(&conv.id, "", &cur.agent_id),
                &ChannelCoordinatorPayload {
                    v: 1,
                    action: "removed".into(),
                    agent_id: Some(cur.agent_id.clone()),
                    reason: Some("用户手动罢免（下次广播触发自选举）".into()),
                },
            )
            .await;
            Ok(())
        }
    }
}

/// 一键让系统补选（频道头部入口；指定档故障降级的推荐出口）——绕过全员
/// 弃权熔断（用户治理权最大），election_running 守卫照常防双跑。
#[tauri::command]
pub async fn reelect_channel_coordinator(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    conversation_id: String,
) -> AppResult<()> {
    let pool = pool.inner();
    let conv = repo::conversation::get_by_id(pool, &conversation_id).await?;
    if conv.kind != "channel" {
        return Err(AppError::Validation(
            "目标会话不是频道——补选只作用于频道".into(),
        ));
    }
    if conv.archived_at.is_some() {
        return Err(AppError::Validation(
            "频道已归档——记录保留为只读，无法补选统筹者".into(),
        ));
    }
    channel::manual_reelect(&app, pool, &conv).await;
    Ok(())
}
