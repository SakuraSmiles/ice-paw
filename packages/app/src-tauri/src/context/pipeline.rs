//! Context 组装 Pipeline — trait-based 架构 (A3-1)
//!
//! 把 `assemble_context` 的 5 步逻辑拆成可插拔的 [`PipelineStage`]：
//!
//! 1. [`TemplateStage`]      — 模板查询 + 变量渲染（注入 rendered_system_prompt / rendered_user_prefix）
//! 2. [`OsContextStage`]     — OS 运行环境上下文注入
//! 3. [`SystemPromptStage`]  — 四级优先 system_prompt 构造（template > agent > tool_hint > os）
//! 4. [`HistoryStage`]       — 历史消息行 → `ChatMessage` 转换
//! 5. [`FinalAssembleStage`] — 最终拼装（图片重排、user_blocks 拼装、messages 列表组装）
//!
//! 后续新增 Stage（如 A3-3 Token 估算、A3-4 滚动摘要）只需实现
//! [`PipelineStage`] trait，并注册到 [`PipelineRunner`] 即可。
//!
//! [`assemble_context`] 保留为向后兼容的薄壳入口：内部走
//! [`PipelineRunner::default_pipeline`]。新代码应直接构造
//! [`PipelineContext`] + [`PipelineRunner`] 以获得更灵活的控制。

use async_trait::async_trait;
use sqlx::SqlitePool;
use tracing::debug;

use crate::context::history::{load_history_with_window, resolve_window};
use crate::context::os_context::build_os_context;
use crate::context::system_prompt::build_system_prompt;
use crate::context::template::render_template;
use crate::db::models::{AgentRow, MessageRow};
use crate::db::repo;
use crate::error::AppResult;
use crate::infra::protocol::{ChatMessage, ContentBlock, TemplateInput};

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
// 保留为 pub 但通过测试侧访问，故允许 dead_code 提示。
#[allow(dead_code)]
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

    // ---- Stage 1: Template 渲染输出 ----
    pub rendered_system_prompt: Option<String>,
    pub rendered_user_prefix: String,

    // ---- Stage 2: OS 上下文 ----
    pub os_context: String,

    // ---- Stage 3: System prompt 构造结果 ----
    pub system_prompt: Option<String>,

    // ---- Stage 4: History 转换结果 ----
    pub history_messages: Vec<ChatMessage>,

    // ---- Stage 5: 最终拼装结果 ----
    pub user_blocks: Vec<ContentBlock>,
    pub messages: Vec<ChatMessage>,
}

impl PipelineContext {
    /// 构造一个新的 `PipelineContext`（输入字段填充，中间 / 输出字段留默认值）
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: SqlitePool,
        agent: AgentRow,
        template_input: Option<TemplateInput>,
        history: Vec<MessageRow>,
        final_blocks: Vec<ContentBlock>,
        tools_enabled: bool,
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
            rendered_system_prompt: None,
            rendered_user_prefix: String::new(),
            os_context: String::new(),
            system_prompt: None,
            history_messages: Vec::new(),
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

