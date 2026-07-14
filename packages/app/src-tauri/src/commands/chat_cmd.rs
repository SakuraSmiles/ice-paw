//! Chat 相关 Tauri Commands
//!
//! - `send_message`：接收用户输入 → 写库 → spawn 流式生成协程 → 立即返回
//! - `stop_generation`：触发 CancellationToken 停止指定会话的生成
//!
//! 流式事件协议（前端通过 `listen` 订阅）：
//! | 事件        | 触发时机       | Payload                                |
//! |-------------|---------------|----------------------------------------|
//! | `chat:start`  | 命令接收到     | `{ conversation_id, user_message_id, assistant_message_id }` |
//! | `chat:chunk`  | 每个 SSE 增量  | `{ conversation_id, message_id, delta }`               |
//! | `chat:done`   | 流正常结束     | `{ conversation_id, message_id, finish_reason }`        |
//! | `chat:error`  | 任意阶段错误   | `{ conversation_id, message_id, kind, message }`        |
//!
//! P2-2 多模态：`send_message` 现在支持 `content_blocks`（含 Image 块）。
//! 旧 `content: String` 仍兼容，优先使用 `content_blocks`（含图片走新路径）。
//!
//! Step 5 拆分后，`stream_loop` 已迁至 `super::chat_loop`，本文件只剩：
//! - `send_message`：Tauri 命令入口（编排 + spawn 流式协程）
//! - `stop_generation`：Tauri 命令入口（触发 CancellationToken）

use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::crypto;
use crate::db::models::NewMessage;
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::llm::{
    self, ChatState,
    ContentBlock, ToolRegistry,
};

use super::chat_context::assemble_context;
use super::chat_protocol::{
    ChatStartPayload, SendMessageInput, validate_images,
};

// =========================================================================
// Commands
// =========================================================================

