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
        // 内置产品帮助文档：种子到全局 KB 的 help/ 子目录，随全局 KB 一起被索引，
        // 所有 agent 都能 search_kb 检索到（自服务帮助）。
        ensure_help_docs(root);
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

    // 项目上下文目录（project.md / conventions.md 存放处）
    ensure_project_context_dirs(pool, default_ws.as_deref()).await;

    Ok(())
}

/// 为每个项目创建上下文目录 {workspace}/projects/{id}/，
/// 并生成默认 project.md。由 IcePaw 管理，不污染用户项目源码目录。
pub async fn ensure_project_context_dirs(pool: &SqlitePool, default_workspace: Option<&str>) {
    let Some(root) = default_workspace else {
        return;
    };
    let projects = match repo::project::list(pool).await {
        Ok(p) => p,
        Err(_) => return,
    };
    for project in &projects {
        let dir = PathBuf::from(root).join("projects").join(&project.id);
        if dir.exists() {
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(target: "ice_paw.kb", "创建项目上下文目录失败: {e}");
            continue;
        }
        // 生成默认 project.md
        let project_md = dir.join("project.md");
        let content = format!(
            "# {}\n\n\
             在此填写项目说明（技术栈、架构、业务背景等）。\n\
             此文件由 IcePaw 管理，位于 IcePaw 工作空间，不会进入项目源码目录。\n\
             修改后即时生效，无需重启。\n",
            project.name
        );
        let _ = std::fs::write(&project_md, content);
        tracing::info!(
            target: "ice_paw.kb",
            "已创建项目上下文目录: {}",
            dir.display()
        );
    }
}

/// workspace 根 → knowledge 目录（约定）。
pub fn knowledge_dir(workspace_root: &str) -> PathBuf {
    PathBuf::from(workspace_root).join(KNOWLEDGE_DIR_NAME)
}

/// 内置产品帮助文档（.md）种子到全局 KB 的 `help/` 子目录。
///
/// - 内容用 `include_str!` 编译期内嵌，随 app 分发，无运行时外部文件依赖。
/// - 落在全局 KB 目录（`<ws>/knowledge/`）下的 `help/`，由 watcher 随全局 KB
///   一起索引 → 所有 agent 都能 `search_kb` 检索到（自服务帮助）。
/// - 文件已存在则跳过（不覆盖用户改动）；删掉文件下次启动自动补回（=重置入口）。
/// - 必须在全局 KB 目录已建好后调用（[`ensure_default_kbs`] 内、`ensure_kb_row`
///   之后）。失败仅 warn，不阻断启动。
fn ensure_help_docs(default_workspace: &str) {
    let dir = knowledge_dir(default_workspace).join("help");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            target: "ice_paw.kb",
            "创建帮助文档目录失败 {}: {}",
            dir.display(),
            e
        );
        return;
    }
    // (文件名, 编译期内嵌的内容)
    let docs: &[(&str, &str)] = &[
        ("getting-started.md", include_str!("../../../resources/help/getting-started.md")),
        ("configure-embedding.md", include_str!("../../../resources/help/configure-embedding.md")),
        ("configure-tools.md", include_str!("../../../resources/help/configure-tools.md")),
        ("agent-yaml.md", include_str!("../../../resources/help/agent-yaml.md")),
        ("project-workspace.md", include_str!("../../../resources/help/project-workspace.md")),
        ("faq.md", include_str!("../../../resources/help/faq.md")),
    ];
    let mut written = 0;
    for (name, content) in docs {
        let path = dir.join(name);
        if path.exists() {
            continue;
        }
        if let Err(e) = std::fs::write(&path, content) {
            tracing::warn!(
                target: "ice_paw.kb",
                "写入帮助文档失败 {}: {}",
                path.display(),
                e
            );
        } else {
            written += 1;
        }
    }
    tracing::info!(
        target: "ice_paw.kb",
        "帮助文档目录 {}（本次新写入 {} 篇）",
        dir.display(),
        written
    );
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

// ==========================================================================
// 单元测试 — ensure_help_docs
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// 进程内唯一临时「workspace」目录。
    fn unique_temp_ws() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join(format!("ice-paw-help-test-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const EXPECTED_DOCS: &[&str] = &[
        "getting-started.md",
        "configure-embedding.md",
        "configure-tools.md",
        "agent-yaml.md",
        "project-workspace.md",
        "faq.md",
    ];

    #[test]
    fn ensure_help_docs_writes_all_docs_with_frontmatter() {
        let ws = unique_temp_ws();
        ensure_help_docs(ws.to_str().unwrap());
        let help_dir = ws.join("knowledge").join("help");
        for name in EXPECTED_DOCS {
            let f = help_dir.join(name);
            assert!(f.exists(), "应写入 {}", name);
            let content = std::fs::read_to_string(&f).unwrap();
            assert!(content.starts_with("---\n"), "{} 应有 frontmatter", name);
            assert!(content.contains("title:"), "{} 应含 title 字段", name);
        }
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn ensure_help_docs_idempotent_does_not_overwrite_user_edits() {
        let ws = unique_temp_ws();
        ensure_help_docs(ws.to_str().unwrap());
        let target = ws.join("knowledge").join("help").join("faq.md");
        // 模拟用户改动
        std::fs::write(&target, "USER EDIT\n").unwrap();
        // 再次运行
        ensure_help_docs(ws.to_str().unwrap());
        let content = std::fs::read_to_string(&target).unwrap();
        assert_eq!(content, "USER EDIT\n", "已存在的文件不应被覆盖");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn ensure_help_docs_recreates_deleted_file() {
        let ws = unique_temp_ws();
        ensure_help_docs(ws.to_str().unwrap());
        let target = ws.join("knowledge").join("help").join("faq.md");
        std::fs::remove_file(&target).unwrap();
        assert!(!target.exists());
        // 重新运行 → 删掉的文件补回（重置入口）
        ensure_help_docs(ws.to_str().unwrap());
        assert!(target.exists(), "删掉的文件下次运行应补回");
        let _ = std::fs::remove_dir_all(&ws);
    }
}
