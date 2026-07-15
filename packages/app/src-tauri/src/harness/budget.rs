//! L2 Loop Budget — 三道熔断配置（W3.1）
//!
//! `LoopBudget` 把原本散落在 `commands/chat_loop.rs` 中的硬编码常量
//! （`MAX_TOOL_ROUNDS`、`MAX_ATTEMPTS` 等）集中起来，便于后续：
//! - 按 agent 配置覆盖（未来读取 agent.json）
//! - 运行时调整（用于测试 / 调试）
//! `stream_loop` 签名参数化（W4.1 已完成）并启用 Token 预算终止（W4.2）。
//!
//! 字段语义：
//! - `max_tool_rounds`：工具调用最大轮数（防止无限循环）
//! - `max_attempts`：每轮内最大重试次数（含首次尝试）
//! - `stuck_threshold`：卡住检测阈值（P2 启用，当前未使用）
//! - `max_total_tokens`：Token 预算上限（W4.2 启用，默认 128_000）
//! - `max_tool_rounds`：工具调用最大轮数（防止无限循环）
//! - `max_attempts`：每轮内最大重试次数（含首次尝试）
//! - `stuck_threshold`：卡住检测阈值（P2 启用，当前未使用）
//! - `max_total_tokens`：Token 预算上限（W4.3 启用，默认无限）

// ============================================================================
// 与 `commands/chat_loop.rs` 中原硬编码常量对齐的 pub const（短期兼容）
// ============================================================================

/// 工具调用循环的最大轮数（原 `commands/chat_loop.rs` 中的 `MAX_TOOL_ROUNDS = 5`）
pub const MAX_TOOL_ROUNDS: u32 = 5;

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
/// `stuck_threshold` 用于 P2 卡住检测（暂未启用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopBudget {
    /// 工具调用最大轮数（W3.1 默认 5，对应原 `MAX_TOOL_ROUNDS`）
    pub max_tool_rounds: u32,
    /// 每轮内最大尝试次数（W3.1 默认 4，对应原 `MAX_ATTEMPTS`）
    pub max_attempts: u32,
    /// 卡住检测阈值（P2 启用，默认 3）
    pub stuck_threshold: u32,
    /// 整次对话累计 token 预算（W4.2 启用，默认 128_000）
    pub max_total_tokens: usize,
}

impl Default for LoopBudget {
    fn default() -> Self {
        Self {
            max_tool_rounds: MAX_TOOL_ROUNDS,
            max_attempts: MAX_ATTEMPTS,
            stuck_threshold: 3,
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

    #[test]
    fn test_loop_budget_default() {
        let budget = LoopBudget::default();
        assert_eq!(budget.max_tool_rounds, 5);
        assert_eq!(budget.max_attempts, 4);
        assert_eq!(budget.stuck_threshold, 3);
        assert_eq!(budget.max_total_tokens, 128_000);
    }

    #[test]
    fn test_loop_budget_custom() {
        let budget = LoopBudget {
            max_tool_rounds: 10,
            max_attempts: 2,
            stuck_threshold: 5,
            max_total_tokens: 100_000,
        };
        assert_eq!(budget.max_tool_rounds, 10);
        assert_eq!(budget.max_attempts, 2);
        assert_eq!(budget.stuck_threshold, 5);
        assert_eq!(budget.max_total_tokens, 100_000);
    }

    /// 兼容旧硬编码常量：保证 LoopBudget::default() 与原常量一致
    /// （防止未来误改默认值导致行为变化时无人察觉）
    #[test]
    fn test_loop_budget_default_matches_legacy_consts() {
        let budget = LoopBudget::default();
        assert_eq!(budget.max_tool_rounds, MAX_TOOL_ROUNDS);
        assert_eq!(budget.max_attempts, MAX_ATTEMPTS);
    }
}