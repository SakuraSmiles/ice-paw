//! `propose_config_change` 工具 —— agent 代配置提案
//!
//! agent 调此工具提出配置变更建议，变更**不直接生效**。
//! 工具 emit `chat:config-proposal` 事件给前端 → 前端渲染提案卡片 →
//! 用户审批/编辑/拒绝 → 前端走现有可信 Tauri 命令（create_agent/update_agent）
//! 真正应用配置。agent 全程无写权限。
//!
//! ## 设计要点
//! - **参数全平铺**（不用 oneOf 嵌套），兼容所有 LLM 的 tool calling 实现
//! - 安全模型：提案进入 → guardrail 校验（红线直接拒绝，不 emit）
//! - 非红线提案 → emit 事件 + oneshot 等待前端响应（120s 超时）
//! - 审计日志 target=`ice_paw.mgmt`

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use uuid::Uuid;

use tauri::Emitter;

use crate::error::{AppError, AppResult};
use crate::harness::proposal_guard;
use crate::infra::protocol::{
    ConfigProposalPayload, ProposalAction, ProposalDecision,
};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

pub struct ProposeConfigChangeTool;

/// 平铺的参数结构 —— 所有字段可选（deserialize 层），必填校验在 guardrail 做。
/// `action` 是简单字符串：`"create_agent"` 或 `"update_agent"`。
#[derive(Deserialize)]
struct ProposeConfigArgs {
    /// "create_agent" | "update_agent"
    action: String,

    // ---- create_agent 字段 ----
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    max_tokens: Option<i32>,
    #[serde(default)]
    enabled_tools: Option<Vec<String>>,
    #[serde(default)]
    workspace_path: Option<String>,

    // ---- update_agent 字段 ----
    #[serde(default)]
    agent_id: Option<String>,

    // ---- 公共 ----
    /// 审批卡片上显示的提案摘要
    summary: String,
}

/// 把平铺参数转为 ProposalAction 枚举
fn into_proposal_action(args: &ProposeConfigArgs) -> AppResult<ProposalAction> {
    match args.action.as_str() {
        "create_agent" => {
            let id = args.id.as_deref().unwrap_or("").to_string();
            let name = args.name.as_deref().unwrap_or("").to_string();
            let provider = args.provider.as_deref().unwrap_or("").to_string();
            let model = args.model.as_deref().unwrap_or("").to_string();
            let api_key = args.api_key.as_deref().unwrap_or("").to_string();

            Ok(ProposalAction::CreateAgent {
                id,
                name,
                provider,
                model,
                api_key,
                base_url: args.base_url.clone(),
                system_prompt: args.system_prompt.clone(),
                temperature: args.temperature,
                max_tokens: args.max_tokens,
                enabled_tools: args.enabled_tools.clone(),
                workspace_path: args.workspace_path.clone(),
            })
        }
        "update_agent" => {
            let agent_id = args.agent_id.as_deref().unwrap_or("").to_string();
            Ok(ProposalAction::UpdateAgent {
                agent_id,
                name: args.name.clone(),
                provider: args.provider.clone(),
                model: args.model.clone(),
                system_prompt: args.system_prompt.clone(),
                base_url: args.base_url.clone(),
                temperature: args.temperature,
                max_tokens: args.max_tokens,
                enabled_tools: args.enabled_tools.clone(),
                workspace_path: args.workspace_path.clone(),
            })
        }
        other => Err(AppError::Validation(format!(
            "不支持的 action 类型: '{other}'，必须是 create_agent 或 update_agent"
        ))),
    }
}

#[async_trait]
impl McpClient for ProposeConfigChangeTool {
    fn name(&self) -> &str {
        "propose_config_change"
    }

