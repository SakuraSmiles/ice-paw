//! Chat 收尾工具：CancellationToken 注销 + 事件 emit
//!
//! 从 `commands/chat_cleanup.rs` 迁入（W5.6）；tool_result 持久化彻底重构后
//! 拆分为职责清晰的收尾函数，供 loop_engine 在不同退出路径调用：
//! - `finalize_assistant_message()` — 每轮结束时即时落盘单条 assistant
//!   （content + content_blocks + 本轮 completion_tokens）
//! - `finalize_success()` — 整次发送成功结束：emit chat:done + 回填原始 user
//!   消息 token_count + 注销 CancellationToken
//! - `finalize_cancel()` — 中途取消：emit chat:done(abort) + 注销
//! - `cleanup()` — 所有退出路径的公共收尾（注销 CancellationToken）
//!
//! 多轮工具下每条 assistant 独立持久化（不再累积到最后一次性写），
//! tool_result 存为独立 user 消息，符合 Anthropic 协议（tool_result 必须在
//! user 消息里）。

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};

use crate::db::repo;
use crate::harness::chat_state::ChatState;
use crate::harness::error_mapping::friendly_error;
use crate::infra::protocol::{ChatDonePayload, ChatErrorPayload, ContentBlock, TokenUsage};

/// Token 数未知时的占位值（provider 未返回 usage）。用 0：前端 badge 的
/// `v-if="token_count"` 对 0 为 falsy，故未知时 badge 不显示，避免「1」看起来像真实值。
const MIN_TOKEN_COUNT: i32 = 0;

/// 即时持久化单条 assistant 消息（每轮结束时调用，同步 await）
///
/// 用单条 UPDATE 原子写 content + content_blocks + 本轮 token_count。必须在
/// chat:done 之前同步完成（loop_engine 阶段 C `.await`），避免紧邻追问读到
/// 「content 已写、content_blocks 仍 "[]"」的半写态导致 tool_use 丢失 → 400。
pub(crate) async fn finalize_assistant_message(
    pool: &SqlitePool,
    asst_msg_id: &str,
    text: &str,
    blocks: &[ContentBlock],
    completion_tokens: Option<u32>,
) {
    let blocks_json = serde_json::to_string(blocks).unwrap_or_else(|e| {
        tracing::error!(target: "ice_paw.cleanup", "ContentBlock 序列化失败 (msg_id={}): {}", asst_msg_id, e);
        "[]".to_string()
    });
    let token_count = completion_tokens
        .map(|t| t.max(1) as i32)
        .unwrap_or(MIN_TOKEN_COUNT);
    if let Err(e) = sqlx::query(
        "UPDATE messages SET content = ?, content_blocks = ?, token_count = ? WHERE id = ?",
    )
    .bind(text)
    .bind(&blocks_json)
    .bind(token_count)
    .bind(asst_msg_id)
    .execute(pool)
    .await
    {
        tracing::warn!(
            target: "ice_paw.cleanup",
            "finalize_assistant_message 落盘失败: id={}, err={}",
            asst_msg_id,
            e
        );
    }
}

// ============================================================================
// 终止路径对称清场守卫（cancel / budget / stuck / 错误路径共用）
// ============================================================================

/// 终止路径的落盘决策（纯逻辑，可单测，不依赖 DB/async）。
///
/// 判定与 sanitize_history 的「assistant 必须含 Text 或 ToolUse」对齐：过滤 ToolUse
/// 后若无 Text 块，则该 assistant 对 OpenAI 协议非法（content=null 且无 tool_calls）。
#[derive(Debug, PartialEq, Eq)]
enum TerminationDecision {
    /// 过滤 ToolUse 后仍有 Text → 落盘过滤后版本（保留 thinking + text）
    PersistFiltered,
    /// 过滤后无 Text，但调用方提供 fallback → 落盘 [Text(fallback)]
    PersistFallback,
    /// 过滤后无 Text 且无 fallback → 删除占位行
    Delete,
}

