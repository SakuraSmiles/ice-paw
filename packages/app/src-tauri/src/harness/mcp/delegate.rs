//! `delegate_to_agent` 工具 v2 —— 委派会话化（MA-1）
//!
//! 主 agent 在 stream_loop 中调用此工具，后端为专家 agent 建一个**真子会话**
//! （kind='delegation'），经 [`session_runner::run_agent_turn`] 跑完整回合
//! （自己的模型/key/工具/预算/hooks），全程落 session_events——取代 v1 的
//! 「主 agent 的 provider 单轮隐形调用」（违反不变式 3：无 session 不留痕）。
//!
//! ## 生命周期（docs/multi-agent-architecture.md §4.3）
//!
//! 1. 校验：目标存在、≠ 自己、∈ 可调度集合（[`resolve_dispatchable`：项目成员
//!    优先，否则全部 agent]）、凭据可读——失败诚实回 Err 给主 LLM；
//! 2. 建子会话（kind='delegation'，继承 project_id，parent_conversation_id=父）；
//! 3. cancel 级联：父 loop 的 token `.child_token()` 注册进 ChatState——父「停止
//!    生成」一键停整棵委派树；子 loop 退出自注销（RAII）；
//! 4. 专家跑完整 loop，inline await 完成信号（[`TurnSummary`]，数据源=turn_ended
//!    事件 + 最终正文——「真相在产物」）；壁钟护栏 15min 只兜「永不回来」的异常，
//!    正常终止器是子 loop 自己的 budget/stuck；
//! 5. tool_result 回传 child_conversation_id + 终态摘要（主 LLM 可引用/续派）；
//!    另附 `progress` 机器事实报告（子会话 session_events 提取的成功工具计数 /
//!    最后失败 / 涉及文件）——正常完成也带，模型自述 ≠ 事实，验收/核对由此起。
//!
//! ## 授权与深度护栏
//!
//! - 授权：委派本身需用户批准（2026-09-03 信任决策前置到委托时刻——
//!   `chat:delegation-auth-request` 弹卡，「逐次审批（默认）/ 命令免问」
//!   两档，见 execute_with_context 步骤 2.5）。工具保持
//!   `AuthorizationLevel::Always`（通用授权层不弹），决策点是工具内部
//!   治理层（屏幕共享通道同款先例）。子会话内部工具授权不变（独立授权
//!   记忆，敏感操作仍过用户手）；「命令免问」档 = 预授 seed 写子会话
//!   授权记忆的 run_command 工具档（父会话零污染）。
//! - 委派深度 = 1：工具注册按会话 kind 判定（`session_runner` 组装期仅对
//!   kind='chat' 注入本工具），delegation 子会话拿不到它——「A委派B、B委派回A」
//!   的乒乓球在结构上不可能。
//!
//! ## 串行边界（v1 接受）
//!
//! `execute_tool_round` 顺序执行工具 → 同轮多个委派串行。专家调用天然分钟级，
//! 串行可预期；并行 fan-out 列为 MA-1.5。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tauri::{Emitter, Manager};
use uuid::Uuid;

use crate::commands::agent_cmd::AgentCmd;
use crate::db::models::NewConversation;
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::chat_state::ChatState;
use crate::harness::provider;
use crate::harness::read_route::ReadRouteRegistry;
use crate::harness::session_runner::{self, AgentTurnInput, TurnEnv};
use crate::infra::protocol::{
    ContentBlock, DelegationAuthRequestPayload, DelegationGrant, DelegationStartedPayload,
};

use super::client::{McpClient, ToolContext};
use super::manager::McpServerManager;
use super::types::AuthorizationLevel;
use super::McpRegistry;

/// 壁钟护栏：子 loop 自身的 budget/stuck 是主要终止器，这里只兜「永不回来」
/// 的异常挂死（如 provider 流卡死不响应 cancel）。超时先 cancel 子会话再回 Err。
const WALL_CLOCK_GUARD_SECS: u64 = 15 * 60;

