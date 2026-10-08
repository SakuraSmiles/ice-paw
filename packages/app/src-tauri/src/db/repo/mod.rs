//! 数据访问层入口
//!
//! 每个子模块对应一张表，函数全部是「纯 SQL + &SqlitePool」风格，
//! 不依赖 Tauri 状态，方便单测和复用。

pub mod agent;
pub mod conversation;
pub mod kb;
pub mod mcp_server;
pub mod memory_embedding;
pub mod memory_store;
pub mod message;
pub mod message_attachment;
pub mod message_attachment_file;
pub mod model_profile;
pub mod preferences;
pub mod project;
pub mod project_ledger;
pub mod session_event;
pub mod summary;
pub mod task;
pub mod template;
pub mod tool_call;

/// LIKE 模式通配符转义（`\` `%` `_` → `\\` `\%` `\_`），配合 SQL `ESCAPE '\'` 使用。
///
/// 用户检索词是字面量不是模式——不转义时 `file_name_v2` 的下划线匹配任意单
/// 字符、`100%` 的百分号匹配任意串（命中面失控 + 排序失真）。2026-10-08 P1：
/// kb 两处检索收口；discovery_tools 原内联版改引共享（单一真相源）。
pub fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::escape_like;

    /// 2026-10-08 P1：LIKE 字面量转义（kb 检索 `file_name_v2` 下划线通配失真收口）
    #[test]
    fn escape_like_escapes_wildcards() {
        assert_eq!(escape_like("file_name_v2"), "file\\_name\\_v2");
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a\\b"), "a\\\\b");
        assert_eq!(escape_like("普通词"), "普通词");
    }
}
