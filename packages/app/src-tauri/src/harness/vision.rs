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
/// minimax 仅 MiniMax-M3 支持图片输入（M2.x 不支持），但 M3 是当前主力多模态模型，故纳入。
pub const SUPPORTED_PROVIDERS: &[&str] = &["openai", "glm", "deepseek", "minimax"];

/// 从 [`UserPreferences`] 解析 vision 配置 `(model, base_url, api_key)`。
///
/// `base_url` 缺省时按 `provider` 推导（openai / glm / deepseek）。`model`、`api_key`
/// 任一缺失或 provider 未知 → `None`（调用方回退：Agent 自带视觉模型，或治标诚实提示）。
///
/// 抽成纯函数，便于单测「前端 JSON 存储能否被正确解析为 vision 配置」。
pub fn resolve_vision_config(prefs: &UserPreferences) -> Option<(String, String, String)> {
    let model = prefs.vision_model.clone()?;
    let provider = prefs.vision_provider.as_deref()?;
    let api_key = prefs.vision_api_key.clone()?;
    let url = match prefs.vision_base_url.as_deref().filter(|s| !s.is_empty()) {
        Some(u) => u.to_string(),
        None => match provider {
            "openai" => "https://api.openai.com".into(),
            "glm" => "https://open.bigmodel.cn/api/paas/v4".into(),
            "deepseek" => "https://api.deepseek.com".into(),
            // MiniMax 聊天走 Anthropic 协议（/anthropic），但视觉走 OpenAI 兼容端点（/v1），
            // 因为 describe_image 用 image_url 格式（OpenAI 协议）。同一 API key 两端点通用。
            "minimax" | "minimax-cn" => "https://api.minimaxi.com/v1".into(),
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
/// `provider ∈ {openai, glm, deepseek, minimax}`，均走 **OpenAI Chat Completions + image_url** 格式，
/// 故同一实现兼容全部。endpoint 由 [`build_chat_endpoint`] 规整。
/// MiniMax 特殊处理（[`is_minimax`]）：① `max_tokens` 已弃用，改发 `max_completion_tokens`；
/// ② M3 默认 adaptive thinking，响应 content 前缀 `<think>...</think>`，用 [`strip_reasoning`] 剥掉。
///
/// `media_type` 决定 `data:` URL 的 MIME 标签（如 `image/png` / `image/jpeg`）；图片字节随原样
/// base64 编码、不转码，故 `media_type` 必须与 `bytes` 真实格式一致（OpenAI/GLM 视觉端点按
/// MIME 解码）。失败（HTTP/解析）归一为 `AppError::Internal`，调用方把错误文本写进 tool_result。
pub async fn describe_image(
    provider: &str,
    model: &str,
    base_url: &str,
    api_key: &str,
    media_type: &str,
    bytes: &[u8],
) -> AppResult<String> {
    use base64::Engine as _;

    let endpoint = build_chat_endpoint(base_url);
    let data_url = format!(
        "data:{media_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    let mut body = serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": VISION_OCR_PROMPT},
                {"type": "image_url", "image_url": {"url": data_url}}
            ]
        }]
    });
    // 输出长度上限：MiniMax 已弃用 max_tokens（官方改用 max_completion_tokens）；
    // 其它 OpenAI 兼容 provider 仍用 max_tokens。两者语义等价，统一 2048（OCR 够用）。
    if is_minimax(provider) {
        body["max_completion_tokens"] = serde_json::json!(2048);
    } else {
        body["max_tokens"] = serde_json::json!(2048);
    }

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
    // MiniMax-M3 等 reasoning 模型默认 adaptive thinking，content 前缀 <think>...</think>；
    // OCR 只要纯识别文本，剥掉前缀推理块（对非 reasoning 模型为 no-op）。
    Ok(strip_reasoning(content).trim().to_string())
}

/// 是否为 MiniMax provider（minimax / minimax-cn 两标签同源，均走 MiniMax OpenAI 兼容端点）。
fn is_minimax(provider: &str) -> bool {
    matches!(provider, "minimax" | "minimax-cn")
}

/// 剥掉 reasoning 模型（MiniMax-M3 等）在 content 前缀的 `<think>...</think>` 推理块。
///
/// M3 默认 adaptive thinking，响应 `choices[0].message.content` 形如
/// `<think>…推理…</think>\n实际答案`。OCR 场景只要识别文本，取首个 `</think>` 之后的部分；
/// 无 `</think>` 则原样返回（GLM-4v / gpt-4o 等非 reasoning 模型不受影响，no-op）。
fn strip_reasoning(content: &str) -> &str {
    match content.find("</think>") {
        Some(idx) => content[idx + "</think>".len()..].trim_start(),
        None => content,
    }
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
    ///
    /// `media_type` 必须与 `bytes` 真实格式一致（如 pdfium 渲染恒为 `image/png`；
    /// 用户上传图可能是 `image/jpeg` / `image/gif` / `image/webp`）。
    pub async fn describe(&self, bytes: &[u8], media_type: &str) -> AppResult<String> {
        describe_image(
            &self.provider,
            &self.model,
            &self.base_url,
            &self.api_key,
            media_type,
            bytes,
        )
        .await
    }
}

