//! `session_events` 事件日志表的 SQL 操作（session-event-log Phase 0）。
//!
//! append-only 不变式：本模块只提供 INSERT 与按 seq 正序的读取，永不
//! UPDATE/DELETE（唯一删除路径是会话 CASCADE）。seq 由 INSERT 内子查询
//! 分配，单语句原子（见 migration 44 头注释）。
//!
//! Phase 0 定位是影子写入：调用方（`harness::event_log`）append 失败仅
//! warn 不阻断主流程；产生的「缺口」无 seq 空洞（MAX+1 连续），只能靠
//! Phase 1 derive 对账发现——这是已文档化的定位取舍，不是疏漏。

use sqlx::SqlitePool;

use crate::db::models::SessionEventRow;
use crate::error::AppResult;

/// 追加一条会话事件，返回自增 id。
///
/// seq 在语句内取 `MAX(seq)+1`（首次为 1）；`payload` 是调用方序列化好的
/// JSON 字符串（强类型 struct 见 `harness::event_log`）。
#[allow(clippy::too_many_arguments)]
pub async fn append(
    pool: &SqlitePool,
    session_id: &str,
    kind: &str,
    actor: &str,
    turn_id: Option<&str>,
    message_id: Option<&str>,
    payload: &str,
) -> AppResult<i64> {
    let result = sqlx::query(
        "INSERT INTO session_events
            (session_id, seq, kind, actor, turn_id, message_id, payload)
         VALUES (?, (SELECT COALESCE(MAX(seq), 0) + 1 FROM session_events WHERE session_id = ?),
                 ?, ?, ?, ?, ?)",
    )
    .bind(session_id)
    .bind(session_id)
    .bind(kind)
    .bind(actor)
    .bind(turn_id)
    .bind(message_id)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

/// 按 seq 正序读取一个会话的事件流（回放序）。
///
/// `limit` 为 `None` 时全量；Phase 1 derive 与 Trajectory 导出共用本入口。
pub async fn list_by_session(
    pool: &SqlitePool,
    session_id: &str,
    limit: Option<i64>,
) -> AppResult<Vec<SessionEventRow>> {
    let rows = sqlx::query_as::<_, SessionEventRow>(
        "SELECT id, session_id, seq, kind, actor, turn_id, message_id, payload, created_at
           FROM session_events
          WHERE session_id = ?
          ORDER BY seq ASC
          LIMIT ?",
    )
    .bind(session_id)
    .bind(limit.unwrap_or(-1))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 取一个会话的当前最大 seq（无事件时为 0）。
pub async fn max_seq(pool: &SqlitePool, session_id: &str) -> AppResult<i64> {
    let (max,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(MAX(seq), 0) FROM session_events WHERE session_id = ?",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await?;
    Ok(max)
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    /// 注：in-memory SQLite 每连接各一个库，pool 必须 max_connections(1)
    /// （与 tool_call.rs 测试同一坑）。并发测试关注的是「语句间交错下 seq
    /// 子查询仍正确」，单连接上 futures 交错执行同样覆盖该逻辑，且确定性。
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

    /// seed agent → conversation（session_events.session_id 外键依赖）。
    /// agent 只 seed 一次；多会话测试先 `seed_agent` 再逐个 `seed_conversation`。
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

    async fn seed_conversation(pool: &SqlitePool, conv_id: &str) {
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES (?, ?, ?)")
            .bind(conv_id)
            .bind("agent-1")
            .bind("test conv")
            .execute(pool)
            .await
            .expect("seed conversation");
    }

    #[tokio::test]
    async fn append_assigns_monotonic_seq_and_ids() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool).await;
        seed_conversation(&pool, "conv-1").await;

        let mut ids = Vec::new();
        for i in 0..5 {
            let id = append(
                &pool,
                "conv-1",
                "user_message",
                "user",
                Some("turn-1"),
                Some(&format!("msg-{i}")),
                "{}",
            )
            .await
            .unwrap();
            ids.push(id);
        }

        let rows = list_by_session(&pool, "conv-1", None).await.unwrap();
        let seqs: Vec<i64> = rows.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5], "seq 应从 1 起单调连续");
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "全局自增 id 应严格递增"
        );
        assert_eq!(max_seq(&pool, "conv-1").await.unwrap(), 5);
        // max_seq 对无事件会话返回 0，不报错
        assert_eq!(max_seq(&pool, "conv-none").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn seq_is_independent_across_sessions() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool).await;
        seed_conversation(&pool, "conv-a").await;
        seed_conversation(&pool, "conv-b").await;

        // 交错追加：a, b, a, b, a
        for conv in ["conv-a", "conv-b", "conv-a", "conv-b", "conv-a"] {
            append(&pool, conv, "turn_context", "user", None, None, "{}")
                .await
                .unwrap();
        }

        let a = list_by_session(&pool, "conv-a", None).await.unwrap();
        let b = list_by_session(&pool, "conv-b", None).await.unwrap();
        assert_eq!(
            a.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "conv-a 各自独立连续"
        );
        assert_eq!(
            b.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![1, 2],
            "conv-b 各自独立连续"
        );
    }

    #[tokio::test]
    async fn concurrent_appends_stay_unique_and_contiguous() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool).await;
        seed_conversation(&pool, "conv-1").await;

        let mut handles = Vec::new();
        for i in 0..20 {
            let pool = pool.clone();
            handles.push(tokio::spawn(async move {
                append(&pool, "conv-1", "assistant_message", "agent:agent-1", Some("t"), Some(&format!("m{i}")), "{}")
                    .await
                    .expect("concurrent append should succeed");
            }));
        }
        for h in handles {
            h.await.expect("task join");
        }

        let rows = list_by_session(&pool, "conv-1", None).await.unwrap();
        assert_eq!(rows.len(), 20, "20 条全部落库");
        let seqs: std::collections::HashSet<i64> = rows.iter().map(|r| r.seq).collect();
        assert_eq!(seqs.len(), 20, "seq 无重复");
        assert_eq!(max_seq(&pool, "conv-1").await.unwrap(), 20, "seq 连续覆盖 1..=20");
    }

    #[tokio::test]
    async fn duplicate_seq_rejected_by_unique_index() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool).await;
        seed_conversation(&pool, "conv-1").await;

        append(&pool, "conv-1", "user_message", "user", None, None, "{}")
            .await
            .unwrap();

        // 绕过 append 的子查询，手工插重复 seq → UNIQUE 约束兜底报错
        let err = sqlx::query(
            "INSERT INTO session_events (session_id, seq, kind, actor, payload)
             VALUES ('conv-1', 1, 'user_message', 'user', '{}')",
        )
        .execute(&pool)
        .await;
        assert!(err.is_err(), "重复 seq 必须被 UNIQUE 索引拒绝");
    }
}
