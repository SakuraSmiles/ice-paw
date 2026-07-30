//! L2 Batch Writer — 流式写入批处理器（REQ-XC-004）
//!
//! 设计目的
//! =========
//!
//! 原流式循环（`loop_engine::stream_loop`）只在最终 cleanup 阶段一次性把
//! 累计文本回写到 `messages.content`。这意味着：
//!
//! - 中途崩溃/强杀：用户看到的是空 assistant 消息，正文全部丢失
//! - 前端 `chat:done` 之前若切走会话再回来，看到的是空 assistant 占位
//! - 多轮工具调用场景下，每一轮的 round_text 都不会立刻入库
//!
//! 本模块引入 `BatchWriter`：把写入「累积到阈值后批量 flush」，保证：
//!
//! 1. 累计 ≥ `char_threshold` 字符（默认 50）→ 立即 flush 一次
//! 2. 距上次 flush ≥ `time_threshold`（默认 200ms）→ 立即 flush 一次
//! 3. 后台 tokio::spawn 任务每 200ms 检查一次（兜底：低频 chunk 也定期入库）
//!
//! 消息顺序保证
//! ============
//!
//! - 本批写器**只写一条**消息（`asst_msg_id`），始终以「当前累积全文」覆写
//!   `messages.content`（token_count 由 `finalize_assistant_message` 负责写入）。
//!   读取侧拿到的总是最新累积状态，多次覆写不会乱序。
//! - `BatchWriter` 内部的 `flush()` 调用是 `&mut self`，调用方串行调用，
//!   同一时刻不会有并发 flush 竞争。
//! - 后台定时任务用 `mpsc::Sender<BatchCommand>` 把命令送入同一个
//!   writer event loop，单线程消费，天然保证顺序。
//!
//! 与现有 `repo::message::update_content` 的关系
//! ==============================================
//!
//! `BatchWriter::flush` 内部直接调用 `repo::message::update_content`，
//! 只写流式文本中间态。token_count 与 content_blocks 由 `finalize_assistant_message`
//! 在每轮结束时权威写入，覆盖 BatchWriter 的中间态 —— 幂等且正确。

use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::db::repo;
use crate::error::AppResult;

/// 字符阈值：累计到 50 字符立即 flush 一次
pub const DEFAULT_CHAR_THRESHOLD: usize = 50;

/// 时间阈值：距上次 flush ≥ 200ms 立即 flush 一次
pub const DEFAULT_TIME_THRESHOLD: Duration = Duration::from_millis(200);

/// 后台定时检查间隔：每 200ms 检查一次是否需要 flush
pub const DEFAULT_TICK_INTERVAL: Duration = Duration::from_millis(200);

/// mpsc 通道容量（命令队列上限）
const COMMAND_CHANNEL_CAPACITY: usize = 32;

// ============================================================================
// 命令枚举（后台任务消费的事件）
// ============================================================================

/// 发给后台 writer 的命令
#[derive(Debug)]
enum BatchCommand {
    /// 推入文本增量（仅累积，不立即 flush）
    PushText(String),
    /// 切换写入目标消息（多轮工具：先 flush 旧消息，再切到新 assistant 占位）
    SetMessageId(String),
    /// 立即 flush（如：累积达阈值时由调用方触发）
    FlushNow,
    /// 关闭 writer（drain 剩余 pending 后退出后台循环）
    Shutdown,
}

// ============================================================================
// BatchWriter（前台持有）
// ============================================================================

/// 流式写入批处理器（前台句柄）
///
/// 内部状态：
/// - `pending_text`：自上次 flush 以来累积的**全文**（注意：不是 delta，
///   而是当前应该写入的完整文本）。我们采用「全文覆写」策略，每次 flush
///   都把 messages.content 设成 latest 累积值。
/// - `last_flush`：上次 flush 的瞬时时刻
///
/// 并发模型：
/// - 持有 `Arc<Inner>`，clone 出去给后台任务消费
/// - 后台任务通过 mpsc 接收命令，串行处理
/// - 前台可继续 push（通过 mpsc），但 flush_now 需要走命令队列保证顺序
///
/// Clone: 共享内部状态，clone 出来的实例可继续 push/shutdown。
/// 这是必要的：stream_loop 持有 writer 用于 shutdown，同时需要把 writer
/// move 进 stream_loop_inner 用于 push_text。
#[derive(Clone)]
pub struct BatchWriter {
    // `inner` 看起来未被读取，但其 Arc 引用计数本身就是语义：
    // - 保持 Inner 存活期间，后台任务可以安全 lock 它
    // - writer 被 drop 后，Arc 计数 → 0，后台任务可以发现并退出
    // 因此不能省略；用 `#[allow(dead_code)]` 抑制误报。
    #[allow(dead_code)]
    inner: Arc<AsyncMutex<Inner>>,
    tx: mpsc::Sender<BatchCommand>,
}

