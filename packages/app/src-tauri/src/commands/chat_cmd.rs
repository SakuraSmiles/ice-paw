//! Chat Tauri Commands 入口 — 仅编排，不含业务逻辑。
//!
//! - `send_message`：入参校验 → 取 agent/api_key → 拼装上下文 → 写库占位 → spawn stream_loop
//! - `stop_generation`：触发 ChatState 上的 CancellationToken
//!
//! 业务分布：protocol → infra::protocol | 上下文 → chat_context.rs | 调度 → harness::loop_engine
//!           错误 → harness::error_mapping | 收尾 → harness::cleanup

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
use crate::harness::loop_engine::LoopContext;
use crate::harness::observable::RoundState;
use crate::harness::provider;
use crate::harness::tool_executor::ToolAuthRegistry;
use crate::harness::tool_registry::{
    authority::{PathAuthSession, PathWhitelistConfig},
    ToolRegistry,
};
use crate::context::pipeline::{AssembledContext, PipelineContext, PipelineRunner};
use crate::context::history::resolve_window;

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

    // --- 3. 拼装上下文（A3-1：trait-based Pipeline） ---
    //
    // A3-2：根据 Agent 配置的 `max_history_messages` 决定 DB 加载上限。
    // 该值可能为 None（→ 系统默认）或 Some(N)；上限仍受
    // `repo::message::MAX_LIMIT` 兜底。Pipeline 内部还会再用一次
    // `resolve_window` 二次裁剪（保证窗口语义集中）。
    let db_load_limit = resolve_window(agent.max_history_messages) as i64;
    let history =
        repo::message::list_by_conversation(pool.inner(), &conv_id, Some(db_load_limit), None).await?;
    // 显式走 PipelineRunner：构造 PipelineContext + 注册 5 个 Stage，
    // 后续 A3-3 / A3-4 在此处追加新 Stage 即可，无需改动业务编排层。
    let mut pipeline_ctx = PipelineContext::new(
        pool.inner().clone(),
        agent.clone(),
        input.template.clone(),
        history,
        final_blocks,
        tools_enabled,
    );
    PipelineRunner::default_pipeline(pool.inner())
        .run(&mut pipeline_ctx)
        .await?;
    let assembled = AssembledContext {
        messages: pipeline_ctx.messages,
        user_blocks: pipeline_ctx.user_blocks,
    };

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

/// spawn LLM 流式协程，把编排结果交给 `harness::loop_engine::stream_loop`。
///
/// W6.2: 入参已封装到 [`LoopContext`] 后传给 `stream_loop`。
///
/// 注意：`spawn_stream_loop` 自身的形参列表仍是 11 个（编排层分散收集
/// 各 Tauri State/输入），所以 `#[allow(clippy::too_many_arguments)]`
/// 仍需保留；要消除这个 lint 需要更彻底地把编排也下沉到 `LoopContext`
/// 子构造器（不在 W6.2 范围内）。
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
        let emit_app = app.clone();

        // A2-3: 工具授权响应注册表（前端响应 → Rust oneshot 解锁）
        let auth_registry = ToolAuthRegistry::new();
        // A2-3: 本次会话级已授权路径表
        let auth_session = PathAuthSession::new();
        // A2-3: 路径白名单配置（当前为空 → 全部走 Confirm 流程）
        let whitelist = PathWhitelistConfig::default();

        // W6.2 + A2-3: 把 16 个输入字段封装到 LoopContext，
        // 消除 stream_loop 的 too_many_arguments 告警。
        let mut ctx = LoopContext::new(
            conv_id.clone(),
            asst_msg_id,
            app,
            pool,
            provider,
            api_key,
            temperature,
            max_tokens,
            messages,
            tool_registry,
            tools_enabled,
            cancel_token,
            budget,
            auth_registry,
            auth_session,
            whitelist,
        );
        crate::harness::loop_engine::stream_loop(&mut ctx, &mut observable).await;
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
