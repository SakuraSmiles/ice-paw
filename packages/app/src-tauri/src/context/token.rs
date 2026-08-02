//! L1 Context Budget — token 限额与估算工具
//!
//! 关键 API：
//! - `estimate_tokens(text)`     —— 文本 token 估算（CJK 1/字，英文 1/4 字节）
//! - `estimate_messages_tokens(messages)` —— 一组 ChatMessage 的总 token
//! - `ContextBudget`             —— 上下文预算配置（threshold 等）
//! - `compute_split_idx(messages, budget)` —— M1.5 摘要分割点
//!   （dev1 §4.2 双保险：保留最近 10 轮 + 尾部 token ≤ threshold×80%）

use crate::infra::protocol::ChatMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    /// 模型最大输入 token 数
    pub max_input_tokens: usize,
    /// 触发摘要压缩的阈值（当上下文超过此值时考虑压缩旧消息）
    pub summary_threshold_tokens: usize,
    /// 当消息数超过此值时触发工具调用结果裁剪；None 表示不裁剪
    pub tool_trim_threshold: Option<usize>,
    /// 裁剪时保留最近多少条消息
    pub trim_top_k: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_input_tokens: 128_000,
            summary_threshold_tokens: 35_000,
            tool_trim_threshold: Some(5),
            trim_top_k: 5,
        }
    }
}

/// Token 估算：英文 1 token ≈ 4 chars，CJK 1 token/char
pub fn estimate_tokens(text: &str) -> usize {
    let mut cjk = 0usize;
    let mut other = 0usize;
    for ch in text.chars() {
        // CJK 判断：覆盖 CJK Unified Ideographs / Extension A / CJK Compatibility
        // 以及 CJK Symbols and Punctuation / Halfwidth and Fullwidth Forms
        if matches!(
            ch,
            '\u{4E00}'..='\u{9FFF}'
                | '\u{3400}'..='\u{4DBF}'
                | '\u{F900}'..='\u{FAFF}'
                | '\u{3000}'..='\u{303F}'
                | '\u{FF00}'..='\u{FFEF}'
        ) {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    other.div_ceil(4) + cjk
}

/// M1.5: 估算一组 `ChatMessage` 的总 token 数
///
/// 仅取每条消息的 `content_text()`（拼接后的纯文本），不展开
/// 图片 / 工具块 —— 摘要阶段不关心原始多模态内容。
///
/// `messages` 为空时返回 0。空数组属于「正常」输入，不需要返回错误。
pub fn estimate_messages_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| estimate_tokens(&m.content_text()))
        .sum()
}

