//! System Prompt 构造逻辑
//!
//! 从 `commands/chat_context.rs` 迁入（W5.2）。
//!
//! 按四级优先级构造最终的 system prompt：template > agent > tool_hint > os_context。
//! ③ 可观测化起拆为 [`SystemPromptParts`] 五段（delegation_hint / word_style
//! 由 SystemPromptStage 追加在 os 之后）——拼接输出与旧拼接路径逐字节等价
//! （测试锁），段级产物供上下文体检（context/anatomy.rs）与缓存 miss 归因的
//! 稳定段哈希消费。

/// 平台层工具能力提示（`tools_enabled` 时追加）。
///
/// MA-1：delegate_to_agent 的指引**不在**这里——委派能力按会话 kind 差异化
/// （session_runner 仅对 kind='chat' 注册该工具并注入可调度清单，见
/// PipelineContext::delegation_hint），base 工具提示保持 kind 无关。
///
/// 2026-08-23 两层设计（docs/agent-prompt-draft.md）：这段是**平台层**——只放
/// 风格中立的行为纪律，所有 agent 背；风格（先结论/简洁默认等）是人格的一部分，
/// 归 agent.yaml system_prompt（前端「风格预设」三档插入，素材不是档位）。
/// 三条纪律刻意互不重叠且角色无关：错误纪律（与工具层错误契约/doom_loop 咬合）、
/// 诚实边界、语言跟随。「与你的人设叠加生效」是两层关系的锚——纪律不覆盖人格，
/// 创作/陪伴 agent 不被工程风格误伤。
const TOOL_HINT: &str = "你已启用工具调用能力。当用户要求读取文件、列出目录等操作时，请使用提供的工具执行，不要回复\"无法访问文件\"。建议在同一轮内尽可能批量执行所需的工具调用（例如一次列出多个目录）；任务完成后直接输出最终回答即可，无需手动终止。\n\n\
通用工作方式（与你的人设叠加生效）：\n\
- 工具失败时，完整阅读返回的错误信息——其中包含恢复指引（如候选路径、修正建议）。按指引修正后重试；同一种失败不要原样重试。\n\
- 不知道、做不到或缺少条件时，直接说明，不要编造。\n\
- 使用用户所用的语言回复。";

/// system prompt 的段级组成（③ 可观测化）。
///
/// `joined()` 的拼接序 = 历史行为：persona → tool_hint → os_context →
/// delegation_hint → word_style（后两段历史上由 SystemPromptStage 在
/// build_system_prompt 输出之后追加，见 stages.rs——移入段结构时保持字节序）。
///
/// `stable_hash()` 只覆盖不含 os_context 的四段：os_context 含秒级当前时间
/// （os_context.rs），整段哈希恒变、对缓存前缀归因是恒真噪声；os_context 的
/// 稳定核由 OsContextStage 单独产出（`PipelineContext::os_stable_hash`）。
#[derive(Debug, Clone, Default)]
pub struct SystemPromptParts {
    /// 模板渲染结果 > agent.system_prompt（两者皆空则 None）
    pub persona: Option<String>,
    /// Some(TOOL_HINT) 当 tools_enabled
    pub tool_hint: Option<&'static str>,
    /// OsContextStage 产物（非空才 Some）
    pub os_context: Option<String>,
    /// MA-1 可调度清单（session_runner 填充，SystemPromptStage 移入）
    pub delegation_hint: Option<String>,
    /// D12 Word 样式档案（格式化后小节，SystemPromptStage 移入）
    pub word_style: Option<String>,
}

impl SystemPromptParts {
    /// 按四级优先级解析前三段（delegation/word_style 由 Stage 后续填充）。
    pub(crate) fn build(
        rendered_system_prompt: Option<&str>,
        agent_system_prompt: &str,
        tools_enabled: bool,
        os_context: &str,
    ) -> Self {
        Self {
            persona: rendered_system_prompt
                .filter(|s| !s.is_empty())
                .or(if agent_system_prompt.is_empty() {
                    None
                } else {
                    Some(agent_system_prompt)
                })
                .map(|s| s.to_string()),
            tool_hint: tools_enabled.then_some(TOOL_HINT),
            os_context: (!os_context.is_empty()).then(|| os_context.to_string()),
            delegation_hint: None,
            word_style: None,
        }
    }

