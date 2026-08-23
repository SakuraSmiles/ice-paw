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
) -> Option<String> {
    let mut effective_system_prompt = rendered_system_prompt
        .filter(|s| !s.is_empty())
        .or(if agent_system_prompt.is_empty() {
            None
        } else {
            Some(agent_system_prompt)
        })
        .map(|s| s.to_string());

    // P2-1: 工具启用时追加工具能力提示 + 平台行为纪律
    // MA-1：delegate_to_agent 的指引**不在**这里——委派能力按会话 kind 差异化
    // （session_runner 仅对 kind='chat' 注册该工具并注入可调度清单，见
    // PipelineContext::delegation_hint），base 工具提示保持 kind 无关。
    //
    // 2026-08-23 两层设计（docs/agent-prompt-draft.md）：这段是**平台层**——只放
    // 风格中立的行为纪律，所有 agent 背；风格（先结论/简洁默认等）是人格的一部分，
    // 归 agent.yaml system_prompt（前端「风格预设」三档插入，素材不是档位）。
    // 三条纪律刻意互不重叠且角色无关：错误纪律（与工具层错误契约/doom_loop 咬合）、
    // 诚实边界、语言跟随。「与你的人设叠加生效」是两层关系的锚——纪律不覆盖人格，
    // 创作/陪伴 agent 不被工程风格误伤。
    if tools_enabled {
        let tool_hint = "你已启用工具调用能力。当用户要求读取文件、列出目录等操作时，请使用提供的工具执行，不要回复\"无法访问文件\"。建议在同一轮内尽可能批量执行所需的工具调用（例如一次列出多个目录）；任务完成后直接输出最终回答即可，无需手动终止。\n\n\
通用工作方式（与你的人设叠加生效）：\n\
- 工具失败时，完整阅读返回的错误信息——其中包含恢复指引（如候选路径、修正建议）。按指引修正后重试；同一种失败不要原样重试。\n\
- 不知道、做不到或缺少条件时，直接说明，不要编造。\n\
- 使用用户所用的语言回复。";
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
        let result = build_system_prompt(None, "你是一个助手", false, "");
        assert_eq!(result, Some("你是一个助手".into()));
    }

    #[test]
    fn system_prompt_template_overrides_agent() {
        let result = build_system_prompt(Some("模板 prompt"), "agent prompt", false, "os info");
        let s = result.unwrap();
        assert!(s.contains("模板 prompt"));
        assert!(s.contains("os info"));
    }

    #[test]
    fn system_prompt_tool_hint_appended() {
        let result = build_system_prompt(None, "base", true, "");
        let s = result.unwrap();
        assert!(s.contains("工具调用能力"));
        assert!(s.starts_with("base"));
    }

    /// 平台层纪律锚（2026-08-23 两层设计）：叠加生效声明 + 三条角色无关纪律。
    /// 意图确认**不在**平台层（与创作预设第一条重复，下沉工程档）。
    #[test]
    fn system_prompt_platform_disciplines_present() {
        let result = build_system_prompt(None, "", true, "").unwrap();
        assert!(result.contains("与你的人设叠加生效"));
        assert!(result.contains("同一种失败不要原样重试"));
        assert!(result.contains("不要编造"));
        assert!(result.contains("使用用户所用的语言回复"));
        // 平台层风格中立：不带工程风格措辞（那是风格预设档的内容）
        assert!(!result.contains("先给结论"));
        assert!(!result.contains("先确认再动手"));
    }

    #[test]
    fn system_prompt_os_always_injected() {
        let result = build_system_prompt(None, "", false, "OS: Linux");
        assert_eq!(result, Some("OS: Linux".into()));
    }

    #[test]
    fn system_prompt_none_when_all_empty() {
        let result = build_system_prompt(None, "", false, "");
        assert!(result.is_none());
    }
}
