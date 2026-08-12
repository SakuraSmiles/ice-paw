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

// ===========================================================================
// 多级凭据 fallback（Phase B+：扫描件代读零配置最大化）
// ---------------------------------------------------------------------------
// 非视觉 agent 调 view_attachment_image 时，按优先级收集视觉凭据，逐个试 describe_image：
//   ① 显式 vision config（用户在「设置-视觉读取」配的——精确控制 model/url，最高优先级）
//   ② 当前 agent 自己的凭据（GLM→glm-4v / OpenAI→gpt-4o，零配置兜底）
//   ③ 已配的视觉 MCP server 的 env（智谱 GLM 视觉 MCP 的 Z_AI_API_KEY→glm-4v，零配置兜底）
//   ④ 都没有 / 都失败 → 如实告知（绝不伪造）
//
// 设计要点：解析逻辑全为纯函数（便于单测）；DB 查询（取 agent / 列 MCP）由调用方做，
// 把候选 Vec<VisionCredential> 组装好后顺序试。某条凭据失效（401/网络）不阻塞——继续下一条。
// ===========================================================================

/// 一个可用的视觉读取凭据（describe_image 用）。多级 fallback 的每一级解析出一个。
#[derive(Debug, Clone)]
pub struct VisionCredential {
    /// provider 标签（describe_image 路由用：glm/openai/...）
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    /// 来源标签（日志 + tool_result.reader 用，便于诊断"图是谁读的"）
    pub source: String,
}

impl VisionCredential {
    /// 用本凭据识别一张图片，返回模型读出的文本（包 [`describe_image`]）。
    pub async fn describe(&self, png: &[u8]) -> AppResult<String> {
        describe_image(&self.provider, &self.model, &self.base_url, &self.api_key, png).await
    }
}

/// provider 的默认视觉模型（fallback 用；用户可在 vision config 显式覆盖成更强模型）。
/// 只列能走 OpenAI 兼容 image_url 的通用视觉模型；deepseek/minimax/anthropic 不在此列。
fn default_vision_model(provider: &str) -> Option<&'static str> {
    match provider {
        "glm" => Some("glm-4v"),
        "openai" => Some("gpt-4o"),
        _ => None,
    }
}

/// provider 的默认 base_url（与 [`resolve_vision_config`] 的默认 URL 同源）。
fn default_provider_base_url(provider: &str) -> Option<String> {
    match provider {
        "openai" => Some("https://api.openai.com".into()),
        "glm" => Some("https://open.bigmodel.cn/api/paas/v4".into()),
        "deepseek" => Some("https://api.deepseek.com".into()),
        _ => None,
    }
}

/// ① 显式 vision config（用户在「设置-视觉读取」配的）——最高优先级，精确控制 model/url。
/// 复用 [`resolve_vision_config`] 的解析（含 provider 默认 URL 推导）。
pub fn from_prefs(prefs: &UserPreferences) -> Option<VisionCredential> {
    let (model, base_url, api_key) = resolve_vision_config(prefs)?;
    let provider = prefs.vision_provider.clone().unwrap_or_default();
    Some(VisionCredential {
        provider,
        model,
        base_url,
        api_key,
        source: "vision 配置".into(),
    })
}

/// ② 当前 agent 自己的凭据——零配置兜底：GLM→glm-4v / OpenAI→gpt-4o，用 agent 的 key。
/// 其它 provider（deepseek/minimax/...）无通用视觉模型，返回 None。
pub fn from_agent(provider: &str, api_key: &str) -> Option<VisionCredential> {
    let model = default_vision_model(provider)?.to_string();
    let base_url = default_provider_base_url(provider)?;
    Some(VisionCredential {
        provider: provider.to_string(),
        model,
        base_url,
        api_key: api_key.to_string(),
        source: format!("agent:{provider}"),
    })
}

/// ③ 已配的视觉 MCP server 的 env 借凭据——零配置兜底第二环。
/// 识别智谱 GLM 视觉 MCP（`@z_ai/mcp-server`）env 里的 `Z_AI_API_KEY` → glm-4v。
/// 不实际启动/调用 MCP（只读静态配置 env），故无 stdio 往返、不依赖 MCP server 运行时状态。
/// 未来其它视觉 MCP 可在此扩展识别（如 OPENAI_API_KEY）。
pub fn from_mcp_env(env: &serde_json::Value) -> Option<VisionCredential> {
    let obj = env.as_object()?;
    if let Some(key) = obj
        .get("Z_AI_API_KEY")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(VisionCredential {
            provider: "glm".into(),
            model: "glm-4v".into(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
            api_key: key.to_string(),
            source: "MCP:GLM 视觉理解".into(),
        });
    }
    None
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

    // ---- 多级凭据 fallback ----

    #[test]
    fn from_agent_glm_and_openai_have_vision_model() {
        let g = from_agent("glm", "gk").expect("glm 有视觉");
        assert_eq!(g.provider, "glm");
        assert_eq!(g.model, "glm-4v");
        assert_eq!(g.base_url, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(g.api_key, "gk");
        assert!(g.source.contains("agent"));

        let o = from_agent("openai", "ok").expect("openai 有视觉");
        assert_eq!(o.model, "gpt-4o");
        assert_eq!(o.base_url, "https://api.openai.com");
    }

    #[test]
    fn from_agent_non_vision_provider_is_none() {
        // deepseek/minimax/anthropic 无通用视觉模型 → 不能借 agent key
        assert!(from_agent("deepseek", "k").is_none());
        assert!(from_agent("minimax", "k").is_none());
        assert!(from_agent("anthropic", "k").is_none());
        assert!(from_agent("unknown", "k").is_none());
    }

    #[test]
    fn from_mcp_env_borrows_zhipu_key() {
        let env = serde_json::json!({"Z_AI_API_KEY": "zhi-xxx", "Z_AI_MODE": "ZHIPU"});
        let c = from_mcp_env(&env).expect("有 Z_AI_API_KEY");
        assert_eq!(c.provider, "glm");
        assert_eq!(c.model, "glm-4v");
        assert_eq!(c.api_key, "zhi-xxx");
        assert!(c.source.contains("MCP"));
    }

    #[test]
    fn from_mcp_env_ignores_empty_or_missing_key() {
        assert!(from_mcp_env(&serde_json::json!({})).is_none());
        assert!(from_mcp_env(&serde_json::json!({"Z_AI_API_KEY": ""})).is_none());
        // 非 object
        assert!(from_mcp_env(&serde_json::json!(["a"])).is_none());
    }

    #[test]
    fn from_prefs_wraps_resolve_vision_config() {
        let p = prefs("glm", "glm-4v-plus", "k", None);
        let c = from_prefs(&p).expect("已配");
        // 用户显式配的 model 优先于默认
        assert_eq!(c.model, "glm-4v-plus");
        assert_eq!(c.provider, "glm");
        assert!(c.source.contains("vision"));
    }

    #[test]
    fn fallback_order_prefs_before_agent_before_mcp() {
        // 调用方负责组装顺序；这里只校验每级都能解析出正确凭据，
        // 且显式 model（prefs）不被 agent 默认 model 覆盖。
        let p = prefs("openai", "gpt-4o-mini", "pk", None);
        let from_p = from_prefs(&p).unwrap();
        let from_a = from_agent("openai", "ak").unwrap();
        assert_eq!(from_p.model, "gpt-4o-mini"); // 显式
        assert_eq!(from_a.model, "gpt-4o"); // 默认
    }
}
