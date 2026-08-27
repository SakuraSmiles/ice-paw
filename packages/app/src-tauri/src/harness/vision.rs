//! `harness::vision` — Vision（视觉/图片识别）平台配置解析。
//!
//! 扫描件/图片型附件文本提取为空，只能靠视觉模型读图。视觉路由为**两档制**
//!（2026-08-27 收敛，取代旧三环 fallback）：
//!
//! - agent 模型有视觉（`effective_supports_vision`）→ 图直发 agent，本模块不参与。
//! - agent 模型无视觉 → 用**平台视觉配置**（设置-通用-视觉读取）代读：主模型 +
//!   可选降级链（[`resolve_vision_entries`]），按序尝试、首个成功即用，marker 标注
//!   实际读图模型。未配置 → 剥图 + 诚实提示（绝不伪造）。
//!
//! 旧三环（agent 借凭据 / GLM 视觉 MCP env 借 key）已删除：借凭据不借端点是
//! 假环（Coding key 打标准端点必败），静默换人不可见；显式配置链把「谁读图」
//! 变成用户可说清、可测试（`test_vision_config`）的事。
//!
//! 与 embedding 的关系：embedding 配置给 KB 向量生成（[`crate::harness::kb::embedding`]），
//! vision 配置给附件图片识别；两者结构对称、各自独立。

use crate::db::models::{UserPreferences, VisionConfigEntry};
use crate::error::{AppError, AppResult};

/// OCR/读图提示词——让视觉模型把文档图片转成结构化文本（扫描件读图的"描述"）。
const VISION_OCR_PROMPT: &str = "\
你是一个 OCR 助手。请仔细识别并完整转录这张文档图片中的全部文字内容，\
保持原有的段落、列表与层次结构；如遇表格，用 markdown 表格还原。\
若有公式/图注，照原样转写。只输出识别到的正文内容，不要添加任何解释、前言或总结。";

/// provider 的默认视觉端点（OpenAI 兼容，`describe_image` 走 image_url 格式）。
///
/// 注意与聊天注册表（PROVIDERS default_url）**不是同一张表**：MiniMax 聊天走
/// Anthropic 协议端点、视觉走 OpenAI 兼容 `/v1`，故视觉端点在本模块单独成表。
fn default_vision_base_url(provider: &str) -> Option<&'static str> {
    match provider {
        "openai" => Some("https://api.openai.com"),
        "glm" | "glm-coding" => Some("https://open.bigmodel.cn/api/paas/v4"),
        "deepseek" => Some("https://api.deepseek.com"),
        // MiniMax 聊天走 Anthropic 协议（/anthropic），但视觉走 OpenAI 兼容端点（/v1），
        // 因为 describe_image 用 image_url 格式（OpenAI 协议）。同一 API key 两端点通用。
        "minimax" | "minimax-cn" => Some("https://api.minimaxi.com/v1"),
        _ => None,
    }
}

/// 把一个配置条目解析成可用凭据；无效条目（provider 未知 / model 或 key 空）→ None。
///
/// 端点成对原则：条目自带 `base_url` 优先，缺省按 provider 推导——key 与端点
/// 属同一鉴权域，永远不猜第三方的。
pub fn entry_to_credential(entry: &VisionConfigEntry, index: usize) -> Option<VisionCredential> {
    let provider = entry.provider.trim();
    let model = entry.model.trim();
    let api_key = entry.api_key.trim();
    if provider.is_empty() || model.is_empty() || api_key.is_empty() {
        return None;
    }
    let base_url = match entry.base_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(u) => u.to_string(),
        None => default_vision_base_url(provider)?.to_string(),
    };
    Some(VisionCredential {
        provider: provider.to_string(),
        model: model.to_string(),
        base_url,
        api_key: api_key.to_string(),
        source: format!("视觉配置#{index}"),
    })
}

