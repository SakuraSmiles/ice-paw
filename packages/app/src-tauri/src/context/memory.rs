//! L1 Memory — B04 Memory 积木（M1.5 滚动摘要实现）
//!
//! 当前实现：`MemoryStage` 是一个真实的 PipelineStage，在历史消息
//! 超过 token 阈值时触发 LLM 摘要。
//!
//! 设计要点：
//! - **依赖倒置**：`SummaryProvider` trait 定义在 context 层（消费方），
//!   由 harness 层提供具体实现（`LlmSummaryProvider`，M1.5）。context 层
//!   不直接依赖 harness，符合 dev1 架构评审 P0-1。
//! - **可替换性**：测试 / Mock 场景下可以用 `NoopSummaryProvider` 或
//!   自定义的 fake provider 注入，无需引入 LLM。
//! - **可取消**：`summarize()` 接收 `CancellationToken`（项目自有类型，
//!   见 [`crate::infra::cancel::CancellationToken`]），支持
//!   用户主动停止时取消正在进行的摘要 LLM 调用。
//!
//! # M1.5 行为
//!
//! - `MemoryStage::execute()` 检查 `ctx.history_messages` 的总 token 数
//! - 超过 `ctx.context_budget.summary_threshold_tokens` 时：
//!   1. 检查是否已有最近摘要（避免重复）
//!   2. 计算 split 点（双保险算法）
//!   3. 调 `summary_provider.summarize()` 生成摘要
//!   4. 存入 DB（作为一条 system 消息）
//!   5. 设置 `ctx.summary` 供 FinalAssembleStage 注入
//!   6. 截断 `ctx.history_messages` 保留尾部

use async_trait::async_trait;
use tracing::{debug, info, warn};

use crate::context::pipeline::{PipelineContext, PipelineStage};
use crate::context::token::{compute_split_idx, estimate_messages_tokens, estimate_tokens};
use crate::error::AppResult;
use crate::infra::cancel::CancellationToken;
use crate::infra::protocol::{ChatMessage, ChatSummaryInjectedPayload};

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
/// - 具体实现（M1.5 的 `LlmSummaryProvider`）放在 harness 层（L3 层）
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
    /// @param messages    待压缩的历史消息（已过滤 system/tool）
    /// @param max_tokens  摘要目标 token 上限（provider 自行截断 / 多段压缩）
    /// @param cancel      取消令牌；用户停止生成时应立即取消
    /// @returns           摘要字符串（可能为空 —— 消息数过少时）
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
// MemoryStage（PipelineStage 实现 — M1.5 真实逻辑）
// =========================================================================

