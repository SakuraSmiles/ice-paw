//! Context 组装 Pipeline — trait-based 架构 (A3-1)
//!
//! 把 `assemble_context` 的 5 步逻辑拆成可插拔的 [`PipelineStage`]：
//!
//! 1. [`crate::context::stages::TemplateStage`]      — 模板查询 + 变量渲染
//! 2. [`crate::context::stages::OsContextStage`]     — OS 运行环境上下文注入
//! 3. [`crate::context::stages::SystemPromptStage`]  — 四级优先 system_prompt 构造
//! 4. [`crate::context::stages::HistoryStage`]       — 历史消息行 → `ChatMessage` 转换
//! 5. [`crate::context::stages::ToolFailureFoldStage`] — 折叠连续重复的失败工具调用
//! 6. [`crate::context::memory::MemoryStage`]        — M1.5 滚动摘要（独立在 memory.rs）
//! 7. [`crate::context::stages::TokenWindowStage`]   — 按 max_input_tokens 裁剪历史（Phase 1）
//! 8. [`crate::context::stages::ModalCapabilityStage`] — 模态能力适配（事2：非视觉 agent 代读/剥离图片）
//! 9. [`crate::context::stages::FinalAssembleStage`] — 最终拼装
//!
//! **M1.4**：移除 `ToolTrimStage` — Pipeline 阶段裁剪工具意义不大，因为
//! 工具裁剪需要每轮动态评估（不同 round 的 query 信号不同），且
//! loop_engine 已经独立通过 `list_tool_defs_with_query()` 打分。
//! 见 `dev2/m1-day3-fix-design.md` P1-3。
//!
//! **M2.2**：将 5 个 Stage 实现从本文件抽出到
//! [`crate::context::stages`]；本文件保留 trait 定义 + Runner +
//! PipelineContext + AssembledContext + 兼容入口 `assemble_context`。
//! `MemoryStage` 按 B04 边界规则继续保留在 [`crate::context::memory`]。
//!
//! 后续新增 Stage（如 A3-3 Token 估算）只需实现
//! [`PipelineStage`] trait，并注册到 [`PipelineRunner`] 即可。
//!
//! [`assemble_context`] 保留为向后兼容的薄壳入口：内部走
//! [`PipelineRunner::default_pipeline`]。新代码应直接构造
//! [`PipelineContext`] + [`PipelineRunner`] 以获得更灵活的控制。

use async_trait::async_trait;
use sqlx::SqlitePool;
use tracing::debug;

use crate::context::memory::{MemoryStage, NoopSummaryProvider, SummaryProvider};
use crate::context::stages::{
    FinalAssembleStage, HistoryStage, ModalCapabilityStage, OsContextStage, SystemPromptStage,
    TemplateStage, TokenWindowStage, ToolFailureFoldStage,
};
use crate::context::token::ContextBudget;
use crate::db::models::{AgentRow, MessageRow};
use crate::error::AppResult;
use crate::infra::cancel::CancellationToken;
use crate::infra::protocol::{ChatMessage, ChatSummaryInjectedPayload, ContentBlock, TemplateInput};

// =========================================================================
// PipelineContext — 贯穿所有 Stage 的可变共享状态
// =========================================================================