pub struct DelegateTool;

#[derive(Deserialize)]
struct DelegateArgs {
    agent_id: String,
    task: String,
}

/// 可调度 agent 的最小档案（system prompt 清单注入 + 委派目标解析共用）。
pub(crate) struct DispatchableAgent {
    pub id: String,
    pub name: String,
}

/// 输入侧校验（纯函数，可测）。task 是专家看到的**全部**输入（无任何上下文
/// 兜底），空任务无意义且必然被专家自由发挥填补——手测坐实：空 task 被
/// 解读成「读仓库 README」。与 update_plan 的 text 非空校验同一纪律。
fn validate_args(parsed: &DelegateArgs) -> AppResult<()> {
    if parsed.task.trim().is_empty() {
        return Err(AppError::Validation(
            "task 为空——委派任务必须自包含（这是专家看到的全部输入），请把要做的事写具体".into(),
        ));
    }
    Ok(())
}

/// 终止原因是否为「正常完成」。stop（OpenAI 系）与 end_turn（Anthropic 系
/// 原样透传）同义（前端 finishReasonLabels 同样将两者视为正常）——只认
/// 单值会把所有 Anthropic 系子会话的正常完成误标「未正常完成」（手测坐实）。
/// 其余（length/max_tokens/budget_exceeded/stuck/abort/tool_use…）response
/// 可能为空或不完整，须附 note 提醒委派方。
fn is_normal_completion(reason: &str) -> bool {
    matches!(reason, "stop" | "end_turn")
}

// =========================================================================
// 进度报告（D15 八波④）——子会话 session_events 的机器事实提取
// =========================================================================

/// 进度报告的文件清单上限（防大会话膨胀；超出截断 + 计数披露）。
const PROGRESS_MAX_FILES: usize = 8;

/// 子会话进度报告：成功工具调用聚合计数 + 最后一次失败 + 涉及文件。
///
/// **机器事实，不是模型自述**——正常完成也带（弱模型常「自认为完成」漏报
/// 样式/漏表，跨核对靠事件流）；子会话被预算掐死时统筹者也能看到做到哪了。
/// 纯 SELECT 无广播副作用；DB 读失败降级为零值报告（进度是增补信息，
/// 不让诊断噪声挡住主结果回传）。
async fn collect_progress(pool: &sqlx::SqlitePool, conv_id: &str) -> serde_json::Value {
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    let mut total: usize = 0;
    let mut files: Vec<String> = Vec::new();
    if let Ok(calls) = repo::session_event::list_successful_tool_calls(pool, conv_id).await {
        for (tool, args) in &calls {
            *counts.entry(tool.clone()).or_default() += 1;
            total += 1;
            if let Some(p) = crate::harness::references::artifact_path(tool, args) {
                if !files.contains(&p) {
                    files.push(p);
                }
            }
        }
    }
    let last_error = repo::session_event::list_failed_tool_calls(pool, conv_id)
        .await
        .ok()
        .and_then(|fails| fails.into_iter().next_back())
        .map(|(tool, msg)| {
            serde_json::json!({
                "tool": tool,
                "message": crate::infra::strings::truncate_to_byte_boundary(&msg, 200, Some("…")),
            })
        });
    let truncated_files = files.len() > PROGRESS_MAX_FILES;
    serde_json::json!({
        "total_successful_tool_calls": total,
        "successful_tool_calls": counts,
        "last_error": last_error,
        "files_touched": {
            "paths": files.iter().take(PROGRESS_MAX_FILES).cloned().collect::<Vec<_>>(),
            "more": if truncated_files { Some(files.len() - PROGRESS_MAX_FILES) } else { None },
        }
    })
}

