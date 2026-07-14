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

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::crypto;
use crate::db::models::NewMessage;
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::llm::{
    self, is_supported_image_media_type, ChatDelta, ChatMessage, CancellationToken, ChatState,
    ContentBlock, LlmProvider, ToolRegistry,
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
// 入参 / 事件 Payload 结构
// =========================================================================

/// P2-2: 单张图片的最大字节数（base64 解码后的原始字节大小）
///
/// 5MB 限制与 OpenAI / Anthropic 官方建议接近：
/// - OpenAI Vision: 单图 base64 ≤ ~20MB，但实践中 5MB 内体验最佳
/// - Anthropic: 单图 ≤ 5MB（推荐），超过会被服务端拒绝
///
/// 用 `base64` 解码后的字节数校验（不是 base64 字符串长度），
/// 避免「字符串看起来不大但解码后超限」的错误。
pub(crate) const MAX_IMAGE_SIZE: usize = 5 * 1024 * 1024; // 5 MiB

/// P2-2: 单条消息最多图片张数
///
/// OpenAI 文档建议 ≤ 20 张/请求；Anthropic 限制更严格（实测 ≤ 100），
/// 这里统一用 20 保持一致。
pub(crate) const MAX_IMAGE_COUNT: usize = 20;

/// P2-2: 校验 content_blocks 中的图片（含尺寸 / 张数 / 类型 / base64 合法性）
///
/// 在 `send_message` 入口处调用，**先于**任何 DB 写入或 LLM 调用。
///
/// 错误信息直接返回给前端用于 toast 提示（使用 `AppError::Validation`
/// → 前端 kind=`"validation"`，可识别为业务级错误）。
pub(crate) fn validate_images(blocks: &[ContentBlock]) -> AppResult<()> {
    let mut image_count = 0usize;

    for (idx, block) in blocks.iter().enumerate() {
        if let ContentBlock::Image { data, media_type } = block {
            image_count += 1;

            // 1. media_type 白名单
            if !is_supported_image_media_type(media_type) {
                return Err(AppError::Validation(format!(
                    "第 {} 张图片格式不支持：{}（允许：png / jpeg / gif / webp）",
                    idx + 1,
                    media_type
                )));
            }

            // 2. base64 解码 + 尺寸校验
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| {
                    AppError::Validation(format!(
                        "第 {} 张图片 base64 解码失败：{}",
                        idx + 1,
                        e
                    ))
                })?;
            if decoded.len() > MAX_IMAGE_SIZE {
                let mb = decoded.len() as f64 / 1024.0 / 1024.0;
                return Err(AppError::Validation(format!(
                    "第 {} 张图片过大：{:.2} MB（最大 {} MB）",
                    idx + 1,
                    mb,
                    MAX_IMAGE_SIZE / 1024 / 1024
                )));
            }
        }
    }

    // 3. 张数上限
    if image_count > MAX_IMAGE_COUNT {
        return Err(AppError::Validation(format!(
            "单条消息最多 {} 张图片，当前 {} 张",
            MAX_IMAGE_COUNT, image_count
        )));
    }

    Ok(())
}

/// `send_message` 入参中的模板部分（P2-4）
///
/// - `template_id`  选中的模板 ID
/// - `values`       变量值字典
#[derive(Debug, Clone, Deserialize)]
pub struct TemplateInput {
    pub template_id: String,
    #[serde(default)]
    pub values: std::collections::HashMap<String, String>,
}

/// `send_message` 入参
///
/// P2-2 双接口：
/// - `content: Option<String>` — 旧接口，纯文本（保持向后兼容）
/// - `content_blocks: Option<Vec<ContentBlock>>` — 新接口，支持图片等多模态块
///
/// 优先级：`content_blocks` 存在时优先使用；否则 fallback 到 `content`。
/// 两者都不提供 → 校验失败（与旧版「content 不能为空」一致）。
#[derive(Debug, Deserialize)]
pub struct SendMessageInput {
    pub conversation_id: String,
    /// 旧接口：纯文本（与 P2-1 之前一致）
    /// P2-2 后改为 `Option<String>`，与 `content_blocks` 二选一
    #[serde(default)]
    pub content: Option<String>,
    /// P2-2: 新接口：富文本块（含 Image 等多模态）
    #[serde(default)]
    pub content_blocks: Option<Vec<ContentBlock>>,
    /// 可选：附加的模板（应用后会被渲染并注入到 system_prompt / user_prompt_prefix）
    #[serde(default)]
    pub template: Option<TemplateInput>,
    /// P2-1: 是否启用工具调用
    #[serde(default)]
    pub tools_enabled: bool,
}

