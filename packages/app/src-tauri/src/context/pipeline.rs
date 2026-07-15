//! Context 组装 Pipeline — 上下文组装主入口
//!
//! 从 `commands/chat_context.rs` 迁入（W5.3）。
//!
//! 提供 `pub(crate)` 函数 [`assemble_context`]，将 (agent, template, history, user_blocks)
//! 转换成最终给 LLM 的 `Vec<ChatMessage>` 和用于 DB 回写的 `user_blocks`。

use sqlx::SqlitePool;

use crate::context::history::load_history;
use crate::context::os_context::build_os_context;
use crate::context::system_prompt::build_system_prompt;
use crate::context::template::render_template;
use crate::db::models::{AgentRow, MessageRow};
use crate::db::repo;
use crate::error::AppResult;
use crate::infra::protocol::{ChatMessage, ContentBlock, TemplateInput};

// =========================================================================
// AssembledContext + assemble_context
// =========================================================================

/// `assemble_context` 的返回结构
///
/// - `messages`：可直接喂给 `provider.stream_chat(messages, ...)` 的完整上下文
///   （含 system / 历史 / 当前 user）
/// - `user_blocks`：含图片重排后的当前用户消息 blocks，供 DB 回写
#[derive(Debug)]
pub(crate) struct AssembledContext {
    pub messages: Vec<ChatMessage>,
    pub user_blocks: Vec<ContentBlock>,
}

/// 组装 LLM 调用上下文
///
/// # Pipeline 流程
///
/// 1. **模板查询 + 渲染**（可选，副作用：只读 SELECT）
/// 2. **user blocks 拼装 + 图片重排**（纯计算）
/// 3. **system prompt 构造**（委托 `context::system_prompt`）
/// 4. **历史消息转换**（委托 `context::history`）
/// 5. **当前用户消息追加**
pub(crate) async fn assemble_context(
    pool: &SqlitePool,
    agent: &AgentRow,
    template_input: Option<&TemplateInput>,
    history: &[MessageRow],
    final_blocks: Vec<ContentBlock>,
    tools_enabled: bool,
) -> AppResult<AssembledContext> {
    // -----------------------------------------------------------------
    // 1) 模板查询 + 渲染（副作用：只读 SELECT）
    // -----------------------------------------------------------------
    let (rendered_system_prompt, rendered_user_prefix) = if let Some(tpl_input) = template_input {
        let tpl = repo::template::get_by_id(pool, &tpl_input.template_id).await?;
        let sys = render_template(&tpl.system_prompt, &tpl_input.values);
        let sys_opt = if sys.trim().is_empty() { None } else { Some(sys) };
        let user_pfx = render_template(&tpl.user_prompt_prefix, &tpl_input.values);
        (sys_opt, user_pfx)
    } else {
        (None, String::new())
    };

    // -----------------------------------------------------------------
    // 2) user_blocks 拼装 + 图片重排（纯计算）
    // -----------------------------------------------------------------
    let mut user_blocks: Vec<ContentBlock> = if rendered_user_prefix.is_empty() {
        final_blocks
    } else {
        let mut v = Vec::with_capacity(final_blocks.len() + 1);
        v.push(ContentBlock::text(rendered_user_prefix));
        v.extend(final_blocks.iter().cloned());
        v
    };

    // 重排：images 在前，texts 在后（OpenAI Vision 要求）
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

    // -----------------------------------------------------------------
    // 3) system prompt 构造
    // -----------------------------------------------------------------
    let os_info = build_os_context();
    let effective_system_prompt = build_system_prompt(
        rendered_system_prompt.as_deref(),
        &agent.system_prompt,
        tools_enabled,
        &os_info,
    );

    // -----------------------------------------------------------------
    // 4) 历史消息转换
    // -----------------------------------------------------------------
    let history_messages = load_history(history);

    // -----------------------------------------------------------------
    // 5) 构造 messages 列表（system + 历史 + 当前 user）
    // -----------------------------------------------------------------
    let mut messages: Vec<ChatMessage> = Vec::with_capacity(history_messages.len() + 2);

    if let Some(sys) = &effective_system_prompt {
        messages.push(ChatMessage::from_text("system", sys.clone()));
    }

    messages.extend(history_messages);

    // 当前用户消息（含图片的 content_blocks）
    messages.push(ChatMessage {
        role: "user".into(),
        content: user_blocks.clone(),
    });

    Ok(AssembledContext {
        messages,
        user_blocks,
    })
}
