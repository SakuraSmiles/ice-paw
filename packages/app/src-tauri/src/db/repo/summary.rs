//! `messages` 表摘要操作（M1.5 A3-4 滚动摘要）
//!
//! 摘要作为一条 `role="system"` 的消息插入 `messages` 表，
//! content 格式为 `"[Previous conversation summary]\n{summary_text}"`。
//!
//! 查询最新摘要：查找该会话中 content 以指定前缀开头的最新一条 system 消息。
//! 该方案遵循 dev1 评审建议：不新建 message_summaries 表，而是在 messages
//! 表加 summary_id 列（migration 07），摘要消息 summary_id=NULL。

use sqlx::SqlitePool;

use crate::error::AppResult;

/// 摘要消息 content 前缀
pub const SUMMARY_PREFIX: &str = "[Previous conversation summary]";

/// 当前摘要状态（Phase 2 滚动增量摘要）
///
/// 每个会话至多一份当前摘要。锚点双值：`covered_until_seq`（事件纪元语义锚，
/// Phase 2B 阶段 2 起）+ `covered_until_rowid`（物理 rowid，兜底）。调用方
/// 读序为 seq 优先、rowid 兜底（`.or_else`）。
/// 双双 `None` = 旧版摘要行（Phase 2 之前写入、无覆盖指针）→ 调用方按「从头折叠」自愈。
#[derive(Debug, Clone)]
pub struct SummaryState {
    /// 摘要消息行的 id（UPDATE-in-place 用）
    pub row_id: String,
    /// 摘要正文（已去前缀）
    pub text: String,
    /// 覆盖的最后一条消息的**首现**事件 seq；None = 无事件锚点（旧会话 / 旧版行）
    pub covered_until_seq: Option<i64>,
    /// 覆盖的最后一条消息 rowid；None = 未知（legacy / 未设）
    pub covered_until_rowid: Option<i64>,
}

/// 插入一条摘要消息（role="system"），返回消息 ID
///
/// - `conversation_id`       会话 ID
/// - `summary_text`           摘要正文（不含前缀，函数会拼接）
/// - `covered_until_seq`      覆盖终点消息的首现事件 seq（事件纪元锚）
/// - `covered_until_rowid`    本摘要覆盖的最后一条 user/assistant 消息的 rowid（兜底锚）
pub async fn insert_summary_message(
    pool: &SqlitePool,
    conversation_id: &str,
    summary_text: &str,
    covered_until_seq: Option<i64>,
    covered_until_rowid: i64,
) -> AppResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let content = format!("{SUMMARY_PREFIX}\n{summary_text}");

    sqlx::query(
        "INSERT INTO messages
            (id, conversation_id, role, content, content_blocks, token_count, error,
             covered_until_seq, covered_until_rowid)
         VALUES (?, ?, 'system', ?, '[]', NULL, NULL, ?, ?)",
    )
    .bind(&id)
    .bind(conversation_id)
    .bind(&content)
    .bind(covered_until_seq)
    .bind(covered_until_rowid)
    .execute(pool)
    .await?;

    // 更新会话的 updated_at
    sqlx::query("UPDATE conversations SET updated_at = datetime('now') WHERE id = ?")
        .bind(conversation_id)
        .execute(pool)
        .await?;

    Ok(id)
}

/// UPDATE-in-place：更新既有摘要行的正文与双覆盖锚点（保持单例、UI 气泡位置稳定）
///
/// 滚动折叠每次推进锚点并改写正文——用 UPDATE 而非 INSERT+保留旧行，
/// 避免摘要行无限堆积、避免 UI 出现多条历史摘要气泡。
pub async fn update_summary_message(
    pool: &SqlitePool,
    row_id: &str,
    summary_text: &str,
    covered_until_seq: Option<i64>,
    covered_until_rowid: i64,
) -> AppResult<()> {
    let content = format!("{SUMMARY_PREFIX}\n{summary_text}");
    sqlx::query(
        "UPDATE messages
            SET content = ?, covered_until_seq = ?, covered_until_rowid = ?
          WHERE id = ?",
    )
    .bind(&content)
    .bind(covered_until_seq)
    .bind(covered_until_rowid)
    .bind(row_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 查询会话最新的摘要文本（去掉前缀）
///
/// 查找该会话中 `role="system"` 且 `content` 以 `[Previous conversation summary]` 开头
/// 的最新一条消息，返回其摘要正文。
///
/// - 有摘要 → `Some(summary_text)`
/// - 无摘要 → `None`
pub async fn get_latest_summary(
    pool: &SqlitePool,
    conversation_id: &str,
) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT content
           FROM messages
          WHERE conversation_id = ?
            AND role = 'system'
            AND instr(content, ?) = 1
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .bind(conversation_id)
    .bind(SUMMARY_PREFIX)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((content,)) => {
            // 去掉前缀 + 换行
            let summary = content
                .strip_prefix(SUMMARY_PREFIX)
                .map(|s| s.strip_prefix('\n').unwrap_or(s).to_string())
                .unwrap_or(content);
            Ok(Some(summary))
        }
        None => Ok(None),
    }
}

