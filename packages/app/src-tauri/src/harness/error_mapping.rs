//! Chat 错误映射：AppError → 前端可读 kind / 友好中文提示
//!
//! 从 `commands/chat_error.rs` 迁入（W5.6）。
//!
//! - `error_kind(e)` — 把 AppError 映射为前端可读的 kind 字符串
//! - `friendly_error(msg)` — 把 LLM/Stream 错误消息映射为用户可读的中文友好提示
//!
//! 友好文案与重试策略的**分类真相源**在 [`crate::error::classify_llm_error`]（单一
//! 分类器，避免两处 substring 匹配同一文本而漂移）。本模块仅做 AppError → kind 标签
//! + 委托分类器产出友好文案。

// 重导出分类器，供 harness 各处（modal / attachment tool）通过 `error_mapping` 统一入口访问。
pub(crate) use crate::error::{classify_llm_error, LlmErrorKind};

/// 把 AppError 映射为前端可读的 kind 字符串。
/// LLM 错误展开细类 `llm.<slug>`（如 `llm.auth`）——前端错误横幅据此挂行动按钮
/// （鉴权/余额类 →「去检查配置」），分类真相源在 [`crate::error::classify_llm_error`]。
pub(crate) fn error_kind(e: &crate::error::AppError) -> String {
    match e {
        crate::error::AppError::Llm(msg) => {
            format!("llm.{}", classify_llm_error(msg).slug())
        }
        crate::error::AppError::Stream(msg) => {
            format!("llm.{}", classify_llm_error(msg).slug())
        }
        crate::error::AppError::Cancelled => "cancelled".into(),
        crate::error::AppError::AuthorizationRequired { .. } => "auth_required".into(),
        _ => "internal".into(),
    }
}

/// 把 LLM/Stream 错误消息映射为用户可读的中文友好提示。
///
/// 委托 [`classify_llm_error`]：命中已知分类返回对应中文提示；未识别（`Unknown`）
/// 返回原消息，便于开发者诊断。
pub(crate) fn friendly_error(msg: &str) -> String {
    let text = classify_llm_error(msg).friendly_text();
    if text.is_empty() {
        msg.to_string()
    } else {
        text.to_string()
    }
}

// ===========================================================================
// 单元测试
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 敏感/内容审核（缺口①：补全 Anthropic / OpenAI / 中文措辞）----

    #[test]
    fn friendly_error_sensitive_content() {
        let raw = "LLM 调用错误: HTTP 500 Internal Server Error: api_error: \
                   input new_sensitive, messages[21]'s content[0] image is sensitive (1026)";
        let out = friendly_error(raw);
        assert!(out.contains("安全审核"), "sensitive: {out}");
        assert!(!out.contains("HTTP 500"));
    }

    #[test]
    fn friendly_error_openai_content_policy() {
        // OpenAI 的 content_policy_violation（旧实现只匹配 content_filter，漏匹配）
        assert!(friendly_error("HTTP 400: content_policy_violation").contains("安全审核"));
        assert!(friendly_error("HTTP 400: content_filter triggered").contains("安全审核"));
        // 带空格的 "content filter"（旧 content_filter 下划线匹配不到）
        assert!(friendly_error("API returned 400: content filter").contains("安全审核"));
    }

    #[test]
    fn friendly_error_anthropic_safety_policy() {
        // Anthropic 的 content policy / safety（旧实现完全漏匹配 → 英文原文暴露）
        assert!(
            friendly_error("HTTP 400: invalid_request_error: content policy violation")
                .contains("安全审核")
        );
        assert!(friendly_error("HTTP 400: safety violation").contains("安全审核"));
    }

    #[test]
    fn friendly_error_sensitive_chinese() {
        assert!(friendly_error("图片内容违规，未通过审核").contains("安全审核"));
    }

    // ---- 限流 / 鉴权 / 权限 ----

    #[test]
    fn friendly_error_rate_limit() {
        let raw1 = "HTTP 429: rate_limit_exceeded";
        let raw2 = "Too Many Requests: rate limit reached, please retry after 30s";
        assert!(friendly_error(raw1).contains("过于频繁"));
        assert!(friendly_error(raw2).contains("过于频繁"));
    }

    #[test]
    fn friendly_error_401() {
        let out = friendly_error("HTTP 401 Unauthorized: invalid api key");
        assert!(out.contains("API 密钥"));
    }

    #[test]
    fn friendly_error_403() {
        let out = friendly_error("HTTP 403 Forbidden: insufficient permissions");
        assert!(out.contains("权限"));
    }

    // ---- 上下文超长 / 网络（旧实现 network 走 passthrough，现已友好化）----

    #[test]
    fn friendly_error_context_length() {
        assert!(friendly_error("context_length_exceeded: max 8192 tokens").contains("过长"));
        assert!(friendly_error("Too many tokens in prompt").contains("过长"));
    }

    #[test]
    fn friendly_error_network() {
        // 旧实现未覆盖网络层措辞 → passthrough 英文；现统一友好提示
        assert!(friendly_error("HTTP 502: bad gateway").contains("网络"));
        assert!(friendly_error("vision 请求失败 (glm): connection timeout").contains("网络"));
    }

    // ---- kind 细类展开（前端行动按钮的路由依据）----
    #[test]
    fn error_kind_llm_slug_expansion() {
        use crate::error::AppError;
        assert_eq!(
            error_kind(&AppError::Llm("HTTP 401 Unauthorized: invalid api key".into())),
            "llm.auth"
        );
        assert_eq!(
            error_kind(&AppError::Stream("HTTP 429: rate_limit_exceeded".into())),
            "llm.rate_limited"
        );
        assert_eq!(error_kind(&AppError::Cancelled), "cancelled");
        assert_eq!(
            error_kind(&AppError::NotFound { resource: "x", id: "1".into() }),
            "internal"
        );
    }

    // ---- 未识别：回落原文（可调试）----

    #[test]
    fn friendly_error_unknown_passthrough() {
        // 纯无关键词串 → 原样返回（开发期诊断）
        let raw = "some opaque internal glitch qzx-123";
        assert_eq!(friendly_error(raw), raw);
    }

    #[test]
    fn friendly_error_empty_string() {
        assert_eq!(friendly_error(""), "");
    }

    #[test]
    fn friendly_error_case_insensitive() {
        let raw = "HTTP 500 Internal Server Error: input SENSITIVE content";
        assert!(friendly_error(raw).contains("安全审核"));
    }
}
