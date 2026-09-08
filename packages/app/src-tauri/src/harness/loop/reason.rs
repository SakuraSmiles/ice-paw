//! Retry reason 分类：将 [`AppError`] 映射为 retry reason 字符串。
//!
//! 从 `harness::loop_engine` 拆出（W2.6）。纯函数，无副作用，便于独立单元测试。

use crate::error::AppError;

// W2.6: 将 AppError 分类为 retry reason 字符串
pub(crate) fn classify_retry_reason(e: &AppError) -> String {
    use AppError::*;
    let msg = match e {
        Llm(s) | Stream(s) | Internal(s) | Stronghold(s) => s.as_str(),
        Io(_) => return "network_error".into(),
        Tauri(s) => s.as_str(),
        _ => return "unknown_error".into(),
    };
    let lower = msg.to_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        "timeout".into()
    } else if lower.contains("rate_limit")
        || lower.contains("429")
        || lower.contains("too many requests")
    {
        "rate_limited".into()
    } else if lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("server_error")
        || lower.contains("internal server error")
        || lower.contains("upstream")
    {
        "server_error_5xx".into()
    } else if lower.contains("connection")
        || lower.contains("network")
        || lower.contains("dns")
        || lower.contains("refused")
        || lower.contains("broken pipe")
        || lower.contains("reset")
    {
        "network_error".into()
    } else {
        "unknown_error".into()
    }
}

/// retry reason slug → 用户可读中文标签（B2-S1：RetryExhausted 终态文案用；
/// 与 `classify_retry_reason` 的五档词表成对维护，未收录 slug 原样透传）。
pub(crate) fn retry_reason_label(slug: &str) -> &str {
    match slug {
        "timeout" => "请求超时",
        "rate_limited" => "触发限流",
        "server_error_5xx" => "服务端错误",
        "network_error" => "网络错误",
        "unknown_error" => "未知错误",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::retry_reason_label;

    #[test]
    fn retry_reason_label_covers_known_slugs_and_passes_through_unknown() {
        // classify_retry_reason 的五档词表全收录（成对维护——新增档位两边一起补）
        assert_eq!(retry_reason_label("timeout"), "请求超时");
        assert_eq!(retry_reason_label("rate_limited"), "触发限流");
        assert_eq!(retry_reason_label("server_error_5xx"), "服务端错误");
        assert_eq!(retry_reason_label("network_error"), "网络错误");
        assert_eq!(retry_reason_label("unknown_error"), "未知错误");
        // 未收录 slug 原样透传（不吞不编）
        assert_eq!(retry_reason_label("some_new_slug"), "some_new_slug");
    }
}
