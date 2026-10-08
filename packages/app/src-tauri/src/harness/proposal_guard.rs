//! 配置提案 Guardrail 校验层
//!
//! 纯校验模块（无副作用，完全可单测）。agent 调 `propose_config_change` 工具时，
//! 提案先经过本模块校验，再决定是否 emit 给前端。
//!
//! ## 校验规则
//!
//! ### 🔴 红线（直接拒绝，不 emit）
//! - 任何 delete 动作 → 拒绝
//! - 跨 agent 修改（update 目标 != 当前 agent）→ 拒绝
//! - api_key 字段非 `"__SLOT__"` 占位 → 拒绝（防止 prompt injection 偷 key）
//! - 提权/权限变更 → 拒绝
//!
//! ### 🟡 敏感（需用户确认）
//! - 创建带 enabled_tools / tool_scopes 的 agent
//! - 修改 enabled_tools / tool_scopes
//! - 修改模型身份/端点（provider/model/base_url/model_profile_id）——端点变更
//!   会使现有 API Key 发往新地址（2026-10-08 P1 收口，原 Low）
//! - 修改 workspace_path（文件访问边界 + {workspace} 注入链前置）
//!
//! ### 🟢 非敏感（一键批准）
//! - 创建无工具的 agent
//! - 修改自己的 name/system_prompt/temperature/max_tokens
//!   /word_style_profile（Word 样式偏好自由文字块；空串=摘除，同 Low——落地走
//!   set_agent_word_profile，agent.yaml 侧无害）
//!
//! ## 安全边界
//! Phase 1 仅处理 agent 域。Phase 2-4 扩展时在此文件加 match arm 即可，
//! 所有新域自动受 guardrail 保护。

use crate::error::{AppError, AppResult};
use crate::infra::protocol::{ProposalAction, SensitivityTier};

/// API key 占位符。agent 传来的 `api_key` 必须等于此值，否则红线拒绝。
pub const API_KEY_SLOT_PLACEHOLDER: &str = "__SLOT__";

