//! 定时任务调度与执行（0.9.3，设计真相源 docs/scheduled-tasks-design.md）。
//!
//! 架构 = MA-3 消费模型复用（`inbox.rs::consume_pending` 同款）：到点 → 载体会话
//! 懒建/复用 → 物化带标注的 user 消息 → [`session_runner::run_agent_turn`] 全链路
//! （预算/工具/hooks/事件照常，零新执行路径）→ 写执行记录 + 算下次运行 + 可选
//! 结果转发（MA-3 `deliver`，目标会话按其收件政策消费）。
//!
//! ## 触发源
//!
//! 1. [`spawn_scheduler`]——60s tick 扫 due（`enabled ∧ next_run <= now`）逐个串行
//!    执行（单循环天然满足全局并发 1；会话撞忙则本轮跳过下轮重试）；
//! 2. [`spawn_boot_sweep`]——boot 延迟 30s 扫错过的任务，按任务级 `miss_policy`
//!    分叉：`run_once` 补跑一次最新（多次错过不连跑，steer boot_sweep 同款防风暴）
//!    / `skip` 记 `missed` 留痕并顺延；
//! 3. `run_scheduled_task_now` 命令——手动触发（独立于计划：不动 next_run）。
//!
//! ## 载体与转发（设计 §3 拍板：仅专属会话 + 投递伪绑定）
//!
//! 载体会话懒建（首跑物化时创建并回写 `target_conv_id`，创建后不可改绑）；
//! 「结果出现在某个既有会话」由转发投递实现。**项目边界适配**：懒建时若配置了
//! 转发目标，载体会话挂目标同项目（MA-3 硬边界要求源与目标同项目——散落源双向
//! 不可投）；后改转发目标到异项目会投递失败（run 记录披露），换任务或改回。
//!
//! ## 护栏
//!
//! - 任务级最小触发间隔 [`MIN_INTERVAL_MINUTES`]（10 分钟，interval 档创建/更新时
//!   钳制——防误配分钟级连跑风暴）；
//! - 全局并发 1（单 scheduler 循环串行）；
//! - 预检失败（agent 配置/provider 创建等）在调度触发下也记 error run 并推进
//!   next_run——坏任务不每分钟重试；手动触发则 Err 回显给用户。

use std::str::FromStr;
use std::time::Duration;

use chrono::{Datelike, DateTime, Local, NaiveTime, TimeZone, Utc};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::commands::agent_cmd::AgentCmd;
use crate::db::models::ScheduledTaskRow;
use crate::db::repo::{conversation, task};
use crate::error::{AppError, AppResult};
use crate::harness::chat_state::ChatState;
use crate::harness::provider;
use crate::harness::session_runner::{self, AgentTurnInput, TurnEnv};
use crate::infra::protocol::ContentBlock;

/// 调度 tick 周期。
pub const TICK: Duration = Duration::from_secs(60);
/// 任务级最小触发间隔（分钟）——interval 档钳制下限（护栏，防误配风暴）。
pub const MIN_INTERVAL_MINUTES: i64 = 10;
/// 执行记录摘要上限（字符）——完整回复在载体会话里，这里只要可扫读的截断。
pub const RUN_SUMMARY_MAX: usize = 200;
/// boot 扫尾初始延迟：避开 boot 迁移/自愈/图片外置写峰（space.rs 同款）。
const BOOT_SWEEP_DELAY: Duration = Duration::from_secs(30);

// =========================================================================
// 调度解析（纯函数，单测友好）
// =========================================================================

/// 调度档位（schedule_data JSON 的解析形态）。
#[derive(Debug, Clone, PartialEq)]
pub enum SchedSpec {
    /// 一次性（本地时间 'YYYY-MM-DD HH:MM:SS'）
    Once(DateTime<Local>),
    /// 每天 HH:MM
    Daily { time: NaiveTime },
    /// 每周几 HH:MM——weekdays 用 chrono 风格 0=周一 … 6=周日（前端词表对齐）
    Weekly { weekdays: Vec<u64>, time: NaiveTime },
    /// 每隔 N 分钟（受 MIN_INTERVAL_MINUTES 钳制）
    Interval { minutes: i64 },
    /// cron 表达式（6/7 域，标准分 时 日 月 周 [秒]）
    Cron(String),
}

