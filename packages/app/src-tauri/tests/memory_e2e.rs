//! MemoryStage + SummaryProvider + DB 端到端集成测试（Phase 2 滚动折叠）
//!
//! 覆盖三个核心场景：
//! 1. `test_memory_stage_folds_and_persists_when_over_trigger`
//!    —— verbatim 后缀超触发线 → 折叠 → 写 DB（covered_until_rowid 非空）
//!    → 设 ctx.summary + summary_event，provider 调一次
//! 2. `test_memory_stage_skips_when_under_trigger`
//!    —— verbatim 未超触发线 → noop
//! 3. `test_memory_stage_incremental_fold_advances_coverage`
//!    —— DB 已有摘要且 covered 落在加载切片内 → 增量折叠（非复用），
//!    provider 拿到前序摘要、covered 前进、UPDATE-in-place 保持单例
//!
//! 运行：`cargo test --test memory_e2e`（注：binary 可能因 sodium DLL 无法启动，
//! `cargo check --tests` 至少保编译）

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ice_paw_lib::context::{
    ContextBudget, MemoryStage, NoopSummaryProvider, PipelineContext, PipelineStage,
    SummaryProvider,
};
use ice_paw_lib::db::models::AgentRow;
use ice_paw_lib::db::repo::summary::{
    get_latest_summary_state, insert_summary_message, SUMMARY_PREFIX,
};
use ice_paw_lib::error::AppResult;
use ice_paw_lib::infra::cancel::CancellationToken;
use ice_paw_lib::infra::protocol::{ChatMessage, ContentBlock};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

// =========================================================================
// make_ctx helper
// =========================================================================

/// 构造 in-memory SQLite + 跑迁移 + 种子 agent/conversation，返回 PipelineContext。
///
/// 与 `src/context/memory.rs::tests::make_test_ctx` 行为对齐，
/// 但额外支持自定义 `history_messages` / `max_input_tokens` / `max_history_messages`。
async fn make_ctx(
    history_messages: Vec<ChatMessage>,
    max_input_tokens: usize,
    max_history_messages: Option<i32>,
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
        max_history_messages,
        context_window: None,
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
        ContextBudget { max_input_tokens },
        "conv-e".into(),
        CancellationToken::new(),
    );

    ctx.history_messages = history_messages;
    ctx
}

// =========================================================================
// 测试用 SummaryProvider 实现（记录入参 + 返回固定回复）
// =========================================================================

#[derive(Clone)]
struct RecordingProvider {
    reply: String,
    calls: Arc<Mutex<Vec<Vec<ChatMessage>>>>,
}

