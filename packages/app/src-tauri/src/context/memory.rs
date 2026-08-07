//! L1 Memory — Phase 2 滚动增量摘要（rolling incremental summary）
//!
//! 每个会话至多一份**滚动摘要** S（存为一条 `role=system` 的 messages 行）。
//! S 覆盖会话的一个**前缀** `[0..covered]`，verbatim 后缀 `[covered..]` 原样
//! 发给 LLM。当 verbatim 后缀的 token 超过预算时，把后缀最旧的若干条**折叠进** S
//! （`S' = summarize(S_old + 新折叠的消息)`），推进 covered。
//!
//! # 三层职责（Phase 2 重设计）
//!
//! 1. **HistoryStage**：全量加载（不在本阶段 count-window）。
//! 2. **MemoryStage**（本模块）：质量层——用压缩保留远古上下文，而非硬丢弃。
//! 3. **TokenWindowStage**：硬安全网——按 `max_input_tokens` 强制裁剪，永不超限。
//!
//! # 关键设计
//!
//! - **keep_n 地板**：`resolve_window(agent.max_history_messages)`（默认 20）。
//!   最后 `keep_n` 条永远 verbatim，永不被摘要。
//! - **触发 / 目标比例**：verbatim 后缀 token 超 `max_input*55%` 触发折叠，
//!   一次折到 `max_input*40%` 以下（使下次需积累 ~15% 新 token 才再触发——
//!   天然频率闸门，无需冷却字段）。比例由 [`ContextBudget::fold_trigger_tokens`]
//!   / [`fold_target_tokens`] 派生，与真实窗口挂钩。
//! - **覆盖追踪 `covered_until_rowid`**：S 覆盖的**最后一条**消息的物理 rowid
//!   （非计数——部分加载下计数会静默丢消息）。持久化在 messages 行上。
//! - **`source_rowid` 桥接**：[`ChatMessage`] 携带 `#[serde(skip)] source_rowid`，
//!   MemoryStage 用按值定位（`iter().position(source_rowid == r)`）找切断点，
//!   天然扛住 [`ToolFailureFold`] 的合并 / 重排（identity-by-value）。
//! - **依赖倒置**：`SummaryProvider` trait 定义在 context 层，harness 层实现
//!   （`LlmSummaryProvider`）。测试用 `NoopSummaryProvider` / fake provider。
//!
//! [`ContextBudget::fold_trigger_tokens`]: crate::context::token::ContextBudget::fold_trigger_tokens
//! [`fold_target_tokens`]: crate::context::token::ContextBudget::fold_target_tokens
//! [`ChatMessage`]: crate::infra::protocol::ChatMessage
//! [`ToolFailureFold`]: crate::context::history::fold_repeated_tool_failures

use async_trait::async_trait;
use tracing::{debug, info, warn};

use crate::context::history::{resolve_window, sanitize_history};
use crate::context::pipeline::{PipelineContext, PipelineStage};
use crate::context::token::{
    estimate_message_tokens, estimate_messages_tokens, estimate_tokens,
};
use crate::db::repo::summary::{
    get_latest_summary_state, insert_summary_message, update_summary_message,
};
use crate::error::AppResult;
use crate::infra::cancel::CancellationToken;
use crate::infra::protocol::{ChatMessage, ChatSummaryInjectedPayload};

/// 单次折叠生成摘要的目标 token 上限（传给 [`SummaryProvider::summarize`]）。
///
/// 摘要本身要短：它将被每轮注入上下文。512 token 足以承载多轮事实要点，
/// 又不会喧宾夺主。provider 内部会按 `max_tokens` clamp。
const SUMMARY_FOLD_TOKENS: usize = 512;

// =========================================================================
// SummaryProvider trait
// =========================================================================

