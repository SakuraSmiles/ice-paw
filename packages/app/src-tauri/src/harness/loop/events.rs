//! Loop 事件发射：中间 round-state / budget 事件。
//!
//! 从 `harness::loop_engine` 拆出。

use tauri::{AppHandle, Emitter};

use crate::harness::observable::RoundState;
use crate::infra::protocol::{ChatBudgetPayload, ChatRoundStatePayload};

/// 中间 round-state 事件发射 — 供前端 ChatStatusBar 实时显示进度。
/// 失败仅记录 warn，不影响主流程。
pub(crate) fn emit_intermediate_round_state(
    app: &AppHandle,
    conv_id: &str,
    observable: &RoundState,
) {
    let payload = ChatRoundStatePayload {
        conversation_id: conv_id.to_string(),
        round: observable.round,
        elapsed_ms: observable.elapsed_ms,
        tokens_prompt: observable.tokens_prompt,
        tokens_completion: observable.tokens_completion,
        cached_tokens: observable.cached_tokens,
        retry_count: observable.retry_count,
    };
    if let Err(e) = app.emit("chat:round-state", payload) {
        tracing::warn!(
            target: "ice_paw.chat",
            "emit intermediate chat:round-state 失败: conv_id={}, err={}",
            conv_id,
            e
        );
    }
}

/// `chat:budget` 事件发射 — 会话级预算状态（前端 HUD / 续期 toast）。
/// 同 round-state 模式：同步 emit、失败仅 warn、无 spawn（事件 inline 纪律）。
#[allow(clippy::too_many_arguments)] // 与 payload 字段一一对应，聚合反而多一层搬运
pub(crate) fn emit_budget_state(
    app: &AppHandle,
    conv_id: &str,
    round: u32,
    cumulative_tokens: usize,
    effective_cap: usize,
    initial_cap: usize,
    renewal_index: u32,
    max_renewals: u32,
    renewed: bool,
) {
    let payload = ChatBudgetPayload {
        conversation_id: conv_id.to_string(),
        cumulative_tokens: cumulative_tokens as u64,
        effective_cap: effective_cap as u64,
        initial_cap: initial_cap as u64,
        renewal_index,
        max_renewals,
        renewed,
        round,
    };
    if let Err(e) = app.emit("chat:budget", payload) {
        tracing::warn!(
            target: "ice_paw.chat",
            "emit chat:budget 失败: conv_id={}, err={}",
            conv_id,
            e
        );
    }
}
