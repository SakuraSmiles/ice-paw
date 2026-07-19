//! 历史消息加载 + HistoryWindowConfig
//!
//! 从 `commands/chat_context.rs` 迁入（W5.3）。
//!
//! 提供历史消息窗口配置和从历史行转换到 `Vec<ChatMessage>` 的逻辑。
//!
//! A3-2 变更：窗口大小可由 Agent 的 `max_history_messages` 字段覆盖；
//! 该字段为 `None` 时回退到本模块的 [`DEFAULT_HISTORY_WINDOW`]。

use crate::db::models::MessageRow;
use crate::infra::protocol::{ChatMessage, ContentBlock};

/// 系统默认历史窗口大小（最近 N 条消息）
///
/// 当 Agent 未配置 `max_history_messages` 时使用该默认值。
/// 集中定义以便全栈统一：后端 [`load_history`] + Pipeline + 前端 placeholder
/// 都引用此值（W6.4 Sprint #6.4 沿用历史行为，避免破坏既有 UI 体验）。
pub const DEFAULT_HISTORY_WINDOW: usize = 20;

/// 历史消息窗口配置
///
/// 控制从数据库加载多少条历史消息注入到 LLM 上下文。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HistoryWindowConfig {
    /// 最近的 N 条消息（默认 `DEFAULT_HISTORY_WINDOW`）
    pub recent_n: usize,
    /// 是否包含摘要（未来 P3 扩展，默认 false）
    pub include_summary: bool,
}

impl Default for HistoryWindowConfig {
    fn default() -> Self {
        Self {
            recent_n: DEFAULT_HISTORY_WINDOW,
            include_summary: false,
        }
    }
}

/// 从 Agent 配置解析得到有效窗口大小
///
/// - `None`（Agent 未配置） → 系统默认 [`DEFAULT_HISTORY_WINDOW`]
/// - 非法值（<= 0 或过大）→ 退回到系统默认
///
/// `max_history_messages` 在 Rust 侧是 `Option<i32>`，
/// 但历史窗口作为「最近 N 条」必须 >= 1，所以兜底到 1；过大值
/// 由调用方在 DB 加载阶段限制（见 `repo::message::MAX_LIMIT`）。
pub fn resolve_window(agent_max: Option<i32>) -> usize {
    match agent_max {
        Some(n) if n > 0 => n as usize,
        // n <= 0 视为非法，回退默认
        _ => DEFAULT_HISTORY_WINDOW,
    }
}

/// 将历史消息行转换为 `Vec<ChatMessage>`（仅文本）
///
/// - 跳过非 user/assistant/system 角色（如 `tool`）
/// - 仅取 `MessageRow.content`（文本），多模态历史 TODO
/// - 如果提供 `window`，仅保留**最近** `window` 条（按切片顺序尾部）
///
/// 设计：
/// - **窗口化在 Stage 内**完成（而非 DB 加载侧）：调用者可以一次加载
///   足够多的历史，未来 A3-4 摘要阶段可读取完整历史，窗口只在最终
///   注入 LLM 时应用。这是 A3-2 设计原则「窗口大小按 Agent 配置」
///   的正确层级。
/// - 当调用方传 `None`（典型场景：未走 PipelineRunner 的内部调用）
///   时保持向后兼容，不过滤。
#[allow(dead_code)]
pub(crate) fn load_history(history: &[MessageRow]) -> Vec<ChatMessage> {
    load_history_with_window(history, None)
}

