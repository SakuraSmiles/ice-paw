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
