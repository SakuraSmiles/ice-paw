//! `message_attachment_files` 表的 SQL 操作 —— 附件**原始字节**留存（Phase B 视觉/出图用）。
//!
//! 仅当文本提取为空（`total_tokens == 0`，疑似扫描件/纯图片/加密 PDF）时才写入——
//! `view_attachment_image` 工具按需读字节、用 pdfium 渲染指定页为 PNG。文本提取成功的
//! 附件不留存（agent 已有文本，无需视觉），避免给 DB 塞无用大 BLOB。
//!
//! 生命周期：`message_id` 外键 `ON DELETE CASCADE`——消息删除时自动清除，无孤儿、无 GC。
//! 详见 migration 43。

use sqlx::{FromRow, SqlitePool};

use crate::error::AppResult;

/// 一行 message_attachment_files 的强类型映射。
#[derive(Debug, FromRow)]
pub struct MessageAttachmentFileRow {
    pub id: String,
    pub message_id: String,
    pub idx: i64,
    pub name: String,
    pub ext: String,
    pub bytes: Vec<u8>,
    pub created_at: String,
}

/// 一个文件字节的写入入参（`message_id` 由 [`insert_batch`] 统一传）。
/// 字段 owned——`chat_cmd` 在循环里解码字节，跨迭代累积，借用会悬垂。
pub struct AttachmentFileInput {
    /// 0-based 文件序号（一消息内）
    pub idx: i64,
    pub name: String,
    pub ext: String,
    pub bytes: Vec<u8>,
}

/// 批量写入一个消息的（视觉候选）文件字节。
///
/// `id = "{message_id}:{idx}"`。逐行 INSERT——视觉候选文件通常 ≤ 几个（一消息一般
/// 只拖一个扫描件），无需批量优化。调用方负责先 `delete_by_message` 清旧（幂等）。
pub async fn insert_batch(
    pool: &SqlitePool,
    message_id: &str,
    files: &[AttachmentFileInput],
) -> AppResult<()> {
    for f in files {
        let id = format!("{}:{}", message_id, f.idx);
        sqlx::query(
            "INSERT INTO message_attachment_files (id, message_id, idx, name, ext, bytes)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(message_id)
        .bind(f.idx)
        .bind(&f.name)
        .bind(&f.ext)
        .bind(&f.bytes)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// 删除一个消息的所有文件字节（重新 materialize 前清旧，幂等）。
pub async fn delete_by_message(pool: &SqlitePool, message_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM message_attachment_files WHERE message_id = ?")
        .bind(message_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 取第 `idx`（0-based）个文件字节；不存在返回 `None`。
pub async fn get_by_idx(
    pool: &SqlitePool,
    message_id: &str,
    idx: i64,
) -> AppResult<Option<MessageAttachmentFileRow>> {
    let row = sqlx::query_as::<_, MessageAttachmentFileRow>(
        "SELECT id, message_id, idx, name, ext, bytes, created_at
         FROM message_attachment_files WHERE message_id = ? AND idx = ?",
    )
    .bind(message_id)
    .bind(idx)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 取该消息的第一个文件字节（v1 视觉工具默认对单文件场景；多文件时取 idx 最小者）。
pub async fn get_first_by_message(
    pool: &SqlitePool,
    message_id: &str,
) -> AppResult<Option<MessageAttachmentFileRow>> {
    let row = sqlx::query_as::<_, MessageAttachmentFileRow>(
        "SELECT id, message_id, idx, name, ext, bytes, created_at
         FROM message_attachment_files WHERE message_id = ?
         ORDER BY idx ASC LIMIT 1",
    )
    .bind(message_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 一个消息留存的文件数。
pub async fn count_by_message(pool: &SqlitePool, message_id: &str) -> AppResult<i64> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM message_attachment_files WHERE message_id = ?")
            .bind(message_id)
            .fetch_one(pool)
            .await?;
    Ok(n)
}

// =========================================================================
// 单元测试（in-memory sqlite，仿 message_attachment 测试模式）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

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

    async fn seed_message(pool: &SqlitePool, id: &str) {
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
    async fn insert_and_get_roundtrip() {
        let pool = fresh_pool().await;
        seed_message(&pool, "msg1").await;
        insert_batch(
            &pool,
            "msg1",
            &[AttachmentFileInput {
                idx: 0,
                name: "scan.pdf".into(),
                ext: "pdf".into(),
                bytes: b"%PDF-1.4 raw bytes".to_vec(),
            }],
        )
        .await
        .unwrap();

        assert_eq!(count_by_message(&pool, "msg1").await.unwrap(), 1);

        let f = get_by_idx(&pool, "msg1", 0).await.unwrap().expect("file 0");
        assert_eq!(f.name, "scan.pdf");
        assert_eq!(f.ext, "pdf");
        assert_eq!(f.bytes, b"%PDF-1.4 raw bytes");
        assert_eq!(f.id, "msg1:0");

        // get_first_by_message 等价于 idx=0（单文件）
        let first = get_first_by_message(&pool, "msg1").await.unwrap().unwrap();
        assert_eq!(first.bytes, b"%PDF-1.4 raw bytes");

        assert!(get_by_idx(&pool, "msg1", 99).await.unwrap().is_none());
    }

    #[sqlx::test]
    async fn cascade_delete_with_message() {
        let pool = fresh_pool().await;
        seed_message(&pool, "msg1").await;
        insert_batch(
            &pool,
            "msg1",
            &[AttachmentFileInput {
                idx: 0,
                name: "scan.pdf".into(),
                ext: "pdf".into(),
                bytes: vec![1, 2, 3],
            }],
        )
        .await
        .unwrap();
        assert_eq!(count_by_message(&pool, "msg1").await.unwrap(), 1);

        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind("msg1")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            count_by_message(&pool, "msg1").await.unwrap(),
            0,
            "CASCADE 应清空文件字节"
        );
    }
}