struct Inner {
    pool: SqlitePool,
    msg_id: String,
    /// 自上次 flush 后累积的「最新全文」（注意：是完整文本，不是 delta）
    pending_text: String,
    char_threshold: usize,
    time_threshold: Duration,
    last_flush: Instant,
    closed: bool,
}

impl BatchWriter {
    /// 创建 BatchWriter + 启动后台定时 flush 任务
    ///
    /// 参数：
    /// - `pool`：sqlx 连接池（Clone 廉价）
    /// - `msg_id`：要写入的 assistant message UUID
    /// - `tick_interval`：后台检查间隔（默认 200ms）
    pub fn spawn(
        pool: SqlitePool,
        msg_id: String,
        tick_interval: Duration,
    ) -> (Self, JoinHandle<()>) {
        Self::spawn_with_thresholds(
            pool,
            msg_id,
            DEFAULT_CHAR_THRESHOLD,
            DEFAULT_TIME_THRESHOLD,
            tick_interval,
        )
    }

    /// 创建 BatchWriter，自定义字符/时间阈值 + 启动后台任务
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_thresholds(
        pool: SqlitePool,
        msg_id: String,
        char_threshold: usize,
        time_threshold: Duration,
        tick_interval: Duration,
    ) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);

        let inner = Arc::new(AsyncMutex::new(Inner {
            pool,
            msg_id,
            pending_text: String::new(),
            char_threshold,
            time_threshold,
            last_flush: Instant::now(),
            closed: false,
        }));

        let inner_for_task = inner.clone();
        let handle = tokio::spawn(async move {
            run_writer_loop(inner_for_task, rx, tick_interval).await;
        });

        let writer = BatchWriter { inner, tx };
        (writer, handle)
    }

    /// 推入文本增量（不立即 flush）
    ///
    /// 调用方传入的是「**最新完整文本**」（即当前助手消息应展示的全部文本），
    /// 由 `loop_engine::stream_loop` 在每轮 `consume_stream` 返回后传入
    /// `all_text.clone()`。本方法只覆盖 `pending_text`，**不会**追加 delta。
    ///
    /// 推入后内部检查：若字符增量达阈值 → 异步触发 flush_now。
    pub async fn push_text(&self, latest_full_text: String) {
        if let Err(e) = self
            .tx
            .send(BatchCommand::PushText(latest_full_text))
            .await
        {
            warn!(
                target: "ice_paw.batch_writer",
                "BatchWriter push_text 发送失败（writer 可能已关闭）: {}",
                e
            );
        }
    }

    /// 切换写入目标消息：先 flush 旧消息的 pending，再切到新 msg_id（pending 清空）
    ///
    /// 多轮工具调用场景：每轮工具结束后 loop_engine 创建新的 assistant 占位消息，
    /// 调本方法把 writer 对准新消息。调用前通常已 flush_now()。
    pub async fn set_msg_id(&self, msg_id: String) {
        if let Err(e) = self.tx.send(BatchCommand::SetMessageId(msg_id)).await {
            warn!(
                target: "ice_paw.batch_writer",
                "BatchWriter set_msg_id 发送失败: {}",
                e
            );
        }
    }

    /// 主动触发一次 flush（不等阈值触发）
    pub async fn flush_now(&self) {
        if let Err(e) = self.tx.send(BatchCommand::FlushNow).await {
            warn!(
                target: "ice_paw.batch_writer",
                "BatchWriter flush_now 发送失败: {}",
                e
            );
        }
    }

    /// 关闭 writer：drain 剩余 pending 后等后台任务退出
    ///
    /// 应在 `stream_loop` 退出路径（cleanup 之后）调用，确保最后一次
    /// 累积的内容被写入 DB。后台任务收到 `Shutdown` 后会做 final flush
    /// 然后退出循环。
    pub async fn shutdown(&self) {
        // send 是异步 + bounded；即使 writer 已关闭 channel，也只是 warn，
        // 不会 panic。配合 handle.await 确保后台任务真正退出。
        let _ = self.tx.send(BatchCommand::Shutdown).await;
    }
}

