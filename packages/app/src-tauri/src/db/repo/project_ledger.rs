//! 项目级派生查询（MA-2 任务台账 / 项目轨迹 / 概览统计）。
//!
//! 与 [`super::session_event`]（单会话、以 session_id 为轴、按 seq 排序）互补：
//! 本模块全部以 `project_id` 为轴 JOIN conversations，是**纯只读派生**——
//! 不写入任何事件、不建表（任务 ≡ kind='delegation' 会话的派生视图）。
//!
//! 跨会话排序键用 `session_events.id`（全局 AUTOINCREMENT，永不复用）——
//! migration 44 头注释明确预留的「跨 session 按全局 id 排序的项目级轨迹序」。
//! per-conv 的 seq 在跨会话语境下不可比，只用于单会话内部（如台账取最后一条
//! turn_ended）。

use sqlx::FromRow;
use sqlx::SqlitePool;

use crate::error::AppResult;

/// 任务台账行：delegation 会话 + 其最后一条 `turn_ended` 的投影。
///
/// `ended_payload` 是 turn_ended 的**原始 JSON 字符串**（不在 SQL 侧
/// json_extract——要取 termination+rounds 两个值且需类型安全的损坏降级，
/// 由命令层 `serde_json::from_str::<TurnEndedPayload>` 解析，失败 warn + None
/// 不吞会话行）。取「最后一条」：delegation 子会话可被用户续聊产生多 turn，
/// 台账关心最新终态。
#[derive(Debug, Clone, FromRow)]
pub struct ProjectTaskRow {
    pub id: String,
    pub title: String,
    /// 执行者（= conversations.agent_id，被委派的专家 agent）
    pub agent_id: String,
    /// 发起者（migration 45 列；NULL ≡ 用户发起）
    pub initiator_agent_id: Option<String>,
    /// 委派图边——父会话（跳转回父会话用，无 FK 语义）
    pub parent_conversation_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// 最后一条 turn_ended 的 payload 原文（无 turn_ended = 进行中/中断，None）
    pub ended_payload: Option<String>,
    /// 最后一条 turn_ended 的落库时间
    pub ended_at: Option<String>,
}

