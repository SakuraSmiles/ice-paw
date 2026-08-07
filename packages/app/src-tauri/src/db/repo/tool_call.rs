//! `tool_calls` 审计表的 SQL 操作
//!
//! 每次工具调用记一行（tool_name / arguments / result / is_error / duration_ms /
//! 起止时间），供调试与审计。同时为 `loop_engine` 的工具打分提供「最近调用历史」
//! （见 `message::list_recent_tool_names`——历史上因本表从未写入而读到空表，导致
//! 打分的「历史权重」维度静默失效；本模块接入后该维度自动恢复）。
//!
//! 写入由 `tool_executor` 在每次工具执行后调用，失败仅 warn，不影响主流程。

use sqlx::SqlitePool;

use crate::error::AppResult;

/// arguments 列存储上限（字符数）。超长截断，避免大参数（如 write_file 的 content）
/// 撑大审计表；完整参数仍在 messages 的 tool_use 块中。
const MAX_ARGUMENTS_LEN: usize = 4_000;
/// result 列存储上限（字符数）。超长截断；完整输出仍在 messages 的 tool_result 块中
/// （shell 等工具已先截到 20000，审计再截到 4000 够看关键结果与成败）。
const MAX_RESULT_LEN: usize = 4_000;

/// 写入一条工具调用审计记录。
///
/// - `started_at` / `finished_at`：UTC 墙钟，格式 `%Y-%m-%d %H:%M:%S`，与 SQLite
///   `datetime('now')` 一致，便于 SQL 直接做时间差/排序。
/// - `result`：`None` 表示无输出（理论上少见；绝大多数工具都有返回，错误也走 `Some`）。
/// - `arguments` 与 `result` 超长会被截断并标注。
///
/// `id` 由调用方生成（UUID v4）。
pub async fn create(
    pool: &SqlitePool,
    id: &str,
    message_id: &str,
    tool_name: &str,
    arguments: &str,
    result: Option<&str>,
    is_error: bool,
    duration_ms: u64,
    started_at: &str,
    finished_at: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO tool_calls
            (id, message_id, tool_name, arguments, result, is_error, duration_ms, started_at, finished_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(message_id)
    .bind(tool_name)
    .bind(truncate(arguments, MAX_ARGUMENTS_LEN))
    .bind(result.map(|r| truncate(r, MAX_RESULT_LEN)))
    .bind(is_error)
    .bind(duration_ms as i64)
    .bind(started_at)
    .bind(finished_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// 按字符数安全截断（不切断 UTF-8），超长则追加标注。
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push_str("…[已截断]");
    out
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::message;
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

    /// seed agent → conversation → message 链（tool_calls.message_id 外键依赖 messages.id）。
    async fn seed_message(pool: &SqlitePool, msg_id: &str, conv_id: &str) {
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

        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES (?, ?, ?)")
            .bind(conv_id)
            .bind("agent-1")
            .bind("test conv")
            .execute(pool)
            .await
            .expect("seed conversation");

        sqlx::query("INSERT INTO messages (id, conversation_id, role, content) VALUES (?, ?, ?, ?)")
            .bind(msg_id)
            .bind(conv_id)
            .bind("assistant")
            .bind("hi")
            .execute(pool)
            .await
            .expect("seed message");
    }

    #[tokio::test]
    async fn create_inserts_row_and_feeds_list_recent() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_message(&pool, "msg-1", "conv-1").await;

        create(
            &pool,
            "tc-1",
            "msg-1",
            "run_command",
            r#"{"command":"ls"}"#,
            Some("total 0\n"),
            false,
            42,
            "2026-08-07 10:00:00",
            "2026-08-07 10:00:01",
        )
        .await
        .unwrap();

        // 行已写入（含 duration_ms 列）
        let row: (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE((SELECT duration_ms FROM tool_calls WHERE id='tc-1'), -1) \
             FROM tool_calls WHERE message_id = ?",
        )
        .bind("msg-1")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, 1, "应写入 1 行审计");
        assert_eq!(row.1, 42, "duration_ms 应被持久化");

        // list_recent_tool_names 不再读空表 → 工具打分「历史权重」链路打通
        let names = message::list_recent_tool_names(&pool, "conv-1", 10)
            .await
            .unwrap();
        assert_eq!(names, vec!["run_command".to_string()]);
    }

    #[test]
    fn truncate_keeps_short_input() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_cuts_and_marks_long_input() {
        let long = "x".repeat(100);
        let t = truncate(&long, 5);
        assert_eq!(t.chars().count(), 5 + "…[已截断]".chars().count());
        assert!(t.ends_with("…[已截断]"));
    }
}
