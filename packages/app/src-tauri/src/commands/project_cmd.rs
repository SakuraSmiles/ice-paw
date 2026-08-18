//! 项目管理 Tauri Commands
//!
//! - 项目 CRUD + 成员管理
//! - 项目内会话查询/移动（复用 repo::conversation）
//! - 项目上下文（project.md / conventions.md）读写——注入链路见
//!   context/stages.rs 的 OsContextStage（本模块只管编辑入口，注入零改动）
//!
//! DB schema 已由 migration 13/14/21 建好，本模块补命令层。

use std::path::PathBuf;

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::State;
use uuid::Uuid;

use crate::db::models::{Conversation, NewProject, Project, ProjectRow, UpdateProject};
use crate::db::repo;
use crate::error::{AppError, AppResult};

/// 列出全部项目（含 agent 成员），两次查询代替逐项目 N+1。
#[tauri::command]
pub async fn list_projects(pool: State<'_, SqlitePool>) -> AppResult<Vec<Project>> {
    let rows = repo::project::list(pool.inner()).await?;
    let agents_map = repo::project::list_all_agents_grouped(pool.inner()).await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let agents = agents_map.get(&row.id).cloned().unwrap_or_default();
        result.push(Project { row, agents });
    }
    Ok(result)
}

/// 创建项目（含初始成员）
#[tauri::command]
pub async fn create_project(
    pool: State<'_, SqlitePool>,
    input: NewProject,
) -> AppResult<ProjectRow> {
    let id = Uuid::new_v4().to_string();
    let row = repo::project::create(pool.inner(), &input, &id).await?;
    // 上下文目录同步建好：新项目即刻可编辑 project.md（不等下次启动的
    // boot 全量 ensure——OsContextStage 只读不建，缺目录期间注入静默为空）
    context_dir_ensured(pool.inner(), &row).await;
    Ok(row)
}

/// 更新项目（partial update）
#[tauri::command]
pub async fn update_project(
    pool: State<'_, SqlitePool>,
    input: UpdateProject,
) -> AppResult<ProjectRow> {
    repo::project::update(pool.inner(), &input).await
}

/// 删除项目（CASCADE 删成员；conversations.project_id 自动 SET NULL）
#[tauri::command]
pub async fn delete_project(pool: State<'_, SqlitePool>, id: String) -> AppResult<()> {
    repo::project::delete(pool.inner(), &id).await
}

/// 批量更新排序
#[tauri::command]
pub async fn reorder_projects(pool: State<'_, SqlitePool>, ids: Vec<String>) -> AppResult<()> {
    repo::project::reorder(pool.inner(), &ids).await
}

/// 全量替换项目成员
#[tauri::command]
pub async fn set_project_agents(
    pool: State<'_, SqlitePool>,
    project_id: String,
    members: Vec<(String, String)>,
) -> AppResult<()> {
    repo::project::set_agents(pool.inner(), &project_id, &members).await
}

/// 添加单个成员
#[tauri::command]
pub async fn add_project_agent(
    pool: State<'_, SqlitePool>,
    project_id: String,
    agent_id: String,
    role: Option<String>,
) -> AppResult<()> {
    let r = role.as_deref().unwrap_or("member");
    repo::project::add_agent(pool.inner(), &project_id, &agent_id, r).await
}

/// 移除单个成员
#[tauri::command]
pub async fn remove_project_agent(
    pool: State<'_, SqlitePool>,
    project_id: String,
    agent_id: String,
) -> AppResult<()> {
    repo::project::remove_agent(pool.inner(), &project_id, &agent_id).await
}

/// 列出项目内的会话（project_id=null → 散落会话）
#[tauri::command]
pub async fn list_conversations_by_project(
    pool: State<'_, SqlitePool>,
    project_id: Option<String>,
) -> AppResult<Vec<Conversation>> {
    let rows = repo::conversation::list_by_project(pool.inner(), project_id.as_deref()).await?;
    Ok(rows.into_iter().map(|r| r.into()).collect())
}

/// 移动会话到项目（project_id=null → 移出项目变散落）
#[tauri::command]
pub async fn move_conversation_to_project(
    pool: State<'_, SqlitePool>,
    conversation_id: String,
    project_id: Option<String>,
) -> AppResult<()> {
    repo::conversation::move_to_project(pool.inner(), &conversation_id, project_id.as_deref()).await
}

/// 归档项目（软删除：从活跃列表收起，会话不动、不丢、不混入散落）
#[tauri::command]
pub async fn archive_project(pool: State<'_, SqlitePool>, id: String) -> AppResult<()> {
    repo::project::set_archived(pool.inner(), &id, true).await
}

/// 恢复归档项目（原样回到活跃列表，会话可见）
#[tauri::command]
pub async fn unarchive_project(pool: State<'_, SqlitePool>, id: String) -> AppResult<()> {
    repo::project::set_archived(pool.inner(), &id, false).await
}

