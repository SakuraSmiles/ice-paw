//! Loop 事件发射：中间 round-state / budget 事件。
//!
//! 从 `harness::loop_engine` 拆出；S6 起走 [`LoopEmitter`] 出口（不再依赖
//! `tauri::AppHandle`），失败仅 warn，不影响主流程。

use crate::harness::observable::RoundState;
use crate::infra::protocol::{ChatBudgetPayload, ChatRoundStatePayload};

/// 中间 round-state 事件发射 — 供前端 ChatStatusBar 实时显示进度。
pub(crate) fn emit_intermediate_round_state(
    emitter: &dyn crate::harness::r#loop::emitter::LoopEmitter,
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
    crate::harness::r#loop::emitter::emit_ser(emitter, "chat:round-state", &payload);
}

/// `chat:budget` 事件发射 — 会话级预算状态（前端 HUD / 续期 toast）。
/// 同 round-state 模式：同步 emit、失败仅 warn、无 spawn（事件 inline 纪律）。
/// `miss_hint`：本轮全 miss 归因 slug（仅常规更新轮带；续期/终态轮传 None）。
#[allow(clippy::too_many_arguments)] // 与 payload 字段一一对应，聚合反而多一层搬运
pub(crate) fn emit_budget_state(
    emitter: &dyn crate::harness::r#loop::emitter::LoopEmitter,
    conv_id: &str,
    round: u32,
    cumulative_tokens: usize,
    cumulative_cached: usize,
    cumulative_prompt: usize,
    effective_cap: usize,
    initial_cap: usize,
    renewal_index: u32,
    max_renewals: u32,
    renewed: bool,
    miss_hint: Option<&[String]>,
) {
    let payload = ChatBudgetPayload {
        conversation_id: conv_id.to_string(),
        cumulative_tokens: cumulative_tokens as u64,
        cumulative_cached_tokens: cumulative_cached as u64,
        cumulative_prompt_tokens: cumulative_prompt as u64,
        effective_cap: effective_cap as u64,
        initial_cap: initial_cap as u64,
        renewal_index,
        max_renewals,
        renewed,
        round,
        miss_hint: miss_hint
            .filter(|s| !s.is_empty())
            .map(|s| s.to_vec()),
    };
    crate::harness::r#loop::emitter::emit_ser(emitter, "chat:budget", &payload);
}