    /// 默认 Pipeline：Template → OsContext → SystemPrompt → History → Final
    ///
    /// 等价于原 `assemble_context` 的 5 步行为。
    pub fn default_pipeline(pool: &SqlitePool) -> Self {
        Self::new(vec![
            Box::new(TemplateStage::new(pool)),
            Box::new(OsContextStage),
            Box::new(SystemPromptStage),
            Box::new(HistoryStage),
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
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.stages.len()
    }

    /// 是否为空 Stage 列表
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
}

// =========================================================================
// AssembledContext + assemble_context（向后兼容入口）
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

/// 组装 LLM 调用上下文（向后兼容入口，内部走 `PipelineRunner`）
///
/// # Pipeline 流程
///
/// 1. **模板查询 + 渲染**（[`TemplateStage`]，副作用：只读 SELECT）
/// 2. **OS 上下文构建**（[`OsContextStage`]）
/// 3. **system prompt 构造**（[`SystemPromptStage`]，四级优先）
/// 4. **历史消息转换**（[`HistoryStage`]）
/// 5. **最终拼装**（[`FinalAssembleStage`]，含图片重排 + content blocks 组装）
#[allow(dead_code)]
pub(crate) async fn assemble_context(
    pool: &SqlitePool,
    agent: &AgentRow,
    template_input: Option<&TemplateInput>,
    history: &[MessageRow],
    final_blocks: Vec<ContentBlock>,
    tools_enabled: bool,
) -> AppResult<AssembledContext> {
    let mut ctx = PipelineContext::new(
        pool.clone(),
        agent.clone(),
        template_input.cloned(),
        history.to_vec(),
        final_blocks,
        tools_enabled,
    );
    PipelineRunner::default_pipeline(pool).run(&mut ctx).await?;
    Ok(AssembledContext {
        messages: ctx.messages,
        user_blocks: ctx.user_blocks,
    })
}

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
pub struct TemplateStage {
    pool: SqlitePool,
}

impl TemplateStage {
    pub fn new(pool: &SqlitePool) -> Self {
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
pub struct OsContextStage;

#[async_trait]
impl PipelineStage for OsContextStage {
    fn name(&self) -> &'static str {
        "os_context"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        ctx.os_context = build_os_context();
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
pub struct SystemPromptStage;

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
pub struct HistoryStage;

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
pub struct FinalAssembleStage;

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

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{AgentRow, NewTemplate, TemplateVariable};
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    // ---- 公共测试夹具 ----

    /// 内存 SQLite + migrations，与 `tests/template_repo_test.rs` 一致
    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite")
    }

    fn make_agent() -> AgentRow {
        AgentRow {
            id: "agent-1".into(),
            name: "test-agent".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet".into(),
            system_prompt: "你是助手".into(),
            api_key_ref: "vault://test".into(),
            base_url: None,
            temperature: 0.7,
            max_tokens: 1024,
            extra_params: "{}".into(),
            sort_order: 0,
            cache_prompt: 0,
            max_history_messages: None, // A3-2: None → 使用系统默认
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn make_msg_row(role: &str, content: &str) -> MessageRow {
        MessageRow {
            id: format!("msg-{}", role),
            conversation_id: "conv-1".into(),
            role: role.into(),
            content: content.into(),
            content_blocks: "[]".into(),
            token_count: None,
            error: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            rowid: 0,
        }
    }

    /// 构造一个填好所有输入字段、`Stage 0` 之前字段保持默认的 PipelineContext
    fn make_ctx(
        pool: SqlitePool,
        agent: AgentRow,
        template_input: Option<TemplateInput>,
        history: Vec<MessageRow>,
        final_blocks: Vec<ContentBlock>,
        tools_enabled: bool,
    ) -> PipelineContext {
        PipelineContext::new(pool, agent, template_input, history, final_blocks, tools_enabled)
    }

    // ---- TemplateStage ----

    #[tokio::test]
    async fn template_stage_renders_known_variables() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();

        // 建一个带变量的模板
        let tpl = repo::template::create(
            &pool,
            &NewTemplate {
                name: "review".into(),
                description: "".into(),
                system_prompt: "你是一位 {{lang}} 专家".into(),
                user_prompt_prefix: "请评审以下 {{lang}} 代码：".into(),
                variables: Some(vec![TemplateVariable {
                    name: "lang".into(),
                    label: "语言".into(),
                    var_type: "text".into(),
                    default: None,
                    options: None,
                }]),
                tools: None,
                sort_order: 0,
            },
            "tpl-1",
        )
        .await
        .unwrap();

        let mut values = std::collections::HashMap::new();
        values.insert("lang".into(), "Rust".into());

        let mut ctx = make_ctx(
            pool.clone(),
            make_agent(),
            Some(TemplateInput {
                template_id: tpl.id.clone(),
                values: values.clone(),
            }),
            vec![],
            vec![ContentBlock::text("fn main() {}")],
            false,
        );
        let stage = TemplateStage::new(&pool);
        stage.execute(&mut ctx).await.unwrap();

        assert_eq!(ctx.rendered_system_prompt.as_deref(), Some("你是一位 Rust 专家"));
        assert_eq!(ctx.rendered_user_prefix, "请评审以下 Rust 代码：");
    }

    #[tokio::test]
    async fn template_stage_missing_template_id_is_noop() {
        // 当 template_input 为 None 时，Stage 不应触库、两个输出字段保持空值
        let pool = fresh_pool().await;
        let mut ctx = make_ctx(
            pool.clone(),
            make_agent(),
            None,
            vec![],
            vec![ContentBlock::text("hello")],
            false,
        );
        let stage = TemplateStage::new(&pool);
        stage.execute(&mut ctx).await.unwrap();

        assert!(ctx.rendered_system_prompt.is_none());
        assert!(ctx.rendered_user_prefix.is_empty());
    }

    // ---- SystemPromptStage ----

    #[tokio::test]
    async fn system_prompt_stage_template_overrides_agent_and_appends_os() {
        // 验证四级优先中：template > agent，且 os 始终注入
        let pool = fresh_pool().await;
        let mut ctx = make_ctx(
            pool,
            make_agent(),
            None,
            vec![],
            vec![ContentBlock::text("hi")],
            false,
        );
        ctx.rendered_system_prompt = Some("模板 prompt".into());
        ctx.os_context = "OS: Linux".into();
        // 保留 agent.system_prompt = "你是助手" 验证不会覆盖 template

        SystemPromptStage.execute(&mut ctx).await.unwrap();
        let s = ctx.system_prompt.unwrap();
        assert!(s.starts_with("模板 prompt"), "template 应优先: {s}");
        assert!(s.contains("OS: Linux"), "os 上下文应被注入: {s}");
        assert!(!s.contains("你是助手"), "agent prompt 不应被注入: {s}");
    }

    #[tokio::test]
    async fn system_prompt_stage_falls_back_to_agent_when_no_template() {
        let pool = fresh_pool().await;
        let mut ctx = make_ctx(
            pool,
            make_agent(),
            None,
            vec![],
            vec![ContentBlock::text("hi")],
            false,
        );
        ctx.os_context = "OS: Linux".into();
        // rendered_system_prompt 保持 None → 应回退到 agent.system_prompt = "你是助手"

        SystemPromptStage.execute(&mut ctx).await.unwrap();
        let s = ctx.system_prompt.unwrap();
        assert!(s.contains("你是助手"), "应回退到 agent: {s}");
        assert!(s.contains("OS: Linux"), "os 应注入: {s}");
    }

    #[tokio::test]
    async fn system_prompt_stage_tools_enabled_appends_hint() {
        let pool = fresh_pool().await;
        let mut ctx = make_ctx(
            pool,
            make_agent(),
            None,
            vec![],
            vec![ContentBlock::text("hi")],
            true, // tools_enabled = true
        );
        ctx.os_context = String::new();

        SystemPromptStage.execute(&mut ctx).await.unwrap();
        let s = ctx.system_prompt.unwrap();
        assert!(s.contains("工具调用能力"), "应追加工具提示: {s}");
        assert!(s.contains("你是助手"), "agent prompt 仍应作为基础: {s}");
    }

    // ---- HistoryStage ----

    #[tokio::test]
    async fn history_stage_converts_rows_and_skips_tool_role() {
        let pool = fresh_pool().await;
        let history = vec![
            make_msg_row("user", "hello"),
            make_msg_row("assistant", "hi"),
            make_msg_row("tool", "should-skip"),
            make_msg_row("system", "sys-msg"),
        ];
        let mut ctx = make_ctx(
            pool,
            make_agent(),
            None,
            history,
            vec![],
            false,
        );

        HistoryStage.execute(&mut ctx).await.unwrap();
        assert_eq!(ctx.history_messages.len(), 3, "tool role 应被跳过");
        assert_eq!(ctx.history_messages[0].role, "user");
        assert_eq!(ctx.history_messages[0].content_text(), "hello");
        assert_eq!(ctx.history_messages[1].role, "assistant");
        assert_eq!(ctx.history_messages[2].role, "system");
    }

    #[tokio::test]
    async fn history_stage_empty_input_yields_empty_output() {
        let pool = fresh_pool().await;
        let mut ctx = make_ctx(pool, make_agent(), None, vec![], vec![], false);
        HistoryStage.execute(&mut ctx).await.unwrap();
        assert!(ctx.history_messages.is_empty());
    }

    // ---- A3-2: HistoryStage 接入 Agent.max_history_messages ----

    /// 构造指定窗口大小的 Agent（仅用于测试）
    fn make_agent_with_window(window: Option<i32>) -> AgentRow {
        let mut a = make_agent();
        a.max_history_messages = window;
        a
    }

    /// A3-2 测试夹具：构造 N 条「交替 user/assistant」历史，
    /// role 都合法（不会因 tool 角色被过滤）
    fn make_history_n(n: usize) -> Vec<MessageRow> {
        (0..n)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                make_msg_row(role, &format!("msg-{i}"))
            })
            .collect()
    }

    #[tokio::test]
    async fn history_stage_uses_agent_window_when_set() {
        // Agent 配置 N=3，仅保留最近 3 条；超出部分被裁剪
        let pool = fresh_pool().await;
        let history = make_history_n(5);
        let mut ctx = make_ctx(
            pool,
            make_agent_with_window(Some(3)),
            None,
            history,
            vec![],
            false,
        );

        HistoryStage.execute(&mut ctx).await.unwrap();
        assert_eq!(ctx.history_messages.len(), 3);
        assert_eq!(ctx.history_messages[0].content_text(), "msg-2");
        assert_eq!(ctx.history_messages[1].content_text(), "msg-3");
        assert_eq!(ctx.history_messages[2].content_text(), "msg-4");
    }

    #[tokio::test]
    async fn history_stage_falls_back_to_default_when_agent_window_none() {
        // Agent 配置 None → 系统默认 DEFAULT_HISTORY_WINDOW（20）
        // 输入 25 条 → 期望保留最后 20 条
        let pool = fresh_pool().await;
        let history = make_history_n(25);
        let mut ctx = make_ctx(
            pool,
            make_agent_with_window(None),
            None,
            history,
            vec![],
            false,
        );

        HistoryStage.execute(&mut ctx).await.unwrap();
        // 25 - 20 = 5 条被裁掉，剩 20 条 msg-5..msg-24
        assert_eq!(ctx.history_messages.len(), 20);
        assert_eq!(ctx.history_messages[0].content_text(), "msg-5");
        assert_eq!(ctx.history_messages[19].content_text(), "msg-24");
    }

    #[tokio::test]
    async fn history_stage_falls_back_to_default_when_agent_window_invalid() {
        // 非法值（0/负数）→ 系统默认
        let pool = fresh_pool().await;
        let history = make_history_n(25);

        for bad_window in [Some(0), Some(-1)] {
            let mut ctx = make_ctx(
                pool.clone(),
                make_agent_with_window(bad_window),
                None,
                history.clone(),
                vec![],
                false,
            );
            HistoryStage.execute(&mut ctx).await.unwrap();
            assert_eq!(
                ctx.history_messages.len(),
                20,
                "非法窗口 {bad_window:?} 应回退默认 20"
            );
        }
    }

    #[tokio::test]
    async fn history_stage_window_larger_than_input_keeps_all() {
        // window > history.len() → 全部保留（不补齐）
        let pool = fresh_pool().await;
        let history = make_history_n(3);
        let mut ctx = make_ctx(
            pool,
            make_agent_with_window(Some(100)),
            None,
            history,
            vec![],
            false,
        );

        HistoryStage.execute(&mut ctx).await.unwrap();
        assert_eq!(ctx.history_messages.len(), 3);
    }

    #[tokio::test]
    async fn history_stage_window_one_keeps_only_last() {
        // 极端场景：window=1 → 仅保留最新一条
        let pool = fresh_pool().await;
        let history = vec![
            make_msg_row("user", "old"),
            make_msg_row("assistant", "middle"),
            make_msg_row("user", "newest"),
        ];
        let mut ctx = make_ctx(
            pool,
            make_agent_with_window(Some(1)),
            None,
            history,
            vec![],
            false,
        );

        HistoryStage.execute(&mut ctx).await.unwrap();
        assert_eq!(ctx.history_messages.len(), 1);
        assert_eq!(ctx.history_messages[0].content_text(), "newest");
    }

    // ---- OsContextStage ----

    #[tokio::test]
    async fn os_context_stage_populates_context() {
        let pool = fresh_pool().await;
        let mut ctx = make_ctx(
            pool,
            make_agent(),
            None,
            vec![],
            vec![ContentBlock::text("hi")],
            false,
        );
        OsContextStage.execute(&mut ctx).await.unwrap();
        assert!(ctx.os_context.contains("操作系统"));
        assert!(ctx.os_context.contains("架构"));
    }

    // ---- FinalAssembleStage ----

    #[tokio::test]
    async fn final_assemble_stage_reorders_images_before_texts() {
        let pool = fresh_pool().await;
        let mut ctx = make_ctx(
            pool,
            make_agent(),
            None,
            vec![],
            vec![
                ContentBlock::text("first text"),
                ContentBlock::image("data", "image/png"),
                ContentBlock::text("second text"),
                ContentBlock::image("data2", "image/jpeg"),
            ],
            false,
        );
        ctx.history_messages = vec![ChatMessage::from_text("user", "history")];
        ctx.system_prompt = Some("sys".into());

        FinalAssembleStage.execute(&mut ctx).await.unwrap();

        // user_blocks: 2 images + 2 texts
        assert_eq!(ctx.user_blocks.len(), 4);
        assert!(ctx.user_blocks[0].is_image());
        assert!(ctx.user_blocks[1].is_image());
        assert!(!ctx.user_blocks[2].is_image());
        assert!(!ctx.user_blocks[3].is_image());

        // messages: system + history + user
        assert_eq!(ctx.messages.len(), 3);
        assert_eq!(ctx.messages[0].role, "system");
        assert_eq!(ctx.messages[0].content_text(), "sys");
        assert_eq!(ctx.messages[1].role, "user");
        assert_eq!(ctx.messages[1].content_text(), "history");
        assert_eq!(ctx.messages[2].role, "user");
    }

    #[tokio::test]
    async fn final_assemble_stage_prepends_rendered_user_prefix() {
        let pool = fresh_pool().await;
        let mut ctx = make_ctx(
            pool,
            make_agent(),
            None,
            vec![],
            vec![ContentBlock::text("user content")],
            false,
        );
        ctx.rendered_user_prefix = "请评审：".into();
        ctx.history_messages = vec![];
        ctx.system_prompt = None;

        FinalAssembleStage.execute(&mut ctx).await.unwrap();

        // user_blocks: prefix + user content
        assert_eq!(ctx.user_blocks.len(), 2);
        assert_eq!(ctx.user_blocks[0].as_text(), Some("请评审："));
        assert_eq!(ctx.user_blocks[1].as_text(), Some("user content"));

        // messages: 只有 user（无 system，无 history）
        assert_eq!(ctx.messages.len(), 1);
        assert_eq!(ctx.messages[0].role, "user");
    }

    // ---- PipelineRunner ----

    #[tokio::test]
    async fn pipeline_runner_executes_stages_in_order() {
        // 用一个 mini Runner 验证 Stage 顺序执行：每个 Stage 写入一个
        // 唯一的 marker 到 ctx.messages 末尾，最后用 messages 顺序回放。
        struct MarkerStage {
            name: &'static str,
            marker: &'static str,
        }
        #[async_trait]
        impl PipelineStage for MarkerStage {
            fn name(&self) -> &'static str {
                self.name
            }
            async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
                ctx.messages.push(ChatMessage::from_text("system", self.marker));
                Ok(())
            }
        }

        let pool = fresh_pool().await;
        let runner = PipelineRunner::new(vec![
            Box::new(MarkerStage {
                name: "a",
                marker: "first",
            }),
            Box::new(MarkerStage {
                name: "b",
                marker: "second",
            }),
            Box::new(MarkerStage {
                name: "c",
                marker: "third",
            }),
        ]);
        assert_eq!(runner.len(), 3);
        assert!(!runner.is_empty());

        let mut ctx = make_ctx(pool, make_agent(), None, vec![], vec![], false);
        runner.run(&mut ctx).await.unwrap();

        assert_eq!(ctx.messages.len(), 3);
        assert_eq!(ctx.messages[0].content_text(), "first");
        assert_eq!(ctx.messages[1].content_text(), "second");
        assert_eq!(ctx.messages[2].content_text(), "third");
    }