/// 解析平台视觉配置为**有序凭据链**（主模型在前，降级依次）。
///
/// - `vision_config = Some(entries)` → 新格式权威（含 `Some(vec![])` = 显式清空 → 空链）。
/// - `vision_config = None` → 旧四键（vision_provider/model/api_key/base_url）拼成单条目
///   （读侧兼容，存量用户零迁移；前端首次保存后即转新格式）。
/// - 无效条目跳过不阻塞（日志提示），返回剩余可用链。
///
/// 纯函数，便于单测「前端 JSON 存储能否被正确解析为凭据链」。
pub fn resolve_vision_entries(prefs: &UserPreferences) -> Vec<VisionCredential> {
    let entries: Vec<VisionConfigEntry> = match &prefs.vision_config {
        Some(entries) => entries.clone(),
        None => {
            // 旧四键回落：拼成单条目（字段不齐 = 未配置，entry_to_credential 会过滤）
            vec![VisionConfigEntry {
                provider: prefs.vision_provider.clone().unwrap_or_default(),
                model: prefs.vision_model.clone().unwrap_or_default(),
                api_key: prefs.vision_api_key.clone().unwrap_or_default(),
                base_url: prefs.vision_base_url.clone(),
            }]
        }
    };
    entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let cred = entry_to_credential(e, i + 1);
            if cred.is_none() {
                tracing::warn!(
                    target: "ice_paw.vision",
                    provider = %e.provider, model = %e.model,
                    "视觉配置条目 #{i} 无效（provider 未知或字段缺失），已跳过"
                );
            }
            cred
        })
        .collect()
}

