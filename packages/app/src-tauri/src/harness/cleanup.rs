//! Chat 收尾工具：CancellationToken 注销 + 成功 DB 回写 + 事件 emit
//!
//! 从 `commands/chat_cleanup.rs` 迁入（W5.6）。
//!
//! 提供两个 pub(crate) 函数，供 chat_cmd.rs / loop_engine 调用：
//! - `cleanup()` — 所有退出路径的公共收尾（注销 CancellationToken）
//! - `cleanup_after_success_with_blocks()` — 正常完成时的 DB 回写 + emit + 注销
//!
//! `cleanup_after_success_with_blocks` 接受 9 个参数，每个参数语义独立
//! （app handle、DB pool、会话 ID、用户消息 ID、助手消息 ID、内容、
//!  内容块、完成原因、用量），不宜合并为 struct，因此通过 `#[allow]`
//! 抑制 clippy::too_many_arguments。
//!
//! M1.3 新增：成功收尾时同时回填 user_msg_id 与 asst_msg_id 的 token_count。
//! - `asst_token_count = prompt_tokens + completion_tokens`（盖底 max(1)）
//! - `user_token_count = prompt_tokens`（provider 已算整个 prompt；
//!   若 provider 未返回 usage，则用 `estimate_tokens` 估值并盖底 max(1)）

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};

use crate::context::token::estimate_tokens;
use crate::db::repo;
use crate::harness::chat_state::ChatState;
use crate::infra::protocol::{ChatDonePayload, ContentBlock, TokenUsage};

/// M1.3: Token 数最小护值 —— 0 表示「未填写」语义不明，调为 1 保证 DB 永远有值
const MIN_TOKEN_COUNT: i32 = 1;

/// M1.3: 计算 user / asst 两条消息的 token_count
///
/// 被 `cleanup_after_success_with_blocks` 调用，本身是纯函数以便单测。
///
/// # 入参
/// - `usage`   provider 返回的 TokenUsage（可能为 None）
/// - `content` 流式产生的最终文本（仅在 `usage == None` 时用于 user 估值）
///
/// # 返回
/// `(asst_token_count, user_token_count)`
///
/// # 策略
/// - provider 返回 usage：
///   - asst = max(prompt + completion, 1)
///   - user = max(prompt, 1)
/// - provider 未返回 usage：
///   - asst = 1（盖底）
///   - user = max(estimate_tokens(content), 1)
pub(crate) fn compute_token_counts(usage: Option<&TokenUsage>, content: &str) -> (i32, i32) {
    match usage {
        Some(u) => {
            let asst = u
                .prompt_tokens
                .saturating_add(u.completion_tokens)
                .max(MIN_TOKEN_COUNT as u32) as i32;
            let user = u.prompt_tokens.max(MIN_TOKEN_COUNT as u32) as i32;
            (asst, user)
        }
        None => {
            let user = (estimate_tokens(content) as i32).max(MIN_TOKEN_COUNT);
            (MIN_TOKEN_COUNT, user)
        }
    }
}

