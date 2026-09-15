//! Loop 事件发射：中间 round-state / budget 事件。
//!
//! 从 `harness::loop_engine` 拆出；S6 起走 [`LoopEmitter`] 出口（不再依赖
//! `tauri::AppHandle`），失败仅 warn，不影响主流程。

use crate::harness::observable::RoundState;
use crate::infra::protocol::{
    ChatBudgetPayload, ChatRoundStatePayload, ChatRoundsRenewedPayload,
};

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

/// `chat:rounds-renewed` 事件发射 — 工具轮数自动续期 toast（前端「已达 N 轮，
/// 自动续期 i/max → 新上限 M 轮」）。同 budget 模式：同步 emit、失败仅 warn、
/// 无 spawn（事件 inline 纪律）；瞬态 UI 事件不入 session-event-log。
#[allow(clippy::too_many_arguments)] // 与 payload 字段一一对应（同 emit_budget_state），聚合反而多一层搬运
pub(crate) fn emit_rounds_renewed(
    emitter: &dyn crate::harness::r#loop::emitter::LoopEmitter,
    conv_id: &str,
    message_id: &str,
    round: u32,
    renewal_index: u32,
    max_renewals: u32,
    initial_max_rounds: u32,
    effective_max_rounds: u32,
) {
    let payload = ChatRoundsRenewedPayload {
        conversation_id: conv_id.to_string(),
        message_id: message_id.to_string(),
        round,
        renewal_index,
        max_renewals,
        initial_max_rounds,
        effective_max_rounds,
    };
    crate::harness::r#loop::emitter::emit_ser(emitter, "chat:rounds-renewed", &payload);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 收集型测试 emitter：只记事件名与 payload（CollectEmitter 的最小形态）。
    #[derive(Default)]
    struct SinkEmitter(std::sync::Mutex<Vec<(String, serde_json::Value)>>);

    impl crate::harness::r#loop::emitter::LoopEmitter for SinkEmitter {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.0
                .lock()
                .expect("sink lock")
                .push((event.to_string(), payload));
        }
        fn on_loop_exit(&self) {}
    }

    /// ① 轻量兜底：rounds-renewed payload 组装全字段断言（e2e 场景 6c 走全链路，
    /// 此处单测直指字段错位——参数序长，swap round/initial_max_rounds 之类 e2e
    /// 报错点位比单测晦涩）。
    #[test]
    fn rounds_renewed_payload_assembles_all_fields() {
        let sink = SinkEmitter::default();
        emit_rounds_renewed(&sink, "conv-1", "msg-9", 50, 1, 4, 50, 100);
        let events = sink.0.lock().expect("sink lock");
        assert_eq!(events.len(), 1);
        let (name, p) = &events[0];
        assert_eq!(name, "chat:rounds-renewed");
        assert_eq!(p["conversation_id"], "conv-1");
        assert_eq!(p["message_id"], "msg-9");
        assert_eq!(p["round"], 50, "触顶轮数");
        assert_eq!(p["renewal_index"], 1);
        assert_eq!(p["max_renewals"], 4);
        assert_eq!(p["initial_max_rounds"], 50);
        assert_eq!(p["effective_max_rounds"], 100);
    }
}
