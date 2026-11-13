//! `start_delegate` / `stop_delegate` —— 异步委派变体（Router 化 Phase 2 批 P2-1）。
//!
//! 与同步 [`crate::harness::mcp::delegate::DelegateTool`] 的分工：
//! - **delegate（同步）**：父 agent 内联等待子会话跑完、结果进本轮对话流——
//!   「我需要这个结果才能继续」的串行语义。
//! - **start_delegate（异步）**：受理即回执（child_conversation_id），父 agent
//!   继续干别的（同轮可再派/直接回答用户）——「扇出并行 / 不用等」语义。
//!   完成后由后台回投任务把结果投回父会话（inbox 机制：pending 入队 + 父会话
//!   空闲自动消费——父 agent 下回合自然看到汇报），等价于简报 §5① 的
//!   「worker 完成事件唤醒发起方」在存量委派上的落地。
//!
//! 执行面与同步版同构（同引擎/同授权/同 cancel 级联），差异仅三处：
//! 1. spawn 后不 await done_rx（活性护栏语义消失——护栏防的是父回合挂死等待，
//!    异步父回合不等待；done_rx 消费端是回投任务，loop 死则 RecvError 即返）；
//! 2. tool_result 是受理回执不是任务结果；
//! 3. 完成事实 = 子会话 turn_ended（既有）+ 回投投递（新）。
//!
//! M6 决策插槽：目标解析当前与同步版同源（父 agent 直选 + 项目成员优先）——
//! Phase 3 Router 落地后接管此环节（未指名委派进 Router 评估），本文件不改语义。

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tauri::{Emitter, Manager};
use uuid::Uuid;

use crate::commands::agent_cmd::AgentCmd;
use crate::db::models::NewConversation;
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::chat_state::ChatState;
use crate::harness::read_route::ReadRouteRegistry;
use crate::harness::session_runner::{self, AgentTurnInput, TurnEnv};
use crate::infra::protocol::{ContentBlock, DelegationAuthRequestPayload, DelegationGrant, DelegationStartedPayload};

use super::client::{McpClient, ToolContext};
use super::delegate::{resolve_dispatchable, resolve_target};
use super::manager::McpServerManager;
use super::types::AuthorizationLevel;
use super::McpRegistry;

// =========================================================================
// start_delegate
// =========================================================================

pub struct StartDelegateTool;

#[derive(Deserialize)]
struct StartDelegateArgs {
    agent_id: String,
    task: String,
}

#[async_trait]
impl McpClient for StartDelegateTool {
    fn name(&self) -> &str {
        "start_delegate"
    }

