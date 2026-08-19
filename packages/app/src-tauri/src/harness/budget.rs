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
//!     兜底（stuck_detect 独立熔断），额度有界保证总开销封顶 = 初始 × (1+额度)
//!     （计费口径，见 [`billed_tokens`]）。

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
/// 总开销封顶 = 初始上限 × (1 + 此值)（计费口径）。默认路径（model-aware
/// 兜底）用此值；agent.yaml 显式 max_total_tokens / tool_max_rounds →
/// chat_cmd 置 0（用户拍板的显式硬上限不被静默突破）。
///
/// 2→4（预算诚实化）：预算已改按缓存折扣计量（高命中时同窗口容纳 4~10×
/// 轮次，正常路径很难触顶），但极端未命中（首轮冷缓存/长任务前几轮）仍按
/// 近全价燃烧——4 次额度保证冷启动长任务不被过早真停；失控循环不靠它兜底
///（stuck_detect 独立熔断），封顶 = 初始 × 5 仍线性有界。
pub const DEFAULT_AUTO_RENEWALS: u32 = 4;

// ============================================================================
// 预算计量（缓存折扣）——预算按「计费口径」而非毛成本累计
// ============================================================================

/// 上下文缓存计价折扣分母：命中部分按 1/10 计入预算。
///
/// 取 1/10 是各 provider 缓存定价的**最贵档**（Anthropic cache read 0.1×、
/// MiniMax 前缀缓存 ~输入价 10%、DeepSeek 1/10~1/50）——按最贵档折算即保守
/// 口径：宁可高估成本早续期，不可低估成本放任熔断失真。生产实证命中 96% 的
/// 长任务曾按全价计量被提前熔断（毛成本 9M 触顶 → 计费口径实际 ≈1.5M）。
pub const CACHE_HIT_DISCOUNT_DIVISOR: u64 = 10;

/// 预算「计费口径」token 数：未命中全价 + 命中按 [`CACHE_HIT_DISCOUNT_DIVISOR`]
/// 折扣 + 输出全价。
///
/// 入参须为规范语义 TokenUsage（prompt 含命中、cached ≤ prompt；适配层归一 +
/// `into_canonical` 自愈保证）；`cached.min(prompt)` 为末道防御——脏数据下宁可
/// 全价不可负数。整除截断每轮 ≤9 token，相对十万级 cached 可忽略。
pub fn billed_tokens(prompt: u64, cached: u64, completion: u64) -> u64 {
    let cached = cached.min(prompt);
    (prompt - cached) + cached / CACHE_HIT_DISCOUNT_DIVISOR + completion
}

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
    /// 累计口径 = [`billed_tokens`] 计费口径（缓存命中按 1/10 折扣——provider 对
    /// 命中部分只收 1/10~1/50 费用，按毛成本 Σ(prompt+completion) 计量会提前熔断
    /// 高命中的长任务）；loop_engine 基于【本轮】usage 累加（避免间歇缺失时旧值
    /// 重复累加致虚高）。
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
        let exceeded =
            budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
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
        let exceeded_1 =
            budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
        assert!(!exceeded_1, "800 tokens 不应超过 1000 预算");

        cumulative_tokens += 800; // 1600
        let exceeded_2 =
            budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
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
        let exceeded =
            budget.max_total_tokens != usize::MAX && cumulative_tokens > budget.max_total_tokens;
        assert!(!exceeded, "usize::MAX 预算永远不应触发终止");
    }

    /// 验证：TokenUsage 累加准确性（计费口径——loop_engine 累加点同款公式）
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
        cumulative += billed_tokens(
            u1.prompt_tokens as u64,
            u1.cached_tokens as u64,
            u1.completion_tokens as u64,
        ) as usize;
        cumulative += billed_tokens(
            u2.prompt_tokens as u64,
            u2.cached_tokens as u64,
            u2.completion_tokens as u64,
        ) as usize;
        // (100-10+1+50) + (200-20+2+80) = 141 + 262 = 403
        assert_eq!(cumulative, 403, "计费口径累计应为 (90+1+50)+(180+2+80)=403");
    }

    /// billed_tokens：无缓存退化 = 全价（Ollama / mock 路径行为不变式）
    #[test]
    fn billed_tokens_full_price_when_no_cache() {
        assert_eq!(billed_tokens(100_000, 0, 5_000), 105_000);
        assert_eq!(billed_tokens(0, 0, 0), 0);
    }

    /// billed_tokens：命中按 1/10 折扣（生产被熔断 turn 末轮量级——毛 405k → 计费 63k）
    #[test]
    fn billed_tokens_cache_hit_discounted_tenth() {
        let prompt = 400_000;
        let cached = 380_000; // 命中 95%
        let completion = 5_000;
        // 未命中 20k 全价 + 命中 38k（1/10） + 输出 5k
        assert_eq!(billed_tokens(prompt, cached, completion), 20_000 + 38_000 + 5_000);
    }

    /// billed_tokens：cached > prompt 钳制（脏数据末道防御，宁可全价不可下溢）
    #[test]
    fn billed_tokens_cached_exceeds_prompt_clamped() {
        // 未归一脏数据：cached=400 但 prompt=100 → 按 prompt 全为命中算也不下溢
        let billed = billed_tokens(100, 400, 10);
        assert_eq!(billed, 100 / CACHE_HIT_DISCOUNT_DIVISOR + 10);
    }

    /// billed_tokens：整除截断边界（cached=9 → +0；cached=19 → +1）
    #[test]
    fn billed_tokens_truncation_boundary() {
        assert_eq!(billed_tokens(9, 9, 0), 0);
        assert_eq!(billed_tokens(19, 19, 0), 1);
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

    /// B1: 轮数续期默认额度与预算一致（总轮数 = 初始 × (1+额度)），可独立置 0
    #[test]
    fn test_round_renewal_defaults_align_with_budget() {
        let budget = LoopBudget::default();
        assert_eq!(budget.max_budget_renewals, DEFAULT_AUTO_RENEWALS);
        assert_eq!(budget.max_round_renewals, DEFAULT_AUTO_RENEWALS);
        // 轮数上限同步封顶：初始 × (1+额度)——用表达式断言，杜绝常量调整后再硬编码
        let mut effective = budget.max_tool_rounds;
        for _ in 0..budget.max_round_renewals {
            effective = effective.saturating_add(budget.max_tool_rounds);
        }
        assert_eq!(
            effective,
            MAX_TOOL_ROUNDS * (1 + DEFAULT_AUTO_RENEWALS)
        );
    }
}
