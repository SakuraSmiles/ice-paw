//! `read_agent_config` 工具：读取当前 agent 自己的 `agent.yaml` 配置
//!
//! `Always` 授权（读自己的配置，安全）。走 `execute_with_context`：按 agent_id
//! 解析该 agent 的**个人工作区**（与 ctx.workspace——未来可能是 project 源码——无关，
//! 永远读 agent 自己的 agent.yaml）。

use async_trait::async_trait;

use crate::error::{AppError, AppResult};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

pub struct ReadAgentConfigTool;

#[async_trait]
impl McpClient for ReadAgentConfigTool {
    fn name(&self) -> &str {
        "read_agent_config"
    }

    fn description(&self) -> &str {
        "Read this agent's own configuration file (agent.yaml): temperature, max_tokens, \
system prompt, tool limits, etc. Takes no arguments."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "read_agent_config 必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, _args: &str, ctx: &ToolContext) -> AppResult<String> {
        // 解析该 agent 自己的工作区（不受 ctx.workspace / project 影响）
        let ws = crate::harness::tool_executor::resolve_agent_workspace(&ctx.pool, &ctx.agent_id)
            .await
            .ok_or_else(|| AppError::NotFound {
                resource: "agent_workspace",
                id: ctx.agent_id.clone(),
            })?;
        let cfg_path = ws.join("agent.yaml");
        let content = tokio::fs::read_to_string(&cfg_path).await.map_err(|e| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("读取 agent.yaml 失败（可能未生成）: {e}"),
            ))
        })?;

        Ok(serde_json::json!({
            "agent_id": ctx.agent_id,
            "path": cfg_path.to_string_lossy(),
            "content": content,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_without_context_returns_error() {
        let tool = ReadAgentConfigTool;
        let result = tool.execute("{}").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("execute_with_context"));
    }

    #[test]
    fn has_correct_name_and_auth_level() {
        let tool = ReadAgentConfigTool;
        assert_eq!(tool.name(), "read_agent_config");
        assert!(matches!(
            tool.authorization_level(),
            AuthorizationLevel::Always
        ));
    }
}
