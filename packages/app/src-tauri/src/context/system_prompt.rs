//! System Prompt 构造逻辑
//!
//! 从 `commands/chat_context.rs` 迁入（W5.2）。
//!
//! 提供 `pub(crate)` 函数 [`build_system_prompt`]，按四级优先级构造最终的 system prompt：
//! template > agent > tool_hint > os_context。

/// 构造 system prompt（四级优先级）
///
/// 优先级（从高到低）：
/// 1. `rendered_system_prompt`（模板渲染后，可能为 None）
/// 2. `agent_system_prompt`（agent 配置中的 system prompt）
/// 3. 工具能力提示（当 `tools_enabled` 时追加）
/// 4. OS 运行环境上下文（始终注入）
///
/// 返回 `Some(String)` 或 `None`（仅当所有来源都为空时）。
pub(crate) fn build_system_prompt(
    rendered_system_prompt: Option<&str>,
    agent_system_prompt: &str,
    tools_enabled: bool,
    os_context: &str,
    tool_max_rounds: Option<u32>,
) -> Option<String> {
    let mut effective_system_prompt = rendered_system_prompt
        .filter(|s| !s.is_empty())
        .or(if agent_system_prompt.is_empty() {
            None
        } else {
            Some(agent_system_prompt)
        })
        .map(|s| s.to_string());

    // P2-1: 工具启用时追加工具能力提示
    if tools_enabled {
        let rounds_hint = match tool_max_rounds {
            Some(r) => format!(
                "注意：每轮对话你共有 {} 轮工具调用机会。建议在同一轮内尽可能批量执行所需的工具调用（例如一次列出多个目录），以避免轮数耗尽。当前配置：最多 {} 轮。",
                r, r
            ),
            None => "建议在同一轮内尽可能批量执行所需的工具调用（例如一次列出多个目录），以避免轮数耗尽。".to_string(),
        };
        let tool_hint = format!(
            "你已启用工具调用能力。当用户要求读取文件、列出目录等操作时，请使用提供的工具（如 list_directory、read_file）来执行，不要回复\"无法访问文件\"。\n\n{}",
            rounds_hint,
        );
        effective_system_prompt = Some(match effective_system_prompt {
            Some(s) => format!("{}\n\n{}", s, tool_hint),
            None => tool_hint.to_string(),
        });
    }

    // 注入运行环境信息（始终注入）
    if !os_context.is_empty() {
        effective_system_prompt = Some(match effective_system_prompt {
            Some(s) => format!("{}\n\n{}", s, os_context),
            None => os_context.to_string(),
        });
    }

    effective_system_prompt
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_agent_only() {
        let result = build_system_prompt(None, "你是一个助手", false, "", None);
        assert_eq!(result, Some("你是一个助手".into()));
    }

    #[test]
    fn system_prompt_template_overrides_agent() {
        let result = build_system_prompt(
            Some("模板 prompt"),
            "agent prompt",
            false,
            "os info",
            None,
        );
        let s = result.unwrap();
        assert!(s.contains("模板 prompt"));
        assert!(s.contains("os info"));
    }

    #[test]
    fn system_prompt_tool_hint_appended() {
        let result = build_system_prompt(None, "base", true, "", None);
        let s = result.unwrap();
        assert!(s.contains("工具调用能力"));
        assert!(s.starts_with("base"));
    }

    #[test]
    fn system_prompt_os_always_injected() {
        let result = build_system_prompt(None, "", false, "OS: Linux", None);
        assert_eq!(result, Some("OS: Linux".into()));
    }

    #[test]
    fn system_prompt_none_when_all_empty() {
        let result = build_system_prompt(None, "", false, "", None);
        assert!(result.is_none());
    }
}
