//! L2 Loop Budget — 三道熔断配置（W3.1 + W4.1 + W4.2）
//!
//! `LoopBudget` 把原本散落在 `commands/chat_loop.rs` 中的硬编码常量
//! （`MAX_TOOL_ROUNDS`、`MAX_ATTEMPTS` 等）集中起来，便于后续：
//!   - 按 agent 配置覆盖（未来读取 agent.json）
//!   - 运行时调整（用于测试 / 调试）
//!   - 单元测试中独立构造边界值
//!
//! W4.1: `stream_loop` 签名参数化已完成。
//! W4.2: `max_total_tokens` Token 预算终止已启用（默认 128_000）。
//!
//! 字段语义：
//!   - `max_tool_rounds`：工具调用最大轮数（防止无限循环）
//!   - `max_attempts`：每轮内最大重试次数（含首次尝试）
//!   - `stuck_threshold`：卡住检测阈值（M2.1 启用，默认 5；降低误判）
//!   - `max_total_tokens`：Token 预算上限（W4.2 启用，默认 128_000）

// ============================================================================
// 与 `commands/chat_loop.rs` 中原硬编码常量对齐的 pub const（短期兼容）
// ============================================================================

/// 工具调用循环的最大轮数（安全网：模型应自行决定何时停止）
/// 正常终止靠停滞检测（连续 stuck_threshold 轮无进展）；这个值只是极端兜底。
/// 可在 agent.yaml 中设置 tool_max_rounds 覆盖。
pub const MAX_TOOL_ROUNDS: u32 = 50;

/// 每轮内的最大尝试次数（含首次，即最多 3 次重试；原 `MAX_ATTEMPTS = 4`）
pub const MAX_ATTEMPTS: u32 = 4;

// ============================================================================
// LoopBudget — 三道熔断配置（W3.1 B1-1 + B1-2）
// ============================================================================

/// Loop 三道熔断配置：
/// 1. `max_tool_rounds`：工具调用最大轮数
/// 2. `max_attempts`：每轮内最大重试次数
/// 3. `max_total_tokens`：整次对话累计 token 预算（W4.2 启用，默认 128_000）
///
/// `stuck_threshold` 用于 M2.1 卡住检测（已启用，默认 5）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopBudget {
    /// 工具调用最大轮数（W3.1 默认 5，对应原 `MAX_TOOL_ROUNDS`）
    pub max_tool_rounds: u32,
    /// 每轮内最大尝试次数（W3.1 默认 4，对应原 `MAX_ATTEMPTS`）
    pub max_attempts: u32,
    /// 卡住检测阈值（M2.1 启用，默认 5；dev1 评审建议降低误判概率）
    pub stuck_threshold: u32,
    /// 整次对话累计 token 预算（W4.2 启用，默认 128_000）
    pub max_total_tokens: usize,
}

impl Default for LoopBudget {
    fn default() -> Self {
        Self {
            max_tool_rounds: MAX_TOOL_ROUNDS,
            max_attempts: MAX_ATTEMPTS,
            // M2.1: 默认阈值从 3 改为 5（dev1 评审建议）
            // 理由：阈值太小会误判正常多步推理为停滞；5 轮无进展已可确信是 LLM 卡住
            stuck_threshold: 5,
            max_total_tokens: 128_000,
        }
    }
}

// ============================================================================
// 单元测试（W3.1）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 兼容旧硬编码常量：保证 LoopBudget::default() 与原常量一致
    /// （防止未来误改默认值导致行为变化时无人察觉）
    #[test]
    fn test_loop_budget_default_matches_legacy_consts() {
        let budget = LoopBudget::default();
        assert_eq!(budget.max_tool_rounds, 50);
        assert_eq!(budget.max_attempts, MAX_ATTEMPTS);
    }
}