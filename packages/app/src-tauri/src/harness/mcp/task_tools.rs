//! `list_scheduled_tasks` / `create_scheduled_task` / `delete_scheduled_task`
//! ——定时任务的 agent 入口（0.9.3）。
//!
//! 用户说「每天早上帮我检查 X」「定时提醒我 Y」时，agent 应直接建定时任务
//! 而不是让用户去设置页手点。**授权 = Always + 后治理**（2026-09-20 用户拍板，
//! `send_message_to_session` 同模式）：创建/删除不弹卡，任务建后在设置·定时
//! 任务页与侧栏快速入口对用户全量可见、可停可删；护栏全部继承（最小间隔
//! 10 分钟钳制、全局并发 1、预检失败推进 next 防重试风暴）。
//!
//! ## 注册边界（与 relay 同闸）
//!
//! 组装期按 `conv.kind == "chat"` 注册（session_runner）——delegation 子会话
//! 拿不到（持久化副作用不逃出委派沙箱）；PLATFORM_TOOLS 白名单补入
//! （enabled_tools 收窄不断定时任务能力）。频道暂不给（v1 边界，与 relay 一致，
//! 后续按需放开）。

use async_trait::async_trait;
use uuid::Uuid;

use crate::db::models::NewScheduledTask;
use crate::db::repo::{conversation, task};
use crate::error::{AppError, AppResult};
use crate::harness::scheduler;

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

const WEEKDAY_LABELS: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];

/// 调度人话摘要（与设置页 scheduleLabel 同口径——agent 汇报用）。
fn schedule_label(kind: &str, data_json: &str) -> String {
    match (
        kind,
        serde_json::from_str::<serde_json::Value>(data_json).ok(),
    ) {
        ("once", Some(d)) => format!("一次性 {}", d["at"].as_str().unwrap_or("?")),
        ("daily", Some(d)) => format!("每天 {}", d["time"].as_str().unwrap_or("?")),
        ("weekly", Some(d)) => {
            let days = d["weekdays"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|w| w.as_u64().and_then(|w| u8::try_from(w).ok()))
                        .map(|w| WEEKDAY_LABELS.get(w as usize).copied().unwrap_or("?"))
                        .collect::<String>()
                })
                .unwrap_or_else(|| "?".into());
            format!("每周{} {}", days, d["time"].as_str().unwrap_or("?"))
        }
        ("interval", Some(d)) => format!("每 {} 分钟", d["minutes"].as_u64().unwrap_or(0)),
        ("cron", Some(d)) => format!("cron：{}", d["expr"].as_str().unwrap_or("?")),
        _ => kind.to_string(),
    }
}

/// 工具创建参数 → (schedule_kind, schedule_data)。档位三选一（cron /
/// daily_time / interval_minutes），多给或都不给都拒（防歧义）。
fn compose_schedule(args: &CreateArgs) -> AppResult<(String, String)> {
    let mut picked = 0;
    let mut out: Option<(String, String)> = None;
    if let Some(cron) = args.cron.as_deref() {
        picked += 1;
        out = Some(("cron".into(), serde_json::json!({ "expr": cron }).to_string()));
    }
    if let Some(time) = args.daily_time.as_deref() {
        picked += 1;
        out = Some(("daily".into(), serde_json::json!({ "time": time }).to_string()));
    }
    if let Some(minutes) = args.interval_minutes {
        picked += 1;
        out = Some((
            "interval".into(),
            serde_json::json!({ "minutes": minutes }).to_string(),
        ));
    }
    match (picked, out) {
        (1, Some(v)) => Ok(v),
        (0, _) => Err(AppError::Validation(
            "缺少调度参数——须给 cron / daily_time / interval_minutes 三者之一".into(),
        )),
        _ => Err(AppError::Validation(
            "调度参数只能给一个（cron / daily_time / interval_minutes 三选一）".into(),
        )),
    }
}

// =========================================================================
// list_scheduled_tasks（只读，Always）
// =========================================================================

