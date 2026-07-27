//! 公共入参校验函数
//!
//! ## 用途
//! 集中放置 REQ-SEC-004 / REQ-SEC-004A 要求的入参校验逻辑，
//! 供各 command 模块统一调用。
//!
//! ## 长度上限（CSCI v4 阶段 1 规格）
//! - `name`：50 字符
//! - `description`：500 字符
//! - `system_prompt`：5,000 字符
//! - `content`（模板正文）：50,000 字符
//! - `input`（聊天发送内容）：32,768 字符（≤ 32KB）
//! - `value`（偏好 value）：32,768 字符（≤ 32KB）
//!
//! ## 名称白名单
//! `[\w\u4e00-\u9fff\s\-._()]+`
//! - `\w` = `[A-Za-z0-9_]`（Rust regex 默认不开启 Unicode 单词字符）
//! - `\u4e00-\u9fff` = CJK Unified Ideographs（基本汉字）
//! - `\s` = 空白字符
//! - `\-._()` = 允许的标点

use regex::Regex;

use crate::error::AppError;

/// 32KB 字节上限（用于聊天输入/偏好 value 等）
pub const MAX_INPUT_LEN: usize = 32_768;

/// 各字段长度上限（与 CSCI v4 规格对齐）
pub const MAX_NAME_LEN: usize = 50;
pub const MAX_DESCRIPTION_LEN: usize = 500;
/// REQ-PROJ-001a：项目描述上限 200 字符（短于通用 description 500）。
///
/// 与 agent description / template description 共用 MAX_DESCRIPTION_LEN=500 不同，
/// 项目描述需要更紧凑（卡片视图展示空间有限），故单独定义。
pub const MAX_PROJECT_DESCRIPTION_LEN: usize = 200;
pub const MAX_SYSTEM_PROMPT_LEN: usize = 5_000;
/// REQ-TMPL-001b：模板 user_prompt_prefix 长度上限（与 system_prompt 保持一致）。
///
/// 模板的 user_prompt_prefix 与 system_prompt 在概念上同属「内容字段」，
/// 故沿用 5,000 字符上限，前端 maxlength 与后端校验保持对齐。
pub const MAX_USER_PROMPT_PREFIX_LEN: usize = 5_000;
pub const MAX_CONTENT_LEN: usize = 50_000;

/// 白名单正则：允许的字符合集。
///
/// - `[A-Za-z0-9_]` 替代 `\w`：确保只匹配 ASCII 单词字符，
///   而非 Rust regex crate 默认的 Unicode `\w`（含 accented letters 等）。
/// - `\u4e00-\u9fff`：CJK 基本汉字区间（不含扩展区）。
/// - `*` 量词：允许空字符串；非空校验由调用方负责。
const NAME_CHARS_PATTERN: &str = r"^[A-Za-z0-9_\u4e00-\u9fff\s\-._()]*$";

/// 惰性编译（避免每次调用都重新编译正则）
/// `OnceLock` 是 1.70+ std 提供的线程安全一次性初始化容器。
/// `lazy_static!` 也可，但 `OnceLock` 已被标准库采纳，无第三方依赖。
fn name_regex() -> &'static Regex {
    static REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    REGEX.get_or_init(|| {
        // unwrap 安全：pattern 是固定字符串常量，编译不可能失败
        Regex::new(NAME_CHARS_PATTERN).expect("name pattern must compile")
    })
}

/// REQ-SEC-004：字符串长度校验
///
/// 当 `value` 的字符数（注意是 char 数而非 byte 数——前端用户看到的是字符）
/// 超过 `max_len` 时返回 `AppError::Validation`。
///
/// 注意：使用 `chars().count()` 而非 `value.len()`，以保证对多字节字符（如汉字）按字符计数，
/// 避免一个汉字（3 byte UTF-8）被算作 3 个字符而误判。
///
/// ## 失败消息格式
/// `"{field} 长度超过上限（{max_len} 字符），当前 {actual} 字符"`
pub fn validate_string_length(value: &str, field: &'static str, max_len: usize) -> Result<(), AppError> {
    let actual = value.chars().count();
    if actual > max_len {
        return Err(AppError::validation(
            field,
            format!("长度超过上限（{max_len} 字符），当前 {actual} 字符"),
        ));
    }
    Ok(())
}

