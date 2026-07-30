//! `kb` / `kb_document` 表的 SQL 操作（RAG v1，agentic 知识库）
//!
//! 三级归属：scope='agent'(owner_id=agent_id) / 'project'(owner_id=project_id) / 'global'(owner_id=NULL)。
//! v1 不含向量/切块，文档级关键词检索。

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::models::{Kb, KbDocumentRow, KbRow, KbSearchHit, NewKb};
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
) -> AppResult<()> {
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
    }
    Ok(())
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