    #[tokio::test]
    async fn pipeline_runner_short_circuits_on_error() {
        struct OkStage;
        #[async_trait]
        impl PipelineStage for OkStage {
            fn name(&self) -> &'static str {
                "ok"
            }
            async fn execute(&self, _ctx: &mut PipelineContext) -> AppResult<()> {
                Ok(())
            }
        }
        struct FailStage;
        #[async_trait]
        impl PipelineStage for FailStage {
            fn name(&self) -> &'static str {
                "fail"
            }
            async fn execute(&self, _ctx: &mut PipelineContext) -> AppResult<()> {
                Err(crate::error::AppError::Validation("intentional".into()).into())
            }
        }
        struct NeverRunsStage {
            flag: std::sync::Arc<std::sync::Mutex<bool>>,
        }
        #[async_trait]
        impl PipelineStage for NeverRunsStage {
            fn name(&self) -> &'static str {
                "never"
            }
            async fn execute(&self, _ctx: &mut PipelineContext) -> AppResult<()> {
                *self.flag.lock().unwrap() = true;
                Ok(())
            }
        }
        let flag = std::sync::Arc::new(std::sync::Mutex::new(false));
        let runner = PipelineRunner::new(vec![
            Box::new(OkStage),
            Box::new(FailStage),
            Box::new(NeverRunsStage { flag: flag.clone() }),
        ]);
        let pool = fresh_pool().await;
        let mut ctx = make_ctx(pool, make_agent(), None, vec![], vec![], false);
        let result = runner.run(&mut ctx).await;
        assert!(result.is_err(), "FailStage 后应返回错误");
        assert!(
            !*flag.lock().unwrap(),
            "FailStage 之后的 Stage 不应被执行"
        );
    }

    #[tokio::test]
    async fn default_pipeline_matches_legacy_assemble_context() {
        // 端到端验证：组装结果与原 assemble_context 行为一致
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();

        // 建一个简单模板
        repo::template::create(
            &pool,
            &NewTemplate {
                name: "t".into(),
                description: "".into(),
                system_prompt: "你是一个 {{role}}".into(),
                user_prompt_prefix: "PFX:".into(),
                variables: Some(vec![TemplateVariable {
                    name: "role".into(),
                    label: "角色".into(),
                    var_type: "text".into(),
                    default: None,
                    options: None,
                }]),
                tools: None,
                sort_order: 0,
            },
            "tpl-1",
        )
        .await
        .unwrap();

        let mut values = std::collections::HashMap::new();
        values.insert("role".into(), "代码评审员".into());

        let history = vec![
            make_msg_row("user", "历史1"),
            make_msg_row("assistant", "历史2"),
        ];
        let final_blocks = vec![
            ContentBlock::text("新消息"),
            ContentBlock::image("img-data", "image/png"),
        ];

        // 跑 PipelineRunner
        let mut ctx = make_ctx(
            pool.clone(),
            make_agent(),
            Some(TemplateInput {
                template_id: "tpl-1".into(),
                values,
            }),
            history,
            final_blocks,
            true, // tools_enabled
        );
        PipelineRunner::default_pipeline(&pool)
            .run(&mut ctx)
            .await
            .unwrap();

        // 验证输出结构：
        // messages = [system(含渲染后模板 + 工具提示 + os), user(hist1), assistant(hist2), user(图片+文本)]
        assert_eq!(ctx.messages.len(), 4);
        assert_eq!(ctx.messages[0].role, "system");
        let sys = ctx.messages[0].content_text();
        assert!(sys.contains("代码评审员"), "模板渲染应生效: {sys}");
        assert!(sys.contains("工具调用能力"), "工具提示应追加: {sys}");
        assert!(sys.contains("操作系统"), "os 上下文应注入: {sys}");

        assert_eq!(ctx.messages[1].role, "user");
        assert_eq!(ctx.messages[1].content_text(), "历史1");
        assert_eq!(ctx.messages[2].role, "assistant");
        assert_eq!(ctx.messages[2].content_text(), "历史2");

        // 当前 user：图片应排在最前
        assert_eq!(ctx.messages[3].role, "user");
        assert!(ctx.messages[3].content[0].is_image());
        // user_blocks 同样顺序
        assert!(ctx.user_blocks[0].is_image());
        // 文本部分：prefix + 新消息
        let text_blocks: Vec<&str> = ctx
            .user_blocks
            .iter()
            .filter_map(|b| b.as_text())
            .collect();
        assert_eq!(text_blocks, vec!["PFX:", "新消息"]);
    }
}
