//! L1 Context Budget — token 限额与估算工具
//!
//! 关键 API：
//! - `estimate_tokens(text)`     —— 文本 token 估算（CJK 1/字，英文 1/4 字节）
//! - `estimate_block_tokens(b)`  —— 单个 ContentBlock 的 token（覆盖工具/图片/思考块）
//! - `estimate_message_tokens(m)`—— 单条 ChatMessage 的 token（block 之和 + 结构开销）
//! - `estimate_messages_tokens(messages)` —— 一组 ChatMessage 的总 token
//! - `ContextBudget`             —— 上下文预算配置（threshold 等）
//! - `compute_split_idx(messages, budget)` —— M1.5 摘要分割点
//!   （dev1 §4.2 双保险：保留最近 10 轮 + 尾部 token ≤ threshold×80%）

use crate::infra::protocol::{ChatMessage, ContentBlock};

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

// ---- block / message 级估算（覆盖全部 ContentBlock 类型）----
//
// 历史 token 估算必须覆盖工具块：一次失败的 tool_result 可能是数百 KB 文本，
// 若只数 Text 块（旧 `content_text()` 路径）会把 tool_result 记成 0 token，
// 导致 MemoryStage 误判「还没到摘要阈值」、窗口被静默撑爆。

/// 工具块 / 结果块除正文外的 JSON 框架开销（type / id / name / tool_use_id 等）。
/// token 估算无需精确，给个保守上界避免低估。
const TOOL_BLOCK_OVERHEAD: usize = 8;
/// 每条消息的角色 / content 数组结构固定开销。
const MESSAGE_OVERHEAD: usize = 4;
/// 图片 token 估算下限（对齐 Anthropic 图片最小计费约 85 token）。
const IMAGE_TOKEN_FLOOR: usize = 85;
/// 图片 token 估算封顶（避免巨幅 base64 失真）。
const IMAGE_TOKEN_CAP: usize = 4_000;

/// 估算单个 [`ContentBlock`] 的 token 数（block 级，覆盖全部变体）。
///
/// - `Text`     → 文本 token
/// - `ToolUse`  → input(JSON 字符串) + 结构开销
/// - `ToolResult` → content + 结构开销
/// - `Thinking` → thinking 文本
/// - `Image`    → 按 base64 解码字节数粗估（无像素尺寸可读，仅代理；历史里图片罕见）
pub fn estimate_block_tokens(block: &ContentBlock) -> usize {
    match block {
        ContentBlock::Text { text } => estimate_tokens(text),
        ContentBlock::ToolUse { input, .. } => estimate_tokens(input) + TOOL_BLOCK_OVERHEAD,
        ContentBlock::ToolResult { content, .. } => estimate_tokens(content) + TOOL_BLOCK_OVERHEAD,
        ContentBlock::Thinking { thinking, .. } => estimate_tokens(thinking),
        ContentBlock::Image { data, .. } => {
            // base64 解码字节数 ≈ len * 3/4；按每 200 字节约 1 token 粗估。
            // Anthropic 实际按像素 (w*h)/750 计，此处无尺寸可读，仅为量级代理。
            let decoded = (data.len() * 3) / 4;
            (decoded / 200).max(IMAGE_TOKEN_FLOOR).min(IMAGE_TOKEN_CAP)
        }
    }
}

/// 估算单条 [`ChatMessage`] 的 token 数 = 各 block 之和 + 消息结构开销。
pub fn estimate_message_tokens(m: &ChatMessage) -> usize {
    m.content.iter().map(estimate_block_tokens).sum::<usize>() + MESSAGE_OVERHEAD
}

/// 估算一组 `ChatMessage` 的总 token 数（block 级，覆盖工具 / 图片 / 思考块）。
///
/// `messages` 为空时返回 0。空数组属于「正常」输入，不需要返回错误。
pub fn estimate_messages_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

