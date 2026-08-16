//! `harness::hooks` — 对话钩子执行器
//!
//! 在对话生命周期点（见 [`crate::db::models::HookPoint`]）执行用户配置的钩子动作
//!（[`crate::db::models::HookAction`]）。由 chat_cmd / loop_engine / tool_executor /
//! cleanup 在各接入点调用 [`run_hooks`]。
//!
//! 设计要点：
//! - `InjectPrompt` 不直接改上下文，而是收集进 [`HookOutcome::injected_prompt`]，
//!   由调用方决定怎么注入（system_prompt / 临时消息）——保持注入逻辑在接入点本地。
//! - `CallTool` 失败仅 warn，不中断对话（钩子是辅助，不该让对话崩）。

use crate::db::models::{HookAction, HookConfig, HookPoint};
use crate::error::AppResult;
use crate::harness::mcp::client::{McpRegistry, ToolContext};

/// 钩子执行结果：收集的 InjectPrompt 片段（调用方负责注入 system/上下文）。
#[derive(Debug, Default, Clone)]
pub struct HookOutcome {
    /// 该点所有 InjectPrompt 动作的 content 拼接（"\n" 分隔）。无则 None。
    pub injected_prompt: Option<String>,
}

/// 该钩子点是否配置了动作。
///
/// 供各接入点在构造 `ToolContext`（含 workspace 解析的 DB 查询）前先做廉价判断：
/// 无动作则直接跳过，避免对未配置钩子的 agent 产生额外开销。
pub fn has_actions(hooks: &HookConfig, point: HookPoint) -> bool {
    hooks.get(&point).map(|v| !v.is_empty()).unwrap_or(false)
}

/// 执行某钩子点的所有动作。
///
/// - [`HookAction::InjectPrompt`] → 拼接进 `outcome.injected_prompt`（调用方注入）。
/// - [`HookAction::CallTool`] → `registry.dispatch(tool, args, tool_ctx)`（失败仅 warn）。
/// - [`HookAction::Log`] → `tracing::info!`（target=`ice_paw.hooks`）。
///
/// 无配置 / 该点无动作 → 快速返回默认 outcome。
pub async fn run_hooks(
    point: HookPoint,
    hooks: &HookConfig,
    tool_ctx: &ToolContext,
    registry: &McpRegistry,
) -> AppResult<HookOutcome> {
    let actions = match hooks.get(&point) {
        Some(a) if !a.is_empty() => a,
        _ => return Ok(HookOutcome::default()),
    };

    let mut injected: Vec<String> = Vec::new();
    for action in actions {
        match action {
            HookAction::InjectPrompt { content } => {
                injected.push(content.clone());
            }
            HookAction::CallTool { tool, args } => {
                match registry.dispatch_catch_panic(tool, args, tool_ctx).await {
                    Ok(_) => tracing::info!(
                        target: "ice_paw.hooks",
                        "钩子 CallTool 成功: point={:?} tool={}", point, tool
                    ),
                    Err(e) => tracing::warn!(
                        target: "ice_paw.hooks",
                        "钩子 CallTool 失败（忽略，不中断对话）: point={:?} tool={} err={}", point, tool, e
                    ),
                }
            }
            HookAction::Log { message } => {
                tracing::info!(target: "ice_paw.hooks", "钩子 Log: point={:?} {}", point, message);
            }
        }
    }

    let injected_prompt = if injected.is_empty() {
        None
    } else {
        Some(injected.join("\n"))
    };
    Ok(HookOutcome { injected_prompt })
}

