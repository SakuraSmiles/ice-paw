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
