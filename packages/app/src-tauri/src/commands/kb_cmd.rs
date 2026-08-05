//! 知识库管理 Tauri Commands（RAG v1）
//!
//! 提供 KB 的 CRUD + 重建索引 + 列文档，供前端 KB 管理 UI 调用。
//! 检索本身不在这里 —— agent 通过 `search_kb` 工具按需检索（见 harness/mcp/kb_tool.rs）。

use std::path::Path;

use sqlx::SqlitePool;
use tauri::State;
use uuid::Uuid;

use crate::db::models::{CreateKbInput, Kb, KbDocumentRow, NewKb, UpdateKb};
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::kb::indexer::{index_directory, IndexStats};

/// 合法 scope 值
const SCOPES: &[&str] = &["agent", "project", "global"];

/// 列出全部知识库
#[tauri::command]
pub async fn list_kb(pool: State<'_, SqlitePool>) -> AppResult<Vec<Kb>> {
    repo::kb::list_all(pool.inner()).await
}

/// 创建知识库（id 由命令层生成；scope 做合法性校验）
#[tauri::command]
pub async fn create_kb(pool: State<'_, SqlitePool>, input: CreateKbInput) -> AppResult<Kb> {
    validate_scope(&input.scope)?;
    let new_kb = NewKb {
        id: Uuid::new_v4().to_string(),
        name: input.name,
        scope: input.scope,
        owner_id: input.owner_id,
        directory: input.directory,
        enabled: input.enabled,
    };
    repo::kb::create(pool.inner(), &new_kb).await
}

/// 更新知识库（仅 name / enabled；directory 改动需删后重建）
#[tauri::command]
pub async fn update_kb(pool: State<'_, SqlitePool>, input: UpdateKb) -> AppResult<Kb> {
    repo::kb::update(pool.inner(), &input).await
}

/// 删除知识库（kb_document 随之外键 CASCADE 删除）
#[tauri::command]
pub async fn delete_kb(pool: State<'_, SqlitePool>, id: String) -> AppResult<()> {
    repo::kb::delete(pool.inner(), &id).await
}

/// 重建某知识库的索引（手动触发全量增量扫描，返回本次统计）
#[tauri::command]
pub async fn reindex_kb(pool: State<'_, SqlitePool>, id: String) -> AppResult<IndexStats> {
    let kb = repo::kb::get_by_id(pool.inner(), &id).await?;
    index_directory(pool.inner(), &kb.id, Path::new(&kb.directory)).await
}

/// 列出某知识库的文档索引
#[tauri::command]
pub async fn list_kb_documents(
    pool: State<'_, SqlitePool>,
    kb_id: String,
) -> AppResult<Vec<KbDocumentRow>> {
    repo::kb::list_documents(pool.inner(), &kb_id).await
}

/// 某 KB 的统计（文档数 + chunk 向量进度），供前端展示可观测性
#[derive(Debug, Clone, serde::Serialize)]
pub struct KbStats {
    pub total_documents: usize,
    pub total_chunks: usize,
    pub embedded_chunks: usize,
}

/// 某 KB 的统计：文档数、chunk 总数、已生成向量的 chunk 数
#[tauri::command]
pub async fn get_kb_stats(pool: State<'_, SqlitePool>, kb_id: String) -> AppResult<KbStats> {
    let docs = repo::kb::list_documents(pool.inner(), &kb_id).await?;
    let (total_chunks, embedded_chunks) = repo::kb::kb_chunk_stats(pool.inner(), &kb_id).await?;
    Ok(KbStats {
        total_documents: docs.len(),
        total_chunks: total_chunks as usize,
        embedded_chunks: embedded_chunks as usize,
    })
}

/// 切换 embedding 模型前的健康检查：用**传入的新配置**（非 preferences，测的是未存的新配置）
/// embed 一条测试文本，验证 provider/model/key/base_url 有效。失败返回 Err。
#[tauri::command]
pub async fn test_embedding_config(
    provider: String,
    model: String,
    api_key: String,
    base_url: Option<String>,
) -> AppResult<()> {
    use crate::harness::provider::embedding::{EmbeddingBackend, OpenAiEmbeddingBackend};
    let url = match base_url.filter(|s| !s.is_empty()) {
        Some(u) => u,
        None => match provider.as_str() {
            "openai" => "https://api.openai.com".into(),
            "glm" => "https://open.bigmodel.cn/api/paas/v4".into(),
            "deepseek" => "https://api.deepseek.com".into(),
            _ => return Err(AppError::Validation(format!("未知 embedding provider: {provider}"))),
        },
    };
    let backend = OpenAiEmbeddingBackend::new(model, url);
    backend.embed(vec!["health check"], &api_key).await?;
    Ok(())
}

/// 切换 embedding 模型后的全量重建：清空所有旧维度向量 + 遍历所有 KB 重新生成。
///
/// 前置：新配置已存 preferences（前端先 test_embedding_config 通过 → saveEmbedding → 本命令）。
/// 同步执行（KB 少、chunk 几十，几秒~几十秒）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RebuildStats {
    pub kbs: usize,
    pub chunks: usize,
}

#[tauri::command]
pub async fn rebuild_all_embeddings(pool: State<'_, SqlitePool>) -> AppResult<RebuildStats> {
    use crate::harness::kb::embedding::resolve_embedding_config;

    // 新配置必须就绪（前端已 test 通过 + saveEmbedding）
    let prefs = repo::preferences::get_all(pool.inner()).await?;
    resolve_embedding_config(&prefs)
        .ok_or_else(|| AppError::Validation("embedding 配置缺失，无法重建".into()))?;

    // 1. 清空所有旧维度向量（index_directory 预生成见 embedding=NULL → 全部重新生成）
    repo::kb::clear_all_kb_embeddings(pool.inner()).await?;

    // 2. 全量重建：遍历所有 KB，index_directory 含阶段1 预生成
    let kbs = repo::kb::list_all(pool.inner()).await?;
    let mut total_chunks = 0usize;
    for kb in &kbs {
        let dir = std::path::Path::new(&kb.directory);
        if let Err(e) = crate::harness::kb::indexer::index_directory(pool.inner(), &kb.id, dir).await {
            tracing::warn!(target: "ice_paw.kb", "重建索引失败 kb={} err={}", kb.id, e);
        }
        let (chunks, _) = repo::kb::kb_chunk_stats(pool.inner(), &kb.id).await?;
        total_chunks += chunks as usize;
    }
    Ok(RebuildStats { kbs: kbs.len(), chunks: total_chunks })
}

/// 校验 scope 合法性
fn validate_scope(scope: &str) -> AppResult<()> {
    if SCOPES.contains(&scope) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "无效的 scope '{scope}'，必须是 agent / project / global 之一"
        )))
    }
}