/// Err 路径（壁钟超时 / 子循环异常退出）附带的紧凑进度行——AppError 只带
/// 文本，塞不进 JSON 结构，退而求其次给一行可读摘要。
async fn progress_summary_line(pool: &sqlx::SqlitePool, conv_id: &str) -> String {
    let v = collect_progress(pool, conv_id).await;
    let total = v["total_successful_tool_calls"].as_u64().unwrap_or(0);
    if total == 0 {
        return "子会话无成功工具调用记录（可能死于起步阶段）。".into();
    }
    let counts = v["successful_tool_calls"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, n)| format!("{k}×{}", n.as_u64().unwrap_or(0)))
                .collect::<Vec<_>>()
                .join("、")
        })
        .unwrap_or_default();
    let mut line = format!("子会话机器事实：成功工具调用 {total} 次（{counts}）");
    if let Some(err) = v["last_error"].as_object() {
        let tool = err.get("tool").and_then(|t| t.as_str()).unwrap_or("?");
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("");
        line.push_str(&format!("；最后失败：{tool}（{msg}）"));
    }
    line.push('。');
    line
}

/// 解析可调度集合（§4.3.1 回退规则）。
///
/// - conv 所属项目**配了成员**（project_agents 非空）→ 该项目成员集合；
/// - 否则（无项目 / 成员为空）→ **全部 agent**——散落会话零摩擦回退，
///   可选配置而非强制前置，存量会话零迁移天然全量可调度。
///
/// 两种情况都剔除发起方自己（自我委派由 [`DelegateTool`] 再单独报友好错误）。
/// 项目成员引用的 agent 已删 → 跳过（project_agents 无 FK，agent 可删）。
pub(crate) async fn resolve_dispatchable(
    pool: &sqlx::SqlitePool,
    project_id: Option<&str>,
    initiator_agent_id: &str,
) -> AppResult<Vec<DispatchableAgent>> {
    let member_ids: Vec<String> = match project_id {
        Some(pid) => repo::project::list_agents(pool, pid)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.agent_id)
            .collect(),
        None => Vec::new(),
    };
    let rows = if member_ids.is_empty() {
        repo::agent::list(pool).await?
    } else {
        let mut v = Vec::with_capacity(member_ids.len());
        for id in &member_ids {
            if let Ok(a) = repo::agent::get_by_id(pool, id).await {
                v.push(a);
            }
        }
        v
    };
    Ok(rows
        .into_iter()
        .filter(|a| a.id != initiator_agent_id)
        .map(|a| DispatchableAgent {
            id: a.id,
            name: a.name,
        })
        .collect())
}

/// 可调度清单 → system prompt 注入段（`PipelineContext.delegation_hint`）。
///
/// 同时携带 id 与 name：agent_id 参数收两者任一（见 [`resolve_target`]），
/// 清单里明示对应关系，避免 LLM 拿 name 当 id 猜。
pub(crate) fn build_dispatch_hint(agents: &[DispatchableAgent]) -> String {
    let lines: Vec<String> = agents
        .iter()
        .map(|a| format!("- {}（agent_id: {}）", a.name, a.id))
        .collect();
    format!(
        "你可以通过 delegate_to_agent 工具把子任务委派给以下 agent（专家会用它自己的模型、\
         系统提示词和工具完整执行任务并回传结果）：\n{}\n\
         注意：task 文本必须自包含（这是专家看到的全部输入）；接收方不能再委派他人，\
         需要多方意见时由你分别委派。",
        lines.join("\n")
    )
}

/// 目标解析：先 id 精确匹配，退 name 精确匹配（重名歧义 → None，走「不在集合」错误）。
fn resolve_target<'a>(
    agents: &'a [DispatchableAgent],
    id_or_name: &str,
) -> Option<&'a DispatchableAgent> {
    if let Some(x) = agents.iter().find(|a| a.id == id_or_name) {
        return Some(x);
    }
    let mut by_name = agents.iter().filter(|a| a.name == id_or_name);
    let first = by_name.next()?;
    if by_name.next().is_some() {
        return None; // 重名歧义：不猜，让调用方用 id 重试
    }
    Some(first)
}