// ==========================================================================
// 单元测试
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{HookAction, HookConfig, HookPoint};
    use std::collections::HashMap;

    /// 构造一个最小 ToolContext（仅 CallTool 路径会用，InjectPrompt/Log 不触碰其字段）
    async fn mk_ctx() -> ToolContext {
        ToolContext {
            conv_id: "c1".into(),
            agent_id: "a1".into(),
            project_id: None,
            workspace: None,
            pool: sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap(),
            api_key: None,
            app_handle: None,
            proposal_registry: None,
            turn_id: None,
            cancel: None,
        }
    }

    fn one_action(point: HookPoint, action: HookAction) -> HookConfig {
        let mut h: HookConfig = HashMap::new();
        h.insert(point, vec![action]);
        h
    }

    // -------- has_actions --------

    #[test]
    fn has_actions_empty_config_is_false() {
        let hooks: HookConfig = HashMap::new();
        assert!(!has_actions(&hooks, HookPoint::BeforeLlm));
        assert!(!has_actions(&hooks, HookPoint::ConversationStart));
    }

    #[test]
    fn has_actions_empty_vec_is_false() {
        let mut hooks: HookConfig = HashMap::new();
        hooks.insert(HookPoint::AfterTool, vec![]);
        assert!(
            !has_actions(&hooks, HookPoint::AfterTool),
            "空动作列表应视为无动作（避免无谓构造 ToolContext）"
        );
    }

    #[test]
    fn has_actions_detects_configured_point_only() {
        let hooks = one_action(
            HookPoint::BeforeLlm,
            HookAction::Log {
                message: "hi".into(),
            },
        );
        assert!(has_actions(&hooks, HookPoint::BeforeLlm));
        assert!(!has_actions(&hooks, HookPoint::ConversationEnd));
    }

    // -------- run_hooks：无配置 / 空动作 → 快速返回 --------

    #[tokio::test]
    async fn run_hooks_no_config_returns_default() {
        let hooks: HookConfig = HashMap::new();
        let ctx = mk_ctx().await;
        let reg = McpRegistry::new();
        let out = run_hooks(HookPoint::BeforeLlm, &hooks, &ctx, &reg)
            .await
            .unwrap();
        assert!(out.injected_prompt.is_none());
    }

    // -------- run_hooks：InjectPrompt 拼接 --------

    #[tokio::test]
    async fn run_hooks_single_inject_prompt() {
        let hooks = one_action(
            HookPoint::ConversationStart,
            HookAction::InjectPrompt {
                content: "always reply in JSON".into(),
            },
        );
        let ctx = mk_ctx().await;
        let reg = McpRegistry::new();
        let out = run_hooks(HookPoint::ConversationStart, &hooks, &ctx, &reg)
            .await
            .unwrap();
        assert_eq!(out.injected_prompt.as_deref(), Some("always reply in JSON"));
    }

    #[tokio::test]
    async fn run_hooks_multiple_inject_prompts_joined_with_newline() {
        let mut hooks: HookConfig = HashMap::new();
        hooks.insert(
            HookPoint::BeforeLlm,
            vec![
                HookAction::InjectPrompt {
                    content: "rule A".into(),
                },
                HookAction::InjectPrompt {
                    content: "rule B".into(),
                },
            ],
        );
        let ctx = mk_ctx().await;
        let reg = McpRegistry::new();
        let out = run_hooks(HookPoint::BeforeLlm, &hooks, &ctx, &reg)
            .await
            .unwrap();
        assert_eq!(out.injected_prompt.as_deref(), Some("rule A\nrule B"));
    }

    // -------- run_hooks：Log 不产生 injected_prompt --------

    #[tokio::test]
    async fn run_hooks_log_only_no_injected_prompt() {
        let hooks = one_action(
            HookPoint::ConversationEnd,
            HookAction::Log {
                message: "ended".into(),
            },
        );
        let ctx = mk_ctx().await;
        let reg = McpRegistry::new();
        let out = run_hooks(HookPoint::ConversationEnd, &hooks, &ctx, &reg)
            .await
            .unwrap();
        assert!(out.injected_prompt.is_none());
    }

    // -------- run_hooks：CallTool 失败仅 warn，不中断，不产生 injected_prompt --------

    #[tokio::test]
    async fn run_hooks_calltool_missing_tool_does_not_propagate_error() {
        let hooks = one_action(
            HookPoint::AfterTool,
            HookAction::CallTool {
                tool: "nonexistent_tool".into(),
                args: "{}".into(),
            },
        );
        let ctx = mk_ctx().await;
        let reg = McpRegistry::new(); // 空 registry → dispatch 必失败
                                      // 钩子失败不向上传播：仍返回 Ok
        let out = run_hooks(HookPoint::AfterTool, &hooks, &ctx, &reg)
            .await
            .unwrap();
        assert!(out.injected_prompt.is_none());
    }

    // -------- run_hooks：混合动作只收集 InjectPrompt --------

    #[tokio::test]
    async fn run_hooks_mixed_actions_collects_inject_only() {
        let mut hooks: HookConfig = HashMap::new();
        hooks.insert(
            HookPoint::BeforeLlm,
            vec![
                HookAction::InjectPrompt {
                    content: "keep going".into(),
                },
                HookAction::Log {
                    message: "round start".into(),
                },
                HookAction::CallTool {
                    tool: "nope".into(),
                    args: "{}".into(),
                },
                HookAction::InjectPrompt {
                    content: "stay focused".into(),
                },
            ],
        );
        let ctx = mk_ctx().await;
        let reg = McpRegistry::new();
        let out = run_hooks(HookPoint::BeforeLlm, &hooks, &ctx, &reg)
            .await
            .unwrap();
        // 仅两个 InjectPrompt 被拼接；Log/CallTool(失败) 不贡献
        assert_eq!(
            out.injected_prompt.as_deref(),
            Some("keep going\nstay focused")
        );
    }
}