/// `chat:start` 事件 payload
#[derive(Clone, Serialize)]
struct ChatStartPayload {
    conversation_id: String,
    user_message_id: String,
    assistant_message_id: String,
}

/// `chat:chunk` 事件 payload
#[derive(Clone, Serialize)]
struct ChatChunkPayload {
    conversation_id: String,
    message_id: String,
    delta: String,
}

/// `chat:done` 事件 payload
#[derive(Clone, Serialize)]
struct ChatDonePayload {
    conversation_id: String,
    message_id: String,
    finish_reason: String,
    /// P2-3: Token 用量信息
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<llm::TokenUsage>,
}

/// `chat:error` 事件 payload
#[derive(Clone, Serialize)]
struct ChatErrorPayload {
    conversation_id: String,
    message_id: String,
    kind: String,
    message: String,
}

/// `chat:retrying` 事件 payload — 通知前端正在重试
#[derive(Clone, Serialize)]
struct ChatRetryingPayload {
    conversation_id: String,
    message_id: String,
    attempt: u32,
    max_attempts: u32,
}

// === P2-1 工具调用事件 payload ===

/// `chat:tool-call-start` 事件 payload
#[derive(Clone, Serialize)]
struct ChatToolCallStartPayload {
    conversation_id: String,
    message_id: String,
    id: String,
    name: String,
}

/// `chat:tool-call-delta` 事件 payload
#[derive(Clone, Serialize)]
struct ChatToolCallDeltaPayload {
    conversation_id: String,
    message_id: String,
    id: String,
    delta: String,
}

/// `chat:tool-call-end` 事件 payload
#[derive(Clone, Serialize)]
struct ChatToolCallEndPayload {
    conversation_id: String,
    message_id: String,
    id: String,
}

/// `chat:tool-result` 事件 payload
#[derive(Clone, Serialize)]
struct ChatToolResultPayload {
    conversation_id: String,
    message_id: String,
    tool_use_id: String,
    content: String,
    is_error: bool,
}

