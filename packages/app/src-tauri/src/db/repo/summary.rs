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

/// 插入一条摘要消息（role="system"），返回消息 ID
///
/// - `conversation_id`  会话 ID
/// - `summary_text`      摘要正文（不含前缀，函数会拼接）
/// - `covered_count`     被摘要覆盖的消息条数（M1.5 暂不写入；保留用于审计 / 未来扩展）
pub async fn insert_summary_message(
    pool: &SqlitePool,
    conversation_id: &str,
    summary_text: &str,
    #[allow(unused_variables)] covered_count: i32,
) -> AppResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let content = format!("{SUMMARY_PREFIX}\n{summary_text}");

    sqlx::query(
        "INSERT INTO messages
            (id, conversation_id, role, content, content_blocks, token_count, error)
         VALUES (?, ?, 'system', ?, '[]', NULL, NULL)",
    )
    .bind(&id)
    .bind(conversation_id)
    .bind(&content)
    .execute(pool)
    .await?;

    // 更新会话的 updated_at
    sqlx::query(
        "UPDATE conversations SET updated_at = datetime('now') WHERE id = ?",
    )
    .bind(conversation_id)
    .execute(pool)
    .await?;

    Ok(id)
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
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed(&pool, "conv-s1").await;

        let id = insert_summary_message(
            &pool,
            "conv-s1",
            "用户想修改 foo 函数，已完成",
            50,
        )
        .await
        .unwrap();

        // 验证消息已写入 DB
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT id, role, content FROM messages WHERE id = ?",
        )
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
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed(&pool, "conv-s2").await;

        // 插入两条摘要（第一条先，第二条后）
        insert_summary_message(&pool, "conv-s2", "第一条摘要", 10)
            .await
            .unwrap();
        insert_summary_message(&pool, "conv-s2", "第二条摘要", 20)
            .await
            .unwrap();

        let summary = get_latest_summary(&pool, "conv-s2").await.unwrap();
        assert_eq!(summary, Some("第二条摘要".to_string()));
    }

    #[tokio::test]
    async fn get_latest_summary_returns_none_when_empty() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed(&pool, "conv-s3").await;

        let summary = get_latest_summary(&pool, "conv-s3").await.unwrap();
        assert!(summary.is_none());
    }
}
