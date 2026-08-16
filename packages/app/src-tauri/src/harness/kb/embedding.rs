//! `harness::kb::embedding` — KB embedding 生命周期（生成 / 持久化 / 配置解析）
//!
//! 收敛 v2 向量检索的 embedding 逻辑，供摄入（indexer 入库预生成）与检索
//! （search_kb 兜底）复用：
//! - [`resolve_embedding_config`]：从 [`UserPreferences`] 解析 backend 配置 (model/url/key)
//! - [`ensure_chunks_embedded`]：对缺向量的 chunk 批量生成 + 持久化 + 回填内存
//!
//! 配置读取必须走 `get_all`（JSON 反序列化）：前端 `bridge.preferences.set` 用
//! `JSON.stringify` 存储，裸 query_scalar 会读到带引号串导致失效（v2 阻断①，96dba9f）。

use sqlx::SqlitePool;

use crate::db::models::UserPreferences;
use crate::db::repo::kb::{embedding_to_bytes, update_chunk_embedding, ChunkWithEmbedding};
use crate::error::AppResult;
use crate::harness::provider::embedding::EmbeddingBackend;

/// 从 [`UserPreferences`] 解析 embedding 后端配置 `(model, base_url, api_key)`。
///
/// `base_url` 缺省时按 `provider` 推导（openai / glm / deepseek）。`model`、`api_key`
/// 任一缺失或 provider 未知 → `None`（调用方回退关键词检索 / 跳过预生成）。
///
/// 抽成纯函数，便于单测「前端 JSON 存储能否被正确解析为 backend 配置」。
pub fn resolve_embedding_config(prefs: &UserPreferences) -> Option<(String, String, String)> {
    let model = prefs.embedding_model.clone()?;
    let provider = prefs.embedding_provider.as_deref()?;
    let api_key = prefs.embedding_api_key.clone()?;
    let url = match prefs
        .embedding_base_url
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        Some(u) => u.to_string(),
        None => match provider {
            "openai" => "https://api.openai.com".into(),
            "glm" => "https://open.bigmodel.cn/api/paas/v4".into(),
            "deepseek" => "https://api.deepseek.com".into(),
            _ => return None,
        },
    };
    Some((model, url, api_key))
}

/// 批量确保 `chunks` 中 `embedding=None` 的 chunk 生成向量并持久化。
///
/// 对每个缺失的 chunk：`backend.embed` → `update_chunk_embedding` 写库 →
/// **回填内存 `chunks` 的 embedding 字段**（调用方无需重新 load）。返回新生成数量。
///
/// 供 indexer（入库预生成）+ search_kb（兜底）复用。
pub async fn ensure_chunks_embedded(
    pool: &SqlitePool,
    chunks: &mut [ChunkWithEmbedding],
    backend: &impl EmbeddingBackend,
    api_key: &str,
) -> AppResult<usize> {
    // 收集缺向量的 chunk 索引（用索引才能回填 &mut 元素）
    let missing_idx: Vec<usize> = chunks
        .iter()
        .enumerate()
        .filter_map(|(i, c)| if c.embedding.is_none() { Some(i) } else { None })
        .collect();
    if missing_idx.is_empty() {
        return Ok(0);
    }

    let texts: Vec<&str> = missing_idx
        .iter()
        .map(|&i| chunks[i].content.as_str())
        .collect();
    let embeddings = backend.embed(texts, api_key).await?;

    for (&i, emb) in missing_idx.iter().zip(embeddings.iter()) {
        let bytes = embedding_to_bytes(emb);
        update_chunk_embedding(pool, &chunks[i].id, &bytes).await?;
        chunks[i].embedding = Some(bytes); // 回填，省一次 DB load
    }
    Ok(missing_idx.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::kb::ChunkWithEmbedding;
    use crate::harness::provider::embedding::NoopEmbeddingBackend;

    /// 建内存库 + 全迁移（update_chunk_embedding 需要表存在）
    async fn fresh_pool() -> sqlx::SqlitePool {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
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

    fn chunk(id: &str, content: &str, emb: Option<Vec<u8>>) -> ChunkWithEmbedding {
        ChunkWithEmbedding {
            id: id.into(),
            doc_id: "d".into(),
            title: "".into(),
            file_path: "".into(),
            summary: "".into(),
            content: content.into(),
            embedding: emb,
        }
    }

    #[tokio::test]
    async fn ensure_fills_missing_and_skips_present() {
        let pool = fresh_pool().await;
        // c2 已有向量（跳过），c1/c3 缺（生成）
        let mut chunks = vec![
            chunk("c1", "hello", None),
            chunk("c2", "world", Some(vec![1, 2, 3, 4])),
            chunk("c3", "rust", None),
        ];
        let n = ensure_chunks_embedded(&pool, &mut chunks, &NoopEmbeddingBackend, "key")
            .await
            .unwrap();
        assert_eq!(n, 2, "c1/c3 缺向量 → 生成 2 个");
        assert!(chunks[0].embedding.is_some(), "c1 应被回填");
        assert_eq!(chunks[1].embedding, Some(vec![1, 2, 3, 4]), "c2 保留不动");
        assert!(chunks[2].embedding.is_some(), "c3 应被回填");
    }

    #[tokio::test]
    async fn ensure_returns_zero_when_all_present() {
        let pool = fresh_pool().await;
        let mut chunks = vec![chunk("c1", "x", Some(vec![1, 2, 3, 4]))];
        let n = ensure_chunks_embedded(&pool, &mut chunks, &NoopEmbeddingBackend, "key")
            .await
            .unwrap();
        assert_eq!(n, 0, "全部已有 → 0");
    }

    #[tokio::test]
    async fn ensure_returns_zero_when_empty() {
        let pool = fresh_pool().await;
        let mut chunks: Vec<ChunkWithEmbedding> = vec![];
        let n = ensure_chunks_embedded(&pool, &mut chunks, &NoopEmbeddingBackend, "key")
            .await
            .unwrap();
        assert_eq!(n, 0, "空切片 → 0");
    }
}
