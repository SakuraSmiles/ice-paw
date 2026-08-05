//! MemoryStage + SummaryProvider + DB 端到端集成测试
//!
//! 覆盖三个核心场景：
//! 1. `test_memory_stage_triggers_summary_when_over_threshold`
//!    —— token 超阈值 → 触发 LLM 摘要 → 写 DB → 设 ctx.summary + summary_event
//! 2. `test_memory_stage_skips_when_under_threshold`
//!    —— token 未超阈值 → noop
//! 3. `test_memory_stage_reuses_existing_summary`
//!    —— DB 已有最近摘要 → 复用 + 截断 history，不再调 provider
//!
//! 运行：`cargo test --test memory_e2e`

use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use ice_paw_lib::context::{ContextBudget, MemoryStage, NoopSummaryProvider, PipelineContext, PipelineStage, SummaryProvider};
use ice_paw_lib::db::models::AgentRow;
use ice_paw_lib::db::repo::summary::{get_latest_summary, insert_summary_message};
use ice_paw_lib::error::AppResult;
use ice_paw_lib::infra::cancel::CancellationToken;
use ice_paw_lib::infra::protocol::{ChatMessage, ContentBlock};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

// =========================================================================
// make_ctx helper —— 复制并适配 src/context/memory.rs::make_test_ctx
// =========================================================================

/// 构造 in-memory SQLite + 跑迁移 + 种子 agent/conversation，返回 PipelineContext。
///
/// 与 `src/context/memory.rs::tests::make_test_ctx` 行为对齐，
/// 但额外支持自定义 `history_messages` / `summary_threshold_tokens`。
async fn make_ctx(
    history_messages: Vec<ChatMessage>,
    summary_threshold: Option<usize>,
) -> PipelineContext {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("valid sqlite url")
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");

    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");

    // seed agent
    sqlx::query(
        "INSERT INTO agents
            (id, name, provider, model, system_prompt, api_key_ref,
             temperature, max_tokens, extra_params, sort_order, cache_prompt)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("a-e")
    .bind("e2e")
    .bind("anthropic")
    .bind("claude")
    .bind("")
    .bind("")
    .bind(0.7)
    .bind(1024)
    .bind("{}")
    .bind(0)
    .bind(0)
    .execute(&pool)
    .await
    .expect("seed agent");

    // seed conversation
    sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES (?, ?, ?)")
        .bind("conv-e")
        .bind("a-e")
        .bind("e2e test")
        .execute(&pool)
        .await
        .expect("seed conversation");

    let agent = AgentRow {
        id: "a-e".into(),
        name: "e2e".into(),
        provider: "anthropic".into(),
        model: "claude-sonnet".into(),
        system_prompt: String::new(),
        api_key_ref: "vault://e2e".into(),
        base_url: None,
        temperature: 0.7,
        max_tokens: 1024,
        extra_params: "{}".into(),
        sort_order: 0,
        cache_prompt: 0,
        max_history_messages: None,
        tool_trim_threshold: None,
        enabled_tools: None,
        avatar: None,
        workspace_path: None,
        description: String::new(),
        supports_vision: 0,
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
    };

    let mut ctx = PipelineContext::new(
        pool,
        agent,
        None,
        vec![],
        vec![ContentBlock::text("hi")],
        false,
        None,
        vec![],
        ContextBudget::default(),
        "conv-e".into(),
        CancellationToken::new(),
    );

    if let Some(threshold) = summary_threshold {
        ctx.context_budget.summary_threshold_tokens = threshold;
    }

    ctx.history_messages = history_messages;
    ctx
}

// =========================================================================
// 测试用 SummaryProvider 实现
// =========================================================================

/// 测试 1 用：返回固定摘要，原子计数调用次数。
struct CountingSummaryProvider {
    summary: String,
    counter: Arc<AtomicUsize>,
}

#[async_trait]
impl SummaryProvider for CountingSummaryProvider {
    async fn summarize(
        &self,
        _messages: &[ChatMessage],
        _max_tokens: usize,
        _cancel: &CancellationToken,
    ) -> AppResult<String> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(self.summary.clone())
    }
}

/// 测试 3 用：永远返回空串，但记录调用次数 —— 用于验证"复用路径不应调用 provider"。
struct CountingNoopProvider {
    counter: Arc<AtomicUsize>,
}

#[async_trait]
impl SummaryProvider for CountingNoopProvider {
    async fn summarize(
        &self,
        _messages: &[ChatMessage],
        _max_tokens: usize,
        _cancel: &CancellationToken,
    ) -> AppResult<String> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(String::new())
    }
}

// =========================================================================
// helpers
// =========================================================================