/// 根据本轮 blocks 决定终止路径如何落盘 assistant。
///
/// 用 `has_text`（而非 `!filtered.is_empty()`）判定——thinking-only 轮（filtered
/// = [Thinking] 非空但无 Text）会被判为「无有效内容」走 Delete/PersistFallback，
/// 不会落盘对 OpenAI 非法的 thinking-only assistant。
fn classify_termination_blocks(
    round_blocks: &[ContentBlock],
    has_fallback: bool,
) -> TerminationDecision {
    let has_text = round_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Text { .. }));
    if has_text {
        TerminationDecision::PersistFiltered
    } else if has_fallback {
        TerminationDecision::PersistFallback
    } else {
        TerminationDecision::Delete
    }
}

/// 终止路径对称清场守卫：在 cancel / budget / stuck / 错误路径上剔除 ToolUse 后落盘
/// assistant，杜绝「有 tool_use 无 tool_result」孤儿（→ thinking-only → OpenAI 400）。
///
/// 复用原 cancel 路径已验证的「filter ToolUse + 删占位/落盘」模式，判定更严格：
/// 用 [`classify_termination_blocks`]（has_text），与 sanitize_history 对齐。
///
/// - `fallback_text = Some`（budget/stuck）：无 Text 时写 [Text(fallback)]，保证 msg_id
///   恒有效（finalize_success 的 final_asst_msg_id 需指向真实存在的消息）；有 Text 时
///   忽略 fallback，保留模型真实文本。
/// - `fallback_text = None`（cancel / 错误）：无 Text 时删占位行。
///
/// 返回 `true` = 占位已删（msg_id 失效）；`false` = 已落盘（msg_id 有效）。
pub(crate) async fn finalize_assistant_without_tool_use(
    pool: &SqlitePool,
    batch_writer: &crate::harness::batch_writer::BatchWriter,
    asst_msg_id: &str,
    round_text: &str,
    round_blocks: &[ContentBlock],
    completion_tokens: Option<u32>,
    fallback_text: Option<&str>,
) -> bool {
    match classify_termination_blocks(round_blocks, fallback_text.is_some()) {
        TerminationDecision::PersistFiltered => {
            let filtered: Vec<ContentBlock> = round_blocks
                .iter()
                .filter(|b| !matches!(b, ContentBlock::ToolUse { .. }))
                .cloned()
                .collect();
            batch_writer.flush_now().await;
            finalize_assistant_message(pool, asst_msg_id, round_text, &filtered, completion_tokens)
                .await;
            false
        }
        TerminationDecision::PersistFallback => {
            let fb = fallback_text.expect("PersistFallback 仅在 has_fallback 时返回");
            batch_writer.flush_now().await;
            finalize_assistant_message(
                pool,
                asst_msg_id,
                fb,
                std::slice::from_ref(&ContentBlock::Text { text: fb.to_string() }),
                completion_tokens,
            )
            .await;
            false
        }
        TerminationDecision::Delete => {
            if let Err(e) = repo::message::delete(pool, asst_msg_id).await {
                tracing::warn!(
                    target: "ice_paw.cleanup",
                    "终止清场删除空占位失败: id={}, err={}",
                    asst_msg_id,
                    e
                );
            }
            true
        }
    }
}

