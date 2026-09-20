//! 定时任务命令（0.9.3）——设置页 CRUD / 手动触发 / 执行日志 / 档位预览。
//!
//! 执行路径（tick / boot 扫尾 / 手动触发）全在 `harness/scheduler.rs`——本文件
//! 只做用户手势侧出口；创建/更新即校验档位并算首个 next_run（坏配置当场暴露，
//! 不拖到调度期）。

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::db::models::{NewScheduledTask, ScheduledTaskRow, ScheduledTaskUpdate, TaskRunRow};
use crate::db::repo::task;
use crate::error::{AppError, AppResult};
use crate::harness::scheduler::{self, parse_spec, preview_next_runs};

/// 会话存在性检查（get_by_id 返回 Err(NotFound) 表示不存在——区分 DB 错误）。
async fn conv_exists(pool: &SqlitePool, id: &str) -> AppResult<bool> {
    match crate::db::repo::conversation::get_by_id(pool, id).await {
        Ok(_) => Ok(true),
        Err(AppError::NotFound { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}

/// 设置页列表视图：任务 + 最近一次执行（状态徽/摘要）。
#[derive(Debug, Serialize)]
pub struct TaskView {
    #[serde(flatten)]
    pub task: ScheduledTaskRow,
    pub last_run: Option<TaskRunRow>,
}

/// 侧栏快速入口视图：跨任务最近执行（含任务名）。
pub use crate::db::repo::task::RecentRunView;

/// 存库格式：本地时刻 → UTC 字符串（DB 惯例 UTC 存储）。
fn fmt_store(dt: chrono::DateTime<chrono::Local>) -> String {
    dt.with_timezone(&chrono::Utc).format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 校验档位并算首个 next_run（enabled 任务必非空；一次性过期时间拒收——
/// 用户想立即跑应该用「立即运行」而非创建一个过去的一次性任务）。
fn first_next_run(kind: &str, data: &str, enabled: bool) -> AppResult<Option<String>> {
    let spec = parse_spec(kind, data)?;
    if !enabled {
        return Ok(None);
    }
    match scheduler::next_run_after(&spec, chrono::Local::now()) {
        Some(next) => Ok(Some(fmt_store(next))),
        None => Err(AppError::Internal(
            "一次性任务的时间已在过去——请选未来时间，或创建后点「立即运行」".into(),
        )),
    }
}

#[tauri::command]
pub async fn list_scheduled_tasks(pool: State<'_, SqlitePool>) -> AppResult<Vec<TaskView>> {
    let tasks = task::list(&pool).await?;
    let mut views = Vec::with_capacity(tasks.len());
    for t in tasks {
        let last_run = task::list_runs(&pool, &t.id, 1).await?.into_iter().next();
        views.push(TaskView { task: t, last_run });
    }
    Ok(views)
}

#[tauri::command]
pub async fn create_scheduled_task(
    pool: State<'_, SqlitePool>,
    input: NewScheduledTask,
) -> AppResult<ScheduledTaskRow> {
    if input.name.trim().is_empty() {
        return Err(AppError::Internal("任务名称不能为空".into()));
    }
    if input.prompt.trim().is_empty() {
        return Err(AppError::Internal("任务提示词不能为空".into()));
    }
    // 转发目标须存在（错 id 拖到执行期才发现 = 假配置）
    if let Some(dst) = &input.deliver_to_conv_id {
        if !conv_exists(&pool, dst).await? {
            return Err(AppError::Internal(
                "转发目标会话不存在——请重新选择（会话可能已被删除）".into(),
            ));
        }
    }
    let enabled = input.enabled.unwrap_or(1) != 0;
    let next = first_next_run(&input.schedule_kind, &input.schedule_data, enabled)?;
    let id = Uuid::new_v4().to_string();
    task::create(&pool, &id, &input, next.as_deref()).await
}

#[tauri::command]
pub async fn update_scheduled_task(
    pool: State<'_, SqlitePool>,
    input: ScheduledTaskUpdate,
) -> AppResult<ScheduledTaskRow> {
    if let Some(name) = &input.name {
        if name.trim().is_empty() {
            return Err(AppError::Internal("任务名称不能为空".into()));
        }
    }
    if let Some(prompt) = &input.prompt {
        if prompt.trim().is_empty() {
            return Err(AppError::Internal("任务提示词不能为空".into()));
        }
    }
    if let Some(Some(dst)) = &input.deliver_to_conv_id {
        if !conv_exists(&pool, dst).await? {
            return Err(AppError::Internal(
                "转发目标会话不存在——请重新选择（会话可能已被删除）".into(),
            ));
        }
    }
    let existing = task::get_by_id(&pool, &input.id)
        .await?
        .ok_or_else(|| AppError::NotFound {
            resource: "scheduled_task",
            id: input.id.clone(),
        })?;
    task::update(&pool, &input).await?;

    // schedule 或 enabled 变更 → 重算 next_run（一次性过期同样拒收——除非禁用）
    let kind = input.schedule_kind.as_ref().unwrap_or(&existing.schedule_kind);
    let data = input.schedule_data.as_ref().unwrap_or(&existing.schedule_data);
    let enabled = match input.enabled {
        Some(v) => v != 0,
        None => existing.enabled != 0,
    };
    if input.schedule_kind.is_some()
        || input.schedule_data.is_some()
        || input.enabled.is_some()
    {
        let next = first_next_run(kind, data, enabled)?;
        task::schedule_next(&pool, &input.id, next.as_deref(), None).await?;
    }
    task::get_by_id(&pool, &input.id)
        .await?
        .ok_or_else(|| AppError::Internal("任务更新后读取失败".into()))
}

#[tauri::command]
pub async fn delete_scheduled_task(pool: State<'_, SqlitePool>, id: String) -> AppResult<()> {
    task::delete(&pool, &id).await
}

/// 手动触发：fire-and-forget（回合异步跑，UI 由执行记录/会话事件驱动刷新）。
#[tauri::command]
pub async fn run_scheduled_task_now(app: AppHandle, pool: State<'_, SqlitePool>, id: String) -> AppResult<()> {
    // 存在性当场校验（404 回显而非静默）
    task::get_by_id(&pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound {
            resource: "scheduled_task",
            id: id.clone(),
        })?;
    scheduler::run_now(app, pool.inner().clone(), id);
    Ok(())
}

#[tauri::command]
pub async fn list_task_runs(
    pool: State<'_, SqlitePool>,
    task_id: String,
    limit: Option<i64>,
) -> AppResult<Vec<TaskRunRow>> {
    task::list_runs(&pool, &task_id, limit.unwrap_or(50).clamp(1, 200)).await
}

/// 侧栏快速入口：跨任务最近执行（含任务名）。
#[tauri::command]
pub async fn list_recent_task_runs(
    pool: State<'_, SqlitePool>,
    limit: Option<i64>,
) -> AppResult<Vec<RecentRunView>> {
    task::recent_runs(&pool, limit.unwrap_or(3).clamp(1, 10)).await
}

/// 档位预览（表单实时显示未来运行时点——cron 逃生舱防写错）。
#[tauri::command]
pub async fn preview_schedule(kind: String, data: String) -> AppResult<Vec<String>> {
    let spec = parse_spec(&kind, &data)?;
    Ok(preview_next_runs(&spec, 3))
}
