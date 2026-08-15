//! L2 Loop Budget — 三道熔断配置（W3.1 + W4.1 + W4.2）
//!
//! `LoopBudget` 把原本散落在 `commands/chat_loop.rs` 中的硬编码常量
//! （`MAX_TOOL_ROUNDS`、`MAX_ATTEMPTS` 等）集中起来，便于后续：
//!   - 按 agent 配置覆盖（未来读取 agent.json）
//!   - 运行时调整（用于测试 / 调试）
//!   - 单元测试中独立构造边界值
//!
//! W4.1: `stream_loop` 签名参数化已完成。
//! W4.2: `max_total_tokens` Token 预算终止已启用（默认按上下文窗口自适应 3×）。
//!
//! 字段语义：
//!   - `max_tool_rounds`：工具调用最大轮数（防止无限循环）
//!   - `max_attempts`：每轮内最大重试次数（含首次尝试）
//!   - `stuck_threshold`：卡住检测阈值（M2.1 启用，默认 5；降低误判）
//!   - `max_total_tokens`：Token 预算上限（默认 3× 上下文窗口，chat_cmd model-aware 兜底；agent.yaml 可覆盖）
//!   - `max_budget_renewals` / `max_round_renewals`：B1 自动续期额度——触顶时若额度
//!     未尽则 +初始上限续跑（合法长任务不误杀），额度用尽才真停。失控循环不靠它
//!     兜底（stuck_detect 独立熔断），额度有界保证总开销封顶 = 初始 × (1+额度)。

// ============================================================================
// 与 `commands/chat_loop.rs` 中原硬编码常量对齐的 pub const（短期兼容）
// ============================================================================

/// 工具调用循环的最大轮数（安全网：模型应自行决定何时停止）
/// 正常终止靠停滞检测（连续 stuck_threshold 轮无进展）；这个值只是极端兜底。
/// 可在 agent.yaml 中设置 tool_max_rounds 覆盖。
pub const MAX_TOOL_ROUNDS: u32 = 50;

/// 每轮内的最大尝试次数（含首次，即最多 3 次重试；原 `MAX_ATTEMPTS = 4`）
pub const MAX_ATTEMPTS: u32 = 4;

/// B1 自动续期默认额度：预算/轮数触顶时可自动续期的次数（每次 +初始上限），
/// 总开销封顶 = 初始上限 × (1 + 此值)。默认路径（model-aware 兜底）用此值；
/// agent.yaml 显式 max_total_tokens / tool_max_rounds → chat_cmd 置 0
///（用户拍板的显式硬上限不被静默突破）。
pub const DEFAULT_AUTO_RENEWALS: u32 = 2;

// ============================================================================
// LoopBudget — 三道熔断配置（W3.1 B1-1 + B1-2）
// ============================================================================

/// Loop 三道熔断配置：
/// 1. `max_tool_rounds`：工具调用最大轮数
/// 2. `max_attempts`：每轮内最大重试次数
/// 3. `max_total_tokens`：整次对话累计 token 预算（W4.2 启用，默认 3× 上下文窗口）
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
    /// 整次对话累计 token 预算（默认 3× 上下文窗口，由 chat_cmd model-aware 兜底；
    /// 此处 `1_000_000` 仅作 `LoopBudget::default()` 的库级兜底，send_message 路径不走它）。
    /// Σ(prompt_i+completion_i) = provider 真实毛成本（历史每轮重发、被重新计费）；
    /// loop_engine 基于【本轮】usage 累加（避免间歇缺失时旧值重复累加致虚高）。
    /// agent.yaml 的 max_total_tokens 可显式覆盖；撞预算由守卫对称清场，不再卡死。
    pub max_total_tokens: usize,
    /// B1：`max_total_tokens` 触顶时可自动续期的次数（每次 +初始上限）。0 = 硬上限即停。
    pub max_budget_renewals: u32,
    /// B1：`max_tool_rounds` 触顶时可自动续期的次数（语义同上）。0 = 硬上限即停。
    pub max_round_renewals: u32,
}

impl Default for LoopBudget {
    fn default() -> Self {
        Self {
            max_tool_rounds: MAX_TOOL_ROUNDS,
            max_attempts: MAX_ATTEMPTS,
            stuck_threshold: 5,
            max_total_tokens: 1_000_000,
            max_budget_renewals: DEFAULT_AUTO_RENEWALS,
            max_round_renewals: DEFAULT_AUTO_RENEWALS,
        }
    }
}

