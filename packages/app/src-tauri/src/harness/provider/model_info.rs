//! 已知模型的上下文窗口默认值（策展表）。
//!
//! 用于 `ContextBudget.max_input_tokens` 的运行时解析：当 agent 未显式设置
//! `context_window`（DB 列为 NULL）时，按 `(provider, model)` 查表取默认；
//! 查不到则由调用方回退（见 `chat_cmd.rs` 接线点）。
//!
//! 数值来自厂商文档 / OpenRouter（2026-08 web 核实），仅收录项目实际使用的
//! 模型族；新增模型时在此追加一行即可，无需迁移。

/// 按 `(provider, model)` 返回已知模型的上下文窗口（token 数）。
///
/// 匹配规则：`model` 名大小写不敏感、子串包含。`provider` 当前未参与判定
/// （model 名已是强信号），保留参数以便未来按厂商细分。
///
/// **GLM-5.x 注意**：1M 窗口需在 model 名带 `[1m]` 后缀才会启用（智谱约束）；
/// 不带后缀的 `glm-5.2` 不给默认，由调用方回退保守值。
pub fn default_context_window(_provider: &str, model: &str) -> Option<usize> {
    let m = model.to_lowercase();

    // MiniMax-M3：1M（OpenRouter=1_048_576；官方 up to 1M，保底 512K）
    if m.contains("minimax-m3") {
        return Some(1_048_576);
    }
    // DeepSeek-V4（Pro / Flash 共享 1M，2026-04 发布）
    if m.contains("deepseek-v4") {
        return Some(1_000_000);
    }
    // GLM-5-Turbo：200K（max output 128K）
    if m.contains("glm-5-turbo") || m.contains("glm5-turbo") {
        return Some(200_000);
    }
    // GLM-5.1 / 5.2 带 [1m] 后缀 → 1M（需显式后缀解锁）
    if (m.contains("glm-5.1") || m.contains("glm-5.2")) && m.contains("[1m]") {
        return Some(1_048_576);
    }

    None
}

/// 按 `(provider, model)` 返回已知模型的**单轮最大输出** token 数（策展表）。
///
/// 与 [`default_context_window`]（输入侧）对称的输出侧表（模式 E）。用于 `chat_cmd`
/// 发送前解析 `effective_max_tokens = agent.max_tokens.max(model_default)`，**只抬不降**
/// —— 把过低的默认/历史值（如 4096/16384）抬到模型真实能力，减少自动续写次数。
///
/// 匹配规则同 `default_context_window`：model 名大小写不敏感、子串包含。
///
/// 数值取保守值（32K）：覆盖绝大多数长报告/长对比，且 prompt+max_tokens 之和不会
/// 撞到 provider 的 window 约束（各模型 window 远大于 32K 输出）。真实上限更高的模型
///（如 deepseek-v4 384K、glm-5-turbo 128K）用户可在 agent.yaml 显式调高，`.max()` 会尊重。
pub fn default_max_output_tokens(_provider: &str, model: &str) -> Option<usize> {
    let m = model.to_lowercase();

    // 主流大模型统一给 32K（保守、覆盖长输出、不撞 window）
    if m.contains("minimax-m3")
        || m.contains("deepseek-v4")
        || m.contains("glm-5-turbo")
        || m.contains("glm5-turbo")
        || ((m.contains("glm-5.1") || m.contains("glm-5.2")) && m.contains("[1m]"))
        || m.contains("claude")
        || m.contains("gpt-4")
        || m.contains("gpt-4o")
        || m.contains("o3-mini")
    {
        return Some(32_768);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimax_m3_is_1m_regardless_of_provider_suffix() {
        assert_eq!(default_context_window("minimax", "MiniMax-M3"), Some(1_048_576));
        assert_eq!(default_context_window("minimax-cn", "minimax-m3"), Some(1_048_576));
    }

    #[test]
    fn deepseek_v4_pro_and_flash_share_1m() {
        assert_eq!(default_context_window("deepseek", "deepseek-v4-pro"), Some(1_000_000));
        assert_eq!(default_context_window("deepseek", "deepseek-v4-flash"), Some(1_000_000));
    }

    #[test]
    fn glm_5_turbo_is_200k() {
        assert_eq!(default_context_window("glm", "glm-5-turbo"), Some(200_000));
    }

    #[test]
    fn glm_52_needs_1m_suffix() {
        assert_eq!(default_context_window("glm", "glm-5.2[1m]"), Some(1_048_576));
        assert_eq!(default_context_window("glm", "glm-5.2"), None);
    }

    #[test]
    fn unknown_model_returns_none() {
        assert_eq!(default_context_window("openai", "gpt-4o"), None);
        assert_eq!(default_context_window("", "some-custom-model"), None);
    }

    // --- 输出侧表（default_max_output_tokens）---

    #[test]
    fn output_known_models_get_32k() {
        assert_eq!(
            default_max_output_tokens("minimax", "MiniMax-M3"),
            Some(32_768)
        );
        assert_eq!(
            default_max_output_tokens("deepseek", "deepseek-v4-pro"),
            Some(32_768)
        );
        assert_eq!(
            default_max_output_tokens("glm", "glm-5-turbo"),
            Some(32_768)
        );
        assert_eq!(
            default_max_output_tokens("anthropic", "claude-sonnet-4-20250514"),
            Some(32_768)
        );
        assert_eq!(default_max_output_tokens("openai", "gpt-4o"), Some(32_768));
    }

    #[test]
    fn output_glm_52_needs_1m_suffix_like_input_table() {
        assert_eq!(
            default_max_output_tokens("glm", "glm-5.2[1m]"),
            Some(32_768)
        );
        // 不带后缀的 glm-5.2 → None（与输入表保持一致：后缀才解锁）
        assert_eq!(default_max_output_tokens("glm", "glm-5.2"), None);
    }

    #[test]
    fn output_unknown_model_returns_none() {
        assert_eq!(default_max_output_tokens("", "some-custom-model"), None);
    }
}
