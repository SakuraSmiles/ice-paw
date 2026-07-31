//! Chat 错误映射：AppError → 前端可读 kind / 友好中文提示
//!
//! 从 `commands/chat_error.rs` 迁入（W5.6）。
//!
//! - `error_kind(e)` — 把 AppError 映射为前端可读的 kind 字符串
//! - `friendly_error(msg)` — 把 LLM/Stream 错误消息映射为用户可读的中文友好提示

/// 把 AppError 映射为前端可读的 kind 字符串
pub(crate) fn error_kind(e: &crate::error::AppError) -> String {
    match e {
        crate::error::AppError::Llm(_) => "llm".into(),
        crate::error::AppError::Stream(_) => "stream".into(),
        crate::error::AppError::Cancelled => "cancelled".into(),
        crate::error::AppError::AuthorizationRequired { .. } => "auth_required".into(),
        _ => "internal".into(),
    }
}

/// 把 LLM/Stream 错误消息映射为用户可读的中文友好提示
///
/// 匹配逻辑：大小写不敏感地扫描常见错误关键词（图片安全审核 / 限流 /
/// 鉴权失败 / token 超限 等）。未匹配时返回原消息，方便开发者调试。
pub(crate) fn friendly_error(msg: &str) -> String {
    let lower = msg.to_lowercase();
    if lower.contains("sensitive") || lower.contains("content_filter") {
        return "图片内容未通过安全审核，请更换图片后重试".into();
    }
    if lower.contains("rate_limit") || lower.contains("rate limit") {
        return "请求过于频繁，请稍后再试".into();
    }
    if lower.contains("401") {
        return "API 密钥无效或已过期，请在设置中检查".into();
    }
    if lower.contains("403") {
        return "API 权限不足，请检查配置".into();
    }
    if lower.contains("context_length") || lower.contains("token") {
        return "消息过长，请缩短内容或清除部分历史消息".into();
    }
    msg.to_string()
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_error_sensitive_content() {
        let raw = "LLM 调用错误: HTTP 500 Internal Server Error: api_error: \
                   input new_sensitive, messages[21]'s content[0] image is sensitive (1026)";
        let out = friendly_error(raw);
        assert!(out.contains("安全审核"), "sensitive: {out}");
        assert!(!out.contains("HTTP 500"));
    }

    #[test]
    fn friendly_error_content_filter() {
        let raw = "API returned 400: content_filter triggered";
        assert!(friendly_error(raw).contains("安全审核"));
    }

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

    #[test]
    fn friendly_error_context_length() {
        assert!(friendly_error("context_length_exceeded: max 8192 tokens").contains("过长"));
        assert!(friendly_error("Too many tokens in prompt").contains("过长"));
    }

    #[test]
    fn friendly_error_unknown_passthrough() {
        let raw = "Some random network glitch XYZ123";
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