/// `chat:thinking` 事件 payload
#[derive(Clone, Serialize)]
struct ChatThinkingPayload {
    conversation_id: String,
    message_id: String,
    content: String,
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

/// 成功完成后的收尾：回写 content + content_blocks + emit done + 注销 token
fn cleanup_after_success_with_blocks(
    app: &AppHandle,
    pool: &SqlitePool,
    conv_id: &str,
    asst_msg_id: &str,
    content: &str,
    content_blocks: &[ContentBlock],
    finish_reason: &str,
    usage: Option<llm::TokenUsage>,
) {
    let pool_clone = pool.clone();
    let asst_msg_id_clone = asst_msg_id.to_string();
    let content_clone = content.to_string();
    let blocks_json = serde_json::to_string(content_blocks).unwrap_or_else(|_| "[]".to_string());

    tokio::spawn(async move {
        let _ = repo::message::update_content(&pool_clone, &asst_msg_id_clone, &content_clone).await;
        let _ = repo::message::update_content_blocks(&pool_clone, &asst_msg_id_clone, &blocks_json).await;
    });

    let _ = app.emit(
        "chat:done",
        ChatDonePayload {
            conversation_id: conv_id.to_string(),
            message_id: asst_msg_id.to_string(),
            finish_reason: finish_reason.to_string(),
            usage,
        },
    );
    cleanup(app, pool, conv_id);
}

/// 注销 CancellationToken（所有退出路径的公共收尾）
fn cleanup(app: &AppHandle, _pool: &SqlitePool, conv_id: &str) {
    let chat_state = app.state::<ChatState>();
    chat_state.unregister(conv_id);
}

/// 把 AppError 映射为前端可读的 kind 字符串
fn error_kind(e: &crate::error::AppError) -> String {
    match e {
        crate::error::AppError::Llm(_) => "llm".into(),
        crate::error::AppError::Stream(_) => "stream".into(),
        crate::error::AppError::Cancelled => "cancelled".into(),
        _ => "internal".into(),
    }
}

/// 把 LLM/Stream 错误消息映射为用户可读的中文友好提示
///
/// 匹配逻辑：大小写不敏感地扫描常见错误关键词（图片安全审核 / 限流 /
/// 鉴权失败 / token 超限 等）。未匹配时返回原消息，方便开发者调试。
///
/// 注意：仅影响通过 `chat:error` 事件下发给前端的 `message` 字段；
/// `repo::message::update_error` 仍写入原始错误（便于日志排查）。
fn friendly_error(msg: &str) -> String {
    let lower = msg.to_lowercase();
    if lower.contains("sensitive") || lower.contains("content_filter") {
        return "图片内容未通过安全审核，请更换图片后重试".into();
    }
    if lower.contains("rate_limit") || lower.contains("rate limit") {
        return "请求过于频繁，请稍后再试".into();
    }
    if lower.contains("401") {
        return "API 密钥无效或已过期，请在设置中检查".into();
    }
    if lower.contains("403") {
        return "API 权限不足，请检查配置".into();
    }
    if lower.contains("context_length") || lower.contains("token") {
        return "消息过长，请缩短内容或清除部分历史消息".into();
    }
    // 其他错误保持原样（开发者调试用）
    msg.to_string()
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
    // P2-2: validate_images 与 SendMessageInput 入参
    // ================================================================

    use base64::Engine as _;

    /// 构造 N 字节原始数据 → base64 字符串
    fn make_b64_bytes(n: usize) -> String {
        base64::engine::general_purpose::STANDARD.encode(vec![0u8; n])
    }

    #[test]
    fn validate_images_empty_blocks_ok() {
        // 无图片 → 直接通过
        assert!(validate_images(&[]).is_ok());
        let blocks = vec![ContentBlock::text("纯文本")];
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_small_image_ok() {
        let blocks = vec![ContentBlock::image(make_b64_bytes(1024), "image/png")];
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_too_large_rejected() {
        // 6 MiB > 5 MiB 上限
        let big = make_b64_bytes(6 * 1024 * 1024);
        let blocks = vec![ContentBlock::image(big, "image/png")];
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(msg.contains("过大"), "错误信息应提示过大，实际: {}", msg);
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_exactly_5mb_ok() {
        // 5 MiB 边界值应放行
        let exact = make_b64_bytes(MAX_IMAGE_SIZE);
        let blocks = vec![ContentBlock::image(exact, "image/png")];
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_5mb_plus_one_rejected() {
        let over = make_b64_bytes(MAX_IMAGE_SIZE + 1);
        let blocks = vec![ContentBlock::image(over, "image/png")];
        assert!(validate_images(&blocks).is_err());
    }

    #[test]
    fn validate_images_unsupported_media_type_rejected() {
        let blocks = vec![ContentBlock::image(make_b64_bytes(100), "image/bmp")];
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(msg.contains("不支持"), "错误信息应提示不支持，实际: {}", msg);
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_invalid_base64_rejected() {
        let blocks = vec![ContentBlock::image("not_base64!@#$%", "image/png")];
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(
                    msg.contains("base64"),
                    "错误信息应提到 base64，实际: {}",
                    msg
                );
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_count_limit() {
        // 21 张 1KB 图片 → 超过 MAX_IMAGE_COUNT=20
        let blocks: Vec<ContentBlock> = (0..21)
            .map(|_| ContentBlock::image(make_b64_bytes(1024), "image/png"))
            .collect();
        let err = validate_images(&blocks).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(msg.contains("最多"), "错误信息应提到最多，实际: {}", msg);
            }
            _ => panic!("应为 Validation 错误"),
        }
    }

    #[test]
    fn validate_images_exactly_max_count_ok() {
        // 恰好 20 张 → 应放行
        let blocks: Vec<ContentBlock> = (0..MAX_IMAGE_COUNT)
            .map(|_| ContentBlock::image(make_b64_bytes(1024), "image/png"))
            .collect();
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_mixed_text_and_images_ok() {
        // 文本 + 多张图片混合
        let mut blocks = vec![ContentBlock::text("看这些图")];
        for _ in 0..3 {
            blocks.push(ContentBlock::image(make_b64_bytes(1024), "image/png"));
        }
        blocks.push(ContentBlock::text("请描述"));
        assert!(validate_images(&blocks).is_ok());
    }

    #[test]
    fn validate_images_supports_all_four_types() {
        for mt in ["image/png", "image/jpeg", "image/gif", "image/webp"] {
            let blocks = vec![ContentBlock::image(make_b64_bytes(100), mt)];
            assert!(validate_images(&blocks).is_ok(), "{} 应被允许", mt);
        }
    }

    // --- SendMessageInput 序列化（确认前端 JSON 格式） ---

    #[test]
    fn send_input_accepts_legacy_content() {
        // 旧版 JSON（仅 content）应能反序列化
        let json = r#"{"conversation_id":"c1","content":"hello"}"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.conversation_id, "c1");
        assert_eq!(input.content.as_deref(), Some("hello"));
        assert!(input.content_blocks.is_none());
        assert!(!input.tools_enabled);
    }

    #[test]
    fn send_input_accepts_content_blocks() {
        // 新版 JSON（含 content_blocks）
        let json = r#"{
            "conversation_id": "c1",
            "content_blocks": [
                {"type": "text", "text": "看图"},
                {"type": "image", "data": "AAAA", "media_type": "image/png"}
            ],
            "tools_enabled": true
        }"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.conversation_id, "c1");
        assert!(input.content.is_none());
        let blocks = input.content_blocks.unwrap();
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            ContentBlock::Text { text } => assert_eq!(text, "看图"),
            _ => panic!("第一个应为 Text"),
        }
        match &blocks[1] {
            ContentBlock::Image { data, media_type } => {
                assert_eq!(data, "AAAA");
                assert_eq!(media_type, "image/png");
            }
            _ => panic!("第二个应为 Image"),
        }
        assert!(input.tools_enabled);
    }

    #[test]
    fn send_input_accepts_both_legacy_and_new() {
        // 同时传 content 和 content_blocks → 都应能反序列化
        // （后端逻辑会优先使用 content_blocks）
        let json = r#"{
            "conversation_id": "c1",
            "content": "legacy text",
            "content_blocks": [
                {"type": "text", "text": "new text"}
            ]
        }"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.content.as_deref(), Some("legacy text"));
        assert!(input.content_blocks.is_some());
    }

    #[test]
    fn send_input_minimal_required_fields() {
        // 仅 conversation_id + content_blocks → 其它字段默认值正确
        let json = r#"{"conversation_id":"c1","content_blocks":[]}"#;
        let input: SendMessageInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.conversation_id, "c1");
        assert!(input.content.is_none());
        // 空数组 → Some(vec![])，后续逻辑会 fallback 到 legacy_content 校验
        let blocks = input.content_blocks.unwrap();
        assert!(blocks.is_empty());
        assert!(!input.tools_enabled);
        assert!(input.template.is_none());
    }

    // ================================================================
    // friendly_error 映射
    // ================================================================

    #[test]
    fn friendly_error_sensitive_content() {
        // 图片安全审核 → 中文友好提示
        let raw = "LLM 调用错误: HTTP 500 Internal Server Error: api_error: \
                   input new_sensitive, messages[21]'s content[0] image is sensitive (1026)";
        let out = friendly_error(raw);
        assert!(
            out.contains("安全审核"),
            "sensitive 错误应映射为中文友好提示，实际: {}",
            out
        );
        assert!(!out.contains("HTTP 500"));
    }

    #[test]
    fn friendly_error_content_filter() {
        // Azure / OpenAI moderation 风格的 content_filter
        let raw = "API returned 400: content_filter triggered";
        let out = friendly_error(raw);
        assert!(out.contains("安全审核"));
    }

    #[test]
    fn friendly_error_rate_limit() {
        let raw1 = "HTTP 429: rate_limit_exceeded";
        let raw2 = "Too Many Requests: rate limit reached, please retry after 30s";
        assert!(friendly_error(raw1).contains("过于频繁"));
        assert!(friendly_error(raw2).contains("过于频繁"));
    }

    #[test]
    fn friendly_error_401() {
        let raw = "HTTP 401 Unauthorized: invalid api key";
        let out = friendly_error(raw);
        assert!(out.contains("API 密钥"));
        assert!(out.contains("设置"));
    }

    #[test]
    fn friendly_error_403() {
        let raw = "HTTP 403 Forbidden: insufficient permissions";
        let out = friendly_error(raw);
        assert!(out.contains("权限"));
    }

    #[test]
    fn friendly_error_context_length() {
        let raw1 = "context_length_exceeded: maximum context length is 8192 tokens";
        let raw2 = "Too many tokens in prompt";
        assert!(friendly_error(raw1).contains("过长"));
        assert!(friendly_error(raw2).contains("过长"));
    }

    #[test]
    fn friendly_error_unknown_passthrough() {
        // 未匹配的错误 → 原样返回，方便开发者调试
        let raw = "Some random network glitch XYZ123";
        assert_eq!(friendly_error(raw), raw);
    }

    #[test]
    fn friendly_error_empty_string() {
        // 空串 → 原样返回（不会 panic）
        assert_eq!(friendly_error(""), "");
    }

    #[test]
    fn friendly_error_case_insensitive() {
        // 关键词匹配大小写不敏感（实现内部已 to_lowercase）
        let raw = "HTTP 500 Internal Server Error: input SENSITIVE content";
        let out = friendly_error(raw);
        assert!(out.contains("安全审核"));
    }
}
