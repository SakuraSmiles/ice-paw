//! `harness::vision` — Vision（视觉/图片识别）全局配置解析（Phase B）。
//!
//! 扫描件/图片型附件文本提取为空，只能靠视觉模型读图。当前 Agent 若不支持视觉
//!（`supports_vision = 0`），附件图片识别 fallback 到此**全局 vision 配置**（独立于
//! 聊天 Agent，仿 embedding）。Agent 自带 `supports_vision = 1` 时优先用它自己的模型。
//!
//! 见 [`resolve_vision_config`] —— 纯函数，便于单测。
//!
//! 与 embedding 的关系：embedding 配置给 KB 向量生成（[`crate::harness::kb::embedding`]），
//! vision 配置给附件图片识别；两者结构对称、各自独立。

use crate::db::models::UserPreferences;

/// 已配置的 vision provider 标签（与 embedding 一致）。
pub const SUPPORTED_PROVIDERS: &[&str] = &["openai", "glm", "deepseek"];

/// 从 [`UserPreferences`] 解析 vision 配置 `(model, base_url, api_key)`。
///
/// `base_url` 缺省时按 `provider` 推导（openai / glm / deepseek）。`model`、`api_key`
/// 任一缺失或 provider 未知 → `None`（调用方回退：Agent 自带视觉模型，或治标诚实提示）。
///
/// 抽成纯函数，便于单测「前端 JSON 存储能否被正确解析为 vision 配置」。
pub fn resolve_vision_config(
    prefs: &UserPreferences,
) -> Option<(String, String, String)> {
    let model = prefs.vision_model.clone()?;
    let provider = prefs.vision_provider.as_deref()?;
    let api_key = prefs.vision_api_key.clone()?;
    let url = match prefs
        .vision_base_url
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        Some(u) => u.to_string(),
        None => match provider {
            "openai" => "https://api.openai.com".into(),
            "glm" => "https://open.bigmodel.cn/api/paas/v4".into(),
            "deepseek" => "https://api.deepseek.com".into(),
            _ => return None,
        },
    };
    Some((model, url, api_key))
}

/// vision 是否已配置（model + provider + api_key 齐全）。供 UI / 路由判断。
pub fn is_vision_configured(prefs: &UserPreferences) -> bool {
    resolve_vision_config(prefs).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(provider: &str, model: &str, key: &str, url: Option<&str>) -> UserPreferences {
        UserPreferences {
            vision_provider: Some(provider.into()),
            vision_model: Some(model.into()),
            vision_api_key: Some(key.into()),
            vision_base_url: url.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn resolves_with_explicit_url() {
        let p = prefs("openai", "gpt-4o", "sk-x", Some("https://my.proxy/v1"));
        let (m, url, k) = resolve_vision_config(&p).expect("explicit url");
        assert_eq!(m, "gpt-4o");
        assert_eq!(url, "https://my.proxy/v1");
        assert_eq!(k, "sk-x");
    }

    #[test]
    fn resolves_default_url_per_provider() {
        let p = prefs("glm", "glm-4v", "k", None);
        let (_, url, _) = resolve_vision_config(&p).expect("glm default");
        assert_eq!(url, "https://open.bigmodel.cn/api/paas/v4");
    }

    #[test]
    fn missing_model_or_key_returns_none() {
        let mut p = prefs("openai", "gpt-4o", "sk-x", None);
        p.vision_model = None;
        assert!(resolve_vision_config(&p).is_none());

        let mut p = prefs("openai", "gpt-4o", "sk-x", None);
        p.vision_api_key = None;
        assert!(resolve_vision_config(&p).is_none());
    }

    #[test]
    fn unknown_provider_returns_none() {
        let p = prefs("anthropic", "claude", "k", None);
        assert!(resolve_vision_config(&p).is_none());
    }

    #[test]
    fn unconfigured_is_none() {
        assert!(resolve_vision_config(&UserPreferences::default()).is_none());
        assert!(!is_vision_configured(&UserPreferences::default()));
    }
}