pub struct ListScheduledTasksTool;

#[async_trait]
impl McpClient for ListScheduledTasksTool {
    fn name(&self) -> &str {
        "list_scheduled_tasks"
    }

    fn description(&self) -> &str {
        "List the user's scheduled tasks (recurring automations that run an agent turn on a \
         schedule). Returns each task's id, name, agent, schedule (e.g. 'daily 09:00'), next \
         run time, enabled state and last run status. Read-only, use it to review existing \
         tasks before creating or deleting one (avoid duplicates)."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(format!(
            "{} 必须通过 execute_with_context 调用",
            self.name()
        )))
    }

    async fn execute_with_context(&self, _args: &str, ctx: &ToolContext) -> AppResult<String> {
        let rows = task::list(&ctx.pool).await?;
        let mut items = Vec::with_capacity(rows.len());
        for t in &rows {
            let last = task::list_runs(&ctx.pool, &t.id, 1).await.ok().and_then(|r| {
                r.into_iter().next().map(|run| {
                    serde_json::json!({
                        "status": run.status,
                        "at": run.started_at,
                    })
                })
            });
            items.push(serde_json::json!({
                "id": t.id,
                "name": t.name,
                "agent_id": t.agent_id,
                "schedule": schedule_label(&t.schedule_kind, &t.schedule_data),
                "enabled": t.enabled != 0,
                "next_run": t.next_run,
                "last_run": last,
            }));
        }
        Ok(serde_json::json!({ "tasks": items }).to_string())
    }
}

// =========================================================================
// create_scheduled_task（Always + 后治理）
// =========================================================================

pub struct CreateScheduledTaskTool;

#[derive(serde::Deserialize)]
struct CreateArgs {
    name: String,
    prompt: String,
    /// cron 表达式（6/7 域含秒，本地时区）——与 daily_time / interval_minutes 三选一
    #[serde(default)]
    cron: Option<String>,
    /// 每天执行的时刻 "HH:MM"——三选一
    #[serde(default)]
    daily_time: Option<String>,
    /// 每隔 N 分钟——三选一（钳制最小 10 分钟）
    #[serde(default)]
    interval_minutes: Option<i64>,
    /// 执行任务的 agent id——缺省为当前 agent 自己
    #[serde(default)]
    agent_id: Option<String>,
    /// 完成后把结果投递到的会话（id 或唯一标题；缺省不转发）
    #[serde(default)]
    deliver_to: Option<String>,
}

#[async_trait]
impl McpClient for CreateScheduledTaskTool {
    fn name(&self) -> &str {
        "create_scheduled_task"
    }

