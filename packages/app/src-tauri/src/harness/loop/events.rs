//! Loop 事件发射：中间 round-state 事件。
//!
//! 从 `harness::loop_engine` 拆出。

use tauri::{AppHandle, Emitter};

use crate::harness::observable::RoundState;
use crate::infra::protocol::ChatRoundStatePayload;

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