#[derive(serde::Deserialize)]
struct OnceData {
    at: String,
}
#[derive(serde::Deserialize)]
struct TimeOnlyData {
    time: String,
}
#[derive(serde::Deserialize)]
struct WeeklyData {
    weekdays: Vec<u64>,
    time: String,
}
#[derive(serde::Deserialize)]
struct IntervalData {
    minutes: i64,
}
#[derive(serde::Deserialize)]
struct CronData {
    expr: String,
}

fn parse_hhmm(s: &str) -> AppResult<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M")
        .map_err(|e| AppError::Internal(format!("时间格式无效（应为 HH:MM）: {s} — {e}")))
}

fn parse_local(s: &str) -> AppResult<DateTime<Local>> {
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .map_err(|e| AppError::Internal(format!("时间格式无效（应为 YYYY-MM-DD HH:MM:SS）: {s} — {e}")))?;
    Local
        .from_local_datetime(&naive)
        .single()
        .ok_or_else(|| AppError::Internal(format!("本地时间不存在或歧义（DST 边界）: {s}")))
}

/// 解析 schedule_kind + schedule_data → SchedSpec。interval 档钳制最小间隔。
pub fn parse_spec(kind: &str, data: &str) -> AppResult<SchedSpec> {
    match kind {
        "once" => {
            let d: OnceData = serde_json::from_str(data)
                .map_err(|e| AppError::Internal(format!("once 档位载荷无效: {e}")))?;
            Ok(SchedSpec::Once(parse_local(&d.at)?))
        }
        "daily" => {
            let d: TimeOnlyData = serde_json::from_str(data)
                .map_err(|e| AppError::Internal(format!("daily 档位载荷无效: {e}")))?;
            Ok(SchedSpec::Daily { time: parse_hhmm(&d.time)? })
        }
        "weekly" => {
            let d: WeeklyData = serde_json::from_str(data)
                .map_err(|e| AppError::Internal(format!("weekly 档位载荷无效: {e}")))?;
            if d.weekdays.is_empty() || d.weekdays.iter().any(|&w| w > 6) {
                return Err(AppError::Internal(
                    "weekly 档位 weekdays 无效（应为 0-6 非空数组，0=周一）".into(),
                ));
            }
            Ok(SchedSpec::Weekly { weekdays: d.weekdays, time: parse_hhmm(&d.time)? })
        }
        "interval" => {
            let d: IntervalData = serde_json::from_str(data)
                .map_err(|e| AppError::Internal(format!("interval 档位载荷无效: {e}")))?;
            Ok(SchedSpec::Interval { minutes: d.minutes.max(MIN_INTERVAL_MINUTES) })
        }
        "cron" => {
            let d: CronData = serde_json::from_str(data)
                .map_err(|e| AppError::Internal(format!("cron 档位载荷无效: {e}")))?;
            // 解析即校验（坏表达式在创建/更新时暴露，不拖到调度期）
            cron::Schedule::from_str(&d.expr)
                .map_err(|e| AppError::Internal(format!("cron 表达式无效: {e}")))?;
            Ok(SchedSpec::Cron(d.expr))
        }
        other => Err(AppError::Internal(format!("未知调度档位: {other}"))),
    }
}

/// after 之后（严格大于）的下一次运行时刻；None = 一次性任务已过时。
pub fn next_run_after(spec: &SchedSpec, after: DateTime<Local>) -> Option<DateTime<Local>> {
    match spec {
        SchedSpec::Once(at) => (*at > after).then_some(*at),
        SchedSpec::Daily { time } => {
            let mut day = after.date_naive();
            loop {
                let cand = day.and_time(*time);
                if let Some(dt) = Local.from_local_datetime(&cand).single() {
                    if dt > after {
                        return Some(dt);
                    }
                }
                // DST 歧义/不存在（凌晨 2:30 类）→ 跳过该天取次日，不硬造时刻
                day += chrono::Duration::days(1);
            }
        }
        SchedSpec::Weekly { weekdays, time } => {
            let mut day = after.date_naive();
            loop {
                let wd = day.weekday().num_days_from_monday() as u64;
                if weekdays.contains(&wd) {
                    if let Some(dt) = Local.from_local_datetime(&day.and_time(*time)).single() {
                        if dt > after {
                            return Some(dt);
                        }
                    }
                }
                day += chrono::Duration::days(1);
            }
        }
        SchedSpec::Interval { minutes } => {
            Some(after + chrono::Duration::minutes(*minutes))
        }
        SchedSpec::Cron(expr) => {
            let sched = cron::Schedule::from_str(expr).ok()?;
            // after 用调用方时区（本地）——cron 字段按本地时间解释（「0 9 * * *」
            // = 本地每天 9 点，用户心智；非 UTC）
            sched.after(&after).next()
        }
    }
}

