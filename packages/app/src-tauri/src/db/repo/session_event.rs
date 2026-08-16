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

/// 尾部优先分页读取（Trajectory 回放的大会话数据层）。
///
/// 取 seq 严格小于 `before_seq`（`None` = 从最新开始）的最大 `limit` 条，
/// 返回前反转为 seq 正序——与 [`list_by_session`] 输出同构，可直接拼接分组。
/// 「加载更早」= 用当前已加载的最小 seq 作为下一页 `before_seq`。
pub async fn list_tail(
    pool: &SqlitePool,
    session_id: &str,
    before_seq: Option<i64>,
    limit: i64,
) -> AppResult<Vec<SessionEventRow>> {
    let mut rows = sqlx::query_as::<_, SessionEventRow>(
        "SELECT id, session_id, seq, kind, actor, turn_id, message_id, payload, created_at
           FROM session_events
          WHERE session_id = ? AND seq < COALESCE(?, 9223372036854775807)
          ORDER BY seq DESC
          LIMIT ?",
    )
    .bind(session_id)
    .bind(before_seq)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.reverse();
    Ok(rows)
}

/// 崩溃自愈扫尾的输入：全部「已开始但未闭合」的 turn——有 `turn_context`
/// 但无同 turn_id 的 `turn_ended`（进程死亡绕过了所有退出路径）。
///
/// 本地单进程应用在启动时刻可确定性判定这些 turn 已死（不可能还有进程在
/// 生成）。返回 `(session_id, turn_id, actor, rounds)`；rounds 取该 turn 已落
/// 的 assistant_message 事件数（每条对应一个 finalize 点，续写 supersede 场景
/// 为近似值——终态 payload 的 rounds 仅作展示，不参与任何判定）。
pub async fn find_open_turns(pool: &SqlitePool) -> AppResult<Vec<(String, String, String, i64)>> {
    let rows = sqlx::query_as::<_, (String, String, String, i64)>(
        "SELECT e.session_id, e.turn_id, e.actor,
                (SELECT COUNT(*) FROM session_events a
                  WHERE a.session_id = e.session_id AND a.turn_id = e.turn_id
                    AND a.kind = 'assistant_message') AS rounds
           FROM session_events e
          WHERE e.kind = 'turn_context' AND e.turn_id IS NOT NULL
            AND NOT EXISTS (
                SELECT 1 FROM session_events t
                 WHERE t.session_id = e.session_id AND t.turn_id = e.turn_id
                   AND t.kind = 'turn_ended')
          ORDER BY e.id ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 正向增量读取（轨迹 live 追加）：取 seq 严格大于 `after_seq` 的最早 `limit` 条
/// （seq 正序）。`after_seq = 0`/`None` = 从头取。轮询方以已载最大 seq 作游标，
/// 返回空 = 已追平。append-only 保证增量只会出现在尾部，无需考虑中间插入。
pub async fn list_after(
    pool: &SqlitePool,
    session_id: &str,
    after_seq: Option<i64>,
    limit: i64,
) -> AppResult<Vec<SessionEventRow>> {
    let rows = sqlx::query_as::<_, SessionEventRow>(
        "SELECT id, session_id, seq, kind, actor, turn_id, message_id, payload, created_at
           FROM session_events
          WHERE session_id = ? AND seq > COALESCE(?, 0)
          ORDER BY seq ASC
          LIMIT ?",
    )
    .bind(session_id)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 取一个会话的当前最大 seq（无事件时为 0）。
pub async fn max_seq(pool: &SqlitePool, session_id: &str) -> AppResult<i64> {
    let (max,): (i64,) =
        sqlx::query_as("SELECT COALESCE(MAX(seq), 0) FROM session_events WHERE session_id = ?")
            .bind(session_id)
            .fetch_one(pool)
            .await?;
    Ok(max)
}

/// 窗口前（`seq < before_seq` 一侧）的全局轮次数——轨迹尾部优先分页的轮号偏移（M3）。
///
/// 按 `COUNT(DISTINCT turn_id)` 计（`turn_id IS NULL` 的孤儿事件经 COALESCE 算作
/// 一组，与前端 `__orphan__` 桶对应）。已知边缘误差：前端按「连续同 turn_key 段」
/// 切桶，孤儿事件若被真实轮分隔成多段，前端算多桶而 DISTINCT 只算一组——纪元前
/// 事件实际连续排列，此场景极罕见，偏差 ≤ 孤儿段数，可接受。
pub async fn count_turns_before(
    pool: &SqlitePool,
    session_id: &str,
    before_seq: i64,
) -> AppResult<i64> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT COALESCE(turn_id, '')) FROM session_events
          WHERE session_id = ? AND seq < ?",
    )
    .bind(session_id)
    .bind(before_seq)
    .fetch_one(pool)
    .await?;
    Ok(n)
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
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
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
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
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
    async fn count_turns_before_counts_distinct_orphan_grouped() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed_agent(&pool).await;
        seed_conversation(&pool, "conv-1").await;

        // 2 个孤儿事件（turn_id=NULL，算一组）+ turn-1 ×2 + turn-2 ×1 + turn-3 ×1
        for (turn, msg) in [
            (None, None),
            (None, None),
            (Some("turn-1"), Some("m1")),
            (Some("turn-1"), Some("m2")),
            (Some("turn-2"), Some("m3")),
            (Some("turn-3"), Some("m4")),
        ] {
            append(
                &pool,
                "conv-1",
                "assistant_message",
                "agent",
                turn,
                msg,
                "{}",
            )
            .await
            .unwrap();
        }
        // seq 1..6；窗口起点取各处：偏移 = 窗口前 distinct 轮组数
        assert_eq!(count_turns_before(&pool, "conv-1", 1).await.unwrap(), 0);
        assert_eq!(
            count_turns_before(&pool, "conv-1", 3).await.unwrap(),
            1,
            "孤儿组算 1"
        );
        assert_eq!(
            count_turns_before(&pool, "conv-1", 5).await.unwrap(),
            2,
            "含 turn-1"
        );
        assert_eq!(
            count_turns_before(&pool, "conv-1", 6).await.unwrap(),
            3,
            "含 turn-1/2"
        );
        assert_eq!(
            count_turns_before(&pool, "conv-1", 99).await.unwrap(),
            4,
            "全量"
        );
        // 其他会话不受影响（无事件不报错）
        assert_eq!(count_turns_before(&pool, "conv-none", 99).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn concurrent_appends_stay_unique_and_contiguous() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed_agent(&pool).await;
        seed_conversation(&pool, "conv-1").await;

        let mut handles = Vec::new();
        for i in 0..20 {
            let pool = pool.clone();
            handles.push(tokio::spawn(async move {
                append(
                    &pool,
                    "conv-1",
                    "assistant_message",
                    "agent:agent-1",
                    Some("t"),
                    Some(&format!("m{i}")),
                    "{}",
                )
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
        assert_eq!(
            max_seq(&pool, "conv-1").await.unwrap(),
            20,
            "seq 连续覆盖 1..=20"
        );
    }

    #[tokio::test]
    async fn list_tail_pages_from_newest_backward() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed_agent(&pool).await;
        seed_conversation(&pool, "conv-1").await;

        for i in 0..10 {
            append(
                &pool,
                "conv-1",
                "user_message",
                "user",
                Some("t"),
                Some(&format!("m{i}")),
                "{}",
            )
            .await
            .unwrap();
        }

        // 首页：从最新往回取 4 条，返回须已反转为正序
        let p1 = list_tail(&pool, "conv-1", None, 4).await.unwrap();
        assert_eq!(seqs(&p1), vec![7, 8, 9, 10], "首页取最尾 4 条且正序");

        // 「加载更早」：before = 当前最小 seq (7)，取上一页
        let p2 = list_tail(&pool, "conv-1", Some(7), 4).await.unwrap();
        assert_eq!(seqs(&p2), vec![3, 4, 5, 6], "第二页边界严格小于 before_seq");

        // 头部剩余不足一页：只返回剩下的
        let p3 = list_tail(&pool, "conv-1", Some(3), 4).await.unwrap();
        assert_eq!(seqs(&p3), vec![1, 2], "不足一页返回剩余全部");

        // 再往前取空 → 调用方据此判定 hasMore=false
        let p4 = list_tail(&pool, "conv-1", Some(1), 4).await.unwrap();
        assert!(p4.is_empty(), "取穿后返回空");

        // 跨会话隔离
        seed_conversation(&pool, "conv-2").await;
        append(&pool, "conv-2", "user_message", "user", None, None, "{}")
            .await
            .unwrap();
        let cross = list_tail(&pool, "conv-2", None, 4).await.unwrap();
        assert_eq!(seqs(&cross), vec![1], "分页不串会话");
    }

    fn seqs(rows: &[SessionEventRow]) -> Vec<i64> {
        rows.iter().map(|r| r.seq).collect()
    }

    #[tokio::test]
    async fn list_after_returns_forward_increment() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed_agent(&pool).await;
        seed_conversation(&pool, "conv-1").await;

        for i in 0..10 {
            append(
                &pool,
                "conv-1",
                "user_message",
                "user",
                Some("t"),
                Some(&format!("m{i}")),
                "{}",
            )
            .await
            .unwrap();
        }

        // from head
        let head = list_after(&pool, "conv-1", None, 3).await.unwrap();
        assert_eq!(seqs(&head), vec![1, 2, 3], "None = 从头取正序");

        // 游标 3 → 拿 4..6
        let inc = list_after(&pool, "conv-1", Some(3), 3).await.unwrap();
        assert_eq!(seqs(&inc), vec![4, 5, 6], "严格大于 after_seq");

        // 追平后返回空
        let done = list_after(&pool, "conv-1", Some(10), 3).await.unwrap();
        assert!(done.is_empty(), "追平返回空");

        // 跨会话隔离
        let cross = list_after(&pool, "conv-none", None, 3).await.unwrap();
        assert!(cross.is_empty(), "不串会话");
    }

    #[tokio::test]
    async fn duplicate_seq_rejected_by_unique_index() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
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
