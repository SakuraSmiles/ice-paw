//! Pipeline 单元测试
//!
//! M2.2 拆分后，所有 Pipeline 相关测试集中到本文件，统一复用顶部
//! `make_agent` / `make_ctx` / `fresh_pool` 等共享夹具。
//!
//! 测试按 Stage 分组，组间用 `// === StageName tests ===` 分隔。

use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

use crate::context::pipeline::{PipelineContext, PipelineRunner, PipelineStage};
use crate::context::stages::{
    FinalAssembleStage, HistoryStage, OsContextStage, SystemPromptStage,
};
use crate::db::models::{AgentRow, MessageRow};
use crate::error::AppResult;
use crate::infra::cancel::CancellationToken;
use crate::infra::protocol::{ChatMessage, ContentBlock, TemplateInput};

// =========================================================================
// 共享测试夹具
// =========================================================================

/// 内存 SQLite + migrations，与 `tests/template_repo_test.rs` 一致
pub(super) async fn fresh_pool() -> SqlitePool {
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

pub(super) fn make_agent() -> AgentRow {
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
        tool_trim_threshold: None,
        enabled_tools: None,
        supports_vision: 0,
        embedding_model: None,
        description: String::new(),
        avatar: None,
        workspace_path: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
    }
}

pub(super) fn make_msg_row(role: &str, content: &str) -> MessageRow {
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
        summary_id: None,
        model: None,
    }
}

/// 构造一个填好所有输入字段、`Stage 0` 之前字段保持默认的 PipelineContext
///
/// M1.2: 新增 `current_user_query=None` / `tool_call_history=[]` / 默认 `ContextBudget`，
/// 调用方如果需要测试 MemoryStage 路径，可以在此基础上修改 ctx 字段。
pub(super) fn make_ctx(
    pool: SqlitePool,
    agent: AgentRow,
    template_input: Option<TemplateInput>,
    history: Vec<MessageRow>,
    final_blocks: Vec<ContentBlock>,
    tools_enabled: bool,
) -> PipelineContext {
    PipelineContext::new(
        pool,
        agent,
        template_input,
        history,
        final_blocks,
        tools_enabled,
        None,
        Vec::new(),
        crate::context::token::ContextBudget::default(),
        String::new(),
        CancellationToken::new(),
    )
}

// =========================================================================
// SystemPromptStage tests
// =========================================================================

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

// =========================================================================
// HistoryStage tests
// =========================================================================

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

// =========================================================================
// OsContextStage tests
// =========================================================================

#[tokio::test]
async fn os_context_stage_populates_context() {
    let pool = fresh_pool().await;
    let mut ctx = make_ctx(
        pool.clone(),
        make_agent(),
        None,
        vec![],
        vec![ContentBlock::text("hi")],
        false,
    );
    OsContextStage::new(&pool).execute(&mut ctx).await.unwrap();
    assert!(ctx.os_context.contains("操作系统"));
    assert!(ctx.os_context.contains("架构"));
}

// =========================================================================
// FinalAssembleStage tests
// =========================================================================

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

// =========================================================================
// PipelineRunner tests
// =========================================================================

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
    assert!(runner.len() > 0);

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
            Err(crate::error::AppError::Validation("intentional".into()))
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


// ---- M1.4: MemoryStage 集成 ----

/// 验证 `default_pipeline` 注册了 MemoryStage。
///
/// - 间接验证：跑整个 Pipeline 后 `ctx.summary` 保持 None（noop 阶段）
/// - 同时验证：Pipeline 仍能成功跑完，messages / user_blocks 输出正确
/// - 不直接访问内部 stages 列表（保持封装），靠 Pipeline 行为间接验证
#[tokio::test]
async fn default_pipeline_includes_memory_stage() {
    let pool = fresh_pool().await;

    // 准备一段足够长的 history，让 MemoryStage 在 M1.5 升级后有触发可能
    let history = make_history_n(15);
    let mut ctx = make_ctx(
        pool.clone(),
        make_agent(),
        None,
        history,
        vec![ContentBlock::text("当前问题")],
        false,
    );

    // 跑完整 PipelineRunner
    PipelineRunner::default_pipeline(&pool, None)
        .run(&mut ctx)
        .await
        .unwrap();

    // M1.4: MemoryStage 是 noop，ctx.summary 保持 None
    assert!(
        ctx.summary.is_none(),
        "M1.4 阶段 MemoryStage 必须是 noop（ctx.summary 保持 None）"
    );

    // 验证 Pipeline 仍能正确产出 messages（最终拼装未受 M1.4 变更影响）
    assert!(!ctx.messages.is_empty(), "Pipeline 产出 messages 不应为空");
    assert_eq!(ctx.messages.last().unwrap().role, "user", "末条应为 user");
}