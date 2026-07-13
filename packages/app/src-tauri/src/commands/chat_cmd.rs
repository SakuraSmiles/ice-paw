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

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::crypto;
use crate::db::models::NewMessage;
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::llm::{self, ChatDelta, ChatMessage, CancellationToken, ChatState, LlmProvider};

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
// 入参 / 事件 Payload 结构
// =========================================================================

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
#[derive(Debug, Deserialize)]
pub struct SendMessageInput {
    pub conversation_id: String,
    pub content: String,
    /// 可选：附加的模板（应用后会被渲染并注入到 system_prompt / user_prompt_prefix）
    #[serde(default)]
    pub template: Option<TemplateInput>,
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
    if input.content.trim().is_empty() {
        return Err(AppError::Validation("content 不能为空".into()));
    }

    let conv_id = input.conversation_id.clone();
    let content = input.content.clone();

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
    let provider = llm::create_provider(&agent.provider, &agent.model, base_url)?;

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

    // --- 拼装最终用户消息（prefix + content）---
    let final_user_content = if rendered_user_prefix.is_empty() {
        content.clone()
    } else {
        format!("{}{}", rendered_user_prefix, content)
    };

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

    // system prompt 优先级：模板 > agent
    // 模板未提供 system_prompt → 使用 agent 的
    // 两者都为空 → 不发 system 消息
    let effective_system_prompt = rendered_system_prompt
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(if agent.system_prompt.is_empty() {
            None
        } else {
            Some(agent.system_prompt.as_str())
        });
    if let Some(sys) = effective_system_prompt {
        messages.push(ChatMessage {
            role: "system".into(),
            content: sys.to_string(),
        });
    }

    // 历史消息
    for msg in &history {
        let role = match msg.role.as_str() {
            "user" | "assistant" | "system" => msg.role.clone(),
            _ => continue, // 跳过 tool 等不支持的角色
        };
        messages.push(ChatMessage {
            role,
            content: msg.content.clone(),
        });
    }

    // 当前用户消息（带模板 prefix）
    messages.push(ChatMessage {
        role: "user".into(),
        content: final_user_content.clone(),
    });

    // --- 写用户消息到 DB（存原始 content；模板 prefix 不入 DB）---
    let user_msg_id = Uuid::new_v4().to_string();
    repo::message::create(
        pool.inner(),
        &user_msg_id,
        &NewMessage {
            conversation_id: conv_id.clone(),
            role: "user".into(),
            content: content.clone(),
            token_count: None,
            error: None,
        },
    )
    .await?;

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
        )
        .await;
    });

    Ok(())
}

