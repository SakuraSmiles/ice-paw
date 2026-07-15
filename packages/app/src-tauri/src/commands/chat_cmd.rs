//! Chat Tauri Commands 入口 — 仅编排，不含业务逻辑。
//!
//! - `send_message`：入参校验 → 取 agent/api_key → 拼装上下文 → 写库占位 → spawn stream_loop
//! - `stop_generation`：触发 ChatState 上的 CancellationToken
//!
//! 业务分布：protocol → infra::protocol | 上下文 → chat_context.rs | 调度 → chat_loop.rs
//!           错误 → chat_error.rs | 收尾 → chat_cleanup.rs

use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::crypto;
use crate::db::models::NewMessage;
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::infra::protocol::{
    ChatMessage, ChatRoundStatePayload, ChatStartPayload, ContentBlock, LlmProvider, SendMessageInput, validate_images,
};
use crate::harness::budget::LoopBudget;
use crate::harness::chat_state::{CancellationToken, ChatState};
use crate::harness::observable::RoundState;
use crate::harness::provider;
use crate::harness::tool_registry::ToolRegistry;
use super::chat_context::assemble_context;

/// 发送消息 — 触发 LLM 流式生成。
#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    chat_state: State<'_, ChatState>,
    input: SendMessageInput,
) -> AppResult<()> {
    // --- 1. 入参校验：content_blocks 优先，回退到 legacy content ---
    let final_blocks = {
        let blocks = input.content_blocks.clone().filter(|v| !v.is_empty());
        let legacy = input.content.as_ref().and_then(|s| {
            let t = s.trim();
            if t.is_empty() { None } else { Some(t.to_owned()) }
        });
        match (blocks, legacy) {
            (Some(b), _) => b,
            (None, Some(t)) => vec![ContentBlock::text(t)],
            (None, None) => return Err(AppError::Validation(
                "content 或 content_blocks 至少提供一个".into(),
            )),
        }
    };
    validate_images(&final_blocks)?;

    let conv_id = input.conversation_id.clone();
    let tools_enabled = input.tools_enabled;
    let content_text = ContentBlock::join_text(&final_blocks);

    // --- 2. 取会话 + agent + api_key → 创建 provider ---
    let conv = repo::conversation::get_by_id(pool.inner(), &conv_id).await?;
    let agent = repo::agent::get_by_id(pool.inner(), &conv.agent_id).await?;
    let (api_key, vault_base_url) = crypto::fetch_api_key(&app, &agent.id)?;
    let base_url = agent
        .base_url
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(vault_base_url.as_deref());
    let llm_provider = provider::create_provider(
        &agent.provider, &agent.model, base_url, agent.cache_prompt != 0,
    )?;

    // --- 3. 拼装上下文 ---
    let history =
        repo::message::list_by_conversation(pool.inner(), &conv_id, Some(20), None).await?;
    let assembled = assemble_context(
        pool.inner(), &agent, input.template.as_ref(), &history, final_blocks, tools_enabled,
    )
    .await?;

    // --- 4. 写用户消息 + assistant 占位 ---
    let user_msg_id = Uuid::new_v4().to_string();
    repo::message::create(
        pool.inner(), &user_msg_id,
        &NewMessage {
            conversation_id: conv_id.clone(), role: "user".into(),
            content: content_text, token_count: None, error: None,
        },
    ).await?;
    let blocks_json = serde_json::to_string(&assembled.user_blocks).unwrap_or_else(|_| "[]".into());
    repo::message::update_content_blocks(pool.inner(), &user_msg_id, &blocks_json).await?;

    let asst_msg_id = Uuid::new_v4().to_string();
    repo::message::create(
        pool.inner(), &asst_msg_id,
        &NewMessage {
            conversation_id: conv_id.clone(), role: "assistant".into(),
            content: String::new(), token_count: None, error: None,
        },
    ).await?;

    // --- 5. 注册 cancel token + emit chat:start ---
    let cancel_token = chat_state.start(&conv_id).inspect_err(|_| {
        tracing::warn!(target: "ice_paw.chat", "send_message: 会话 {} 已有在途生成任务", conv_id);
    })?;
    app.emit("chat:start", ChatStartPayload {
        conversation_id: conv_id.clone(),
        user_message_id: user_msg_id.clone(),
        assistant_message_id: asst_msg_id.clone(),
    })?;

    // --- 6. spawn 流式协程 ---
    spawn_stream_loop(
        app, pool.inner().clone(), llm_provider, api_key,
        assembled.messages, agent.temperature, agent.max_tokens,
        cancel_token, conv_id, asst_msg_id, tools_enabled,
    );
    Ok(())
}

/// 停止指定会话的流式生成。
#[tauri::command]
pub async fn stop_generation(
    chat_state: State<'_, ChatState>,
    conversation_id: String,
) -> AppResult<()> {
    if !chat_state.stop(&conversation_id) {
        tracing::warn!(target: "ice_paw.chat",
            "stop_generation: 会话 {} 无在途生成任务", conversation_id);
    }
    Ok(())
}

/// spawn LLM 流式协程，把编排结果交给 `chat_loop::stream_loop`。
#[allow(clippy::too_many_arguments)]
fn spawn_stream_loop(
    app: AppHandle, pool: SqlitePool, provider: Arc<dyn LlmProvider>,
    api_key: String, messages: Vec<ChatMessage>, temperature: f64, max_tokens: i32,
    cancel_token: CancellationToken, conv_id: String, asst_msg_id: String, tools_enabled: bool,
) {
    tokio::spawn(async move {
        let tool_registry = if tools_enabled {
            ToolRegistry::with_builtin()
        } else {
            ToolRegistry::new()
        };
        // W2.4: maintain observable state across the stream loop
        let mut observable = RoundState::default();
        // W4.1: 传入 LoopBudget（当前用 default，等价于原硬编码常量）
        let budget = LoopBudget::default();
        let round_conv_id = conv_id.clone();
        let emit_app = app.clone();
        super::chat_loop::stream_loop(
            app, pool, provider, api_key, messages,
            temperature, max_tokens, cancel_token,
            round_conv_id, asst_msg_id, tool_registry, tools_enabled,
            budget,
            &mut observable,
        ).await;
        // W2.4: emit final round-state after stream_loop completes
        let _ = emit_app.emit(
            "chat:round-state",
            ChatRoundStatePayload {
                conversation_id: conv_id,
                round: observable.round,
                elapsed_ms: observable.elapsed_ms,
                tokens_prompt: observable.tokens_prompt,
                tokens_completion: observable.tokens_completion,
                cached_tokens: observable.cached_tokens,
                retry_count: observable.retry_count,
            },
        );
    });
}