/// 整个发送周期成功结束：emit chat:done + 回填 user 消息 token_count + 注销
///
/// 各 assistant 消息的 content/blocks/token 已由 `finalize_assistant_message`
/// 即时落盘。`final_asst_msg_id` 为最终那条 assistant（chat:done 的 message_id）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn finalize_success(
    app: &AppHandle,
    pool: &SqlitePool,
    conv_id: &str,
    final_asst_msg_id: &str,
    finish_reason: &str,
    usage: Option<TokenUsage>,
    user_msg_id: &str,
    first_prompt_tokens: Option<u32>,
) {
    let pool_clone = pool.clone();
    let user_id = user_msg_id.to_string();
    let user_tokens = first_prompt_tokens
        .map(|p| p.max(1) as i32)
        .unwrap_or(MIN_TOKEN_COUNT);
    tokio::spawn(async move {
        if let Err(e) = repo::message::update_token_count(&pool_clone, &user_id, user_tokens).await {
            tracing::warn!(target: "ice_paw.cleanup", "回写 user token_count 失败: msg_id={}, err={}", user_id, e);
        }
    });
    // ★ 先 unregister 再 emit chat:done：消除竞态窗口——
    // 前端收到 chat:done 后可能立即发送下一条消息，若此时 token 尚未注销，
    // chat_state.start() 会命中"会话已有在途生成任务"。
    cleanup(app, pool, conv_id);
    if let Err(e) = app.emit(
        "chat:done",
        ChatDonePayload {
            conversation_id: conv_id.to_string(),
            message_id: final_asst_msg_id.to_string(),
            finish_reason: finish_reason.to_string(),
            usage,
        },
    ) {
        tracing::warn!(target: "ice_paw.cleanup", "emit chat:done 失败: conv_id={}, err={}", conv_id, e);
    }
}

/// 中途取消：emit chat:done(abort) + 注销
///
/// 当前 assistant 消息已由 BatchWriter flush 部分内容；本函数只负责收尾信号。
pub(crate) fn finalize_cancel(app: &AppHandle, pool: &SqlitePool, conv_id: &str, asst_msg_id: &str) {
    // ★ 先 unregister 再 emit chat:done（同 finalize_success 的排序修复）
    cleanup(app, pool, conv_id);
    if let Err(e) = app.emit(
        "chat:done",
        ChatDonePayload {
            conversation_id: conv_id.to_string(),
            message_id: asst_msg_id.to_string(),
            finish_reason: "abort".to_string(),
            usage: None,
        },
    ) {
        tracing::warn!(target: "ice_paw.cleanup", "emit chat:done(abort) 失败: conv_id={}, err={}", conv_id, e);
    }
}

/// 本轮失败的错误收尾（不含 finalize）：回写错误信息 + emit `chat:error`。
///
/// 供 `stream_with_retry` 的不可重试错误路径使用——那里不能自行 `finalize_cancel`
/// （中止语义由调用方统一处理），故与 [`fail_round_and_cancel`] 拆成两层。
///
/// 顺序：先 `update_error` 再 emit。原 loop_engine 内各错误块的 emit/update 顺序
/// 不一致（重试循环内 emit 在前、其余 update 在前），但二者互无数据依赖，
/// 统一为 update→emit 不影响可观测行为。
pub(crate) async fn emit_round_error(
    app: &AppHandle,
    pool: &SqlitePool,
    conv_id: &str,
    msg_id: &str,
    kind: &str,
    err_msg: &str,
) {
    if let Err(eu) = repo::message::update_error(pool, msg_id, err_msg).await {
        tracing::warn!(
            target: "ice_paw.chat",
            "回写 asst 错误信息失败: msg_id={}, err={}",
            msg_id,
            eu
        );
    }
    if let Err(em) = app.emit(
        "chat:error",
        ChatErrorPayload {
            conversation_id: conv_id.to_string(),
            message_id: msg_id.to_string(),
            kind: kind.to_string(),
            message: friendly_error(err_msg),
        },
    ) {
        tracing::warn!(
            target: "ice_paw.chat",
            "emit chat:error 失败: conv_id={}, err={}",
            conv_id,
            em
        );
    }
}

/// 本轮失败收尾：[`emit_round_error`] + [`finalize_cancel`]。
///
/// 统一了 `stream_loop_inner` 内原先 6 处复制粘贴的「emit chat:error + update_error
/// + finalize_cancel」块（重试耗尽 / 工具执行失败 / 持久化失败 / 占位创建失败等）。
pub(crate) async fn fail_round_and_cancel(
    app: &AppHandle,
    pool: &SqlitePool,
    conv_id: &str,
    msg_id: &str,
    kind: &str,
    err_msg: &str,
) {
    emit_round_error(app, pool, conv_id, msg_id, kind, err_msg).await;
    finalize_cancel(app, pool, conv_id, msg_id);
}

