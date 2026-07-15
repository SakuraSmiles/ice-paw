//! 聊天上下文组装 Pipeline（Step 4 抽出）
//!
//! 把 `send_message` 中的「模板渲染 + OS 上下文注入 + system prompt 拼装
//! + 历史转换 + 图片重排」这一整段逻辑抽成单一入口 [`assemble_context`]。
//!
//! 设计目标：
//! - **单一职责**：只负责把 (agent, template, history, user_blocks) 转换成
//!   最终给 LLM 的 `Vec<ChatMessage>`，外加重排后的 user content blocks（供 DB 回写）。
//! - **可测**：模板渲染 / OS 注入 / 图片重排都是纯计算，方便单测。
//! - **行为不变**：把 `send_message` 内的同源代码原样抽出，未引入任何逻辑变更。
//!
//! ## 副作用 / 幂等性
//!
//! - **副作用**：仅 `repo::template::get_by_id` 的只读 SELECT（当传入了 `template_input` 时）。
//!   不会写 DB、不会发事件、不会操作网络。
//! - **幂等性**：纯计算 + 只读查询；相同输入 → 相同输出；无外部可观测状态变更。
//!   （系统时间戳、随机数等仅在 `stream_loop` 阶段使用，与本函数无关。）
//! - **OS 信息注入**：`build_os_context()` 产出的 `os_info` 已在此函数内部
//!   拼入 system prompt，不会作为独立字段返回（详见 [`assemble_context`] 文档）。
//!
//! ## 历史消息的多模态
//!
//! 当前实现把历史消息当成纯文本处理（`ChatMessage::from_text`），即历史中的
//! `content_blocks` 字段（图片/工具块等）**未还原到 LLM context**。
//! 标记为 TODO：等「会话级多模态历史」需求落地时再扩展。

use sqlx::SqlitePool;

use crate::context::template::render_template;
use crate::db::models::{AgentRow, MessageRow};
use crate::db::repo;
use crate::error::AppResult;
use crate::infra::protocol::{ChatMessage, ContentBlock, TemplateInput};

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
// assemble_context — 上下文组装 Pipeline
// =========================================================================

/// `assemble_context` 的返回结构
///
/// - `messages`：可直接喂给 `provider.stream_chat(messages, ...)` 的完整上下文
///   （含 system / 历史 / 当前 user）
/// - `user_blocks`：含图片重排后的当前用户消息 blocks，供 DB 回写
///   （`user_blocks_json` 列 + `content_text_for_db` 文本部分）
#[derive(Debug)]
pub(crate) struct AssembledContext {
    pub messages: Vec<ChatMessage>,
    pub user_blocks: Vec<ContentBlock>,
}

/// 组装 LLM 调用上下文
///
/// # Pipeline 流程
///
/// 1. **模板查询 + 渲染**（可选）
///    - 如果 `template_input` 是 `Some`，调用 `repo::template::get_by_id` 取出模板行
///      （找不到 → 返回 `AppError::NotFound`）。
///    - `render_template` 把 `values` 代入 `system_prompt` 和 `user_prompt_prefix`。
///    - 模板 `system_prompt` 渲染后为空字符串 → 视为「不覆盖 system」（保留 agent 的）。
/// 2. **user blocks 拼装 + 图片重排**
///    - `rendered_user_prefix` 非空 → 在 user blocks 头部插入一个 Text 块。
///    - 含图片 → 重排为「images 在前，texts 在后」（OpenAI Vision 要求）。
/// 3. **system prompt 构造**
///    - 优先级：`template.system_prompt` > `agent.system_prompt` > 工具能力提示 > OS 上下文。
///    - OS 上下文在**此阶段**注入 system prompt，不作为独立字段返回。
/// 4. **历史消息转换**
///    - 跳过非 user/assistant/system 角色（如 `tool`）。
///    - 仅取 `MessageRow.content`（文本），历史中的 `content_blocks` 不还原 → **TODO**。
/// 5. **当前用户消息追加**（含 `user_blocks`）。
///
/// # 副作用
///
/// 仅 `repo::template::get_by_id` 的只读 SELECT（提供 `template_input` 时触发）。
/// 无 DB 写入、无事件 emit、无网络 IO。
///
/// # 幂等性
///
/// 纯计算 + 只读查询；相同输入 → 相同 `AssembledContext`。
///
/// # 错误
///
/// - `AppError::NotFound { resource: "template", id }` — `template_id` 查不到模板
/// - `AppError::Database(_)` — 来自 `repo::template::get_by_id` 的 SQL 错误
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
        // 模板 system_prompt 渲染后为空 → 视为「不覆盖」，fallback 到 agent
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

    // -----------------------------------------------------------------
    // 3) system prompt 构造（template > agent > tool_hint > os_context）
    // -----------------------------------------------------------------
    let mut effective_system_prompt = rendered_system_prompt
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(if agent.system_prompt.is_empty() {
            None
        } else {
            Some(agent.system_prompt.as_str())
        })
        .map(|s| s.to_string());

    // P2-1: 工具启用时追加工具能力提示
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

    // -----------------------------------------------------------------
    // 4) 构造 messages 列表（system + 历史 + 当前 user）
    // -----------------------------------------------------------------
    let mut messages: Vec<ChatMessage> = Vec::with_capacity(history.len() + 2);

    if let Some(sys) = &effective_system_prompt {
        messages.push(ChatMessage::from_text("system", sys.clone()));
    }

    // 历史消息（仅文本；多模态 TODO：等会话级多模态历史需求落地时再扩展）
    for msg in history {
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

    Ok(AssembledContext {
        messages,
        user_blocks,
    })
}