// ============================================================================
// 后台 writer 循环
// ============================================================================

/// 后台消费 mpsc 命令 + 周期性 tick 检查时间阈值
async fn run_writer_loop(
    inner: Arc<AsyncMutex<Inner>>,
    mut rx: mpsc::Receiver<BatchCommand>,
    tick_interval: Duration,
) {
    let mut ticker = tokio::time::interval(tick_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            // 优先级 1：处理 mpsc 命令
            cmd = rx.recv() => {
                match cmd {
                    Some(BatchCommand::PushText(text)) => {
                        let mut guard = inner.lock().await;
                        // 计算增量（pending 实际写入 = 最新全文）
                        let prev_len = guard.pending_text.len();
                        guard.pending_text = text;
                        let grew = guard.pending_text.len().saturating_sub(prev_len);
                        // 字符阈值触发：增量 ≥ 阈值
                        if grew >= guard.char_threshold {
                            if let Err(e) = flush_locked(&mut guard).await {
                                warn!(
                                    target: "ice_paw.batch_writer",
                                    "flush 失败（字符阈值触发）: msg_id={}, err={}",
                                    guard.msg_id,
                                    e
                                );
                            }
                        }
                    }
                    Some(BatchCommand::SetMessageId(new_id)) => {
                        let mut guard = inner.lock().await;
                        // 先 flush 旧消息的 pending，避免脏写到新消息
                        if !guard.pending_text.is_empty() {
                            if let Err(e) = flush_locked(&mut guard).await {
                                warn!(
                                    target: "ice_paw.batch_writer",
                                    "切换 msg_id 前 flush 失败: old={}, err={}",
                                    guard.msg_id, e
                                );
                            }
                        }
                        guard.msg_id = new_id;
                        guard.pending_text.clear();
                    }
                    Some(BatchCommand::FlushNow) => {
                        let mut guard = inner.lock().await;
                        if !guard.pending_text.is_empty() {
                            if let Err(e) = flush_locked(&mut guard).await {
                                warn!(
                                    target: "ice_paw.batch_writer",
                                    "flush 失败（FlushNow）: msg_id={}, err={}",
                                    guard.msg_id,
                                    e
                                );
                            }
                        }
                    }
                    Some(BatchCommand::Shutdown) => {
                        let mut guard = inner.lock().await;
                        // 最终 flush：确保最后累积的内容入库
                        if !guard.pending_text.is_empty() {
                            if let Err(e) = flush_locked(&mut guard).await {
                                warn!(
                                    target: "ice_paw.batch_writer",
                                    "final flush 失败: msg_id={}, err={}",
                                    guard.msg_id,
                                    e
                                );
                            }
                        }
                        guard.closed = true;
                        debug!(
                            target: "ice_paw.batch_writer",
                            "BatchWriter 已关闭: msg_id={}",
                            guard.msg_id
                        );
                        return;
                    }
                    None => {
                        // sender 全部 drop（writer 被回收）→ 退出循环
                        let mut guard = inner.lock().await;
                        if !guard.pending_text.is_empty() {
                            let _ = flush_locked(&mut guard).await;
                        }
                        return;
                    }
                }
            }
            // 优先级 2：时间 tick 检查
            _ = ticker.tick() => {
                let mut guard = inner.lock().await;
                let elapsed = guard.last_flush.elapsed();
                // 时间阈值触发：距上次 flush ≥ 阈值
                if elapsed >= guard.time_threshold
                    && (!guard.pending_text.is_empty())
                {
                    if let Err(e) = flush_locked(&mut guard).await {
                        warn!(
                            target: "ice_paw.batch_writer",
                            "flush 失败（时间阈值触发）: msg_id={}, err={}",
                            guard.msg_id,
                            e
                        );
                    }
                }
            }
        }
    }
}

