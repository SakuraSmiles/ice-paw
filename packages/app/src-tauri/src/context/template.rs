//! 模板渲染（P2-4）
//!
//! 从 `commands/chat_context.rs` 迁入（W5.1）。
//!
//! 提供两个 `pub(crate)` 函数：
//! - [`render_template`] — 把 `{{var_name}}` 变量替换为 `values` 中的值
//! - [`is_valid_var_name`] — 校验变量名合法性（字母/下划线开头 + 字母/数字/下划线）
//!
//! 与 mustache 的差异：
//! - 不支持 `{{#section}}...{{/section}}` / `{{! comment}}` / `{{>partial}}` 等高级语法
//! - 不支持 `.` 路径访问
//!
//! 故意保持简单：模板只是「带变量的纯文本」，不引入模板引擎依赖。

use std::collections::HashMap;

/// 用变量值渲染模板内容。
///
/// 规则：扫描文本中的 `{{var_name}}` 段，依次替换为 `values` 中对应 key 的值。
/// - 变量名必须是 `[a-zA-Z_][a-zA-Z0-9_]*`
/// - 模板中出现的 `var_name` 不在 `values` 中：保持原样（`{{var_name}}`）
///   以便 LLM 能看到「未填的占位符」并主动追问
/// - `values` 中多余的 key 会被忽略
pub(crate) fn render_template(template: &str, values: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // 查找下一个 {{
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // 寻找匹配的 }}
            let mut j = i + 2;
            let mut found = None;
            while j + 1 < bytes.len() {
                if bytes[j] == b'}' && bytes[j + 1] == b'}' {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = found {
                // 取出变量名（trim 空白）
                let name_raw = &template[i + 2..end];
                let name = name_raw.trim();
                // 校验变量名合法性
                if is_valid_var_name(name) {
                    if let Some(v) = values.get(name) {
                        out.push_str(v);
                    } else {
                        // 未提供的变量：保持原样
                        out.push_str(&template[i..end + 2]);
                    }
                } else {
                    // 非法变量名：保持原样
                    out.push_str(&template[i..end + 2]);
                }
                i = end + 2;
                continue;
            }
        }
        // 加上当前字符
        out.push(template[i..].chars().next().unwrap());
        i += template[i..].chars().next().unwrap().len_utf8();
    }
    out
}

/// 变量名合法性：字母/下划线开头 + 字母/数字/下划线
pub(crate) fn is_valid_var_name(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    true
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn render_replaces_known_vars() {
        let mut v = HashMap::new();
        v.insert("language".into(), "Rust".into());
        v.insert("framework".into(), "Actix".into());
        let out = render_template("请用 {{language}} + {{framework}} 实现", &v);
        assert_eq!(out, "请用 Rust + Actix 实现");
    }

    #[test]
    fn render_keeps_unknown_vars_intact() {
        let v = vals(&[("lang", "TS")]);
        let out = render_template("Hello {{name}} in {{lang}}", &v);
        assert_eq!(out, "Hello {{name}} in TS");
    }

    #[test]
    fn render_handles_no_vars() {
        let v = HashMap::new();
        assert_eq!(render_template("plain text", &v), "plain text");
    }

    #[test]
    fn render_handles_unicode_value() {
        let v = vals(&[("city", "北京")]);
        let out = render_template("我在 {{city}}", &v);
        assert_eq!(out, "我在 北京");
    }

    #[test]
    fn render_rejects_invalid_var_name_passthrough() {
        // 变量名含空格 / 点 / 数字开头 → 不替换
        let v = vals(&[("good", "OK")]);
        let out = render_template("a {{good}} b {{1bad}} c {{a.b}} d", &v);
        assert_eq!(out, "a OK b {{1bad}} c {{a.b}} d");
    }

    #[test]
    fn render_handles_extra_values() {
        // values 中多余的 key → 忽略
        let v = vals(&[("a", "1"), ("b", "2"), ("c", "3")]);
        let out = render_template("{{a}}/{{b}}", &v);
        assert_eq!(out, "1/2");
    }

    #[test]
    fn render_unmatched_brackets_kept_intact() {
        // 单独的 { 或 } 不应影响
        let v = vals(&[("x", "Y")]);
        let out = render_template("a { single } b {{x}} c { unclosed", &v);
        assert_eq!(out, "a { single } b Y c { unclosed");
    }

    #[test]
    fn render_adjacent_vars() {
        let v = vals(&[("a", "X"), ("b", "Y")]);
        assert_eq!(render_template("{{a}}{{b}}", &v), "XY");
    }

    #[test]
    fn is_valid_var_name_basic() {
        assert!(is_valid_var_name("foo"));
        assert!(is_valid_var_name("_bar"));
        assert!(is_valid_var_name("a1_b2"));
        assert!(!is_valid_var_name(""));
        assert!(!is_valid_var_name("1abc"));
        assert!(!is_valid_var_name("a-b"));
        assert!(!is_valid_var_name("a.b"));
    }
}
