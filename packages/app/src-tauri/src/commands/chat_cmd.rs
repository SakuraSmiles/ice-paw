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
    // M1.2: 提取当前用户消息的纯文本 query（仅 Text 块拼接；与 LLM 拼装时使用的 content_text 一致）
    let current_user_query = Some(content_text.clone());

    // --- 2. 取会话 + agent + api_key → 创建 provider ---
    let conv = repo::conversation::get_by_id(pool.inner(), &conv_id).await?;
    let agent = repo::agent::get_by_id(pool.inner(), &conv.agent_id).await?;

    // Task 4: 从 Agent 配置读取工具白名单（NULL = 全部启用）
    let agent_enabled_tools: Option<Vec<String>> = agent
        .enabled_tools
        .as_deref()
        .map(|s| serde_json::from_str(s).unwrap_or_default());

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
    //
    // M1.2: 同时查询「最近 10 次」tool 消息以填充 `tool_call_history`，
    // M1.4 后供 loop_engine 在每轮调用 list_tool_defs_with_query 打分时使用。
    let db_load_limit = resolve_window(agent.max_history_messages) as i64;
    let history =
        repo::message::list_by_conversation(pool.inner(), &conv_id, Some(db_load_limit), None).await?;
    // M1.2: 加载最近 10 条工具消息，提取 tool_use 块中的工具名
    let tool_call_history =
        repo::message::list_recent_tool_names(pool.inner(), &conv_id, 10).await?;

    // --- 3.5 注册 cancel token（必须先于 Pipeline，让 MemoryStage 摘要也能响应取消）---
    let cancel_token = chat_state.start(&conv_id).inspect_err(|_| {
        tracing::warn!(target: "ice_paw.chat", "send_message: 会话 {} 已有在途生成任务", conv_id);
    })?;

    // 显式走 PipelineRunner：构造 PipelineContext + 注册 6 个 Stage（M1.4：
    // Template → OsContext → SystemPrompt → History → Memory → Final；
    // M1.4 起不再含 ToolTrimStage，工具裁剪下沉到 loop_engine）。
    // 后续 A3-3 在此处追加新 Stage 即可，无需改动业务编排层。
    //
    // M1.5: PipelineContext 现在携带 conversation_id + cancel_token，
    //       MemoryStage 需要二者才能调 summary_provider 并响应取消。
    let mut pipeline_ctx = PipelineContext::new(
        pool.inner().clone(),
        agent.clone(),
        None,
        history,
        final_blocks,
        tools_enabled,
        current_user_query.clone(),
        tool_call_history.clone(),
        crate::context::token::ContextBudget::default(),
        conv_id.clone(),
        cancel_token.clone(),
    );

    // M1.5: 构造 LlmSummaryProvider 注入 Pipeline
    use crate::harness::summary_provider::LlmSummaryProvider;
    use crate::context::memory::SummaryProvider;
    let summary_provider: Box<dyn SummaryProvider> = Box::new(
        LlmSummaryProvider::new(llm_provider.clone(), api_key.clone()),
    );
    PipelineRunner::default_pipeline(pool.inner(), Some(summary_provider))
        .run(&mut pipeline_ctx)
        .await?;

    // M1.5: emit chat:summary-injected if MemoryStage triggered
    if let Some(event) = pipeline_ctx.summary_event {
        let _ = app.emit("chat:summary-injected", event);
    }

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
            model: None,
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
            model: Some(agent.model.clone()),
        },
    ).await?;

    // --- 5. emit chat:start（cancel_token 已在 3.5 注册） ---
    app.emit("chat:start", ChatStartPayload {
        conversation_id: conv_id.clone(),
        user_message_id: user_msg_id.clone(),
        assistant_message_id: asst_msg_id.clone(),
    })?;

    // --- 6. spawn 流式协程 ---
    spawn_stream_loop(
        app, pool.inner().clone(), llm_provider, api_key,
        assembled.messages, agent.temperature, agent.max_tokens,
        cancel_token, conv_id, user_msg_id, asst_msg_id, tools_enabled,
        agent_enabled_tools,
        current_user_query, tool_call_history,
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
/// M1.4: 移除 `trimmed_tool_defs` 参数——Pipeline 不再做工具裁剪，
/// loop_engine 在每轮独立调 `list_tool_defs_with_query()` 打分。
///
/// 注意：`spawn_stream_loop` 自身的形参列表仍是 11 个（编排层分散收集
/// 各 Tauri State/输入），所以 `#[allow(clippy::too_many_arguments)]`
/// 仍需保留；要消除这个 lint 需要更彻底地把编排也下沉到 `LoopContext`
/// 子构造器（不在 W6.2 范围内）。
#[allow(clippy::too_many_arguments)]
fn spawn_stream_loop(
    app: AppHandle, pool: SqlitePool, provider: Arc<dyn LlmProvider>,
    api_key: String, messages: Vec<ChatMessage>, temperature: f64, max_tokens: i32,
    cancel_token: CancellationToken, conv_id: String, user_msg_id: String,
    asst_msg_id: String, tools_enabled: bool,
    agent_enabled_tools: Option<Vec<String>>,
    query: Option<String>, call_history: Vec<String>,
) {
    tokio::spawn(async move {
        // Task 4: 工具列表来自 Agent 配置（enabled_tools），对话 toggle 控制是否启用
        let tool_registry = if tools_enabled {
            match &agent_enabled_tools {
                Some(names) if !names.is_empty() => ToolRegistry::with_filter(names),
                Some(_) => ToolRegistry::new(), // 空 vec = 全部禁用
                None => ToolRegistry::with_builtin(), // None = 全部启用（向后兼容）
            }
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

        // W6.2 + A2-3 + M1.2: 把 18 个输入字段封装到 LoopContext，
        // 消除 stream_loop 的 too_many_arguments 告警。
        // M1.2: query + call_history 用于 list_tool_defs_with_query 打分
        let mut ctx = LoopContext::new(
            conv_id.clone(),
            asst_msg_id,
            user_msg_id,
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
            query,
            call_history,
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