/// 构造 n 条长 user/assistant 交替消息，每条 content 重复 `char_repeat` 次。
/// 3000 ASCII chars ≈ 750 tokens（估算规则 1 token / 4 chars）。
fn make_long_messages(n: usize, char_repeat: usize) -> Vec<ChatMessage> {
    (0..n)
        .map(|i| {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            ChatMessage::from_text(role, "a".repeat(char_repeat))
        })
        .collect()
}

/// 构造 n 条短 user/assistant 交替消息，每条约 2 tokens。
fn make_short_messages(n: usize) -> Vec<ChatMessage> {
    (0..n)
        .map(|i| {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            ChatMessage::from_text(role, format!("msg-{i}"))
        })
        .collect()
}

// =========================================================================
// 测试 1：token 超阈值 → 触发摘要
// =========================================================================

#[tokio::test]
async fn test_memory_stage_triggers_summary_when_over_threshold() {
    // 40 条 × 3000 ASCII chars ≈ 30_000 tokens，远超 threshold=100
    let history = make_long_messages(40, 3000);

    let mut ctx = make_ctx(history, Some(100)).await;

    let counter = Arc::new(AtomicUsize::new(0));
    let provider = CountingSummaryProvider {
        summary: "测试摘要内容".to_string(),
        counter: counter.clone(),
    };
    let stage = MemoryStage::new(Box::new(provider));

    stage.execute(&mut ctx).await.expect("MemoryStage execute");

    // 1. ctx.summary 被设置
    assert_eq!(
        ctx.summary.as_deref(),
        Some("测试摘要内容"),
        "summary 应被填充为 provider 返回值"
    );

    // 2. history 被截断（40 → < 40）
    assert!(
        ctx.history_messages.len() < 40,
        "history 应被截断：kept={}",
        ctx.history_messages.len()
    );

    // 3. DB 中应能查到摘要
    let stored = get_latest_summary(&ctx.pool, &ctx.conversation_id)
        .await
        .expect("get_latest_summary");
    assert_eq!(
        stored.as_deref(),
        Some("测试摘要内容"),
        "DB 中应存有新摘要"
    );

    // 4. summary_event 被设置
    assert!(
        ctx.summary_event.is_some(),
        "新摘要触发时 summary_event 应被设置"
    );

    // 5. provider 恰好被调一次
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "provider 应被调用恰好一次"
    );
}

// =========================================================================
// 测试 2：token 未超阈值 → 跳过
// =========================================================================

#[tokio::test]
async fn test_memory_stage_skips_when_under_threshold() {
    // 5 条短消息（每条约 2 tokens），远低于默认阈值 35_000
    let history = make_short_messages(5);

    let mut ctx = make_ctx(history, None).await;

    let stage = MemoryStage::new(Box::new(NoopSummaryProvider));
    stage.execute(&mut ctx).await.expect("MemoryStage execute");

    assert!(
        ctx.summary.is_none(),
        "未超阈值时 summary 应保持 None"
    );
    assert_eq!(
        ctx.history_messages.len(),
        5,
        "未超阈值时 history 不应被截断"
    );
    assert!(
        ctx.summary_event.is_none(),
        "未超阈值时 summary_event 应保持 None"
    );
}

// =========================================================================
// 测试 3：复用 DB 中已有摘要
// =========================================================================

#[tokio::test]
async fn test_memory_stage_reuses_existing_summary() {
    // 40 条长消息 + threshold=100 强制进入"摘要"分支
    let history = make_long_messages(40, 3000);

    let mut ctx = make_ctx(history, Some(100)).await;

    // 预先插入一条摘要到 DB（模拟「之前会话已生成过摘要」）
    insert_summary_message(&ctx.pool, &ctx.conversation_id, "旧摘要", 10)
        .await
        .expect("pre-insert summary");

    // 用 CountingNoopProvider 验证 provider 是否被调用
    let counter = Arc::new(AtomicUsize::new(0));
    let provider = CountingNoopProvider {
        counter: counter.clone(),
    };
    let stage = MemoryStage::new(Box::new(provider));

    stage.execute(&mut ctx).await.expect("MemoryStage execute");

    // 1. ctx.summary 应复用旧摘要
    assert_eq!(
        ctx.summary.as_deref(),
        Some("旧摘要"),
        "应复用 DB 中的旧摘要"
    );

    // 2. history 也应被截断（P1-1 修复：复用路径也截断 history，打破 token 死循环）
    assert!(
        ctx.history_messages.len() < 40,
        "复用摘要时 history 也应被截断：kept={}",
        ctx.history_messages.len()
    );

    // 3. provider 不应被调用
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "复用摘要时 provider 不应被调用"
    );

    // 4. summary_event 不应被设置（复用不发 toast）
    assert!(
        ctx.summary_event.is_none(),
        "复用摘要时 summary_event 应保持 None"
    );
}