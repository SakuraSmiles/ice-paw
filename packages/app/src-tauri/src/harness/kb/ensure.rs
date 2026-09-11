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
    let projects = match repo::project::list(pool).await {
        Ok(p) => p,
        Err(_) => return,
    };
    for project in &projects {
        ensure_project_context_dir(default_workspace, &project.id, &project.name);
    }
}

/// 为单个项目创建上下文目录 {workspace}/projects/{id}/ 并生成默认 project.md。
/// 幂等（已存在不覆盖用户内容）；返回目录路径，default_workspace 为 None → None。
///
/// 除了 boot 全量扫描（[`ensure_project_context_dirs`]），创建项目
/// （project_cmd::create_project）与读写命令（get/set_project_context）也会
/// 调它做 lazy ensure——新建项目不必等下次启动就有 project.md。
pub fn ensure_project_context_dir(
    default_workspace: Option<&str>,
    project_id: &str,
    project_name: &str,
) -> Option<PathBuf> {
    let root = default_workspace?;
    let dir = PathBuf::from(root).join("projects").join(project_id);
    if dir.exists() {
        return Some(dir);
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        // 返回路径而非 None：目录建失败与「无默认工作区」是两回事——
        // 后续读写会报出具体 fs 错误，不静默改道
        tracing::warn!(target: "ice_paw.kb", "创建项目上下文目录失败: {e}");
        return Some(dir);
    }
    // 生成默认 project.md
    let project_md = dir.join("project.md");
    let content = format!(
        "# {}\n\n\
         在此填写项目说明（技术栈、架构、业务背景等）。\n\
         此文件由 IcePaw 管理，位于 IcePaw 工作空间，不会进入项目源码目录。\n\
         修改后即时生效，无需重启。\n",
        project_name
    );
    let _ = std::fs::write(&project_md, content);
    tracing::info!(
        target: "ice_paw.kb",
        "已创建项目上下文目录: {}",
        dir.display()
    );
    Some(dir)
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
/// - **版本化更新（2026-09-11 起，取代旧「存在不动」）**：已落盘文件按内容
///   hash 判定——等于现行种子 → 跳过；等于某个历史种子（= 原样旧版，用户没
///   动过）→ **自动覆盖为新版**（跟随安装版本更新，多机零手工）；两者都不是
///   （= 用户自己改过）→ 永不覆盖。删掉文件下次启动补回最新版（重置入口保留）。
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
    let docs: &[HelpDoc] = &[
        HelpDoc::new(
            "getting-started.md",
            include_str!("../../../resources/help/getting-started.md"),
            &["c4621fa0fd5f455d", "f35f879f5c3b5775"],
        ),
        HelpDoc::new(
            "configure-embedding.md",
            include_str!("../../../resources/help/configure-embedding.md"),
            &["6f812dd412a4d370"],
        ),
        HelpDoc::new(
            "configure-tools.md",
            include_str!("../../../resources/help/configure-tools.md"),
            &["23f3d53fe102bfa8", "f4798568bba0cbb7"],
        ),
        HelpDoc::new(
            "agent-yaml.md",
            include_str!("../../../resources/help/agent-yaml.md"),
            &[
                "5d560488efbbfd10",
                "b9720ea052a457ca",
                "cf55e4c2448608cf",
                "d5555b6eb11a85cb",
            ],
        ),
        HelpDoc::new(
            "project-workspace.md",
            include_str!("../../../resources/help/project-workspace.md"),
            &["3c399afa06d8b73c"],
        ),
        HelpDoc::new("faq.md", include_str!("../../../resources/help/faq.md"), &[
            "29b185fb67391af7",
        ]),
        // ---- 2026-09-11 全面更新批：新增六篇（首发，known_hashes 为空）----
        HelpDoc::new(
            "model-profiles.md",
            include_str!("../../../resources/help/model-profiles.md"),
            &[],
        ),
        HelpDoc::new(
            "vision-images.md",
            include_str!("../../../resources/help/vision-images.md"),
            &[],
        ),
        HelpDoc::new(
            "word-documents.md",
            include_str!("../../../resources/help/word-documents.md"),
            &[],
        ),
        HelpDoc::new(
            "screen-read-and-share.md",
            include_str!("../../../resources/help/screen-read-and-share.md"),
            &[],
        ),
        HelpDoc::new(
            "multi-agent.md",
            include_str!("../../../resources/help/multi-agent.md"),
            &[],
        ),
        HelpDoc::new(
            "channel.md",
            include_str!("../../../resources/help/channel.md"),
            &[],
        ),
    ];
    let mut written = 0;
    let mut updated = 0;
    let mut kept = 0;
    for doc in docs {
        match sync_help_doc(&dir, doc) {
            HelpSyncOutcome::WriteNew => written += 1,
            HelpSyncOutcome::Upgraded => updated += 1,
            HelpSyncOutcome::Skipped => {}
            HelpSyncOutcome::KeptUserEdited => kept += 1,
        }
    }
    tracing::info!(
        target: "ice_paw.kb",
        "帮助文档目录 {}（新写入 {} 篇 / 随版本更新 {} 篇 / 用户改动保留 {} 篇）",
        dir.display(),
        written,
        updated,
        kept
    );
}

