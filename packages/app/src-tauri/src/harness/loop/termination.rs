//! 终止收尾辅助（U3-5 ④ loop_engine 再拆）。
//!
//! 从 `harness::loop_engine` 拆出的两个 `stream_loop_inner` 各终止分支复用的独立辅助：
//! - [`budget_exceeded_fallback`] — 预算超限时的终止提示文案（纯函数，便于单测）
//! - [`finalize_guard_logged`] — 终止守卫：落盘成功 → 发 `assistant_message`、
//!   删占位 → 发 `message_discarded`，返回值透传 `PersistOutcome`

use crate::harness::batch_writer::BatchWriter;
use crate::harness::cleanup::{finalize_assistant_without_tool_use, PersistOutcome};
use crate::harness::event_log::{self, EventCtx};
use crate::infra::protocol::ContentBlock;

/// budget_exceeded 终止时的 fallback 提示文案（纯函数便于单测）。
///
/// 两分支：显式硬上限（续期额度 0）给出「注释掉该行恢复自适应+续期」的
/// 自助指引；默认额度用尽则说明续期次数已耗完。数字与 chat:budget 终态
/// 事件一致（计费口径：缓存命中按 1/10 折扣），用户在提示行即可看到
/// 「已用多少 / 上限多少」。
pub(crate) fn budget_exceeded_fallback(cumulative: usize, cap: usize, max_renewals: u32) -> String {
    if max_renewals == 0 {
        format!(
            "（本次累计已消耗 {cumulative} tokens，达到显式预算上限 {cap}，已停止。\
             发送新消息即可继续。注：agent.yaml 显式设置的 max_total_tokens 为\
             硬上限、不自动续期，长对话会频繁触顶；注释掉该行可恢复按上下文\
             窗口 3× 自适应并自动续期。）"
        )
    } else {
        format!(
            "（本次累计已消耗 {cumulative} tokens，已达预算上限 {cap} 且自动续期\
             额度（{max_renewals} 次）用尽，已停止。发送新消息即可继续。）"
        )
    }
}

/// 终止守卫 + 事件镜像：[`finalize_assistant_without_tool_use`] 落盘成功 → 发
/// `assistant_message`（镜像 PersistOutcome 里的实际写入值，含 round/continuation
/// 等 loop 语境——这就是事件发在 loop 侧而非 cleanup 侧的原因）；删占位 →
/// 发 `message_discarded`。
///
/// 返回值透传 PersistOutcome（调用方若需区分处理仍可用）。
// 11 个参数均为该守卫点独立输入且仅 5 个调用点（全在 loop_engine）；事件化收纳进
// EventCtx 后已是最小参数面，进一步收敛需把 round 语境打包成 struct，收益不足。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn finalize_guard_logged(
    pool: &sqlx::SqlitePool,
    batch_writer: &BatchWriter,
    ev: &EventCtx,
    asst_msg_id: &str,
    msg_text: &str,
    round_blocks: &[ContentBlock],
    completion_tokens: Option<u32>,
    fallback_text: Option<&str>,
    round: u32,
    model: Option<&str>,
    continuation: bool,
    duration_ms: u64,
) -> PersistOutcome {
    let outcome = finalize_assistant_without_tool_use(
        pool,
        batch_writer,
        asst_msg_id,
        msg_text,
        round_blocks,
        completion_tokens,
        fallback_text,
    )
    .await;
    match &outcome {
        PersistOutcome::Persisted { content, blocks } => {
            event_log::log_assistant_message(
                pool,
                ev,
                asst_msg_id,
                model,
                content,
                blocks,
                completion_tokens.map(|t| t.max(1) as i64),
                Some(duration_ms),
                round,
                continuation,
            )
            .await;
        }
        PersistOutcome::Deleted => {
            event_log::log_message_discarded(pool, ev, asst_msg_id, "termination_guard_no_text")
                .await;
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 显式硬上限分支：含数字 + 自助指引（注释掉该行恢复自适应+续期）
    #[test]
    fn budget_exceeded_fallback_explicit_cap_has_numbers_and_hint() {
        let s = budget_exceeded_fallback(845_000, 800_000, 0);
        assert!(s.contains("845000"), "应含累计数: {s}");
        assert!(s.contains("800000"), "应含上限数: {s}");
        assert!(s.contains("硬上限"), "应说明硬上限语义: {s}");
        assert!(s.contains("注释掉该行"), "应给自助指引: {s}");
    }

    /// 默认额度用尽分支：说明续期次数已耗完
    #[test]
    fn budget_exceeded_fallback_renewed_out_mentions_quota() {
        let s = budget_exceeded_fallback(1_900_000, 1_800_000, 2);
        assert!(s.contains("1900000"), "应含累计数: {s}");
        assert!(s.contains("1800000"), "应含上限数: {s}");
        assert!(s.contains("续期"), "应提及续期额度: {s}");
        assert!(!s.contains("注释掉该行"), "非硬上限不给 yaml 指引: {s}");
    }
}