    fn description(&self) -> &str {
        "Asynchronously delegate a task to an agent: returns immediately with a receipt \
(conversation id), you keep working. When the expert finishes, the result is delivered \
back into this conversation automatically. IMPORTANT: the report arrives only AFTER \
your current turn ends — never wait or poll inside the turn for it (deadlock: the \
report waits for your turn to end). Finish the turn; the report starts a new turn. \
Use when you do NOT need the result to continue (parallel fan-out). Use \
delegate_to_agent instead when you must wait for the result in this turn. Cancel \
with stop_delegate."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "Target agent's id or exact name (see the dispatchable agent list in your system prompt)."
                },
                "task": {
                    "type": "string",
                    "description": "Self-contained task description - the only input the target agent receives."
                }
            },
            "required": ["agent_id", "task"]
        })
    }

    /// 与 delegate 同款：授权决策点是工具内部治理层（全自动短路同语义接通）
    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "start_delegate 必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: StartDelegateArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("start_delegate 参数解析失败: {e}")))?;
        if parsed.task.trim().is_empty() {
            return Err(AppError::Validation(
                "start_delegate 任务描述为空——task 必须自包含（这是专家看到的全部输入）".into(),
            ));
        }
        let app = ctx.app_handle.clone().ok_or_else(|| {
            AppError::Internal("start_delegate 需要 App 上下文（app_handle）".into())
        })?;

        // --- 1. 目标解析（与同步版同源：项目成员优先 / 剔自己 / 重名不猜） ---
        let parent = repo::conversation::get_by_id(&ctx.pool, &ctx.conv_id).await?;
        if parsed.agent_id == ctx.agent_id {
            return Err(AppError::Validation(
                "不能委派给自己——请指定一个不同的 agent".into(),
            ));
        }
        let dispatchable =
            resolve_dispatchable(&ctx.pool, parent.project_id.as_deref(), &ctx.agent_id).await?;
        let target = resolve_target(&dispatchable, &parsed.agent_id).ok_or_else(|| {
            AppError::Validation(format!(
                "目标 agent '{}' 不在可调度集合内。当前可调度：{}",
                parsed.agent_id,
                dispatchable
                    .iter()
                    .map(|a| format!("{}({})", a.name, a.id))
                    .collect::<Vec<_>>()
                    .join("、")
            ))
        })?;

        // --- 2. 专家凭据 + provider（与同步版同构） ---
        let agent_cmd = app.state::<Arc<dyn AgentCmd>>().inner().clone();
        let creds = agent_cmd
            .get_with_credentials(&target.id)
            .await
            .map_err(|e| {
                AppError::Internal(format!(
                    "读取专家 agent（{}）配置/凭据失败: {e}——请换一个 agent 或如实告知用户",
                    target.name
                ))
            })?;
        let expert_provider = crate::harness::provider::create_provider(
            &creds.agent.provider,
            &creds.agent.model,
            creds.base_url.as_deref(),
            creds.agent.cache_prompt != 0,
        )
        .map_err(|e| {
            AppError::Internal(format!(
                "为专家 agent（{}，{}/{}) 创建 provider 失败: {e}",
                target.name, creds.agent.provider, creds.agent.model
            ))
        })?;

        // --- 2.5 委派授权（与同步版同一张卡 + 全自动短路——异步不改变信任语义） ---
        let parent_cancel = ctx
            .cancel
            .clone()
            .ok_or_else(|| AppError::Internal("start_delegate 缺少父会话取消令牌".into()))?;
        let grant = if crate::harness::auto_approve::global()
            .is_on(&ctx.conv_id)
            .await
        {
            tracing::info!(
                target: "ice_paw.delegate",
                parent_conv = %ctx.conv_id,
                to_agent = %target.id,
                "全自动放行异步委派授权（会话级开关，不弹卡）"
            );
            None
        } else {
            let auth_registry = app
                .state::<crate::harness::tool_executor::ToolAuthRegistry>()
                .inner()
                .clone();
            let auth_request_id = Uuid::new_v4().to_string();
            let rx = auth_registry.register(auth_request_id.clone()).await;
            let auth_payload = DelegationAuthRequestPayload {
                request_id: auth_request_id.clone(),
                conversation_id: ctx.conv_id.clone(),
                message_id: ctx.turn_id.clone().unwrap_or_default(),
                agent_name: target.name.clone(),
                agent_id: target.id.clone(),
                task: format!("[异步] {}", parsed.task),
            };
            if let Err(e) = app.emit("chat:delegation-auth-request", &auth_payload) {
                let _ = auth_registry.take(&auth_request_id).await;
                return Err(AppError::Internal(format!("无法通知前端委派授权请求: {e}")));
            }
            tracing::info!(
                target: "ice_paw.delegate",
                parent_conv = %ctx.conv_id,
                to_agent = %target.id,
                request_id = %auth_request_id,
                "等待用户异步委派授权（120s 超时）"
            );
            let auth_emitter =
                crate::harness::r#loop::emitter::tauri_emitter(app.clone(), ctx.conv_id.clone());
            match crate::harness::tool_executor::wait_for_auth_response(
                rx,
                &parent_cancel,
                &auth_request_id,
                &auth_registry,
                auth_emitter.as_ref(),
                &ctx.conv_id,
            )
            .await
            {
                Some(resp) if resp.allowed => resp.delegation_grant,
                Some(_) => {
                    return Err(AppError::AuthorizationRequired {
                        tool: "start_delegate".into(),
                        reason: format!(
                            "用户拒绝了这次异步委派（目标 {}，任务：{}…）——请与用户确认意图后再试",
                            target.name,
                            crate::infra::strings::truncate_to_byte_boundary(
                                parsed.task.trim(), 40, Some("…")
                            )
                        ),
                    });
                }
                None => {
                    return Err(AppError::AuthorizationRequired {
                        tool: "start_delegate".into(),
                        reason: "异步委派授权被取消或超时（120 秒内未获用户批准）".into(),
                    });
                }
            }
        };

        // --- 3. 建子会话（kind='delegation'，同同步版） ---
        let title =
            crate::infra::strings::truncate_to_byte_boundary(parsed.task.trim(), 60, Some("…"));
        let child_conv_id = Uuid::new_v4().to_string();
        let child_conv = repo::conversation::create(
            &ctx.pool,
            &child_conv_id,
            &NewConversation {
                agent_id: creds.agent.id.clone(),
                title: Some(title.clone()),
                project_id: parent.project_id.clone(),
                kind: Some("delegation".into()),
                initiator_agent_id: Some(ctx.agent_id.clone()),
                parent_conversation_id: Some(ctx.conv_id.clone()),
                auto_approve: (parent.auto_approve != 0).then_some(1),
            },
        )
        .await?;
        crate::harness::auto_approve::global()
            .inherit(&ctx.conv_id, &child_conv_id)
            .await;

        // --- 4. cancel 级联 + ChatState 注册（RAII 移交：spawn 后由回投任务善后） ---
        let child_cancel = parent_cancel.child_token();
        let chat_state = app.state::<ChatState>().inner().clone();
        chat_state.register(&child_conv_id, child_cancel.clone());
        let guard_conv_id = child_conv_id.clone();
        let cancel_guard =
            scopeguard::guard((), |_| chat_state.unregister(&guard_conv_id));

        let _ = app.emit(
            "chat:delegation-started",
            DelegationStartedPayload {
                conversation_id: ctx.conv_id.clone(),
                child_conversation_id: child_conv_id.clone(),
                agent_name: target.name.clone(),
                title: title.clone(),
                tool_use_id: ctx.tool_use_id.clone(),
            },
        );

        // --- 4.5 预授权 seed（命令免问档，与同步版同构） ---
        if grant == Some(DelegationGrant::Commands) {
            app.state::<crate::harness::authority::AuthSessionRegistry>()
                .inner()
                .session_for(&child_conv_id)
                .mark_tool_authorized("run_command")
                .await;
        }

        tracing::info!(
            target: "ice_paw.delegate",
            parent_conv = %ctx.conv_id,
            child_conv = %child_conv_id,
            to_agent = %target.id,
            "异步委派受理：专家独立执行，完成后回投本会话"
        );

        // --- 5. spawn 回合 + 后台回投任务（不等 done_rx——异步语义核心） ---
        let done_rx = session_runner::run_agent_turn(
            &TurnEnv {
                emitter: crate::harness::r#loop::emitter::tauri_emitter(
                    app.clone(),
                    child_conv_id.clone(),
                ),
                tool_app: Some(app.clone()),
                pool: ctx.pool.clone(),
                route_registry: app.state::<ReadRouteRegistry>().inner(),
                chat_state: chat_state.clone(),
                global_registry: Arc::clone(app.state::<Arc<McpRegistry>>().inner()),
                mcp_manager: Arc::clone(app.state::<Arc<McpServerManager>>().inner()),
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
                conv: child_conv,
                agent: creds.agent.clone(),
                hooks: creds.hooks,
                word_style_profile: creds.word_style_profile,
                provider: expert_provider,
                api_key: creds.api_key,
                user_msg_id: Uuid::new_v4().to_string(),
                content_text: parsed.task.clone(),
                relevance_query: None,
                llm_blocks: vec![ContentBlock::text(parsed.task.clone())],
                persist_blocks: vec![ContentBlock::text(parsed.task.clone())],
                attach_db_inputs: Vec::new(),
                attach_file_inputs: Vec::new(),
                emit_user_blocks: false,
                incoming_source: None,
                sender_name: None,
                pre_materialized: false,
                tools_enabled: true,
                model_override: None,
                cancel_token: child_cancel,
                fallback: crate::harness::fallback_plan::delegation_fallback_plan(
                    &app,
                    &ctx.pool,
                    &creds.agent,
                    &ctx.agent_id,
                )
                .await,
            },
        )
        .await?;
        // spawn 成功：注销责任移交回投任务（它退出时子 loop 必已结束）
        let _ = scopeguard::ScopeGuard::into_inner(cancel_guard);

        // 回投任务：等完成信号 → 结果/失败事实投回父会话（internal_chain 跳过
        // 项目边界——父子关系自证；is_reply=true 走 accept 自动消费 + 免 hold 扣）
        let report_app = app.clone();
        let report_pool = ctx.pool.clone();
        let report_child = child_conv_id.clone();
        let report_parent = ctx.conv_id.clone();
        let report_expert = target.name.clone();
        let report_task = crate::infra::strings::truncate_to_byte_boundary(
            parsed.task.trim(),
            80,
            Some("…"),
        )
        .to_string();
        tauri::async_runtime::spawn(async move {
            let summary = match done_rx.await {
                Ok(s) => s,
                Err(_) => {
                    tracing::warn!(
                        target: "ice_paw.delegate",
                        child_conv = %report_child,
                        "异步委派子会话异常退出（未产出完成信号）——回投失败事实"
                    );
                    deliver_completion_report(
                        &report_app,
                        &report_pool,
                        &report_child,
                        &report_parent,
                        &report_expert,
                        &report_task,
                        None,
                        Some(("异常中止", 0, "子会话流式循环异常退出，未产出完成信号")),
                    )
                    .await;
                    return;
                }
            };
            let normal =
                crate::harness::mcp::delegate::is_normal_completion(&summary.finish_reason);
            let (final_text, abnormal) = if normal {
                (Some(summary.final_text.clone()), None)
            } else {
                (
                    None,
                    Some((
                        summary.finish_reason.clone(),
                        summary.rounds,
                        "任务未正常完成，结果可能不完整".to_string(),
                    )),
                )
            };
            deliver_completion_report(
                &report_app,
                &report_pool,
                &report_child,
                &report_parent,
                &report_expert,
                &report_task,
                final_text.as_deref(),
                abnormal.as_ref().map(|(r, rounds, note)| {
                    (r.as_str(), *rounds, note.as_str())
                }),
            )
            .await;
        });

        // --- 6. 受理回执（立刻返回——异步语义） ---
        Ok(serde_json::json!({
            "status": "accepted",
            "child_conversation_id": child_conv_id,
            "agent_name": target.name,
            "note": "任务已受理，专家正在独立会话执行。⚠ 重要：完成汇报只在你结束当前回合后才会到达（它排在队列里等你会话空闲）——如果你在本回合内等待或轮询，汇报永远不会到来（互等）。请直接收尾本回合或继续其他工作；汇报到达时会自动开启你的新回合，届时再转告用户即可。可用 stop_delegate(child_conversation_id) 取消。"
        })
        .to_string())
    }
}