/// 预览未来 n 个运行时点（cron 逃生舱防写错——前端表单实时显示）。
pub fn preview_next_runs(spec: &SchedSpec, n: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(n);
    let mut cursor = Local::now();
    for _ in 0..n {
        match next_run_after(spec, cursor) {
            Some(next) => {
                out.push(next.format("%Y-%m-%d %H:%M:%S").to_string());
                cursor = next;
            }
            None => break,
        }
    }
    out
}

/// 存库格式：本地调度时刻 → UTC 字符串（DB 惯例 UTC 存储；显示层转本地）。
fn fmt_store(dt: DateTime<Local>) -> String {
    dt.with_timezone(&Utc).format("%Y-%m-%d %H:%M:%S").to_string()
}

// =========================================================================
// 执行器
// =========================================================================

/// 执行结果（手动/调度触发共用）。
#[derive(Debug)]
pub enum ExecOutcome {
    /// 回合已发起并完成（run 行已落终态）
    Ran,
    /// 载体会话正忙（调度触发 = 下轮重试；手动触发应提示用户）
    Busy,
}

/// 懒建载体会话：配置了转发则挂目标同项目（MA-3 项目边界适配，见模块头）。
async fn ensure_target_conv(
    pool: &SqlitePool,
    app: &AppHandle,
    t: &ScheduledTaskRow,
) -> AppResult<crate::db::models::ConversationRow> {
    if let Some(cid) = &t.target_conv_id {
        match conversation::get_by_id(pool, cid).await {
            Ok(conv) => return Ok(conv),
            // 载体会话被用户删除 → 重建（同名同项目/散落），回写新 id
            Err(crate::error::AppError::NotFound { .. }) => {}
            Err(e) => return Err(e),
        }
    }
    let project_id = match &t.deliver_to_conv_id {
        Some(dst) => match conversation::get_by_id(pool, dst).await {
            Ok(c) => c.project_id,
            // 转发目标不存在 → 散落载体（投递期会诚实报错并记入 run）
            Err(_) => None,
        },
        None => None,
    };
    let conv_id = Uuid::new_v4().to_string();
    let conv = conversation::create(
        pool,
        &conv_id,
        &crate::db::models::NewConversation {
            agent_id: t.agent_id.clone(),
            title: Some(t.name.clone()),
            project_id,
            kind: Some("chat".into()),
            initiator_agent_id: None,
            parent_conversation_id: None,
        },
    )
    .await?;
    sqlx::query("UPDATE scheduled_tasks SET target_conv_id = ?, updated_at = datetime('now','localtime') WHERE id = ?")
        .bind(&conv_id)
        .bind(&t.id)
        .execute(pool)
        .await?;
    let _ = app;
    Ok(conv)
}

