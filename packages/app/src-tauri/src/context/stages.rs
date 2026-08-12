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
//! 7. [`TokenWindowStage`]   — 按 max_input_tokens 硬上限裁剪历史（Phase 1 token 窗口）
//! 8. [`ModalCapabilityStage`] — 模态能力适配（事2：非视觉 agent 代读/剥离图片块）
//! 9. [`FinalAssembleStage`] — 最终拼装（图片重排、user_blocks 拼装、messages 列表组装）
//!
//! `MemoryStage` 不在本模块 —— 它属于 L1 Memory 层（`memory.rs`），按
//! M2.2 拆分规则独立维护。

use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::context::history::{
    fold_repeated_tool_failures, load_history_with_window, sanitize_history,
};
use crate::context::os_context::build_os_context;
use crate::context::pipeline::{PipelineContext, PipelineStage};
use crate::context::system_prompt::build_system_prompt;
use crate::context::template::render_template;
use crate::context::token::{estimate_block_tokens, estimate_tokens, trim_history_to_budget};
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

/// Stage 4：历史消息行 → `ChatMessage` 转换（全量，不在本阶段裁剪）。
///
/// 输入：`ctx.history`（`Vec<MessageRow>`，按时间正序；DB 侧已由
/// `MEMORY_LOAD_LIMIT` 限量加载，见 `chat_cmd.rs`）
///
/// 输出：`ctx.history_messages`（`Vec<ChatMessage>`，含每条的 `source_rowid`；
/// 摘要行已被 [`load_history_with_window`] 过滤——双注入修复）
///
/// Phase 2 变更：不再在此 count-window。`max_history_messages` 语义重定义为
/// MemoryStage 的 **keep_n 地板**（verbatim 保留窗），不再决定「加载/发送上限」。
/// 裁剪职责：摘要压缩由 [`MemoryStage`]，token 硬上限由 [`TokenWindowStage`]。
pub(crate) struct HistoryStage;

#[async_trait]
impl PipelineStage for HistoryStage {
    fn name(&self) -> &'static str {
        "history"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        ctx.history_messages = load_history_with_window(&ctx.history, None);
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
// Stage 4.7: TokenWindowStage — 按 max_input_tokens 硬上限裁剪历史
// =========================================================================

/// Phase 1: token 窗口守门人（消费 Phase 0 接好的 `max_input_tokens`）。
///
/// 在 [`crate::context::memory::MemoryStage`]（软摘要压缩）之后、
/// [`FinalAssembleStage`] 之前执行：估算 system + summary + 当前 user + 历史 的总 token，
/// 若超过 `context_budget.max_input_tokens` 的 [`TOKEN_WINDOW_TARGET_PCT`]%（保守余量），
/// 从历史最旧端裁剪到预算内，再 [`sanitize_history`] 修复切断处的孤儿 tool 块。
///
/// system / summary / 当前 user 不裁（必须保留）；只动 `ctx.history_messages`。
/// 这是最后的安全网：大窗口模型（如 MiniMax-M3 1M）几乎不触发，小窗口模型才生效。
pub(crate) struct TokenWindowStage;

/// Token 窗口目标比例：估算总 token 超过 `max_input` 的此百分比即裁剪历史。
///
/// 取 **80%**（留 20% 余量）而非更激进的比例，原因：
/// - **工具定义不计入估算**：本 Stage 只估 system/summary/user/历史，工具 JSON schema
///   （native 文件工具 + MCP）完全没数，工具密集 agent 可达数 K token；
/// - **估算器偏保守**：`estimate_tokens`（CJK 1/字、其余 ÷4）对 JSON / 代码 / 标点密集
///   内容会**低估**，而那正是 tool_input 高发场景——越该裁剪时低估越严重；
/// - **风险不对称**：超限 = API 硬报错整轮失败；多裁 = 损失部分旧历史（可接受）；
/// - 本 Stage 极少触发（仅长对话接近上限），保守代价极小。
///
/// 若手测出现 context-length 报错，调低此值；若发现大量历史被过早裁掉，可调高。
const TOKEN_WINDOW_TARGET_PCT: usize = 80;

#[async_trait]
impl PipelineStage for TokenWindowStage {
    fn name(&self) -> &'static str {
        "token_window"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        let max_input = ctx.context_budget.max_input_tokens;
        if max_input == 0 || ctx.history_messages.is_empty() {
            return Ok(());
        }
        // 目标 ≤ max_input 的 TOKEN_WINDOW_TARGET_PCT%（余量给工具定义 / 估算误差 / 端点差异）
        let target = max_input * TOKEN_WINDOW_TARGET_PCT / 100;

