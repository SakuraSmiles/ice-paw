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
/// 不带后缀的裸名按厂商文档基准 200K（此前不给默认、由调用方回退 128K——
/// 偏小且与 glm-5-turbo 不一致）。
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
    // GLM-5.1 / 5.2 / 5.3 系（含 5.3-Flash）：官方页声明 1M 支持，但 5.x 的
    // 1M 一直需 model 名带 [1m] 后缀显式解锁（智谱约束）；裸名按厂商基准 200K
    if m.contains("glm-5.1") || m.contains("glm-5.2") || m.contains("glm-5.3") {
        return if m.contains("[1m]") {
            Some(1_048_576)
        } else {
            Some(200_000)
        };
    }
    // Claude 3 / 4 / 5 全系 200K 标准窗口（家族名兜底覆盖带日期戳的变体）
    if m.contains("claude-3")
        || m.contains("claude-4")
        || m.contains("claude-5")
        || m.contains("claude-opus")
        || m.contains("claude-sonnet")
        || m.contains("claude-haiku")
        || m.contains("claude-fable")
    {
        return Some(200_000);
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
        || m.contains("glm-5.1")
        || m.contains("glm-5.2")
        || m.contains("glm-5.3")
        || m.contains("glm-5v")
        || m.contains("claude")
        || m.contains("gpt-4")
        || m.contains("gpt-4o")
        || m.contains("o3-mini")
    {
        return Some(32_768);
    }

    None
}

/// 已知模型的模态能力（策展表）。
///
/// 用于 [`effective_supports_vision`]：当 agent 未显式声明 `supports_vision`（DB 值 =0，
/// 多为默认值/未配置）时，按模型自动探测，免去用户手填每个模型的能力位。未知模型
/// 保守返回 `vision = false`（绝不误报"支持"，否则图会被原样硬发给非视觉模型 → 400）。
///
/// 仅收录**确定支持视觉**的模型族（子串匹配，同 [`default_context_window`] 规则）；
/// 不确定的不收录 → 保守 false，由 agent 显式 `supports_vision = 1` 兜底。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModelCapabilities {
    /// 是否支持图片输入（vision / multimodal）。
    pub vision: bool,
    // 未来扩展：audio / video。当前项目无音频模态，YAGNI 暂不收。
}

/// 按 `(provider, model)` 返回已知模型的模态能力。匹配规则同
/// [`default_context_window`]：model 名大小写不敏感、子串包含。`provider` 当前未参与
/// 判定（model 名已是强信号），保留参数以便未来按厂商细分。
pub fn model_capabilities(_provider: &str, model: &str) -> ModelCapabilities {
    let m = model.to_lowercase();
    let vision = m.contains("gpt-4o")        // gpt-4o / gpt-4o-mini（OpenAI 视觉主力）
        || m.contains("gpt-4-vision")
        || m.contains("gpt-4-turbo")         // gpt-4 vision 变体
        // Claude 3.0+ / 4.x / 5.x 全系视觉（claude-2 太老，项目不用，不收录）
        || m.contains("claude-3") || m.contains("claude-4") || m.contains("claude-5")
        || m.contains("claude-sonnet") || m.contains("claude-opus") || m.contains("claude-haiku")
        || m.contains("claude-fable")
        || m.contains("gemini")              // Gemini 1.0+ 全系视觉
        // 智谱视觉系列：4v/4.6v 老三样 + 4.5v/4.1v（不含 "glm-4v" 子串需单列）
        // + glm-5v 系（GLM-5V-Turbo 多模态 Coding 基座）+ glm-5.3-flash
        //（GLM-5 系首个原生多模态，2026-08）。
        // ⚠️ 只认 "glm-5.3-flash" 不认 "glm-5.3"——5.3 旗舰是纯文本模型，
        // 宽匹配会把图硬发给非视觉模型 → 400。
        || m.contains("glm-4v") || m.contains("glm-4.6v") || m.contains("glm4v")
        || m.contains("glm-4.5v") || m.contains("glm-4.1v")
        || m.contains("glm-5v") || m.contains("glm-5.3-flash")
        || m.contains("qwen-vl") || m.contains("qwen2-vl") || m.contains("qwenvl") // 通义视觉
        || m.contains("minimax-m3")          // MiniMax M3 支持视觉（M2.x 不支持，不匹配 m3）
        // DeepSeek 视觉两线：开源 VL 系列 + API 实验模型 deepseek-v4-flash-vision-exp
        //（2026-08 上线；连字符锚定 "-vision" 防 "provision" 类误报）。v4 chat
        // 纯文本不含 -vl/-vision → 保守 false
        || m.contains("deepseek-vl") || m.contains("-vision");
    ModelCapabilities { vision }
}