/// vision 是否已配置（至少一条有效凭据）。供 UI / 路由判断。
pub fn is_vision_configured(prefs: &UserPreferences) -> bool {
    !resolve_vision_entries(prefs).is_empty()
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
// 视觉凭据（平台视觉配置的一条，见 [`resolve_vision_entries`])
// ---------------------------------------------------------------------------
// 非视觉 agent 的图按条目顺序逐个试 describe_image：主模型 → 降级① → …；
// 每条失败不阻塞——继续下一条；全失败 → 如实告知（绝不伪造）。
// 旧三环（prefs 单配置 / agent 借凭据 / MCP env 借 key）已收敛为本配置链
// （2026-08-27）：链的组装权从系统暗门交还用户显式配置。
// ===========================================================================

/// 一个可用的视觉读取凭据（describe_image 用）。配置链的每一项解析出一个。
#[derive(Debug, Clone)]
pub struct VisionCredential {
    /// provider 标签（describe_image 路由用：glm/openai/...）
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    /// 来源标签（日志诊断用，如「视觉配置#1」）
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(provider: &str, model: &str, key: &str) -> VisionConfigEntry {
        VisionConfigEntry {
            provider: provider.into(),
            model: model.into(),
            api_key: key.into(),
            base_url: None,
        }
    }

    /// 旧四键拼出的 prefs（读侧兼容回落用）
    fn legacy_prefs(provider: &str, model: &str, key: &str, url: Option<&str>) -> UserPreferences {
        UserPreferences {
            vision_provider: Some(provider.into()),
            vision_model: Some(model.into()),
            vision_api_key: Some(key.into()),
            vision_base_url: url.map(String::from),
            ..Default::default()
        }
    }

    fn entries_prefs(entries: Vec<VisionConfigEntry>) -> UserPreferences {
        UserPreferences {
            vision_config: Some(entries),
            ..Default::default()
        }
    }

    // ---- 新格式：条目链 ----

    #[test]
    fn entries_resolve_in_order_with_default_urls() {
        let p = entries_prefs(vec![
            entry("glm", "glm-5.3-flash", "gk"),
            entry("deepseek", "deepseek-v4-flash-vision-exp", "dk"),
        ]);
        let chain = resolve_vision_entries(&p);
        assert_eq!(chain.len(), 2, "两条有效条目都在链上");
        assert_eq!(chain[0].model, "glm-5.3-flash");
        assert_eq!(chain[0].base_url, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(chain[1].model, "deepseek-v4-flash-vision-exp");
        assert_eq!(chain[1].base_url, "https://api.deepseek.com");
        // 来源标签带序号（日志诊断用）
        assert_eq!(chain[0].source, "视觉配置#1");
        assert_eq!(chain[1].source, "视觉配置#2");
    }

    #[test]
    fn entry_explicit_url_wins() {
        let mut e = entry("openai", "gpt-4o", "sk-x");
        e.base_url = Some("https://my.proxy/v1".into());
        let c = entry_to_credential(&e, 1).expect("显式 url");
        assert_eq!(c.base_url, "https://my.proxy/v1");
    }

    #[test]
    fn invalid_entries_are_skipped_not_blocking() {
        let p = entries_prefs(vec![
            entry("", "some-model", "k"),       // provider 空
            entry("glm", "  ", "k"),            // model 空（含空白）
            entry("deepseek", "m", ""),         // key 空
            entry("anthropic", "claude", "k"),  // provider 无视觉端点
            entry("minimax", "MiniMax-M3", "mk"), // 有效
        ]);
        let chain = resolve_vision_entries(&p);
        assert_eq!(chain.len(), 1, "只剩 minimax 一条有效");
        assert_eq!(chain[0].model, "MiniMax-M3");
    }

    #[test]
    fn explicit_empty_entries_means_unconfigured() {
        // Some(vec![]) = 用户显式清空 → 权威空链，不回落旧键（旧键可能还躺在库里）
        let mut p = legacy_prefs("glm", "glm-4v", "k", None);
        p.vision_config = Some(vec![]);
        assert!(resolve_vision_entries(&p).is_empty());
        assert!(!is_vision_configured(&p));
    }

    // ---- 旧格式回落：vision_config = None 时旧四键拼单条目 ----

    #[test]
    fn legacy_four_keys_fallback_to_single_entry() {
        let p = legacy_prefs("glm", "glm-4v-plus", "k", None);
        let chain = resolve_vision_entries(&p);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].model, "glm-4v-plus");
        assert_eq!(chain[0].base_url, "https://open.bigmodel.cn/api/paas/v4");
        assert!(is_vision_configured(&p));
    }

    #[test]
    fn legacy_with_explicit_url() {
        let p = legacy_prefs("openai", "gpt-4o", "sk-x", Some("https://my.proxy/v1"));
        let chain = resolve_vision_entries(&p);
        assert_eq!(chain[0].base_url, "https://my.proxy/v1");
    }

    #[test]
    fn legacy_missing_fields_is_unconfigured() {
        let mut p = legacy_prefs("openai", "gpt-4o", "sk-x", None);
        p.vision_model = None;
        assert!(resolve_vision_entries(&p).is_empty());

        let mut p = legacy_prefs("openai", "gpt-4o", "sk-x", None);
        p.vision_api_key = None;
        assert!(resolve_vision_entries(&p).is_empty());

        let p = legacy_prefs("anthropic", "claude", "k", None);
        assert!(resolve_vision_entries(&p).is_empty());
    }

    #[test]
    fn unconfigured_is_empty() {
        assert!(resolve_vision_entries(&UserPreferences::default()).is_empty());
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

    // ---- MiniMax 适配（M3 reasoning + 弃用字段 + 默认 URL）----

    #[test]
    fn minimax_entry_defaults_to_openai_compat_url() {
        // 条目选 minimax、不填 base_url → 默认 OpenAI 兼容端点 /v1（非聊天的 Anthropic 端点）
        let p = entries_prefs(vec![entry("minimax", "MiniMax-M3", "mk")]);
        let chain = resolve_vision_entries(&p);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].base_url, "https://api.minimaxi.com/v1");

        // minimax-cn 标签同源
        let p = entries_prefs(vec![entry("minimax-cn", "MiniMax-M3", "mk")]);
        assert_eq!(resolve_vision_entries(&p)[0].base_url, "https://api.minimaxi.com/v1");
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