/// 流式生成内部协程 — 支持指数退避重试
///
/// 重试策略：
/// - 首次失败 → 等待 1s → 第 2 次尝试
/// - 二次失败 → 等待 2s → 第 3 次尝试
/// - 三次失败 → 等待 4s → 第 4 次尝试（总计 4 次，即最多 3 次重试）
/// - 超过 4 次 → 放弃，emit chat:error
///
/// 重试时：
/// - 保留已收到的内容（buffer 不清空）
/// - 前端通过 chat:retrying 事件显示「正在重新连接...」过渡态
/// - 重试请求时把已收集的 buffer 拼到 messages 末尾作为 assistant 消息，让 LLM 接上
///
/// 不重试的情况：
/// - 用户主动取消（cancel.is_cancelled()）
/// - 不可重试错误（401/403 等）
async fn stream_loop(
    app: AppHandle,
    pool: SqlitePool,
    provider: Arc<dyn LlmProvider>,
    api_key: String,
    messages: Vec<ChatMessage>,
    temperature: f64,
    max_tokens: i32,
    cancel: CancellationToken,
    conv_id: String,
    asst_msg_id: String,
) {
    use futures::StreamExt;
    use std::time::Duration;

    let mut buffer = String::new();
    let mut finish_reason = "stop".to_string();
    let had_error = std::cell::RefCell::new(false);
    const MAX_ATTEMPTS: u32 = 4; // 1 次初始 + 3 次重试

    for attempt in 0..MAX_ATTEMPTS {
        // 取消检查（重试前也检查）
        if cancel.is_cancelled() {
            // 取消：跳出循环，由末尾的清理逻辑处理
            // （不发 chat:done，cleanup() 路径仅注销 token）
            break;
        }

        // 如果不是首次尝试，等待指数退避并通知前端
        if attempt > 0 {
            let wait_secs = 1u64 << (attempt - 1); // 1s, 2s, 4s
            tracing::info!(
                target: "ice_paw.chat",
                "重试 LLM 请求: attempt={}/{}，等待 {}s",
                attempt + 1,
                MAX_ATTEMPTS,
                wait_secs,
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

            // 等待后再次检查取消
            if cancel.is_cancelled() {
                // 同上：跳出循环，由末尾清理逻辑处理
                break;
            }
        }

        // 构造重试时的 messages：如果有已收集内容，追加为 assistant 消息
        let retry_messages = if !buffer.is_empty() && attempt > 0 {
            let mut msgs = messages.clone();
            msgs.push(ChatMessage {
                role: "assistant".into(),
                content: format!(
                    "[以下是上一轮因网络中断已收到的部分回复，请从此处继续]\n{}",
                    &buffer
                ),
            });
            msgs
        } else {
            messages.clone()
        };

        // 调 provider 拿到流
        let stream_result = provider
            .stream_chat(&api_key, retry_messages, temperature, max_tokens, cancel.clone())
            .await;

        match stream_result {
            Ok(mut stream) => {
                let mut attempt_ok = true;

                while let Some(item) = stream.next().await {
                    if cancel.is_cancelled() {
                        // 正常退出循环，不标记 error
                        return cleanup(&app, &pool, &conv_id);
                    }

                    match item {
                        Ok(ChatDelta::Delta { content: delta }) => {
                            buffer.push_str(&delta);
                            let _ = app.emit(
                                "chat:chunk",
                                ChatChunkPayload {
                                    conversation_id: conv_id.clone(),
                                    message_id: asst_msg_id.clone(),
                                    delta,
                                },
                            );
                        }
                        Ok(ChatDelta::Done { finish_reason: fr }) => {
                            if let Some(fr) = fr {
                                finish_reason = fr;
                            }
                            // 成功完成
                            return cleanup_after_success(
                                &app,
                                &pool,
                                &conv_id,
                                &asst_msg_id,
                                &buffer,
                                &finish_reason,
                            );
                        }
                        Err(e) => {
                            // 流中错误：判断是否可重试
                            if e.is_retryable() {
                                attempt_ok = false;
                                tracing::warn!(
                                    target: "ice_paw.chat",
                                    "流中可重试错误 (attempt {}/{}): {}",
                                    attempt + 1,
                                    MAX_ATTEMPTS,
                                    e
                                );
                                break; // 跳出 inner while，进入下一轮重试
                            } else {
                                // 不可重试：直接报错退出
                                *had_error.borrow_mut() = true;
                                let err_msg = e.to_string();
                                let _ = app.emit(
                                    "chat:error",
                                    ChatErrorPayload {
                                        conversation_id: conv_id.clone(),
                                        message_id: asst_msg_id.clone(),
                                        kind: error_kind(&e),
                                        message: err_msg.clone(),
                                    },
                                );
                                let _ = repo::message::update_error(&pool, &asst_msg_id, &err_msg).await;
                                return cleanup(&app, &pool, &conv_id);
                            }
                        }
                    }
                }

                // 如果 inner while 正常退出（stream 自然结束但没收到 Done），
                // 且 attempt_ok 仍为 true，说明流正常结束
                if attempt_ok {
                    return cleanup_after_success(
                        &app,
                        &pool,
                        &conv_id,
                        &asst_msg_id,
                        &buffer,
                        &finish_reason,
                    );
                }
                // attempt_ok == false → 继续重试
            }
            Err(e) => {
                // provider.stream_chat 本身失败
                if e.is_retryable() {
                    tracing::warn!(
                        target: "ice_paw.chat",
                        "请求失败可重试 (attempt {}/{}): {}",
                        attempt + 1,
                        MAX_ATTEMPTS,
                        e
                    );
                    // 继续下一轮重试
                } else {
                    // 不可重试：直接报错退出
                    *had_error.borrow_mut() = true;
                    let err_msg = e.to_string();
                    let _ = app.emit(
                        "chat:error",
                        ChatErrorPayload {
                            conversation_id: conv_id.clone(),
                            message_id: asst_msg_id.clone(),
                            kind: error_kind(&e),
                            message: err_msg.clone(),
                        },
                    );
                    let _ = repo::message::update_error(&pool, &asst_msg_id, &err_msg).await;
                    return cleanup(&app, &pool, &conv_id);
                }
            }
        }
    }

    // 重试耗尽，回写已收集内容 + 错误标记
    let err_msg = format!("连接重试已耗尽（共 {} 次），已收到部分内容", MAX_ATTEMPTS);
    if !buffer.is_empty() {
        // 有部分内容：回写内容但不标记 error（用户能看到部分结果）
        let _ = repo::message::update_content(&pool, &asst_msg_id, &buffer).await;
    }
    let _ = repo::message::update_error(&pool, &asst_msg_id, &err_msg).await;
    let _ = app.emit(
        "chat:error",
        ChatErrorPayload {
            conversation_id: conv_id.clone(),
            message_id: asst_msg_id.clone(),
            kind: "stream".into(),
            message: err_msg,
        },
    );
    cleanup(&app, &pool, &conv_id);
}

/// 成功完成后的收尾：回写内容 + emit done + 注销 token
fn cleanup_after_success(
    app: &AppHandle,
    pool: &SqlitePool,
    conv_id: &str,
    asst_msg_id: &str,
    buffer: &str,
    finish_reason: &str,
) {
    // 回写内容是异步的，spawn 一个 detached task
    let pool_clone = pool.clone();
    let asst_msg_id_clone = asst_msg_id.to_string();
    let buffer_clone = buffer.to_string();
    tokio::spawn(async move {
        let _ = repo::message::update_content(&pool_clone, &asst_msg_id_clone, &buffer_clone).await;
    });

    let _ = app.emit(
        "chat:done",
        ChatDonePayload {
            conversation_id: conv_id.to_string(),
            message_id: asst_msg_id.to_string(),
            finish_reason: finish_reason.to_string(),
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
}