/// 带窗口的版本：A3-2 引入，供 [`crate::context::stages::HistoryStage`] 使用。
///
/// `window = Some(n)` → 仅保留最后 n 条
/// `window = None`    → 不过滤（向后兼容）
///
/// P2-2 (G1)：当 [`MessageRow::content_blocks`] 非空时，从该 JSON 还原完整
/// 多模态消息（含 `ContentBlock::Image`），否则回退到纯文本 [`MessageRow::content`]，
/// 以兼容旧消息（`content_blocks = "[]"`）。
pub(crate) fn load_history_with_window(
    history: &[MessageRow],
    window: Option<usize>,
) -> Vec<ChatMessage> {
    // 窗口裁剪：仅在有窗口且需要时执行
    let slice: &[MessageRow] = match window {
        Some(n) if n < history.len() => &history[history.len() - n..],
        _ => history,
    };

    let mut messages = Vec::with_capacity(slice.len());
    for msg in slice {
        let role = match msg.role.as_str() {
            "user" | "assistant" | "system" => msg.role.clone(),
            _ => continue,
        };

        // P2-2 G1: 优先从 content_blocks 还原多模态消息。
        // 空数组 / 无效 JSON / 解析失败 → 回退到纯文本（兼容旧消息）。
        let blocks = parse_content_blocks(&msg.content_blocks);
        if blocks.is_empty() {
            messages.push(ChatMessage::from_text(role, msg.content.clone()));
        } else {
            messages.push(ChatMessage {
                role,
                content: blocks,
            });
        }
    }
    messages
}