/// 成功完成后的收尾：回写 content + content_blocks + emit done + 注销 token
///
/// # M1.3 变更
/// - 新增 `user_msg_id` 参数
/// - 在原有的 update_content / update_content_blocks 之后，额外发起
///   两个 `update_token_count` 调用（user / asst 各一）
/// - token 计算逻辑委托给 [`compute_token_counts`]
#[allow(clippy::too_many_arguments)]
pub(crate) fn cleanup_after_success_with_blocks(
    app: &AppHandle,
    pool: &SqlitePool,
    conv_id: &str,
    user_msg_id: &str,
    asst_msg_id: &str,
    content: &str,
    content_blocks: &[ContentBlock],
    finish_reason: &str,
    usage: Option<TokenUsage>,
) {
    let pool_clone = pool.clone();
    let user_msg_id_clone = user_msg_id.to_string();
    let asst_msg_id_clone = asst_msg_id.to_string();
    let content_clone = content.to_string();
    let blocks_json = serde_json::to_string(content_blocks).unwrap_or_else(|_| "[]".to_string());

    // M1.3: 计算 token count
    let (asst_token_count, user_token_count) = compute_token_counts(usage.as_ref(), content);

    tokio::spawn(async move {
        // M1.4: DB 回写失败必须留日志；原有 `let _ =` 会静默丢失错误，
        // 当磁盘满 / SQLite 锁竞争 / 进程被 kill 等场景下无法排查。
        if let Err(e) =
            repo::message::update_content(&pool_clone, &asst_msg_id_clone, &content_clone).await
        {
            tracing::warn!(
                target: "ice_paw.cleanup",
                "回写助手消息内容失败: msg_id={}, err={}",
                asst_msg_id_clone,
                e
            );
        }
        if let Err(e) =
            repo::message::update_content_blocks(&pool_clone, &asst_msg_id_clone, &blocks_json).await
        {
            tracing::warn!(
                target: "ice_paw.cleanup",
                "回写 content_blocks 失败: msg_id={}, err={}",
                asst_msg_id_clone,
                e
            );
        }
        // M1.3: 回填 token_count（顺序不影响语义，独立写入）
        if let Err(e) =
            repo::message::update_token_count(&pool_clone, &asst_msg_id_clone, asst_token_count)
                .await
        {
            tracing::warn!(
                target: "ice_paw.cleanup",
                "回写 asst token_count 失败: msg_id={}, err={}",
                asst_msg_id_clone,
                e
            );
        }
        if let Err(e) =
            repo::message::update_token_count(&pool_clone, &user_msg_id_clone, user_token_count)
                .await
        {
            tracing::warn!(
                target: "ice_paw.cleanup",
                "回写 user token_count 失败: msg_id={}, err={}",
                user_msg_id_clone,
                e
            );
        }
    });

    if let Err(e) = app.emit(
        "chat:done",
        ChatDonePayload {
            conversation_id: conv_id.to_string(),
            message_id: asst_msg_id.to_string(),
            finish_reason: finish_reason.to_string(),
            usage,
        },
    ) {
        tracing::warn!(
            target: "ice_paw.cleanup",
            "emit chat:done 失败: conv_id={}, err={}",
            conv_id,
            e
        );
    }
    cleanup(app, pool, conv_id);
}

/// 注销 CancellationToken（所有退出路径的公共收尾）
pub(crate) fn cleanup(app: &AppHandle, _pool: &SqlitePool, conv_id: &str) {
    let chat_state = app.state::<ChatState>();
    chat_state.unregister(conv_id);
}

// =========================================================================
// 单元测试（M1.3）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// helper：构造一个标准 usage 对象
    fn usage(prompt: u32, completion: u32) -> TokenUsage {
        TokenUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            cached_tokens: 0,
        }
    }

    #[test]
    fn cleanup_writes_token_count_for_both_when_provider_returns_usage() {
        // provider 返回完整 usage：asst = prompt + completion，user = prompt
        let u = usage(100, 50);
        let (asst, user) = compute_token_counts(Some(&u), "hello world");
        assert_eq!(asst, 150, "asst 应为 prompt + completion");
        assert_eq!(user, 100, "user 应为 prompt");
    }

    #[test]
    fn cleanup_estimates_for_user_when_provider_omits_usage() {
        // provider 未返回 usage：asst 盖底 1，user 用 estimate_tokens
        let (asst, user) = compute_token_counts(None, "hello world hello world hello world hello world hello");
        // "hello world hello world hello world hello world hello" 大约 12 token
        // （11 ascii / 4 = 2.75 → 3 + 5 个 hello + 5 个 world 单词全小写 = 估算类似 10 token）
        assert_eq!(asst, 1, "asst 盖底为 1");
        assert!(
            user >= 1,
            "user 应至少为 1（cover empty content case）"
        );
        assert!(
            user > 5,
            "实际文本应该估算出超过 5 个 token，得到 {user}"
        );
    }

    #[test]
    fn cleanup_writes_max_one_for_asst_when_no_usage() {
        // provider 未返回 usage 且 content 为空 → asst = 1, user = 1
        let (asst, user) = compute_token_counts(None, "");
        assert_eq!(asst, 1);
        assert_eq!(user, 1, "空文本估值 0 → 盖底 1");
    }

    #[test]
    fn cleanup_writes_min_one_for_user_when_provider_prompt_zero() {
        // provider 返回 prompt=0 (极少见) → user 盖底为 1
        let u = usage(0, 10);
        let (asst, user) = compute_token_counts(Some(&u), "content");
        assert_eq!(asst, 10);
        assert_eq!(user, 1);
    }
}