/// 按 token 硬上限裁剪历史：从尾部保留尽量多的消息，使保留部分 token 总和 ≤ `max_tokens`。
///
/// 返回保留的尾部切片 `&history[keep_from..]`。若最近一条消息本身就超 `max_tokens`，
/// 仍至少保留它（无法再裁）。**不保证协议合规**——可能在 `tool_use` / `tool_result`
/// 边界切断，调用方应随后 [`crate::context::history::sanitize_history`] 清理孤儿。
///
/// 该函数是纯函数：只读切片、不算 DB、不发请求。
pub fn trim_history_to_budget(history: &[ChatMessage], max_tokens: usize) -> &[ChatMessage] {
    if history.is_empty() {
        return history;
    }
    let mut acc: usize = 0;
    let mut keep_from = history.len(); // 初始：空保留
    for (i, m) in history.iter().enumerate().rev() {
        let t = estimate_message_tokens(m);
        // 加入此条会超预算，且其后已有保留条目 → 停在此条之前
        if i + 1 < history.len() && acc + t > max_tokens {
            keep_from = i + 1;
            break;
        }
        acc += t;
        keep_from = i;
    }
    &history[keep_from..]
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
        tail_tokens += estimate_message_tokens(m);
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
        // "hi" = 1 token; "hello world" = 3 tokens; 每条 +MESSAGE_OVERHEAD(4)
        // → (1+4) + (3+4) = 12
        let v = msgs(&["hi", "hello world"]);
        assert_eq!(estimate_messages_tokens(&v), 12);
    }

    #[test]
    fn estimate_messages_tokens_handles_cjk() {
        // "你好" = 2 CJK = 2; "test" = 4 ascii → div_ceil(4,4)=1; 各 +4 overhead
        // → (2+4) + (1+4) = 11
        let v = msgs(&["你好", "test"]);
        assert_eq!(estimate_messages_tokens(&v), 11);
    }

    // ---- block 级估算（覆盖工具 / 图片 / 思考块）----

    #[test]
    fn estimate_block_tokens_text() {
        // "test" = 4 ascii → 1 token
        assert_eq!(
            estimate_block_tokens(&ContentBlock::text("test")),
            1
        );
    }

    #[test]
    fn estimate_block_tokens_tool_use_counts_input_plus_overhead() {
        // input "{\"a\":1}" = 7 ascii → div_ceil(7,4)=2; +TOOL_BLOCK_OVERHEAD(8) = 10
        let b = ContentBlock::ToolUse {
            id: "call_1".into(),
            name: "run_command".into(),
            input: "{\"a\":1}".into(),
        };
        assert_eq!(estimate_block_tokens(&b), 2 + TOOL_BLOCK_OVERHEAD);
    }

    #[test]
    fn estimate_block_tokens_tool_result_counts_content_plus_overhead() {
        // 100 个 'a' → div_ceil(100,4)=25; +overhead(8) = 33
        let b = ContentBlock::ToolResult {
            tool_use_id: "call_1".into(),
            content: "a".repeat(100),
            is_error: Some(true),
        };
        assert_eq!(
            estimate_block_tokens(&b),
            (100usize).div_ceil(4) + TOOL_BLOCK_OVERHEAD
        );
    }

    #[test]
    fn estimate_block_tokens_thinking() {
        // thinking 文本应被计入（4 ascii → 1 token）
        let b = ContentBlock::Thinking {
            thinking: "test".into(),
            signature: None,
        };
        assert_eq!(estimate_block_tokens(&b), 1);
    }

    #[test]
    fn estimate_block_tokens_image_floor_and_nonzero() {
        // 空数据 → 解码 0 字节 → 0/200=0，但下限 IMAGE_TOKEN_FLOOR 兜底
        let small = ContentBlock::image("", "image/png");
        assert_eq!(estimate_block_tokens(&small), IMAGE_TOKEN_FLOOR);
        assert!(IMAGE_TOKEN_FLOOR > 0);

        // 巨幅 base64（模拟 5MB 解码量）应被封顶
        let huge = ContentBlock::image("x".repeat(8_000_000), "image/png");
        assert_eq!(estimate_block_tokens(&huge), IMAGE_TOKEN_CAP);
    }

    #[test]
    fn estimate_message_tokens_counts_tool_blocks_not_just_text() {
        // 关键回归：一条带 tool_result 的消息，旧 content_text() 路径会记 0；
        // block 级估算必须 > 0。content 400 ascii → 100 token + overhead 8 + msg overhead 4
        let m = ChatMessage {
            role: "user".into(),
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content: "a".repeat(400),
                is_error: Some(true),
            }],
        };
        let est = estimate_message_tokens(&m);
        assert!(est > 0, "tool_result 必须被计入，实际 {est}");
        // 400/4=100 + TOOL_BLOCK_OVERHEAD(8) + MESSAGE_OVERHEAD(4) = 112
        assert_eq!(est, 100 + TOOL_BLOCK_OVERHEAD + MESSAGE_OVERHEAD);
    }

    #[test]
    fn estimate_messages_tokens_sums_tool_heavy_history() {
        // 旧路径：tool_result 全记 0 → 总计仅 Text 消息；
        // 新路径：工具块必须贡献 token，使总计数显著大于纯文本。
        let tool_msg = ChatMessage {
            role: "user".into(),
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "c1".into(),
                content: "x".repeat(4000), // 1000 token
                is_error: None,
            }],
        };
        let text_msg = ChatMessage::from_text("assistant", "ok"); // 1 token + 4
        let total = estimate_messages_tokens(&[tool_msg, text_msg]);
        // tool_msg ≈ 1000+8+4=1012; text_msg = 1+4=5 → 1017
        assert!(total > 1000, "工具密集历史应被如实计为大体积，实际 {total}");
    }

    // ---- trim_history_to_budget（Phase 1 token 窗口）----

    #[test]
    fn trim_history_empty_returns_empty() {
        assert!(trim_history_to_budget(&[], 1000).is_empty());
    }

    #[test]
    fn trim_history_all_fits_returns_all() {
        // 3 条小消息，预算充足 → 全保留，顺序不变
        let v = msgs(&["a", "b", "c"]);
        let kept = trim_history_to_budget(&v, 10_000);
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[0].content_text(), "a");
        assert_eq!(kept[2].content_text(), "c");
    }

    #[test]
    fn trim_history_drops_oldest_to_fit_budget() {
        // 每条 "aaaa"(4 ascii → 1 token) + MESSAGE_OVERHEAD(4) = 5 token；5 条 = 25
        // 预算 11 → 最多保留 ceil 不足 3 条（3 条=15>11, 2 条=10≤11）→ 保留最后 2 条
        let v = msgs(&["aaaa", "bbbb", "cccc", "dddd", "eeee"]);
        let kept = trim_history_to_budget(&v, 11);
        assert_eq!(kept.len(), 2, "应仅保留尾部 2 条");
        assert_eq!(kept[0].content_text(), "dddd");
        assert_eq!(kept[1].content_text(), "eeee");
    }

    #[test]
    fn trim_history_keeps_at_least_latest_when_single_exceeds() {
        // 单条超预算 → 仍保留它（不能再裁）
        let big = ChatMessage::from_text("user", "x".repeat(10_000));
        let kept = trim_history_to_budget(std::slice::from_ref(&big), 5);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn trim_history_zero_budget_keeps_latest_only() {
        // 预算 0 → 无法容纳，但至少保留最近一条（防御性最小保留）
        let v = msgs(&["a", "b", "c"]);
        let kept = trim_history_to_budget(&v, 0);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].content_text(), "c");
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