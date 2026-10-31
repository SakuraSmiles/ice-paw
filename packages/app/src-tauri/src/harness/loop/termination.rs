//! 终止收尾辅助（U3-5 ④ loop_engine 再拆）。
//!
//! 从 `harness::loop_engine` 拆出的独立辅助：
//! - [`finalize_guard_logged`] — 终止守卫：落盘成功 → 发 `assistant_message`、
//!   删占位 → 发 `message_discarded`，返回值透传 `PersistOutcome`
//! - [`round_limit_reminder`] / [`round_limit_warning`] / [`budget_limit_warning`] —
//!   纯软护栏提醒文案（2026-10-30：轮数/预算触顶不再硬停、不再续期，只注入提醒
//!   让 agent 自管理收尾；轮数维度另有 ≥90% 提前提醒 [`round_limit_reminder`]）

use crate::harness::batch_writer::BatchWriter;
use crate::harness::cleanup::{finalize_assistant_without_tool_use, PersistOutcome};
use crate::harness::event_log::{self, EventCtx};
use crate::infra::protocol::ContentBlock;

/// 纯软护栏：工具轮数临近触顶（≥90% 已用）时提前提醒一次，让 agent 有充足轮数余量
/// 自管理收尾。语义与预算维度的「≥90% 提前提醒」（S8-3 budget_reminder）对称——一个
/// 治「撞顶才发觉」的措手不及，给模型留收敛/分段的跑道。
///
/// 只提醒一次（首达阈值那轮），不逐轮轰炸。
pub(crate) fn round_limit_reminder(round: u32, cap: u32) -> String {
    format!(
        "[System] 工具调用轮数即将用尽（已用 {round}/{cap} 轮）。请尽快收敛当前任务：\
         输出阶段性结论，或把未完成部分整理成清晰的手接点（下一步做什么、关键状态在\
         哪），让用户可以低成本继续。"
    )
}

/// 纯软护栏：工具轮数触顶不再硬停、不再自动续期，改注入此提醒让 agent 自管理收尾。
///
/// 文案给两个选择：接近完成 → 收尾；确有大额剩余 → 说明理由后继续。只提醒一次
/// （阶段 H 首次触顶那轮），不逐轮轰炸——与 [`round_limit_reminder`] 的「首达阈值
/// 只提醒一次」语义对齐。真正的失控兜底由 stuck_detect / doom_detect 独立承担。
pub(crate) fn round_limit_warning(round: u32, cap: u32) -> String {
    format!(
        "[System] 已达工具调用轮数上限（{round} 轮，配置上限 {cap}）。请评估当前任务：\
         若已接近完成，请直接输出最终回答收尾；若确实还有大量必要工作，请简明说明继续的\
         理由并继续调用工具。"
    )
}

/// 纯软护栏：Token 预算触顶后不再硬停、不再自动续期，改注入此提醒让 agent
/// 自管理收尾。语义与 [`round_limit_warning`] 对称，只提醒一次。
pub(crate) fn budget_limit_warning(cumulative: usize, cap: usize) -> String {
    format!(
        "[System] Token 预算已用尽（本次累计 {cumulative} tokens，上限 {cap}）。请评估\
         当前任务：若已接近完成，请直接输出最终回答收尾；若确实还有大量必要工作，请简明\
         说明继续的理由并继续调用工具。"
    )
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

    /// 轮数临近触顶提醒：含轮数/上限 + 收敛指引，不含「已停止」
    #[test]
    fn round_limit_reminder_nudges_converge_not_stop() {
        let s = round_limit_reminder(45, 50);
        assert!(s.contains("45"), "应含已用轮数: {s}");
        assert!(s.contains("50"), "应含上限: {s}");
        assert!(s.contains("即将用尽"), "应说明临近触顶: {s}");
        assert!(s.contains("收敛"), "应给收敛指引: {s}");
        assert!(!s.contains("已停止"), "纯软护栏不应出现停止措辞: {s}");
    }

    /// 轮数触顶提醒：含轮数/上限 + 两个选择（收尾 / 说明理由继续），不含「已停止」
    #[test]
    fn round_limit_warning_gives_two_choices_not_stop() {
        let s = round_limit_warning(250, 250);
        assert!(s.contains("250"), "应含轮数: {s}");
        assert!(s.contains("轮数上限"), "应说明触顶维度: {s}");
        assert!(s.contains("收尾"), "应给收尾选择: {s}");
        assert!(s.contains("继续"), "应给继续选择: {s}");
        assert!(!s.contains("已停止"), "纯软护栏不应出现停止措辞: {s}");
    }

    /// 预算触顶提醒：含累计/上限 + 两个选择，不含「已停止」
    #[test]
    fn budget_limit_warning_gives_two_choices_not_stop() {
        let s = budget_limit_warning(1_900_000, 1_800_000);
        assert!(s.contains("1900000"), "应含累计数: {s}");
        assert!(s.contains("1800000"), "应含上限数: {s}");
        assert!(s.contains("预算已用尽"), "应说明触顶维度: {s}");
        assert!(s.contains("收尾"), "应给收尾选择: {s}");
        assert!(s.contains("继续"), "应给继续选择: {s}");
        assert!(!s.contains("已停止"), "纯软护栏不应出现停止措辞: {s}");
    }
}