/// 执行一个任务回合。`scheduled = true`（调度/补跑触发）会推进 next_run；
/// 手动触发（false）不动计划（只记 run + last_run_at 不更新——last_run 表述
/// 的是计划执行语义，手动跑不算）。
pub async fn execute_task(
    app: &AppHandle,
    pool: &SqlitePool,
    t: &ScheduledTaskRow,
    scheduled: bool,
) -> AppResult<ExecOutcome> {
    let spec = match parse_spec(&t.schedule_kind, &t.schedule_data) {
        Ok(s) => s,
        Err(e) => {
            // 档位坏（手改库等）——调度触发记 error run + 推进 next 防每分钟重试
            if scheduled {
                record_error_run(pool, t, None, &e.to_string()).await;
                advance_schedule(pool, t, &spec_placeholder(&t.schedule_kind, &t.schedule_data)).await;
            }
            return Err(e);
        }
    };

    // --- 预检：agent 凭据 + provider（同 inbox 消费模型；失败不静默） ---
    let pre = async {
        let agent_cmd = app.state::<std::sync::Arc<dyn AgentCmd>>().inner().clone();
        let creds = agent_cmd.get_with_credentials(&t.agent_id).await.map_err(|e| {
            AppError::Internal(format!(
                "读取任务 agent（{}）配置/凭据失败: {e}——请到设置·智能体检查该 agent",
                t.agent_id
            ))
        })?;
        let llm_provider = provider::create_provider(
            &creds.agent.provider,
            &creds.agent.model,
            creds.base_url.as_deref(),
            creds.agent.cache_prompt != 0,
        )
        .map_err(|e| {
            AppError::Internal(format!(
                "为任务 agent（{}，{}/{}）创建 provider 失败: {e}",
                creds.agent.name, creds.agent.provider, creds.agent.model
            ))
        })?;
        Ok::<_, AppError>((creds, llm_provider))
    };
    let (creds, llm_provider) = match pre.await {
        Ok(v) => v,
        Err(e) => {
            if scheduled {
                record_error_run(pool, t, None, &e.to_string()).await;
                advance_schedule(pool, t, &spec).await;
            }
            return Err(e);
        }
    };

    // --- 载体会话 + 单写者仲裁（忙 = 下轮重试，不算执行过） ---
    let conv = ensure_target_conv(pool, app, t).await?;
    let chat_state = app.state::<ChatState>().inner().clone();
    let Ok(cancel_token) = chat_state.start(&conv.id) else {
        return Ok(ExecOutcome::Busy);
    };
    let conv_id_guard = conv.id.clone();
    let cancel_guard = scopeguard::guard((), |_| chat_state.unregister(&conv_id_guard));

    // --- 无人值守预授权 seed（2026-09-20 拍板三档）：任务 = 用户事先委托的
    //     自动化，授权范围建任务时划定——治理权最大原则。必须在回合 spawn 前
    //     完成（委派 4.5 同款时序）；只写载体会话的授权记忆（他处零污染），
    //     生效路径 = Confirm 级判定的 is_tool_authorized 分支。屏幕家族仍走
    //     通道治理不进预授权（委派不变式 3 对齐：request_screen_session 排除）。
    match t.preauth.as_str() {
        "commands" => {
            app.state::<crate::harness::authority::AuthSessionRegistry>()
                .inner()
                .session_for(&conv.id)
                .mark_tool_authorized("run_command")
                .await;
            tracing::info!(
                target: "ice_paw.scheduler",
                task = %t.name,
                "任务预授权: commands 档（载体会话 run_command 免问）"
            );
        }
        "all" => {
            let registry = app
                .state::<std::sync::Arc<crate::harness::mcp::McpRegistry>>()
                .inner()
                .clone();
            let snap = registry.snapshot().await;
            let mut seeded = 0usize;
            let session = app
                .state::<crate::harness::authority::AuthSessionRegistry>()
                .inner();
            // 复用 session_for：逐工具 mark（幂等）
            for (name, tool) in &snap {
                if *name == "request_screen_session" {
                    continue; // 屏幕家族走通道治理，不进预授权
                }
                if tool.authorization_level()
                    == crate::harness::mcp::types::AuthorizationLevel::Confirm
                {
                    session.session_for(&conv.id).mark_tool_authorized(name).await;
                    seeded += 1;
                }
            }
            tracing::info!(
                target: "ice_paw.scheduler",
                task = %t.name,
                "任务预授权: all 档（载体会话 {seeded} 个 Confirm 工具免问）"
            );
        }
        _ => {}
    }

    // --- 执行记录（running；载体会话已确定） ---
    let run_id = Uuid::new_v4().to_string();
    task::insert_run(pool, &run_id, &t.id, &conv.id).await?;

    // --- 任务回合：双块物化（标注 + 正文），run_agent_turn 全链路 ---
    // content_text = prompt 原文（检索面即用户意图——relevance_query None 回落
    // content_text，P3 精神天然满足）；标注块披露「系统定时触发」非用户手打。
    let annotation = format!("[定时任务 · {}]", t.name);
    let blocks = vec![
        ContentBlock::Text { text: annotation },
        ContentBlock::Text { text: t.prompt.clone() },
    ];
    let fallback =
        crate::harness::fallback_plan::production_fallback_plan(app, pool, &creds.agent);

    let result = session_runner::run_agent_turn(
        &TurnEnv {
            emitter: crate::harness::r#loop::emitter::tauri_emitter(app.clone(), conv.id.clone()),
            tool_app: Some(app.clone()),
            pool: pool.clone(),
            route_registry: app.state::<crate::harness::read_route::ReadRouteRegistry>().inner(),
            chat_state: chat_state.clone(),
            global_registry: std::sync::Arc::clone(
                app.state::<std::sync::Arc<crate::harness::mcp::McpRegistry>>().inner(),
            ),
            mcp_manager: std::sync::Arc::clone(
                app.state::<std::sync::Arc<crate::harness::mcp::McpServerManager>>().inner(),
            ),
            auth_registry: app
                .state::<crate::harness::tool_executor::ToolAuthRegistry>()
                .inner()
                .clone(),
            auth_sessions: app
                .state::<crate::harness::authority::AuthSessionRegistry>()
                .inner()
                .clone(),
        },
        AgentTurnInput {
            conv: conv.clone(),
            agent: creds.agent.clone(),
            hooks: creds.hooks,
            word_style_profile: creds.word_style_profile,
            provider: llm_provider,
            api_key: creds.api_key,
            user_msg_id: Uuid::new_v4().to_string(),
            content_text: t.prompt.clone(),
            relevance_query: None,
            llm_blocks: blocks.clone(),
            persist_blocks: blocks,
            attach_db_inputs: Vec::new(),
            attach_file_inputs: Vec::new(),
            emit_user_blocks: false,
            incoming_source: None,
            sender_name: None,
            pre_materialized: false,
            tools_enabled: true,
            model_override: None,
            cancel_token,
            fallback,
        },
    )
    .await;

    let done_rx = match result {
        Ok(rx) => rx,
        Err(e) => {
            // 发起失败：注销责任在 run_agent_turn 内部已处理（guard 未移交）
            task::finish_run(pool, &run_id, "error", None, Some(&e.to_string())).await?;
            if scheduled {
                advance_schedule(pool, t, &spec).await;
            }
            return Err(e);
        }
    };
    // 发起成功：注销责任移交 loop 的 RAII 守卫
    scopeguard::ScopeGuard::into_inner(cancel_guard);

    // --- 同步等完成（调度循环串行 = 全局并发 1；手动触发整函数被 spawn） ---
    let final_text = match done_rx.await {
        Ok(summary) => {
            let text = summary.final_text.trim().to_string();
            let short: String = text.chars().take(RUN_SUMMARY_MAX).collect();
            task::finish_run(pool, &run_id, "done", Some(&short), None).await?;
            text
        }
        Err(e) => {
            let msg = e.to_string();
            task::finish_run(pool, &run_id, "error", None, Some(&msg)).await?;
            String::new()
        }
    };
    if scheduled {
        advance_schedule(pool, t, &spec).await;
    }

    // --- 可选转发（MA-3 通道；失败披露进 run 记录，不掩盖） ---
    if let Some(dst) = &t.deliver_to_conv_id {
        if !final_text.is_empty() {
            let source = crate::harness::inbox::SourceInfo {
                conv_id: conv.id.clone(),
                conv_title: conv.title.clone(),
                agent_id: conv.agent_id.clone(),
                agent_name: creds.agent.name.clone(),
                project_id: conv.project_id.clone(),
            };
            let deliver_result = crate::harness::inbox::deliver(
                app, pool, &source, dst, &final_text, false, false,
            )
            .await;
            let note = match deliver_result {
                Ok(_) => "\\n[转发投递：已送达]".to_string(),
                Err(e) => format!("\\n[转发投递失败：{e}]"),
            };
            let _ = sqlx::query(
                "UPDATE task_runs SET summary = COALESCE(summary,'') || ? WHERE id = ?",
            )
            .bind(&note)
            .bind(&run_id)
            .execute(pool)
            .await;
        }
    }
    Ok(ExecOutcome::Ran)
}

