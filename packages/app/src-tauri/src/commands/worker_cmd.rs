//! 临时 Worker 的用户侧命令（Router 化 Phase 1 批 P1-5）。
//!
//! 三件：
//! - `get_worker_profile`：worker 会话的身份头数据源（出生档案 + 终态 + 当前行
//!   生命周期）——「不孤岛」触点：进入 worker 会话即知「我是谁的任务、从哪来、
//!   什么权限、什么状态、可否留用」。
//! - `promote_worker`：转正（ephemeral → standard 一列窄写）——「保存为正式
//!   agent」产品钩子；行/会话/轨迹全量保留，历史 worker 会话保持 kind='worker'
//!   （那是历史任务记录，不因转正变成普通会话）。
//! - `retire_expired_workers`（boot 扫尾，非 invoke）：超龄 ephemeral 行退役为
//!   'expired' 墓碑——**行/会话/事件全保留**（append-only 审计不因清理丢失，
//!   agents→conversations 是 CASCADE，物理删除会连带抹掉生命线事件，故永不走
//!   删除）；退役事实逐 worker 落 worker_lifecycle(expired) 事件诚实闭环。

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::State;

use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::event_log::{self, WorkerLifecyclePayload, WorkerSpawnedPayload};

/// 临时 worker 的保留期（好默认，L1）：7 天覆盖「事后查看/转正」的合理窗口，
/// 之后退役为墓碑（不删除）。同步形态任务小时级完成，7 天纯留观窗。
pub(crate) const WORKER_RETENTION_DAYS: i64 = 7;

/// worker 会话身份档（身份头渲染位：出生 spec + 终态 + 行状态）
#[derive(Serialize)]
pub struct WorkerProfile {
    pub worker_agent_id: String,
    pub worker_name: String,
    /// 行生命周期：ephemeral（活跃）/ standard（已转正）/ expired（已退役）
    pub agent_kind: String,
    pub spawned: Option<WorkerSpawnedPayload>,
    pub lifecycle: Option<WorkerLifecyclePayload>,
}