/// 永久删除项目：delete_conversations=true 连同该项目会话一起删；
/// false 则会话转为散落（conversations.project_id ON DELETE SET NULL）。
#[tauri::command]
pub async fn permanent_delete_project(
    pool: State<'_, SqlitePool>,
    id: String,
    delete_conversations: bool,
) -> AppResult<()> {
    repo::project::permanent_delete(pool.inner(), &id, delete_conversations).await
}

// =========================================================================
// 项目上下文（project.md / conventions.md）读写
// =========================================================================

/// 上下文文件白名单——`set_project_context` 的 `file` 参数只认这两个字面量，
/// 杜绝路径穿越（file 参数来自前端，按不可信输入处理）。
const CONTEXT_FILES: &[&str] = &["project.md", "conventions.md"];

/// 读取项目上下文的返回。
#[derive(Serialize, Clone, Debug)]
pub struct ProjectContextOut {
    /// false = 未解析到默认工作区（get_all 失败的防御分支；正常启动必为 true，
    /// preferences::get_all 会自动初始化默认工作区）
    pub available: bool,
    /// 上下文目录绝对路径（「打开目录」入口用）
    pub dir: Option<String>,
    /// project.md 内容（目录刚建/读失败 → 默认模板或空串）
    pub project_md: String,
    /// conventions.md 内容（未编辑过 → 空串，OsContextStage 对空内容不注入）
    pub conventions_md: String,
}

#[tauri::command]
pub async fn get_project_context(
    pool: State<'_, SqlitePool>,
    project_id: String,
) -> AppResult<ProjectContextOut> {
    get_project_context_impl(pool.inner(), &project_id).await
}

#[tauri::command]
pub async fn set_project_context(
    pool: State<'_, SqlitePool>,
    project_id: String,
    file: String,
    content: String,
) -> AppResult<()> {
    set_project_context_impl(pool.inner(), &project_id, &file, &content).await
}

/// 用系统文件管理器打开项目的上下文目录（project.md / conventions.md 所在处）。
/// 走后端 opener Rust API（同 log_cmd::open_data_dir 先例）。
#[tauri::command]
pub async fn open_project_context_dir(
    pool: State<'_, SqlitePool>,
    project_id: String,
) -> AppResult<()> {
    let project = find_project(pool.inner(), &project_id).await?;
    let Some(dir) = context_dir_ensured(pool.inner(), &project).await else {
        return Err(AppError::Validation(
            "尚未解析到默认工作区，无法定位项目上下文目录".into(),
        ));
    };
    tauri_plugin_opener::open_path(&dir, None::<&str>)
        .map_err(|e| AppError::Internal(format!("打开项目上下文目录失败: {e}")))?;
    Ok(())
}

// ---- impl 层（命令壳只做 State 解包，逻辑在此便于测试）----

async fn find_project(pool: &SqlitePool, project_id: &str) -> AppResult<ProjectRow> {
    // get_by_id 对不存在/已删项目返回 Err（与 conversation::get_by_id 同语义）
    repo::project::get_by_id(pool, project_id)
        .await
        .map_err(|_| AppError::NotFound {
            resource: "project",
            id: project_id.to_string(),
        })
}

/// 解析项目上下文目录并 lazy ensure（建目录 + 默认 project.md）。
/// 返回 None = 未解析到默认工作区（防御分支，正常启动不会发生）。
async fn context_dir_ensured(pool: &SqlitePool, project: &ProjectRow) -> Option<PathBuf> {
    let prefs = repo::preferences::get_all(pool).await.ok()?;
    crate::harness::kb::ensure::ensure_project_context_dir(
        prefs.default_workspace_path.as_deref(),
        &project.id,
        &project.name,
    )
}

async fn get_project_context_impl(
    pool: &SqlitePool,
    project_id: &str,
) -> AppResult<ProjectContextOut> {
    let project = find_project(pool, project_id).await?;
    let Some(dir) = context_dir_ensured(pool, &project).await else {
        return Ok(ProjectContextOut {
            available: false,
            dir: None,
            project_md: String::new(),
            conventions_md: String::new(),
        });
    };
    let project_md =
        tokio::fs::read_to_string(dir.join("project.md")).await.unwrap_or_default();
    let conventions_md =
        tokio::fs::read_to_string(dir.join("conventions.md")).await.unwrap_or_default();
    Ok(ProjectContextOut {
        available: true,
        dir: Some(dir.display().to_string()),
        project_md,
        conventions_md,
    })
}

async fn set_project_context_impl(
    pool: &SqlitePool,
    project_id: &str,
    file: &str,
    content: &str,
) -> AppResult<()> {
    if !CONTEXT_FILES.contains(&file) {
        return Err(AppError::Validation(format!(
            "不支持的项目上下文文件：{file}（仅 project.md / conventions.md）"
        )));
    }
    let project = find_project(pool, project_id).await?;
    let Some(dir) = context_dir_ensured(pool, &project).await else {
        return Err(AppError::Validation(
            "尚未解析到默认工作区，无法保存项目上下文（project.md 存放在 IcePaw 工作区，不进项目源码目录）".into(),
        ));
    };
    // 原子写：同目录 .tmp → rename（半途失败不留下截断文件；先例 agent_yaml.rs）
    let path = dir.join(file);
    let tmp_path = dir.join(format!("{file}.tmp"));
    tokio::fs::write(&tmp_path, content).await?;
    tokio::fs::rename(&tmp_path, &path).await?;
    Ok(())
}

