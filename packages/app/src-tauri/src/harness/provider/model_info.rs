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
}