/// 完成回投（批 P2-1）：把终态事实投回父会话。
/// 正常完成附结果摘要（截 1500 字符——投递上限内的务实平衡，全文在子会话）；
/// 异常终态如实披露 reason/轮数/提示。回投失败仅 warn（子会话事实已完整落库）。
#[allow(clippy::too_many_arguments)]
async fn deliver_completion_report(
    app: &tauri::AppHandle,
    pool: &sqlx::SqlitePool,
    child_conv_id: &str,
    parent_conv_id: &str,
    expert_name: &str,
    task_summary: &str,
    final_text: Option<&str>,
    abnormal: Option<(&str, u32, &str)>,
) {
    // 源 = 子会话（专家）；读取失败降级用已知字段拼 SourceInfo
    let source = match repo::conversation::get_by_id(pool, child_conv_id).await {
        Ok(c) => crate::harness::inbox::SourceInfo {
            conv_id: c.id.clone(),
            conv_title: c.title.clone(),
            agent_id: c.agent_id.clone(),
            agent_name: expert_name.to_string(),
            project_id: c.project_id.clone(),
        },
        Err(_) => crate::harness::inbox::SourceInfo {
            conv_id: child_conv_id.to_string(),
            conv_title: task_summary.to_string(),
            agent_id: String::new(),
            agent_name: expert_name.to_string(),
            project_id: None,
        },
    };
    let content = if let Some(text) = final_text.map(str::trim).filter(|t| !t.is_empty()) {
        let digest = crate::infra::strings::truncate_to_byte_boundary(text, 1500, Some("…"));
        format!(
            "【异步委派完成】{expert_name} 已完成任务「{task_summary}」。\n结果摘要：\n{digest}\n\n（完整过程见子会话轨迹 {child_conv_id}）"
        )
    } else if let Some((reason, rounds, note)) = abnormal {
        format!(
            "【异步委派异常结束】{expert_name} 的任务「{task_summary}」以 {reason} 终止（{rounds} 轮）——{note}。\n子会话已落库的部分可在轨迹查看（{child_conv_id}），可考虑重派或换人。"
        )
    } else {
        format!(
            "【异步委派完成】{expert_name} 已完成任务「{task_summary}」（最终回复为空）。\n子会话轨迹：{child_conv_id}"
        )
    };
    let content = crate::infra::strings::truncate_to_byte_boundary(
        content.trim(),
        7500,
        Some("…（超长截断，完整内容见子会话）"),
    )
    .to_string();
    if let Err(e) = crate::harness::inbox::deliver(
        app,
        pool,
        &source,
        parent_conv_id,
        &content,
        false,
        true,  // is_reply：accept 自动消费 + 免 hold 扣（既有对话回路的延续）
        true,  // internal_chain：父子关系自证，跳过项目边界（散落会话不误拦）
    )
    .await
    {
        tracing::warn!(
            target: "ice_paw.delegate",
            parent_conv = %parent_conv_id,
            child_conv = %child_conv_id,
            "异步委派完成回投失败: {e}"
        );
    }
}

