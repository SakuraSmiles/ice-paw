//! `kb` / `kb_document` 表的 SQL 操作（RAG v1，agentic 知识库）
//!
//! 三级归属：scope='agent'(owner_id=agent_id) / 'project'(owner_id=project_id) / 'global'(owner_id=NULL)。
//! v1 不含向量/切块，文档级关键词检索。

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::models::{Kb, KbDocumentRow, KbRow, KbSearchHit, NewKb, UpdateKb};
use crate::error::{AppError, AppResult};

const KB_COLS: &str = "id, name, scope, owner_id, directory, enabled, created_at, updated_at";
const DOC_COLS: &str =
    "id, kb_id, file_path, title, summary, tags, content_hash, file_mtime, indexed_at";

// ============================ KB CRUD ============================

/// 列出全部知识库，按 created_at 降序
pub async fn list_all(pool: &SqlitePool) -> AppResult<Vec<Kb>> {
    let sql = format!("SELECT {KB_COLS} FROM kb ORDER BY created_at DESC");
    let rows = sqlx::query_as::<_, KbRow>(&sql).fetch_all(pool).await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// 按层级查 KB（search_kb 确定检索范围用）：
/// - scope='global' → owner_id 传 None（匹配 owner_id IS NULL）
/// - scope='agent'  → owner_id 传 Some(agent_id)
pub async fn list_by_scope(
    pool: &SqlitePool,
    scope: &str,
    owner_id: Option<&str>,
) -> AppResult<Vec<Kb>> {
    let sql = format!(
        "SELECT {KB_COLS} FROM kb WHERE scope = ? AND owner_id IS ? ORDER BY created_at DESC"
    );
    let rows = sqlx::query_as::<_, KbRow>(&sql)
        .bind(scope)
        .bind(owner_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<Kb> {
    let sql = format!("SELECT {KB_COLS} FROM kb WHERE id = ?");
    let row = sqlx::query_as::<_, KbRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound {
            resource: "kb",
            id: id.to_string(),
        })?;
    Ok(row.into())
}

pub async fn create(pool: &SqlitePool, input: &NewKb) -> AppResult<Kb> {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    sqlx::query(
        "INSERT INTO kb (id, name, scope, owner_id, directory, enabled, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.id)
    .bind(&input.name)
    .bind(&input.scope)
    .bind(&input.owner_id)
    .bind(&input.directory)
    .bind(input.enabled as i32)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    get_by_id(pool, &input.id).await
}

pub async fn delete(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let affected = sqlx::query("DELETE FROM kb WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "kb",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 更新 KB（仅 name / enabled；directory 不支持改 —— 改动需删后重建，
/// 否则 watcher 监听目录与 DB 不一致）。
pub async fn update(pool: &SqlitePool, input: &UpdateKb) -> AppResult<Kb> {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let existing = get_by_id(pool, &input.id).await?;
    let name = input.name.as_ref().unwrap_or(&existing.name);
    let enabled = input.enabled.unwrap_or(existing.enabled);
    sqlx::query("UPDATE kb SET name = ?, enabled = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(enabled as i32)
        .bind(&now)
        .bind(&input.id)
        .execute(pool)
        .await?;
    get_by_id(pool, &input.id).await
}

// ============================ kb_document（文档索引） ============================

/// 列出某 KB 的全部文档，按 file_path
pub async fn list_documents(pool: &SqlitePool, kb_id: &str) -> AppResult<Vec<KbDocumentRow>> {
    let sql = format!("SELECT {DOC_COLS} FROM kb_document WHERE kb_id = ? ORDER BY file_path");
    let rows = sqlx::query_as::<_, KbDocumentRow>(&sql)
        .bind(kb_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 按 (kb_id, file_path) 取文档（增量索引：判断是否需要重建）
pub async fn get_document_by_path(
    pool: &SqlitePool,
    kb_id: &str,
    file_path: &str,
) -> AppResult<Option<KbDocumentRow>> {
    let sql = format!(
        "SELECT {DOC_COLS} FROM kb_document WHERE kb_id = ? AND file_path = ?"
    );
    let row = sqlx::query_as::<_, KbDocumentRow>(&sql)
        .bind(kb_id)
        .bind(file_path)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// upsert 文档索引（按 kb_id + file_path：存在则更新，不存在则插入）
#[allow(clippy::too_many_arguments)]
pub async fn upsert_document(
    pool: &SqlitePool,
    kb_id: &str,
    file_path: &str,
    title: &str,
    summary: &str,
    tags: &str,
    content_hash: Option<&str>,
    file_mtime: Option<&str>,
) -> AppResult<String> {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    if let Some(existing) = get_document_by_path(pool, kb_id, file_path).await? {
        sqlx::query(
            "UPDATE kb_document SET title=?, summary=?, tags=?, content_hash=?, file_mtime=?, indexed_at=? WHERE id=?",
        )
        .bind(title)
        .bind(summary)
        .bind(tags)
        .bind(content_hash)
        .bind(file_mtime)
        .bind(&now)
        .bind(&existing.id)
        .execute(pool)
        .await?;
        Ok(existing.id)
    } else {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO kb_document (id, kb_id, file_path, title, summary, tags, content_hash, file_mtime, indexed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(kb_id)
        .bind(file_path)
        .bind(title)
        .bind(summary)
        .bind(tags)
        .bind(content_hash)
        .bind(file_mtime)
        .bind(&now)
        .execute(pool)
        .await?;
        Ok(id)
    }
}

/// 删除某 KB 下指定文档（源文件被删时）
pub async fn delete_document(pool: &SqlitePool, kb_id: &str, file_path: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM kb_document WHERE kb_id = ? AND file_path = ?")
        .bind(kb_id)
        .bind(file_path)
        .execute(pool)
        .await?;
    Ok(())
}

/// 删除某 KB 的全部文档（重建索引前清空）
pub async fn delete_documents_by_kb(pool: &SqlitePool, kb_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM kb_document WHERE kb_id = ?")
        .bind(kb_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ============================ 检索（关键词匹配，v1 无向量） ============================

/// 在指定 KB 集合里按关键词检索文档。
/// 匹配 title/tags/summary/file_path（任一含关键词即命中），title 命中优先排序。
pub async fn search(
    pool: &SqlitePool,
    query: &str,
    kb_ids: &[String],
    limit: i64,
) -> AppResult<Vec<KbSearchHit>> {
    if kb_ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders = kb_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let pattern = format!("%{query}%");
    let sql = format!(
        "SELECT d.kb_id AS kb_id, k.name AS kb_name, d.file_path AS file_path, d.title AS title, d.summary AS summary
         FROM kb_document d JOIN kb k ON d.kb_id = k.id
         WHERE d.kb_id IN ({placeholders})
           AND (d.title LIKE ? OR d.summary LIKE ? OR d.tags LIKE ? OR d.file_path LIKE ?)
         ORDER BY (d.title LIKE ?) DESC, d.indexed_at DESC
         LIMIT ?"
    );
    let mut q = sqlx::query_as::<_, KbSearchHit>(&sql);
    for id in kb_ids {
        q = q.bind(id);
    }
    q = q.bind(&pattern).bind(&pattern).bind(&pattern).bind(&pattern);
    q = q.bind(&pattern); // title 命中优先
    q = q.bind(limit);
    let hits = q.fetch_all(pool).await?;
    Ok(hits)
}

// ============================ chunk 管理（RAG v2） ============================

/// 替换文档的所有 chunk（先删后插）。embedding 暂留 NULL，后续 embedding 阶段填充。
pub async fn upsert_chunks(
    pool: &SqlitePool,
    doc_id: &str,
    chunks: &[String],
) -> AppResult<()> {
    // 先删除旧 chunk
    sqlx::query("DELETE FROM kb_document_chunk WHERE doc_id = ?")
        .bind(doc_id)
        .execute(pool)
        .await?;

    // 插入新 chunk
    for (idx, content) in chunks.iter().enumerate() {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO kb_document_chunk (id, doc_id, chunk_idx, content) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(doc_id)
        .bind(idx as i64)
        .bind(content)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// 删除文档的所有 chunk（文档被删除时调用）
pub async fn delete_chunks_by_doc(pool: &SqlitePool, doc_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM kb_document_chunk WHERE doc_id = ?")
        .bind(doc_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// chunk 级关键词搜索（比文档级更精确，返回命中的 chunk 内容 + 来源文档信息）
#[derive(sqlx::FromRow)]
pub struct ChunkSearchHit {
    pub doc_id: String,
    pub kb_name: String,
    pub file_path: String,
    pub title: String,
    pub chunk_idx: i64,
    pub content: String,
}

pub async fn search_chunks(
    pool: &SqlitePool,
    query: &str,
    kb_ids: &[String],
    limit: i64,
) -> AppResult<Vec<ChunkSearchHit>> {
    if kb_ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders = kb_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let pattern = format!("%{query}%");
    let sql = format!(
        "SELECT c.doc_id, k.name AS kb_name, d.file_path, d.title, c.chunk_idx, c.content
         FROM kb_document_chunk c
         JOIN kb_document d ON c.doc_id = d.id
         JOIN kb k ON d.kb_id = k.id
         WHERE d.kb_id IN ({placeholders})
           AND c.content LIKE ?
         ORDER BY (d.title LIKE ?) DESC, c.chunk_idx ASC
         LIMIT ?"
    );
    let mut q = sqlx::query_as::<_, ChunkSearchHit>(&sql);
    for id in kb_ids {
        q = q.bind(id);
    }
    q = q.bind(&pattern).bind(&pattern).bind(limit);
    q.fetch_all(pool).await.map_err(Into::into)
}

// ============================ embedding 向量（RAG v2） ============================

/// embedding 向量 → little-endian bytes（存 BLOB）
pub fn embedding_to_bytes(vec: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vec.len() * 4);
    for f in vec {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

/// little-endian bytes → embedding 向量
pub fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// 加载指定 KB 范围内所有 chunk（含 embedding 向量），供向量检索用。
/// 返回 (chunk_id, doc_id, title, file_path, content, embedding_bytes)
#[derive(sqlx::FromRow)]
pub struct ChunkWithEmbedding {
    pub id: String,
    pub doc_id: String,
    pub title: String,
    pub file_path: String,
    pub content: String,
    pub embedding: Option<Vec<u8>>,
}

pub async fn load_chunks_for_vector_search(
    pool: &SqlitePool,
    kb_ids: &[String],
) -> AppResult<Vec<ChunkWithEmbedding>> {
    if kb_ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders = kb_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT c.id, c.doc_id, d.title, d.file_path, c.content, c.embedding
         FROM kb_document_chunk c
         JOIN kb_document d ON c.doc_id = d.id
         WHERE d.kb_id IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, ChunkWithEmbedding>(&sql);
    for id in kb_ids {
        q = q.bind(id);
    }
    q.fetch_all(pool).await.map_err(Into::into)
}

/// 更新 chunk 的 embedding 向量
pub async fn update_chunk_embedding(
    pool: &SqlitePool,
    chunk_id: &str,
    embedding: &[u8],
) -> AppResult<()> {
    sqlx::query("UPDATE kb_document_chunk SET embedding = ? WHERE id = ?")
        .bind(embedding)
        .bind(chunk_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    fn new_kb(id: &str, name: &str) -> NewKb {
        NewKb {
            id: id.into(),
            name: name.into(),
            scope: "global".into(),
            owner_id: None,
            directory: format!("/tmp/{id}"),
            enabled: true,
        }
    }

    #[tokio::test]
    async fn update_changes_provided_fields() {
        let pool = fresh_pool().await;
        create(&pool, &new_kb("k1", "原名称")).await.unwrap();

        let updated = update(
            &pool,
            &UpdateKb {
                id: "k1".into(),
                name: Some("新名称".into()),
                enabled: Some(false),
            },
        )
        .await
        .unwrap();

        assert_eq!(updated.name, "新名称");
        assert!(!updated.enabled);
        // scope / directory 不被 update 触及
        assert_eq!(updated.scope, "global");
        assert_eq!(updated.directory, "/tmp/k1");
    }

    #[tokio::test]
    async fn update_preserves_unset_fields() {
        let pool = fresh_pool().await;
        create(&pool, &new_kb("k1", "原名")).await.unwrap();

        // 只改 name，enabled 传 None → 应保留原值 true
        let updated = update(
            &pool,
            &UpdateKb {
                id: "k1".into(),
                name: Some("改名".into()),
                enabled: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(updated.name, "改名");
        assert!(updated.enabled, "未提供的 enabled 应保留原值");
    }

    #[tokio::test]
    async fn update_nonexistent_returns_not_found() {
        let pool = fresh_pool().await;
        let err = update(
            &pool,
            &UpdateKb {
                id: "不存在".into(),
                name: Some("x".into()),
                enabled: None,
            },
        )
        .await;
        assert!(err.is_err());
    }
}
