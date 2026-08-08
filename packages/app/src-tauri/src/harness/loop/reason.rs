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
