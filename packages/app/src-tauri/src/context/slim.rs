//! S8-2 历史工具结果瘦身（tool result slimming）——上下文最大头的节省。
//!
//! 现状：折叠区之外的近区（verbatim 窗口）tool_result 以全量进上下文——
//! shell 输出/文件内容动辄数千 token，且随会话滚雪球。
//!
//! 方案：ver­batim 区内超过阈值的 tool_result 截头尾、中段以指针标记：
//! `[已省略 N 字符 · 完整结果在消息 #rowid · 可用 @引用 取回]`
//! - 不可逆：信息仍可经 @引用机制从日志取回（指针给出去路）
//! - 纯投影：不落库（session_events 无损，不变式 5）
//! - 与 S8-1 分工：S8-1 管折叠区（摘要失败兜底），S8-2 管近区（日常大头）
//!
//! 阈值策略（L1 好默认，不成为配置）：阈值内的结果原样保留——短结果
//! （git status / 文件片段）零损耗，只有真正的巨结果才瘦身。

use crate::infra::protocol::{ChatMessage, ContentBlock};

/// tool_result 触发瘦身的字符阈值（中英混排按 char 计）
const SLIM_THRESHOLD_CHARS: usize = 2000;
/// 瘦身保留的头/尾字符数
const SLIM_KEEP_HEAD: usize = 400;
const SLIM_KEEP_TAIL: usize = 200;

/// 对 verbatim 窗口内的消息做工具结果瘦身（原地投影）。
/// 返回是否发生了任何瘦身（供日志观测）。
pub(crate) fn slim_tool_results(messages: &mut [ChatMessage]) -> bool {
    let mut any = false;
    for m in messages.iter_mut() {
        for b in m.content.iter_mut() {
            if let ContentBlock::ToolResult { content, tool_use_id, .. } = b {
                let total = content.chars().count();
                if total > SLIM_THRESHOLD_CHARS {
                    let head: String = content.chars().take(SLIM_KEEP_HEAD).collect();
                    let tail: String = content
                        .chars()
                        .skip(total.saturating_sub(SLIM_KEEP_TAIL))
                        .collect();
                    let omitted = total - SLIM_KEEP_HEAD - SLIM_KEEP_TAIL;
                    let rowid_hint = m
                        .source_rowid
                        .map(|r| format!("消息 #{r}"))
                        .unwrap_or_else(|| "会话日志".to_string());
                    *content = format!(
                        "{head}\n…[已省略 {omitted} 字符 · 完整结果在 {rowid_hint} · 可用 @引用 取回]…\n{tail}"
                    );
                    // 指纹去重标记：同轮多次瘦身不重复计（tool_use_id 保留在块上，无需处理）
                    let _ = tool_use_id;
                    any = true;
                }
            }
        }
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg_with_result(content: &str, rowid: i64) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: content.to_string(),
                is_error: None,
            }],
            source_rowid: Some(rowid),
            source_seq: None,
        }
    }

    #[test]
    fn 短结果原样保留() {
        let mut msgs = [msg_with_result("ok\ndone", 1)];
        assert!(!slim_tool_results(&mut msgs));
        match &msgs[0].content[0] {
            ContentBlock::ToolResult { content, .. } => assert_eq!(content, "ok\ndone"),
            _ => panic!(),
        }
    }

    #[test]
    fn 巨结果截头尾带指针() {
        let big = "a".repeat(5000);
        let mut msgs = [msg_with_result(&big, 42)];
        assert!(slim_tool_results(&mut msgs));
        match &msgs[0].content[0] {
            ContentBlock::ToolResult { content, .. } => {
                assert!(content.contains("已省略 4400 字符"));
                assert!(content.contains("消息 #42"));
                assert!(content.contains("@引用"));
                // 头尾保留
                assert!(content.starts_with(&"a".repeat(100)));
                assert!(content.ends_with(&"a".repeat(50)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn 中文按字符计数() {
        let big = "字".repeat(3000);
        let mut msgs = [msg_with_result(&big, 1)];
        assert!(slim_tool_results(&mut msgs));
        match &msgs[0].content[0] {
            ContentBlock::ToolResult { content, .. } => {
                assert!(content.contains("已省略 2400 字符"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn 非tool块不受影响() {
        let mut msgs = [ChatMessage {
            role: "assistant".to_string(),
            content: vec![ContentBlock::Text {
                text: "x".repeat(9000),
            }],
            source_rowid: Some(1),
            source_seq: None,
        }];
        assert!(!slim_tool_results(&mut msgs));
    }
}