/// 摘要 provider trait — context 层定义，harness 层实现
///
/// # 职责
/// 把一段历史 `ChatMessage` 列表压缩成短文本摘要（用于超出上下文窗口时
/// 保留「远古对话」语义）。
///
/// # 协作模式
/// - trait 定义在本模块（context / L2 层）
/// - 具体实现（`LlmSummaryProvider`）放在 harness 层（L3 层）
/// - 由调用方（`commands/chat_cmd.rs`）负责注入具体实例到 `MemoryStage`
///
/// # cancel 语义
/// - 调用方（`MemoryStage`）传入 `&CancellationToken`，provider 在每次
///   LLM chunk / 内部循环 yield 时检查 `cancel.is_cancelled()`
/// - 已取消时 provider 应立刻返回 `AppError::Cancelled`
#[async_trait]
pub trait SummaryProvider: Send + Sync {
    /// 把 `messages` 压缩成短文本摘要
    ///
    /// @param messages    待压缩的历史消息（首条可能是 `[Prior summary]` 前序摘要）
    /// @param max_tokens  摘要目标 token 上限（provider 自行截断 / 多段压缩）
    /// @param cancel      取消令牌；用户停止生成时应立即取消
    /// @returns           摘要字符串（空串 = 取消 / 失败，调用方应跳过落库）
    async fn summarize(
        &self,
        messages: &[ChatMessage],
        max_tokens: usize,
        cancel: &CancellationToken,
    ) -> AppResult<String>;
}

// =========================================================================
// NoopSummaryProvider（默认实现 / 测试用）
// =========================================================================

/// noop 实现：永远返回空字符串。
///
/// 用途：
/// - `assemble_context` 等向后兼容入口使用（不需要摘要）
/// - 测试 / 离线场景下避免真实 LLM 调用
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSummaryProvider;

#[async_trait]
impl SummaryProvider for NoopSummaryProvider {
    async fn summarize(
        &self,
        _messages: &[ChatMessage],
        _max_tokens: usize,
        _cancel: &CancellationToken,
    ) -> AppResult<String> {
        Ok(String::new())
    }
}

// =========================================================================
// MemoryBackend trait（长期 / 短期记忆后端，未来扩展）
// =========================================================================

/// 长期 / 短期记忆后端 trait（占位，未来扩展）。
///
/// 当前方法都有默认实现（返回 None / Ok(())），实现者按需覆盖。
#[allow(dead_code)]
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    /// 检索与 query 相关的记忆（默认返回 None）
    async fn recall(&self, _query: &str) -> AppResult<Option<String>> {
        Ok(None)
    }

    /// 存储键值对记忆（默认 noop）
    async fn store(&self, _key: &str, _value: &str) -> AppResult<()> {
        Ok(())
    }
}

/// 内存后端占位实现
#[cfg(test)]
#[derive(Debug, Default, Clone, Copy)]
pub struct InMemoryBackend;

#[cfg(test)]
#[async_trait]
impl MemoryBackend for InMemoryBackend {}

// =========================================================================
// MemoryStage（PipelineStage — Phase 2 滚动折叠）
// =========================================================================

/// Pipeline Stage — 滚动增量摘要（Phase 2）。
///
/// 每轮发送前执行：读当前摘要覆盖状态 → 决定是否折叠一段 verbatim 后缀进摘要 →
/// 推进 `covered_until_rowid` 并落库 → 注入 `ctx.summary` + 截断已覆盖前缀。
/// 详见模块级文档。
pub struct MemoryStage {
    summary_provider: Box<dyn SummaryProvider>,
}

impl MemoryStage {
    /// 构造 MemoryStage，注入 SummaryProvider 实现
    pub fn new(summary_provider: Box<dyn SummaryProvider>) -> Self {
        Self { summary_provider }
    }
}