/// 档位解析失败时的占位推进（interval 分钟后重试一次坏档位不再死循环）。
fn spec_placeholder(kind: &str, data: &str) -> SchedSpec {
    let _ = (kind, data);
    SchedSpec::Interval { minutes: MIN_INTERVAL_MINUTES }
}

/// 推进调度：once → 完成禁用；其余 → 下一个时点。last_run_at 一并落。
async fn advance_schedule(pool: &SqlitePool, t: &ScheduledTaskRow, spec: &SchedSpec) {
    match next_run_after(spec, Local::now()) {
        Some(next) => {
            let _ = task::schedule_next(pool, &t.id, Some(&fmt_store(next)), None).await;
        }
        None => {
            // 一次性任务完成：禁用 + next_run 置空（不删——执行日志保留）
            let _ = task::schedule_next(pool, &t.id, None, Some(0)).await;
        }
    }
}

async fn record_error_run(
    pool: &SqlitePool,
    t: &ScheduledTaskRow,
    conv_id: Option<&str>,
    msg: &str,
) {
    let run_id = Uuid::new_v4().to_string();
    let _ = task::insert_run(pool, &run_id, &t.id, conv_id.unwrap_or("")).await;
    let _ = task::finish_run(pool, &run_id, "error", None, Some(msg)).await;
}

