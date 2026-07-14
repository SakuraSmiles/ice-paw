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

use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::crypto;
use crate::db::models::NewMessage;
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::llm::{
    self, ChatDelta, ChatMessage, CancellationToken, ChatState,
    ContentBlock, LlmProvider, ToolRegistry,
};

use super::chat_cleanup::{cleanup, cleanup_after_success_with_blocks};
use super::chat_context::assemble_context;
use super::chat_error::{error_kind, friendly_error};
use super::chat_protocol::{
    ChatChunkPayload, ChatErrorPayload, ChatRetryingPayload, ChatStartPayload,
    ChatThinkingPayload, ChatToolCallDeltaPayload, ChatToolCallEndPayload, ChatToolCallStartPayload,
    ChatToolResultPayload, SendMessageInput, validate_images,
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

        stream_loop(
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

/// 流式生成内部协程 — 支持指数退避重试 + 工具执行循环
///
/// P2-1 工具执行循环：
/// 1. 调 provider.stream_chat(messages, tools?, ...)
/// 2. 消费 stream，收集文本 delta / 思考 delta / 工具调用
/// 3. stream 结束后：
///    a. 如果产生了工具调用（tool_calls 非空）：
///       - 在 Rust 侧通过 ToolRegistry 执行工具
///       - 将 tool_use + tool_result 作为 content block 追加到 messages
///       - emit chat:tool-result
///       - 回到步骤 1（最多 5 轮，防止无限循环）
///    b. 如果没有工具调用 → 正常结束，emit chat:done
///
/// 重试策略：
/// - 首次失败 → 等待 1s → 第 2 次尝试
/// - 二次失败 → 等待 2s → 第 3 次尝试
/// - 三次失败 → 等待 4s → 第 4 次尝试（总计 4 次，即最多 3 次重试）
/// - 超过 4 次 → 放弃，emit chat:error
///
/// 不重试的情况：
/// - 用户主动取消（cancel.is_cancelled()）
/// - 不可重试错误（401/403 等）
async fn stream_loop(
    app: AppHandle,
    pool: SqlitePool,
    provider: Arc<dyn LlmProvider>,
    api_key: String,
    mut messages: Vec<ChatMessage>,
    temperature: f64,
    max_tokens: i32,
    cancel: CancellationToken,
    conv_id: String,
    asst_msg_id: String,
    tool_registry: ToolRegistry,
    tools_enabled: bool,
) {
    use futures::StreamExt;
    use std::collections::HashMap;
    use std::time::Duration;

    const MAX_TOOL_ROUNDS: u32 = 5;
    const MAX_ATTEMPTS: u32 = 4;

    /// 一轮流式消费中收集到的工具调用信息
    #[derive(Debug, Clone)]
    struct CollectedToolCall {
        id: String,
        name: String,
        /// 累积的 arguments JSON 片段
        arguments: String,
        /// 是否已收到 ToolCallEnd
        ended: bool,
    }

    // 累积所有轮次的文本
    let mut all_text = String::new();
    // 累积所有轮次的 content_blocks（用于 DB 回写）
    let mut all_content_blocks: Vec<ContentBlock> = Vec::new();
    // P2-3: 累积 token usage（最后一个 Usage delta 覆盖前面的）
    let mut collected_usage: Option<llm::TokenUsage> = None;

    // === 工具执行循环 ===
    for tool_round in 0..MAX_TOOL_ROUNDS {
        if cancel.is_cancelled() {
            return cleanup(&app, &pool, &conv_id);
        }

        // 准备本轮的 tools 定义
        // 所有轮次都传 tools：messages 中含 assistant 的 tool_calls 时，
        // 部分 API（GLM 等）要求请求必须带 tools 定义，否则返回 400
        let tools: Option<Vec<crate::llm::ToolDef>> = if tools_enabled {
            Some(tool_registry.list_tool_defs().await)
        } else {
            None
        };

        // 本轮收集
        let mut round_text = String::new();
        let mut round_think = String::new();
        let mut round_finish_reason = "stop".to_string();
        let mut tool_calls_map: HashMap<String, CollectedToolCall> = HashMap::new();
        let mut round_success = false;

        // === 重试循环（每轮内）===
        'retry_loop: for attempt in 0..MAX_ATTEMPTS {
            if cancel.is_cancelled() {
                return cleanup(&app, &pool, &conv_id);
            }

            if attempt > 0 {
                let wait_secs = 1u64 << (attempt - 1);
                tracing::info!(
                    target: "ice_paw.chat",
                    "重试 LLM 请求: tool_round={} attempt={}/{}，等待 {}s",
                    tool_round, attempt + 1, MAX_ATTEMPTS, wait_secs,
                );
                let _ = app.emit(
                    "chat:retrying",
                    ChatRetryingPayload {
                        conversation_id: conv_id.clone(),
                        message_id: asst_msg_id.clone(),
                        attempt: attempt + 1,
                        max_attempts: MAX_ATTEMPTS,
                    },
                );
                tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                if cancel.is_cancelled() {
                    return cleanup(&app, &pool, &conv_id);
                }
            }

            // 构造重试消息
            let retry_messages = if !round_text.is_empty() && attempt > 0 {
                let mut msgs = messages.clone();
                msgs.push(ChatMessage::from_text(
                    "assistant",
                    format!(
                        "[以下是上一轮因网络中断已收到的部分回复，请从此处继续]\n{}",
                        &round_text
                    ),
                ));
                msgs
            } else {
                messages.clone()
            };

            let stream_result = provider
                .stream_chat(
                    &api_key,
                    retry_messages,
                    tools.clone(),
                    temperature,
                    max_tokens,
                    cancel.clone(),
                )
                .await;

            match stream_result {
                Ok(mut stream) => {
                    let mut attempt_ok = true;

                    while let Some(item) = stream.next().await {
                        if cancel.is_cancelled() {
                            return cleanup(&app, &pool, &conv_id);
                        }

                        match item {
                            Ok(ChatDelta::Delta { content: delta }) => {
                                round_text.push_str(&delta);
                                let _ = app.emit(
                                    "chat:chunk",
                                    ChatChunkPayload {
                                        conversation_id: conv_id.clone(),
                                        message_id: asst_msg_id.clone(),
                                        delta,
                                    },
                                );
                            }
                            Ok(ChatDelta::ToolCallStart { id, name }) => {
                                tool_calls_map.insert(
                                    id.clone(),
                                    CollectedToolCall {
                                        id: id.clone(),
                                        name: name.clone(),
                                        arguments: String::new(),
                                        ended: false,
                                    },
                                );
                                let _ = app.emit(
                                    "chat:tool-call-start",
                                    ChatToolCallStartPayload {
                                        conversation_id: conv_id.clone(),
                                        message_id: asst_msg_id.clone(),
                                        id: id.clone(),
                                        name,
                                    },
                                );
                            }
                            Ok(ChatDelta::ToolCallDelta { id, delta: tool_delta }) => {
                                if let Some(tc) = tool_calls_map.get_mut(&id) {
                                    tc.arguments.push_str(&tool_delta);
                                }
                                let _ = app.emit(
                                    "chat:tool-call-delta",
                                    ChatToolCallDeltaPayload {
                                        conversation_id: conv_id.clone(),
                                        message_id: asst_msg_id.clone(),
                                        id,
                                        delta: tool_delta,
                                    },
                                );
                            }
                            Ok(ChatDelta::ToolCallEnd { id }) => {
                                if let Some(tc) = tool_calls_map.get_mut(&id) {
                                    tc.ended = true;
                                }
                                let _ = app.emit(
                                    "chat:tool-call-end",
                                    ChatToolCallEndPayload {
                                        conversation_id: conv_id.clone(),
                                        message_id: asst_msg_id.clone(),
                                        id,
                                    },
                                );
                            }
                            Ok(ChatDelta::Thinking { content: think_content }) => {
                                round_think.push_str(&think_content);
                                let _ = app.emit(
                                    "chat:thinking",
                                    ChatThinkingPayload {
                                        conversation_id: conv_id.clone(),
                                        message_id: asst_msg_id.clone(),
                                        content: think_content,
                                    },
                                );
                            }
                            // P2-3: Token usage
                            Ok(ChatDelta::Usage { usage: u }) => {
                                collected_usage = Some(u);
                            }
                            Ok(ChatDelta::Done { finish_reason: fr }) => {
                                if let Some(fr) = fr {
                                    round_finish_reason = fr;
                                }
                                round_success = true;
                                break 'retry_loop;
                            }
                            Err(e) => {
                                if e.is_retryable() {
                                    attempt_ok = false;
                                    tracing::warn!(
                                        target: "ice_paw.chat",
                                        "流中可重试错误 (round={} attempt={}/{}): {}",
                                        tool_round, attempt + 1, MAX_ATTEMPTS, e
                                    );
                                    break; // 跳出 inner while，进入下一轮重试
                                } else {
                                    let err_msg = e.to_string();
                                    let _ = app.emit(
                                        "chat:error",
                                        ChatErrorPayload {
                                            conversation_id: conv_id.clone(),
                                            message_id: asst_msg_id.clone(),
                                            kind: error_kind(&e),
                                            message: friendly_error(&err_msg),
                                        },
                                    );
                                    let _ = repo::message::update_error(&pool, &asst_msg_id, &err_msg).await;
                                    return cleanup(&app, &pool, &conv_id);
                                }
                            }
                        }
                    }

                    // stream 自然结束但没收到 Done
                    if attempt_ok {
                        round_success = true;
                        break 'retry_loop;
                    }
                }
                Err(e) => {
                    if e.is_retryable() {
                        tracing::warn!(
                            target: "ice_paw.chat",
                            "请求失败可重试 (round={} attempt={}/{}): {}",
                            tool_round, attempt + 1, MAX_ATTEMPTS, e
                        );
                    } else {
                        let err_msg = e.to_string();
                        let _ = app.emit(
                            "chat:error",
                            ChatErrorPayload {
                                conversation_id: conv_id.clone(),
                                message_id: asst_msg_id.clone(),
                                kind: error_kind(&e),
                                message: friendly_error(&err_msg),
                            },
                        );
                        let _ = repo::message::update_error(&pool, &asst_msg_id, &err_msg).await;
                        return cleanup(&app, &pool, &conv_id);
                    }
                }
            }
        }

        if !round_success {
            // 重试耗尽
            let err_msg = format!("连接重试已耗尽（共 {} 次），已收到部分内容", MAX_ATTEMPTS);
            if !round_text.is_empty() {
                let _ = repo::message::update_content(&pool, &asst_msg_id, &round_text).await;
            }
            let _ = repo::message::update_error(&pool, &asst_msg_id, &err_msg).await;
            let _ = app.emit(
                "chat:error",
                ChatErrorPayload {
                    conversation_id: conv_id.clone(),
                    message_id: asst_msg_id.clone(),
                    kind: "stream".into(),
                    message: friendly_error(&err_msg),
                },
            );
            return cleanup(&app, &pool, &conv_id);
        }

        // 累积文本
        all_text.push_str(&round_text);

        // 累积 thinking
        if !round_think.is_empty() {
            all_content_blocks.push(ContentBlock::Thinking {
                thinking: round_think,
                signature: None,
            });
        }

        // 检查是否有工具调用需要执行
        let completed_calls: Vec<(String, String, String)> = tool_calls_map
            .into_values()
            .filter(|tc| tc.ended)
            .map(|tc| (tc.id, tc.name, tc.arguments))
            .collect();

        if completed_calls.is_empty() {
            // 没有工具调用 → 正常结束
            // 先保存文本副本用于 DB content 字段，再 move 进 content block
            let content_for_db = all_text.clone();
            if !all_text.is_empty() {
                all_content_blocks.push(ContentBlock::Text {
                    text: all_text,
                });
            }
            return cleanup_after_success_with_blocks(
                &app, &pool, &conv_id, &asst_msg_id,
                &content_for_db, &all_content_blocks, &round_finish_reason,
                collected_usage,
            );
        }

        // === 执行工具调用 ===
        tracing::info!(
            target: "ice_paw.chat",
            "工具调用循环: round={} tool_count={}",
            tool_round, completed_calls.len(),
        );

        let mut tool_use_blocks: Vec<ContentBlock> = Vec::new();
        let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

        for (tc_id, tc_name, tc_args) in &completed_calls {
            let result = tool_registry.dispatch(tc_name, tc_args).await;

            match result {
                Ok(content) => {
                    let _ = app.emit(
                        "chat:tool-result",
                        ChatToolResultPayload {
                            conversation_id: conv_id.clone(),
                            message_id: asst_msg_id.clone(),
                            tool_use_id: tc_id.clone(),
                            content: content.clone(),
                            is_error: false,
                        },
                    );
                    tool_result_blocks.push(ContentBlock::ToolResult {
                        tool_use_id: tc_id.clone(),
                        content,
                        is_error: Some(false),
                    });
                }
                Err(e) => {
                    let err_content = e.to_string();
                    let _ = app.emit(
                        "chat:tool-result",
                        ChatToolResultPayload {
                            conversation_id: conv_id.clone(),
                            message_id: asst_msg_id.clone(),
                            tool_use_id: tc_id.clone(),
                            content: err_content.clone(),
                            is_error: true,
                        },
                    );
                    tool_result_blocks.push(ContentBlock::ToolResult {
                        tool_use_id: tc_id.clone(),
                        content: err_content,
                        is_error: Some(true),
                    });
                }
            }

            tool_use_blocks.push(ContentBlock::ToolUse {
                id: tc_id.clone(),
                name: tc_name.clone(),
                input: tc_args.clone(),
            });
        }

        // 累积到 content_blocks
        all_content_blocks.extend(tool_use_blocks.clone());
        all_content_blocks.extend(tool_result_blocks.clone());

        // 追加到 messages：assistant 消息含 tool_use + 文本
        // tool_result 以 tool 角色回传（OpenAI 格式要求 role=tool + tool_call_id）
        // Anthropic adapter 的 split_system_prompt 会把 tool 角色转为 user
        let mut asst_blocks: Vec<ContentBlock> = Vec::new();
        if !round_text.is_empty() {
            asst_blocks.push(ContentBlock::Text {
                text: round_text,
            });
        }
        asst_blocks.extend(tool_use_blocks);
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: asst_blocks,
        });

        // 每个 tool_result 作为独立的 tool 角色消息（OpenAI 格式）
        for block in &tool_result_blocks {
            messages.push(ChatMessage {
                role: "tool".into(),
                content: vec![block.clone()],
            });
        }

        tracing::info!(
            target: "ice_paw.chat",
            "工具执行完成: round={}，准备下一轮 LLM 调用",
            tool_round,
        );
    }

    // 达到最大轮数 → 正常结束（所有工具已完成）
    let content_for_db = all_text.clone();
    if !all_text.is_empty() {
        all_content_blocks.push(ContentBlock::Text {
            text: all_text,
        });
    }
    cleanup_after_success_with_blocks(
        &app, &pool, &conv_id, &asst_msg_id,
        &content_for_db, &all_content_blocks, "tool_use",
        collected_usage,
    );
}



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