/// 任务台账：项目内全部 delegation 会话 + 最后一条 turn_ended。
///
/// 执行计划：`idx_conversations_kind_project(kind, project_id)` 圈行，关联
/// 子查询的 MAX(seq) 走 `idx_session_events_session_seq`，无 N+1 全表扫。
pub async fn list_project_tasks(
    pool: &SqlitePool,
    project_id: &str,
) -> AppResult<Vec<ProjectTaskRow>> {
    let rows = sqlx::query_as::<_, ProjectTaskRow>(
        "SELECT c.id, c.title, c.agent_id, c.initiator_agent_id, c.parent_conversation_id,
                c.created_at, c.updated_at,
                te.payload AS ended_payload, te.created_at AS ended_at
           FROM conversations c
           LEFT JOIN session_events te
                  ON te.session_id = c.id AND te.kind = 'turn_ended'
                 AND te.seq = (SELECT MAX(e2.seq) FROM session_events e2
                                WHERE e2.session_id = c.id AND e2.kind = 'turn_ended')
          WHERE c.kind = 'delegation' AND c.project_id = ?
          ORDER BY c.updated_at DESC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 项目事件流行：`SessionEventRow` 全字段 + 会话标注列（前端会话徽章）。
#[derive(Debug, Clone, FromRow)]
pub struct ProjectEventRow {
    pub id: i64,
    pub session_id: String,
    pub seq: i64,
    pub kind: String,
    pub actor: String,
    pub turn_id: Option<String>,
    pub message_id: Option<String>,
    pub payload: String,
    pub created_at: String,
    /// 事件所属会话标题（徽章显示）
    pub session_title: String,
    /// 会话类型 chat/delegation（徽章分色）
    pub session_kind: String,
}

const PROJECT_EVENT_COLS: &str = "e.id, e.session_id, e.seq, e.kind, e.actor, e.turn_id, \
                                  e.message_id, e.payload, e.created_at, \
                                  c.title AS session_title, c.kind AS session_kind";

/// 项目轨迹尾部优先分页（游标 = 全局 id；`after` 增量见下方 [`list_project_events_after`]）。
///
/// 取 id 严格小于 `before_id`（`None` = 从最新开始）的最大 `limit` 条，返回前
/// 反转为全局 id 正序。「加载更早」= 用当前已载最小 id 作下一页 `before_id`。
///
/// 先子查询物化一页 id 再回表取 payload：单层 JOIN 版优化器以 conversations 为
/// 外圈、对**全部**项目事件（含 payload 行）做 TEMP B-TREE 排序后才 LIMIT——
/// 生产库 2598 事件实测 33ms，随项目事件量线性劣化；子查询版只对 id 排序、
/// payload 由 rowid 回表逐行取，同库实测 12ms，逐行结果等价（2026-08-31 验证）。
/// [`list_project_events_after`] 不改：`after_id` 锚定下同款计划实测 0.5ms
///（空页 0.1ms），无排序压力。
pub async fn list_project_events_tail(
    pool: &SqlitePool,
    project_id: &str,
    before_id: Option<i64>,
    limit: i64,
) -> AppResult<Vec<ProjectEventRow>> {
    let mut rows = sqlx::query_as::<_, ProjectEventRow>(&format!(
        "SELECT {PROJECT_EVENT_COLS}
           FROM (SELECT e2.id AS page_id
                   FROM session_events e2 JOIN conversations c2 ON c2.id = e2.session_id
                  WHERE c2.project_id = ? AND e2.id < COALESCE(?, 9223372036854775807)
                  ORDER BY e2.id DESC
                  LIMIT ?) page
           JOIN session_events e ON e.id = page.page_id
           JOIN conversations c ON c.id = e.session_id
          ORDER BY page.page_id DESC"
    ))
    .bind(project_id)
    .bind(before_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.reverse();
    Ok(rows)
}

/// 项目轨迹正向增量（与 [`super::session_event::list_after`] 同构）：
/// id 严格大于 `after_id` 的最早 `limit` 条。返回空 = 已追平。
pub async fn list_project_events_after(
    pool: &SqlitePool,
    project_id: &str,
    after_id: Option<i64>,
    limit: i64,
) -> AppResult<Vec<ProjectEventRow>> {
    let rows = sqlx::query_as::<_, ProjectEventRow>(&format!(
        "SELECT {PROJECT_EVENT_COLS}
           FROM session_events e JOIN conversations c ON c.id = e.session_id
          WHERE c.project_id = ? AND e.id > COALESCE(?, 0)
          ORDER BY e.id ASC
          LIMIT ?"
    ))
    .bind(project_id)
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 概览统计行（任务分桶不在 SQL——命令层复用 [`list_project_tasks`] 结果分桶）。
#[derive(Debug, Clone, FromRow)]
pub struct ProjectOverviewRow {
    pub chat_conversations: i64,
    pub delegation_conversations: i64,
    pub messages: i64,
    /// 项目内会话的最近 updated_at（message 落库时 touch，见 repo::message）
    pub last_activity_at: Option<String>,
}

/// 成员消息占比行（概览「成员负载」环图 + 横条排行的数据源）。
///
/// 口径 = messages 行数（工作量 proxy，覆盖 chat + delegation 全部会话）；
/// tokens = SUM(token_count)——**本地估算值**（列可空「流式完成后回填」，
/// SUM 忽略 NULL，展示层标 ≈ 诚实）。agent 名/模型不在 SQL 解析（前端
/// agent store getById——migration 45 明确不设 FK 到 agents，名字解析是
/// 展示层职责）。
#[derive(Debug, Clone, FromRow)]
pub struct ProjectAgentShareRow {
    pub agent_id: String,
    pub messages: i64,
    pub tokens: i64,
}

/// 成员消息占比：本项目会话按 agent 分组计数（消息数 + token 估算），
/// 按消息数降序（token 含 NULL 缺失不宜做序）。
pub async fn list_project_agent_shares(
    pool: &SqlitePool,
    project_id: &str,
) -> AppResult<Vec<ProjectAgentShareRow>> {
    let rows = sqlx::query_as::<_, ProjectAgentShareRow>(
        "SELECT c.agent_id, COUNT(*) AS messages, COALESCE(SUM(m.token_count), 0) AS tokens
           FROM messages m JOIN conversations c ON c.id = m.conversation_id
          WHERE c.project_id = ?
          GROUP BY c.agent_id
          ORDER BY messages DESC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 项目概览统计：会话分 kind 计数 + 消息数 + 最近活跃。
///
/// 多条小 SQL 各走现成索引（kind 索引 / idx_messages_conversation），单职责
/// 可测；messages COUNT 不取 content 大字段。
pub async fn get_project_overview(pool: &SqlitePool, project_id: &str) -> AppResult<ProjectOverviewRow> {
    let (chat_n, delegation_n): (i64, i64) = sqlx::query_as(
        "SELECT
            COALESCE(SUM(CASE WHEN kind = 'chat' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN kind = 'delegation' THEN 1 ELSE 0 END), 0)
           FROM conversations WHERE project_id = ?",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?;

    let (messages,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM messages m JOIN conversations c ON c.id = m.conversation_id
          WHERE c.project_id = ?",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?;

    let last: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT MAX(updated_at) FROM conversations WHERE project_id = ?",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?;

    Ok(ProjectOverviewRow {
        chat_conversations: chat_n,
        delegation_conversations: delegation_n,
        messages,
        last_activity_at: last.and_then(|(v,)| v),
    })
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::session_event;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    /// in-memory SQLite 每连接各一个库，pool 必须 max_connections(1)。
    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite")
    }

    async fn migrated_pool() -> SqlitePool {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed_agent(&pool).await;
        // conversations.project_id 有 FK 到 projects——测试项目行先落
        for pid in ["p1", "p2"] {
            sqlx::query("INSERT INTO projects (id, name) VALUES (?, ?)")
                .bind(pid)
                .bind(format!("project {pid}"))
                .execute(&pool)
                .await
                .expect("seed project");
        }
        pool
    }

    async fn seed_agent(pool: &SqlitePool) {
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("agent-1")
        .bind("test-agent")
        .bind("anthropic")
        .bind("claude-test")
        .bind("")
        .bind("")
        .bind(0.7)
        .bind(1024)
        .bind("{}")
        .bind(0)
        .bind(0)
        .execute(pool)
        .await
        .expect("seed agent");
    }

    /// 全形态会话 seed：kind / project_id / 发起者 / 父会话边。
    /// updated_at 显式传入——`trg_conversations_upd` 触发器会把任何 UPDATE 后的
    /// updated_at 重置为 datetime('now')，排序测试只能在 INSERT 时写死分层值。
    async fn seed_conv(
        pool: &SqlitePool,
        id: &str,
        kind: &str,
        project_id: Option<&str>,
        initiator_agent_id: Option<&str>,
        parent: Option<&str>,
        updated_at: &str,
    ) {
        sqlx::query(
            "INSERT INTO conversations (id, agent_id, title, kind, project_id, initiator_agent_id, parent_conversation_id, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind("agent-1")
        .bind(format!("conv {id}"))
        .bind(kind)
        .bind(project_id)
        .bind(initiator_agent_id)
        .bind(parent)
        .bind(updated_at)
        .execute(pool)
        .await
        .expect("seed conversation");
    }

    async fn seed_message(pool: &SqlitePool, id: &str, conv_id: &str) {
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, role, content, content_blocks, token_count)
             VALUES (?, ?, 'user', 'hi', '[]', 5)",
        )
        .bind(id)
        .bind(conv_id)
        .execute(pool)
        .await
        .expect("seed message");
    }

    fn ended_payload(termination: &str, rounds: u32) -> String {
        format!(r#"{{"v":1,"termination":"{termination}","rounds":{rounds},"usage":null,"user_token_count":null}}"#)
    }

    #[tokio::test]
    async fn list_project_tasks_joins_last_turn_ended() {
        let pool = migrated_pool().await;

        // 父会话（chat）+ 两个子任务（delegation）：一个 done、一个进行中
        seed_conv(&pool, "parent", "chat", Some("p1"), None, None, "2026-08-18 10:00:00").await;
        seed_conv(&pool, "task-done", "delegation", Some("p1"), Some("agent-1"), Some("parent"), "2026-08-18 12:00:00").await;
        seed_conv(&pool, "task-running", "delegation", Some("p1"), Some("agent-1"), Some("parent"), "2026-08-18 13:00:00").await;
        // 混入者：他项目任务 / 本项目 chat / 散落 delegation —— 都不该出现
        seed_conv(&pool, "other-proj", "delegation", Some("p2"), None, None, "2026-08-18 10:00:00").await;
        seed_conv(&pool, "chat-conv", "chat", Some("p1"), None, None, "2026-08-18 10:00:00").await;
        seed_conv(&pool, "loose-task", "delegation", None, None, None, "2026-08-18 10:00:00").await;

        // task-done：两轮 turn_ended（续聊场景）→ 取 seq 更大的最后一条
        session_event::append(&pool, "task-done", "turn_context", "user", Some("t1"), None, "{}")
            .await
            .unwrap();
        session_event::append(
            &pool,
            "task-done",
            "turn_ended",
            "agent:agent-1",
            Some("t1"),
            None,
            &ended_payload("abort", 1),
        )
        .await
        .unwrap();
        session_event::append(&pool, "task-done", "turn_context", "user", Some("t2"), None, "{}")
            .await
            .unwrap();
        session_event::append(
            &pool,
            "task-done",
            "turn_ended",
            "agent:agent-1",
            Some("t2"),
            None,
            &ended_payload("stop", 3),
        )
        .await
        .unwrap();
        // task-running：有事件但无 turn_ended
        session_event::append(&pool, "task-running", "turn_context", "user", Some("t1"), None, "{}")
            .await
            .unwrap();
        // 混入者也给事件（证明排除靠 WHERE 不靠缺事件）
        session_event::append(
            &pool,
            "other-proj",
            "turn_ended",
            "agent:agent-1",
            Some("t1"),
            None,
            &ended_payload("stop", 1),
        )
        .await
        .unwrap();

        // updated_at 分层已在 seed_conv INSERT 时写死（UPDATE 会触发重置，见 helper 注释）

        let tasks = list_project_tasks(&pool, "p1").await.unwrap();
        assert_eq!(tasks.len(), 2, "只含本项目 delegation 会话");
        assert_eq!(tasks[0].id, "task-running", "updated_at 倒序");
        assert!(tasks[0].ended_payload.is_none() && tasks[0].ended_at.is_none(), "进行中双 None");
        assert_eq!(tasks[0].initiator_agent_id.as_deref(), Some("agent-1"));
        assert_eq!(tasks[0].parent_conversation_id.as_deref(), Some("parent"));

        assert_eq!(tasks[1].id, "task-done");
        let payload = tasks[1].ended_payload.as_deref().expect("done 带 payload");
        assert!(payload.contains(r#""termination":"stop""#), "多 turn 取最后一条");
        assert!(tasks[1].ended_at.is_some(), "ended_at 有值");
    }

    #[tokio::test]
    async fn list_project_events_tail_and_after_use_global_id_order() {
        let pool = migrated_pool().await;

        seed_conv(&pool, "a", "chat", Some("p1"), None, None, "2026-08-18 10:00:00").await;
        seed_conv(&pool, "b", "delegation", Some("p1"), Some("agent-1"), Some("a"), "2026-08-18 10:00:00").await;
        seed_conv(&pool, "c", "chat", Some("p2"), None, None, "2026-08-18 10:00:00").await; // 他项目

        // 跨会话交错追加：a, b, c, a, b（全局 id 单调即插入序）
        for (conv, marker) in [
            ("a", "m1"),
            ("b", "m2"),
            ("c", "m3"),
            ("a", "m4"),
            ("b", "m5"),
        ] {
            session_event::append(&pool, conv, "user_message", "user", Some("t"), Some(marker), "{}")
                .await
                .unwrap();
        }

        // 尾部优先：最新 3 条（m3/m4/m5 中属于 p1 的 m4/m5 + 再往前 m2）→ 反转为 id ASC
        let tail = list_project_events_tail(&pool, "p1", None, 3).await.unwrap();
        let markers: Vec<&str> = tail
            .iter()
            .map(|r| r.message_id.as_deref().unwrap_or_default())
            .collect();
        assert_eq!(markers, vec!["m2", "m4", "m5"], "p1 全量仅 3 条且全局 id 正序");
        assert!(
            tail.windows(2).all(|w| w[0].id < w[1].id),
            "返回前已反转为 id ASC"
        );

        // 会话标注列
        assert_eq!(tail[0].session_title, "conv b");
        assert_eq!(tail[0].session_kind, "delegation");
        assert_eq!(tail[1].session_kind, "chat");

        // before_id 严格小于边界：p1 事件序是 m1,m2,m4,m5，尾页丢了 m1 → 游标 m2 前恰剩 m1
        let m2_id = tail[0].id;
        let earlier = list_project_events_tail(&pool, "p1", Some(m2_id), 3).await.unwrap();
        assert_eq!(
            earlier
                .iter()
                .map(|r| r.message_id.as_deref().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["m1"],
            "游标严格小于边界，取回页外更早事件"
        );
        let m1_id = earlier[0].id;
        let head = list_project_events_tail(&pool, "p1", Some(m1_id), 3).await.unwrap();
        assert!(head.is_empty(), "取穿后返回空");

        // 正向增量：after = m4 的 id → 只拿 m5
        let m4_id = tail[1].id;
        let inc = list_project_events_after(&pool, "p1", Some(m4_id), 10).await.unwrap();
        assert_eq!(inc.len(), 1);
        assert_eq!(inc[0].message_id.as_deref(), Some("m5"));

        // 追平返回空
        let done = list_project_events_after(&pool, "p1", Some(tail[2].id), 10).await.unwrap();
        assert!(done.is_empty());

        // 他项目隔离
        let p2 = list_project_events_tail(&pool, "p2", None, 10).await.unwrap();
        assert_eq!(p2.len(), 1);
        assert_eq!(p2[0].message_id.as_deref(), Some("m3"));
    }

    /// seed_message 的 token 可空版本（「估算未回填」行，SUM 须忽略 NULL）
    async fn seed_message_tokens(
        pool: &SqlitePool,
        id: &str,
        conv_id: &str,
        token_count: Option<i32>,
    ) {
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, role, content, content_blocks, token_count)
             VALUES (?, ?, 'user', 'hi', '[]', ?)",
        )
        .bind(id)
        .bind(conv_id)
        .bind(token_count)
        .execute(pool)
        .await
        .expect("seed message with tokens");
    }

    #[tokio::test]
    async fn list_project_agent_shares_groups_and_isolates() {
        let pool = migrated_pool().await;
        // 第二个 agent（conversations.agent_id 有 FK 到 agents）
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES ('agent-2', 'agent-two', 'openai', 'gpt-test', '', '', 0.7, 1024, '{}', 1, 0)",
        )
        .execute(&pool)
        .await
        .expect("seed agent-2");

        // agent-1 名下 2 条（token 100 + NULL 未回填）；t1 划给 agent-2 名下 1 条（token 40）
        seed_conv(&pool, "c1", "chat", Some("p1"), None, None, "2026-08-18 10:00:00").await;
        seed_conv(&pool, "t1", "delegation", Some("p1"), Some("agent-1"), Some("c1"), "2026-08-18 10:00:00").await;
        // 注意：seed_conv 的 agent_id 固定 agent-1——第二条会话用 agent-2 需 UPDATE
        sqlx::query("UPDATE conversations SET agent_id = 'agent-2' WHERE id = 't1'")
            .execute(&pool)
            .await
            .unwrap();
        seed_message_tokens(&pool, "m1", "c1", Some(100)).await;
        seed_message_tokens(&pool, "m2", "c1", None).await;
        seed_message_tokens(&pool, "m3", "t1", Some(40)).await;
        // 他项目混入（不进任何桶）
        seed_conv(&pool, "x", "chat", Some("p2"), None, None, "2026-08-18 10:00:00").await;
        seed_message(&pool, "m4", "x").await;

        let shares = list_project_agent_shares(&pool, "p1").await.unwrap();
        assert_eq!(shares.len(), 2, "按 agent 分组");
        assert_eq!(shares[0].agent_id, "agent-1");
        assert_eq!(shares[0].messages, 2, "降序：多者在前");
        assert_eq!(shares[0].tokens, 100, "SUM 忽略 NULL 未回填行");
        assert_eq!(shares[1].agent_id, "agent-2");
        assert_eq!(shares[1].messages, 1);
        assert_eq!(shares[1].tokens, 40);

        // 空项目空数组（成员招了没说话也不出现——口径是消息数不是成员数）
        let empty = list_project_agent_shares(&pool, "p-none").await.unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn get_project_overview_counts_and_empty_project() {
        let pool = migrated_pool().await;

        seed_conv(&pool, "c1", "chat", Some("p1"), None, None, "2026-08-18 10:00:00").await;
        seed_conv(&pool, "c2", "chat", Some("p1"), None, None, "2026-08-18 10:00:00").await;
        seed_conv(&pool, "t1", "delegation", Some("p1"), Some("agent-1"), Some("c1"), "2026-08-18 10:00:00").await;
        seed_message(&pool, "m1", "c1").await;
        seed_message(&pool, "m2", "t1").await;
        // 他项目混入
        seed_conv(&pool, "x", "chat", Some("p2"), None, None, "2026-08-18 10:00:00").await;
        seed_message(&pool, "m3", "x").await;

        let ov = get_project_overview(&pool, "p1").await.unwrap();
        assert_eq!(ov.chat_conversations, 2);
        assert_eq!(ov.delegation_conversations, 1);
        assert_eq!(ov.messages, 2, "只数本项目会话的消息");
        assert!(ov.last_activity_at.is_some());

        // 空项目全 0/None
        let empty = get_project_overview(&pool, "p-none").await.unwrap();
        assert_eq!(empty.chat_conversations, 0);
        assert_eq!(empty.delegation_conversations, 0);
        assert_eq!(empty.messages, 0);
        assert!(empty.last_activity_at.is_none());
    }
}
