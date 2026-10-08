//! 字符串工具：字节安全的截断等。
//!
//! `String::truncate(n)` / `&s[..n]` 在 `n` 非 UTF-8 字符边界时 **panic**
//!（`is_char_boundary` 断言失败）。工具输出截断（git/shell 等含中文的输出）
//! 必须用这里的函数，避免多字节文本触发 panic——release 是 `panic=unwind` +
//! 工具执行有 `dispatch_catch_panic` 兜底，但根本不应在截断处 panic。

/// 把字符串截断到 ≤ `max_bytes` 字节，回退到最近的 UTF-8 字符边界。
///
/// 与 [`String::truncate`] 不同，此函数**永不 panic**：若 `max_bytes` 落在多字节
/// 字符中间，向前回退到字符边界再截。超长时追加 `suffix`（suffix 长度从
/// `max_bytes` 预留，保证「截断正文 + suffix」总长 ≤ `max_bytes`）。
///
/// 用于限制返回给 LLM 的工具输出字节数（git/shell 等中文输出）。
pub(crate) fn truncate_to_byte_boundary(s: &str, max_bytes: usize, suffix: Option<&str>) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // 预留 suffix 空间，保证「截断正文 + suffix」总长 ≤ max_bytes
    let reserve = suffix.map(|sf| sf.len()).unwrap_or(0);
    let mut cut = max_bytes.saturating_sub(reserve);
    // 回退到最近的 char 边界（绝不切到多字节字符中间）
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut + reserve);
    out.push_str(&s[..cut]);
    if let Some(sf) = suffix {
        out.push_str(sf);
    }
    out
}

/// 错误文本中的凭据模式掩码（2026-10-08 顺手批）。
///
/// 部分 API/网关在报错体里回显 key 片段（智谱 1113 族、某些 401 网关）——
/// 错误原文会进 session_events / 持久日志 / 前端横幅，此处对两类模式做
/// **防御性**掩码（不依赖对端行为实证）：
/// - `sk-` 前缀 key：`sk-[A-Za-z0-9_-]{8,}` → `sk-***`
/// - Bearer 凭据：`bearer xxxxx`（大小写不敏感）→ `Bearer ***`
///
/// 掩码后的 `***` 不会再命中模式（`*` 不在字符类），幂等。
pub(crate) fn mask_credentials(text: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static SK: OnceLock<Regex> = OnceLock::new();
    static BEARER: OnceLock<Regex> = OnceLock::new();
    let bearer = BEARER.get_or_init(|| {
        Regex::new(r"(?i)bearer\s+[A-Za-z0-9._~+/=-]{8,}").expect("静态凭据正则必合法")
    });
    let sk = SK.get_or_init(|| Regex::new(r"sk-[A-Za-z0-9_-]{8,}").expect("静态凭据正则必合法"));
    let masked = bearer.replace_all(text, "Bearer ***");
    sk.replace_all(&masked, "sk-***").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_within_limit_unchanged() {
        assert_eq!(truncate_to_byte_boundary("hello", 10, None), "hello");
    }

    #[test]
    fn ascii_truncate_with_suffix() {
        let out = truncate_to_byte_boundary("abcdef", 4, Some(".."));
        // reserve suffix(2) → cut<=2 → "ab" + ".."，总长 ≤ 4
        assert_eq!(out, "ab..");
        assert!(out.len() <= 4);
    }

    #[test]
    fn cjk_long_truncate_never_panics_and_on_boundary() {
        // 3 字节中文，超长截断：cut 回退到字符边界（不 panic）
        let s = "中".repeat(10_000); // 30000 字节
        let out = truncate_to_byte_boundary(&s, 20_000, Some("\n...[输出已截断]"));
        assert!(out.ends_with("\n...[输出已截断]"));
        let body = out.strip_suffix("\n...[输出已截断]").unwrap();
        // 正文每个字符 3 字节 → 截断点必为 3 的倍数
        assert_eq!(body.len() % 3, 0, "body len {} 不是 3 的倍数", body.len());
        assert!(out.len() <= 20_000);
        // 确是有效 UTF-8（未被切到字符中间）
        assert!(std::str::from_utf8(body.as_bytes()).is_ok());
    }

    #[test]
    fn cjk_max_in_middle_of_char_falls_back() {
        // max=8 落在第三个字符（字节 6..9）中间 → 回退到 6（两个完整字符）
        let out = truncate_to_byte_boundary("中文测试", 8, None);
        assert_eq!(out, "中文");
    }

    #[test]
    fn empty_string() {
        assert_eq!(truncate_to_byte_boundary("", 10, None), "");
    }

    #[test]
    fn exactly_at_length() {
        // max 恰等于字符串长度 → 走 <= 分支原样返回
        assert_eq!(truncate_to_byte_boundary("abc", 3, None), "abc");
    }

    #[test]
    fn mask_credentials_covers_both_patterns() {
        // sk- key（含 - _ 字符类成员）
        assert_eq!(
            mask_credentials("鉴权失败: sk-abc123XYZ_9-cred 无效"),
            "鉴权失败: sk-*** 无效"
        );
        // Bearer（大小写不敏感、token 含 . _ ~ + / = -）
        assert_eq!(
            mask_credentials("401: bearer eyJhbGciOi.JOSE.abc 失效"),
            "401: Bearer *** 失效"
        );
        assert_eq!(mask_credentials("Bearer abc.def-ghi_jkl"), "Bearer ***");
        // 普通文本零改动；短于 8 的 token 不误伤
        assert_eq!(mask_credentials("余额不足 1113，请充值"), "余额不足 1113，请充值");
        assert_eq!(mask_credentials("token: sk-short"), "token: sk-short");
        // 幂等（已掩码不再命中）
        assert_eq!(mask_credentials("Bearer ***"), "Bearer ***");
    }
}
