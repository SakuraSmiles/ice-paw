//! Pipeline Stage 实现 — 5 个具体 Stage
//!
//! 每个 Stage 都是 `PipelineStage` trait 的一个具体实现，封装了
//! [`crate::context::pipeline::PipelineContext`] 中某一个或多个字段
//! 的读写逻辑。
//!
//! # 阶段顺序（由 [`crate::context::pipeline::PipelineRunner::default_pipeline`] 决定）
//!
//! 1. [`TemplateStage`]      — 模板查询 + 变量渲染
//! 2. [`OsContextStage`]     — OS 运行环境上下文注入
//! 3. [`SystemPromptStage`]  — 四级优先 system_prompt 构造
//! 4. [`HistoryStage`]       — 历史消息行 → `ChatMessage` 转换
//! 5. [`ToolFailureFoldStage`] — 折叠连续重复的失败工具调用（非破坏，仅影响 LLM 视图）
//! 6. [`MemoryStage`]        — M1.5 滚动摘要（独立放在 [`crate::context::memory`] 中）
//! 7. [`FinalAssembleStage`] — 最终拼装（图片重排、user_blocks 拼装、messages 列表组装）
//!
//! `MemoryStage` 不在本模块 —— 它属于 L1 Memory 层（`memory.rs`），按
//! M2.2 拆分规则独立维护。

use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::context::history::{fold_repeated_tool_failures, load_history_with_window, resolve_window};
use crate::context::os_context::build_os_context;
use crate::context::pipeline::{PipelineContext, PipelineStage};
use crate::context::system_prompt::build_system_prompt;
use crate::context::template::render_template;
use crate::db::repo;
use crate::error::AppResult;
use crate::infra::protocol::{ChatMessage, ContentBlock};

// =========================================================================
// Stage 1: TemplateStage — 模板查询 + 变量渲染
// =========================================================================

/// Stage 1：模板查询 + 变量渲染。
///
/// 输入：`ctx.template_input`（来自调用者）
/// 输出：`ctx.rendered_system_prompt` / `ctx.rendered_user_prefix`
///
/// 当 `ctx.template_input` 为 `None` 时，两个输出字段保持空值。
///
/// 持有 `SqlitePool` 的 clone —— `SqlitePool` 内部为 `Arc`，
/// clone 仅增加引用计数，可安全地放进 `Box<dyn PipelineStage + 'static>`。
pub(crate) struct TemplateStage {
    pool: SqlitePool,
}

impl TemplateStage {
    pub(crate) fn new(pool: &SqlitePool) -> Self {
        Self { pool: pool.clone() }
    }
}

#[async_trait]
impl PipelineStage for TemplateStage {
    fn name(&self) -> &'static str {
        "template"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        if let Some(tpl_input) = &ctx.template_input {
            let tpl = repo::template::get_by_id(&self.pool, &tpl_input.template_id).await?;
            let sys = render_template(&tpl.system_prompt, &tpl_input.values);
            ctx.rendered_system_prompt = if sys.trim().is_empty() {
                None
            } else {
                Some(sys)
            };
            ctx.rendered_user_prefix = render_template(&tpl.user_prompt_prefix, &tpl_input.values);
        } else {
            ctx.rendered_system_prompt = None;
            ctx.rendered_user_prefix = String::new();
        }
        Ok(())
    }
}

// =========================================================================
// Stage 2: OsContextStage — OS 运行环境上下文注入
// =========================================================================

/// Stage 2：构建 OS 运行环境上下文字符串。
///
/// 输入：无（基于 `std::env::consts` 派生）
/// 输出：`ctx.os_context`
pub(crate) struct OsContextStage {
    pool: SqlitePool,
}

impl OsContextStage {
    pub(crate) fn new(pool: &SqlitePool) -> Self {
        Self { pool: pool.clone() }
    }
}

#[async_trait]
impl PipelineStage for OsContextStage {
    fn name(&self) -> &'static str {
        "os_context"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        // 从用户预设中读取时区配置
        let tz: Option<String> = sqlx::query_scalar(
            "SELECT value FROM user_preferences WHERE key = 'timezone'"
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();
        ctx.os_context = build_os_context(
            tz.as_deref(),
            ctx.agent.workspace_path.as_deref(),
            ctx.project_workspace.as_deref(),
        );

        // 读取项目级上下文（从 IcePaw 管理的 {workspace}/projects/{id}/ 目录，
        // 不从项目源码目录读——避免泄露、误删、污染用户项目）
        if let Some(ref ctx_dir) = ctx.project_context_dir {
            let dir_path = std::path::Path::new(ctx_dir);

            // project.md — 项目说明（技术栈、架构、业务背景）
            let project_md = dir_path.join("project.md");
            if let Ok(content) = tokio::fs::read_to_string(&project_md).await {
                if !content.trim().is_empty() {
                    ctx.os_context.push_str(&format!(
                        "\n\n## 项目说明\n{}",
                        content.trim()
                    ));
                }
            }

            // conventions.md — 编码规范（命名、格式、最佳实践）
            let conv_md = dir_path.join("conventions.md");
            if let Ok(content) = tokio::fs::read_to_string(&conv_md).await {
                if !content.trim().is_empty() {
                    ctx.os_context.push_str(&format!(
                        "\n\n## 编码规范\n{}",
                        content.trim()
                    ));
                }
            }
        }

        Ok(())
    }
}

// =========================================================================
// Stage 3: SystemPromptStage — 四级优先 system_prompt 构造
// =========================================================================