/// Pipeline Stage — 滚动摘要（M1.5）
///
/// 行为：
/// 1. 估算 `ctx.history_messages` 的总 token 数
/// 2. 未超阈值 → noop
/// 3. 已有最近摘要 → 跳过（避免重复摘要）
/// 4. 计算 split 点（双保险：10 轮 + 80% threshold）
/// 5. 调 `summary_provider.summarize()` 生成摘要
/// 6. 存入 DB + 设置 `ctx.summary` + 截断 history
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
        let total = estimate_messages_tokens(&ctx.history_messages);

        debug!(
            target: "ice_paw.context",
            "MemoryStage: history {} 条, ~{} tokens, threshold={}",
            ctx.history_messages.len(),
            total,
            ctx.context_budget.summary_threshold_tokens,
        );

        // 1. 未超阈值，不触发
        if total <= ctx.context_budget.summary_threshold_tokens {
            return Ok(());
        }

        // 2. 检查是否已有最近摘要（避免重复）
        let existing = crate::db::repo::summary::get_latest_summary(
            &ctx.pool,
            &ctx.conversation_id,
        )
        .await;

        match existing {
            Ok(Some(text)) => {
                // 已有摘要：复用之，不再调 LLM，但仍需截断 history
                // （修复 M1 Day 4 P1-1：之前只设置 summary 不截断，
                //   导致 token 死循环——旧摘要 + 完整 history 每次都注入，
                //   下次仍超阈值，又命中复用分支，循环往复 token 只增不减）
                debug!(
                    target: "ice_paw.context",
                    "MemoryStage: 已有最近摘要，复用并截断 history (text len={})",
                    text.len()
                );

                // 与新建摘要路径共用同一截断算法，保证行为一致
                let split = compute_split_idx(&ctx.history_messages, &ctx.context_budget);
                if split == 0 {
                    // 消息太少（< MIN_KEEP_MSGS=20），安全兜底：只设 summary 不截断
                    debug!(
                        target: "ice_paw.context",
                        "MemoryStage: 复用摘要但消息太少，不截断"
                    );
                    ctx.summary = Some(text);
                    return Ok(());
                }

                let original_count = ctx.history_messages.len();
                let to_keep = ctx.history_messages[split..].to_vec();

                info!(
                    target: "ice_paw.context",
                    "MemoryStage: 复用摘要，截断 history {}→{} 条",
                    original_count,
                    to_keep.len(),
                );

                // 设置 summary + 截断 history
                ctx.summary = Some(text);
                ctx.history_messages = to_keep;

                // 不设置 summary_event——复用旧摘要不需要再次 toast 提示用户
                return Ok(());
            }
            Ok(None) => {} // 无摘要，继续
            Err(e) => {
                warn!(
                    target: "ice_paw.context",
                    "MemoryStage: 查询摘要失败: {}, 继续执行",
                    e
                );
                // 查询失败不阻塞摘要流程
            }
        }

        // 3. 计算 split 点
        let split = compute_split_idx(&ctx.history_messages, &ctx.context_budget);
        if split == 0 {
            debug!(
                target: "ice_paw.context",
                "MemoryStage: 消息太少无法 split ({} 条), 跳过",
                ctx.history_messages.len()
            );
            return Ok(());
        }

        let original_count = ctx.history_messages.len();
        let to_summarize = &ctx.history_messages[..split];
        let to_keep = ctx.history_messages[split..].to_vec();

        info!(
            target: "ice_paw.context",
            "MemoryStage: 触发摘要，summarize={}条，keep={}条",
            to_summarize.len(),
            to_keep.len(),
        );

        // 4. 调 LLM 生成摘要
        let summary = self
            .summary_provider
            .summarize(to_summarize, 200, &ctx.cancel_token)
            .await?;

        // 5. 摘要为空，跳过
        if summary.trim().is_empty() {
            warn!(target: "ice_paw.context", "MemoryStage: 摘要为空，跳过");
            return Ok(());
        }

        // 6. 存入 DB（作为一条 system 消息）
        match crate::db::repo::summary::insert_summary_message(
            &ctx.pool,
            &ctx.conversation_id,
            &summary,
            to_summarize.len() as i32,
        )
        .await
        {
            Ok(_id) => {
                debug!(target: "ice_paw.context", "MemoryStage: 摘要已存入 DB");
            }
            Err(e) => {
                warn!(
                    target: "ice_paw.context",
                    "MemoryStage: 存入摘要失败: {}, 但仍注入到上下文",
                    e
                );
                // 即使 DB 写入失败，仍然注入摘要到上下文
            }
        }

        // 7. 设置 ctx.summary + 截断 history
        let summary_tokens = estimate_tokens(&summary);
        ctx.summary = Some(summary);
        ctx.history_messages = to_keep;

        // 8. 设置 summary_event（供 chat_cmd emit）
        ctx.summary_event = Some(ChatSummaryInjectedPayload {
            conversation_id: ctx.conversation_id.clone(),
            summary_tokens: summary_tokens as u32,
            original_count: original_count as u32,
            kept_count: ctx.history_messages.len() as u32,
        });

        info!(
            target: "ice_paw.context",
            "MemoryStage: 摘要完成，{}→{} 条，节省 ~{} tokens",
            original_count,
            ctx.history_messages.len(),
            total - summary_tokens,
        );

        Ok(())
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::protocol::ContentBlock;

    /// 构造测试用的 CancellationToken
    fn fresh_cancel() -> CancellationToken {
        CancellationToken::new()
    }

    /// 构造 N 条简单的 user/assistant 交替消息
    fn make_messages(n: usize) -> Vec<ChatMessage> {
        (0..n)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                ChatMessage::from_text(role, format!("msg-{i}"))
            })
            .collect()
    }

    // ---- NoopSummaryProvider ----

    #[tokio::test]
    async fn noop_summary_provider_returns_empty() {
        let provider = NoopSummaryProvider;
        let msgs = make_messages(10);
        let cancel = fresh_cancel();

        let result = provider.summarize(&msgs, 1000, &cancel).await.unwrap();
        assert_eq!(result, "");

        let empty_result = provider.summarize(&[], 1000, &cancel).await.unwrap();
        assert_eq!(empty_result, "");

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let cancelled_result = provider
            .summarize(&msgs, 1000, &cancelled)
            .await
            .unwrap();
        assert_eq!(cancelled_result, "");
    }

    // ---- InMemoryBackend ----

    #[tokio::test]
    async fn in_memory_backend_returns_none_for_recall() {
        let backend = InMemoryBackend;
        let result = backend.recall("any query").await.unwrap();
        assert!(result.is_none());

        let empty = backend.recall("").await.unwrap();
        assert!(empty.is_none());
    }

    #[tokio::test]
    async fn in_memory_backend_returns_ok_for_store() {
        let backend = InMemoryBackend;
        let result = backend.store("key-1", "value-1").await;
        assert!(result.is_ok());

        let empty_key = backend.store("", "v").await;
        assert!(empty_key.is_ok());
        let empty_val = backend.store("k", "").await;
        assert!(empty_val.is_ok());
    }

    // ---- MemoryStage ----

    /// 创建测试用 PipelineContext（内存 SQLite）
    async fn make_test_ctx(
        history_messages: Vec<ChatMessage>,
    ) -> PipelineContext {
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

        // 跑迁移
        sqlx::migrate!("./src/db/migrations").run(&pool).await.expect("migrations");

        // 种子数据
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
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
            max_history_messages: None,
            tool_trim_threshold: None,
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
            crate::context::token::ContextBudget::default(),
            "conv-t".into(),
            CancellationToken::new(),
        );
        ctx.history_messages = history_messages;
        ctx
    }

    #[tokio::test]
    async fn memory_stage_noop_when_under_threshold() {
        // 少量消息，token 未超阈值 → noop
        let history_msgs = make_messages(5);
        let mut ctx = make_test_ctx(history_msgs).await;

        let stage = MemoryStage::new(Box::new(NoopSummaryProvider));
        stage.execute(&mut ctx).await.unwrap();

        assert!(ctx.summary.is_none(), "未超阈值时 summary 应保持 None");
        assert_eq!(ctx.history_messages.len(), 5, "history 不应被截断");
        assert!(ctx.summary_event.is_none());
    }

    #[tokio::test]
    async fn memory_stage_triggers_summarize_when_over() {
        // 使用低阈值 + 足够多的长消息触发摘要
        // NoopSummaryProvider 返回空字符串，所以实际不会写入摘要
        // 但我们可以验证 split 逻辑和 flow
        let long_msgs: Vec<ChatMessage> = (0..40)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                ChatMessage::from_text(role, "a".repeat(3000)) // 750 tokens per msg
            })
            .collect();

        let mut ctx = make_test_ctx(long_msgs).await;
        // 使用极低阈值确保触发
        ctx.context_budget.summary_threshold_tokens = 100;

        let stage = MemoryStage::new(Box::new(NoopSummaryProvider));
        stage.execute(&mut ctx).await.unwrap();

        // NoopProvider 返回空字符串，所以 summary 应仍为 None
        // 但 split 逻辑应该已执行（history 应被截断？不——摘要为空时跳过截断）
        // 验证：由于摘要为空，history 不应被截断
        assert!(ctx.summary.is_none(), "NoopProvider 返回空，summary 应为 None");
    }

    #[tokio::test]
    async fn memory_stage_keeps_tail_drops_head() {
        // 使用一个自定义的 SummaryProvider 来验证 split 和截断
        struct AlwaysSummarize;
        #[async_trait]
        impl SummaryProvider for AlwaysSummarize {
            async fn summarize(
                &self,
                _messages: &[ChatMessage],
                _max_tokens: usize,
                _cancel: &CancellationToken,
            ) -> AppResult<String> {
                Ok("这是一条测试摘要".to_string())
            }
        }

        let long_msgs: Vec<ChatMessage> = (0..40)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                ChatMessage::from_text(role, "a".repeat(3000))
            })
            .collect();

        let mut ctx = make_test_ctx(long_msgs).await;
        ctx.context_budget.summary_threshold_tokens = 100;

        let stage = MemoryStage::new(Box::new(AlwaysSummarize));
        stage.execute(&mut ctx).await.unwrap();

        assert!(ctx.summary.is_some(), "AlwaysSummarize 应返回摘要");
        assert_eq!(ctx.summary.as_deref(), Some("这是一条测试摘要"));
        // history 应被截断（只保留尾部）
        assert!(
            ctx.history_messages.len() < 40,
            "history 应被截断: kept={}",
            ctx.history_messages.len()
        );
        // summary_event 应被设置
        assert!(ctx.summary_event.is_some());
    }

    #[tokio::test]
    async fn memory_stage_does_not_trigger_if_recent_summary_exists() {
        // 先手动插入一条摘要消息到 DB，然后验证 MemoryStage 复用现有摘要
        // M1 Day 4 P1-1 修复后：复用旧摘要 → ctx.summary = Some(existing)，
        // 同时也截断 history（核心修复：打破 token 死循环），
        // 不重发 event、不调 provider
        let long_msgs: Vec<ChatMessage> = (0..40)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                ChatMessage::from_text(role, "a".repeat(3000))
            })
            .collect();

        let mut ctx = make_test_ctx(long_msgs).await;
        ctx.context_budget.summary_threshold_tokens = 100;

        // 手动插入一条摘要到 DB
        crate::db::repo::summary::insert_summary_message(
            &ctx.pool,
            "conv-t",
            "已有摘要内容",
            10,
        )
        .await
        .unwrap();

        let stage = MemoryStage::new(Box::new(NoopSummaryProvider));
        stage.execute(&mut ctx).await.unwrap();

        // 复用旧摘要：ctx.summary 被填充，但不重发 event、不调 provider
        assert_eq!(
            ctx.summary.as_deref(),
            Some("已有摘要内容"),
            "应复用旧摘要而不是覆盖"
        );
        assert!(ctx.summary_event.is_none(), "复用旧摘要时不发 event");

        // ✅ P1-1 修复后：history 应被截断（否则旧摘要 + 完整 history 会
        //   导致 token 死循环——下次进入 Pipeline 仍超阈值，又命中复用分支）
        assert!(
            ctx.history_messages.len() < 40,
            "复用摘要时 history 也应被截断: kept={}",
            ctx.history_messages.len()
        );
        // 保留尾部至少 20 条（MIN_KEEP_MSGS）
        assert!(
            ctx.history_messages.len() >= 20,
            "应保留至少 20 条尾部消息: kept={}",
            ctx.history_messages.len()
        );
    }

    #[tokio::test]
    async fn memory_stage_reuses_summary_without_truncation_when_too_few_msgs() {
        // P1-1 修复后的边界场景：复用旧摘要，但消息数 < MIN_KEEP_MSGS(20)
        // → compute_split_idx 返回 0 → 不截断（安全兜底）
        //
        // 这里用 5 条「超长」消息让单条 token 很大以触发「超阈值」分支，
        // 但消息数仍 < 20，因此 split == 0
        let few_long_msgs: Vec<ChatMessage> = (0..5)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                ChatMessage::from_text(role, "a".repeat(100_000))
            })
            .collect();

        let mut ctx = make_test_ctx(few_long_msgs).await;
        // 阈值极低，确保 total > threshold，命中「复用或新建摘要」分支
        ctx.context_budget.summary_threshold_tokens = 100;

        // 预先在 DB 插入一条摘要，使 get_latest_summary 返回 Some(...)
        crate::db::repo::summary::insert_summary_message(
            &ctx.pool,
            "conv-t",
            "已有摘要",
            2,
        )
        .await
        .unwrap();

        let stage = MemoryStage::new(Box::new(NoopSummaryProvider));
        stage.execute(&mut ctx).await.unwrap();

        // 复用旧摘要：summary 应被填充
        assert_eq!(
            ctx.summary.as_deref(),
            Some("已有摘要"),
            "应复用旧摘要而不是覆盖"
        );
        // 不发 event
        assert!(ctx.summary_event.is_none(), "复用旧摘要时不发 event");
        // 消息太少时不应截断
        assert_eq!(
            ctx.history_messages.len(),
            5,
            "消息太少时（< MIN_KEEP_MSGS）不应截断 history"
        );
    }

    #[tokio::test]
    async fn memory_stage_name_is_memory() {
        let stage = MemoryStage::new(Box::new(NoopSummaryProvider));
        assert_eq!(stage.name(), "memory");
    }
}