/// REQ-SEC-004A：名称参数字符白名单校验
///
/// 仅允许：`\w`（字母数字下划线）、`\u4e00-\u9fff`（基本汉字）、`\s`（空白）、
/// `\-`、`\.`、`_`、`(`、`)`。
///
/// ## 失败消息格式
/// `"{field} 包含非法字符：{value}"`（截断展示，避免泄露超长内容）
///
/// ## 边界
/// - 空字符串：允许（与 `name` 长度校验配合使用，由调用方决定是否拒绝空名）
/// - 全空白：允许（与 `name.trim().is_empty()` 配合使用）
pub fn validate_name_chars(value: &str, field: &'static str) -> Result<(), AppError> {
    if !name_regex().is_match(value) {
        // 截断展示值（最多 50 字符），避免错误消息无限长
        let preview: String = value.chars().take(50).collect();
        let ellipsis = if value.chars().count() > 50 { "..." } else { "" };
        return Err(AppError::validation(
            field,
            format!("包含非法字符：{preview}{ellipsis}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // validate_string_length
    // -----------------------------------------------------------------

    #[test]
    fn validate_length_accepts_short() {
        assert!(validate_string_length("hello", "name", 50).is_ok());
    }

    #[test]
    fn validate_length_accepts_exact_max() {
        let s = "a".repeat(50);
        assert!(validate_string_length(&s, "name", 50).is_ok());
    }

    #[test]
    fn validate_length_rejects_over() {
        let s = "a".repeat(51);
        let err = validate_string_length(&s, "name", 50).unwrap_err();
        match err {
            AppError::Validation { field, message } => {
                assert_eq!(field, "name");
                assert!(message.contains("50"));
                assert!(message.contains("51"));
            }
            e => panic!("expected Validation, got {e:?}"),
        }
    }

    #[test]
    fn validate_length_counts_chars_not_bytes() {
        // "中" 是 1 个字符但 3 个字节——按字符计为 1
        let s = "中";
        // max=1：恰好 1 字符应通过
        assert!(validate_string_length(s, "name", 1).is_ok());
        // max=0：任何非空都失败
        assert!(validate_string_length(s, "name", 0).is_err());
    }

    #[test]
    fn validate_length_input_max_is_32kb() {
        // 32768 字符应通过
        let s = "a".repeat(32_768);
        assert!(validate_string_length(&s, "input", MAX_INPUT_LEN).is_ok());
        // 32769 字符应失败
        let s = "a".repeat(32_769);
        assert!(validate_string_length(&s, "input", MAX_INPUT_LEN).is_err());
    }

    #[test]
    fn validate_length_empty_string_ok() {
        assert!(validate_string_length("", "description", 500).is_ok());
    }

    // -----------------------------------------------------------------
    // validate_name_chars
    // -----------------------------------------------------------------

    #[test]
    fn validate_name_accepts_alnum_underscore() {
        assert!(validate_name_chars("hello_world_123", "name").is_ok());
        assert!(validate_name_chars("MyAgent1", "name").is_ok());
    }

    #[test]
    fn validate_name_accepts_cjk() {
        assert!(validate_name_chars("小明", "name").is_ok());
        assert!(validate_name_chars("小明的助手", "name").is_ok());
    }

    #[test]
    fn validate_name_accepts_whitespace_and_punct() {
        assert!(validate_name_chars("hello world", "name").is_ok());
        assert!(validate_name_chars("my-agent.v1", "name").is_ok());
        assert!(validate_name_chars("agent_(copy)", "name").is_ok());
    }

    #[test]
    fn validate_name_rejects_special_chars() {
        // XSS 尝试
        let err = validate_name_chars("<script>alert(1)</script>", "name").unwrap_err();
        match err {
            AppError::Validation { field, .. } => assert_eq!(field, "name"),
            e => panic!("expected Validation, got {e:?}"),
        }
        // SQL 注入尝试
        assert!(validate_name_chars("'; DROP TABLE agents;--", "name").is_err());
        // 路径分隔符
        assert!(validate_name_chars("../etc/passwd", "name").is_err());
        // 常见 shell 字符
        assert!(validate_name_chars("foo&bar", "name").is_err());
        assert!(validate_name_chars("foo|bar", "name").is_err());
        assert!(validate_name_chars("foo$bar", "name").is_err());
        // 其它 Unicode（CJK 扩展区——超出 \u9fff）
        assert!(validate_name_chars("𠮷", "name").is_err());
    }

    #[test]
    fn validate_name_accepts_empty() {
        // 空字符串本身是白名单的子集；非空性由调用方单独校验
        assert!(validate_name_chars("", "name").is_ok());
    }

    #[test]
    fn validate_name_rejects_accented_letters() {
        // 白名单使用 [A-Za-z0-9_]（ASCII-only），不含 accented letters
        assert!(validate_name_chars("Café", "name").is_err());
        // 但 CJK 基本汉字在 \u4e00-\u9fff 范围内，应通过
        assert!(validate_name_chars("编程助手", "name").is_ok());
    }
}
