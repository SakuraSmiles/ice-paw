//! `kb` / `kb_document` 表的 SQL 操作（RAG v1，agentic 知识库）
//!
//! 三级归属：scope='agent'(owner_id=agent_id) / 'project'(owner_id=project_id) / 'global'(owner_id=NULL)。
//! v1 不含向量/切块，文档级关键词检索。

use std::collections::HashMap;

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
    let sql = format!("SELECT {DOC_COLS} FROM kb_document WHERE kb_id = ? AND file_path = ?");
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
    q = q
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern);
    q = q.bind(&pattern); // title 命中优先
    q = q.bind(limit);
    let hits = q.fetch_all(pool).await?;
    Ok(hits)
}

// ============================ chunk 管理（RAG v2） ============================

/// 替换文档的所有 chunk，**增量保留内容未变 chunk 的 embedding**。
///
/// 避免重建索引/文件变更时把向量全清白烧 API（v2 阻断②修复后的性能债）。策略：
/// 先读现有 (content → embedding) 建保留映射，DELETE + INSERT 时内容命中的复用旧
/// embedding，未命中的插 NULL。返回需要生成 embedding 的 `(chunk_id, content)` 列表，
/// 供 indexer 预生成（`ensure_chunks_embedded`）。
pub async fn upsert_chunks_incremental(
    pool: &SqlitePool,
    doc_id: &str,
    chunks: &[String],
) -> AppResult<Vec<(String, String)>> {
    use std::collections::HashMap;

    // 1. 读现有 chunk 的 (content → embedding)，按内容匹配保留向量（一次查询，避免 N+1）
    let existing: Vec<(String, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT content, embedding FROM kb_document_chunk WHERE doc_id = ?")
            .bind(doc_id)
            .fetch_all(pool)
            .await?;
    let mut kept: HashMap<String, Option<Vec<u8>>> = HashMap::new();
    for (content, emb) in existing {
        kept.entry(content).or_insert(emb);
    }

    // 2. DELETE + INSERT（命中的复用旧 embedding，未命中的插 NULL）
    sqlx::query("DELETE FROM kb_document_chunk WHERE doc_id = ?")
        .bind(doc_id)
        .execute(pool)
        .await?;

    let mut need_embed: Vec<(String, String)> = Vec::new();
    for (idx, content) in chunks.iter().enumerate() {
        let id = Uuid::new_v4().to_string();
        let emb = kept.get(content).and_then(|e| e.clone());
        sqlx::query(
            "INSERT INTO kb_document_chunk (id, doc_id, chunk_idx, content, embedding)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(doc_id)
        .bind(idx as i64)
        .bind(content)
        .bind(emb.as_ref())
        .execute(pool)
        .await?;
        if emb.is_none() {
            need_embed.push((id, content.clone()));
        }
    }
    Ok(need_embed)
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
    // 长度恒为 4 的倍数（embedding_to_bytes 对称写出）；as_chunks 免运行时余数检查
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

/// 加载指定 KB 范围内所有 chunk（含 embedding 向量），供向量检索用。
/// 返回 (chunk_id, doc_id, kb_id, title, file_path, content, embedding_bytes)
///
/// `kb_id` 供 KB 向量缓存（vector_cache）按 KB 分组（2026-09-04 质检 Q7）。
#[derive(sqlx::FromRow)]
pub struct ChunkWithEmbedding {
    pub id: String,
    pub doc_id: String,
    pub kb_id: String,
    pub title: String,
    pub file_path: String,
    pub summary: String,
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
        "SELECT c.id, c.doc_id, d.kb_id, d.title, d.file_path, d.summary, c.content, c.embedding
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

/// 一次 GROUP BY 查询返回每个 KB 的向量缓存签名（2026-09-04 质检 Q7）。
///
/// 签名 = `(COUNT(*), COUNT(embedding), MAX(rowid), SUM(LENGTH(content)))`：
/// - `COUNT(*)` 变化 → chunk 增删（文档重索引 / 增量 upsert 的 DELETE+INSERT）
/// - `COUNT(embedding)` 变化 → 向量补齐（懒生成 backfill）或清空（切换模型）
/// - `MAX(rowid)` 变化 → 同数量下的行替换兜底（增量 upsert 删旧插新）
/// - `SUM(LENGTH(content))` 变化 → **内容变化**——rowid 兜底不够：SQLite 在
///   DELETE 后回收 rowid（删最高行再插同数量，MAX 不变；单测实测踩中），且
///   变更 chunk 的向量会被 indexer 预生成立即回填，前三维全数复原 → 唯有
///   内容长度和能区分「换过内容的同构表」。LENGTH 只做字符计数不物化到
///   Rust，GROUP BY 扫行本就触页，无额外 IO。
///
/// 已知盲区（信任边界）：同维度向量**原地重写**（update_chunk_embedding 对
/// 非 NULL 行直接覆盖）不改变任何一维——仓内无此路径（ensure 只填 NULL、
/// 换模型走 clear 全清），仅直改 DB 可触发，与 read_route 指纹缓存同级信任。
///
/// 查询 KB 无 chunk → 不在返回 map 中（调用方以「缺项 = 未缓存/失效」处理；
/// 空 KB 的零签条目由缓存层显式存入）。
pub type KbSig = (i64, i64, i64, i64);

pub async fn chunk_signatures(
    pool: &SqlitePool,
    kb_ids: &[String],
) -> AppResult<HashMap<String, KbSig>> {
    let mut out = HashMap::new();
    if kb_ids.is_empty() {
        return Ok(out);
    }
    let placeholders = kb_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT d.kb_id, COUNT(*), COUNT(c.embedding), COALESCE(MAX(c.rowid), 0), \
                COALESCE(SUM(LENGTH(c.content)), 0)
         FROM kb_document_chunk c
         JOIN kb_document d ON c.doc_id = d.id
         WHERE d.kb_id IN ({placeholders})
         GROUP BY d.kb_id"
    );
    let mut q = sqlx::query_as::<_, (String, i64, i64, i64, i64)>(&sql);
    for id in kb_ids {
        q = q.bind(id);
    }
    for (kb_id, total, embedded, max_rowid, content_len) in q.fetch_all(pool).await? {
        out.insert(kb_id, (total, embedded, max_rowid, content_len));
    }
    Ok(out)
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

/// 统计某 KB 的 chunk 总数与已有 embedding 的数量（前端展示向量索引进度）。
///
/// 返回 `(total_chunks, embedded_chunks)`。`COUNT(c.embedding)` 忽略 NULL，
/// 正好对应"已生成向量"。
pub async fn kb_chunk_stats(pool: &SqlitePool, kb_id: &str) -> AppResult<(i64, i64)> {
    let row: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COUNT(c.embedding)
         FROM kb_document_chunk c
         JOIN kb_document d ON c.doc_id = d.id
         WHERE d.kb_id = ?",
    )
    .bind(kb_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// 清空**所有** KB 的 chunk embedding 向量（切换 embedding 模型时用：旧维度向量必须清，
/// 否则维度不匹配会被 top_k_recall 永久过滤）。返回受影响行数。
pub async fn clear_all_kb_embeddings(pool: &SqlitePool) -> AppResult<u64> {
    let result = sqlx::query("UPDATE kb_document_chunk SET embedding = NULL")
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
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

    /// 增量保留：内容未变的 chunk 保留 embedding，仅返回需要生成向量的。
    #[tokio::test]
    async fn upsert_chunks_incremental_preserves_unchanged_embeddings() {
        let pool = fresh_pool().await;
        create(&pool, &new_kb("k1", "t")).await.unwrap();
        let doc_id = upsert_document(&pool, "k1", "a.md", "T", "s", "[]", Some("h"), None)
            .await
            .unwrap();

        // 首次入库：3 chunk，无现有 → 全 NULL，返回 3 个 need_embed
        let need = upsert_chunks_incremental(
            &pool,
            &doc_id,
            &["alpha".into(), "beta".into(), "gamma".into()],
        )
        .await
        .unwrap();
        assert_eq!(need.len(), 3, "首次入库全部需生成: {need:?}");

        // 给 "alpha" 填上 embedding（模拟已预生成）
        let alpha_id = need.iter().find(|(_, c)| c == "alpha").unwrap().0.clone();
        update_chunk_embedding(&pool, &alpha_id, &embedding_to_bytes(&[0.1, 0.2]))
            .await
            .unwrap();

        // 再次 upsert 同内容：alpha 向量保留（不在 need），beta/gamma 仍 NULL
        let need2 = upsert_chunks_incremental(
            &pool,
            &doc_id,
            &["alpha".into(), "beta".into(), "gamma".into()],
        )
        .await
        .unwrap();
        assert_eq!(
            need2.len(),
            2,
            "alpha 保留向量，仅 beta/gamma 需生成: {need2:?}"
        );
        assert!(
            need2.iter().all(|(_, c)| c != "alpha"),
            "alpha 不应在 need_embed 里"
        );

        // 内容变化：alpha → alpha2（新内容 NULL），beta/gamma 仍无向量 → 全需生成
        let need3 = upsert_chunks_incremental(
            &pool,
            &doc_id,
            &["alpha2".into(), "beta".into(), "gamma".into()],
        )
        .await
        .unwrap();
        assert_eq!(
            need3.len(),
            3,
            "alpha2 新内容 + beta/gamma 仍无向量 → 全需生成: {need3:?}"
        );
    }

    /// kb_chunk_stats：统计 KB 的 chunk 总数 + 已有 embedding 数
    #[tokio::test]
    async fn kb_chunk_stats_counts_total_and_embedded() {
        let pool = fresh_pool().await;
        create(&pool, &new_kb("k1", "t")).await.unwrap();
        let doc_id = upsert_document(&pool, "k1", "a.md", "T", "s", "[]", Some("h"), None)
            .await
            .unwrap();
        let need = upsert_chunks_incremental(&pool, &doc_id, &["x".into(), "y".into(), "z".into()])
            .await
            .unwrap();
        // 给前 2 个填 embedding，第 3 个留空
        update_chunk_embedding(&pool, &need[0].0, &embedding_to_bytes(&[0.1]))
            .await
            .unwrap();
        update_chunk_embedding(&pool, &need[1].0, &embedding_to_bytes(&[0.2]))
            .await
            .unwrap();

        let (total, embedded) = kb_chunk_stats(&pool, "k1").await.unwrap();
        assert_eq!(total, 3, "3 个 chunk");
        assert_eq!(embedded, 2, "前 2 个有向量");
    }

    /// chunk_signatures：per-KB 四标量签名（Q7 向量缓存的失效判据）。
    /// 覆盖：缺项（无 chunk 的 KB 不在 map）/ 数量与向量数 / 向量补齐改变
    /// COUNT(embedding) / **rowid 回收陷阱**（同数量行替换、MAX(rowid) 都不变，
    /// 唯 SUM(LENGTH(content)) 能区分——单测实测踩中）/ kb_id 字段回读。
    #[tokio::test]
    async fn chunk_signatures_track_kb_state() {
        let pool = fresh_pool().await;
        create(&pool, &new_kb("k1", "t")).await.unwrap();
        create(&pool, &new_kb("k2", "t2")).await.unwrap();
        let doc1 = upsert_document(&pool, "k1", "a.md", "T", "s", "[]", Some("h"), None)
            .await
            .unwrap();

        // k2 无 chunk → 不在 map
        let sigs = chunk_signatures(&pool, &["k1".into(), "k2".into()])
            .await
            .unwrap();
        assert!(!sigs.contains_key("k2"), "无 chunk 的 KB 不在签名 map: {sigs:?}");

        let need = upsert_chunks_incremental(&pool, &doc1, &["x".into(), "yy".into()])
            .await
            .unwrap();
        let sigs = chunk_signatures(&pool, &["k1".into()]).await.unwrap();
        let (total, embedded, _, _) = sigs["k1"];
        assert_eq!((total, embedded), (2, 0), "2 chunk，0 向量");

        // 补向量 → COUNT(embedding) 变
        update_chunk_embedding(&pool, &need[0].0, &embedding_to_bytes(&[0.1]))
            .await
            .unwrap();
        update_chunk_embedding(&pool, &need[1].0, &embedding_to_bytes(&[0.2]))
            .await
            .unwrap();
        let sigs = chunk_signatures(&pool, &["k1".into()]).await.unwrap();
        assert_eq!(sigs["k1"].1, 2, "补满向量");

        // rowid 回收陷阱（生产场景同构）：此 doc 的行就是全表最高行，DELETE 后
        // 新行回收同号 rowid，MAX(rowid) 不变；向量被 indexer 预生成回填后
        // COUNT(embedding) 也复原——前三维全数相同，签名只能靠内容长度和区分
        let sig_before = sigs["k1"];
        assert_eq!(sig_before.2, 2, "前置：本 doc 行即全表最高 rowid");
        upsert_chunks_incremental(&pool, &doc1, &["zzz".into(), "yy".into()])
            .await
            .unwrap();
        let sigs_after = chunk_signatures(&pool, &["k1".into()]).await.unwrap();
        assert_eq!(sigs_after["k1"].0, 2, "仍 2 chunk");
        assert_eq!(
            sigs_after["k1"].2, sig_before.2,
            "rowid 回收：MAX 不变（正是陷阱所在）"
        );
        assert_ne!(
            sigs_after["k1"].3, sig_before.3,
            "内容长度和必须区分换过内容的同构表"
        );

        // load_chunks_for_vector_search 回读 kb_id（缓存按 KB 分组依赖）
        let chunks = load_chunks_for_vector_search(&pool, &["k1".into()])
            .await
            .unwrap();
        assert!(
            chunks.iter().all(|c| c.kb_id == "k1"),
            "kb_id 应随行回读: {:?}",
            chunks.iter().map(|c| c.kb_id.clone()).collect::<Vec<_>>()
        );
    }

    /// clear_all_kb_embeddings：清空所有 chunk 的 embedding
    #[tokio::test]
    async fn clear_all_kb_embeddings_nulls_all() {
        let pool = fresh_pool().await;
        create(&pool, &new_kb("k1", "t")).await.unwrap();
        let doc_id = upsert_document(&pool, "k1", "a.md", "T", "s", "[]", Some("h"), None)
            .await
            .unwrap();
        let need = upsert_chunks_incremental(&pool, &doc_id, &["x".into(), "y".into()])
            .await
            .unwrap();
        update_chunk_embedding(&pool, &need[0].0, &embedding_to_bytes(&[0.1]))
            .await
            .unwrap();
        update_chunk_embedding(&pool, &need[1].0, &embedding_to_bytes(&[0.2]))
            .await
            .unwrap();
        let (_, embedded) = kb_chunk_stats(&pool, "k1").await.unwrap();
        assert_eq!(embedded, 2, "前置：2 个有向量");

        let n = clear_all_kb_embeddings(&pool).await.unwrap();
        assert_eq!(n, 2, "清空 2 个 chunk 的向量");
        let (_, embedded2) = kb_chunk_stats(&pool, "k1").await.unwrap();
        assert_eq!(embedded2, 0, "清空后 embedded=0");
    }
}