    fn description(&self) -> &str {
        "Create a scheduled task that runs an agent turn automatically on a schedule (daily at a \
         time, every N minutes, or a cron expression). Use it whenever the user asks for \
         recurring or delayed work: 'every morning check X', 'remind me daily', 'run Y every \
         hour'. The task runs in its own dedicated conversation (history accumulates across \
         runs); the user can view, pause or delete it in Settings > Scheduled Tasks at any \
         time. Pick exactly ONE of: cron / daily_time / interval_minutes. Minimum interval is \
         10 minutes. The prompt should be self-contained - it is sent verbatim to the agent \
         on every run. Optionally set deliver_to (conversation id or unique title) to forward \
         each run's final answer to that conversation."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short task name shown in lists, e.g. 'morning project digest'." },
                "prompt": { "type": "string", "description": "Sent verbatim to the agent on every run. Self-contained: the task conversation only sees its own history." },
                "cron": { "type": "string", "description": "6/7-field cron with seconds, local timezone, e.g. '0 0 9 * * *' = daily 09:00. Mutually exclusive with daily_time / interval_minutes." },
                "daily_time": { "type": "string", "description": "'HH:MM' every day, local time. Mutually exclusive with cron / interval_minutes." },
                "interval_minutes": { "type": "integer", "description": "Run every N minutes (clamped to minimum 10). Mutually exclusive with cron / daily_time." },
                "agent_id": { "type": "string", "description": "Agent that executes the task. Defaults to yourself (the current agent)." },
                "deliver_to": { "type": "string", "description": "Optional conversation id or unique title to forward each run's final answer to." }
            },
            "required": ["name", "prompt"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(format!(
            "{} 必须通过 execute_with_context 调用",
            self.name()
        )))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: CreateArgs = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("create_scheduled_task 参数解析失败: {e}"))
        })?;
        if parsed.name.trim().is_empty() {
            return Err(AppError::Validation("任务名称不能为空".into()));
        }
        if parsed.prompt.trim().is_empty() {
            return Err(AppError::Validation("任务提示词不能为空".into()));
        }

        let (kind, data) = compose_schedule(&parsed)?;
        // 档位即校验 + 首个 next_run（一次性过去时间当场拒——同命令层口径）
        let spec = scheduler::parse_spec(&kind, &data)?;
        let next_run = scheduler::next_run_after(&spec, chrono::Local::now()).ok_or_else(|| {
            AppError::Validation(
                "一次性任务的时间已在过去——请选择未来时间".into(),
            )
        })?;

        // 转发目标解析（id 精确 → 标题唯一；重名不猜——relay 同哲学）
        let deliver_to_conv_id = match &parsed.deliver_to {
            Some(t) => Some(resolve_target(&ctx.pool, t).await?),
            None => None,
        };

        let new_task = NewScheduledTask {
            name: parsed.name.trim().to_string(),
            agent_id: parsed.agent_id.clone().unwrap_or_else(|| ctx.agent_id.clone()),
            schedule_kind: kind,
            schedule_data: data,
            prompt: parsed.prompt,
            miss_policy: None,
            deliver_to_conv_id,
            // 预授权范围是用户治理决策——agent 工具不暴露此参数，恒走默认
            // none（用户可在设置页为任务改档）
            preauth: None,
            enabled: None,
        };
        let id = Uuid::new_v4().to_string();
        let row = task::create(
            &ctx.pool,
            &id,
            &new_task,
            Some(&next_run.with_timezone(&chrono::Utc).format("%Y-%m-%d %H:%M:%S").to_string()),
        )
        .await?;

        Ok(serde_json::json!({
            "created": true,
            "id": row.id,
            "name": row.name,
            "schedule": schedule_label(&row.schedule_kind, &row.schedule_data),
            "next_run": row.next_run,
            "note": "任务已创建——用户可在「设置 · 定时任务」查看、暂停或删除",
        })
        .to_string())
    }
}

/// 目标会话解析（id 精确 → 标题唯一匹配；只认 kind='chat'）。
async fn resolve_target(pool: &sqlx::SqlitePool, target: &str) -> AppResult<String> {
    if let Ok(c) = conversation::get_by_id(pool, target).await {
        if c.kind == "chat" {
            return Ok(c.id);
        }
    }
    let chats = conversation::list_all(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.kind == "chat")
        .collect::<Vec<_>>();
    let mut by_title = chats.iter().filter(|c| c.title == target);
    if let Some(hit) = by_title.next() {
        if by_title.next().is_none() {
            return Ok(hit.id.clone());
        }
    }
    Err(AppError::Validation(format!(
        "找不到转发目标会话 '{}'——请确认标题唯一（重名须用会话 id）或请用户提供 id",
        target
    )))
}

// =========================================================================
// delete_scheduled_task（Always + 后治理）
// =========================================================================

pub struct DeleteScheduledTaskTool;

#[derive(serde::Deserialize)]
struct DeleteArgs {
    /// 任务 id 或名称（名称须唯一匹配，重名不猜——用 id 重试）
    task: String,
}

#[async_trait]
impl McpClient for DeleteScheduledTaskTool {
    fn name(&self) -> &str {
        "delete_scheduled_task"
    }