// ============================================================================
// 单元测试（W3.1）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::protocol::TokenUsage;

    /// 兼容旧硬编码常量：保证 LoopBudget::default() 与原常量一致
    /// （防止未来误改默认值导致行为变化时无人察觉）
    #[test]
    fn test_loop_budget_default_matches_legacy_consts() {
        let budget = LoopBudget::default();
        assert_eq!(budget.max_tool_rounds, 50);
        assert_eq!(budget.max_attempts, MAX_ATTEMPTS);
    }

    /// 验证：默认预算（1_000_000）不会意外触发终止
    #[test]
    fn test_budget_not_exceeded_with_default() {
        let budget = LoopBudget::default();
        assert_eq!(budget.max_total_tokens, 1_000_000);
        // 模拟一个 round 使用了 5000 tokens → 远低于 1_000_000
        let cumulative_tokens: usize = 5_000;
        let exceeded = budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
        assert!(!exceeded, "默认预算不应在 5000 tokens 时触发终止");
    }

    /// 验证：自定义小预算在超限时正确标记 exceeded
    #[test]
    fn test_budget_exceeded_with_small_limit() {
        let budget = LoopBudget {
            max_tool_rounds: 5,
            max_attempts: 4,
            stuck_threshold: 3,
            max_total_tokens: 1_000,
            ..LoopBudget::default()
        };
        // 模拟 round 1 用了 800 tokens，round 2 累计到 1600 → 超过 1000
        let mut cumulative_tokens: usize = 800;
        let exceeded_1 = budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
        assert!(!exceeded_1, "800 tokens 不应超过 1000 预算");

        cumulative_tokens += 800; // 1600
        let exceeded_2 = budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
        assert!(exceeded_2, "1600 tokens 应超过 1000 预算");
    }

    /// 验证：usize::MAX 预算永远不触发终止（无限模式）
    #[test]
    fn test_budget_unlimited_never_exceeds() {
        let budget = LoopBudget {
            max_tool_rounds: 5,
            max_attempts: 4,
            stuck_threshold: 3,
            max_total_tokens: usize::MAX,
            ..LoopBudget::default()
        };
        // 模拟极端大的累计值
        let cumulative_tokens: usize = usize::MAX - 1;
        let exceeded = budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
        assert!(!exceeded, "usize::MAX 预算永远不应触发终止");
    }

    /// 验证：TokenUsage 累加准确性
    #[test]
    fn test_token_accumulation_accuracy() {
        let u1 = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            cached_tokens: 10,
        };
        let u2 = TokenUsage {
            prompt_tokens: 200,
            completion_tokens: 80,
            cached_tokens: 20,
        };
        let mut cumulative: usize = 0;
        cumulative += u1.prompt_tokens as usize + u1.completion_tokens as usize;
        cumulative += u2.prompt_tokens as usize + u2.completion_tokens as usize;
        assert_eq!(cumulative, 430, "累计应为 100+50+200+80=430");
    }

    /// B1: 预算续期总量有界——每次触顶 +初始上限，N 次续期后 = 初始 × (N+1)
    #[test]
    fn test_budget_renewal_ceiling_is_bounded() {
        let budget = LoopBudget {
            max_total_tokens: 1_000,
            max_budget_renewals: 2,
            ..LoopBudget::default()
        };
        // 复现 loop_engine 的续期语义：触顶 → +初始上限，共 max_budget_renewals 次
        let mut effective = budget.max_total_tokens;
        let mut renewals: u32 = 0;
        while renewals < budget.max_budget_renewals {
            effective = effective.saturating_add(budget.max_total_tokens);
            renewals += 1;
        }
        assert_eq!(effective, 3_000, "2 次续期后总上限应为初始 × 3");
        assert_eq!(renewals, budget.max_budget_renewals, "续期次数不可超出额度");
        // 额度用尽后再触顶 → 无第四次续期，走 budget_exceeded 终止
        assert!(effective < 3_001);
    }

    /// B1: 额度 0 = 显式硬上限（agent.yaml 显式 max_total_tokens 的语义），触顶即停
    #[test]
    fn test_budget_renewal_zero_means_hard_cap() {
        let budget = LoopBudget {
            max_total_tokens: 1_000,
            max_budget_renewals: 0,
            ..LoopBudget::default()
        };
        let mut effective = budget.max_total_tokens;
        let mut renewals: u32 = 0;
        while renewals < budget.max_budget_renewals {
            effective = effective.saturating_add(budget.max_total_tokens);
            renewals += 1;
        }
        // 零额度下循环体一次都不进：上限不变、仍为初始值
        assert_eq!(effective, 1_000, "零额度不应抬升上限");
        assert_eq!(renewals, 0);
    }

    /// B1: 轮数续期默认额度与预算一致（2 次 → 总轮数 = 初始 × 3），可独立置 0
    #[test]
    fn test_round_renewal_defaults_align_with_budget() {
        let budget = LoopBudget::default();
        assert_eq!(budget.max_budget_renewals, DEFAULT_AUTO_RENEWALS);
        assert_eq!(budget.max_round_renewals, DEFAULT_AUTO_RENEWALS);
        // 轮数上限同步封顶：初始 50 × (1+2) = 150 轮
        let mut effective = budget.max_tool_rounds;
        for _ in 0..budget.max_round_renewals {
            effective = effective.saturating_add(budget.max_tool_rounds);
        }
        assert_eq!(effective, 150);
    }
}