#[async_trait]
impl McpClient for DelegateTool {
    fn name(&self) -> &str {
        "delegate_to_agent"
    }

    fn description(&self) -> &str {
        "Delegate a self-contained sub-task to another agent. The target agent runs a full \
         autonomous session with its own model, system prompt and tools, then returns its \
         final answer. Use it when a sub-task needs another agent's specialty (different \
         model, tools or knowledge). The `task` text is all the target agent sees - include \
         all necessary context in it. The target cannot delegate further; for multiple \
         opinions, delegate to each agent separately. The result JSON always includes a \
         `progress` object of machine facts from the child session (successful tool call \
         counts, last failed tool, files touched) - use it to verify the target's \
         self-report instead of trusting the final text alone."
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

    /// 工具本身恒 Allow（通用授权层不弹窗）——委派授权是工具内部决策点
    /// （execute_with_context 步骤 2.5 自弹 `chat:delegation-auth-request`，
    /// 屏幕共享通道同款先例），不与通用 Confirm 通道纠缠（scope 三档语义
    /// 错位、grant 传递只能靠跨函数暂存）。
    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "delegate_to_agent 必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: DelegateArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("delegate_to_agent 参数解析失败: {e}")))?;
        validate_args(&parsed)?;

        // run_agent_turn 的环境依赖全部经 Tauri managed state 取（工具轮已注入 app_handle）
        let app = ctx.app_handle.clone().ok_or_else(|| {
            AppError::Internal("delegate_to_agent 需要 App 上下文（app_handle）".into())
        })?;

        // --- 1. 校验：父会话 + 目标 ∈ 可调度集合 ---
        let parent = repo::conversation::get_by_id(&ctx.pool, &ctx.conv_id).await?;
        if parsed.agent_id == ctx.agent_id {
            return Err(AppError::Validation(
                "不能委派给自己——请指定一个不同的 agent".into(),
            ));
        }
        let dispatchable =
            resolve_dispatchable(&ctx.pool, parent.project_id.as_deref(), &ctx.agent_id).await?;
        let target = resolve_target(&dispatchable, &parsed.agent_id).ok_or_else(|| {
            let roster = dispatchable
                .iter()
                .map(|a| format!("{}({})", a.name, a.id))
                .collect::<Vec<_>>()
                .join("、");
            AppError::Validation(format!(
                "目标 agent '{}' 不在可调度集合内。当前可调度：[{roster}]。\
                 （项目配置了成员时仅成员可调度；请用上列 id 或名称重试）",
                parsed.agent_id
            ))
        })?;

        // --- 2. 专家档案 + 凭据（专家用自己的模型，与主 agent 无关） ---
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
        let expert_provider = provider::create_provider(
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

        // --- 2.5 委派授权：信任决策前置到委托时刻（2026-09-03 两档拍板）---
        // 委派是「子 agent 将以自己的模型/工具跑完整子会话」的高信任动作——用户
        // 在此决定放行与否，并可选「命令免问」预授权（子会话 run_command 工具
        // 档）。工具本身保持 Always 级（通用授权层零改动），决策点是工具内部
        // 治理层（屏幕共享通道同款先例）；应答与工具授权共用同一 oneshot
        // registry + respond_tool_auth 命令，前端审批卡/后台栈/toast 全复用。
        let parent_cancel = ctx
            .cancel
            .clone()
            .ok_or_else(|| AppError::Internal("delegate_to_agent 缺少父会话取消令牌".into()))?;
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
            task: parsed.task.clone(),
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
            "等待用户委派授权（120s 超时）"
        );
        let auth_emitter =
            crate::harness::r#loop::emitter::tauri_emitter(app.clone(), ctx.conv_id.clone());
        let grant = match crate::harness::tool_executor::wait_for_auth_response(
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
                tracing::info!(
                    target: "ice_paw.delegate",
                    to_agent = %target.id,
                    "用户拒绝了委派"
                );
                return Err(AppError::AuthorizationRequired {
                    tool: "delegate_to_agent".into(),
                    reason: format!(
                        "用户拒绝了这次委派（目标 {}，任务：{}…）——请与用户确认意图\
                         后再试，或改为在当前会话直接完成",
                        target.name,
                        crate::infra::strings::truncate_to_byte_boundary(
                            parsed.task.trim(),
                            40,
                            Some("…")
                        )
                    ),
                });
            }
            // 取消/超时与 wait_for_auth_response 通用语义一致（其内部已清
            // registry + emit cancel 事件），委派未发生、零残留
            None => {
                return Err(AppError::AuthorizationRequired {
                    tool: "delegate_to_agent".into(),
                    reason: "委派授权被取消或超时（120 秒内未获用户批准）".into(),
                });
            }
        };