/// Pipeline 上下文，贯穿所有 Stage。
///
/// 字段按「调用者输入」与「Stage 输出」分组，方便审计每一步的副作用边界：
/// - **输入段**：由 `PipelineContext::new` 填充，运行期只读
/// - **中间段**：由对应 Stage 写入，供后续 Stage 消费
/// - **输出段**：`messages` / `user_blocks`，运行期只读，由 `assemble_context` 返回
///
/// 后续 A3-3 / A3-4 扩展建议：直接在中间段追加新字段（如
/// `estimated_tokens: Option<usize>` / `summary: Option<String>`），
/// 不需要修改现有 Stage 的签名。
#[allow(clippy::struct_excessive_bools)]
// A3-3 / A3-4 等未来 Stage 会消费 `pool` / `cache_prompt`；现阶段
pub struct PipelineContext {
    // ---- 输入（由调用者填充，运行期只读） ----
    /// 数据库连接池（供需要读 DB 的 Stage 使用，如 TemplateStage）
    pub pool: SqlitePool,
    pub agent: AgentRow,
    pub template_input: Option<TemplateInput>,
    pub history: Vec<MessageRow>,
    pub final_blocks: Vec<ContentBlock>,
    pub tools_enabled: bool,
    /// Agent 端的 prompt caching 开关（透传，未来 A3-3 / A3-4 可消费）
    pub cache_prompt: bool,
    /// M1.2: 当前用户消息纯文本（最初供 ToolTrimStage 做相关性打分；M1.4
    /// 删除 ToolTrimStage 后，该字段由 `loop_engine` 在每轮调用
    /// `list_tool_defs_with_query()` 时消费）。
    /// 不用 `final_blocks` 推导是因为：图片/工具结果块不应进入打分 query。
    pub current_user_query: Option<String>,
    /// M1.2: 最近调用过的工具名称列表（顺序不限；推荐最近 10 条）。
    pub tool_call_history: Vec<String>,
    /// 上下文预算（token 上限 + fold 摘要派生）。工具排序阈值已移至
    /// [`crate::harness::scoring::DEFAULT_TOOL_SORT_THRESHOLD`]，不再由预算承载。
    pub context_budget: ContextBudget,
    /// M1.5: 会话 ID（MemoryStage 写入摘要时需要）
    pub conversation_id: String,
    /// M1.5: 取消令牌（MemoryStage 摘要 LLM 调用时需要）
    pub cancel_token: CancellationToken,
    /// 项目工作目录（None = 散落会话或项目无 workspace）
    pub project_workspace: Option<String>,
    /// 项目上下文目录（IcePaw 管理的 {workspace}/projects/{id}/，存 project.md）
    pub project_context_dir: Option<String>,
    /// 已解析的 agent 明文 API key（DB 只存引用槽位，由 `chat_cmd` 解析后注入）。
    /// 供 [`ModalCapabilityStage`] 收集视觉凭据（`vision::from_agent` 借 agent key 做零配置兜底）。
    pub api_key: Option<String>,

    // ---- Stage 1: Template 渲染输出 ----
    pub rendered_system_prompt: Option<String>,
    pub rendered_user_prefix: String,

    // ---- Stage 2: OS 上下文 ----
    pub os_context: String,

    // ---- Stage 3: System prompt 构造结果 ----
    pub system_prompt: Option<String>,

    // ---- Stage 4: History 转换结果 ----
    pub history_messages: Vec<ChatMessage>,

    // ---- M1.4: Memory Stage 输出 ----
    /// 历史对话的滚动摘要（M1.5 升级后由 [`MemoryStage`] 填充；当前 noop 始终为 None）。
    /// `FinalAssembleStage` 在拼装 messages 时检测 `Some`，在 system 之后、history
    /// 之前插入 `[Previous conversation summary]` 段。
    pub summary: Option<String>,
    /// M1.5: 摘要事件 payload（MemoryStage 触发后填充，由 chat_cmd 读取并 emit）
    pub summary_event: Option<ChatSummaryInjectedPayload>,

    // ---- Stage 5: 最终拼装结果 ----
    pub user_blocks: Vec<ContentBlock>,
    pub messages: Vec<ChatMessage>,
}

impl PipelineContext {
    /// 构造一个新的 `PipelineContext`（输入字段填充，中间 / 输出字段留默认值）
    ///
    /// M1.2: 新增 `current_user_query` / `tool_call_history` / `context_budget` 三个字段；
    /// 调用方（chat_cmd）需要在构造前准备好这些值。
    /// M1.5: 新增 `conversation_id` / `cancel_token` 两个字段（MemoryStage 需要）。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: SqlitePool,
        agent: AgentRow,
        template_input: Option<TemplateInput>,
        history: Vec<MessageRow>,
        final_blocks: Vec<ContentBlock>,
        tools_enabled: bool,
        current_user_query: Option<String>,
        tool_call_history: Vec<String>,
        context_budget: ContextBudget,
        conversation_id: String,
        cancel_token: CancellationToken,
    ) -> Self {
        let cache_prompt = agent.cache_prompt != 0;
        Self {
            pool,
            agent,
            template_input,
            history,
            final_blocks,
            tools_enabled,
            cache_prompt,
            current_user_query,
            tool_call_history,
            context_budget,
            conversation_id,
            cancel_token,
            project_workspace: None,
            project_context_dir: None,
            api_key: None,
            rendered_system_prompt: None,
            rendered_user_prefix: String::new(),
            os_context: String::new(),
            system_prompt: None,
            history_messages: Vec::new(),
            summary: None,
            summary_event: None,
            user_blocks: Vec::new(),
            messages: Vec::new(),
        }
    }
}