// =========================================================================
// 调度循环 + boot 扫尾
// =========================================================================

/// tick 扫描并串行执行 due 任务（单循环 = 全局并发 1）。撞忙任务不动 next_run，
/// 下一轮自然重试。
async fn sweep_due(app: &AppHandle, pool: &SqlitePool) {
    let now = task::utc_now_str();
    let due = match task::due_tasks(pool, &now).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(target: "ice_paw.scheduler", "due 扫描失败: {e}");
            return;
        }
    };
    for t in due {
        match execute_task(app, pool, &t, true).await {
            Ok(ExecOutcome::Ran) => {}
            Ok(ExecOutcome::Busy) => {
                tracing::info!(
                    target: "ice_paw.scheduler",
                    task = %t.name,
                    "载体会话正忙，本轮跳过（下一 tick 重试）"
                );
            }
            Err(e) => {
                // 预检/发起失败已在 execute_task 内记 error run + 推进 next
                tracing::warn!(target: "ice_paw.scheduler", task = %t.name, "任务执行失败: {e}");
            }
        }
    }
}

/// 调度主循环（lib.rs boot 挂点）。
pub fn spawn_scheduler(app: AppHandle, pool: SqlitePool) {
    tauri::async_runtime::spawn(async move {
        tracing::info!(target: "ice_paw.scheduler", "调度器已启动（tick {:?}）", TICK);
        loop {
            tokio::time::sleep(TICK).await;
            sweep_due(&app, &pool).await;
        }
    });
}

/// boot 扫尾：错过的任务按 miss_policy 分叉（补跑一次 / 记 missed 顺延）。
pub fn spawn_boot_sweep(app: AppHandle, pool: SqlitePool) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(BOOT_SWEEP_DELAY).await;
        let now = task::utc_now_str();
        let due = match task::due_tasks(&pool, &now).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(target: "ice_paw.scheduler", "boot 扫尾扫描失败: {e}");
                return;
            }
        };
        if due.is_empty() {
            return;
        }
        tracing::info!(target: "ice_paw.scheduler", "boot 扫尾：{} 个错过的定时任务", due.len());
        for t in due {
            if t.miss_policy == "skip" {
                // 顺延到未来首个时点 + missed 留痕（诚实：错过的次数可查）
                if let Ok(spec) = parse_spec(&t.schedule_kind, &t.schedule_data) {
                    if let Some(next) = next_run_after(&spec, Local::now()) {
                        let _ = task::schedule_next(&pool, &t.id, Some(&fmt_store(next)), None).await;
                    } else {
                        let _ = task::schedule_next(&pool, &t.id, None, Some(0)).await;
                    }
                }
                let run_id = Uuid::new_v4().to_string();
                if let Some(conv_id) = &t.target_conv_id {
                    let _ = task::insert_run(&pool, &run_id, &t.id, conv_id).await;
                    let _ = task::finish_run(&pool, &run_id, "missed", None, None).await;
                }
                tracing::info!(target: "ice_paw.scheduler", task = %t.name, "错过任务按 skip 顺延");
            } else {
                tracing::info!(target: "ice_paw.scheduler", task = %t.name, "错过任务补跑一次（run_once）");
                let _ = execute_task(&app, &pool, &t, true).await;
            }
        }
    });
}