        // --- 3. 建子会话（kind='delegation'，继承项目，挂父边） ---
        // 标题 = 裸 task 文本（UX #4：「委派: 」前缀与正文冗余——上下文里
        // kind/父边/agent 已各自可见，标题只负责可读的任务摘要）。旧数据的
        // 前缀由前端展示侧归一剥离，不做 migration。
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
            },
        )
        .await?;

        // --- 4. cancel 级联 + ChatState 注册（早退路径 RAII 兜底注销） ---
        let child_cancel = parent_cancel.child_token();
        let chat_state = app.state::<ChatState>().inner().clone();
        chat_state.register(&child_conv_id, child_cancel.clone());
        let guard_conv_id = child_conv_id.clone();
        let cancel_guard = scopeguard::guard((), |_| chat_state.unregister(&guard_conv_id));

        // 子会话已入库且必可跳转 → 即刻通知前端（运行中委派卡片/任务胶囊可达；
        // 前端刷新会话列表拿到子会话行）。emit 失败不影响委派本身。
        let _ = app.emit(
            "chat:delegation-started",
            DelegationStartedPayload {
                conversation_id: ctx.conv_id.clone(),
                child_conversation_id: child_conv_id.clone(),
                agent_name: target.name.clone(),
                title: title.clone(),
            },
        );

        tracing::info!(
            target: "ice_paw.delegate",
            parent_conv = %ctx.conv_id,
            child_conv = %child_conv_id,
            from_agent = %ctx.agent_id,
            to_agent = %target.id,
            task_chars = parsed.task.len(),
            "委派开始：专家跑完整子会话"
        );

        // --- 4.5 委派预授权 seed：命令免问档 → 子会话授权记忆的 run_command 工具档 ---
        // 必须在 run_agent_turn spawn 前完成（子 loop 启动即可能执行命令）；
        // 只写子会话的表（父会话零污染），生效路径 = Confirm 级判定的
        // is_tool_authorized 分支（run_command 免弹）。屏幕家族走通道治理，
        // 不进预授权（不变式 3）。
        if grant == Some(DelegationGrant::Commands) {
            app.state::<crate::harness::authority::AuthSessionRegistry>()
                .inner()
                .session_for(&child_conv_id)
                .mark_tool_authorized("run_command")
                .await;
            tracing::info!(
                target: "ice_paw.delegate",
                child_conv = %child_conv_id,
                "委派预授权: commands 档（子会话 run_command 免问）"
            );
        }

        // --- 5. 专家跑完整回合 + inline await 完成信号 ---
        let task_text = parsed.task.clone();
        let done_rx = session_runner::run_agent_turn(
            &TurnEnv {
                emitter: crate::harness::r#loop::emitter::tauri_emitter(
                    app.clone(),
                    child_conv_id.clone(),
                ),
                tool_app: Some(app.clone()),
                pool: ctx.pool.clone(),
                route_registry: app.state::<ReadRouteRegistry>().inner(),
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
                // 专家 agent 写交付物文档同样吃到其 yaml 的样式偏好（hooks 同款旁路）
                word_style_profile: creds.word_style_profile,
                provider: expert_provider,
                api_key: creds.api_key,
                user_msg_id: Uuid::new_v4().to_string(),
                content_text: task_text.clone(),
                llm_blocks: vec![ContentBlock::text(task_text.clone())],
                persist_blocks: vec![ContentBlock::text(task_text)],
                attach_db_inputs: Vec::new(),
                attach_file_inputs: Vec::new(),
                emit_user_blocks: false,
                tools_enabled: true,
                model_override: None,
                cancel_token: child_cancel,
            },
        )
        .await?;
        // spawn 成功：子会话注销责任移交子 loop 的 RAII 守卫（任何退出路径必注销）
        let _ = scopeguard::ScopeGuard::into_inner(cancel_guard);

        // --- 6. 壁钟护栏 await（子 loop 的 budget/stuck 才是主要终止器） ---
        let summary =
            match tokio::time::timeout(Duration::from_secs(WALL_CLOCK_GUARD_SECS), done_rx).await {
                Ok(Ok(summary)) => summary,
                // 超时：先 cancel 子会话（其 loop 会走 finalize_cancel 自清）再回 Err；
                // 附机器事实进度行——统筹者拿到「做到哪了」而非只有一句超时
                Err(_) => {
                    chat_state.stop(&child_conv_id);
                    let prog = progress_summary_line(&ctx.pool, &child_conv_id).await;
                    return Err(AppError::Internal(format!(
                        "委派超时（{} 分钟壁钟护栏）——子会话 {child_conv_id} 已被取消，\
                     请缩小任务范围或换一个 agent。{prog}",
                        WALL_CLOCK_GUARD_SECS / 60
                    )));
                }
                // RecvError：spawn 任务在发送完成信号前消失（panic / runtime 关闭）
                Ok(Err(_)) => {
                    let prog = progress_summary_line(&ctx.pool, &child_conv_id).await;
                    return Err(AppError::Internal(
                        format!("子会话流式循环异常退出（未产出完成信号）——请如实告知用户该委派失败，\
                     可在轨迹页查看子会话已落库的部分。{prog}"),
                    ));
                }
            };

        tracing::info!(
            target: "ice_paw.delegate",
            parent_conv = %ctx.conv_id,
            child_conv = %child_conv_id,
            to_agent = %target.id,
            finish_reason = %summary.finish_reason,
            rounds = summary.rounds,
            response_chars = summary.final_text.len(),
            "委派完成：TurnSummary 回传主 agent"
        );

        // --- 7. tool_result：子会话 id + 终态摘要（主 LLM 可引用/续派） ---
        let mut result = serde_json::json!({
            "child_conversation_id": child_conv_id,
            "agent_name": creds.agent.name,
            "finish_reason": summary.finish_reason,
            "rounds": summary.rounds,
            "response": summary.final_text,
        });
        if let Some(u) = &summary.usage {
            result["tokens"] = serde_json::json!({
                "prompt": u.prompt_tokens,
                "completion": u.completion_tokens,
            });
        }
        if !is_normal_completion(&summary.finish_reason) {
            result["note"] = serde_json::Value::String(format!(
                "子会话未正常完成（finish_reason={}），response 可能为空或不完整",
                summary.finish_reason
            ));
        }
        // 机器事实进度报告（正常完成也带——模型自述≠事实，跨核对由此起）
        result["progress"] = collect_progress(&ctx.pool, &child_conv_id).await;
        Ok(result.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, name: &str) -> DispatchableAgent {
        DispatchableAgent {
            id: id.into(),
            name: name.into(),
        }
    }

    #[test]
    fn resolve_target_matches_id_first() {
        let list = vec![agent("a1", "翻译"), agent("a2", "写作")];
        let t = resolve_target(&list, "a2").unwrap();
        assert_eq!(t.id, "a2");
    }

    #[test]
    fn resolve_target_falls_back_to_unique_name() {
        let list = vec![agent("a1", "翻译"), agent("a2", "写作")];
        let t = resolve_target(&list, "翻译").unwrap();
        assert_eq!(t.id, "a1");
    }

    #[test]
    fn resolve_target_ambiguous_name_returns_none() {
        let list = vec![agent("a1", "写作"), agent("a2", "写作")];
        assert!(
            resolve_target(&list, "写作").is_none(),
            "重名不猜，逼调用方用 id"
        );
    }

    #[test]
    fn resolve_target_unknown_returns_none() {
        let list = vec![agent("a1", "翻译")];
        assert!(resolve_target(&list, "nope").is_none());
    }

    #[test]
    fn dispatch_hint_lists_id_and_name() {
        let list = vec![agent("a1", "翻译"), agent("a2", "写作")];
        let hint = build_dispatch_hint(&list);
        assert!(
            hint.contains("翻译（agent_id: a1）"),
            "清单须同时含 name 与 id: {hint}"
        );
        assert!(hint.contains("delegate_to_agent"));
        assert!(hint.contains("自包含"), "须提示 task 自包含");
    }

    #[test]
    fn validate_args_rejects_empty_task() {
        for task in ["", "   ", "\n\t"] {
            let args = DelegateArgs {
                agent_id: "a1".into(),
                task: task.into(),
            };
            assert!(
                validate_args(&args).is_err(),
                "空/纯空白 task（{task:?}）必须输入侧拒绝——专家看不到任何上下文，空任务只会被自由发挥填补"
            );
        }
    }

    #[test]
    fn validate_args_accepts_normal_task() {
        let args = DelegateArgs {
            agent_id: "a1".into(),
            task: "  翻译 README 首页  ".into(),
        };
        assert!(validate_args(&args).is_ok());
    }

    #[test]
    fn normal_completion_covers_both_provider_families() {
        // OpenAI 系正常 = stop；Anthropic 系原样透传 = end_turn——两者同义
        assert!(is_normal_completion("stop"));
        assert!(
            is_normal_completion("end_turn"),
            "手测坐实的误标根因：end_turn 是正常完成"
        );
    }

    #[test]
    fn normal_completion_rejects_abnormal_reasons() {
        for r in [
            "length",
            "max_tokens",
            "budget_exceeded",
            "stuck",
            "abort",
            "tool_use",
            "",
        ] {
            assert!(!is_normal_completion(r), "{r} 须附「可能不完整」note");
        }
    }

    // ------------------------------------------------------------------
    // 进度报告（D15 八波④）——in-memory sqlite + typed emitter 种子法
    //（session_events 无 FK，直接对任意 session_id 写事件即可）
    // ------------------------------------------------------------------

    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;
    use std::str::FromStr;

    async fn progress_pool() -> SqlitePool {
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
            .unwrap();
        // session_events.session_id 有 FK → conversations，须先落最小会话行
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES ('agent-1', 't', 'anthropic', 'claude-test', '', '', 0.7, 1024, '{}', 0, 0)",
        )
        .execute(&pool)
        .await
        .expect("seed agent");
        sqlx::query(
            "INSERT INTO conversations (id, agent_id, title) VALUES ('conv-child', 'agent-1', 't')",
        )
        .execute(&pool)
        .await
        .expect("seed conversation");
        pool
    }

    async fn seed_tool_call(
        pool: &SqlitePool,
        ctx: &crate::harness::event_log::EventCtx,
        n: usize,
        tool: &str,
        path: &str,
        is_error: bool,
    ) {
        let args = serde_json::json!({ "path": path, "op": "demo" }).to_string();
        crate::harness::event_log::log_tool_execution(
            pool,
            ctx,
            &format!("msg-{n}"),
            &format!("tc-{n}"),
            None,
            tool,
            &args,
            Some(if is_error { "失败示例：断言未过" } else { "ok" }),
            is_error,
            10,
        )
        .await;
    }

    #[tokio::test]
    async fn collect_progress_reports_counts_files_and_last_error() {
        let pool = progress_pool().await;
        let ctx = crate::harness::event_log::EventCtx::new("conv-child", "turn-1", "agent-1");
        // 2× edit_docx 同一文件（去重）+ 1× inspect_docx 另一文件 + 末位失败
        seed_tool_call(&pool, &ctx, 1, "edit_docx", "D:/doc/a.docx", false).await;
        seed_tool_call(&pool, &ctx, 2, "edit_docx", "D:/doc/a.docx", false).await;
        seed_tool_call(&pool, &ctx, 3, "inspect_docx", "D:/doc/b.docx", false).await;
        seed_tool_call(&pool, &ctx, 4, "edit_docx", "D:/doc/a.docx", true).await;

        let v = collect_progress(&pool, "conv-child").await;
        assert_eq!(v["total_successful_tool_calls"], 3, "失败不计入成功计数");
        assert_eq!(v["successful_tool_calls"]["edit_docx"], 2);
        assert_eq!(v["successful_tool_calls"]["inspect_docx"], 1);
        let paths = v["files_touched"]["paths"].as_array().unwrap();
        assert_eq!(paths.len(), 2, "同文件去重，异文件各自计入");
        assert!(v["files_touched"]["more"].is_null());
        let last = v["last_error"].as_object().expect("末位失败须上报");
        assert_eq!(last["tool"], "edit_docx");
        assert!(last["message"].as_str().unwrap().contains("断言未过"));
    }

    #[tokio::test]
    async fn collect_progress_empty_session_is_zero_report() {
        let pool = progress_pool().await;
        let v = collect_progress(&pool, "conv-empty").await;
        assert_eq!(v["total_successful_tool_calls"], 0);
        assert!(v["successful_tool_calls"].as_object().unwrap().is_empty());
        assert!(v["last_error"].is_null());
        assert!(v["files_touched"]["paths"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            progress_summary_line(&pool, "conv-empty").await,
            "子会话无成功工具调用记录（可能死于起步阶段）。"
        );
    }

    #[tokio::test]
    async fn progress_summary_line_renders_machine_facts() {
        let pool = progress_pool().await;
        let ctx = crate::harness::event_log::EventCtx::new("conv-child", "turn-1", "agent-1");
        seed_tool_call(&pool, &ctx, 1, "edit_docx", "D:/doc/a.docx", false).await;
        seed_tool_call(&pool, &ctx, 2, "validate_docx", "D:/doc/a.docx", false).await;
        seed_tool_call(&pool, &ctx, 3, "edit_docx", "D:/doc/a.docx", true).await;

        let line = progress_summary_line(&pool, "conv-child").await;
        assert!(line.contains("成功工具调用 2 次"), "{line}");
        assert!(line.contains("edit_docx×1"), "BTreeMap 名序聚合计数：{line}");
        assert!(line.contains("validate_docx×1"), "{line}");
        assert!(line.contains("最后失败：edit_docx"), "{line}");
        assert!(line.ends_with('。'), "{line}");
    }

    #[tokio::test]
    async fn collect_progress_caps_file_list_at_eight() {
        let pool = progress_pool().await;
        let ctx = crate::harness::event_log::EventCtx::new("conv-child", "turn-1", "agent-1");
        for i in 0..10 {
            seed_tool_call(&pool, &ctx, i, "write_file", &format!("D:/doc/f{i}.md"), false).await;
        }
        let v = collect_progress(&pool, "conv-child").await;
        let paths = v["files_touched"]["paths"].as_array().unwrap();
        assert_eq!(paths.len(), PROGRESS_MAX_FILES, "清单截到上限");
        assert_eq!(v["files_touched"]["more"], 10 - PROGRESS_MAX_FILES, "截断计数披露");
        assert_eq!(v["total_successful_tool_calls"], 10, "计数不截断");
    }
}
