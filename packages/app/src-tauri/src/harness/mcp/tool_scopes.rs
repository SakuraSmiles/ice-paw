//! 工具集范围（`tool_scopes`）——组选择模型的单一真相源（2026-09-12 拍板批次）。
//!
//! ## 模型
//! agent.yaml / DB 列 `tool_scopes: Option<Vec<String>>`，条目三形态：
//! - `group:<组键>` —— 内置工具组（[`TOOL_GROUPS`] 固定名单快照）
//! - `server:<server 配置 id>` —— 外部 MCP server 全部工具（按归属探针
//!   `server_config_id()` 匹配，t{idx}_ 前缀漂移免疫——id 稳定不随删建复用变化）
//! - 裸工具名 —— 逃生舱（内置组没覆盖的 / 平台元工具外的单点）
//!
//! 缺省 / 空 = 全部工具（默认全开，与 enabled_tools「空 ≡ 全开」同一约定）。
//! 与既有 `enabled_tools` 白名单**串联**：先 scopes 过滤、再名单过滤（交集语义），
//! 存量 agent 零迁移零行为变化。
//!
//! ## 三层权限模型定位（勿混淆）
//! 本模块只管**组装可见性**（agent 能看到哪些工具）；运行时授权（AuthScope 四档
//! 弹卡）与 server 全局开关（mcp_servers.enabled）是另外两层——看得见 ≠ 免问，
//! **审批卡才是安全边界，收窄的价值是降噪 / 省 token / 缩小默认攻击面**。
//!
//! ## 组语义：固定名单快照（用户拍板 2026-09-12）
//! 组 = 建组时固化的工具名单，**新增内置工具不自动进组**——安全面不静默扩大；
//! 要放行新工具须显式改 scopes（用户可感知）。分组归属镜像设置页展示分组
//! （McpSettings BUILTIN_TOOL_META），本表为后端真相源，前端经 `list_builtin_tools`
//! 的 `group` 字段读取，勿双维护。

/// 内置工具组（有序：files/web/kb/attach/docx/config/screen）。
///
/// 名单快照 = 2026-09-12 时点 `register_builtin` 的 39 件全量分组（零遗漏，
/// 测试 `groups_cover_all_registered_tools` 锁定）；「其他」组是设置页展示兜底，
/// **不是 scope 组**（无固定名单语义，`group:other` 视为死条目——要放行未分组
/// 工具用裸工具名）。
pub const TOOL_GROUPS: &[(&str, &[&str])] = &[
    (
        "files",
        &[
            "read_file",
            "list_directory",
            "directory_tree",
            "get_file_info",
            "read_multiple_files",
            "write_file",
            "edit_file",
            "delete_file",
            "move_file",
            "copy_file",
            "create_directory",
            "search_files",
            "run_command",
            "git",
        ],
    ),
    ("web", &["web_fetch"]),
    (
        "kb",
        &["search_kb", "read_kb_document", "save_to_kb"],
    ),
    (
        "attach",
        &[
            "read_attachment_page",
            "view_attachment_image",
            "read_reference",
        ],
    ),
    (
        "docx",
        &["inspect_docx", "edit_docx", "validate_docx", "write_docx"],
    ),
    (
        "config",
        &["read_agent_config", "propose_config_change", "update_plan"],
    ),
    (
        "screen",
        &[
            "capture_screen",
            "list_windows",
            "capture_window",
            "mouse_move",
            "mouse_click",
            "mouse_drag",
            "mouse_scroll",
            "type_text",
            "press_key",
            "wait",
            "request_screen_session",
        ],
    ),
];

/// 组键是否合法（`group:<key>` 条目的 key 校验）
pub fn is_known_group(key: &str) -> bool {
    TOOL_GROUPS.iter().any(|(k, _)| *k == key)
}