/// 一篇帮助种子：文件名 + 现行内容 + 历史发布版本的内容 hash 引导集。
struct HelpDoc {
    name: &'static str,
    content: &'static str,
    /// 本篇**历史发布过**的所有种子内容 hash（blake2b256 前 16 hex；现行版本
    /// 无需登记，运行时自算）。落盘文件 hash 命中集合 = 原样旧版（用户没动过）
    /// → 允许自动升级。
    ///
    /// ⚠️ 维护纪律：**每次修改种子内容后，把旧版内容的 hash 补进本数组**——
    /// 漏补的后果是既有安装的原样旧文件永远不自动更新（回退旧「存在不动」）。
    /// 历史多版全收（多收无害：用户恰好改回历史某版才误判，概率≈0）。
    known_hashes: &'static [&'static str],
}

impl HelpDoc {
    const fn new(
        name: &'static str,
        content: &'static str,
        known_hashes: &'static [&'static str],
    ) -> Self {
        Self {
            name,
            content,
            known_hashes,
        }
    }
}

/// [`sync_help_doc`] 的判定结果。
#[derive(Debug, PartialEq, Eq)]
enum HelpSyncOutcome {
    /// 文件不存在 → 写入现行版
    WriteNew,
    /// 原样历史版（hash 命中引导集）→ 已覆盖为现行版
    Upgraded,
    /// 已是现行版
    Skipped,
    /// 用户改动过（hash 两边都不匹配）→ 保留不覆盖
    KeptUserEdited,
}

/// 同步一篇帮助文档到目录（核心判定，纯逻辑便于测试）。
/// 写失败 warn 后返回 Skipped 计数语义由调用方日志兜底。
fn sync_help_doc(dir: &Path, doc: &HelpDoc) -> HelpSyncOutcome {
    let path = dir.join(doc.name);
    let action = match std::fs::read(&path) {
        Err(_) => HelpSyncOutcome::WriteNew,
        Ok(existing) => {
            let cur = help_content_hash(&existing);
            let latest = help_content_hash(doc.content.as_bytes());
            if cur == latest {
                HelpSyncOutcome::Skipped
            } else if doc.known_hashes.contains(&cur.as_str()) {
                HelpSyncOutcome::Upgraded
            } else {
                HelpSyncOutcome::KeptUserEdited
            }
        }
    };
    if matches!(action, HelpSyncOutcome::WriteNew | HelpSyncOutcome::Upgraded) {
        if let Err(e) = std::fs::write(&path, doc.content) {
            tracing::warn!(
                target: "ice_paw.kb",
                "写入帮助文档失败 {}: {}",
                path.display(),
                e
            );
            // 写失败按原状计数（下次启动重试）
            return HelpSyncOutcome::Skipped;
        }
        if action == HelpSyncOutcome::Upgraded {
            tracing::info!(
                target: "ice_paw.kb",
                "帮助文档 {} 为原样旧版，已随版本自动更新",
                doc.name
            );
        }
    } else if action == HelpSyncOutcome::KeptUserEdited {
        tracing::info!(
            target: "ice_paw.kb",
            "帮助文档 {} 检测到用户改动，保留不覆盖（如需恢复出厂版可删除该文件）",
            doc.name
        );
    }
    action
}

