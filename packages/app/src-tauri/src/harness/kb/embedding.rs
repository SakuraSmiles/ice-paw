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
use crate::db::repo;
use crate::db::repo::kb::{embedding_to_bytes, update_chunk_embedding, ChunkWithEmbedding};
use crate::error::AppResult;
use crate::harness::provider::embedding::EmbeddingBackend;

/// 从 [`UserPreferences`] 解析 embedding 后端配置 `(model, base_url, api_key)`。
///
/// `base_url` 缺省时按 `provider` 查 PROVIDERS 注册表 `openai_url` 档推导
///（表收敛 ModelProfile Phase 1：minimax 自此成为合法候选）。`model`、
/// `api_key` 任一缺失或 provider 无 OpenAI 兼容端点 → `None`（调用方回退
/// 关键词检索 / 跳过预生成）。
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
        None => crate::harness::provider::provider_openai_url(provider)?.to_string(),
    };
    Some((model, url, api_key))
}

/// 解析后的 embedding 后端配置（双路径归一，含归因标签）。
#[derive(Debug, Clone)]
pub struct ResolvedEmbedding {
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    /// 归因标签（日志用）：profile 路径 = 「模型配置「别名」」；旧格式 = 「旧配置」。
    pub source: String,
    /// profile 归因（状态监控用）：profile 引用 = Some(id)——embedding 成败回写
    /// 健康三列；旧格式 = None（无实体可记，跳过）。
    pub profile_id: Option<String>,
}

/// 组合层解析（ModelProfile Phase 1）：`embedding_profile_id = Some` → **profile
/// 引用**（查 model_profiles 行 + Stronghold 解 key；端点三层 = 行显式 > vault
/// 副本 > 注册表 `openai_url`）；`None` → **旧四键回落**（[`resolve_embedding_config`]
/// 原样，存量测试零改动）。
///
/// profile 路径需要 `app`（Stronghold 解密）；`app=None`（测试 / 早期启动）时该
/// 路径降级 `None`（warn）——既有「配置缺失跳过预生成 / 回退关键词检索」语义的
/// 延伸。无效引用（行没了 / key 缺 / 厂商无 OpenAI 兼容端点）warn + None 指路
/// 设置-模型。**刻意不加缓存**（与 [`crate::harness::modal::gather_vision_candidates`]
/// 同理）：换 Key / 换模型立即生效是实体化的核心卖点。
pub async fn resolve_embedding_backend(
    app: Option<&tauri::AppHandle>,
    pool: &SqlitePool,
) -> Option<ResolvedEmbedding> {
    let prefs = match repo::preferences::get_all(pool).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(target: "ice_paw.kb", err = %e, "读取 preferences 失败，embedding 配置不可用");
            return None;
        }
    };
    match prefs
        .embedding_profile_id
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        Some(pid) => match app {
            Some(app) => resolve_embedding_from_profile(app, pool, pid).await,
            None => {
                tracing::warn!(
                    target: "ice_paw.kb",
                    "语义检索已引用模型配置（{pid}）但无 AppHandle 可解 Stronghold，跳过 embedding"
                );
                None
            }
        },
        None => {
            resolve_embedding_config(&prefs).map(|(model, base_url, api_key)| ResolvedEmbedding {
                model,
                base_url,
                api_key,
                source: "旧配置".into(),
                profile_id: None,
            })
        }
    }
}

/// 单条 profile → embedding 配置。无效（行没了 / key 缺失 / 厂商无 OpenAI 兼容
/// 端点）返回 None + warn——与视觉链 `resolve_profile_credential` 同构。
async fn resolve_embedding_from_profile(
    app: &tauri::AppHandle,
    pool: &SqlitePool,
    profile_id: &str,
) -> Option<ResolvedEmbedding> {
    let row = match repo::model_profile::get_by_id(pool, profile_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                target: "ice_paw.kb",
                err = %e,
                "语义检索引用的模型配置 {profile_id} 不存在（已删？）——请到「设置-模型」重新选择"
            );
            return None;
        }
    };
    let (api_key, vault_url) = match crate::crypto::fetch_api_key(app, &row.api_key_ref) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(
                target: "ice_paw.kb",
                err = %e,
                alias = %row.alias,
                "语义检索引用的模型配置「{}」Key 缺失——请到「设置-模型」补 Key",
                row.alias
            );
            return None;
        }
    };
    // 端点三层：行显式 > vault 副本 > 注册表 openai_url（端点成对原则，不猜第三方）
    let explicit = row
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let base_url = explicit
        .or_else(|| {
            vault_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        })
        .or_else(|| crate::harness::provider::provider_openai_url(&row.provider).map(String::from));
    let Some(base_url) = base_url else {
        tracing::warn!(
            target: "ice_paw.kb",
            provider = %row.provider,
            alias = %row.alias,
            "语义检索引用的模型配置「{}」厂商无 OpenAI 兼容端点",
            row.alias
        );
        return None;
    };
    Some(ResolvedEmbedding {
        model: row.model,
        base_url,
        api_key,
        source: format!("模型配置「{}」", row.alias),
        // 状态监控归因：embedding 成败回写该 profile 的健康三列
        profile_id: Some(profile_id.to_string()),
    })
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
            kb_id: "k".into(),
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

    // ===== resolve_embedding_backend 双路径路由（ModelProfile Phase 1）=====
    // profile 腿的 Stronghold 解 key 需要真 AppHandle（测试不可得），此处覆盖
    // 路由层三态：旧四键回落 / 引用键 + app=None 降级 / 全空 None。

    #[tokio::test]
    async fn resolve_backend_falls_back_to_legacy_four_keys() {
        let pool = fresh_pool().await;
        // 模拟前端 JSON.stringify 存储（bridge.preferences.set 惯例，值带引号）
        repo::preferences::set(&pool, "embedding_provider", "\"glm\"")
            .await
            .unwrap();
        repo::preferences::set(&pool, "embedding_model", "\"embedding-3\"")
            .await
            .unwrap();
        repo::preferences::set(&pool, "embedding_api_key", "\"sk-test-xxx\"")
            .await
            .unwrap();

        let r = resolve_embedding_backend(None, &pool)
            .await
            .expect("无引用键 → 旧四键回落应解析成功");
        assert_eq!(r.model, "embedding-3");
        assert_eq!(r.base_url, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(r.api_key, "sk-test-xxx");
        assert_eq!(r.source, "旧配置");
    }

    #[tokio::test]
    async fn resolve_backend_profile_ref_without_app_degrades_to_none() {
        let pool = fresh_pool().await;
        // 引用键已落（Some=权威）但无 AppHandle → 降级 None，不回落旧四键
        repo::preferences::set(&pool, "embedding_profile_id", "\"mp-embed\"")
            .await
            .unwrap();
        repo::preferences::set(&pool, "embedding_provider", "\"glm\"")
            .await
            .unwrap();
        repo::preferences::set(&pool, "embedding_model", "\"embedding-3\"")
            .await
            .unwrap();
        repo::preferences::set(&pool, "embedding_api_key", "\"sk-test-xxx\"")
            .await
            .unwrap();

        assert!(
            resolve_embedding_backend(None, &pool).await.is_none(),
            "引用键 Some = 权威，app=None 应降级 None 而非回落旧四键"
        );
    }

    #[tokio::test]
    async fn resolve_backend_none_when_unconfigured() {
        let pool = fresh_pool().await;
        assert!(resolve_embedding_backend(None, &pool).await.is_none());
    }
}
