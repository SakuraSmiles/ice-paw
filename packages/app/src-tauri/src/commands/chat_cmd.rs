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
use super::chat_error::{error_kind, friendly_error};
use super::chat_protocol::{
    ChatChunkPayload, ChatErrorPayload, ChatRetryingPayload, ChatStartPayload,
    ChatThinkingPayload, ChatToolCallDeltaPayload, ChatToolCallEndPayload, ChatToolCallStartPayload,
    ChatToolResultPayload, SendMessageInput, validate_images,
};

// =========================================================================
// 模板渲染（P2-4）
// =========================================================================

/// 用变量值渲染模板内容。
///
/// 规则：扫描文本中的 `{{var_name}}` 段，依次替换为 `values` 中对应 key 的值。
/// - 变量名必须是 `[a-zA-Z_][a-zA-Z0-9_]*`
/// - 模板中出现的 `var_name` 不在 `values` 中：保持原样（`{{var_name}}`）
///   以便 LLM 能看到「未填的占位符」并主动追问
/// - `values` 中多余的 key 会被忽略
///
/// 与 mustache 的差异：
/// - 不支持 `{{#section}}...{{/section}}` / `{{! comment}}` / `{{>partial}}` 等高级语法
/// - 不支持 `.` 路径访问
///
/// 故意保持简单：模板只是「带变量的纯文本」，不引入模板引擎依赖。
pub(crate) fn render_template(
    template: &str,
    values: &std::collections::HashMap<String, String>,
) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // 查找下一个 {{
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // 寻找匹配的 }}
            let mut j = i + 2;
            let mut found = None;
            while j + 1 < bytes.len() {
                if bytes[j] == b'}' && bytes[j + 1] == b'}' {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = found {
                // 取出变量名（trim 空白）
                let name_raw = &template[i + 2..end];
                let name = name_raw.trim();
                // 校验变量名合法性
                if is_valid_var_name(name) {
                    if let Some(v) = values.get(name) {
                        out.push_str(v);
                    } else {
                        // 未提供的变量：保持原样
                        out.push_str(&template[i..end + 2]);
                    }
                } else {
                    // 非法变量名：保持原样
                    out.push_str(&template[i..end + 2]);
                }
                i = end + 2;
                continue;
            }
        }
        // 加上当前字符
        out.push(template[i..].chars().next().unwrap());
        i += template[i..].chars().next().unwrap().len_utf8();
    }
    out
}

/// 变量名合法性：字母/下划线开头 + 字母/数字/下划线
fn is_valid_var_name(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    true
}

// =========================================================================
// 运行环境上下文（B1-3）
// =========================================================================

/// 构建运行环境上下文字符串，注入 system prompt
///
/// 包含：
/// - 操作系统类型（Windows / macOS / Linux）
/// - CPU 架构（如 x86_64 / arm64）
/// - 用户主目录路径（尽力获取，失败则省略）
///
/// 用于帮助 LLM 在工具调用（如 `list_directory`）时使用与当前 OS 兼容的路径，
/// 避免在 Windows 上调用 Linux 风格的 `/home/user/Desktop` 等错误路径。
fn build_os_context() -> String {
    let mut parts: Vec<String> = Vec::new();

    // OS 类型
    let os_name = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    parts.push(format!("操作系统: {}", os_name));

    // CPU 架构（帮助 LLM 理解路径风格，如 arm64 vs x86_64）
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "arm64",
        other => other,
    };
    parts.push(format!("架构: {}", arch));

    // 用户主目录
    let home = get_home_dir();
    if let Some(h) = &home {
        parts.push(format!("用户主目录: {}", h));
    }

    // 组装为提示文本
    let env_info = parts.join("\n");
    format!(
        "## 运行环境\n{}\n\n\
         注意：文件路径必须使用与当前操作系统兼容的格式。\
         调用工具时请使用绝对路径。",
        env_info
    )
}

