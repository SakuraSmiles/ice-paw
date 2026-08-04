//! `delegate_to_agent` 工具：多 Agent 协作——委托子任务给专家 Agent
//!
//! 主 Agent 在 stream_loop 中调用此工具，后端用目标 Agent 的 system_prompt
//! + 主 Agent 的 provider/api_key 做单轮 LLM 调用，返回专家意见。
//!
//! MVP 限制：
//! - 单轮调用（专家 Agent 不调工具）
//! - 同模型（用主 Agent 的 provider/model/api_key）
//! - 弹窗确认（AuthorizationLevel::Confirm）

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;

use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::chat_state::CancellationToken;
use crate::harness::provider;
use crate::infra::protocol::{ChatDelta, ChatMessage, LlmProvider};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

/// 委托超时（专家 Agent 的单轮调用上限）
const DELEGATE_TIMEOUT_SECS: u64 = 60;

pub struct DelegateTool;

#[derive(Deserialize)]
struct DelegateArgs {
    agent_id: String,
    task: String,
}

#[async_trait]
impl McpClient for DelegateTool {
    fn name(&self) -> &str {
        "delegate_to_agent"
    }

    fn description(&self) -> &str {
        "Delegate a sub-task to another agent for expert advice. The target agent \
         will use its own system prompt to respond. Use when you need a different \
         perspective or specialized knowledge that another agent has."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "The ID of the agent to delegate to (must be a different agent)."
                },
                "task": {
                    "type": "string",
                    "description": "Clear description of the sub-task or question for the expert agent."
                }
            },
            "required": ["agent_id", "task"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
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

        // 安全：不能委托给自己
        if parsed.agent_id == ctx.agent_id {
            return Err(AppError::Validation(
                "不能委托给自己——请指定一个不同的 Agent".into(),
            ));
        }

        // 从 DB 读目标 Agent（专家）
        let target = repo::agent::get_by_id(&ctx.pool, &parsed.agent_id)
            .await
            .map_err(|_| AppError::NotFound {
                resource: "agent",
                id: parsed.agent_id.clone(),
            })?;

        // 从 DB 读主 Agent（获取 provider/model/base_url）
        let main_agent = repo::agent::get_by_id(&ctx.pool, &ctx.agent_id).await?;

        // 创建 LLM provider（用主 Agent 的配置）
        let llm_provider = provider::create_provider(
            &main_agent.provider,
            &main_agent.model,
            main_agent.base_url.as_deref(),
            main_agent.cache_prompt != 0,
        )?;

        let api_key = ctx.api_key.as_deref().ok_or_else(|| {
            AppError::Internal("缺少 API Key，无法执行委托".into())
        })?;

        // 构造 messages：专家 system prompt + 用户任务
        let messages = vec![
            ChatMessage::from_text("system", target.system_prompt.as_str()),
            ChatMessage::from_text("user", parsed.task.as_str()),
        ];

        // 调 stream_chat（带超时）
        let cancel = CancellationToken::new();
        let temperature = target.temperature;
        let max_tokens = target.max_tokens;

        let response = tokio::time::timeout(
            Duration::from_secs(DELEGATE_TIMEOUT_SECS),
            collect_llm_response(llm_provider.as_ref(), api_key, messages, temperature, max_tokens, cancel),
        )
        .await
        .map_err(|_| {
            AppError::Internal(format!(
                "委托超时（{}s）——专家 Agent 未在规定时间内回复",
                DELEGATE_TIMEOUT_SECS
            ))
        })??;

        tracing::info!(
            target: "ice_paw.delegate",
            "委托完成: 主={} → 专家={} ({} 字)",
            ctx.agent_id,
            parsed.agent_id,
            response.len()
        );

        Ok(serde_json::json!({
            "agent_id": parsed.agent_id,
            "agent_name": target.name,
            "response": response,
        })
        .to_string())
    }
}

/// 消费 LLM stream，收集完整文本回复
async fn collect_llm_response(
    provider: &dyn LlmProvider,
    api_key: &str,
    messages: Vec<ChatMessage>,
    temperature: f64,
    max_tokens: i32,
    cancel: CancellationToken,
) -> AppResult<String> {
    let mut stream = provider
        .stream_chat(api_key, messages, None, temperature, max_tokens, None, cancel)
        .await?;

    let mut text = String::new();
    while let Some(delta) = stream.next().await {
        match delta? {
            ChatDelta::Delta { content } => text.push_str(&content),
            ChatDelta::Done { .. } => break,
            _ => {}
        }
    }
    Ok(text)
}