/// 手动触发（命令层入口）：fire-and-forget，不动调度计划。
pub fn run_now(app: AppHandle, pool: SqlitePool, task_id: String) {
    tauri::async_runtime::spawn(async move {
        match task::get_by_id(&pool, &task_id).await {
            Ok(Some(t)) => match execute_task(&app, &pool, &t, false).await {
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(target: "ice_paw.scheduler", "手动触发失败: {e}");
                }
            },
            Ok(None) => {
                tracing::warn!(target: "ice_paw.scheduler", "手动触发：任务不存在（{task_id}）");
            }
            Err(e) => {
                tracing::warn!(target: "ice_paw.scheduler", "手动触发读取任务失败: {e}");
            }
        }
    });
}

// =========================================================================
// 单元测试（调度解析纯函数）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn local(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    #[test]
    fn daily_next_run_crosses_midnight() {
        let spec = SchedSpec::Daily { time: NaiveTime::from_hms_opt(9, 0, 0).unwrap() };
        // 09:30 当天已过 → 次日 09:00
        let next = next_run_after(&spec, local(2026, 9, 20, 9, 30)).unwrap();
        assert_eq!(next, local(2026, 9, 21, 9, 0));
        // 08:30 → 当天 09:00
        let next = next_run_after(&spec, local(2026, 9, 20, 8, 30)).unwrap();
        assert_eq!(next, local(2026, 9, 20, 9, 0));
    }

    #[test]
    fn weekly_picks_next_matching_weekday() {
        // 周一(0)/周三(2)/周五(4) 08:00；周四 10:00 出发 → 周五 08:00
        let spec = SchedSpec::Weekly {
            weekdays: vec![0, 2, 4],
            time: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        };
        // 2026-09-17 是周四
        let next = next_run_after(&spec, local(2026, 9, 17, 10, 0)).unwrap();
        assert_eq!(next, local(2026, 9, 18, 8, 0)); // 周五
        // 同日早于 08:00 → 当天（周四不在集 → 周五）
        let next = next_run_after(&spec, local(2026, 9, 17, 6, 0)).unwrap();
        assert_eq!(next, local(2026, 9, 18, 8, 0));
    }

    #[test]
    fn interval_is_clamped_to_minimum() {
        let spec = parse_spec("interval", r#"{"minutes":1}"#).unwrap();
        assert!(matches!(spec, SchedSpec::Interval { minutes: 10 }));
        // 从 10:00:00 出发 → 10:10:00（严格大于）
        let next = next_run_after(&spec, local(2026, 9, 20, 10, 0)).unwrap();
        assert_eq!(next, local(2026, 9, 20, 10, 10));
    }

    #[test]
    fn once_in_future_and_expired() {
        let spec = SchedSpec::Once(local(2026, 9, 21, 9, 0));
        assert!(next_run_after(&spec, local(2026, 9, 20, 9, 0)).is_some());
        assert!(next_run_after(&spec, local(2026, 9, 22, 9, 0)).is_none());
    }

    #[test]
    fn cron_expr_roundtrip() {
        // cron crate 为 6/7 域（含秒）：秒 分 时 日 月 周——本地时区解释（见实现注释）
        let spec = parse_spec("cron", r#"{"expr":"0 0 9 * * *"}"#).unwrap();
        let next = next_run_after(&spec, local(2026, 9, 20, 10, 0)).unwrap();
        assert_eq!(next, local(2026, 9, 21, 9, 0));
    }

    #[test]
    fn bad_payloads_rejected() {
        assert!(parse_spec("daily", "{bad json").is_err());
        assert!(parse_spec("daily", r#"{"time":"25:00"}"#).is_err());
        assert!(parse_spec("weekly", r#"{"weekdays":[],"time":"08:00"}"#).is_err());
        assert!(parse_spec("weekly", r#"{"weekdays":[7],"time":"08:00"}"#).is_err());
        assert!(parse_spec("cron", r#"{"expr":"not a cron"}"#).is_err());
        assert!(parse_spec("nope", "{}").is_err());
    }

    #[test]
    fn preview_returns_strictly_ascending() {
        let spec = parse_spec("daily", r#"{"time":"09:00"}"#).unwrap();
        let out = preview_next_runs(&spec, 3);
        assert_eq!(out.len(), 3);
        assert!(out[0] < out[1] && out[1] < out[2]);
    }
}
