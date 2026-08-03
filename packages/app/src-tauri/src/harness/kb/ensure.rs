//! 约定 KB 的自动建立（RAG v1 架构修正：约定单库模型）
//!
//! KB 不再由用户手动创建，而是按级别**约定存在**。本模块在启动时确保：
//! - global KB：`<default_workspace_path>/knowledge`
//! - 每个 agent KB：`<agent.workspace_path>/knowledge`（workspace_path 为空则回退
//!   `<default_workspace_path>/agents/<id>/knowledge`）
//!
//! directory 由系统按约定推导（不让用户填），并自动创建目录。
//! `watcher::start` 先调用本模块建好 KB 行，再 `list_all` → watch → 索引。

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

use crate::db::models::NewKb;
use crate::db::repo;
use crate::error::AppResult;

/// 各级 KB 的内容目录名约定（挂在 workspace 根下）。
const KNOWLEDGE_DIR_NAME: &str = "knowledge";

/// 启动时确保所有「约定 KB」存在并建好目录。幂等。
///
/// 已存在（按 scope+owner_id）则保留不重建。directory 变更（agent 改了
/// workspace_path）暂不自动同步 —— 属边缘场景，删 KB 行重启即可重建。
pub async fn ensure_default_kbs(pool: &SqlitePool) -> AppResult<()> {
    let prefs = repo::preferences::get_all(pool).await?;
    let default_ws = prefs.default_workspace_path;

    // global KB
    if let Some(root) = default_ws.as_deref() {
        let dir = knowledge_dir(root);
        ensure_kb_row(pool, "global", None, "全局知识库", &dir).await?;
    }

    // 各 agent KB
    let agents = repo::agent::list(pool).await?;
    for agent in agents {
        let Some(root) = agent_workspace_root(
            agent.workspace_path.as_deref(),
            default_ws.as_deref(),
            &agent.id,
        ) else {
            continue;
        };
        let dir = knowledge_dir(&root);
        let name = format!("{} 的知识库", agent.name);
        ensure_kb_row(pool, "agent", Some(&agent.id), &name, &dir).await?;
    }
    Ok(())
}

/// workspace 根 → knowledge 目录（约定）。
pub fn knowledge_dir(workspace_root: &str) -> PathBuf {
    PathBuf::from(workspace_root).join(KNOWLEDGE_DIR_NAME)
}

/// 推导 agent 的 workspace 根：优先用 agent 自己的 workspace_path，
/// 为空则回退 `<default_workspace>/agents/<agent_id>`。两者都无 → None。
pub fn agent_workspace_root(
    agent_workspace: Option<&str>,
    default_workspace: Option<&str>,
    agent_id: &str,
) -> Option<String> {
    agent_workspace.map(String::from).or_else(|| {
        default_workspace.map(|d| {
            format!("{}/agents/{}", d.trim_end_matches(['/', '\\']), agent_id)
        })
    })
}

/// 为单个 Agent 确保 KB 行存在（创建 Agent 时调用，无需重启）。
pub(crate) async fn ensure_agent_kb(
    pool: &SqlitePool,
    agent_id: &str,
    agent_name: &str,
    agent_workspace: Option<&str>,
    default_workspace: Option<&str>,
) {
    let Some(root) = agent_workspace_root(agent_workspace, default_workspace, agent_id) else {
        return;
    };
    let dir = knowledge_dir(&root);
    let name = format!("{} 的知识库", agent_name);
    if let Err(e) = ensure_kb_row(pool, "agent", Some(agent_id), &name, &dir).await {
        tracing::warn!(target: "ice_paw.kb", "创建 Agent KB 失败: {e}");
    }
}

/// 确保 (scope, owner_id) 对应的 KB 行存在；不存在则建目录 + 建行。
/// 已存在则跳过。
async fn ensure_kb_row(
    pool: &SqlitePool,
    scope: &str,
    owner_id: Option<&str>,
    name: &str,
    directory: &Path,
) -> AppResult<()> {
    let existing = repo::kb::list_by_scope(pool, scope, owner_id).await?;
    if !existing.is_empty() {
        return Ok(());
    }

    let dir_str = directory.to_string_lossy().replace('\\', "/");
    // 建目录（失败仅 warn，不阻断 —— 后续写文件时也会建）
    if let Err(e) = std::fs::create_dir_all(directory) {
        tracing::warn!(
            target: "ice_paw.kb",
            "建 knowledge 目录失败 {}: {}",
            dir_str,
            e
        );
    }

    let id = uuid::Uuid::new_v4().to_string();
    repo::kb::create(
        pool,
        &NewKb {
            id,
            name: name.to_string(),
            scope: scope.to_string(),
            owner_id: owner_id.map(String::from),
            directory: dir_str,
            enabled: true,
        },
    )
    .await?;
    tracing::info!(
        target: "ice_paw.kb",
        "已建立约定 KB: scope={} owner={:?} dir={}",
        scope,
        owner_id,
        directory.display()
    );
    Ok(())
}
