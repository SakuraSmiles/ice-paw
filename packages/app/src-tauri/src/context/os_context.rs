//! 运行环境上下文（B1-3）
//!
//! 从 `commands/chat_context.rs` 迁入（W5.2）。
//!
//! 提供两个 `pub(crate)` 函数：
//! - [`build_os_context`] — 构建运行环境上下文字符串（OS / 架构 / 主目录）
//! - [`get_home_dir`] — 尽力获取用户主目录

/// 构建运行环境上下文字符串，注入 system prompt
///
/// 包含：
/// - 操作系统类型 / CPU 架构 / 用户主目录 / 时区 / 当前时间
/// - Agent 配置目录（KB 和 agent.yaml 所在路径）
/// - 工作目录外的文件访问说明
pub(crate) fn build_os_context(timezone: Option<&str>, agent_workspace: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();

    // OS 类型
    let os_name = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    parts.push(format!("操作系统: {}", os_name));

    // CPU 架构（帮助 LLM 理解路径风格，如 arm64 vs x86_64）
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "arm64",
        other => other,
    };
    parts.push(format!("架构: {}", arch));

    // 用户主目录
    let home = get_home_dir();
    if let Some(h) = &home {
        parts.push(format!("用户主目录: {}", h));
    }

    // 用户时区（由用户偏好设置，辅助 LLM 理解本地时间上下文）
    if let Some(tz) = timezone {
        if !tz.is_empty() {
            parts.push(format!("时区: {}", tz));
        }
    }

    // 当前本地时间（基于用户时区，帮助 LLM 感知"现在"）
    let now = chrono::Utc::now();
    parts.push(format!("当前时间: {}", now.format("%Y-%m-%d %H:%M:%S UTC")));

    // Agent 配置目录（KB 和 agent.yaml 所在）
    if let Some(ws) = agent_workspace {
        parts.push(format!("Agent 配置目录: {}", ws));
    }

    // 组装为提示文本
    let env_info = parts.join("\n");
    let mut ctx = format!(
        "## 运行环境\n{}\n\n\
         注意：文件路径必须使用与当前操作系统兼容的格式。\
         调用工具时请使用绝对路径。",
        env_info
    );

    // 告知 Agent 可以操作工作目录外的文件
    ctx.push_str("\n\n文件工具（read_file/write_file/edit_file）默认在工作目录下操作。");
    ctx.push_str("如需访问工作目录外的文件，直接使用绝对路径即可，系统会询问用户确认。");
    ctx.push_str("同一会话中对同一路径批准一次后即可持续访问。");

    ctx
}

/// 尽力获取用户主目录
///
/// 优先级：
/// 1. Windows: %USERPROFILE%
/// 2. Unix (macOS/Linux): $HOME
/// 3. 兜底：返回 None
pub(crate) fn get_home_dir() -> Option<String> {
    // Windows: USERPROFILE
    if let Ok(p) = std::env::var("USERPROFILE") {
        if !p.is_empty() {
            return Some(p);
        }
    }
    // Unix: HOME
    if let Ok(p) = std::env::var("HOME") {
        if !p.is_empty() {
            return Some(p);
        }
    }
    None
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_os_context_contains_os() {
        let ctx = build_os_context(None, None);
        assert!(ctx.contains("运行环境"));
        assert!(ctx.contains("操作系统"));
        assert!(ctx.contains("架构"));
        assert!(ctx.contains("当前时间"));
        assert!(ctx.contains("UTC"));
    }

    #[test]
    fn build_os_context_includes_timezone() {
        let ctx = build_os_context(Some("Asia/Shanghai"), None);
        assert!(ctx.contains("时区: Asia/Shanghai"));
    }

    #[test]
    fn build_os_context_empty_tz_omitted() {
        let ctx = build_os_context(Some(""), None);
        assert!(!ctx.contains("时区:"));
    }

    #[test]
    fn get_home_dir_returns_some_or_none() {
        // 在 CI 或 WSL 环境可能返回 None 或 Some，只需验证不 panic
        let _ = get_home_dir();
    }
}