impl RecordingProvider {
    fn new(reply: &str) -> Self {
        Self {
            reply: reply.to_string(),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl SummaryProvider for RecordingProvider {
    async fn summarize(
        &self,
        messages: &[ChatMessage],
        _max_tokens: usize,
        _cancel: &CancellationToken,
    ) -> AppResult<String> {
        self.calls.lock().unwrap().push(messages.to_vec());
        Ok(self.reply.clone())
    }
}

// =========================================================================
// helpers
// =========================================================================

fn big_msg(role: &str, text: &str, rowid: i64) -> ChatMessage {
    ChatMessage {
        role: role.into(),
        content: vec![ContentBlock::text(text)],
        source_rowid: Some(rowid),
        source_seq: None,
    }
}

/// n 条长 user/assistant 交替消息，每条 content 重复 `char_repeat` 次，带 source_rowid。
/// 3000 ASCII chars ≈ 750 tokens + 4 overhead = 754 tokens。
fn make_long_messages(n: usize, char_repeat: usize) -> Vec<ChatMessage> {
    (0..n)
        .map(|i| {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            big_msg(role, &"a".repeat(char_repeat), i as i64)
        })
        .collect()
}

/// n 条短消息，带 source_rowid
fn make_short_messages(n: usize) -> Vec<ChatMessage> {
    (0..n)
        .map(|i| {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            big_msg(role, &format!("msg-{i}"), i as i64)
        })
        .collect()
}

// =========================================================================
// 测试 1：verbatim 超触发线 → 折叠 + 落库
// =========================================================================

#[tokio::test]
async fn test_memory_stage_folds_and_persists_when_over_trigger() {
    // 40 条 × ~754 tokens ≈ 30160；max_input=10000 → trigger=5500（超）target=4000
    let history = make_long_messages(40, 3000);
    let mut ctx = make_ctx(history, 10_000, None).await;

    let provider = RecordingProvider::new("测试摘要内容");
    let stage = MemoryStage::new(Box::new(provider.clone()));
    stage.execute(&mut ctx).await.expect("MemoryStage execute");

    // 1. ctx.summary 被设置
    assert_eq!(
        ctx.summary.as_deref(),
        Some("测试摘要内容"),
        "summary 应被填充为 provider 返回值"
    );

    // 2. history 被截断（折掉前缀）
    assert!(
        ctx.history_messages.len() < 40,
        "history 应被截断：kept={}",
        ctx.history_messages.len()
    );

    // 3. DB 中查到摘要，且 covered_until_rowid 非空（Phase 2 新列生效）
    let state = get_latest_summary_state(&ctx.pool, &ctx.conversation_id)
        .await
        .expect("get_latest_summary_state")
        .expect("应有摘要行");
    assert_eq!(state.text, "测试摘要内容");
    assert!(
        state.covered_until_rowid.is_some(),
        "covered_until_rowid 应已落库: {:?}",
        state.covered_until_rowid
    );

    // 4. summary_event 被设置
    assert!(
        ctx.summary_event.is_some(),
        "折叠触发时 summary_event 应被设置"
    );

    // 5. provider 恰好被调一次
    let calls = provider.calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "provider 应被调用恰好一次");
    assert!(
        !calls[0][0].content_text().contains("[Prior summary]"),
        "首次折叠入参不应含前序摘要"
    );
}

// =========================================================================
// 测试 2：verbatim 未超触发线 → 跳过
// =========================================================================

#[tokio::test]
async fn test_memory_stage_skips_when_under_trigger() {
    // 5 条短消息；max_input=100000 → trigger=55000，verbatim 远未达
    let history = make_short_messages(5);
    let mut ctx = make_ctx(history, 100_000, None).await;

    let stage = MemoryStage::new(Box::new(NoopSummaryProvider));
    stage.execute(&mut ctx).await.expect("MemoryStage execute");

    assert!(ctx.summary.is_none(), "未超触发线时 summary 应保持 None");
    assert_eq!(
        ctx.history_messages.len(),
        5,
        "未超触发线时 history 不应被截断"
    );
    assert!(
        ctx.summary_event.is_none(),
        "未超触发线时 summary_event 应保持 None"
    );
}

// =========================================================================
// 测试 3：增量折叠（既有摘要 + covered 在切片内）
// =========================================================================

#[tokio::test]
async fn test_memory_stage_incremental_fold_advances_coverage() {
    // 40 条长消息 rowid 0..39；预置摘要 covered=10（落在切片内）
    let history = make_long_messages(40, 3000);
    let mut ctx = make_ctx(history, 10_000, None).await;

    insert_summary_message(&ctx.pool, &ctx.conversation_id, "旧摘要", None, 10)
        .await
        .expect("pre-insert summary");

    let provider = RecordingProvider::new("新摘要");
    let stage = MemoryStage::new(Box::new(provider.clone()));
    stage.execute(&mut ctx).await.expect("MemoryStage execute");

    // 1. ctx.summary 为新摘要（增量折叠，非复用旧摘要）
    assert_eq!(ctx.summary.as_deref(), Some("新摘要"));

    // 2. provider 被调一次，且首条入参含前序摘要
    {
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "增量折叠应调用 provider 一次");
        assert!(
            calls[0][0].content_text().contains("[Prior summary]"),
            "应把旧摘要作为前序喂入: {}",
            calls[0][0].content_text()
        );
        assert!(calls[0][0].content_text().contains("旧摘要"));
    } // guard 在下方 DB await 前 drop，避免 await_holding_lock

    // 3. DB：摘要行单例（UPDATE-in-place），covered 前进（> 10）
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE role='system' AND instr(content, ?) = 1",
    )
    .bind(SUMMARY_PREFIX)
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "UPDATE-in-place 应保持摘要行单例");

    let state = get_latest_summary_state(&ctx.pool, &ctx.conversation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.text, "新摘要");
    assert!(
        state.covered_until_rowid.unwrap() > 10,
        "covered 应前进超过 10: {:?}",
        state.covered_until_rowid
    );

    // 4. summary_event 被设置（折叠发生）
    assert!(ctx.summary_event.is_some());
}
