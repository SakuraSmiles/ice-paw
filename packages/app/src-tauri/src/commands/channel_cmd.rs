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

use crate::db::models::{Conversation, ConversationRow};
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

/// 幂等确保项目频道存在的核心（懒建命令入口与「成员就位即自动建」共用）。
/// Ok(None) = 项目还没有成员——语义由调用方定：手动入口报人话、自动路径跳过
/// （空项目不产生空频道会话是 C1 的原始设计意图，自动建不翻案）。统筹投影 =
/// 现任 coordinator，无则 joined_at 最早成员（临时投影不阻塞使用；正式统筹由
/// 首次广播触发的自选举产生）。标题 = 项目名本体（2026-09-11 拍板：频道身份
/// 由头部 tag 徽章呈现；存量「 · 频道」后缀已由 migration 55 剥除）。
pub(crate) async fn ensure_channel_row(
    pool: &SqlitePool,
    project_id: &str,
) -> AppResult<Option<ConversationRow>> {
    let proj = repo::project::get_by_id(pool, project_id).await?;
    let members = repo::project::list_member_profiles(pool, project_id).await?;
    if members.is_empty() {
        return Ok(None);
    }
    let projection = members
        .iter()
        .find(|m| m.role == "coordinator")
        .or_else(|| members.first())
        .map(|m| m.agent_id.clone())
        .unwrap_or_default();
    let conv = repo::conversation::ensure_channel(pool, project_id, &proj.name, &projection).await?;
    Ok(Some(conv))
}

/// 成员就位即自动建频道（2026-09-16 拍板，取代侧栏纯懒建）：添加成员/全量
/// 替换成员/带初始成员建项目等成员写路径后的 best-effort ensure——幂等（已
/// 存在零动作）、空成员项目跳过、失败仅 warn 不阻塞成员写入（侧栏「开启
/// 频道」入口保留为兜底）。boot 存量补建同源。
pub(crate) async fn ensure_channel_auto(pool: &SqlitePool, project_id: &str) {
    if let Err(e) = ensure_channel_row(pool, project_id).await {
        tracing::warn!(
            target: "ice_paw.channel",
            "成员就位自动建频道失败（不阻塞成员写入，侧栏入口可手动开启）: {e}"
        );
    }
}

/// 幂等确保项目频道存在（侧栏入口懒建兜底；新项目成员写路径已自动建——
/// ensure_channel_auto，此命令主要服务存量与异常路径）。
#[tauri::command]
pub async fn ensure_channel(
    pool: State<'_, SqlitePool>,
    project_id: String,
) -> AppResult<ChannelView> {
    let pool = pool.inner();
    match ensure_channel_row(pool, &project_id).await? {
        Some(conv) => build_view(pool, Some(conv.into())).await,
        None => {
            let proj = repo::project::get_by_id(pool, &project_id).await?;
            Err(AppError::Validation(format!(
                "项目「{}」还没有成员，无法开启频道——先在项目设置里添加成员",
                proj.name
            )))
        }
    }
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
            // 投影回落 joined_at 最早成员（ensure 同款语义；零成员由 FK 不可能）。
            // 与上方任命分支同款 `?` 传播：失败静默吞掉会留下「频道路由仍指向
            // 已罢免统筹者」的语义错位且无任何日志（U0-8）。
            if let Some(earliest) = members.iter().find(|m| m.agent_id != cur.agent_id) {
                repo::conversation::set_conversation_agent(pool, &conv.id, &earliest.agent_id)
                    .await?;
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

// =========================================================================
// 单元测试（in-memory SQLite）——成员就位即自动建（2026-09-16）的幂等 /
// 空成员跳过 / 统筹投影锁
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{NewAgent, NewProject};
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        pool
    }

    async fn seed_project(pool: &SqlitePool, id: &str) {
        repo::project::create(
            pool,
            &NewProject {
                name: format!("项目-{id}"),
                description: None,
                icon: None,
                workspace_path: None,
                theme_color: None,
                avatar: None,
                agent_ids: vec![],
            },
            id,
        )
        .await
        .expect("seed project");
    }

    async fn seed_agent(pool: &SqlitePool, id: &str) {
        repo::agent::create(
            pool,
            &NewAgent {
                id: id.into(),
                name: format!("成员-{id}"),
                provider: "mock".into(),
                model: "mock-model".into(),
                system_prompt: String::new(),
                api_key: String::new(),
                base_url: None,
                temperature: 0.7,
                max_tokens: 1024,
                extra_params: None,
                sort_order: 0,
                cache_prompt: true,
                max_history_messages: None,
                context_window: None,
                enabled_tools: None,
                supports_vision: false,
                workspace_path: None,
                avatar: None,
                model_profile_id: None,
                fallback_profile_ids: None,
            },
            id,
            "test-slot",
        )
        .await
        .expect("seed agent");
    }

    #[tokio::test]
    async fn ensure_row_skips_memberless_project() {
        let pool = test_pool().await;
        seed_project(&pool, "p1").await;
        // 空项目：None + 不产生频道行（空项目不产生空频道会话）
        let out = ensure_channel_row(&pool, "p1").await.unwrap();
        assert!(out.is_none());
        assert!(
            repo::conversation::active_channel_for_project(&pool, "p1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn ensure_row_creates_then_idempotent_with_projection() {
        let pool = test_pool().await;
        seed_project(&pool, "p1").await;
        seed_agent(&pool, "a1").await;
        seed_agent(&pool, "a2").await;
        repo::project::add_agent(&pool, "p1", "a1", "member")
            .await
            .unwrap();
        repo::project::add_agent(&pool, "p1", "a2", "coordinator")
            .await
            .unwrap();

        // 首次：建行——标题=项目名本体；投影=现任 coordinator（a2）优先
        let conv = ensure_channel_row(&pool, "p1").await.unwrap().expect("channel row");
        assert_eq!(conv.kind, "channel");
        assert_eq!(conv.title, "项目-p1");
        assert_eq!(conv.agent_id, "a2");
        assert_eq!(conv.project_id.as_deref(), Some("p1"));

        // 幂等：二次调用返回既有行（同 id，不重复建）
        let again = ensure_channel_row(&pool, "p1").await.unwrap().expect("channel row");
        assert_eq!(again.id, conv.id);

        // 自动路径 wrapper：已有频道时零动作、不报错（成员写路径每次都调）
        ensure_channel_auto(&pool, "p1").await;
        let third = ensure_channel_row(&pool, "p1").await.unwrap().expect("channel row");
        assert_eq!(third.id, conv.id);
    }

    #[tokio::test]
    async fn ensure_auto_creates_on_first_member() {
        let pool = test_pool().await;
        seed_project(&pool, "p1").await;
        seed_agent(&pool, "a1").await;
        // 首个成员落位 → 自动建（投影回落唯一成员=joined_at 最早）
        ensure_channel_auto(&pool, "p1").await;
        assert!(
            repo::conversation::active_channel_for_project(&pool, "p1")
                .await
                .unwrap()
                .is_none(),
            "无成员时 auto 不建"
        );
        repo::project::add_agent(&pool, "p1", "a1", "member")
            .await
            .unwrap();
        ensure_channel_auto(&pool, "p1").await;
        let conv = repo::conversation::active_channel_for_project(&pool, "p1")
            .await
            .unwrap()
            .expect("成员就位后频道已建");
        assert_eq!(conv.agent_id, "a1");
    }
}