        // 不可裁剪部分（必须保留）：system prompt + 摘要 + 当前用户消息
        let sys_tokens = ctx.system_prompt.as_deref().map(estimate_tokens).unwrap_or(0);
        let summary_tokens = ctx.summary.as_deref().map(estimate_tokens).unwrap_or(0);
        let user_tokens: usize = ctx.final_blocks.iter().map(estimate_block_tokens).sum();
        let fixed = sys_tokens + summary_tokens + user_tokens;

        // 分配给历史的预算 = 目标 - 不可裁剪部分
        let history_budget = target.saturating_sub(fixed);
        let before_n = ctx.history_messages.len();

        let kept = trim_history_to_budget(&ctx.history_messages, history_budget);
        if kept.len() < before_n {
            let dropped = before_n - kept.len();
            // 裁剪可能在 tool_use/tool_result 边界切断 → sanitize 清理孤儿
            ctx.history_messages = sanitize_history(kept.to_vec());
            tracing::info!(
                target: "ice_paw.context",
                before_n,
                after_n = ctx.history_messages.len(),
                max_input,
                target,
                "TokenWindowStage: 超 max_input 裁剪历史 {dropped} 条",
            );
        }
        Ok(())
    }
}

// =========================================================================
// Stage 4.8: ModalCapabilityStage — 模态能力适配（事2 / 方案 C）
// =========================================================================

/// Stage 4.8：按 agent「有效视觉能力」适配图片块，位置在 TokenWindow 之后、FinalAssemble 之前。
///
/// 4 个图片入口中本 Stage 覆盖 2 个：
/// - **门① 当前用户消息**（`ctx.final_blocks`）：非视觉 agent → 收集视觉凭据逐图代读（OCR）成
///   `Text`，代读不了的剥离 + 诚实提示；视觉 agent → 原样过（事1 元提示已在 `chat_cmd` 注入）。
/// - **门③ 历史**（`ctx.history_messages`）：非视觉 agent → 每条消息的 `Image` 剥成一条 marker
///   （不重复 OCR，避免 N×M 次 视觉调用 + 重复提示噪声）；视觉 agent → 原样过。
///
/// 另两个入口由别处接同一套适配函数：门② 工具返图（`tool_executor`）、门④ `view_attachment_image`
/// 工具（其判断改 `effective_supports_vision`）。
///
/// 代读网络错误被 [`crate::harness::modal::adapt_blocks_for_vision`] 内部吸收，**绝不向上抛**
/// （视觉适配失败不中断主对话）；凭据收集的 DB 查询失败仅降级到能拿到的凭据。
pub(crate) struct ModalCapabilityStage;

#[async_trait]
impl PipelineStage for ModalCapabilityStage {
    fn name(&self) -> &'static str {
        "modal_capability"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        let eff_vision = crate::harness::provider::effective_supports_vision(
            ctx.agent.supports_vision,
            &ctx.agent.provider,
            &ctx.agent.model,
        );
        // 视觉模型：final_blocks 原样（含事1 元提示 + 图片），历史图片也保留 → 直送模型。
        if eff_vision {
            return Ok(());
        }

        // 非视觉：收集凭据（显式 vision 配置 → agent 自带视觉模型 → GLM 视觉 MCP env）。
        let candidates = crate::harness::modal::gather_vision_candidates(
            &ctx.pool,
            &ctx.agent,
            ctx.api_key.as_deref(),
        )
        .await;

        // 门① 当前用户消息：完整适配（OCR 成功→Text；失败/无凭据→剥离+诚实提示）。
        let outcome =
            crate::harness::modal::adapt_blocks_for_vision(&ctx.final_blocks, false, &candidates)
                .await;
        if outcome.changed() {
            tracing::info!(
                target: "ice_paw.modal",
                ocr_replaced = outcome.ocr_replaced,
                dropped = outcome.dropped,
                agent_model = %ctx.agent.model,
                "非视觉 agent 当前消息图片已适配（代读/剥离）"
            );
            ctx.final_blocks = outcome.blocks;
        }

        // 门③ 历史：每条消息的图片剥成 marker（避免每轮重复 OCR；诚实告知曾含图）。
        let mut history_touched = 0u32;
        for msg in &mut ctx.history_messages {
            if msg.content.iter().any(|b| b.is_image()) {
                let original = std::mem::take(&mut msg.content);
                msg.content = crate::harness::modal::strip_image_blocks_to_marker(&original);
                history_touched += 1;
            }
        }
        if history_touched > 0 {
            tracing::info!(
                target: "ice_paw.modal",
                messages = history_touched,
                agent_model = %ctx.agent.model,
                "非视觉 agent 历史消息图片已剥为 marker"
            );
        }
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
            source_rowid: None,
        });

        ctx.messages = messages;
        Ok(())
    }
}