// =========================================================================
// PipelineStage trait
// =========================================================================

/// Pipeline 中的一个可插拔处理步骤。
///
/// 实现要求：
/// - `Send + Sync`：Stage 注册到 `Box<dyn PipelineStage>` 后在多 Stage 间共享
/// - `name()`：用于 tracing 日志，必须是 `&'static str`（通常是字面量）
/// - `execute()`：原地修改 `ctx`，可读输入段、写中间段；失败返回 `AppError`
#[async_trait]
pub trait PipelineStage: Send + Sync {
    /// Stage 名称（用于日志）
    fn name(&self) -> &'static str;
    /// 执行该 Stage，修改 pipeline context
    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()>;
}

// =========================================================================
// PipelineRunner
// =========================================================================

/// 按顺序执行一组 Stage。
///
/// `stages` 字段为 `Vec<Box<dyn PipelineStage>>`，
/// 保证每个 Stage 拥有自己的运行时上下文（如 TemplateStage 持有 pool 引用）。
pub struct PipelineRunner {
    stages: Vec<Box<dyn PipelineStage>>,
}

impl PipelineRunner {
    /// 用一组 Stage 构造 Runner
    pub fn new(stages: Vec<Box<dyn PipelineStage>>) -> Self {
        Self { stages }
    }

    /// 默认 Pipeline：Template → OsContext → SystemPrompt → History → ToolFailureFold → Memory → TokenWindow → Final
    ///
    /// 等价于原 `assemble_context` 的 5 步行为 + M1.4 MemoryStage。
    ///
    /// # M1.4 变更
    /// - 移除 ToolTrimStage：工具裁剪需每轮动态评估，loop_engine 已经独立
    ///   通过 `list_tool_defs_with_query()` 打分，Pipeline 阶段裁剪意义不大。
    /// - 同时简化签名：不再需要 `tool_registry` 参数。
    ///
    /// # M1.4 MemoryStage 插入位置
    /// - 在 HistoryStage **之后**、FinalAssembleStage **之前**：这样 MemoryStage
    ///   可以消费 HistoryStage 已经转换好的 `ctx.history_messages`，并把摘要结果
    ///   写入 `ctx.summary`，供 FinalAssembleStage 在拼装 messages 时插入。
    /// - 当前使用 `NoopSummaryProvider`（永远返回空字符串）；M1.5 升级时
    ///   改用 harness 层的 `LlmSummaryProvider`。
    ///
    /// # M2.2 拆分
    /// - 5 个具体 Stage 实现位于 [`crate::context::stages`]；本方法通过
    ///   `use super::stages::*` 引入并装箱。
    pub fn default_pipeline(
        pool: &SqlitePool,
        summary_provider: Option<Box<dyn SummaryProvider>>,
    ) -> Self {
        let provider = summary_provider.unwrap_or_else(|| Box::new(NoopSummaryProvider));
        Self::new(vec![
            Box::new(TemplateStage::new(pool)),
            Box::new(OsContextStage::new(pool)),
            Box::new(SystemPromptStage),
            Box::new(HistoryStage),
            Box::new(ToolFailureFoldStage),
            Box::new(MemoryStage::new(provider)),
            Box::new(TokenWindowStage),
            Box::new(ModalCapabilityStage),
            Box::new(FinalAssembleStage),
        ])
    }

    /// 顺序执行所有 Stage；任一 Stage 失败立即返回错误
    pub async fn run(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        for stage in &self.stages {
            debug!(
                target: "ice_paw.context",
                "Pipeline stage start: {}",
                stage.name()
            );
            stage.execute(ctx).await?;
            debug!(
                target: "ice_paw.context",
                "Pipeline stage done:  {}",
                stage.name()
            );
        }
        Ok(())
    }

    /// 已注册的 Stage 数量（测试 / 调试用）
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.stages.len()
    }
}

/// `PipelineRunner` 产出的上下文组装结果
///
/// - `messages`：可直接喂给 `provider.stream_chat(messages, ...)` 的完整上下文
///   （含 system / 历史 / 当前 user）
/// - `user_blocks`：含图片重排后的当前用户消息 blocks，供 DB 回写
#[derive(Debug)]
pub(crate) struct AssembledContext {
    pub messages: Vec<ChatMessage>,
    pub user_blocks: Vec<ContentBlock>,
}