    /// 按历史顺序拼接为最终 system prompt（None = 所有段皆空）。
    pub(crate) fn joined(&self) -> Option<String> {
        let parts = [
            self.persona.as_deref(),
            self.tool_hint,
            self.os_context.as_deref(),
            self.delegation_hint.as_deref(),
            self.word_style.as_deref(),
        ];
        let joined: Vec<&str> = parts.into_iter().flatten().collect();
        (!joined.is_empty()).then(|| joined.join("\n\n"))
    }

    /// 稳定段指纹（persona + tool_hint + delegation_hint + word_style，不含 os）。
    /// FNV-1a 12 hex，跨版本稳定（anatomy::fnv1a_12hex）。
    pub(crate) fn stable_hash(&self) -> String {
        crate::context::anatomy::fnv1a_12hex(&[
            self.persona.as_deref().unwrap_or(""),
            self.tool_hint.unwrap_or(""),
            self.delegation_hint.as_deref().unwrap_or(""),
            self.word_style.as_deref().unwrap_or(""),
        ])
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn build_system_prompt(
        rendered: Option<&str>,
        agent: &str,
        tools: bool,
        os: &str,
    ) -> Option<String> {
        SystemPromptParts::build(rendered, agent, tools, os).joined()
    }

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

    // ---- ③ 可观测化：Parts 段化等价与稳定段哈希 ----

    /// 段化重构的硬锁：Parts::build(..).joined() 与历史拼接路径在
    /// persona×tool×os 组合矩阵下逐字节相等（delegation/word_style 追加
    /// 在 os 之后 = stages.rs 历史行为）。
    #[test]
    fn parts_joined_matches_legacy_concatenation_matrix() {
        for rendered in [None, Some(""), Some("模板 P")] {
            for agent in ["", "agent 人设"] {
                for tools in [false, true] {
                    for os in ["", "## 运行环境\nOS: Windows"] {
                        let base =
                            SystemPromptParts::build(rendered, agent, tools, os).joined();
                        // 历史 delegation/word_style 追加路径（stages.rs 原样模拟）
                        let mut legacy = base.clone();
                        for section in ["委派清单 hint", "## Word 文档样式偏好\n\n正文"] {
                            legacy = Some(match legacy.take() {
                                Some(s) => format!("{s}\n\n{section}"),
                                None => section.to_string(),
                            });
                        }
                        // Parts 路径
                        let mut parts =
                            SystemPromptParts::build(rendered, agent, tools, os);
                        parts.delegation_hint = Some("委派清单 hint".into());
                        parts.word_style = Some("## Word 文档样式偏好\n\n正文".into());
                        assert_eq!(
                            parts.joined(),
                            legacy,
                            "rendered={rendered:?} agent={agent:?} tools={tools} os={os:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn stable_hash_excludes_os_context() {
        let a = SystemPromptParts::build(None, "人设", true, "OS 甲");
        let b = SystemPromptParts::build(None, "人设", true, "OS 乙（时间行已变）");
        assert_eq!(a.stable_hash(), b.stable_hash(), "os 变化不得影响稳定段哈希");

        // 稳定段任一变化 → 哈希变
        let c = SystemPromptParts::build(None, "人设（改）", true, "OS 甲");
        assert_ne!(a.stable_hash(), c.stable_hash());
        let d = SystemPromptParts::build(None, "人设", false, "OS 甲");
        assert_ne!(a.stable_hash(), d.stable_hash(), "工具提示增删是稳定段变化");
        let mut e = SystemPromptParts::build(None, "人设", true, "OS 甲");
        e.word_style = Some("## Word 文档样式偏好\n\n宋体".into());
        assert_ne!(a.stable_hash(), e.stable_hash());
    }
}
