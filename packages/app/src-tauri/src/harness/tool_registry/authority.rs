//! 工具权限策略 — 路径白名单判断
//!
//! W5.5: 实现 `PathWhitelistConfig` + `is_path_allowed()` 判断逻辑。
//!
//! 当工具的 `AuthorizationLevel` 为 `PathWhitelist` 时，在执行前检查
//! 路径是否在允许列表内。白名单为空时默认拒绝所有路径。

use crate::error::{AppError, AppResult};

use super::AuthorizationLevel;

/// 路径白名单配置
#[derive(Debug, Clone, Default)]
pub struct PathWhitelistConfig {
    /// 允许的路径前缀列表（例如 `["~/workspace/", "/tmp/"]`）
    pub allowed_paths: Vec<String>,
}

/// 判断给定路径是否在白名单内
///
/// 规则：
/// - 白名单为空 → 全部拒绝（安全默认）
/// - 前缀匹配：路径以任一 `allowed_paths` 元素开头即放行
/// - 后续可扩展为 glob 匹配
pub fn is_path_allowed(path: &str, config: &PathWhitelistConfig) -> bool {
    if config.allowed_paths.is_empty() {
        return false;
    }
    config
        .allowed_paths
        .iter()
        .any(|allowed| path.starts_with(allowed))
}

/// 非白名单路径的授权错误
pub fn path_not_whitelisted_error(tool: &str, path: &str) -> AppError {
    AppError::AuthorizationRequired {
        tool: tool.to_string(),
        reason: format!("路径 '{}' 不在白名单中", path),
    }
}

/// 检查工具授权：根据 AuthorizationLevel 和配置决定是否放行
///
/// - `Always`：直接放行
/// - `PathWhitelist`：检查路径白名单
/// - `Confirm`：暂未实现，视为拒绝（未来扩展）
pub fn check_authorization(
    level: AuthorizationLevel,
    path: &str,
    config: &PathWhitelistConfig,
    tool_name: &str,
) -> AppResult<()> {
    match level {
        AuthorizationLevel::Always => Ok(()),
        AuthorizationLevel::PathWhitelist => {
            if is_path_allowed(path, config) {
                Ok(())
            } else {
                Err(path_not_whitelisted_error(tool_name, path))
            }
        }
        AuthorizationLevel::Confirm => Err(AppError::AuthorizationRequired {
            tool: tool_name.to_string(),
            reason: "此工具需要用户确认授权（功能尚未实现）".into(),
        }),
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn whitelist(paths: &[&str]) -> PathWhitelistConfig {
        PathWhitelistConfig {
            allowed_paths: paths.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn whitelist_allows() {
        let cfg = whitelist(&["/workspace/"]);
        assert!(is_path_allowed("/workspace/file.txt", &cfg));
    }

    #[test]
    fn whitelist_denies() {
        let cfg = whitelist(&["/workspace/"]);
        assert!(!is_path_allowed("/etc/passwd", &cfg));
    }

    #[test]
    fn whitelist_empty_denies_all() {
        let cfg = PathWhitelistConfig::default();
        assert!(!is_path_allowed("/any/path", &cfg));
        assert!(!is_path_allowed("/", &cfg));
    }

    #[test]
    fn whitelist_subpath() {
        let cfg = whitelist(&["/workspace/"]);
        assert!(is_path_allowed("/workspace/sub/inner.txt", &cfg));
        assert!(is_path_allowed("/workspace/sub/deep/file.rs", &cfg));
    }

    #[test]
    fn whitelist_exact_match() {
        let cfg = whitelist(&["/tmp/test.txt"]);
        assert!(is_path_allowed("/tmp/test.txt", &cfg));
        // 前缀匹配：/tmp/test.txt.bak 以 /tmp/test.txt 开头，所以放行
        assert!(is_path_allowed("/tmp/test.txt.bak", &cfg));
        // 但 /tmp/test2.txt 不以 /tmp/test.txt 开头
        assert!(!is_path_allowed("/tmp/test2.txt", &cfg));
    }

    #[test]
    fn whitelist_partial_rejected() {
        let cfg = whitelist(&["/workspace/"]);
        // /workspace-secret/ 不应以 /workspace/ 匹配（前缀不包含 -secret）
        assert!(!is_path_allowed("/workspace-secret/file.txt", &cfg));
    }

    #[test]
    fn check_authorization_always_passes() {
        let cfg = PathWhitelistConfig::default();
        assert!(check_authorization(AuthorizationLevel::Always, "/etc/passwd", &cfg, "test").is_ok());
    }

    #[test]
    fn check_authorization_whitelist_allowed() {
        let cfg = whitelist(&["/home/"]);
        assert!(check_authorization(
            AuthorizationLevel::PathWhitelist, "/home/doc.txt", &cfg, "read_file"
        ).is_ok());
    }

    #[test]
    fn check_authorization_whitelist_denied() {
        let cfg = whitelist(&["/home/"]);
        let result = check_authorization(
            AuthorizationLevel::PathWhitelist, "/etc/passwd", &cfg, "read_file"
        );
        assert!(result.is_err());
    }

    #[test]
    fn check_authorization_confirm_rejected() {
        let cfg = whitelist(&["/home/"]);
        let result = check_authorization(
            AuthorizationLevel::Confirm, "/home/doc.txt", &cfg, "some_tool"
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("需要用户确认"));
    }
}