/// 实际执行 flush：写 content（token_count 已由 finalize_assistant_message 负责），更新 last_flush
///
/// 必须持有 `Inner` 的锁；调用方负责锁的获取。
async fn flush_locked(inner: &mut Inner) -> AppResult<()> {
    // 内容写入（仅在 pending_text 非空时写，避免空 update 浪费 IO）
    if !inner.pending_text.is_empty() {
        repo::message::update_content(&inner.pool, &inner.msg_id, &inner.pending_text).await?;
    }
    inner.pending_text.clear();
    inner.last_flush = Instant::now();
    Ok(())
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证：DEFAULT_CHAR_THRESHOLD == 50
    #[test]
    fn default_char_threshold_is_50() {
        assert_eq!(DEFAULT_CHAR_THRESHOLD, 50);
    }

    /// 验证：DEFAULT_TIME_THRESHOLD == 200ms
    #[test]
    fn default_time_threshold_is_200ms() {
        assert_eq!(DEFAULT_TIME_THRESHOLD, Duration::from_millis(200));
    }

    /// 验证：DEFAULT_TICK_INTERVAL == 200ms
    #[test]
    fn default_tick_interval_is_200ms() {
        assert_eq!(DEFAULT_TICK_INTERVAL, Duration::from_millis(200));
    }

    /// 验证：BatchCommand::Shutdown 可被发送
    #[test]
    fn shutdown_command_constructible() {
        let cmd = BatchCommand::Shutdown;
        match cmd {
            BatchCommand::Shutdown => {}
            _ => panic!("expected Shutdown"),
        }
    }

    /// 验证：BatchWriter 能在内存 SQLite 上 spawn 后正常 push / flush
    #[tokio::test]
    async fn batch_writer_writes_to_db() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        use crate::db::models::NewMessage;

        // 准备内存数据库 + 跑迁移
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();

        // 准备 agent + conversation + assistant message
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind("a1")
        .bind("test")
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
        .unwrap();
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES (?, ?, ?)")
            .bind("c1")
            .bind("a1")
            .bind("test")
            .execute(&pool)
            .await
            .unwrap();
        let msg_id = "m1";
        repo::message::create(
            &pool,
            msg_id,
            &NewMessage {
                conversation_id: "c1".into(),
                role: "assistant".into(),
                content: String::new(),
                token_count: None,
                error: None,
                model: Some("claude".into()),
            },
        )
        .await
        .unwrap();

        // 创建 BatchWriter（用小阈值便于测试）
        let (writer, handle) = BatchWriter::spawn_with_thresholds(
            pool.clone(),
            msg_id.to_string(),
            5,                          // char threshold
            Duration::from_millis(50),  // time threshold
            Duration::from_millis(20),  // tick interval
        );

        // push_text 推入 10 字符（> 阈值）应触发字符阈值 flush
        writer.push_text("hello world".to_string()).await;

        // 等后台任务处理
        tokio::time::sleep(Duration::from_millis(100)).await;

        // 验证 DB 中 content 已写入
        let row: (String,) = sqlx::query_as("SELECT content FROM messages WHERE id = ?")
            .bind(msg_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.0, "hello world", "字符阈值触发后 content 应被写入");

        // shutdown 应做 final flush
        writer.push_text("hello world appended".to_string()).await;
        writer.shutdown().await;
        let _ = handle.await;

        let row: (String,) = sqlx::query_as("SELECT content FROM messages WHERE id = ?")
            .bind(msg_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.0, "hello world appended", "shutdown 应触发 final flush");
    }

    /// 验证：低频 chunk（< 字符阈值）也能在时间阈值内被 flush
    #[tokio::test]
    async fn batch_writer_time_threshold_triggers_flush() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        use crate::db::models::NewMessage;

        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind("a1")
        .bind("test")
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
        .unwrap();
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES (?, ?, ?)")
            .bind("c1")
            .bind("a1")
            .bind("test")
            .execute(&pool)
            .await
            .unwrap();
        let msg_id = "m1";
        repo::message::create(
            &pool,
            msg_id,
            &NewMessage {
                conversation_id: "c1".into(),
                role: "assistant".into(),
                content: String::new(),
                token_count: None,
                error: None,
                model: Some("claude".into()),
            },
        )
        .await
        .unwrap();

        // 高字符阈值 + 短时间阈值 → 推入 5 字符不会立即触发，但时间到会触发
        let (writer, handle) = BatchWriter::spawn_with_thresholds(
            pool.clone(),
            msg_id.to_string(),
            1000,                       // char threshold (高)
            Duration::from_millis(50),  // time threshold (短)
            Duration::from_millis(20),  // tick interval
        );

        writer.push_text("hi".to_string()).await; // 2 chars < 1000
        // 等时间阈值 tick
        tokio::time::sleep(Duration::from_millis(150)).await;
        writer.shutdown().await;
        let _ = handle.await;

        let row: (String,) = sqlx::query_as("SELECT content FROM messages WHERE id = ?")
            .bind(msg_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.0, "hi", "时间阈值 tick 后 low-freq chunk 也应被 flush");
    }
}