// =========================================================================
// 单元测试（in-memory SQLite + 临时目录）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        pool
    }

    /// 进程内唯一临时工作区目录（default_workspace_path 指向它）。
    fn unique_temp_ws() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "ice-paw-ctx-test-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn seed_project(pool: &SqlitePool, id: &str, name: &str) {
        repo::project::create(
            pool,
            &NewProject {
                name: name.into(),
                description: None,
                icon: None,
                workspace_path: None,
                theme_color: None,
                agent_ids: vec![],
            },
            id,
        )
        .await
        .expect("seed project");
    }

    #[tokio::test]
    async fn context_get_lazy_creates_dir_with_default_template() {
        let pool = test_pool().await;
        let ws = unique_temp_ws();
        repo::preferences::set(&pool, "default_workspace_path", &ws.to_string_lossy())
            .await
            .unwrap();
        // 项目先于偏好存在（= 新建项目后从未启动 boot ensure 的场景）：
        // get 应 lazy 建目录并给默认模板，不必等下次启动
        seed_project(&pool, "p1", "旗舰工作台").await;

        let out = get_project_context_impl(&pool, "p1").await.unwrap();
        assert!(out.available);
        assert!(out.dir.as_deref().unwrap().contains("projects"));
        assert!(out.project_md.contains("# 旗舰工作台"));
        assert!(out.project_md.contains("技术栈"));
        assert!(out.conventions_md.is_empty()); // conventions 不生成默认文件
        let dir = ws.join("projects").join("p1");
        assert!(dir.join("project.md").exists());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[tokio::test]
    async fn context_set_get_roundtrip_preserves_crlf_and_bom() {
        let pool = test_pool().await;
        let ws = unique_temp_ws();
        repo::preferences::set(&pool, "default_workspace_path", &ws.to_string_lossy())
            .await
            .unwrap();
        seed_project(&pool, "p1", "P").await;

        // CRLF + BOM：编辑器两端常见的字节形态必须原样保真
        let md = "﻿# 项目说明\r\n- Tauri v2\r\n- Rust 后端\r\n";
        let conv = "# 规范\r\n- 命名用英文\r\n";
        set_project_context_impl(&pool, "p1", "project.md", md)
            .await
            .unwrap();
        set_project_context_impl(&pool, "p1", "conventions.md", conv)
            .await
            .unwrap();

        let out = get_project_context_impl(&pool, "p1").await.unwrap();
        assert_eq!(out.project_md, md);
        assert_eq!(out.conventions_md, conv);

        // 原子写不留 .tmp 残留
        let dir = ws.join("projects").join("p1");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "不应残留 .tmp 文件");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[tokio::test]
    async fn context_set_rejects_non_whitelisted_file() {
        let pool = test_pool().await;
        let ws = unique_temp_ws();
        repo::preferences::set(&pool, "default_workspace_path", &ws.to_string_lossy())
            .await
            .unwrap();
        seed_project(&pool, "p1", "P").await;

        for evil in ["../agent.yaml", "agent.yaml", "project.md.bak", "./project.md"] {
            let err = set_project_context_impl(&pool, "p1", evil, "x")
                .await
                .expect_err("白名单外文件应被拒");
            assert!(err.to_string().contains("不支持"), "{evil}: {err}");
        }
        // 白名单外文件不得被写到磁盘上
        assert!(!ws.join("agent.yaml").exists());
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[tokio::test]
    async fn context_missing_project_not_found() {
        let pool = test_pool().await;
        let ws = unique_temp_ws();
        repo::preferences::set(&pool, "default_workspace_path", &ws.to_string_lossy())
            .await
            .unwrap();
        let err = get_project_context_impl(&pool, "ghost")
            .await
            .expect_err("项目不存在应报 NotFound");
        assert!(matches!(err, AppError::NotFound { .. }));
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// 二次 get 不覆盖用户内容（ensure 幂等：目录已存在直接返回）。
    #[tokio::test]
    async fn context_get_does_not_overwrite_user_edits() {
        let pool = test_pool().await;
        let ws = unique_temp_ws();
        repo::preferences::set(&pool, "default_workspace_path", &ws.to_string_lossy())
            .await
            .unwrap();
        seed_project(&pool, "p1", "P").await;
        set_project_context_impl(&pool, "p1", "project.md", "用户写的说明")
            .await
            .unwrap();
        let out = get_project_context_impl(&pool, "p1").await.unwrap();
        assert_eq!(out.project_md, "用户写的说明");
        let _ = std::fs::remove_dir_all(&ws);
    }
}