    fn description(&self) -> &str {
        "创建新的 AI agent，或修改已有 agent 的配置。\
         这是从对话中创建/修改 agent 的**唯一**入口。\
         \
         当用户说「帮我创建一个 agent」「建个写代码的助手」「加个翻译机器人」\
         或任何类似的创建 agent 请求时——你必须调用本工具。\
         \
         当用户要求修改 agent 名称、模型、system prompt、temperature、\
         或启用的工具列表时——你必须调用本工具。\
         \
         新建 agent：action='create_agent'，填写 id/name/provider/model，\
         api_key 固定为 '__SLOT__'（用户在审批卡片上填写真实 key）。\
         \
         修改 agent：action='update_agent'，agent_id 设为当前 agent 自己的 ID\
         （如不确定，先调 read_agent_config 查看）。\
         \
         提案会以审批卡片形式展示给用户，用户批准后生效。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create_agent", "update_agent"],
                    "description": "操作类型：create_agent（新建 agent）或 update_agent（修改已有 agent）。"
                },
                "id": {
                    "type": "string",
                    "description": "【create_agent 必填】新 agent 的唯一 ID（建议英文 kebab-case，如 'code-buddy'）。"
                },
                "name": {
                    "type": "string",
                    "description": "【create_agent 必填】显示名称（如「代码助手」「翻译官」）。update_agent 可选。"
                },
                "provider": {
                    "type": "string",
                    "enum": ["anthropic", "openai", "deepseek", "glm", "minimax"],
                    "description": "【create_agent 必填】LLM 厂商。根据用户偏好或任务需要选择。"
                },
                "model": {
                    "type": "string",
                    "description": "【create_agent 必填】模型名称。各厂商推荐：anthropic→claude-sonnet-5 / claude-opus-5 / claude-haiku-4-5；openai→gpt-4o / gpt-4o-mini / o3-mini；deepseek→deepseek-v4-pro / deepseek-v4-flash；glm→glm-5.2 / glm-5-turbo；minimax→MiniMax-M3 / MiniMax-M2.5。简单任务选小模型，复杂推理/编程选大模型。"
                },
                "api_key": {
                    "type": "string",
                    "description": "【create_agent 必填，固定值】必须填 '__SLOT__'。真实 key 由用户在审批卡片上填写。"
                },
                "base_url": {
                    "type": "string",
                    "description": "自定义 API 地址（可选，通常留空）。"
                },
                "system_prompt": {
                    "type": "string",
                    "description": "系统提示词，定义 agent 的角色和行为（可选）。"
                },
                "temperature": {
                    "type": "number",
                    "description": "温度参数 0.0-2.0。越低越确定，越高越有创意。一般 0.7 即可，编程场景建议 0.2-0.4。"
                },
                "max_tokens": {
                    "type": "integer",
                    "description": "单次输出最大 token 数（可选，留空用默认值）。"
                },
                "enabled_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "为该 agent 启用的工具列表。常用：read_file, write_file, edit_file, list_directory, search_files, run_command, git, web_fetch, search_kb, save_to_kb。编程 agent 建议启用 read_file+write_file+edit_file+run_command+git+search_files。非空会触发额外用户确认。"
                },
                "workspace_path": {
                    "type": "string",
                    "description": "工作区目录路径（可选，留空自动生成）。"
                },
                "agent_id": {
                    "type": "string",
                    "description": "【update_agent 必填】要修改的 agent ID（必须是你自己的 agent ID。不确定先调 read_agent_config）。"
                },
                "summary": {
                    "type": "string",
                    "description": "审批卡片上显示的提案摘要（一行）。如「创建带文件读写和命令执行能力的编程助手」或「把当前 agent 的 temperature 降到 0.2」。"
                }
            },
            "required": ["action", "summary"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "propose_config_change 必须通过 execute_with_context 调用（需要 agent_id + app_handle 上下文）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: ProposeConfigArgs =
            serde_json::from_str(args).map_err(|e| {
                AppError::Validation(format!("propose_config_change 参数解析失败: {e}"))
            })?;

        // 1. 平铺参数 → ProposalAction 枚举
        let action = into_proposal_action(&parsed)?;
        let summary = parsed.summary;

        // 2. Guardrail 校验（红线直接返回 Err 给 LLM，不 emit 事件）
        let (sensitivity, _warnings) =
            proposal_guard::validate_proposal(&action, &ctx.agent_id)?;

        // 3. 获取 AppHandle 和 ProposalRegistry（由 execute_tool_round 注入到 ctx）
        let app_handle = ctx.app_handle.as_ref().ok_or_else(|| {
            AppError::Internal("propose_config_change: app_handle 未注入到 ToolContext".into())
        })?;
        let proposal_registry = ctx.proposal_registry.as_ref().ok_or_else(|| {
            AppError::Internal("propose_config_change: proposal_registry 未注入到 ToolContext".into())
        })?;

        // 4. 生成 request_id
        let request_id = Uuid::new_v4().to_string();

        // 审计：提案发起
        let action_label = match &action {
            ProposalAction::CreateAgent { name, .. } => format!("create_agent({name})"),
            ProposalAction::UpdateAgent { agent_id, .. } => format!("update_agent({agent_id})"),
        };
        tracing::info!(
            target: "ice_paw.mgmt",
            request_id = %request_id,
            agent_id = %ctx.agent_id,
            conv_id = %ctx.conv_id,
            action = %action_label,
            sensitivity = ?sensitivity,
            "配置变更提案已发送"
        );

        // 5. 注册 oneshot receiver
        let rx = proposal_registry.register(request_id.clone()).await;

        // 6. Emit 事件给前端
        let payload = ConfigProposalPayload {
            request_id: request_id.clone(),
            conversation_id: ctx.conv_id.clone(),
            message_id: String::new(),
            tool_use_id: String::new(),
            sensitivity,
            action,
            summary,
        };

        if let Err(e) = app_handle.emit("chat:config-proposal", payload) {
            let _ = proposal_registry.take(&request_id).await;
            return Err(AppError::Internal(format!("无法发送配置提案事件: {e}")));
        }

        // 7. 等待前端响应（120s 超时）
        const TIMEOUT: Duration = Duration::from_secs(120);

        let response = tokio::time::timeout(TIMEOUT, rx).await;

        match response {
            Ok(Ok(resp)) => {
                tracing::info!(
                    target: "ice_paw.mgmt",
                    request_id = %request_id,
                    decision = ?resp.decision,
                    "配置提案已处理"
                );

                match resp.decision {
                    ProposalDecision::Approved => Ok(serde_json::json!({
                        "status": "approved",
                        "message": "用户已批准此配置变更，变更已应用。"
                    })
                    .to_string()),
                    ProposalDecision::Modified { changes } => Ok(serde_json::json!({
                        "status": "modified",
                        "message": "用户修改后批准了此配置变更。",
                        "changes": changes,
                    })
                    .to_string()),
                    ProposalDecision::Rejected { reason } => {
                        let msg = reason
                            .as_deref()
                            .unwrap_or("用户拒绝了此配置变更");
                        Ok(serde_json::json!({
                            "status": "rejected",
                            "message": msg,
                        })
                        .to_string())
                    }
                }
            }
            Ok(Err(_)) => {
                tracing::warn!(
                    target: "ice_paw.mgmt",
                    request_id = %request_id,
                    "提案响应通道已关闭（可能被取消）"
                );
                Ok(serde_json::json!({
                    "status": "cancelled",
                    "message": "提案已被取消或超时。"
                })
                .to_string())
            }
            Err(_elapsed) => {
                let _ = proposal_registry.take(&request_id).await;
                tracing::warn!(
                    target: "ice_paw.mgmt",
                    request_id = %request_id,
                    "提案响应超时（120s）"
                );
                Ok(serde_json::json!({
                    "status": "timeout",
                    "message": "用户未在 120 秒内响应提案，已自动取消。可以重新发起。"
                })
                .to_string())
            }
        }
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_metadata() {
        let tool = ProposeConfigChangeTool;
        assert_eq!(tool.name(), "propose_config_change");
        assert!(!tool.description().is_empty());
        assert_eq!(tool.authorization_level(), AuthorizationLevel::Always);

        let params = tool.parameters();
        assert_eq!(params["type"], "object");
        assert!(params["required"].as_array().unwrap().contains(&serde_json::json!("action")));
        assert!(params["required"].as_array().unwrap().contains(&serde_json::json!("summary")));
    }

    #[tokio::test]
    async fn execute_without_context_returns_error() {
        let tool = ProposeConfigChangeTool;
        let result = tool.execute("{}").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("execute_with_context"));
    }

    #[test]
    fn parse_create_agent_flat_args() {
        let json = r#"{
            "action": "create_agent",
            "id": "test-bot",
            "name": "测试助手",
            "provider": "anthropic",
            "model": "claude-sonnet-5",
            "api_key": "__SLOT__",
            "temperature": 0.3,
            "summary": "创建一个测试 agent"
        }"#;
        let args: ProposeConfigArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.action, "create_agent");
        assert_eq!(args.id.as_deref(), Some("test-bot"));
        assert_eq!(args.name.as_deref(), Some("测试助手"));
        assert_eq!(args.temperature, Some(0.3));

        let action = into_proposal_action(&args).unwrap();
        match action {
            ProposalAction::CreateAgent { id, name, provider, model, api_key, temperature, .. } => {
                assert_eq!(id, "test-bot");
                assert_eq!(name, "测试助手");
                assert_eq!(provider, "anthropic");
                assert_eq!(model, "claude-sonnet-5");
                assert_eq!(api_key, "__SLOT__");
                assert_eq!(temperature, Some(0.3));
            }
            _ => panic!("expected CreateAgent"),
        }
    }

    #[test]
    fn parse_update_agent_flat_args() {
        let json = r#"{
            "action": "update_agent",
            "agent_id": "my-agent",
            "name": "新名字",
            "temperature": 0.5,
            "summary": "改个名"
        }"#;
        let args: ProposeConfigArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.action, "update_agent");
        assert_eq!(args.agent_id.as_deref(), Some("my-agent"));

        let action = into_proposal_action(&args).unwrap();
        match action {
            ProposalAction::UpdateAgent { agent_id, name, temperature, .. } => {
                assert_eq!(agent_id, "my-agent");
                assert_eq!(name, Some("新名字".into()));
                assert_eq!(temperature, Some(0.5));
            }
            _ => panic!("expected UpdateAgent"),
        }
    }

    #[test]
    fn parse_invalid_action_type() {
        let json = r#"{"action": "delete_agent", "summary": "删掉"}"#;
        let args: ProposeConfigArgs = serde_json::from_str(json).unwrap();
        let err = into_proposal_action(&args).unwrap_err();
        assert!(err.to_string().contains("delete_agent"));
    }

    #[test]
    fn missing_summary_is_rejected() {
        let json = r#"{"action": "create_agent", "id": "x", "name": "x", "provider": "x", "model": "x", "api_key": "__SLOT__"}"#;
        let err = serde_json::from_str::<ProposeConfigArgs>(json).unwrap_err();
        assert!(err.to_string().contains("summary"));
    }
}