/// M1.5: 摘要分割点（双保险算法，源自 dev1 m1-review.md §4.2）
///
/// 给定一段历史消息，返回「该从哪里开始向前摘要」的索引。
/// 返回值满足 `0 <= split_idx <= messages.len()`，调用方用：
/// ```ignore
/// let split = compute_split_idx(&msgs, &budget);
/// let (older, recent) = msgs.split_at(split);
/// ```
///
/// 双保险策略（必须**两个条件都满足**才停止增长 keep_count）：
/// 1. **至少保留最近 10 轮**（即 20 条消息 `MIN_KEEP_MSGS = 20`），
///    防止短轮对话被过度压缩。
/// 2. **尾部 token ≤ threshold 的 80%**（且下界为 5000 tokens，
///    防止 threshold 太小导致永远压不满）。
///
/// 关键不变量：
/// - `split_idx == 0`  → 全部消息都被保留（不触发摘要或消息太少）
/// - `split_idx == messages.len()` → 整段都被压缩（极端长对话）
/// - 当 `messages.len() < MIN_KEEP_MSGS` → 直接返回 `0`（不摘要）
///
/// 该函数是**纯函数**：不读 DB、不改 ctx、不发请求；只算索引。
pub fn compute_split_idx(messages: &[ChatMessage], budget: &ContextBudget) -> usize {
    const MIN_KEEP_MSGS: usize = 20; // 10 轮 = 20 条 user/assistant 交替
    const MIN_KEEP_TOKENS: usize = 5_000; // 阈值下界，避免小 threshold 导致永远压不满

    if messages.len() < MIN_KEEP_MSGS {
        // 消息太少，全部保留（不摘要）
        return 0;
    }

    let max_tail_tokens = (budget.summary_threshold_tokens * 80 / 100).max(MIN_KEEP_TOKENS);

    let mut tail_tokens: usize = 0;
    let mut keep_count: usize = 0;

    // 从尾部向前累计，直到满足「两个条件都达成」
    for m in messages.iter().rev() {
        tail_tokens += estimate_tokens(&m.content_text());
        keep_count += 1;
        if tail_tokens >= max_tail_tokens && keep_count >= MIN_KEEP_MSGS {
            break;
        }
    }

    // 注意：极端情况 total < max_tail_tokens 时，keep_count 仍会增长到 messages.len()，
    // 此时 split_idx = 0（整段都保留，不触发摘要）—— 这与 budget 还没到阈值的语义一致。
    messages.len().saturating_sub(keep_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_empty_string_returns_zero() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_tokens_pure_english() {
        // 16 ASCII chars → (16 + 3) / 4 = 4 tokens
        assert_eq!(estimate_tokens("Hello, World!!!!"), 4);
    }

    #[test]
    fn estimate_tokens_pure_cjk() {
        // 3 CJK chars → 3 tokens
        assert_eq!(estimate_tokens("你好世"), 3);
    }

    #[test]
    fn estimate_tokens_mixed() {
        // "Hello" (5 ascii → (5+3)/4=2) + "你好" (2 cjk → 2) = 4
        assert_eq!(estimate_tokens("Hello你好"), 4);
    }

    // ---- M1.5: estimate_messages_tokens ----

    fn msgs(texts: &[&str]) -> Vec<ChatMessage> {
        texts
            .iter()
            .map(|t| ChatMessage::from_text("user", *t))
            .collect()
    }

    #[test]
    fn estimate_messages_tokens_empty_returns_zero() {
        assert_eq!(estimate_messages_tokens(&[]), 0);
    }

    #[test]
    fn estimate_messages_tokens_sums_each_message() {
        // "hi" = 1 token; "hello world" = 3 tokens; total = 4
        let v = msgs(&["hi", "hello world"]);
        assert_eq!(estimate_messages_tokens(&v), 4);
    }

    #[test]
    fn estimate_messages_tokens_handles_cjk() {
        // 2 CJK chars + 4 ASCII chars = 2 + 1 = 3 tokens
        let v = msgs(&["你好", "test"]);
        assert_eq!(estimate_messages_tokens(&v), 3);
    }

    // ---- M1.5: compute_split_idx ----

    #[test]
    fn compute_split_idx_short_history_returns_zero() {
        // 少于 20 条 → 不摘要，split_idx = 0
        let v = msgs(&["x"; 10]);
        let budget = ContextBudget::default();
        assert_eq!(compute_split_idx(&v, &budget), 0);
    }

    #[test]
    fn compute_split_idx_long_history_splits_correctly() {
        // 构造 40 条每条约 1000 token 的消息
        // total ≈ 40_000 tokens; max_tail_tokens = 35_000 * 0.8 = 28_000
        // 从尾部累加：累计到 >= 28_000 tokens 且 keep_count >= 20 时 break
        let long_text = "a".repeat(3996); // 3996/4 = 999 tokens，加上前缀约 1000
        let v: Vec<ChatMessage> = (0..40)
            .map(|_| ChatMessage::from_text("user", long_text.to_string()))
            .collect();
        let budget = ContextBudget::default();
        let split = compute_split_idx(&v, &budget);
        // split 应该 > 0（说明至少一部分消息会被摘要）
        assert!(split > 0, "长对话应触发摘要，split={split}");
        assert!(split < v.len(), "应至少保留尾部: split={split}");

        // 保留尾部消息数 >= 20
        assert!(v.len() - split >= 20, "应保留至少 20 条尾部消息");
    }

    #[test]
    fn compute_split_idx_respects_min_tail_floor() {
        // 即使 threshold 很小，MIN_KEEP_TOKENS=5000 兜底
        // 30 条每条约 200 token → total = 6000 tokens
        // budget.threshold=2000 → max_tail_tokens = max(2000*0.8=1600, 5000) = 5000
        // 从尾部累加：累计到 >= 5000 tokens 且 keep_count >= 20 时 break
        let v: Vec<ChatMessage> = (0..30)
            .map(|_| ChatMessage::from_text("user", "a".repeat(800))) // ~200 tokens
            .collect();
        let budget = ContextBudget {
            summary_threshold_tokens: 2_000, // 故意设小
            ..Default::default()
        };
        let split = compute_split_idx(&v, &budget);
        assert!(split > 0, "小 threshold 也应能分割, split={split}");
        assert!(v.len() - split >= 20, "至少保留 20 条尾部");
    }

    #[test]
    fn compute_split_idx_exact_boundary() {
        // 边界：恰好 20 条时，split_idx=0（不触发，< MIN_KEEP_MSGS 的下一条也不行）
        let v = msgs(&["x"; 20]);
        let budget = ContextBudget::default();
        assert_eq!(compute_split_idx(&v, &budget), 0);
    }
}