/// 解析 `content_blocks` JSON 字符串为 `Vec<ContentBlock>`。
///
/// - 空字符串 / `"[]"` / 无效 JSON → 返回空 `Vec`（调用方会回退到纯文本）
/// - 仅含 `Text` 块 → 返回该 `Vec`（含若干 `Text` 块）
/// - 含 `Image` 等多模态块 → 返回完整还原的 blocks
///
/// **注意**：assistant 消息的 `ToolUse` / `ToolResult` 块也需要通过此路径还原，
/// 否则多轮工具调用对话的历史上下文会丢失工具调用记录。
fn parse_content_blocks(json: &str) -> Vec<ContentBlock> {
    if json.is_empty() || json == "[]" {
        return Vec::new();
    }
    serde_json::from_str::<Vec<ContentBlock>>(json).unwrap_or_default()
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row(idx: usize, role: &str, content: &str) -> MessageRow {
        MessageRow {
            id: format!("msg-{idx}"),
            conversation_id: "conv".into(),
            role: role.into(),
            content: content.into(),
            content_blocks: "[]".into(),
            token_count: None,
            error: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            rowid: idx as i64,
            summary_id: None,
        }
    }

    #[test]
    fn history_window_default() {
        let cfg = HistoryWindowConfig::default();
        assert_eq!(cfg.recent_n, DEFAULT_HISTORY_WINDOW);
        assert_eq!(cfg.recent_n, 20);
        assert!(!cfg.include_summary);
    }

    #[test]
    fn default_history_window_constant_is_20() {
        // A3-2: 跨端占位常量必须保持 20（兼容旧 UI）
        assert_eq!(DEFAULT_HISTORY_WINDOW, 20);
    }

    #[test]
    fn resolve_window_uses_default_when_agent_none() {
        assert_eq!(resolve_window(None), DEFAULT_HISTORY_WINDOW);
    }

    #[test]
    fn resolve_window_uses_agent_value_when_positive() {
        assert_eq!(resolve_window(Some(60)), 60);
        assert_eq!(resolve_window(Some(1)), 1);
    }

    #[test]
    fn resolve_window_falls_back_to_default_on_non_positive() {
        // 0 / 负数视为未配置 → 回退默认
        assert_eq!(resolve_window(Some(0)), DEFAULT_HISTORY_WINDOW);
        assert_eq!(resolve_window(Some(-5)), DEFAULT_HISTORY_WINDOW);
    }

    #[test]
    fn load_history_skips_tool_role() {
        let rows = vec![
            make_row(1, "user", "hello"),
            make_row(2, "assistant", "hi"),
            make_row(3, "tool", "result"),
            make_row(4, "system", "sys"),
        ];
        let msgs = load_history(&rows);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[2].role, "system");
    }

    #[test]
    fn load_history_empty() {
        let msgs = load_history(&[]);
        assert!(msgs.is_empty());
    }

    #[test]
    fn load_history_with_window_keeps_last_n() {
        let rows: Vec<MessageRow> = (0..10)
            .map(|i| make_row(i, "user", &format!("msg-{i}")))
            .collect();
        let msgs = load_history_with_window(&rows, Some(3));
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].content_text(), "msg-7");
        assert_eq!(msgs[1].content_text(), "msg-8");
        assert_eq!(msgs[2].content_text(), "msg-9");
    }

    #[test]
    fn load_history_with_window_none_keeps_all() {
        // window=None 走向后兼容路径，不过滤
        let rows: Vec<MessageRow> = (0..5)
            .map(|i| make_row(i, "user", &format!("msg-{i}")))
            .collect();
        let msgs = load_history_with_window(&rows, None);
        assert_eq!(msgs.len(), 5);
    }

    #[test]
    fn load_history_with_window_larger_than_input_keeps_all() {
        // window >= input 长度 → 全部保留
        let rows: Vec<MessageRow> = (0..3)
            .map(|i| make_row(i, "user", &format!("msg-{i}")))
            .collect();
        let msgs = load_history_with_window(&rows, Some(100));
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn load_history_with_window_still_skips_tool_role() {
        // 窗口裁剪 + role 过滤 是两个独立动作，都要生效
        let rows = vec![
            make_row(1, "user", "u1"),
            make_row(2, "assistant", "a1"),
            make_row(3, "tool", "skip"),
            make_row(4, "user", "u2"),
            make_row(5, "assistant", "a2"),
        ];
        let msgs = load_history_with_window(&rows, Some(3));
        // 最后 3 条: tool(user? no—tool), user(u2), assistant(a2)
        // tool 被过滤后剩 user(u2) + assistant(a2)
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content_text(), "u2");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content_text(), "a2");
    }

    // ===== P2-2 G1：历史消息图片重注入 =====

    fn make_row_with_blocks(
        idx: usize,
        role: &str,
        content: &str,
        content_blocks: &str,
    ) -> MessageRow {
        MessageRow {
            id: format!("msg-{idx}"),
            conversation_id: "conv".into(),
            role: role.into(),
            content: content.into(),
            content_blocks: content_blocks.into(),
            token_count: None,
            error: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            rowid: idx as i64,
            summary_id: None,
        }
    }

    #[test]
    fn load_history_restores_image_from_content_blocks() {
        // 含图片的多模态消息: content_blocks 是权威源，应优先使用
        let blocks_json = r#"[{"type":"text","text":"看图"},{"type":"image","data":"AAAA","media_type":"image/png"}]"#;
        let row = make_row_with_blocks(1, "user", "看图", blocks_json);
        let msgs = load_history(&[row]);
        assert_eq!(msgs.len(), 1);
        // content_blocks 还原出 2 个块：Text + Image
        assert_eq!(msgs[0].content.len(), 2);
        assert!(!msgs[0].content[0].is_image(), "第一个块应是 Text");
        assert!(msgs[0].content[1].is_image(), "第二个块应是 Image");
    }

    #[test]
    fn load_history_fallback_to_text_when_blocks_empty() {
        // 纯文本消息（content_blocks = "[]"）走原有路径，行为不变
        let row = make_row_with_blocks(1, "user", "hello", "[]");
        let msgs = load_history(&[row]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content_text(), "hello");
        // 纯文本回退后仅一个 Text 块
        assert_eq!(msgs[0].content.len(), 1);
        assert!(!msgs[0].content[0].is_image());
    }

    #[test]
    fn load_history_invalid_blocks_json_falls_back_gracefully() {
        // 无效 JSON 不应崩溃，应静默回退到纯文本
        let row = make_row_with_blocks(1, "user", "hello", "INVALID JSON {");
        let msgs = load_history(&[row]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content_text(), "hello");
        // 回退后仅一个 Text 块
        assert_eq!(msgs[0].content.len(), 1);
        assert!(!msgs[0].content[0].is_image());
    }
}