#[async_trait]
impl PipelineStage for MemoryStage {
    fn name(&self) -> &'static str {
        "memory"
    }

    async fn execute(&self, ctx: &mut PipelineContext) -> AppResult<()> {
        let max_input = ctx.context_budget.max_input_tokens;
        let len = ctx.history_messages.len();
        if len == 0 || max_input == 0 {
            return Ok(());
        }

        let keep_n = resolve_window(ctx.agent.max_history_messages);
        let trigger = ctx.context_budget.fold_trigger_tokens();
        let target = ctx.context_budget.fold_target_tokens();

        // 读取当前摘要状态（覆盖前缀 + 行 id）。查询失败不阻塞——按「无摘要」继续。
        let state = match get_latest_summary_state(&ctx.pool, &ctx.conversation_id).await {
            Ok(s) => s,
            Err(e) => {
                warn!(target: "ice_paw.context", "MemoryStage: 读取摘要状态失败: {e}，按无摘要继续");
                None
            }
        };

        // 按值定位覆盖切断点（source_rowid 桥接，扛 ToolFailureFold 合并 / 重排）。
        // covered_until_rowid 不在加载切片内（None）→ 当作「从头折叠」自愈。
        let covered_idx = state
            .as_ref()
            .and_then(|s| s.covered_until_rowid)
            .and_then(|r| {
                ctx.history_messages
                    .iter()
                    .position(|m| m.source_rowid == Some(r))
            });
        let verbatim_start = covered_idx.map(|i| i + 1).unwrap_or(0);
        let verbatim_tokens = estimate_messages_tokens(&ctx.history_messages[verbatim_start..]);

        // 不可折叠的尾部地板：最后 keep_n 条始终 verbatim。
        let foldable_end = len.saturating_sub(keep_n);

        debug!(
            target: "ice_paw.context",
            len, keep_n, verbatim_start, verbatim_tokens, trigger, target,
            has_state = state.is_some(),
            "MemoryStage: 评估折叠",
        );

        // 不折叠：(1) verbatim 后缀未超触发线；或 (2) 连第一条可折的都越过了 keep_n 地板。
        // 仍需：注入既有摘要（若有）+ 丢掉已覆盖前缀。
        if verbatim_tokens <= trigger || verbatim_start >= foldable_end {
            if let Some(s) = &state {
                ctx.summary = Some(s.text.clone());
            }
            truncate_history_to(ctx, verbatim_start);
            return Ok(());
        }

        // ---- 折叠一批 ≈ target token，推进 verbatim_start，尊重 keep_n 地板 ----
        let mut acc = 0usize;
        let mut fold_end = verbatim_start;
        while fold_end < foldable_end {
            acc += estimate_message_tokens(&ctx.history_messages[fold_end]);
            fold_end += 1;
            if acc >= target {
                break;
            }
        }

        // 构造摘要输入：旧摘要（作为前序 system 消息，让模型在其基础上扩展）+ 本次折叠的消息
        let mut summary_input: Vec<ChatMessage> = Vec::with_capacity(fold_end - verbatim_start + 1);
        if let Some(s) = &state {
            summary_input.push(ChatMessage::from_text(
                "system",
                format!("[Prior summary]\n{}", s.text),
            ));
        }
        summary_input.extend(ctx.history_messages[verbatim_start..fold_end].iter().cloned());

        info!(
            target: "ice_paw.context",
            fold_from = verbatim_start,
            fold_to = fold_end,
            acc_tokens = acc,
            input_msgs = summary_input.len(),
            "MemoryStage: 触发滚动折叠",
        );

        let s_new = self
            .summary_provider
            .summarize(&summary_input, SUMMARY_FOLD_TOKENS, &ctx.cancel_token)
            .await?;

        // 取消 / 空 → 不落库，下轮重试。仍注入既有摘要（若有）+ 丢已覆盖前缀。
        if s_new.trim().is_empty() {
            warn!(target: "ice_paw.context", "MemoryStage: 摘要返回空（取消或 provider 空），跳过落库");
            if let Some(s) = &state {
                ctx.summary = Some(s.text.clone());
            }
            truncate_history_to(ctx, verbatim_start);
            return Ok(());
        }

        // 新覆盖点 = 本次折叠的最后一条消息的 source_rowid。
        // 生产路径下 history_messages 全部来自 load_history_with_window，source_rowid 必为 Some；
        // 若意外为 None（不可能状态），跳过落库避免 panic，仅本轮注入。
        let new_covered_rowid = match ctx.history_messages[fold_end - 1].source_rowid {
            Some(r) => r,
            None => {
                warn!(
                    target: "ice_paw.context",
                    "MemoryStage: 折叠终点消息无 source_rowid（应为 load_history_with_window 填充），跳过摘要落库"
                );
                ctx.summary = Some(s_new);
                truncate_history_to(ctx, fold_end);
                return Ok(());
            }
        };

        // 落库：有旧摘要 → UPDATE-in-place（保持单例、UI 气泡位置稳定）；无 → INSERT。
        match &state {
            Some(s) => {
                if let Err(e) =
                    update_summary_message(&ctx.pool, &s.row_id, &s_new, new_covered_rowid).await
                {
                    warn!(target: "ice_paw.context", "MemoryStage: 更新摘要失败: {e}，仍注入上下文");
                }
            }
            None => {
                if let Err(e) =
                    insert_summary_message(&ctx.pool, &ctx.conversation_id, &s_new, new_covered_rowid)
                        .await
                {
                    warn!(target: "ice_paw.context", "MemoryStage: 存入摘要失败: {e}，仍注入上下文");
                }
            }
        }

        let original_count = len;
        let summary_tokens = estimate_tokens(&s_new);
        ctx.summary = Some(s_new);
        truncate_history_to(ctx, fold_end);
        let kept_count = ctx.history_messages.len();

        ctx.summary_event = Some(ChatSummaryInjectedPayload {
            conversation_id: ctx.conversation_id.clone(),
            summary_tokens: summary_tokens as u32,
            original_count: original_count as u32,
            kept_count: kept_count as u32,
        });

        info!(
            target: "ice_paw.context",
            original_count, kept_count, summary_tokens,
            covered_until_rowid = new_covered_rowid,
            "MemoryStage: 滚动折叠完成",
        );

        Ok(())
    }
}