    fn description(&self) -> &str {
        "Delete a scheduled task by id or unique name. The task's dedicated conversation is \
         kept (user data); only the schedule and its run history are removed. Use \
         list_scheduled_tasks first to find the id. Ambiguous names are rejected - retry with \
         the id."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "Task id or its exact unique name." }
            },
            "required": ["task"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(format!(
            "{} 必须通过 execute_with_context 调用",
            self.name()
        )))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: DeleteArgs = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("delete_scheduled_task 参数解析失败: {e}"))
        })?;
        // id 精确 → 名称唯一（重名不猜）
        let id = match task::get_by_id(&ctx.pool, &parsed.task).await {
            Ok(Some(t)) => t.id,
            _ => {
                let tasks = task::list(&ctx.pool).await?;
                let mut by_name = tasks.iter().filter(|t| t.name == parsed.task);
                match (by_name.next(), by_name.next()) {
                    (Some(t), None) => t.id.clone(),
                    _ => {
                        return Err(AppError::Validation(format!(
                            "找不到任务 '{}' 或名称不唯一——请用 list_scheduled_tasks 查 id 后重试",
                            parsed.task
                        )));
                    }
                }
            }
        };
        let deleted_name = task::get_by_id(&ctx.pool, &id)
            .await
            .ok()
            .flatten()
            .map(|t| t.name)
            .unwrap_or_else(|| id.clone());
        task::delete(&ctx.pool, &id).await?;
        Ok(serde_json::json!({
            "deleted": true,
            "name": deleted_name,
        })
        .to_string())
    }
}

// =========================================================================
// update_scheduled_task（Always + 后治理；2026-09-20 补批——改任务不丢载体
// 与执行历史。preauth 不暴露：授权范围是用户治理决策，agent 侧恒不可改）
// =========================================================================

pub struct UpdateScheduledTaskTool;

#[derive(serde::Deserialize)]
struct UpdateArgs {
    /// 任务 id 或名称（名称须唯一匹配，重名不猜——用 id 重试）
    task: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    /// 调度参数三选一（与 create 同规：多给或混给都拒）
    #[serde(default)]
    cron: Option<String>,
    #[serde(default)]
    daily_time: Option<String>,
    #[serde(default)]
    interval_minutes: Option<i64>,
    #[serde(default)]
    deliver_to: Option<String>,
    #[serde(default)]
    miss_policy: Option<String>,
}

#[async_trait]
impl McpClient for UpdateScheduledTaskTool {
    fn name(&self) -> &str {
        "update_scheduled_task"
    }

