//! L1 Context Budget — token 限额与估算工具
//!
//! 关键 API：
//! - `estimate_tokens(text)`     —— 文本 token 估算（CJK 1/字，英文 1/4 字节）
//! - `estimate_block_tokens(b)`  —— 单个 ContentBlock 的 token（覆盖工具/图片/思考块）
//! - `estimate_message_tokens(m)`—— 单条 ChatMessage 的 token（block 之和 + 结构开销）
//! - `estimate_messages_tokens(messages)` —— 一组 ChatMessage 的总 token
//! - `ContextBudget`             —— 上下文预算配置（max_input_tokens 等；fold 预算派生）

use crate::infra::protocol::{ChatMessage, ContentBlock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    /// 模型最大输入 token 数（Phase 0：由 agent.context_window / 已知模型默认 / 128K 兜底解析）
    pub max_input_tokens: usize,
}

impl ContextBudget {
    /// Phase 2 滚动摘要：verbatim 后缀 token 超过此值即触发折叠（max_input 的 55%）。
    pub fn fold_trigger_tokens(&self) -> usize {
        self.max_input_tokens * 55 / 100
    }
    /// Phase 2 滚动摘要：一次折叠到该预算以下（max_input 的 40%），
    /// 使下次需积累 ~15% 新 token 才再触发（天然频率闸门）。
    pub fn fold_target_tokens(&self) -> usize {
        self.max_input_tokens * 40 / 100
    }
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            // ContextBudget 仅承载 token 预算（max_input + fold 摘要派生）。
            // 工具列表的排序阈值已移至 scoring::DEFAULT_TOOL_SORT_THRESHOLD——它与 token
            // 预算属不同维度，混在同一 struct 既造成语义混淆，也让 loop_engine 误把本
            // default 当工具排序阈值（dead config：per-agent 配置从未流入）。回归单一职责。
            max_input_tokens: 128_000,
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
            source_rowid: None,
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
            source_rowid: None,
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

    // ---- Phase 2: ContextBudget fold 预算派生 ----

    #[test]
    fn fold_budgets_proportional_to_max_input() {
        let b = ContextBudget {
            max_input_tokens: 10_000,
            ..Default::default()
        };
        assert_eq!(b.fold_trigger_tokens(), 5_500); // 55%
        assert_eq!(b.fold_target_tokens(), 4_000); // 40%
        // trigger 必须 > target，否则折叠逻辑无意义
        assert!(b.fold_trigger_tokens() > b.fold_target_tokens());
    }
}