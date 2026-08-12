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
use crate::error::{AppError, AppResult};

/// OCR/读图提示词——让视觉模型把文档图片转成结构化文本（扫描件读图的"描述"）。
const VISION_OCR_PROMPT: &str = "\
你是一个 OCR 助手。请仔细识别并完整转录这张文档图片中的全部文字内容，\
保持原有的段落、列表与层次结构；如遇表格，用 markdown 表格还原。\
若有公式/图注，照原样转写。只输出识别到的正文内容，不要添加任何解释、前言或总结。";

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

/// 用全局 vision 配置识别一张图片，返回模型读出的文本。
///
/// 扫描件/图片型 PDF：当前 agent 不支持视觉（`supports_vision = 0`）时，把 pdfium 渲染出
/// 的 PNG 发给**全局 vision 配置**指定的模型，让它把图读成文本回灌进 `tool_result`——
/// 这样非视觉 agent 也能"看到"扫描件内容（[`crate::db::models`] AgentRow 注释的 fallback 语义）。
///
/// `provider ∈ {openai, glm, deepseek}`，三者均走 **OpenAI Chat Completions + image_url** 格式，
/// 故同一实现兼容全部。endpoint 由 [`build_chat_endpoint`] 规整。
///
/// 失败（HTTP/解析）归一为 `AppError::Internal`，调用方把错误文本写进 tool_result 让 LLM 知悉。
pub async fn describe_image(
    provider: &str,
    model: &str,
    base_url: &str,
    api_key: &str,
    png: &[u8],
) -> AppResult<String> {
    use base64::Engine as _;

    let endpoint = build_chat_endpoint(base_url);
    let data_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    );
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 2048,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": VISION_OCR_PROMPT},
                {"type": "image_url", "image_url": {"url": data_url}}
            ]
        }]
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| AppError::Internal(format!("vision HTTP client 构造失败: {e}")))?;
    let resp = client
        .post(&endpoint)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("vision 请求失败 ({provider}): {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AppError::Internal(format!("vision 响应读取失败 ({provider}): {e}")))?;
    if !status.is_success() {
        let snippet: String = text.chars().take(500).collect();
        return Err(AppError::Internal(format!(
            "vision {provider} 返回 {status}: {snippet}"
        )));
    }

    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Internal(format!("vision 响应 JSON 解析失败 ({provider}): {e}")))?;
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| {
            let snippet: String = text.chars().take(300).collect();
            AppError::Internal(format!(
                "vision 响应缺少 choices[0].message.content ({provider}): {snippet}"
            ))
        })?;
    Ok(content.to_string())
}

/// 把 vision `base_url` 规整为 `{base}/chat/completions`（OpenAI 兼容）。
///
/// 容错：去尾部 `/`；已含 `/chat/completions` 则不重复追加。
fn build_chat_endpoint(base_url: &str) -> String {
    let b = base_url.trim_end_matches('/');
    if b.ends_with("/chat/completions") {
        b.to_string()
    } else {
        format!("{b}/chat/completions")
    }
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

    #[test]
    fn chat_endpoint_normalizes_base_urls() {
        assert_eq!(
            build_chat_endpoint("https://api.openai.com"),
            "https://api.openai.com/chat/completions"
        );
        // 去尾斜杠
        assert_eq!(
            build_chat_endpoint("https://open.bigmodel.cn/api/paas/v4/"),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        // 已含完整 endpoint 不重复追加
        assert_eq!(
            build_chat_endpoint("https://api.deepseek.com/chat/completions"),
            "https://api.deepseek.com/chat/completions"
        );
    }
}