/// 校验提案并返回 (sensitivity_tier, warnings)。
/// 🔴 红线直接返回 Err。
pub fn validate_proposal(
    action: &ProposalAction,
    caller_agent_id: &str,
) -> AppResult<(SensitivityTier, Vec<String>)> {
    let mut warnings: Vec<String> = Vec::new();

    match action {
        ProposalAction::CreateAgent {
            id,
            name,
            provider,
            model,
            api_key,
            enabled_tools,
            tool_scopes,
            ..
        } => {
            // --- 红线检查 ---
            // 1. api_key 必须是占位符
            if api_key != API_KEY_SLOT_PLACEHOLDER {
                return Err(AppError::Validation(
                    "api_key 必须为 \"__SLOT__\" 占位符，不能填入真实密钥。请将 api_key 设为 \"__SLOT__\"。".into(),
                ));
            }
            // 2. 必填字段非空
            if id.trim().is_empty() {
                return Err(AppError::Validation("agent id 不能为空".into()));
            }
            if name.trim().is_empty() {
                return Err(AppError::Validation("agent name 不能为空".into()));
            }
            if provider.trim().is_empty() {
                return Err(AppError::Validation("provider 不能为空".into()));
            }
            if model.trim().is_empty() {
                return Err(AppError::Validation("model 不能为空".into()));
            }

            // --- 敏感度分级 ---
            let has_tools = enabled_tools.as_ref().is_some_and(|t| !t.is_empty())
                || tool_scopes.as_ref().is_some_and(|s| !s.is_empty());

            if has_tools {
                warnings.push("新建的 agent 启用了工具调用，请确认工具列表符合预期。".into());
                Ok((SensitivityTier::Medium, warnings))
            } else {
                Ok((SensitivityTier::Low, warnings))
            }
        }

        ProposalAction::UpdateAgent {
            agent_id,
            enabled_tools,
            tool_scopes,
            provider,
            model,
            base_url,
            model_profile_id,
            workspace_path,
            ..
        } => {
            // --- 红线检查 ---
            // 1. 只能改自己
            if agent_id != caller_agent_id {
                return Err(AppError::Validation(format!(
                    "跨 agent 修改被禁止：提案目标 agent_id='{agent_id}'，当前对话 agent_id='{caller_agent_id}'。只能修改当前对话所属的 agent。"
                )));
            }

            // --- 敏感度分级 ---
            // 2026-10-08 P1（兑现原前瞻备忘）：模型身份/端点字段显式 🟡 Medium——
            // base_url 变更会把 legacy agent 的现有 API Key 逐轮发往任意新端点
            // （key 外泄通道）；workspace_path 变更改写文件访问边界且是 {workspace}
            // 占位符注入链的前置。此前这些字段落 rest pattern = 🟢 Low 一键批准，
            // 防线只剩一张无警示的卡。
            let changing_model_identity =
                provider.is_some() || model.is_some() || base_url.is_some() || model_profile_id.is_some();
            if changing_model_identity {
                warnings.push(
                    "正在修改模型身份/端点配置（provider/model/base_url/模型配置引用）。请核对新值是否可信：端点变更会使该 agent 的现有 API Key 发往新地址。".into(),
                );
            }
            if workspace_path.is_some() {
                warnings.push(
                    "正在修改工作区路径——这会改变该 agent 的文件访问边界与 {workspace} 占位符的解析结果。".into(),
                );
            }
            let changing_tools = enabled_tools.is_some() || tool_scopes.is_some();
            if changing_tools {
                warnings.push(
                    "正在修改 agent 的工具面（启用名单/工具集范围），请确认变更符合预期。".into(),
                );
            }
            if changing_model_identity || workspace_path.is_some() || changing_tools {
                Ok((SensitivityTier::Medium, warnings))
            } else {
                Ok((SensitivityTier::Low, warnings))
            }
        }
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_create_agent(api_key: &str, enabled_tools: Option<Vec<String>>) -> ProposalAction {
        ProposalAction::CreateAgent {
            id: "test-agent".into(),
            name: "Test Agent".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-5".into(),
            api_key: api_key.into(),
            base_url: None,
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            enabled_tools,
            tool_scopes: None,
            workspace_path: None,
            model_profile_id: None,
        }
    }

    fn make_update_agent(
        agent_id: &str,
        enabled_tools: Option<Option<Vec<String>>>,
    ) -> ProposalAction {
        ProposalAction::UpdateAgent {
            agent_id: agent_id.into(),
            name: None,
            provider: None,
            model: None,
            system_prompt: None,
            base_url: None,
            temperature: None,
            max_tokens: None,
            enabled_tools: enabled_tools.unwrap_or(None),
            tool_scopes: None,
            workspace_path: None,
            word_style_profile: None,
            model_profile_id: None,
        }
    }

    // === 合法提案放行 ===

    #[test]
    fn create_agent_without_tools_is_low_sensitivity() {
        let action = make_create_agent("__SLOT__", None);
        let (tier, warnings) = validate_proposal(&action, "caller-1").unwrap();
        assert_eq!(tier, SensitivityTier::Low);
        assert!(warnings.is_empty());
    }

    #[test]
    fn create_agent_with_tools_is_medium_sensitivity() {
        let action = make_create_agent(
            "__SLOT__",
            Some(vec!["read_file".into(), "write_file".into()]),
        );
        let (tier, warnings) = validate_proposal(&action, "caller-1").unwrap();
        assert_eq!(tier, SensitivityTier::Medium);
        assert!(!warnings.is_empty());
    }

    #[test]
    fn update_self_is_low_sensitivity() {
        let action = make_update_agent("caller-1", None);
        let (tier, warnings) = validate_proposal(&action, "caller-1").unwrap();
        assert_eq!(tier, SensitivityTier::Low);
        assert!(warnings.is_empty());
    }

    #[test]
    fn update_self_tools_is_medium_sensitivity() {
        let action = make_update_agent("caller-1", Some(Some(vec!["git".into()])));
        let (tier, warnings) = validate_proposal(&action, "caller-1").unwrap();
        assert_eq!(tier, SensitivityTier::Medium);
        assert!(!warnings.is_empty());
    }

    // === 2026-10-08 P1：模型身份/端点/工作区字段显式 Medium（原 rest pattern Low）===

    /// 身份/边界字段的参数化构造器（enum variant 不支持 record-update 语法）
    fn make_update_agent_identity(
        provider: Option<&str>,
        model: Option<&str>,
        base_url: Option<&str>,
        model_profile_id: Option<&str>,
        workspace_path: Option<&str>,
    ) -> ProposalAction {
        ProposalAction::UpdateAgent {
            agent_id: "caller-1".into(),
            name: None,
            provider: provider.map(str::to_string),
            model: model.map(str::to_string),
            system_prompt: None,
            base_url: base_url.map(str::to_string),
            temperature: None,
            max_tokens: None,
            enabled_tools: None,
            tool_scopes: None,
            workspace_path: workspace_path.map(str::to_string),
            word_style_profile: None,
            model_profile_id: model_profile_id.map(str::to_string),
        }
    }

    #[test]
    fn update_model_identity_fields_are_medium() {
        // 任一身份/端点/边界字段 Some → Medium（base_url 重定向 key 外泄通道、
        // workspace_path 文件边界 + {workspace} 注入链前置）
        let cases = [
            make_update_agent_identity(Some("custom"), None, None, None, None),
            make_update_agent_identity(None, Some("some-model"), None, None, None),
            make_update_agent_identity(None, None, Some("https://attacker.example"), None, None),
            make_update_agent_identity(None, None, None, Some("mp-1"), None),
            make_update_agent_identity(None, None, None, None, Some("C:\\other\\ws")),
        ];
        for action in &cases {
            let (tier, warnings) = validate_proposal(action, "caller-1").unwrap();
            assert_eq!(tier, SensitivityTier::Medium);
            assert!(!warnings.is_empty(), "身份/边界变更应有 warning");
        }
    }

    #[test]
    fn update_endpoint_plus_tools_gives_both_warnings() {
        // 端点 + 工具面同时改：Medium 且两条 warning 并存
        let mut action =
            make_update_agent_identity(None, None, Some("https://x.example"), None, None);
        if let ProposalAction::UpdateAgent {
            enabled_tools, ..
        } = &mut action
        {
            *enabled_tools = Some(vec!["git".into()]);
        }
        let (tier, warnings) = validate_proposal(&action, "caller-1").unwrap();
        assert_eq!(tier, SensitivityTier::Medium);
        assert_eq!(
            warnings.len(),
            2,
            "端点与工具面两条 warning 都给: {warnings:?}"
        );
    }

    #[test]
    fn update_pure_birthcert_fields_stay_low() {
        // 纯出生证字段（name）仍 Low——回归：不把整个 update 升 Medium
        let mut action = make_update_agent_identity(None, None, None, None, None);
        if let ProposalAction::UpdateAgent { name, .. } = &mut action {
            *name = Some("新名字".into());
        }
        let (tier, warnings) = validate_proposal(&action, "caller-1").unwrap();
        assert_eq!(tier, SensitivityTier::Low);
        assert!(warnings.is_empty());
    }

    // === tool_scopes（工具集范围）= 工具面变更，Medium ===

    #[test]
    fn create_agent_with_scopes_is_medium_sensitivity() {
        let mut action = make_create_agent("__SLOT__", None);
        if let ProposalAction::CreateAgent { tool_scopes, .. } = &mut action {
            *tool_scopes = Some(vec!["group:files".into(), "server:ue5".into()]);
        }
        let (tier, warnings) = validate_proposal(&action, "caller-1").unwrap();
        assert_eq!(tier, SensitivityTier::Medium);
        assert!(!warnings.is_empty());
    }

    #[test]
    fn update_self_scopes_is_medium_sensitivity() {
        // 非空收窄与空数组摘除（Some([])）都是工具面变更——都 Medium
        for scopes in [Some(vec!["group:kb".into()]), Some(vec![])] {
            let mut action = make_update_agent("caller-1", None);
            if let ProposalAction::UpdateAgent { tool_scopes, .. } = &mut action {
                *tool_scopes = scopes;
            }
            let (tier, warnings) = validate_proposal(&action, "caller-1").unwrap();
            assert_eq!(tier, SensitivityTier::Medium);
            assert!(!warnings.is_empty());
        }
    }

    #[test]
    fn update_self_word_profile_is_low_sensitivity() {
        // D12：Word 样式偏好（含空串=摘除语义）不触任何红线——自由文字块，
        // 落地走 set_agent_word_profile 写 agent.yaml，无权限面变化
        for profile in [Some("正文宋体小四".to_string()), Some(String::new())] {
            let mut action = make_update_agent("caller-1", None);
            if let ProposalAction::UpdateAgent {
                word_style_profile, ..
            } = &mut action
            {
                *word_style_profile = profile;
            }
            let (tier, warnings) = validate_proposal(&action, "caller-1").unwrap();
            assert_eq!(tier, SensitivityTier::Low);
            assert!(warnings.is_empty());
        }
    }

    // === 🔴 红线拒绝 ===

    #[test]
    fn create_agent_with_real_api_key_is_redline() {
        let action = make_create_agent("sk-actual-key-12345", None);
        let err = validate_proposal(&action, "caller-1").unwrap_err();
        assert!(err.to_string().contains("__SLOT__"));
    }

    #[test]
    fn update_other_agent_is_redline() {
        let action = make_update_agent("other-agent", None);
        let err = validate_proposal(&action, "caller-1").unwrap_err();
        assert!(err.to_string().contains("跨 agent"));
    }

    #[test]
    fn create_agent_empty_id_is_redline() {
        let action = ProposalAction::CreateAgent {
            id: "  ".into(),
            name: "Test".into(),
            provider: "anthropic".into(),
            model: "claude".into(),
            api_key: "__SLOT__".into(),
            base_url: None,
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            enabled_tools: None,
            tool_scopes: None,
            workspace_path: None,
            model_profile_id: None,
        };
        let err = validate_proposal(&action, "caller-1").unwrap_err();
        assert!(err.to_string().contains("id"));
    }

    #[test]
    fn create_agent_empty_name_is_redline() {
        let action = ProposalAction::CreateAgent {
            id: "test".into(),
            name: "  ".into(),
            provider: "anthropic".into(),
            model: "claude".into(),
            api_key: "__SLOT__".into(),
            base_url: None,
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            enabled_tools: None,
            tool_scopes: None,
            workspace_path: None,
            model_profile_id: None,
        };
        let err = validate_proposal(&action, "caller-1").unwrap_err();
        assert!(err.to_string().contains("name"));
    }
}