/// 尽力获取用户主目录
///
/// 优先级：
/// 1. Windows: %USERPROFILE%
/// 2. Unix (macOS/Linux): $HOME
/// 3. 兜底：返回 None
fn get_home_dir() -> Option<String> {
    // Windows: USERPROFILE
    if let Ok(p) = std::env::var("USERPROFILE") {
        if !p.is_empty() {
            return Some(p);
        }
    }
    // Unix: HOME
    if let Ok(p) = std::env::var("HOME") {
        if !p.is_empty() {
            return Some(p);
        }
    }
    None
}

// =========================================================================
// Commands
// =========================================================================

/// 发送消息 — 触发 LLM 流式生成
///
/// 流程：
/// 1. 取会话 → 取 agent → 取 api_key
/// 2. 拉历史消息拼上下文
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

    // --- 如果传入了模板，查表 + 渲染变量（失败 → 报错给前端）---
    let rendered_system_prompt: Option<String> = if let Some(tpl_input) = &input.template {
        let tpl = repo::template::get_by_id(pool.inner(), &tpl_input.template_id).await?;
        let rendered = render_template(&tpl.system_prompt, &tpl_input.values);
        // 模板 system_prompt 为空字符串 → 退化为不使用模板 system_prompt（保留 agent 的）
        if rendered.trim().is_empty() {
            None
        } else {
            Some(rendered)
        }
    } else {
        None
    };
    let rendered_user_prefix: String = if let Some(tpl_input) = &input.template {
        let tpl = repo::template::get_by_id(pool.inner(), &tpl_input.template_id).await?;
        render_template(&tpl.user_prompt_prefix, &tpl_input.values)
    } else {
        String::new()
    };

    // --- P2-2: 拼装最终 user content blocks ---
    // 模板 prefix 注入到 user 消息块数组头部（与旧版「prefix + content」等效）。
    // 图片/文本混合时仍按 OpenAI 要求保持「image 在前，text 在后」的顺序：
    // - Anthropic 无顺序要求
    // - OpenAI Vision 明确要求 image_url 在 text 之前
    //
    // 实现：合并 prefix 与 blocks → 重排为「images → texts」，
    // 这样后面拼 context messages 时直接 push 即可。
    let mut user_blocks: Vec<ContentBlock> = if rendered_user_prefix.is_empty() {
        final_blocks.clone()
    } else {
        let mut v = Vec::with_capacity(final_blocks.len() + 1);
        v.push(ContentBlock::text(rendered_user_prefix));
        v.extend(final_blocks.iter().cloned());
        v
    };

    // 重排：images 在前，texts 在后（OpenAI Vision 要求）
    // ToolUse / ToolResult / Thinking 在 user 消息中理论上不应出现，保留原顺序
    let has_image_in_user = user_blocks.iter().any(|b| b.is_image());
    if has_image_in_user {
        let mut images: Vec<ContentBlock> = Vec::new();
        let mut others: Vec<ContentBlock> = Vec::new();
        for b in user_blocks.drain(..) {
            if b.is_image() {
                images.push(b);
            } else {
                others.push(b);
            }
        }
        user_blocks = images;
        user_blocks.extend(others);
    }

    // --- 拉最近 20 条消息作为上下文 ---
    let history = repo::message::list_by_conversation(
        pool.inner(),
        &conv_id,
        Some(20),
        None,
    )
    .await?;

    // --- 构造上下文消息列表 ---
    let mut messages: Vec<ChatMessage> = Vec::with_capacity(history.len() + 2);

    let tools_enabled = input.tools_enabled;

    // system prompt 优先级：模板 > agent
    // 模板未提供 system_prompt → 使用 agent 的
    // 两者都为空 → 不发 system 消息
    let mut effective_system_prompt = rendered_system_prompt
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(if agent.system_prompt.is_empty() {
            None
        } else {
            Some(agent.system_prompt.as_str())
        })
        .map(|s| s.to_string());

    // P2-1: 工具启用时追加工具能力提示（帮助 GLM 等模型识别工具可用）
    if tools_enabled {
        let tool_hint = "你已启用工具调用能力。当用户要求读取文件、列出目录等操作时，请使用提供的工具（如 list_directory、read_file）来执行，不要回复“无法访问文件”。";
        effective_system_prompt = Some(match effective_system_prompt {
            Some(s) => format!("{}\n\n{}", s, tool_hint),
            None => tool_hint.to_string(),
        });
    }

    // === 注入运行环境信息（始终注入）===
    let os_info = build_os_context();
    if !os_info.is_empty() {
        effective_system_prompt = Some(match effective_system_prompt {
            Some(s) => format!("{}\n\n{}", s, os_info),
            None => os_info,
        });
    }

    if let Some(sys) = &effective_system_prompt {
        messages.push(ChatMessage::from_text("system", sys.clone()));
    }

    // 历史消息
    for msg in &history {
        let role = match msg.role.as_str() {
            "user" | "assistant" | "system" => msg.role.clone(),
            _ => continue, // 跳过 tool 等不支持的角色
        };
        messages.push(ChatMessage::from_text(role, msg.content.clone()));
    }

    // 当前用户消息（含图片的 content_blocks）
    messages.push(ChatMessage {
        role: "user".into(),
        content: user_blocks.clone(),
    });

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

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn vals(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn render_replaces_known_vars() {
        let mut v = HashMap::new();
        v.insert("language".into(), "Rust".into());
        v.insert("framework".into(), "Actix".into());
        let out = render_template("请用 {{language}} + {{framework}} 实现", &v);
        assert_eq!(out, "请用 Rust + Actix 实现");
    }

    #[test]
    fn render_keeps_unknown_vars_intact() {
        let v = vals(&[("lang", "TS")]);
        let out = render_template("Hello {{name}} in {{lang}}", &v);
        assert_eq!(out, "Hello {{name}} in TS");
    }

    #[test]
    fn render_handles_no_vars() {
        let v = HashMap::new();
        assert_eq!(render_template("plain text", &v), "plain text");
    }

    #[test]
    fn render_handles_unicode_value() {
        let v = vals(&[("city", "北京")]);
        let out = render_template("我在 {{city}}", &v);
        assert_eq!(out, "我在 北京");
    }

    #[test]
    fn render_rejects_invalid_var_name_passthrough() {
        // 变量名含空格 / 点 / 数字开头 → 不替换
        let v = vals(&[("good", "OK")]);
        let out = render_template("a {{good}} b {{1bad}} c {{a.b}} d", &v);
        assert_eq!(out, "a OK b {{1bad}} c {{a.b}} d");
    }

    #[test]
    fn render_handles_extra_values() {
        // values 中多余的 key → 忽略
        let v = vals(&[("a", "1"), ("b", "2"), ("c", "3")]);
        let out = render_template("{{a}}/{{b}}", &v);
        assert_eq!(out, "1/2");
    }

    #[test]
    fn render_unmatched_brackets_kept_intact() {
        // 单独的 { 或 } 不应影响
        let v = vals(&[("x", "Y")]);
        let out = render_template("a { single } b {{x}} c { unclosed", &v);
        assert_eq!(out, "a { single } b Y c { unclosed");
    }

    #[test]
    fn render_adjacent_vars() {
        let v = vals(&[("a", "X"), ("b", "Y")]);
        assert_eq!(render_template("{{a}}{{b}}", &v), "XY");
    }

    #[test]
    fn is_valid_var_name_basic() {
        assert!(is_valid_var_name("foo"));
        assert!(is_valid_var_name("_bar"));
        assert!(is_valid_var_name("a1_b2"));
        assert!(!is_valid_var_name(""));
        assert!(!is_valid_var_name("1abc"));
        assert!(!is_valid_var_name("a-b"));
        assert!(!is_valid_var_name("a.b"));
    }

    // ================================================================
}
