//! 历史消息加载 + HistoryWindowConfig
//!
//! 从 `commands/chat_context.rs` 迁入（W5.3）。
//!
//! 提供历史消息窗口配置和从历史行转换到 `Vec<ChatMessage>` 的逻辑。

use crate::db::models::MessageRow;
use crate::infra::protocol::ChatMessage;

/// 历史消息窗口配置
///
/// 控制从数据库加载多少条历史消息注入到 LLM 上下文。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HistoryWindowConfig {
    /// 最近的 N 条消息（默认 20）
    pub recent_n: usize,
    /// 是否包含摘要（未来 P3 扩展，默认 false）
    pub include_summary: bool,
}

impl Default for HistoryWindowConfig {
    fn default() -> Self {
        Self {
            recent_n: 20,
            include_summary: false,
        }
    }
}

/// 将历史消息行转换为 `Vec<ChatMessage>`（仅文本）
///
/// - 跳过非 user/assistant/system 角色（如 `tool`）
/// - 仅取 `MessageRow.content`（文本），多模态历史 TODO
pub(crate) fn load_history(history: &[MessageRow]) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(history.len());
    for msg in history {
        let role = match msg.role.as_str() {
            "user" | "assistant" | "system" => msg.role.clone(),
            _ => continue,
        };
        messages.push(ChatMessage::from_text(role, msg.content.clone()));
    }
    messages
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_window_default() {
        let cfg = HistoryWindowConfig::default();
        assert_eq!(cfg.recent_n, 20);
        assert!(!cfg.include_summary);
    }

    #[test]
    fn load_history_skips_tool_role() {
        let make_row = |role: &str, content: &str| MessageRow {
            id: "test".into(),
            conversation_id: "conv".into(),
            role: role.into(),
            content: content.into(),
            content_blocks: "[]".into(),
            token_count: None,
            error: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            rowid: 0,
        };
        let rows = vec![
            make_row("user", "hello"),
            make_row("assistant", "hi"),
            make_row("tool", "result"),
            make_row("system", "sys"),
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
}
