//! Token usage 合成：多轮工具调用场景下的 usage 聚合
//!
//! 从 `harness::loop_engine` 拆出。

use crate::infra::protocol::TokenUsage;

/// 合成最终 usage（多轮工具调用场景）
///
/// - `first_prompt_tokens`：首次出现的 prompt_tokens（整个 prompt 包含所有历史）
/// - `total_completion_tokens`：所有轮的 completion_tokens 之和
///
/// 如果整个流期间 provider 未返回任何 usage，则保留 `None` 让
/// cleanup 函数走 estimate_tokens 兜底路径。
pub(crate) fn synthesize_usage(
    first_prompt_tokens: Option<u32>,
    total_completion_tokens: u32,
    last_collected: Option<TokenUsage>,
) -> Option<TokenUsage> {
    match (first_prompt_tokens, last_collected) {
        (Some(p), Some(last)) => Some(TokenUsage {
            prompt_tokens: p,
            completion_tokens: total_completion_tokens,
            cached_tokens: last.cached_tokens,
        }),
        (Some(p), None) => Some(TokenUsage {
            prompt_tokens: p,
            completion_tokens: total_completion_tokens,
            cached_tokens: 0,
        }),
        (None, _) => None,
    }
}
