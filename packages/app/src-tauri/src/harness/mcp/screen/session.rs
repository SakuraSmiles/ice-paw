//! `request_screen_session` —— agent 主动请求开启屏幕共享通道（批次④ 步骤 1）。
//!
//! 三入口之一（§4.1）：页面开关（主入口）/ 本工具（agent 建议）/ 会话内直接调
//! screen 工具（现状，逐次 Confirm）。本工具 Confirm 级——审批卡由前端特判为
//! 二键「开启/拒绝」（scope 档对它无意义）；批准 → 通道开启（已开则仅附着），
//! 拒绝 → 正常错误返回，agent 自行措辞回应。
//!
//! 注意它**不在 [`SCREEN_TOOLS`](super::channel::SCREEN_TOOLS) 集合**：授权短路
//! 刻刻排除它——请求开通道的 Confirm 就是这个工具的存在意义，被短路即死循环。

use async_trait::async_trait;

use crate::error::{AppError, AppResult};

use super::channel;
use crate::harness::mcp::client::{McpClient, ToolContext, ToolOutput};
use crate::harness::mcp::types::AuthorizationLevel;

/// `request_screen_session`：请求用户开启屏幕共享（无参数——批准与否全在用户）。
pub struct RequestScreenSessionTool;

#[async_trait]
impl McpClient for RequestScreenSessionTool {
    fn name(&self) -> &str {
        "request_screen_session"
    }

    fn description(&self) -> &str {
        "Ask the user to turn ON the 'screen sharing' channel for this conversation. Once \
         granted, the screen tools (capture_screen / capture_window / mouse_* / type_text / \
         press_key) run WITHOUT per-call approval dialogs, so you can operate the screen \
         continuously. Call this when: the user asks you to watch and operate their screen, or \
         per-call approval popups are clearly interrupting a multi-step screen workflow. The \
         user sees one approval card (grant = channel on, deny = declined). If declined, tell \
         the user honestly and stop pursuing screen control. Do NOT call it when the channel is \
         already granted (approval-free screen tool calls succeeding means it is on)."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    /// Confirm 级：开通道是一次性的知情同意动作（画面将交给模型 + 输入将作用于机器）。
    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
    }

    fn auth_reason(&self) -> Option<String> {
        Some("请求开启屏幕共享——开启后本会话可直接截屏与操作屏幕（免逐次授权）".into())
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        // 需要 conv_id + pool（附着信息查库），走 execute_with_output。
        Err(AppError::Internal(
            "request_screen_session 必须通过 execute_with_output 调用（需要 conv_id + pool）".into(),
        ))
    }

    async fn execute_with_output(&self, _args: &str, ctx: &ToolContext) -> AppResult<ToolOutput> {
        let ch = channel::global();
        let info = channel::attach_info_from_db(&ctx.pool, &ctx.agent_id, &ctx.conv_id).await;
        let newly = ch.open(&ctx.conv_id, info.clone());
        // 通道生命周期走 tracing 不进 session_events（§4.11：无 conv/turn 容器）。
        tracing::info!(
            target: "ice_paw.screen_channel",
            conv = %ctx.conv_id, agent = %info.agent_name, newly_opened = newly,
            "屏幕通道开启/附着（request_screen_session 用户批准）"
        );
        if let Some(app) = &ctx.app_handle {
            channel::emit_state(app);
        }
        let msg = if newly {
            "屏幕共享通道已开启，本会话已附着——截屏与操作工具即刻起免逐次授权，可连续操作屏幕。\
             用户可随时关闭共享（届时会收到「screen 通道已关闭」错误，如实向用户说明即可）。"
        } else {
            "屏幕共享通道本已开启，本会话已附着——截屏与操作工具即刻起免逐次授权。"
        };
        Ok(ToolOutput::text(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// trait 契约：名字/级别/schema 形状（通道状态机的行为测试在 channel.rs）。
    #[test]
    fn tool_contract() {
        let t = RequestScreenSessionTool;
        assert_eq!(t.name(), "request_screen_session");
        assert!(matches!(
            t.authorization_level(),
            AuthorizationLevel::Confirm
        ));
        assert!(t.auth_reason().is_some());
        let schema = t.parameters();
        assert_eq!(schema["type"], "object", "无参数工具 schema 应为空 object");
    }
}