/// 发送消息 — 触发 LLM 流式生成
///
/// 流程：
/// 1. 取会话 → 取 agent → 取 api_key
/// 2. 调用 [`assemble_context`] 拼装 messages + 重排 user_blocks
/// 3. 写用户消息 + assistant 占位消息
/// 4. 注册 CancellationToken
/// 5. emit `chat:start`
/// 6. spawn 流式协程（不 await，立即返回）
#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    chat_state: State<'_, ChatState>,
    input: SendMessageInput,
) -> AppResult<()> {
    // --- 入参校验 ---
    // P2-2: 兼容旧 content + 新 content_blocks 二选一入参
    // 优先级：content_blocks 存在且非空 → 使用；否则 fallback 到 content。
    // 两者都为 None / 空 → 报错。
    let legacy_content: Option<String> = input.content.as_ref().and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() { None } else { Some(s.clone()) }
    });
    let blocks_from_input: Option<Vec<ContentBlock>> = input
        .content_blocks
        .clone()
        .filter(|v| !v.is_empty());

    let final_blocks: Vec<ContentBlock> = match (blocks_from_input, legacy_content) {
        (Some(blocks), _) => blocks,
        (None, Some(text)) => vec![ContentBlock::text(text)],
        (None, None) => {
            return Err(AppError::Validation(
                "content 或 content_blocks 至少提供一个".into(),
            ));
        }
    };

    // P2-2: 图片尺寸 / 张数 / 类型 校验（入库前最后一关）
    validate_images(&final_blocks)?;

    let conv_id = input.conversation_id.clone();
    // P2-2: user 入参的“纯文本部分”用于 DB `content` 列双写
    // （兼容旧消息读取逻辑：`msg.content` 仍能拿到文本预览）
    let content_text_for_db = ContentBlock::join_text(&final_blocks);

    // --- 取会话 → 拿 agent_id ---
    let conv = repo::conversation::get_by_id(pool.inner(), &conv_id).await?;

    // --- 取 agent ---
    let agent = repo::agent::get_by_id(pool.inner(), &conv.agent_id).await?;

    // --- 从 stronghold 取 api_key ---
    let (api_key, vault_base_url) = crypto::fetch_api_key(&app, &agent.id)?;

    // base_url 优先级：agent.base_url > vault_base_url > provider 默认
    let base_url = agent
        .base_url
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(vault_base_url.as_deref());

    // --- 创建 provider ---
    let provider = llm::create_provider(&agent.provider, &agent.model, base_url, agent.cache_prompt != 0)?;

    // --- 拉最近 20 条消息作为上下文 ---
    let history = repo::message::list_by_conversation(
        pool.inner(),
        &conv_id,
        Some(20),
        None,
    )
    .await?;

    let tools_enabled = input.tools_enabled;

    // --- 拼装上下文 messages + 重排后的 user_blocks ---
    // 详情见 `super::chat_context::assemble_context`：
    // 1. 模板查询 + 渲染（如提供）
    // 2. user_blocks 拼装 + 图片重排（OpenAI Vision 要求）
    // 3. system prompt 拼装（template > agent > tool_hint > os_context）
    // 4. 历史消息转换（多模态支持标记为 TODO）
    // 5. 当前 user 消息追加
    let assembled = assemble_context(
        pool.inner(),
        &agent,
        input.template.as_ref(),
        &history,
        final_blocks,
        tools_enabled,
    )
    .await?;
    let messages = assembled.messages;
    let user_blocks = assembled.user_blocks;

    // --- 写用户消息到 DB ---
    // P2-2 双写：
    // - `content` 列：仅存文本部分（join_text，兼容旧读取逻辑）
    // - `content_blocks` 列：完整块数组 JSON（含 Image）
    let user_msg_id = Uuid::new_v4().to_string();
    repo::message::create(
        pool.inner(),
        &user_msg_id,
        &NewMessage {
            conversation_id: conv_id.clone(),
            role: "user".into(),
            content: content_text_for_db.clone(),
            token_count: None,
            error: None,
        },
    )
    .await?;
    // 补充写入 content_blocks（P2-1 的 update_content_blocks 同样适用 user 消息）
    let user_blocks_json = serde_json::to_string(&user_blocks).unwrap_or_else(|_| "[]".to_string());
    repo::message::update_content_blocks(pool.inner(), &user_msg_id, &user_blocks_json).await?;

    // --- 创建 assistant 占位消息（content="" 后续更新）---
    let assistant_msg_id = Uuid::new_v4().to_string();
    repo::message::create(
        pool.inner(),
        &assistant_msg_id,
        &NewMessage {
            conversation_id: conv_id.clone(),
            role: "assistant".into(),
            content: String::new(),
            token_count: None,
            error: None,
        },
    )
    .await?;

    // --- 注册 CancellationToken（检查重复）---
    let cancel_token = chat_state.start(&conv_id).inspect_err(|_| {
        tracing::warn!(
            target: "ice_paw.chat",
            "send_message: 会话 {} 已有在途生成任务",
            conv_id
        );
    })?;

    // --- emit chat:start ---
    app.emit(
        "chat:start",
        ChatStartPayload {
            conversation_id: conv_id.clone(),
            user_message_id: user_msg_id.clone(),
            assistant_message_id: assistant_msg_id.clone(),
        },
    )?;

    // --- spawn 流式协程 ---
    let pool_clone = pool.inner().clone();
    let app_clone = app.clone();
    let conv_id_clone = conv_id.clone();
    let asst_msg_id_clone = assistant_msg_id.clone();
    let cancel_clone = cancel_token.clone();
    let temperature = agent.temperature;
    let max_tokens = agent.max_tokens;

    tokio::spawn(async move {
        let tool_registry = if tools_enabled {
            ToolRegistry::with_builtin()
        } else {
            // 不启用工具 → 空注册表
            ToolRegistry::new()
        };

        super::chat_loop::stream_loop(
            app_clone,
            pool_clone,
            provider,
            api_key,
            messages,
            temperature,
            max_tokens,
            cancel_clone,
            conv_id_clone,
            asst_msg_id_clone,
            tool_registry,
            tools_enabled,
        )
        .await;
    });

    Ok(())
}

// =========================================================================
// stop_generation 命令（独立，无需 chat_loop）
// =========================================================================

/// 停止指定会话的流式生成
#[tauri::command]
pub async fn stop_generation(
    chat_state: State<'_, ChatState>,
    conversation_id: String,
) -> AppResult<()> {
    let hit = chat_state.stop(&conversation_id);
    if !hit {
        tracing::warn!(
            target: "ice_paw.chat",
            "stop_generation: 会话 {} 无在途生成任务",
            conversation_id
        );
    }
    Ok(())
}