/// 把 `ctx.history_messages` 截断为 `[idx..]`，并重新 [`sanitize_history`] 清理
/// 切断点可能产生的孤儿 tool 块。
///
/// **为什么这里必须 sanitize**：MemoryStage 在 [`TokenWindowStage`] 之前执行，
/// 而 TokenWindowStage 只在它自己 trim 时才 sanitize。若 Memory 切片后留下孤儿
/// `tool_result`（其 `tool_use` 落在被丢弃的前缀），TokenWindow 未必触发（suffix
/// 已装下）→ 孤儿存活 → 严格端点（MiniMax）400。
///
/// [`TokenWindowStage`]: crate::context::stages::TokenWindowStage
fn truncate_history_to(ctx: &mut PipelineContext, idx: usize) {
    if idx == 0 {
        return;
    }
    let len = ctx.history_messages.len();
    if idx >= len {
        ctx.history_messages.clear();
        return;
    }
    let tail = ctx.history_messages[idx..].to_vec();
    ctx.history_messages = sanitize_history(tail);
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::summary::SUMMARY_PREFIX;
    use crate::infra::protocol::ContentBlock;
    use std::sync::{Arc, Mutex};

    /// 构造测试用的 CancellationToken
    fn fresh_cancel() -> CancellationToken {
        CancellationToken::new()
    }

    /// 交替 user / assistant 角色名
    fn alt(i: usize) -> &'static str {
        if i % 2 == 0 {
            "user"
        } else {
            "assistant"
        }
    }

    /// 带 source_rowid 的短文本消息（rowid 来自测试索引，usize 直接转 i64）
    fn msg_with_rowid(role: &str, text: &str, rowid: usize) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: vec![ContentBlock::text(text)],
            source_rowid: Some(rowid as i64),
        }
    }

    /// 带 source_rowid 的「大」消息（"a"*1000 ≈ 250 token + 4 overhead = 254 token）
    fn big_msg_with_rowid(role: &str, rowid: usize) -> ChatMessage {
        msg_with_rowid(role, &"a".repeat(1000), rowid)
    }

    /// 记录每次调用入参、返回固定回复的 fake provider（克隆共享调用记录）
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

    // ---- NoopSummaryProvider ----

    #[tokio::test]
    async fn noop_summary_provider_returns_empty() {
        let provider = NoopSummaryProvider;
        let msgs: Vec<_> = (0..10).map(|i| msg_with_rowid(alt(i), "m", i)).collect();
        let cancel = fresh_cancel();

        let result = provider.summarize(&msgs, 1000, &cancel).await.unwrap();
        assert_eq!(result, "");

        let empty_result = provider.summarize(&[], 1000, &cancel).await.unwrap();
        assert_eq!(empty_result, "");

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let cancelled_result = provider.summarize(&msgs, 1000, &cancelled).await.unwrap();
        assert_eq!(cancelled_result, "");
    }

    // ---- InMemoryBackend ----

    #[tokio::test]
    async fn in_memory_backend_returns_none_for_recall() {
        let backend = InMemoryBackend;
        assert!(backend.recall("any query").await.unwrap().is_none());
        assert!(backend.recall("").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn in_memory_backend_returns_ok_for_store() {
        let backend = InMemoryBackend;
        assert!(backend.store("key-1", "value-1").await.is_ok());
        assert!(backend.store("", "v").await.is_ok());
        assert!(backend.store("k", "").await.is_ok());
    }

    // ---- MemoryStage ----

    /// 创建测试用 PipelineContext（内存 SQLite + 种子 agent/conversation）
    async fn make_test_ctx(
        history_messages: Vec<ChatMessage>,
        max_input_tokens: usize,
        max_history_messages: Option<i32>,
    ) -> PipelineContext {
        use crate::context::token::ContextBudget;
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;

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
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("a-t")
        .bind("t")
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
            .bind("conv-t")
            .bind("a-t")
            .bind("test")
            .execute(&pool)
            .await
            .expect("seed conversation");

        let agent = crate::db::models::AgentRow {
            id: "a-t".into(),
            name: "t".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet".into(),
            system_prompt: String::new(),
            api_key_ref: "vault://t".into(),
            base_url: None,
            temperature: 0.7,
            max_tokens: 1024,
            extra_params: "{}".into(),
            sort_order: 0,
            cache_prompt: 0,
            max_history_messages,
            tool_trim_threshold: None,
            context_window: None,
            enabled_tools: None,
            supports_vision: 0,
            description: String::new(),
            avatar: None,
            workspace_path: None,
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
            ContextBudget {
                max_input_tokens,
                ..Default::default()
            },
            "conv-t".into(),
            CancellationToken::new(),
        );
        ctx.history_messages = history_messages;
        ctx
    }

    #[tokio::test]
    async fn memory_stage_noop_when_under_trigger() {
        // 5 条小消息 + 大预算（trigger=55000）→ verbatim 远未达触发线，noop
        let history: Vec<_> = (0..5).map(|i| msg_with_rowid(alt(i), "hi", i)).collect();
        let mut ctx = make_test_ctx(history, 100_000, None).await;

        let stage = MemoryStage::new(Box::new(NoopSummaryProvider));
        stage.execute(&mut ctx).await.unwrap();

        assert!(ctx.summary.is_none(), "未达触发线 summary 应为 None");
        assert_eq!(ctx.history_messages.len(), 5, "history 不应被截断");
        assert!(ctx.summary_event.is_none());
    }

    #[tokio::test]
    async fn memory_stage_first_fold_inserts_summary_and_sets_coverage() {
        // 30 条 * ~254 token = 7620 > trigger(5500)；keep_n=20 → foldable_end=10
        let history: Vec<_> = (0..30).map(|i| big_msg_with_rowid(alt(i), i)).collect();
        let mut ctx = make_test_ctx(history, 10_000, None).await;

        let provider = RecordingProvider::new("summary-1");
        let stage = MemoryStage::new(Box::new(provider.clone()));
        stage.execute(&mut ctx).await.unwrap();

        assert_eq!(ctx.summary.as_deref(), Some("summary-1"));
        assert_eq!(ctx.history_messages.len(), 20, "应保留尾部 20 条");
        assert!(ctx.summary_event.is_some());

        // provider 调用一次，入参 10 条（无前序摘要）
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].len(), 10);
        assert!(
            !calls[0][0].content_text().contains("[Prior summary]"),
            "首次折叠入参不应含前序摘要"
        );

        // DB：摘要行单例，covered_until_rowid=9（折进摘要的最后一条）
        let state = get_latest_summary_state(&ctx.pool, "conv-t")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.text, "summary-1");
        assert_eq!(state.covered_until_rowid, Some(9));
    }

    #[tokio::test]
    async fn memory_stage_incremental_fold_advances_coverage() {
        // 50 条；预置摘要 covered=9（msg idx9）→ verbatim 从 idx10 起 = 40 条
        let history: Vec<_> = (0..50).map(|i| big_msg_with_rowid(alt(i), i)).collect();
        let mut ctx = make_test_ctx(history, 10_000, None).await;

        insert_summary_message(&ctx.pool, "conv-t", "summary-1", 9)
            .await
            .unwrap();

        let provider = RecordingProvider::new("summary-2");
        let stage = MemoryStage::new(Box::new(provider.clone()));
        stage.execute(&mut ctx).await.unwrap();

        assert_eq!(ctx.summary.as_deref(), Some("summary-2"));

        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        // 首条应是前序摘要，正文包含旧 summary-1
        assert!(calls[0][0].content_text().contains("[Prior summary]"));
        assert!(calls[0][0].content_text().contains("summary-1"));

        // UPDATE-in-place：摘要行仍单例，正文更新为 summary-2，covered 前进 > 9
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE role='system' AND instr(content, ?) = 1",
        )
        .bind(SUMMARY_PREFIX)
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "UPDATE-in-place 应保持摘要行单例");

        let state = get_latest_summary_state(&ctx.pool, "conv-t")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.text, "summary-2");
        assert!(
            state.covered_until_rowid.unwrap() > 9,
            "covered 应前进: {:?}",
            state.covered_until_rowid
        );
    }

    #[tokio::test]
    async fn memory_stage_keep_n_floor_preserved() {
        // keep_n=20，25 条 → foldable_end=5，最多折 5，保留 ≥ 20
        let history: Vec<_> = (0..25).map(|i| big_msg_with_rowid(alt(i), i)).collect();
        let mut ctx = make_test_ctx(history, 10_000, None).await;

        let provider = RecordingProvider::new("s");
        let stage = MemoryStage::new(Box::new(provider.clone()));
        stage.execute(&mut ctx).await.unwrap();

        assert!(
            ctx.history_messages.len() >= 20,
            "keep_n 地板：至少保留 20 条，实际 {}",
            ctx.history_messages.len()
        );
    }

    #[tokio::test]
    async fn memory_stage_custom_keep_n_respected() {
        // 显式 max_history_messages=5 → keep_n=5；30 条 → foldable_end=25
        let history: Vec<_> = (0..30).map(|i| big_msg_with_rowid(alt(i), i)).collect();
        let mut ctx = make_test_ctx(history, 10_000, Some(5)).await;

        let provider = RecordingProvider::new("s");
        let stage = MemoryStage::new(Box::new(provider.clone()));
        stage.execute(&mut ctx).await.unwrap();

        assert!(
            ctx.history_messages.len() >= 5,
            "自定义 keep_n=5 地板，实际 {}",
            ctx.history_messages.len()
        );
        // 折叠量应远大于 keep_n=20 默认场景（foldable_end=25 vs 10）
        let calls = provider.calls.lock().unwrap();
        assert!(calls[0].len() > 10, "小 keep_n 应允许折叠更多消息");
    }

    #[tokio::test]
    async fn memory_stage_covered_rowid_not_in_slice_self_heals() {
        // 预置 legacy 摘要 covered=9999（不在切片 rowid 0..29）→ covered_idx=None
        // → 从头折叠（自愈），但仍把 legacy 摘要作为前序喂入
        let history: Vec<_> = (0..30).map(|i| big_msg_with_rowid(alt(i), i)).collect();
        let mut ctx = make_test_ctx(history, 10_000, None).await;
        insert_summary_message(&ctx.pool, "conv-t", "legacy-summary", 9999)
            .await
            .unwrap();

        let provider = RecordingProvider::new("healed");
        let stage = MemoryStage::new(Box::new(provider.clone()));
        stage.execute(&mut ctx).await.unwrap();

        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        // 自愈：从头折，首条仍是前序摘要（保留 legacy 事实）
        assert!(calls[0][0].content_text().contains("[Prior summary]"));
        assert!(calls[0][0].content_text().contains("legacy-summary"));
        assert_eq!(ctx.summary.as_deref(), Some("healed"));
    }

    #[tokio::test]
    async fn memory_stage_truncate_re_sanitizes_orphan_tool_result() {
        // 构造折叠边界制造孤儿：idx9=assistant ToolUse(X)，idx10=user ToolResult(X)。
        // keep_n=20 → foldable_end=10；折叠 [0..10) 后保留 [10..]，首条是孤儿 ToolResult(X)
        // （其 ToolUse(X) 在 idx9 已折进摘要）→ sanitize_history 必须移除它。
        let mut history: Vec<ChatMessage> = Vec::with_capacity(30);
        for i in 0..30 {
            let msg = if i == 9 {
                ChatMessage {
                    role: "assistant".into(),
                    content: vec![ContentBlock::ToolUse {
                        id: "X".into(),
                        name: "tool".into(),
                        input: "{}".into(),
                    }],
                    source_rowid: Some(9),
                }
            } else if i == 10 {
                ChatMessage {
                    role: "user".into(),
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: "X".into(),
                        content: "result".into(),
                        is_error: None,
                    }],
                    source_rowid: Some(10),
                }
            } else {
                big_msg_with_rowid(alt(i), i)
            };
            history.push(msg);
        }
        let mut ctx = make_test_ctx(history, 10_000, None).await;

        let provider = RecordingProvider::new("s");
        let stage = MemoryStage::new(Box::new(provider.clone()));
        stage.execute(&mut ctx).await.unwrap();

        // 保留部分首条不应是孤儿 tool_result
        assert!(!ctx.history_messages.is_empty());
        let first = &ctx.history_messages[0];
        assert!(
            !matches!(first.content.first(), Some(ContentBlock::ToolResult { .. })),
            "孤儿 tool_result 应被 re-sanitize 移除"
        );
        // 全量无孤儿 tool_result
        let valid_ids: std::collections::HashSet<&str> = ctx
            .history_messages
            .iter()
            .flat_map(|m| {
                m.content.iter().filter_map(|b| match b {
                    ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
                    _ => None,
                })
            })
            .collect();
        for m in &ctx.history_messages {
            for b in &m.content {
                if let ContentBlock::ToolResult { tool_use_id, .. } = b {
                    assert!(
                        valid_ids.contains(tool_use_id.as_str()),
                        "孤儿 tool_result({tool_use_id}) 漏网"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn memory_stage_empty_summary_skips_persist() {
        // provider 返回空（取消 / 失败）→ 不落库、不发 event，仍丢已覆盖前缀
        let history: Vec<_> = (0..30).map(|i| big_msg_with_rowid(alt(i), i)).collect();
        let mut ctx = make_test_ctx(history, 10_000, None).await;
        insert_summary_message(&ctx.pool, "conv-t", "old", 9)
            .await
            .unwrap();

        let stage = MemoryStage::new(Box::new(NoopSummaryProvider)); // 返回空
        stage.execute(&mut ctx).await.unwrap();

        // 空摘要 → 跳过落库，但既有摘要仍注入
        assert_eq!(ctx.summary.as_deref(), Some("old"));
        assert!(ctx.summary_event.is_none(), "空摘要不应发 event");
        // 摘要行未被改写
        let state = get_latest_summary_state(&ctx.pool, "conv-t")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.text, "old");
        assert_eq!(state.covered_until_rowid, Some(9));
    }

    #[tokio::test]
    async fn memory_stage_name_is_memory() {
        let stage = MemoryStage::new(Box::new(NoopSummaryProvider));
        assert_eq!(stage.name(), "memory");
    }
}
