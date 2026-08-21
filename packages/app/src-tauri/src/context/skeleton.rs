//! S8-1 确定性折叠（deterministic fold）——摘要失败路径的最后回退。
//!
//! 问题：MemoryStage 摘要失败/熔断时降级为「裸截断」——中段历史整段蒸发，
//! agent 失忆式续跑（比断掉更隐蔽地坏）。
//!
//! 方案：失败路径不再丢中段，而是本地压缩为「工具调用骨架」：
//! - tool_use 保留 name + 参数一句话摘要（截断）
//! - tool_result 保留 首行预览 + 成败标记，丢结果体
//! - 纯文本消息保留首 N 字符
//! - system 消息与首末 verbatim 区不经过本模块（调用方保证）
//!
//! 特性：**纯本地计算，永不失败**——这是回退链的终点，不依赖任何 LLM。
//! 不落库：仅影响本次请求的上下文投影；session_events 日志无损（不变式 5）。
//! 下一轮若摘要恢复，滚动摘要照常接管（骨架只是过渡态）。

use crate::infra::protocol::{ChatMessage, ContentBlock};

/// 纯文本消息骨架保留的字符数
const TEXT_KEEP_CHARS: usize = 160;
/// tool_use 参数摘要保留字符数
const TOOL_INPUT_KEEP_CHARS: usize = 120;
/// tool_result 首行预览保留字符数
const TOOL_RESULT_KEEP_CHARS: usize = 200;

/// 把一段（折叠区）消息压缩为骨架消息。
/// 输入消息的 role/source_* 原样保留（sanitize_history 配对校验依赖结构不变）。
pub(crate) fn skeletonize_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    messages.iter().map(skeletonize_one).collect()
}

fn skeletonize_one(m: &ChatMessage) -> ChatMessage {
    let mut out = m.clone();
    out.content = m
        .content
        .iter()
        .map(skeletonize_block)
        .collect::<Vec<_>>();
    out
}

fn skeletonize_block(b: &ContentBlock) -> ContentBlock {
    match b {
        ContentBlock::Text { text } => ContentBlock::Text {
            text: truncate_chars(text, TEXT_KEEP_CHARS),
        },
        ContentBlock::Image { data, media_type } => {
            // 图片字节是上下文大头且骨架区无需视觉——占位即可（完整图在日志）
            ContentBlock::Text {
                text: format!("[图片已折叠: {}]", media_type),
            }
            .with_original_image_len(data.len())
        }
        ContentBlock::ToolUse { id, name, input } => ContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: truncate_chars(input, TOOL_INPUT_KEEP_CHARS),
        },
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => ContentBlock::ToolResult {
            tool_use_id: tool_use_id.clone(),
            content: format!(
                "{}{}",
                if is_error.unwrap_or(false) { "[失败] " } else { "" },
                truncate_chars(content, TOOL_RESULT_KEEP_CHARS)
            ),
            is_error: *is_error,
        },
        other => other.clone(),
    }
}

/// 字符级截断（中文安全——按 char 计数），带省略标记。
fn truncate_chars(s: &str, keep: usize) -> String {
    let total = s.chars().count();
    if total <= keep {
        return s.to_string();
    }
    let head: String = s.chars().take(keep).collect();
    format!("{}…[已折叠，省略 {} 字符]", head, total - keep)
}

// ContentBlock 不带附加字段的辅助：Image 折叠为占位时把原始体积记进文案
impl ContentBlock {
    fn with_original_image_len(self, _len: usize) -> ContentBlock {
        // 占位实现：文案已含类型；体积信息在 debug 日志层足够，不进上下文
        self
    }
}

/// 单测辅助：估算骨架化后的 token 缩减比
#[cfg(test)]
pub(crate) fn skeleton_ratio(before: &[ChatMessage], after: &[ChatMessage]) -> f64 {
    use crate::context::token::estimate_message_tokens;
    let b: usize = before.iter().map(estimate_message_tokens).sum();
    let a: usize = after.iter().map(estimate_message_tokens).sum();
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, blocks: Vec<ContentBlock>) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: blocks,
            source_rowid: Some(1),
            source_seq: Some(1),
        }
    }

    #[test]
    fn 长文本截断带省略标记() {
        let long = "字".repeat(500);
        let m = msg(
            "assistant",
            vec![ContentBlock::Text { text: long.clone() }],
        );
        let out = skeletonize_messages(&[m]);
        let text = match &out[0].content[0] {
            ContentBlock::Text { text } => text.clone(),
            _ => panic!(),
        };
        assert!(text.contains("…[已折叠，省略 340 字符]"));
        assert!(text.chars().count() < 200);
    }

    #[test]
    fn 短消息原样保留() {
        let m = msg("user", vec![ContentBlock::Text { text: "你好".into() }]);
        let out = skeletonize_messages(&[m]);
        assert_eq!(
            match &out[0].content[0] {
                ContentBlock::Text { text } => text.clone(),
                _ => panic!(),
            },
            "你好"
        );
    }

    #[test]
    fn tool_use_保留名字与截断参数() {
        let m = msg(
            "assistant",
            vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "read_file".into(),
                input: format!("{{\"path\":\"{}\"}}", "a".repeat(500)),
            }],
        );
        let out = skeletonize_messages(&[m]);
        match &out[0].content[0] {
            ContentBlock::ToolUse { name, input, .. } => {
                assert_eq!(name, "read_file");
                assert!(input.contains("已折叠"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn tool_result_错误标记保留() {
        let m = msg(
            "user",
            vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: "x".repeat(600),
                is_error: Some(true),
            }],
        );
        let out = skeletonize_messages(&[m]);
        match &out[0].content[0] {
            ContentBlock::ToolResult { content, is_error, .. } => {
                assert!(content.starts_with("[失败] "));
                assert_eq!(*is_error, Some(true));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn 骨架化显著缩减token() {
        let big = msg(
            "assistant",
            vec![ContentBlock::Text { text: "字".repeat(4000) }],
        );
        let sk = skeletonize_messages(std::slice::from_ref(&big));
        assert!(skeleton_ratio(&[big], &sk) < 0.1, "骨架应缩减 90%+");
    }

    #[test]
    fn 配对结构不变_tool_use与result的id保留() {
        // sanitize_history 依赖 tool_use/tool_result 按 id 配对——骨架化不得破坏
        let pair = vec![
            msg(
                "assistant",
                vec![ContentBlock::ToolUse {
                    id: "pair1".into(),
                    name: "shell".into(),
                    input: "{}".into(),
                }],
            ),
            msg(
                "user",
                vec![ContentBlock::ToolResult {
                    tool_use_id: "pair1".into(),
                    content: "ok".repeat(300),
                    is_error: Some(false),
                }],
            ),
        ];
        let out = skeletonize_messages(&pair);
        assert!(matches!(
            &out[0].content[0],
            ContentBlock::ToolUse { id, .. } if id == "pair1"
        ));
        assert!(matches!(
            &out[1].content[0],
            ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "pair1"
        ));
    }
}
