//! `message_attachments` 表的 SQL 操作 —— 聊天附件分页的「提取后按块文本」存储。
//!
//! 写入由 `chat_cmd::send_message` 在 materialize 大附件时批量插入（每文件多块）；
//! 读取由 `read_attachment_page` 工具按 `(message_id, idx)` 取单块。
//!
//! 生命周期：`message_id` 外键 `ON DELETE CASCADE`——消息删除时块自动清除，
//! 无孤儿、无 GC。详见 migration 42。

use sqlx::{FromRow, SqlitePool};

use crate::error::AppResult;

/// 一行 message_attachments 的强类型映射。
#[derive(Debug, FromRow)]
pub struct MessageAttachmentRow {
    pub id: String,
    pub message_id: String,
    pub idx: i64,
    pub name: String,
    pub kind: String,
    pub label: String,
    pub text: String,
    pub token_est: i64,
}

/// 一块的写入入参（`message_id` 由 [`insert_batch`] 统一传，故不在此结构内）。
/// 字段 owned——`chat_cmd` 在循环里逐文件提取，块跨循环迭代累积，借用会悬垂。
pub struct AttachmentChunkInput {
    /// 0-based 块序（read_attachment_page 的 page = idx + 1）
    pub idx: i64,
    pub name: String,
    pub kind: String,
    pub label: String,
    pub text: String,
    pub token_est: i64,
}

/// 批量写入一个消息的所有块。
///
/// `id = "{message_id}:{idx}"`（确定性复合主键，幂等；同消息重复 materialize 走
/// `DELETE + INSERT`，调用方负责先清旧块）。逐行 INSERT——附件块数通常 ≤ 几十
/// （PDF 几百页是极端上限），无需事务批量优化。
pub async fn insert_batch(
    pool: &SqlitePool,
    message_id: &str,
    chunks: &[AttachmentChunkInput],
) -> AppResult<()> {
    for c in chunks {
        let id = format!("{}:{}", message_id, c.idx);
        sqlx::query(
            "INSERT INTO message_attachments
                (id, message_id, idx, name, kind, label, text, token_est)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(message_id)
        .bind(c.idx)
        .bind(&c.name)
        .bind(&c.kind)
        .bind(&c.label)
        .bind(&c.text)
        .bind(c.token_est)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// 删除一个消息的所有块（重新 materialize 前清旧块，保持幂等）。
pub async fn delete_by_message(pool: &SqlitePool, message_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM message_attachments WHERE message_id = ?")
        .bind(message_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 取第 `idx`（0-based）块；不存在返回 `None`。
pub async fn get_page(
    pool: &SqlitePool,
    message_id: &str,
    idx: i64,
) -> AppResult<Option<MessageAttachmentRow>> {
    let row = sqlx::query_as::<_, MessageAttachmentRow>(
        "SELECT id, message_id, idx, name, kind, label, text, token_est
         FROM message_attachments WHERE message_id = ? AND idx = ?",
    )
    .bind(message_id)
    .bind(idx)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 一个消息的块数（= read_attachment_page 的 `total_pages`）。
pub async fn count_by_message(pool: &SqlitePool, message_id: &str) -> AppResult<i64> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM message_attachments WHERE message_id = ?")
        .bind(message_id)
        .fetch_one(pool)
        .await?;
    Ok(n)
}

// =========================================================================
// 单元测试（in-memory sqlite，仿 tool_call 测试模式）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 内存库 + 跑迁移（建 message_attachments 及其 messages 外键依赖）。
    async fn fresh_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .expect("connect :memory:");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        pool
    }

    /// 建一条 messages 行作为外键父（FK CASCADE 测试需要真实父行）。
    /// 连同其外键祖先（agents → conversations）一起 seed，否则 messages 插入违反
    /// FK 787。conv1/a1 是常量主键、用 OR IGNORE 幂等（多次调用安全）。
    async fn seed_message(pool: &SqlitePool, id: &str) {
        sqlx::query(
            "INSERT OR IGNORE INTO agents (id,name,provider,model,api_key_ref)
             VALUES ('a1','test-agent','anthropic','claude-test','k')",
        )
        .execute(pool)
        .await
        .expect("seed agent");
        sqlx::query("INSERT OR IGNORE INTO conversations (id,agent_id) VALUES ('conv1','a1')")
            .execute(pool)
            .await
            .expect("seed conversation");
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, role, content, content_blocks, created_at)
             VALUES (?, 'conv1', 'user', '', '[]', '2026-01-01 00:00:00')",
        )
        .bind(id)
        .execute(pool)
        .await
        .expect("seed message");
    }

    #[sqlx::test]
    async fn insert_and_get_page_roundtrip() {
        let pool = fresh_pool().await;
        seed_message(&pool, "msg1").await;
        let chunks = vec![
            AttachmentChunkInput {
                idx: 0,
                name: "x.pdf".into(),
                kind: "pdf".into(),
                label: "第1页".into(),
                text: "首页内容".into(),
                token_est: 4,
            },
            AttachmentChunkInput {
                idx: 1,
                name: "x.pdf".into(),
                kind: "pdf".into(),
                label: "第2页".into(),
                text: "第二页".into(),
                token_est: 3,
            },
        ];
        insert_batch(&pool, "msg1", &chunks).await.expect("insert");

        assert_eq!(count_by_message(&pool, "msg1").await.unwrap(), 2);

        let p1 = get_page(&pool, "msg1", 0).await.unwrap().expect("page 0");
        assert_eq!(p1.label, "第1页");
        assert_eq!(p1.text, "首页内容");
        assert_eq!(p1.id, "msg1:0");

        let miss = get_page(&pool, "msg1", 99).await.unwrap();
        assert!(miss.is_none(), "越界页应 None");
    }

    #[sqlx::test]
    async fn cascade_delete_with_message() {
        let pool = fresh_pool().await;
        seed_message(&pool, "msg1").await;
        insert_batch(
            &pool,
            "msg1",
            &[AttachmentChunkInput {
                idx: 0,
                name: "x.pdf".into(),
                kind: "pdf".into(),
                label: "第1页".into(),
                text: "c".into(),
                token_est: 1,
            }],
        )
        .await
        .unwrap();
        assert_eq!(count_by_message(&pool, "msg1").await.unwrap(), 1);

        // 删父消息 → 块应被 CASCADE 清掉
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind("msg1")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            count_by_message(&pool, "msg1").await.unwrap(),
            0,
            "CASCADE 应清空块"
        );
    }

    #[sqlx::test]
    async fn delete_by_message_is_idempotent() {
        let pool = fresh_pool().await;
        seed_message(&pool, "msg1").await;
        insert_batch(
            &pool,
            "msg1",
            &[AttachmentChunkInput {
                idx: 0,
                name: "x.pdf".into(),
                kind: "pdf".into(),
                label: "第1页".into(),
                text: "c".into(),
                token_est: 1,
            }],
        )
        .await
        .unwrap();
        delete_by_message(&pool, "msg1").await.unwrap();
        delete_by_message(&pool, "msg1").await.unwrap(); // 再删不报错
        assert_eq!(count_by_message(&pool, "msg1").await.unwrap(), 0);
    }
}