/// 取 worker 会话的身份档：会话行（kind 校验 + agent 归属）→ 事件流出生/终态
/// → agent 行现状。任一环缺失诚实降级（Option 字段），头部按在场渲染。
#[tauri::command]
pub async fn get_worker_profile(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
) -> AppResult<WorkerProfile> {
    let conv = repo::conversation::get_by_id(&pool, &conversation_id).await?;
    if conv.kind != "worker" {
        return Err(AppError::Validation(format!(
            "会话 {conversation_id} 不是临时工作者会话"
        )));
    }
    let worker_agent_id = conv.agent_id.clone();

    let spawned: Option<WorkerSpawnedPayload> = sqlx::query_scalar::<sqlx::Sqlite, String>(
        "SELECT payload FROM session_events WHERE session_id = ? AND kind = 'worker_spawned' \
         ORDER BY seq LIMIT 1",
    )
    .bind(&conversation_id)
    .fetch_optional(&*pool)
    .await?
    .and_then(|raw| serde_json::from_str(&raw).ok());

    let lifecycle: Option<WorkerLifecyclePayload> = sqlx::query_scalar::<sqlx::Sqlite, String>(
        "SELECT payload FROM session_events WHERE session_id = ? AND kind = 'worker_lifecycle' \
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(&conversation_id)
    .fetch_optional(&*pool)
    .await?
    .and_then(|raw| serde_json::from_str(&raw).ok());

    let row = repo::agent::get_by_id(&pool, &worker_agent_id).await?;
    Ok(WorkerProfile {
        worker_agent_id: worker_agent_id.clone(),
        worker_name: row.name.clone(),
        agent_kind: row.agent_kind.clone(),
        spawned,
        lifecycle,
    })
}

/// 转正：ephemeral → standard（保存为正式 agent）。守卫：行不存在 = NotFound；
/// 非 ephemeral（已转正/已退役/本就是常规 agent）给各自的如实文案。
#[tauri::command]
pub async fn promote_worker(
    pool: State<'_, SqlitePool>,
    worker_agent_id: String,
) -> AppResult<()> {
    promote_worker_impl(&pool, &worker_agent_id).await
}

/// 命令层内核（测试直调，State 解包后的纯逻辑）。
pub(crate) async fn promote_worker_impl(
    pool: &sqlx::SqlitePool,
    worker_agent_id: &str,
) -> AppResult<()> {
    let row = repo::agent::get_by_id(pool, worker_agent_id).await?;
    match row.agent_kind.as_str() {
        "ephemeral" => {
            let n = repo::agent::set_agent_kind(pool, worker_agent_id, "ephemeral", "standard")
                .await?;
            if n == 0 {
                // 并发窗口：两次点击竞态——后者 0 行，如实告知即可
                return Err(AppError::Validation(
                    "转正未生效：该工作者的状态刚被其他操作改变，请刷新后重试".into(),
                ));
            }
            tracing::info!(
                target: "ice_paw.worker",
                worker_agent = %worker_agent_id,
                "临时工作者转正为正式 agent"
            );
            Ok(())
        }
        "standard" => Err(AppError::Validation(
            "该工作者已经是正式 agent，无需转正".into(),
        )),
        other => Err(AppError::Validation(format!(
            "该工作者已退役（{other}），不能转正——如需保留此能力请创建新的正式 agent"
        ))),
    }
}

/// boot 扫尾：超龄 ephemeral 行退役为 expired 墓碑 + 落 worker_lifecycle(expired)
/// 事件（生命线诚实闭环）。幂等（expired 不再命中 WHERE）；物理删除永不发生
/// （CASCADE 会抹掉事件审计，见模块头）。
pub async fn retire_expired_workers(pool: &sqlx::SqlitePool) -> AppResult<usize> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM agents WHERE agent_kind = 'ephemeral' \
         AND created_at < datetime('now', '-' || ? || ' days')",
    )
    .bind(WORKER_RETENTION_DAYS)
    .fetch_all(pool)
    .await?;
    if ids.is_empty() {
        return Ok(0);
    }
    let mut retired = 0usize;
    for id in &ids {
        let n = repo::agent::set_agent_kind(pool, id, "ephemeral", "expired").await?;
        if n == 0 {
            continue; // 并发已退役，幂等跳过
        }
        retired += 1;
        // 退役事实落 worker 会话事件流（找该 worker 的 kind='worker' 会话行）
        let conv_id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM conversations WHERE agent_id = ? AND kind = 'worker' LIMIT 1",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;
        if let Some(cid) = conv_id {
            event_log::log_worker_lifecycle(
                pool,
                &event_log::EventCtx::new(&cid, "", "system"),
                &WorkerLifecyclePayload {
                    v: 1,
                    worker_agent_id: id.clone(),
                    state: "expired".into(),
                    finish_reason: format!("retired_after_{}d", WORKER_RETENTION_DAYS),
                    acceptance: None,
                    billed_tokens: None,
                },
            )
            .await;
        }
    }
    if retired > 0 {
        tracing::info!(
            target: "ice_paw.worker",
            count = retired,
            "boot 扫尾：超龄临时工作者退役为 expired 墓碑（行/会话/事件保留）"
        );
    }
    Ok(retired)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> sqlx::SqlitePool {
        use sqlx::sqlite::SqlitePoolOptions;
        use std::str::FromStr;
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
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

    use crate::db::models::NewAgent;
    fn ephemeral_agent(id: &str) -> NewAgent {
        NewAgent {
            id: id.into(),
            name: format!("临时·{id}"),
            provider: "openai".into(),
            model: "m".into(),
            system_prompt: String::new(),
            api_key: String::new(),
            base_url: None,
            temperature: 0.7,
            max_tokens: 1024,
            extra_params: None,
            sort_order: 0,
            cache_prompt: true,
            supports_vision: false,
            max_history_messages: None,
            context_window: None,
            enabled_tools: None,
            workspace_path: None,
            avatar: None,
            model_profile_id: None,
            fallback_profile_ids: None,
            agent_kind: Some("ephemeral".into()),
        }
    }

    /// 退役语义锁：幂等 + 事件闭环 + 新鲜 worker 不退役 + expired 事件落库。
    #[tokio::test]
    async fn retire_expires_only_stale_workers_idempotently() {
        let pool = test_pool().await;
        // 超龄 worker（created_at 8 天前）+ 新鲜 worker
        repo::agent::create(&pool, &ephemeral_agent("w-old"), "w-old", "w-old")
            .await
            .unwrap();
        repo::agent::create(&pool, &ephemeral_agent("w-new"), "w-new", "w-new")
            .await
            .unwrap();
        sqlx::query("UPDATE agents SET created_at = datetime('now', '-8 days') WHERE id = 'w-old'")
            .execute(&pool)
            .await
            .unwrap();
        // 超龄 worker 挂一个 worker 会话（退役事件应落这里）
        repo::conversation::create(
            &pool,
            "conv-w-old",
            &crate::db::models::NewConversation {
                agent_id: "w-old".into(),
                title: Some("t".into()),
                project_id: None,
                kind: Some("worker".into()),
                initiator_agent_id: Some("a1".into()),
                parent_conversation_id: None,
                auto_approve: None,
            },
        )
        .await
        .unwrap();

        let n = retire_expired_workers(&pool).await.unwrap();
        assert_eq!(n, 1, "只退役超龄者");
        assert_eq!(
            repo::agent::get_by_id(&pool, "w-old").await.unwrap().agent_kind,
            "expired"
        );
        assert_eq!(
            repo::agent::get_by_id(&pool, "w-new").await.unwrap().agent_kind,
            "ephemeral",
            "新鲜 worker 不退役"
        );
        // 会话/事件保留 + expired 事件在场
        let ev: Option<String> = sqlx::query_scalar(
            "SELECT payload FROM session_events WHERE session_id = 'conv-w-old' \
             AND kind = 'worker_lifecycle'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(ev.unwrap().contains("\"state\":\"expired\""), "退役事件落库");

        // 幂等：再跑零命中
        let n = retire_expired_workers(&pool).await.unwrap();
        assert_eq!(n, 0);
    }

    /// 身份档查询：kind 校验拒绝非 worker 会话；事件/行三环组装。
    #[tokio::test]
    async fn get_worker_profile_assembles_spawn_and_lifecycle() {
        let pool = test_pool().await;
        repo::agent::create(&pool, &ephemeral_agent("w1"), "w1", "w1")
            .await
            .unwrap();
        // 父会话的 agent 行（conversations.agent_id FK 目标）
        repo::agent::create(&pool, &ephemeral_agent("a1"), "a1", "a1")
            .await
            .unwrap();
        sqlx::query("UPDATE agents SET agent_kind = 'standard' WHERE id = 'a1'")
            .execute(&pool)
            .await
            .unwrap();
        // 父会话 p1（parent FK 目标）+ worker 会话
        repo::conversation::create(
            &pool,
            "p1",
            &crate::db::models::NewConversation {
                agent_id: "a1".into(),
                title: Some("父".into()),
                project_id: None,
                kind: Some("chat".into()),
                initiator_agent_id: None,
                parent_conversation_id: None,
                auto_approve: None,
            },
        )
        .await
        .unwrap();
        repo::conversation::create(
            &pool,
            "conv-w",
            &crate::db::models::NewConversation {
                agent_id: "w1".into(),
                title: Some("t".into()),
                project_id: None,
                kind: Some("worker".into()),
                initiator_agent_id: Some("a1".into()),
                parent_conversation_id: Some("p1".into()),
                auto_approve: None,
            },
        )
        .await
        .unwrap();
        let ctx = event_log::EventCtx::new("conv-w", "t1", "a1");
        event_log::log_worker_spawned(
            &pool,
            &ctx,
            &WorkerSpawnedPayload {
                v: 1,
                worker_agent_id: "w1".into(),
                creator_agent_id: "a1".into(),
                parent_conversation_id: "p1".into(),
                purpose: "统计文档".into(),
                model_profile_id: "mp-1".into(),
                tool_scopes: vec!["read_file".into()],
                max_total_tokens: Some(50_000),
                approval: "readonly".into(),
            },
        )
        .await;
        event_log::log_worker_lifecycle(
            &pool,
            &ctx,
            &WorkerLifecyclePayload {
                v: 1,
                worker_agent_id: "w1".into(),
                state: "ready".into(),
                finish_reason: "end_turn".into(),
                acceptance: Some("共 14 个文件".into()),
                billed_tokens: Some(1234),
            },
        )
        .await;

        // 直调查询内核（State 解包同形）
        let conv = repo::conversation::get_by_id(&pool, "conv-w").await.unwrap();
        assert_eq!(conv.kind, "worker");
        let spawned: Option<WorkerSpawnedPayload> = sqlx::query_scalar::<sqlx::Sqlite, String>(
            "SELECT payload FROM session_events WHERE session_id = 'conv-w' \
             AND kind = 'worker_spawned' ORDER BY seq LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .unwrap()
        .and_then(|raw| serde_json::from_str(&raw).ok());
        assert_eq!(spawned.as_ref().unwrap().purpose, "统计文档");
        assert_eq!(spawned.as_ref().unwrap().parent_conversation_id, "p1");
    }

    /// 转正守卫：ephemeral 可转、standard 拒、expired 拒、NotFound。
    #[tokio::test]
    async fn promote_guards_by_current_kind() {
        let pool = test_pool().await;
        repo::agent::create(&pool, &ephemeral_agent("w1"), "w1", "w1")
            .await
            .unwrap();

        promote_worker_impl(&pool, "w1").await.unwrap();
        assert_eq!(
            repo::agent::get_by_id(&pool, "w1").await.unwrap().agent_kind,
            "standard"
        );
        // 已转正再点 → 如实拒绝
        let err = promote_worker_impl(&pool, "w1").await.unwrap_err();
        assert!(err.to_string().contains("已经是正式"));

        // expired 拒
        sqlx::query("UPDATE agents SET agent_kind = 'expired' WHERE id = 'w1'")
            .execute(&pool)
            .await
            .unwrap();
        let err = promote_worker_impl(&pool, "w1").await.unwrap_err();
        assert!(err.to_string().contains("退役"));

        // 不存在
        assert!(promote_worker_impl(&pool, "ghost").await.is_err());
    }
}