/// 内容 hash：blake2b-256 前 8 字节 hex（16 字符）。碰撞面 2^64，对「区分种子
/// 版本 vs 用户改动」的判定足够；非安全边界（恶意构造等同 hash 无收益）。
/// （crate 未导出 Blake2b256 别名，按 crypto.rs 同款自建——U32 = 32 字节输出）
fn help_content_hash(bytes: &[u8]) -> String {
    use blake2::digest::consts::U32;
    use blake2::digest::Digest;
    type Blake2b256 = blake2::Blake2b<U32>;
    let digest = Blake2b256::digest(bytes);
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// 推导 agent 的 workspace 根：优先用 agent 自己的 workspace_path，
/// 为空则回退 `<default_workspace>/agents/<agent_id>`。两者都无 → None。
pub fn agent_workspace_root(
    agent_workspace: Option<&str>,
    default_workspace: Option<&str>,
    agent_id: &str,
) -> Option<String> {
    agent_workspace.map(String::from).or_else(|| {
        default_workspace
            .map(|d| format!("{}/agents/{}", d.trim_end_matches(['/', '\\']), agent_id))
    })
}

/// 为单个 Agent 确保 KB 行存在（创建 Agent 时调用，无需重启）。
///
/// 注意：KB **行**确实无需重启即可创建，但「文件监听」是 App 启动时一次性注册的，
/// 运行期新建的 agent 目录不会被 watcher 监听。因此这里额外做一次初始索引，让目录
/// 里已有的文件（迁移/手动放入/之前 save_to_kb 写入的）立即进 `kb_document` 表、
/// UI 列表与 search_kb 可见。后续若用户继续用 save_to_kb 写入，工具内部也会内联索引。
pub(crate) async fn ensure_agent_kb(
    app: Option<&tauri::AppHandle>,
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
        return;
    }

    // 触发一次初始索引（后台，不阻塞 agent 创建流程）。查行失败/查不到都
    // warn 披露——静默跳过会让「建了 agent 但 KB 目录里的存量文件搜不到」
    // 无迹可循（ensure_kb_row 刚跑完还查不到行本身就是异常信号）。
    match repo::kb::list_by_scope(pool, "agent", Some(agent_id)).await {
        Ok(kbs) => {
            if let Some(kb) = kbs.into_iter().next() {
                let pool = pool.clone();
                let app = app.cloned();
                let kb_id = kb.id.clone();
                let dir = dir.clone();
                tokio::spawn(async move {
                    match super::indexer::index_directory(app.as_ref(), &pool, &kb_id, &dir).await {
                        Ok(stats) => tracing::info!(
                            target: "ice_paw.kb",
                            "新建 agent KB 初始索引完成 kb={} indexed={} skipped={}",
                            kb_id, stats.indexed, stats.skipped
                        ),
                        Err(e) => {
                            tracing::warn!(target: "ice_paw.kb", "新建 agent KB 初始索引失败 kb={}: {e}", kb_id)
                        }
                    }
                });
            } else {
                tracing::warn!(
                    target: "ice_paw.kb",
                    "agent {agent_id} 约定 KB 行查询为空——初始索引未触发，请排查 ensure_kb_row 是否生效"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                target: "ice_paw.kb",
                "agent {agent_id} 约定 KB 行查询失败——初始索引未触发: {e}"
            );
        }
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
        let dir =
            std::env::temp_dir().join(format!("ice-paw-help-test-{}-{}", std::process::id(), n));
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
        "model-profiles.md",
        "vision-images.md",
        "word-documents.md",
        "screen-read-and-share.md",
        "multi-agent.md",
        "channel.md",
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
        // 重新运行 → 删掉的文件补回最新版（重置入口）
        ensure_help_docs(ws.to_str().unwrap());
        assert!(target.exists(), "删掉的文件下次运行应补回");
        let _ = std::fs::remove_dir_all(&ws);
    }

    // ===== 版本化更新：sync_help_doc 四分支 =====

    /// 测试用 HelpDoc（known_hashes 需 &'static，运行时算的 hash 用 Box::leak 固化）。
    fn test_doc(name: &str, old_content: &str) -> HelpDoc {
        let old_hash = help_content_hash(old_content.as_bytes());
        HelpDoc {
            name: Box::leak(name.to_string().into_boxed_str()),
            content: Box::leak("NEW CONTENT v2".to_string().into_boxed_str()),
            known_hashes: Box::leak(
                vec![Box::leak(old_hash.into_boxed_str()) as &'static str].into_boxed_slice(),
            ),
        }
    }

    #[test]
    fn sync_help_doc_writes_new_when_missing() {
        let ws = unique_temp_ws();
        let doc = test_doc("a.md", "OLD v1");
        assert_eq!(sync_help_doc(&ws, &doc), HelpSyncOutcome::WriteNew);
        assert_eq!(
            std::fs::read_to_string(ws.join("a.md")).unwrap(),
            doc.content
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn sync_help_doc_upgrades_stale_seed() {
        let ws = unique_temp_ws();
        let doc = test_doc("b.md", "OLD v1");
        // 落盘一个「原样旧版」（hash 在引导集里）
        std::fs::write(ws.join("b.md"), "OLD v1").unwrap();
        assert_eq!(sync_help_doc(&ws, &doc), HelpSyncOutcome::Upgraded);
        // 自动覆盖为现行版——跟随版本更新、多机零手工的核心路径
        assert_eq!(
            std::fs::read_to_string(ws.join("b.md")).unwrap(),
            doc.content
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn sync_help_doc_skips_when_already_latest() {
        let ws = unique_temp_ws();
        let doc = test_doc("c.md", "OLD v1");
        std::fs::write(ws.join("c.md"), doc.content).unwrap();
        assert_eq!(sync_help_doc(&ws, &doc), HelpSyncOutcome::Skipped);
        assert_eq!(
            std::fs::read_to_string(ws.join("c.md")).unwrap(),
            doc.content
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn sync_help_doc_keeps_user_edited() {
        let ws = unique_temp_ws();
        let doc = test_doc("d.md", "OLD v1");
        // 用户改动：既不等于新版也不等于任何历史种子 → 永不覆盖
        std::fs::write(ws.join("d.md"), "USER CUSTOM NOTES").unwrap();
        assert_eq!(sync_help_doc(&ws, &doc), HelpSyncOutcome::KeptUserEdited);
        assert_eq!(
            std::fs::read_to_string(ws.join("d.md")).unwrap(),
            "USER CUSTOM NOTES"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// hash 引导集包含多个历史版本时，任一命中都可升级（历史多版全收语义）。
    #[test]
    fn sync_help_doc_upgrades_any_known_historical_version() {
        let ws = unique_temp_ws();
        let v1 = "OLD v1";
        let v2 = "OLD v2";
        let hashes: Vec<&'static str> = vec![
            Box::leak(help_content_hash(v1.as_bytes()).into_boxed_str()),
            Box::leak(help_content_hash(v2.as_bytes()).into_boxed_str()),
        ];
        let doc = HelpDoc {
            name: "e.md",
            content: "NEW v3",
            known_hashes: Box::leak(hashes.into_boxed_slice()),
        };
        for stale in [v1, v2] {
            std::fs::write(ws.join("e.md"), stale).unwrap();
            assert_eq!(sync_help_doc(&ws, &doc), HelpSyncOutcome::Upgraded);
            assert_eq!(std::fs::read_to_string(ws.join("e.md")).unwrap(), "NEW v3");
        }
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn ensure_project_context_dir_creates_template_and_idempotent() {
        let ws = unique_temp_ws();
        let dir = ensure_project_context_dir(Some(ws.to_str().unwrap()), "p9", "测试项目").unwrap();
        assert_eq!(dir, ws.join("projects").join("p9"));
        let md = std::fs::read_to_string(dir.join("project.md")).unwrap();
        assert!(md.contains("# 测试项目"));
        assert!(md.contains("技术栈"));

        // 幂等：已存在不重建、不覆盖
        std::fs::write(dir.join("project.md"), "用户内容").unwrap();
        ensure_project_context_dir(Some(ws.to_str().unwrap()), "p9", "测试项目").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("project.md")).unwrap(),
            "用户内容"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn ensure_project_context_dir_none_without_workspace() {
        assert!(ensure_project_context_dir(None, "p1", "P").is_none());
    }
}
