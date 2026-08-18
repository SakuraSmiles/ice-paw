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
// MA-2 项目台账 / 项目轨迹 / 概览（纯只读派生，repo 见 repo::project_ledger）
// =========================================================================

use crate::db::models::SessionEvent;
use crate::db::repo::project_ledger;
use crate::harness::event_log::TurnEndedPayload;

/// 任务台账行（前端终态推导见 utils/taskStatus.ts——running 由流式 overlay、
/// done/failed 由 termination 分桶，两侧注释互指）。
#[derive(Serialize, Clone, Debug)]
pub struct ProjectTaskOut {
    pub conv_id: String,
    pub title: String,
    /// 执行者（被委派的专家 agent；名字前端 agent store 解析，无 FK 语义）
    pub executor_agent_id: String,
    /// 发起者（NULL ≡ 用户发起）
    pub initiator_agent_id: Option<String>,
    /// 委派图边——父会话（跳转回父会话用）
    pub parent_conversation_id: Option<String>,
    pub started_at: String,
    pub updated_at: String,
    /// 最后一条 turn_ended 落库时间（无 = 进行中/中断）
    pub ended_at: Option<String>,
    pub termination: Option<String>,
    pub rounds: Option<u32>,
}

#[tauri::command]
pub async fn list_project_tasks(
    pool: State<'_, SqlitePool>,
    project_id: String,
) -> AppResult<Vec<ProjectTaskOut>> {
    list_project_tasks_impl(pool.inner(), &project_id).await
}

/// 项目事件流行：`SessionEvent` 同构 + 会话标注列（前端会话徽章）。
/// `#[serde(flatten)]` 使 JSON 形态与单会话事件完全一致，只是多两列。
#[derive(Serialize, Clone, Debug)]
pub struct ProjectEventOut {
    #[serde(flatten)]
    pub event: SessionEvent,
    pub session_title: String,
    pub session_kind: String,
}

/// `limit=Some` + `after_id` → 正向增量（live 追加，返回空 = 已追平）；
/// `limit=Some` → 尾部优先分页（`before_id` 游标向前翻）；`None` → 全量正序。
/// 语义与 `list_session_events` 对齐，游标从 per-conv seq 换成全局 id。
#[tauri::command]
pub async fn list_project_events(
    pool: State<'_, SqlitePool>,
    project_id: String,
    limit: Option<i64>,
    before_id: Option<i64>,
    after_id: Option<i64>,
) -> AppResult<Vec<ProjectEventOut>> {
    list_project_events_impl(pool.inner(), &project_id, limit, before_id, after_id).await
}

/// 概览统计（详情页统计卡 + 台账分桶 + 成员分布）。
#[derive(Serialize, Clone, Debug)]
pub struct ProjectOverviewOut {
    pub chat_conversations: i64,
    pub delegation_conversations: i64,
    pub messages: i64,
    pub tasks_total: i64,
    pub tasks_done: i64,
    pub tasks_failed: i64,
    pub tasks_ended_other: i64,
    pub last_activity_at: Option<String>,
    /// 成员消息占比（「成员分布」横条排行数据源；名字/模型前端解析）
    pub agent_shares: Vec<ProjectAgentShareOut>,
}

/// 成员消息占比行（repo `ProjectAgentShareRow` 的序列化形态）。
#[derive(Serialize, Clone, Debug)]
pub struct ProjectAgentShareOut {
    pub agent_id: String,
    pub messages: i64,
}

#[tauri::command]
pub async fn get_project_overview(
    pool: State<'_, SqlitePool>,
    project_id: String,
) -> AppResult<ProjectOverviewOut> {
    get_project_overview_impl(pool.inner(), &project_id).await
}

/// 台账任务终态三桶——done/failed 的权威分桶（与前端 utils/taskStatus.ts
/// 同款规则，注释互指）。词表外值归 failed（不猜：技术兜底，与 termLabels
/// 裸透原值一致；台账粒度只需三桶）。
pub(crate) enum TaskBucket {
    Done,
    Failed,
    EndedOther,
}

pub(crate) fn termination_bucket(t: &str) -> TaskBucket {
    // 正常完成判定与 delegate.rs is_normal_completion 同源（stop | end_turn）
    match t {
        "stop" | "end_turn" => TaskBucket::Done,
        // backfill 是历史补录（boot 扫尾给零事件旧会话合成的事件）——中性，
        // 不算 failed（与 termLabels isWarnTermination 同款豁免）
        "backfill" => TaskBucket::EndedOther,
        _ => TaskBucket::Failed,
    }
}