/// 查询会话当前摘要的完整状态（正文 + 覆盖指针 + 行 id）—— Phase 2 滚动折叠主用
///
/// 与 [`get_latest_summary`] 同样的定位条件（最新一条 role=system 摘要行），
/// 但额外返回 `id`（供 [`update_summary_message`]）与 `covered_until_rowid`。
pub async fn get_latest_summary_state(
    pool: &SqlitePool,
    conversation_id: &str,
) -> AppResult<Option<SummaryState>> {
    let row: Option<(String, String, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT id, content, covered_until_seq, covered_until_rowid
           FROM messages
          WHERE conversation_id = ?
            AND role = 'system'
            AND instr(content, ?) = 1
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .bind(conversation_id)
    .bind(SUMMARY_PREFIX)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((row_id, content, covered_until_seq, covered_until_rowid)) => {
            let text = content
                .strip_prefix(SUMMARY_PREFIX)
                .map(|s| s.strip_prefix('\n').unwrap_or(s).to_string())
                .unwrap_or(content);
            Ok(Some(SummaryState {
                row_id,
                text,
                covered_until_seq,
                covered_until_rowid,
            }))
        }
        None => Ok(None),
    }
}

/// 标记一组消息为「已被某条摘要覆盖」
///
/// 将 first_message_id 到 last_message_id 之间的消息的 summary_id 设为
/// summary_msg_id。仅用于审计目的（记录哪些消息被摘要替代）。
///
/// M1.5 阶段暂不调用（简化实现），后续可用。
#[allow(dead_code)]
pub async fn mark_as_summarized(
    pool: &SqlitePool,
    first_message_id: &str,
    last_message_id: &str,
    summary_msg_id: &str,
) -> AppResult<u64> {
    let affected = sqlx::query(
        "UPDATE messages
            SET summary_id = ?
          WHERE summary_id IS NULL
            AND id >= ?
            AND id <= ?",
    )
    .bind(summary_msg_id)
    .bind(first_message_id)
    .bind(last_message_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected)
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

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

    /// 种子数据：创建 agent + conversation（外键依赖）
    async fn seed(pool: &SqlitePool, conv_id: &str) {
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind("agent-s")
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
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES (?, ?, ?)")
            .bind(conv_id)
            .bind("agent-s")
            .bind("test conv")
            .execute(pool)
            .await
            .expect("seed conversation");
    }

    #[tokio::test]
    async fn insert_summary_message_writes_row() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv-s1").await;

        let id = insert_summary_message(&pool, "conv-s1", "用户想修改 foo 函数，已完成", None, 50)
            .await
            .unwrap();

        // 验证消息已写入 DB
        let row: Option<(String, String, String)> =
            sqlx::query_as("SELECT id, role, content FROM messages WHERE id = ?")
                .bind(&id)
                .fetch_optional(&pool)
                .await
                .unwrap();

        let (msg_id, role, content) = row.unwrap();
        assert_eq!(msg_id, id);
        assert_eq!(role, "system");
        assert!(content.starts_with(SUMMARY_PREFIX));
        assert!(content.contains("用户想修改 foo 函数"));
    }

    #[tokio::test]
    async fn get_latest_summary_returns_most_recent() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv-s2").await;

        // 插入两条摘要（第一条先，第二条后）
        insert_summary_message(&pool, "conv-s2", "第一条摘要", None, 10)
            .await
            .unwrap();
        insert_summary_message(&pool, "conv-s2", "第二条摘要", None, 20)
            .await
            .unwrap();

        let summary = get_latest_summary(&pool, "conv-s2").await.unwrap();
        assert_eq!(summary, Some("第二条摘要".to_string()));
    }

    #[tokio::test]
    async fn get_latest_summary_returns_none_when_empty() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv-s3").await;

        let summary = get_latest_summary(&pool, "conv-s3").await.unwrap();
        assert!(summary.is_none());
    }

    #[tokio::test]
    async fn get_latest_summary_state_returns_three_fields() {
        // Phase 2：返回 (row_id, text 去前缀, covered_until_rowid)
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv-s4").await;

        let id = insert_summary_message(&pool, "conv-s4", "状态摘要正文", Some(44), 42)
            .await
            .unwrap();

        let state = get_latest_summary_state(&pool, "conv-s4")
            .await
            .unwrap()
            .expect("应有摘要状态");
        assert_eq!(state.row_id, id, "row_id 应为插入返回的 id");
        assert_eq!(state.text, "状态摘要正文", "text 应已去前缀");
        assert_eq!(state.covered_until_rowid, Some(42));
        assert_eq!(state.covered_until_seq, Some(44));
    }

    #[tokio::test]
    async fn update_summary_message_updates_in_place() {
        // UPDATE-in-place：保持同一行 id，更新正文 + covered_until_rowid
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv-s5").await;

        let id = insert_summary_message(&pool, "conv-s5", "第一版", None, 10)
            .await
            .unwrap();

        update_summary_message(&pool, &id, "第二版", None, 25)
            .await
            .unwrap();

        // 仍只有一条摘要行（单例）
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE role='system' AND instr(content, ?) = 1",
        )
        .bind(SUMMARY_PREFIX)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "UPDATE-in-place 应保持单例");

        let state = get_latest_summary_state(&pool, "conv-s5")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.row_id, id, "行 id 不变");
        assert_eq!(state.text, "第二版", "正文应已更新");
        assert_eq!(state.covered_until_rowid, Some(25), "covered 应已推进");
        assert_eq!(state.covered_until_seq, None, "显式传 None 时 seq 保持空");
    }

    /// migration 46 回填语义：`covered_until_seq` = 锚点消息**首现**消息类事件 seq。
    ///
    /// fresh 库 `migrate!` 时 UPDATE 空转（行是之后才插的），此处手工重放与
    /// 46 号 migration **逐字相同**的 UPDATE 验证 SQL 逻辑：
    /// - supersede（同 message_id 两条 assistant_message seq5/7）→ MIN 取 5
    ///   （first_seq 定义，与 derive 排序位一致；若误用 MAX 会得 7）
    /// - kind IN 过滤：tool_execution(m-a) seq4 **前置于**首条 assistant_message
    ///   ——合成排布，pin「非消息类事件不参与回填」（无过滤会得 4）
    /// - 零事件锚点 → NULL（运行期 rowid 兜底）
    #[tokio::test]
    async fn covered_until_seq_backfill_takes_first_message_event() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed(&pool, "conv-s6").await;
        // 零事件会话（pre-Phase-0 旧库残留、backfill 未覆盖的形态）
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES ('conv-s7', 'agent-s', 't')")
            .execute(&pool)
            .await
            .unwrap();

        // conv-s6：锚点消息 m-a + 事件序（seq 由 per-session MAX+1 分配）
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, role, content, content_blocks)
             VALUES ('m-a', 'conv-s6', 'assistant', 'x', '[]')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let anchor_rowid: i64 =
            sqlx::query_scalar("SELECT rowid FROM messages WHERE id = 'm-a'")
                .fetch_one(&pool)
                .await
                .unwrap();
        // 事件 payload 对本 SQL 不敏感（只看 kind/message_id/seq），统一 "{}"。
        let mut seqs = Vec::new();
        for (kind, mid) in [
            ("turn_context", None),             // seq1
            ("user_message", Some("m-u")),      // seq2
            ("tool_execution", Some("m-a")),    // seq3 ← 非消息类且最小，kind 过滤的靶子
            ("assistant_message", Some("m-a")), // seq4 ← 首现（期望值）
            ("tool_execution", Some("m-a")),    // seq5
            ("assistant_message", Some("m-a")), // seq6 ← supersede（MIN 不取它）
            ("turn_ended", None),               // seq7
        ] {
            seqs.push(
                crate::db::repo::session_event::append(
                    &pool,
                    "conv-s6",
                    kind,
                    "agent:agent-s",
                    Some("t1"),
                    mid,
                    "{}",
                )
                .await
                .unwrap(),
            );
        }
        assert_eq!(
            seqs,
            vec![1, 2, 3, 4, 5, 6, 7],
            "per-session seq 应从 1 连续分配"
        );

        // conv-s7：零事件锚点消息 m-b
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, role, content, content_blocks)
             VALUES ('m-b', 'conv-s7', 'user', 'y', '[]')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let zero_rowid: i64 =
            sqlx::query_scalar("SELECT rowid FROM messages WHERE id = 'm-b'")
                .fetch_one(&pool)
                .await
                .unwrap();

        // 预置 pre-migration 形态的摘要行（covered_until_seq=NULL）
        insert_summary_message(&pool, "conv-s6", "有事件会话摘要", None, anchor_rowid)
            .await
            .unwrap();
        insert_summary_message(&pool, "conv-s7", "零事件会话摘要", None, zero_rowid)
            .await
            .unwrap();

        // 手工重放 migration 46 的 UPDATE（空库 migrate 时已空转）
        sqlx::query(
            "UPDATE messages
             SET covered_until_seq = (
                 SELECT MIN(e.seq)
                   FROM session_events e
                   JOIN messages m ON m.rowid = messages.covered_until_rowid
                  WHERE e.session_id = messages.conversation_id
                    AND e.message_id = m.id
                    AND e.kind IN ('user_message', 'assistant_message', 'tool_result_message')
             )
             WHERE covered_until_rowid IS NOT NULL
               AND role = 'system'
               AND instr(content, '[Previous conversation summary]') = 1",
        )
        .execute(&pool)
        .await
        .unwrap();

        let state = get_latest_summary_state(&pool, "conv-s6")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            state.covered_until_seq,
            Some(4),
            "应取首条消息类事件 seq（MIN+kind 过滤）；无过滤得 3，误用 MAX 得 6"
        );
        let state_zero = get_latest_summary_state(&pool, "conv-s7")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            state_zero.covered_until_seq, None,
            "零事件锚点 → NULL（运行期 rowid 兜底）"
        );
    }
}
