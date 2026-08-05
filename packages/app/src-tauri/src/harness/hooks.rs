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
            HookAction::CallTool { tool, args } => match registry.dispatch(tool, args, tool_ctx).await {
                Ok(_) => tracing::info!(
                    target: "ice_paw.hooks",
                    "钩子 CallTool 成功: point={:?} tool={}", point, tool
                ),
                Err(e) => tracing::warn!(
                    target: "ice_paw.hooks",
                    "钩子 CallTool 失败（忽略，不中断对话）: point={:?} tool={} err={}", point, tool, e
                ),
            },
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