// ---- impl 层（命令壳只做 State 解包，逻辑在此便于测试）----

async fn list_project_tasks_impl(
    pool: &SqlitePool,
    project_id: &str,
) -> AppResult<Vec<ProjectTaskOut>> {
    find_project(pool, project_id).await?;
    let rows = project_ledger::list_project_tasks(pool, project_id).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            // payload 解析失败 warn 降级 None，不吞会话行（行本身仍要显示）
            let ended = r.ended_payload.as_deref().and_then(|p| {
                serde_json::from_str::<TurnEndedPayload>(p)
                    .map_err(|e| {
                        tracing::warn!(
                            conv = %r.id,
                            error = %e,
                            "turn_ended payload 解析失败，台账终态降级为未知"
                        )
                    })
                    .ok()
            });
            ProjectTaskOut {
                conv_id: r.id,
                title: r.title,
                executor_agent_id: r.agent_id,
                initiator_agent_id: r.initiator_agent_id,
                parent_conversation_id: r.parent_conversation_id,
                started_at: r.created_at,
                updated_at: r.updated_at,
                ended_at: r.ended_at,
                termination: ended.as_ref().map(|e| e.termination.clone()),
                rounds: ended.map(|e| e.rounds),
            }
        })
        .collect())
}

async fn list_project_events_impl(
    pool: &SqlitePool,
    project_id: &str,
    limit: Option<i64>,
    before_id: Option<i64>,
    after_id: Option<i64>,
) -> AppResult<Vec<ProjectEventOut>> {
    find_project(pool, project_id).await?;
    let rows = if let Some(n) = limit {
        if after_id.is_some() {
            project_ledger::list_project_events_after(pool, project_id, after_id, n).await?
        } else {
            project_ledger::list_project_events_tail(pool, project_id, before_id, n).await?
        }
    } else {
        // 全量正序 = after_id=0 的正向读取（与 list_session_events 的 None 分支同语义）
        project_ledger::list_project_events_after(pool, project_id, None, -1).await?
    };
    // payload parse 兜底（非法 JSON 降级字符串值，与 SessionEvent::from 同款）
    let mut events: Vec<ProjectEventOut> = rows
        .into_iter()
        .map(|r| ProjectEventOut {
            event: SessionEvent {
                id: r.id,
                session_id: r.session_id,
                seq: r.seq,
                kind: r.kind,
                actor: r.actor,
                turn_id: r.turn_id,
                message_id: r.message_id,
                payload: serde_json::from_str::<serde_json::Value>(&r.payload)
                    .unwrap_or(serde_json::Value::String(r.payload.clone())),
                created_at: r.created_at,
            },
            session_title: r.session_title,
            session_kind: r.session_kind,
        })
        .collect();
    // image_ref 原位水合（Phase 2B 不变式：ref 形态不得以非 Text 形态流出）
    let mut payloads: Vec<&mut serde_json::Value> =
        events.iter_mut().map(|e| &mut e.event.payload).collect();
    crate::commands::conversation_cmd::hydrate_image_refs_json(pool, &mut payloads).await;
    Ok(events)
}