    fn description(&self) -> &str {
        "Update an existing scheduled task (schedule, prompt, name, forwarding or miss policy) \
         WITHOUT losing its dedicated conversation or run history - prefer this over \
         delete+recreate when the user asks to change a task ('make the report run at 8 \
         instead', 'update the prompt to also check X'). Identify the task by id or unique \
         name (use list_scheduled_tasks first). Provide only the fields to change; pick \
         exactly ONE schedule field (cron / daily_time / interval_minutes) when rescheduling. \
         The tool authorizations the task runs with (preauth) are user-governed and cannot \
         be changed through this tool."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "Task id or exact unique name." },
                "name": { "type": "string", "description": "New task name (optional)." },
                "prompt": { "type": "string", "description": "New prompt sent on every run (optional)." },
                "cron": { "type": "string", "description": "Reschedule: 6/7-field cron with seconds, local time. Mutually exclusive with daily_time / interval_minutes." },
                "daily_time": { "type": "string", "description": "Reschedule: 'HH:MM' daily. Mutually exclusive with cron / interval_minutes." },
                "interval_minutes": { "type": "integer", "description": "Reschedule: every N minutes (min 10). Mutually exclusive with cron / daily_time." },
                "deliver_to": { "type": "string", "description": "Forward target conversation id or unique title (empty string clears)." },
                "miss_policy": { "type": "string", "description": "'run_once' (catch up on next launch) or 'skip' (postpone and record)." }
            },
            "required": ["task"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(format!(
            "{} 必须通过 execute_with_context 调用",
            self.name()
        )))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: UpdateArgs = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("update_scheduled_task 参数解析失败: {e}"))
        })?;

        // 目标解析（id 精确 → 名称唯一，重名不猜——delete 同款）
        let existing = match task::get_by_id(&ctx.pool, &parsed.task).await {
            Ok(Some(t)) => t,
            _ => {
                let tasks = task::list(&ctx.pool).await?;
                let mut by_name = tasks.iter().filter(|t| t.name == parsed.task);
                match (by_name.next(), by_name.next()) {
                    (Some(t), None) => t.clone(),
                    _ => {
                        return Err(AppError::Validation(format!(
                            "找不到任务 '{}' 或名称不唯一——请用 list_scheduled_tasks 查 id 后重试",
                            parsed.task
                        )));
                    }
                }
            }
        };

        // 调度变更（三选一；变了就校验 + 重算 next_run——与命令层同口径）
        let (kind, data) = if parsed.cron.is_some()
            || parsed.daily_time.is_some()
            || parsed.interval_minutes.is_some()
        {
            let pseudo = CreateArgs {
                name: String::new(),
                prompt: String::new(),
                cron: parsed.cron.clone(),
                daily_time: parsed.daily_time.clone(),
                interval_minutes: parsed.interval_minutes,
                agent_id: None,
                deliver_to: None,
            };
            compose_schedule(&pseudo)?
        } else {
            (existing.schedule_kind.clone(), existing.schedule_data.clone())
        };
        let spec = scheduler::parse_spec(&kind, &data)?;
        let next_run = scheduler::next_run_after(&spec, chrono::Local::now()).ok_or_else(|| {
            AppError::Validation("一次性任务的时间已在过去——请选择未来时间".into())
        })?;

        if let Some(miss) = parsed.miss_policy.as_deref() {
            if !matches!(miss, "run_once" | "skip") {
                return Err(AppError::Validation(
                    "miss_policy 无效——应为 run_once 或 skip".into(),
                ));
            }
        }
        if let Some(name) = parsed.name.as_deref() {
            if name.trim().is_empty() {
                return Err(AppError::Validation("任务名称不能为空".into()));
            }
        }
        if let Some(prompt) = parsed.prompt.as_deref() {
            if prompt.trim().is_empty() {
                return Err(AppError::Validation("任务提示词不能为空".into()));
            }
        }

        // 转发目标（空串 = 清除；None = 不改）
        let deliver_upd: Option<Option<String>> = match &parsed.deliver_to {
            Some(t) => {
                if t.is_empty() {
                    Some(None)
                } else {
                    Some(Some(resolve_target(&ctx.pool, t).await?))
                }
            }
            None => None,
        };

        task::update(
            &ctx.pool,
            &crate::db::models::ScheduledTaskUpdate {
                id: existing.id.clone(),
                name: parsed.name.clone(),
                agent_id: None,
                schedule_kind: Some(kind),
                schedule_data: Some(data),
                prompt: parsed.prompt.clone(),
                miss_policy: parsed.miss_policy.clone(),
                deliver_to_conv_id: deliver_upd,
                // 授权范围是用户治理决策——agent 侧恒不可改
                preauth: None,
                enabled: None,
            },
        )
        .await?;
        task::schedule_next(
            &ctx.pool,
            &existing.id,
            Some(&next_run.with_timezone(&chrono::Utc).format("%Y-%m-%d %H:%M:%S").to_string()),
            None,
        )
        .await?;

        let updated = task::get_by_id(&ctx.pool, &existing.id)
            .await
            .ok()
            .flatten()
            .unwrap_or(existing);
        Ok(serde_json::json!({
            "updated": true,
            "id": updated.id,
            "name": updated.name,
            "schedule": schedule_label(&updated.schedule_kind, &updated.schedule_data),
            "next_run": updated.next_run,
            "note": "载体与执行历史已保留",
        })
        .to_string())
    }
}
