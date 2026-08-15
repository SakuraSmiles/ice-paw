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
//! 5. tool_result 回传 child_conversation_id + 终态摘要（主 LLM 可引用/续派）。
//!
//! ## 授权与深度护栏
//!
//! - 授权：项目组内免弹窗（`AuthorizationLevel::Always`；设计稿称 Silent）。
//!   边界是可调度集合校验，不是弹窗。子会话内部工具授权不变（独立
//!   PathAuthSession，敏感操作仍过用户手）。
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
use tauri::Manager;
use uuid::Uuid;

use crate::commands::agent_cmd::AgentCmd;
use crate::db::models::NewConversation;
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::chat_state::ChatState;
use crate::harness::provider;
use crate::harness::read_route::ReadRouteRegistry;
use crate::harness::session_runner::{self, AgentTurnInput, TurnEnv};
use crate::infra::protocol::ContentBlock;

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
        .map(|a| DispatchableAgent { id: a.id, name: a.name })
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
         opinions, delegate to each agent separately."
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

    /// 项目组内免弹窗（设计稿称 Silent；现有枚举的 Always 即免授权）。
    /// 边界是可调度集合校验（execute_with_context 内），不是弹窗。
    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "delegate_to_agent 必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: DelegateArgs = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("delegate_to_agent 参数解析失败: {e}"))
        })?;

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
        let creds = agent_cmd.get_with_credentials(&target.id).await.map_err(|e| {
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

        // --- 3. 建子会话（kind='delegation'，继承项目，挂父边） ---
        let title = format!(
            "委派: {}",
            crate::infra::strings::truncate_to_byte_boundary(parsed.task.trim(), 60, Some("…"))
        );
        let child_conv_id = Uuid::new_v4().to_string();
        let child_conv = repo::conversation::create(
            &ctx.pool,
            &child_conv_id,
            &NewConversation {
                agent_id: creds.agent.id.clone(),
                title: Some(title),
                project_id: parent.project_id.clone(),
                kind: Some("delegation".into()),
                initiator_agent_id: Some(ctx.agent_id.clone()),
                parent_conversation_id: Some(ctx.conv_id.clone()),
            },
        )
        .await?;

        // --- 4. cancel 级联 + ChatState 注册（早退路径 RAII 兜底注销） ---
        let parent_cancel = ctx.cancel.clone().ok_or_else(|| {
            AppError::Internal("delegate_to_agent 缺少父会话取消令牌".into())
        })?;
        let child_cancel = parent_cancel.child_token();
        let chat_state = app.state::<ChatState>().inner().clone();
        chat_state.register(&child_conv_id, child_cancel.clone());
        let guard_conv_id = child_conv_id.clone();
        let cancel_guard = scopeguard::guard((), |_| chat_state.unregister(&guard_conv_id));

        tracing::info!(
            target: "ice_paw.delegate",
            parent_conv = %ctx.conv_id,
            child_conv = %child_conv_id,
            from_agent = %ctx.agent_id,
            to_agent = %target.id,
            task_chars = parsed.task.len(),
            "委派开始：专家跑完整子会话"
        );

        // --- 5. 专家跑完整回合 + inline await 完成信号 ---
        let task_text = parsed.task.clone();
        let done_rx = session_runner::run_agent_turn(
            &TurnEnv {
                app: app.clone(),
                pool: ctx.pool.clone(),
                route_registry: app.state::<ReadRouteRegistry>().inner(),
                global_registry: Arc::clone(app.state::<Arc<McpRegistry>>().inner()),
                mcp_manager: Arc::clone(app.state::<Arc<McpServerManager>>().inner()),
                auth_registry: app
                    .state::<crate::harness::tool_executor::ToolAuthRegistry>()
                    .inner()
                    .clone(),
            },
            AgentTurnInput {
                conv: child_conv,
                agent: creds.agent.clone(),
                hooks: creds.hooks,
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
        let summary = match tokio::time::timeout(
            Duration::from_secs(WALL_CLOCK_GUARD_SECS),
            done_rx,
        )
        .await
        {
            Ok(Ok(summary)) => summary,
            // 超时：先 cancel 子会话（其 loop 会走 finalize_cancel 自清）再回 Err
            Err(_) => {
                chat_state.stop(&child_conv_id);
                return Err(AppError::Internal(format!(
                    "委派超时（{} 分钟壁钟护栏）——子会话 {child_conv_id} 已被取消，\
                     请缩小任务范围或换一个 agent",
                    WALL_CLOCK_GUARD_SECS / 60
                )));
            }
            // RecvError：spawn 任务在发送完成信号前消失（panic / runtime 关闭）
            Ok(Err(_)) => {
                return Err(AppError::Internal(
                    "子会话流式循环异常退出（未产出完成信号）——请如实告知用户该委派失败，\
                     可在轨迹页查看子会话已落库的部分"
                        .into(),
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
        if summary.finish_reason != "stop" {
            result["note"] = serde_json::Value::String(format!(
                "子会话未正常完成（finish_reason={}），response 可能为空或不完整",
                summary.finish_reason
            ));
        }
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
        assert!(resolve_target(&list, "写作").is_none(), "重名不猜，逼调用方用 id");
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
        assert!(hint.contains("翻译（agent_id: a1）"), "清单须同时含 name 与 id: {hint}");
        assert!(hint.contains("delegate_to_agent"));
        assert!(hint.contains("自包含"), "须提示 task 自包含");
    }
}