/// 解析 agent 的"有效视觉能力"（OR 关系，**零 schema 改动**）。
///
/// `agent.supports_vision != 0`（用户显式开启）**或**模型表自动探测支持 → 任一为真即支持：
/// - 显式 `supports_vision = 1` → 永远生效（权威 override）；
/// - `supports_vision = 0`（默认/未配置）但模型实际支持（如 MiniMax-M3）→ 自动探测生效，
///   顺手修配置遗漏；
/// - 显式 = 0 且模型也不支持 → 不支持。
///
/// **无"显式关闭"语义**：0 既可能是"未配置"也可能是"我明确不要视觉"，OR 关系把两者都当
/// "未配置"。若将来需要"我知道这模型支持但想关掉"，再加 `vision_mode` 三态列；当前 OR
/// 覆盖 99% 场景且零 migration、零行为回退。
pub fn effective_supports_vision(agent_supports_vision: i32, provider: &str, model: &str) -> bool {
    agent_supports_vision != 0 || model_capabilities(provider, model).vision
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimax_m3_is_1m_regardless_of_provider_suffix() {
        assert_eq!(
            default_context_window("minimax", "MiniMax-M3"),
            Some(1_048_576)
        );
        assert_eq!(
            default_context_window("minimax-cn", "minimax-m3"),
            Some(1_048_576)
        );
    }

    #[test]
    fn deepseek_v4_pro_and_flash_share_1m() {
        assert_eq!(
            default_context_window("deepseek", "deepseek-v4-pro"),
            Some(1_000_000)
        );
        assert_eq!(
            default_context_window("deepseek", "deepseek-v4-flash"),
            Some(1_000_000)
        );
    }

    #[test]
    fn glm_5_turbo_is_200k() {
        assert_eq!(default_context_window("glm", "glm-5-turbo"), Some(200_000));
    }

    #[test]
    fn glm_52_bare_200k_and_1m_suffix_unlocks() {
        assert_eq!(
            default_context_window("glm", "glm-5.2[1m]"),
            Some(1_048_576)
        );
        // 裸名不再回退：厂商文档基准 200K
        assert_eq!(default_context_window("glm", "glm-5.2"), Some(200_000));
        assert_eq!(default_context_window("glm", "glm-5.1"), Some(200_000));
    }

    #[test]
    fn glm_53_family_same_suffix_rule() {
        // 5.3 / 5.3-Flash（2026-08 发布）沿用 5.x [1m] 后缀约束
        assert_eq!(default_context_window("glm", "glm-5.3"), Some(200_000));
        assert_eq!(
            default_context_window("glm", "glm-5.3-flash"),
            Some(200_000)
        );
        assert_eq!(
            default_context_window("glm", "GLM-5.3-Flash[1M]"),
            Some(1_048_576)
        );
    }

    #[test]
    fn claude_family_200k_window_and_vision() {
        // 目录现役系列（②-5）+ 带日期戳的存量变体，家族名兜底全覆盖
        for model in [
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-fable-5",
            "claude-haiku-4-5",
            "claude-sonnet-4-20250514",
            "claude-haiku-3-5-20241022",
        ] {
            assert_eq!(default_context_window("anthropic", model), Some(200_000));
            assert!(model_capabilities("anthropic", model).vision, "{model}");
        }
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
    fn output_glm_52_bare_and_suffixed_both_32k() {
        assert_eq!(
            default_max_output_tokens("glm", "glm-5.2[1m]"),
            Some(32_768)
        );
        // 裸名不再因缺后缀被排除（输出上限与窗口后缀无关）
        assert_eq!(default_max_output_tokens("glm", "glm-5.2"), Some(32_768));
        assert_eq!(default_max_output_tokens("glm", "glm-5.1"), Some(32_768));
        // 5.3 系（官方上限 128K）与 5V 系：保守 32K，用户可 yaml 显式调高
        assert_eq!(default_max_output_tokens("glm", "glm-5.3"), Some(32_768));
        assert_eq!(
            default_max_output_tokens("glm", "glm-5.3-flash"),
            Some(32_768)
        );
        assert_eq!(
            default_max_output_tokens("glm", "glm-5v-turbo"),
            Some(32_768)
        );
        assert_eq!(
            default_max_output_tokens("deepseek", "deepseek-v4-flash-vision-exp"),
            Some(32_768)
        );
    }

    #[test]
    fn output_unknown_model_returns_none() {
        assert_eq!(default_max_output_tokens("", "some-custom-model"), None);
    }

    // --- 模态能力表（model_capabilities / effective_supports_vision）---

    #[test]
    fn vision_known_models_supported() {
        assert!(model_capabilities("openai", "gpt-4o").vision);
        assert!(model_capabilities("openai", "GPT-4o-mini").vision);
        assert!(model_capabilities("anthropic", "claude-sonnet-4-20250514").vision);
        assert!(model_capabilities("anthropic", "claude-3-5-sonnet").vision);
        assert!(model_capabilities("minimax", "MiniMax-M3").vision);
        assert!(model_capabilities("glm", "glm-4v").vision);
        assert!(model_capabilities("", "gemini-1.5-pro").vision);
        assert!(model_capabilities("qwen", "qwen2-vl-72b").vision);
        // 2026-08 新增：GLM-5 系首个原生多模态 + 5V 系 + 4.5v/4.1v 存量补漏
        assert!(model_capabilities("glm", "glm-5.3-flash").vision);
        assert!(model_capabilities("glm", "GLM-5.3-Flash").vision);
        assert!(model_capabilities("glm", "glm-5v-turbo").vision);
        assert!(model_capabilities("glm", "glm-4.5v").vision);
        assert!(model_capabilities("glm", "glm-4.1v-thinking").vision);
        // DeepSeek API 视觉实验模型（2026-08-21 上线）
        assert!(model_capabilities("deepseek", "deepseek-v4-flash-vision-exp").vision);
    }

    #[test]
    fn vision_text_only_models_false() {
        // MiniMax-M2 不支持视觉（只有 M3 支持，M2 不匹配 minimax-m3）
        assert!(!model_capabilities("minimax", "MiniMax-M2").vision);
        // deepseek-v4 chat 不含 -vl → 保守 false（避免误报导致图硬发给非视觉模型）
        assert!(!model_capabilities("deepseek", "deepseek-v4-pro").vision);
        // glm-5.2（coding）不含 4v → 保守 false
        assert!(!model_capabilities("glm", "glm-5.2").vision);
        // ⚠️ glm-5.3 旗舰是纯文本（官方文档明示「仅支持处理文本模态」）——
        // 只有 -flash 后缀是多模态，宽匹配 "glm-5.3" 会把图硬发给它 → 400
        assert!(!model_capabilities("glm", "glm-5.3").vision);
        assert!(!model_capabilities("glm-coding", "glm-5.3[1m]").vision);
        // 未知模型保守 false
        assert!(!model_capabilities("", "some-custom-llm").vision);
    }

    #[test]
    fn effective_vision_agent_override_is_authoritative() {
        // agent 显式 supports_vision=1 → 永远 true，即便模型表不认
        assert!(effective_supports_vision(1, "", "unknown-text-model"));
    }

    #[test]
    fn effective_vision_auto_detects_when_agent_unspecified() {
        // agent supports_vision=0（未配置）但 MiniMax-M3 模型表支持 → 自动 true
        assert!(effective_supports_vision(0, "minimax", "MiniMax-M3"));
        // agent =0 且模型也不支持 → false
        assert!(!effective_supports_vision(0, "deepseek", "deepseek-v4-pro"));
        // agent =0 且未知模型 → false
        assert!(!effective_supports_vision(0, "", "some-custom-llm"));
    }
}
