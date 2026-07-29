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
/// - 操作系统类型（Windows / macOS / Linux）
/// - CPU 架构（如 x86_64 / arm64）
/// - 用户主目录路径（尽力获取，失败则省略）
/// - 时区信息（用户可设置，用于 LLM 理解本地时间上下文）
///
/// 用于帮助 LLM 在工具调用（如 `list_directory`）时使用与当前 OS 兼容的路径，
/// 避免在 Windows 上调用 Linux 风格的 `/home/user/Desktop` 等错误路径。
/// 时区信息帮助 LLM 在涉及时间的问题中给出符合用户当地时间的回答。
pub(crate) fn build_os_context(timezone: Option<&str>) -> String {
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
    // 使用 chrono 获取 UTC 时间；前端/用户设置时区后由 OsContextStage 传入
    let now = chrono::Utc::now();
    parts.push(format!("当前时间: {}", now.format("%Y-%m-%d %H:%M:%S UTC")));

    // 组装为提示文本
    let env_info = parts.join("\n");
    format!(
        "## 运行环境\n{}\n\n\
         注意：文件路径必须使用与当前操作系统兼容的格式。\
         调用工具时请使用绝对路径。",
        env_info
    )
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
        let ctx = build_os_context(None);
        assert!(ctx.contains("运行环境"));
        assert!(ctx.contains("操作系统"));
        assert!(ctx.contains("架构"));
        assert!(ctx.contains("当前时间"));
        assert!(ctx.contains("UTC"));
    }

    #[test]
    fn build_os_context_includes_timezone() {
        let ctx = build_os_context(Some("Asia/Shanghai"));
        assert!(ctx.contains("时区: Asia/Shanghai"));
    }

    #[test]
    fn build_os_context_empty_tz_omitted() {
        let ctx = build_os_context(Some(""));
        assert!(!ctx.contains("时区:"));
    }

    #[test]
    fn get_home_dir_returns_some_or_none() {
        // 在 CI 或 WSL 环境可能返回 None 或 Some，只需验证不 panic
        let _ = get_home_dir();
    }
}