// =========================================================================
// stop_delegate
// =========================================================================

pub struct StopDelegateTool;

#[derive(Deserialize)]
struct StopDelegateArgs {
    child_conversation_id: String,
}

#[async_trait]
impl McpClient for StopDelegateTool {
    fn name(&self) -> &str {
        "stop_delegate"
    }

    fn description(&self) -> &str {
        "Cancel a running async delegation (started via start_delegate) by its \
child_conversation_id. The expert's session is stopped gracefully; its already \
persisted work stays in the trajectory."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "child_conversation_id": {
                    "type": "string",
                    "description": "The conversation id from start_delegate's receipt."
                }
            },
            "required": ["child_conversation_id"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "stop_delegate 必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: StopDelegateArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("stop_delegate 参数解析失败: {e}")))?;
        // 只能停自己发起的异步委派（parent 边硬校验——不给人停别人的任务）
        let child = repo::conversation::get_by_id(&ctx.pool, &parsed.child_conversation_id)
            .await
            .map_err(|_| {
                AppError::Validation(format!(
                    "任务会话不存在（{}）——可能已完成或被删除",
                    parsed.child_conversation_id
                ))
            })?;
        if child.parent_conversation_id.as_deref() != Some(ctx.conv_id.as_str()) {
            return Err(AppError::Validation(format!(
                "任务会话 {} 不是本会话发起的委派——只能停止自己发起的任务",
                parsed.child_conversation_id
            )));
        }
        let app = ctx.app_handle.clone().ok_or_else(|| {
            AppError::Internal("stop_delegate 需要 App 上下文（app_handle）".into())
        })?;
        let chat_state = app.state::<ChatState>().inner().clone();
        let hit = chat_state.stop(&parsed.child_conversation_id);
        if hit {
            tracing::info!(
                target: "ice_paw.delegate",
                parent_conv = %ctx.conv_id,
                child_conv = %parsed.child_conversation_id,
                "异步委派被父 agent 停止（cancel 级联）"
            );
            Ok(serde_json::json!({
                "status": "cancelled",
                "child_conversation_id": parsed.child_conversation_id,
                "note": "已请求停止：专家会话将优雅收场（已落库内容保留在轨迹），完成后会有终止回投。"
            })
            .to_string())
        } else {
            Ok(serde_json::json!({
                "status": "not_running",
                "child_conversation_id": parsed.child_conversation_id,
                "note": "该任务不在运行中（可能已完成或已停止）。"
            })
            .to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    /// 受理回执形状锁：异步语义的核心契约（accepted + child id + 不含结果字段）。
    /// 附死锁警告文案锁（2026-11-13 生产实案：模型在回合内 ping 轮询 21 分钟等
    /// 一个「只有回合结束才会被消费」的回投——note 必须把互等说透，防文案回退）。
    #[test]
    fn receipt_shape_is_accepted_not_result() {
        let receipt: serde_json::Value = serde_json::from_str(
            r#"{"status":"accepted","child_conversation_id":"c-1","agent_name":"专家",
                "note":"任务已受理，专家正在独立会话执行。⚠ 重要：完成汇报只在你结束当前回合后才会到达（它排在队列里等你会话空闲）——如果你在本回合内等待或轮询，汇报永远不会到来（互等）。请直接收尾本回合或继续其他工作；汇报到达时会自动开启你的新回合，届时再转告用户即可。可用 stop_delegate(child_conversation_id) 取消。"}"#,
        )
        .unwrap();
        assert_eq!(receipt["status"], "accepted");
        assert!(receipt.get("response").is_none(), "异步回执不得携带任务结果");
        assert!(receipt.get("finish_reason").is_none());
        assert!(receipt["note"].as_str().unwrap().contains("互等"), "死锁警告必须在回执里");
    }
}