/// 注销 CancellationToken（所有退出路径的公共收尾）
pub(crate) fn cleanup(app: &AppHandle, _pool: &SqlitePool, conv_id: &str) {
    let chat_state = app.state::<ChatState>();
    chat_state.unregister(conv_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> ContentBlock {
        ContentBlock::Text { text: t.into() }
    }
    fn thinking(t: &str) -> ContentBlock {
        ContentBlock::Thinking { thinking: t.into(), signature: None }
    }
    fn tool_use() -> ContentBlock {
        ContentBlock::ToolUse { id: "tu_1".into(), name: "search".into(), input: "{}".into() }
    }

    // --- 有 Text：恒 PersistFiltered（fallback 不抢占真实文本）---
    #[test]
    fn classify_text_only() {
        assert_eq!(
            classify_termination_blocks(&[text("hi")], false),
            TerminationDecision::PersistFiltered
        );
    }

    #[test]
    fn classify_text_plus_tool_use_no_fallback() {
        // 有 tool_use 但也有 text → 剔除 tool_use 后保留 text（杜绝孤儿）
        assert_eq!(
            classify_termination_blocks(&[text("hi"), tool_use()], false),
            TerminationDecision::PersistFiltered
        );
    }

    #[test]
    fn classify_text_plus_tool_use_with_fallback() {
        // 有 text 时 fallback 被忽略，仍保留真实文本
        assert_eq!(
            classify_termination_blocks(&[text("hi"), tool_use()], true),
            TerminationDecision::PersistFiltered
        );
    }

    #[test]
    fn classify_thinking_plus_text() {
        assert_eq!(
            classify_termination_blocks(&[thinking("..."), text("hi")], false),
            TerminationDecision::PersistFiltered
        );
    }

    // --- 无 Text：视 fallback 决定 PersistFallback / Delete ---
    #[test]
    fn classify_pure_tool_use_no_fallback() {
        // 纯 tool_use（无文本）→ 删占位（cancel / 错误语义）
        assert_eq!(
            classify_termination_blocks(&[tool_use()], false),
            TerminationDecision::Delete
        );
    }

    #[test]
    fn classify_pure_tool_use_with_fallback() {
        // 纯 tool_use + fallback → 写 fallback 文本（budget/stuck 语义）
        assert_eq!(
            classify_termination_blocks(&[tool_use()], true),
            TerminationDecision::PersistFallback
        );
    }

    // ★ thinking-only 是关键边界：原 cancel 用 is_empty() 判定会放过它（=[Thinking] 非空），
    // 落盘 thinking-only → OpenAI 400。守卫用 has_text 判定，归入 Delete/PersistFallback。
    #[test]
    fn classify_thinking_only_no_fallback() {
        assert_eq!(
            classify_termination_blocks(&[thinking("...")], false),
            TerminationDecision::Delete
        );
    }

    #[test]
    fn classify_thinking_only_with_fallback() {
        assert_eq!(
            classify_termination_blocks(&[thinking("...")], true),
            TerminationDecision::PersistFallback
        );
    }

    #[test]
    fn classify_thinking_plus_tool_use_no_fallback() {
        // reasoning 模型常见输出 [Thinking, ToolUse]，过滤 ToolUse 后仅剩 Thinking（无 Text）
        assert_eq!(
            classify_termination_blocks(&[thinking("..."), tool_use()], false),
            TerminationDecision::Delete
        );
    }

    #[test]
    fn classify_thinking_plus_tool_use_with_fallback() {
        assert_eq!(
            classify_termination_blocks(&[thinking("..."), tool_use()], true),
            TerminationDecision::PersistFallback
        );
    }

    #[test]
    fn classify_empty_blocks_no_fallback() {
        assert_eq!(
            classify_termination_blocks(&[], false),
            TerminationDecision::Delete
        );
    }

    #[test]
    fn classify_empty_blocks_with_fallback() {
        assert_eq!(
            classify_termination_blocks(&[], true),
            TerminationDecision::PersistFallback
        );
    }
}