/// 反查工具所属组（`list_builtin_tools` 的 group 字段数据源；未分组 → None，
/// 前端落「其他」组展示）
pub fn tool_group_of(tool: &str) -> Option<&'static str> {
    TOOL_GROUPS
        .iter()
        .find(|(_, members)| members.contains(&tool))
        .map(|(k, _)| *k)
}

/// scope 条目（解析后三态）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeEntry<'a> {
    Group(&'a str),
    Server(&'a str),
    Tool(&'a str),
}

/// 解析单条 scope 条目；空串 / 纯前缀（`group:` 无键）→ None（调用方计入死条目）
pub fn parse_scope_entry(raw: &str) -> Option<ScopeEntry<'_>> {
    let raw = raw.trim();
    if let Some(key) = raw.strip_prefix("group:") {
        return (!key.is_empty()).then_some(ScopeEntry::Group(key));
    }
    if let Some(id) = raw.strip_prefix("server:") {
        return (!id.is_empty()).then_some(ScopeEntry::Server(id));
    }
    (!raw.is_empty()).then_some(ScopeEntry::Tool(raw))
}

/// 过滤判定（纯函数）：工具名 + 归属 server id 在 scopes 下是否保留。
///
/// scopes 为空 = 全部保留（默认全开）。平台元工具的恒保留语义在调用方
/// （session_runner 的 PLATFORM_TOOLS）——本函数只判 scopes 本身。
pub fn scope_allows(scopes: &[String], name: &str, server_id: Option<&str>) -> bool {
    if scopes.is_empty() {
        return true;
    }
    scopes.iter().any(|raw| match parse_scope_entry(raw) {
        Some(ScopeEntry::Group(key)) => TOOL_GROUPS
            .iter()
            .find(|(k, _)| *k == key)
            .is_some_and(|(_, members)| members.contains(&name)),
        Some(ScopeEntry::Server(id)) => server_id == Some(id),
        Some(ScopeEntry::Tool(tool)) => tool == name,
        None => false,
    })
}

/// 死条目检测（纯函数）：对工具快照零命中的 scopes 条目。
///
/// 生产实案驱动（2026-09-12 设计调研）：3 个 agent 白名单里的 `read_kb` 不存在
/// （真名 `read_kb_document`）——**权限假象比缺工具危险**，零命中必须 warn 可见。
/// `server:<id>` 零命中的可能原因：server 已删除 / 已禁用（禁用的 server 工具
/// 不进快照，属预期而非错误——warn 文案由调用方给全口径）。
pub fn dead_scope_entries(
    scopes: &[String],
    tools: &[(String, Option<String>)],
) -> Vec<String> {
    scopes
        .iter()
        .filter(|raw| {
            let entry = raw.trim().to_string();
            !tools.iter().any(|(name, server_id)| {
                scope_allows(std::slice::from_ref(&entry), name, server_id.as_deref())
            })
        })
        .map(|raw| raw.trim().to_string())
        .collect()
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 组名单快照完整性锁：register_builtin 的 39 件内置工具全覆盖、无重复入组、
    /// 无组内重复。新增内置工具忘了进组 → 本测试红（固定名单快照语义的守门员，
    /// 提醒维护者显式决定新工具归属——安全面不静默扩大）。
    #[test]
    fn groups_cover_all_registered_tools() {
        let registered: &[&str] = &[
            // 只读 / 文件
            "read_file", "list_directory", "directory_tree", "get_file_info",
            "read_multiple_files",
            // Word 四件
            "inspect_docx", "edit_docx", "validate_docx", "write_docx",
            // KB
            "search_kb", "save_to_kb", "read_kb_document",
            // 附件与引用
            "read_attachment_page", "view_attachment_image", "read_reference",
            // agentic 工具集
            "write_file", "edit_file", "delete_file", "move_file", "copy_file",
            "create_directory", "run_command", "search_files", "git", "web_fetch",
            // 配置与计划
            "read_agent_config", "propose_config_change", "update_plan",
            // 屏幕十一件
            "capture_screen", "list_windows", "capture_window", "mouse_move",
            "mouse_click", "mouse_drag", "mouse_scroll", "type_text", "press_key",
            "wait", "request_screen_session",
        ];
        for tool in registered {
            assert!(
                tool_group_of(tool).is_some(),
                "内置工具 {tool} 未进任何组——固定名单快照要求显式归属（补 TOOL_GROUPS）"
            );
        }
        // 无重复入组
        let mut seen: Vec<&str> = Vec::new();
        for (_, members) in TOOL_GROUPS {
            for m in *members {
                assert!(!seen.contains(m), "工具 {m} 重复入组");
                seen.push(m);
            }
        }
        assert_eq!(seen.len(), registered.len(), "组内总件数应 = 注册件数（无多余幽灵名）");
    }

    #[test]
    fn parse_entry_three_forms() {
        assert_eq!(parse_scope_entry("group:files"), Some(ScopeEntry::Group("files")));
        assert_eq!(parse_scope_entry("server:ue5"), Some(ScopeEntry::Server("ue5")));
        assert_eq!(parse_scope_entry("run_command"), Some(ScopeEntry::Tool("run_command")));
        // 空串 / 纯前缀 / 纯空白 → None（死条目）
        assert_eq!(parse_scope_entry(""), None);
        assert_eq!(parse_scope_entry("group:"), None);
        assert_eq!(parse_scope_entry("   "), None);
        // 前后空白容错
        assert_eq!(parse_scope_entry("  group:kb  "), Some(ScopeEntry::Group("kb")));
    }

    #[test]
    fn scope_allows_group_server_tool() {
        let scopes = vec![
            "group:kb".to_string(),
            "server:ue5".to_string(),
            "run_command".to_string(),
        ];
        // 组命中
        assert!(scope_allows(&scopes, "search_kb", None));
        assert!(scope_allows(&scopes, "save_to_kb", None));
        // 组外内置不沾光
        assert!(!scope_allows(&scopes, "read_file", None));
        // server 按归属 id 命中（工具名无关——t 前缀漂移免疫）
        assert!(scope_allows(&scopes, "t6_call_tool", Some("ue5")));
        assert!(!scope_allows(&scopes, "t6_call_tool", Some("other")));
        assert!(!scope_allows(&scopes, "t6_call_tool", None));
        // 裸工具名命中
        assert!(scope_allows(&scopes, "run_command", None));
        assert!(!scope_allows(&scopes, "git", None));
        // 空 scopes = 全开
        assert!(scope_allows(&[], "anything", None));
        // 未知组键 = 该条永不命中
        let bad = vec!["group:nope".to_string()];
        assert!(!scope_allows(&bad, "read_file", None));
    }

    #[test]
    fn dead_entries_detected() {
        let scopes = vec![
            "group:kb".to_string(),
            "read_kb".to_string(),          // 死条目（真名 read_kb_document——生产实案）
            "server:gone".to_string(),      // server 已删
            "group:bogus".to_string(),      // 未知组键
        ];
        let tools = vec![
            ("search_kb".to_string(), None),
            ("read_kb_document".to_string(), None),
            ("t6_call_tool".to_string(), Some("ue5".to_string())),
        ];
        let dead = dead_scope_entries(&scopes, &tools);
        assert_eq!(dead, vec!["read_kb", "server:gone", "group:bogus"]);
    }

    /// server 条目对「禁用 server」的语义：工具不在快照 → 死条目 warn（预期行为，
    /// 由调用方文案给全口径「已删除或已禁用」）。
    #[test]
    fn disabled_server_counts_as_dead_in_snapshot() {
        let scopes = vec!["server:ue5".to_string()];
        let tools = vec![("read_file".to_string(), None)]; // 快照无 ue5 工具
        assert_eq!(dead_scope_entries(&scopes, &tools), vec!["server:ue5"]);
    }
}
