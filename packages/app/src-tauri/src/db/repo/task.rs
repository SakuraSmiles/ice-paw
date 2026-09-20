//! 定时任务数据层（migration 57）。
//!
//! next_run / 时间戳全部为本地时间字符串 'YYYY-MM-DD HH:MM:SS'——与
//! conversations 表惯例一致；due 判定用字符串字典序比较（同格式下等价时序）。

use serde::Serialize;
use sqlx::{FromRow, SqlitePool};

use crate::db::models::{NewScheduledTask, ScheduledTaskRow, ScheduledTaskUpdate, TaskRunRow};
use crate::error::{AppError, AppResult};

/// UTC 时间字符串（全 DB 惯例：UTC 存储，显示层 parseDbTime 转本地）。
pub fn utc_now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub async fn create(
    pool: &SqlitePool,
    id: &str,
    new_task: &NewScheduledTask,
    next_run: Option<&str>,
) -> AppResult<ScheduledTaskRow> {
    let miss_policy = new_task.miss_policy.as_deref().unwrap_or("run_once");
    let preauth = new_task.preauth.as_deref().unwrap_or("none");
    let enabled = new_task.enabled.unwrap_or(1);
    sqlx::query(
        "INSERT INTO scheduled_tasks (id, name, agent_id, schedule_kind, schedule_data, prompt, miss_policy, deliver_to_conv_id, preauth, enabled, next_run)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&new_task.name)
    .bind(&new_task.agent_id)
    .bind(&new_task.schedule_kind)
    .bind(&new_task.schedule_data)
    .bind(&new_task.prompt)
    .bind(miss_policy)
    .bind(&new_task.deliver_to_conv_id)
    .bind(preauth)
    .bind(enabled)
    .bind(next_run)
    .execute(pool)
    .await?;
    get_by_id(pool, id).await?
        .ok_or_else(|| AppError::Internal("定时任务创建后读取失败".into()))
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<Option<ScheduledTaskRow>> {
    let row = sqlx::query_as::<_, ScheduledTaskRow>(
        "SELECT * FROM scheduled_tasks WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 全量任务（设置页列表；按 next_run 升序，NULL 排尾）。
pub async fn list(pool: &SqlitePool) -> AppResult<Vec<ScheduledTaskRow>> {
    let rows = sqlx::query_as::<_, ScheduledTaskRow>(
        "SELECT * FROM scheduled_tasks ORDER BY enabled DESC, next_run IS NULL, next_run ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// due 扫描：启用且 next_run 非空且已到点（含错过的——boot 扫尾复用同查询）。
pub async fn due_tasks(pool: &SqlitePool, now: &str) -> AppResult<Vec<ScheduledTaskRow>> {
    let rows = sqlx::query_as::<_, ScheduledTaskRow>(
        "SELECT * FROM scheduled_tasks WHERE enabled = 1 AND next_run IS NOT NULL AND next_run <= ? ORDER BY next_run ASC",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 部分更新（None = 不改）。schedule 变更后由调用方重算 next_run 一并落。
pub async fn update(pool: &SqlitePool, upd: &ScheduledTaskUpdate) -> AppResult<()> {
    let affected = sqlx::query(
        "UPDATE scheduled_tasks SET
            name = COALESCE(?, name),
            agent_id = COALESCE(?, agent_id),
            schedule_kind = COALESCE(?, schedule_kind),
            schedule_data = COALESCE(?, schedule_data),
            prompt = COALESCE(?, prompt),
            miss_policy = COALESCE(?, miss_policy),
            deliver_to_conv_id = COALESCE(?, deliver_to_conv_id),
            preauth = COALESCE(?, preauth),
            enabled = COALESCE(?, enabled),
            updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(&upd.name)
    .bind(&upd.agent_id)
    .bind(&upd.schedule_kind)
    .bind(&upd.schedule_data)
    .bind(&upd.prompt)
    .bind(&upd.miss_policy)
    .bind(&upd.deliver_to_conv_id)
    .bind(&upd.preauth)
    .bind(upd.enabled)
    .bind(&upd.id)
    .execute(pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "scheduled_task",
            id: upd.id.clone(),
        });
    }
    Ok(())
}

/// 调度簿记：next_run（None = 一次性完成/禁用）+ last_run_at。
pub async fn schedule_next(
    pool: &SqlitePool,
    id: &str,
    next_run: Option<&str>,
    enabled: Option<i32>,
) -> AppResult<()> {
    let next_run = next_run.map(str::to_string);
    sqlx::query(
        "UPDATE scheduled_tasks
         SET next_run = ?, last_run_at = datetime('now'),
             enabled = COALESCE(?, enabled), updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(&next_run)
    .bind(enabled)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 删除任务（执行记录随任务清；载体会话是用户数据，保留）。
pub async fn delete(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM task_runs WHERE task_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let affected = sqlx::query("DELETE FROM scheduled_tasks WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "scheduled_task",
            id: id.to_string(),
        });
    }
    Ok(())
}

// =========================================================================
// 执行记录（task_runs）
// =========================================================================

pub async fn insert_run(
    pool: &SqlitePool,
    id: &str,
    task_id: &str,
    conv_id: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO task_runs (id, task_id, conv_id, status) VALUES (?, ?, ?, 'running')",
    )
    .bind(id)
    .bind(task_id)
    .bind(conv_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn finish_run(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    summary: Option<&str>,
    error: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE task_runs SET status = ?, summary = ?, error = ?,
            finished_at = datetime('now') WHERE id = ?",
    )
    .bind(status)
    .bind(summary)
    .bind(error)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 单任务执行历史（执行日志页）。
pub async fn list_runs(pool: &SqlitePool, task_id: &str, limit: i64) -> AppResult<Vec<TaskRunRow>> {
    let rows = sqlx::query_as::<_, TaskRunRow>(
        "SELECT * FROM task_runs WHERE task_id = ? ORDER BY started_at DESC, id DESC LIMIT ?",
    )
    .bind(task_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 跨任务最近执行（侧栏快速入口）：JOIN 任务名。
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct RecentRunView {
    pub run_id: String,
    pub task_id: String,
    pub task_name: String,
    pub status: String,
    pub summary: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

pub async fn recent_runs(pool: &SqlitePool, limit: i64) -> AppResult<Vec<RecentRunView>> {
    let rows = sqlx::query_as::<_, RecentRunView>(
        "SELECT r.id AS run_id, r.task_id, t.name AS task_name, r.status, r.summary,
                r.started_at, r.finished_at
         FROM task_runs r JOIN scheduled_tasks t ON t.id = r.task_id
         ORDER BY r.started_at DESC, r.id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