/// provider 的默认视觉模型（fallback 用；用户可在 vision config 显式覆盖成更强模型）。
/// 只列能走 OpenAI 兼容 image_url 的通用视觉模型；deepseek/anthropic 不在此列
///（deepseek 标准 API 无视觉模型；anthropic 走另一套 image block 格式，不兼容 image_url）。
/// MiniMax 仅 M3 支持图片输入（M2.x 不支持多模态）。
fn default_vision_model(provider: &str) -> Option<&'static str> {
    match provider {
        "glm" => Some("glm-4v"),
        "openai" => Some("gpt-4o"),
        "minimax" | "minimax-cn" => Some("MiniMax-M3"),
        _ => None,
    }
}

/// provider 的默认 base_url（与 [`resolve_vision_config`] 的默认 URL 同源）。
fn default_provider_base_url(provider: &str) -> Option<String> {
    match provider {
        "openai" => Some("https://api.openai.com".into()),
        "glm" => Some("https://open.bigmodel.cn/api/paas/v4".into()),
        "deepseek" => Some("https://api.deepseek.com".into()),
        "minimax" | "minimax-cn" => Some("https://api.minimaxi.com/v1".into()),
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

/// ② 当前 agent 自己的凭据——零配置兜底：GLM→glm-4v / OpenAI→gpt-4o / MiniMax→MiniMax-M3，
/// 用 agent 的 key。其它 provider（deepseek/anthropic/...）无通用视觉模型，返回 None。
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
    fn from_agent_minimax_borrows_m3() {
        // minimax / minimax-cn 两标签都应映射到 MiniMax-M3（M3 是 MiniMax 唯一支持图片的模型）
        let m = from_agent("minimax", "mk").expect("minimax 有视觉 (M3)");
        assert_eq!(m.provider, "minimax");
        assert_eq!(m.model, "MiniMax-M3");
        assert_eq!(m.base_url, "https://api.minimaxi.com/v1");
        assert_eq!(m.api_key, "mk");
        assert!(m.source.contains("agent"));

        let cn = from_agent("minimax-cn", "ck").expect("minimax-cn 同源 M3");
        assert_eq!(cn.model, "MiniMax-M3");
        assert_eq!(cn.base_url, "https://api.minimaxi.com/v1");
    }

    #[test]
    fn from_agent_non_vision_provider_is_none() {
        // deepseek/anthropic 无通用视觉模型 → 不能借 agent key
        //（minimax 已支持，见 from_agent_minimax_borrows_m3）
        assert!(from_agent("deepseek", "k").is_none());
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

    // ---- MiniMax 适配（M3 reasoning + 弃用字段 + 默认 URL）----

    #[test]
    fn resolve_vision_config_minimax_default_url() {
        // 显式 vision config 选 minimax、不填 base_url → 默认 OpenAI 兼容端点 /v1
        let p = prefs("minimax", "MiniMax-M3", "mk", None);
        let (m, url, k) = resolve_vision_config(&p).expect("minimax 可解析");
        assert_eq!(m, "MiniMax-M3");
        assert_eq!(url, "https://api.minimaxi.com/v1");
        assert_eq!(k, "mk");
    }

    #[test]
    fn is_minimax_matches_both_labels() {
        assert!(is_minimax("minimax"));
        assert!(is_minimax("minimax-cn"));
        assert!(!is_minimax("glm"));
        assert!(!is_minimax("openai"));
        assert!(!is_minimax("deepseek"));
    }

    #[test]
    fn strip_reasoning_removes_minimax_think_block() {
        // MiniMax-M3 官方响应实测：content 前缀 <think>...</think>
        let raw = "<think>\nThe user wants OCR.\n</think>\n这是识别出的正文。";
        assert_eq!(strip_reasoning(raw), "这是识别出的正文。");
    }

    #[test]
    fn strip_reasoning_noop_without_think_block() {
        // GLM-4v / gpt-4o 等非 reasoning 模型无 <think>，原样返回
        assert_eq!(strip_reasoning("纯文本"), "纯文本");
        assert_eq!(strip_reasoning(""), "");
    }
}