/// Stage 3：四级优先 system_prompt 构造。
///
/// 优先级（从高到低）：
/// 1. `ctx.rendered_system_prompt`（来自 `TemplateStage`）
/// 2. `ctx.agent.system_prompt`（agent 配置）
/// 3. 工具能力提示（`ctx.tools_enabled` 时追加）
/// 4. `ctx.os_context`（始终注入）
///
/// 输入：`ctx.rendered_system_prompt` / `ctx.agent.system_prompt` / `ctx.tools_enabled` / `ctx.os_context`
/// 输出：`ctx.system_prompt`
pub(crate) struct SystemPromptStage;

#[async_trait]
impl PipelineStage for SystemPromptStage {
    fn name(&self) -> &'static str {
        "system_prompt"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        ctx.system_prompt = build_system_prompt(
            ctx.rendered_system_prompt.as_deref(),
            &ctx.agent.system_prompt,
            ctx.tools_enabled,
            &ctx.os_context,
        );
        Ok(())
    }
}

// =========================================================================
// Stage 4: HistoryStage — 历史消息行 → ChatMessage 转换
// =========================================================================

/// Stage 4：历史消息行 → `ChatMessage` 转换。
///
/// 输入：
/// - `ctx.history`（`Vec<MessageRow>`，按时间正序）
/// - `ctx.agent.max_history_messages`（`Option<i32>`，A3-2 字段）
///
/// 输出：`ctx.history_messages`（`Vec<ChatMessage>`，纯文本）
///
/// A3-2 行为变更：
/// - 根据 `agent.max_history_messages` 解析出有效窗口
///   （`None` / 非正值 → 系统默认 [`crate::context::history::DEFAULT_HISTORY_WINDOW`]）
/// - 仅保留**最近** N 条消息注入 LLM 上下文
/// - 跳过 `tool` 角色（与原 `load_history` 行为一致）
///
/// 为什么在 Stage 而非 DB 加载侧裁剪：
/// - 后续 A3-4 摘要阶段需要读完整历史 → 调用方在 chat_cmd.rs
///   一次性加载充足数据即可，避免双重查询
/// - 窗口配置集中在 Stage 内，未来 P3 「按 token 预算动态截断」
///   可以在同一位置扩展
pub(crate) struct HistoryStage;

#[async_trait]
impl PipelineStage for HistoryStage {
    fn name(&self) -> &'static str {
        "history"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        let window = resolve_window(ctx.agent.max_history_messages);
        ctx.history_messages = load_history_with_window(&ctx.history, Some(window));
        Ok(())
    }
}

// =========================================================================
// Stage 4.5: ToolFailureFoldStage — 折叠连续重复的失败工具调用
// =========================================================================

/// Stage 4.5：折叠连续重复的失败工具调用。
///
/// 在 [`HistoryStage`]（已 sanitize）之后、[`crate::context::memory::MemoryStage`]
/// 之前执行：把"连续 N 次同工具同参数的失败调用"压成 1 条摘要，避免卡死循环的
/// 失败记录占满历史窗口、诱发模型反复道歉。
///
/// - **只读 / 非破坏**：仅变换 `ctx.history_messages`（发给 LLM 的视图），
///   不触碰 DB 与聊天 UI。
/// - 详见 [`fold_repeated_tool_failures`]。
pub(crate) struct ToolFailureFoldStage;

#[async_trait]
impl PipelineStage for ToolFailureFoldStage {
    fn name(&self) -> &'static str {
        "tool_failure_fold"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        let folded = fold_repeated_tool_failures(std::mem::take(&mut ctx.history_messages));
        ctx.history_messages = folded;
        Ok(())
    }
}

// =========================================================================
// Stage 5: FinalAssembleStage — 最终拼装
// =========================================================================

/// Stage 5：最终拼装。
///
/// 1. 拼装 `user_blocks`：可选前置 `rendered_user_prefix`
/// 2. 重排：images 在前，texts 在后（OpenAI Vision 要求）
/// 3. 构造 `messages`：`system`（可选） + `history` + `current user`
///
/// 输入：`ctx.rendered_user_prefix` / `ctx.final_blocks` / `ctx.system_prompt` / `ctx.history_messages`
/// 输出：`ctx.user_blocks` / `ctx.messages`
pub(crate) struct FinalAssembleStage;

#[async_trait]
impl PipelineStage for FinalAssembleStage {
    fn name(&self) -> &'static str {
        "final_assemble"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        // 1) 拼装 user_blocks：可选前置 user_prompt_prefix
        let mut user_blocks: Vec<ContentBlock> = if ctx.rendered_user_prefix.is_empty() {
            ctx.final_blocks.clone()
        } else {
            let mut v = Vec::with_capacity(ctx.final_blocks.len() + 1);
            v.push(ContentBlock::text(ctx.rendered_user_prefix.clone()));
            v.extend(ctx.final_blocks.iter().cloned());
            v
        };

        // 2) 重排：images 在前，texts 在后（OpenAI Vision 要求）
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

        ctx.user_blocks = user_blocks.clone();

        // 3) 构造 messages 列表（system + 历史 + 当前 user）
        let mut messages: Vec<ChatMessage> = Vec::with_capacity(ctx.history_messages.len() + 2);

        if let Some(sys) = &ctx.system_prompt {
            messages.push(ChatMessage::from_text("system", sys.clone()));
        }

        // M1.5: 注入摘要（在 system prompt 之后、history 之前）
        if let Some(summary) = &ctx.summary {
            messages.push(ChatMessage::from_text(
                "system",
                format!("[Previous conversation summary]\n{}", summary),
            ));
        }

        messages.extend(ctx.history_messages.iter().cloned());

        // 当前用户消息（含图片的 content_blocks）
        messages.push(ChatMessage {
            role: "user".into(),
            content: user_blocks,
        });

        ctx.messages = messages;
        Ok(())
    }
}