async fn get_project_overview_impl(
    pool: &SqlitePool,
    project_id: &str,
) -> AppResult<ProjectOverviewOut> {
    find_project(pool, project_id).await?;
    let ov = project_ledger::get_project_overview(pool, project_id).await?;
    // 任务分桶复用台账查询结果（零重复 SQL）。payload 解析失败与无 turn_ended
    // 同归 None（不进三桶）——与 list_project_tasks_impl 的降级语义一致；
    // 进行中/中断也不进三桶：running 是前端流式 overlay，静态视角终止未落
    // = interrupted。前端可由 total - 三桶推得 open 数。
    let tasks = project_ledger::list_project_tasks(pool, project_id).await?;
    let mut done = 0i64;
    let mut failed = 0i64;
    let mut ended_other = 0i64;
    for t in &tasks {
        let term = t
            .ended_payload
            .as_deref()
            .and_then(|p| serde_json::from_str::<TurnEndedPayload>(p).ok())
            .map(|e| e.termination);
        match term.as_deref().map(termination_bucket) {
            Some(TaskBucket::Done) => done += 1,
            Some(TaskBucket::Failed) => failed += 1,
            Some(TaskBucket::EndedOther) => ended_other += 1,
            None => {}
        }
    }
    let agent_shares = project_ledger::list_project_agent_shares(pool, project_id)
        .await?
        .into_iter()
        .map(|r| ProjectAgentShareOut {
            agent_id: r.agent_id,
            messages: r.messages,
        })
        .collect();
    Ok(ProjectOverviewOut {
        chat_conversations: ov.chat_conversations,
        delegation_conversations: ov.delegation_conversations,
        messages: ov.messages,
        tasks_total: tasks.len() as i64,
        tasks_done: done,
        tasks_failed: failed,
        tasks_ended_other: ended_other,
        last_activity_at: ov.last_activity_at,
        agent_shares,
    })
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

    // ---- MA-2 台账 / 事件流 / 概览 ----

    async fn seed_agent_row(pool: &SqlitePool) {
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES ('a1', '专家', 'anthropic', 'claude-test', '', '', 0.7, 1024, '{}', 0, 0)",
        )
        .execute(pool)
        .await
        .expect("seed agent");
    }

    /// 会话 seed（updated_at INSERT 时写死——UPDATE 触发器会重置，见
    /// project_ledger 测试 helper 同款注释）。
    async fn seed_conv_row(
        pool: &SqlitePool,
        id: &str,
        kind: &str,
        project_id: Option<&str>,
        updated_at: &str,
    ) {
        sqlx::query(
            "INSERT INTO conversations (id, agent_id, title, kind, project_id, updated_at)
             VALUES (?, 'a1', ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(format!("conv {id}"))
        .bind(kind)
        .bind(project_id)
        .bind(updated_at)
        .execute(pool)
        .await
        .expect("seed conversation");
    }

    fn ended_payload(termination: &str, rounds: u32) -> String {
        format!(
            r#"{{"v":1,"termination":"{termination}","rounds":{rounds},"usage":null,"user_token_count":null}}"#
        )
    }

    /// termination 全词表 + 词表外兜底（与前端 utils/taskStatus.ts 同款规则，
    /// 两侧注释互指——此处是后端权威分桶）。
    #[test]
    fn termination_bucket_full_vocabulary() {
        use TaskBucket::*;
        for t in ["stop", "end_turn"] {
            assert!(matches!(termination_bucket(t), Done), "{t}");
        }
        for t in [
            "length",
            "max_tokens",
            "tool_use",
            "budget_exceeded",
            "stuck",
            "abort",
            "error",
            "interrupted",
        ] {
            assert!(matches!(termination_bucket(t), Failed), "{t}");
        }
        assert!(matches!(termination_bucket("backfill"), EndedOther));
        // 词表外（未来新增/脏数据）不猜，归 failed
        assert!(matches!(termination_bucket("who_dis"), Failed));
    }

    #[tokio::test]
    async fn list_project_tasks_impl_parses_and_degrades_bad_payload() {
        let pool = test_pool().await;
        seed_agent_row(&pool).await;
        seed_project(&pool, "p1", "P").await;

        seed_conv_row(&pool, "done", "delegation", Some("p1"), "2026-08-18 10:00:00").await;
        repo::session_event::append(
            &pool,
            "done",
            "turn_ended",
            "agent:a1",
            Some("t1"),
            None,
            &ended_payload("stop", 3),
        )
        .await
        .unwrap();
        // 损坏 payload：解析失败必须降级 None，不吞会话行
        seed_conv_row(&pool, "corrupt", "delegation", Some("p1"), "2026-08-18 11:00:00").await;
        repo::session_event::append(
            &pool,
            "corrupt",
            "turn_ended",
            "agent:a1",
            Some("t1"),
            None,
            r#"{"v":1,"termination":"stop""#, // 截断 JSON
        )
        .await
        .unwrap();
        // 进行中：有事件但无 turn_ended
        seed_conv_row(&pool, "running", "delegation", Some("p1"), "2026-08-18 12:00:00").await;
        repo::session_event::append(&pool, "running", "turn_context", "user", Some("t1"), None, "{}")
            .await
            .unwrap();

        let tasks = list_project_tasks_impl(&pool, "p1").await.unwrap();
        assert_eq!(tasks.len(), 3, "损坏 payload 不吞行");
        let by_id = |id: &str| tasks.iter().find(|t| t.conv_id == id).unwrap();

        let d = by_id("done");
        assert_eq!(d.termination.as_deref(), Some("stop"));
        assert_eq!(d.rounds, Some(3));
        assert!(d.ended_at.is_some());

        let c = by_id("corrupt");
        assert!(c.termination.is_none() && c.rounds.is_none(), "降级 None");
        assert!(c.ended_at.is_some(), "落库时间本身可用");

        let r = by_id("running");
        assert!(r.termination.is_none() && r.rounds.is_none() && r.ended_at.is_none());

        // 项目不存在 → NotFound（不返回空数组造成「无任务」误导）
        let err = list_project_tasks_impl(&pool, "ghost").await.unwrap_err();
        assert!(matches!(err, AppError::NotFound { .. }));
    }

    #[tokio::test]
    async fn list_project_events_impl_cursors_and_payload_parse() {
        let pool = test_pool().await;
        seed_agent_row(&pool).await;
        seed_project(&pool, "p1", "P").await;
        seed_conv_row(&pool, "a", "chat", Some("p1"), "2026-08-18 10:00:00").await;
        seed_conv_row(&pool, "b", "delegation", Some("p1"), "2026-08-18 10:00:00").await;

        for (conv, marker) in [("a", "m1"), ("b", "m2"), ("a", "m3")] {
            repo::session_event::append(
                &pool,
                conv,
                "user_message",
                "user",
                Some("t"),
                Some(marker),
                r#"{"text":"你好"}"#,
            )
            .await
            .unwrap();
        }

        // 尾部优先分页：最新 2 条 → 反转为全局 id 正序
        let tail = list_project_events_impl(&pool, "p1", Some(2), None, None)
            .await
            .unwrap();
        assert_eq!(
            tail.iter().map(|e| e.event.message_id.as_deref()).collect::<Vec<_>>(),
            vec![Some("m2"), Some("m3")]
        );
        // payload 已 parse 为对象 + 会话标注列挂上
        assert!(tail[0].event.payload.is_object(), "payload 服务端 parse");
        assert_eq!(tail[0].session_title, "conv b");
        assert_eq!(tail[0].session_kind, "delegation");
        assert_eq!(tail[1].session_kind, "chat");

        // 游标向前翻：m2 之前恰剩 m1
        let m2_id = tail[0].event.id;
        let earlier = list_project_events_impl(&pool, "p1", Some(2), Some(m2_id), None)
            .await
            .unwrap();
        assert_eq!(earlier.len(), 1);
        assert_eq!(earlier[0].event.message_id.as_deref(), Some("m1"));

        // 正向增量：m3 之后追平返回空
        let m3_id = tail[1].event.id;
        let inc = list_project_events_impl(&pool, "p1", Some(10), None, Some(m3_id))
            .await
            .unwrap();
        assert!(inc.is_empty());

        // 全量（limit=None）
        let full = list_project_events_impl(&pool, "p1", None, None, None)
            .await
            .unwrap();
        assert_eq!(full.len(), 3);

        // NotFound
        let err = list_project_events_impl(&pool, "ghost", None, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound { .. }));
    }

    #[tokio::test]
    async fn get_project_overview_impl_buckets_and_not_found() {
        let pool = test_pool().await;
        seed_agent_row(&pool).await;
        seed_project(&pool, "p1", "P").await;

        seed_conv_row(&pool, "chat1", "chat", Some("p1"), "2026-08-18 10:00:00").await;
        // done / failed / ended-other / 进行中 各一
        for (id, term) in [
            ("done", Some("stop")),
            ("failed", Some("abort")),
            ("backfill", Some("backfill")),
            ("open", None),
        ] {
            seed_conv_row(&pool, id, "delegation", Some("p1"), "2026-08-18 10:00:00").await;
            if let Some(t) = term {
                repo::session_event::append(
                    &pool,
                    id,
                    "turn_ended",
                    "agent:a1",
                    Some("t1"),
                    None,
                    &ended_payload(t, 1),
                )
                .await
                .unwrap();
            }
        }

        let ov = get_project_overview_impl(&pool, "p1").await.unwrap();
        assert_eq!(ov.chat_conversations, 1);
        assert_eq!(ov.delegation_conversations, 4);
        assert_eq!(ov.tasks_total, 4);
        assert_eq!(ov.tasks_done, 1);
        assert_eq!(ov.tasks_failed, 1);
        assert_eq!(ov.tasks_ended_other, 1);
        // open（进行中+损坏）不进三桶，前端由 total - 三桶推得
        assert!(ov.last_activity_at.is_some());

        let err = get_project_overview_impl(&pool, "ghost").await.unwrap_err();
        assert!(matches!(err, AppError::NotFound { .. }));
    }
}
