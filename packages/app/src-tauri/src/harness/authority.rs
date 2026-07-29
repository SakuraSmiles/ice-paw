//! 工具权限策略 — 路径白名单 + 用户确认授权
//!
//! Phase 1: 从 `tool_registry/authority.rs` 迁出，位于 `harness/` 顶层。
//!
//! 设计要点：
//! - `PathWhitelistConfig` 定义路径白名单配置
//! - `PathAuthSession` 跟踪「本次会话已授权的路径」
//! - `AuthorizationDecision` 表达「直接放行 / 需要用户确认 / 拒绝」三态

use std::collections::HashSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::harness::mcp::AuthorizationLevel;

// =========================================================================
// PathWhitelistConfig — 路径白名单配置
// =========================================================================

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

// =========================================================================
// AuthorizationDecision — 授权决策
// =========================================================================

/// 授权决策结果
///
/// 由 `check_authorization_with_session()` 返回，调用方根据结果决定
/// 直接执行 / 弹窗 / 拒绝。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthorizationDecision {
    /// 直接放行（路径在白名单 / 工具 Always）
    Allow,
    /// 需要用户在前端弹窗确认
    Confirm {
        /// 唯一请求 ID（用于匹配前端响应）
        request_id: String,
        /// 工具名
        tool_name: String,
        /// 待访问的路径
        file_path: String,
        /// 工具调用参数（JSON 字符串，便于前端展示）
        arguments: String,
        /// 触发原因（前端展示文案）
        reason: String,
    },
    /// 永久拒绝（保留：未来扩展）
    Deny {
        reason: String,
    },
}

impl AuthorizationDecision {
    /// 是否直接放行
    pub fn is_allowed(&self) -> bool {
        matches!(self, AuthorizationDecision::Allow)
    }

    /// 是否需要前端弹窗
    pub fn needs_confirm(&self) -> bool {
        matches!(self, AuthorizationDecision::Confirm { .. })
    }
}

// =========================================================================
// PathAuthSession — 会话级已授权路径表
// =========================================================================

/// 会话级已授权路径表
///
/// 跟踪「本次 LLM 流式循环中已被用户 `Allow` 的路径」，
/// 同一会话内再次访问相同路径不再弹窗。
/// 会话结束 / 流式取消时由上层调 `clear()` 清空。
#[derive(Debug, Clone, Default)]
pub struct PathAuthSession {
    inner: Arc<Mutex<HashSet<String>>>,
}

impl PathAuthSession {
    /// 新建空会话
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// 当前路径是否已被本会话授权
    pub async fn is_authorized(&self, path: &str) -> bool {
        let set = self.inner.lock().await;
        set.contains(path)
    }

    /// 把路径加入已授权集合
    pub async fn mark_authorized(&self, path: &str) {
        let mut set = self.inner.lock().await;
        set.insert(path.to_string());
    }

    /// 清空会话授权（流式结束 / 取消时调用）
    pub async fn clear(&self) {
        let mut set = self.inner.lock().await;
        set.clear();
    }

    /// 已授权条目数（仅供测试 / 调试）
    #[cfg(test)]
    pub async fn len(&self) -> usize {
        let set = self.inner.lock().await;
        set.len()
    }
}

// =========================================================================
// check_authorization_with_session — 升级版授权判断
// =========================================================================

/// 检查工具授权，结合会话级已授权路径表。
///
/// 决策逻辑：
/// - `Always` → `Allow`
/// - `Confirm` → 总是 `Confirm`
/// - `PathWhitelist`:
///   - 路径在白名单 → `Allow`
///   - 路径在会话已授权集合中 → `Allow`
///   - 都不满足 → `Confirm`
pub async fn check_authorization_with_session(
    level: AuthorizationLevel,
    path: &str,
    config: &PathWhitelistConfig,
    tool_name: &str,
    tool_args: &str,
    session: &PathAuthSession,
) -> AuthorizationDecision {
    match level {
        AuthorizationLevel::Always => AuthorizationDecision::Allow,
        AuthorizationLevel::Confirm => AuthorizationDecision::Confirm {
            request_id: Uuid::new_v4().to_string(),
            tool_name: tool_name.to_string(),
            file_path: path.to_string(),
            arguments: tool_args.to_string(),
            reason: "此工具需要用户确认授权".to_string(),
        },
        AuthorizationLevel::PathWhitelist => {
            if is_path_allowed(path, config) || session.is_authorized(path).await {
                AuthorizationDecision::Allow
            } else {
                AuthorizationDecision::Confirm {
                    request_id: Uuid::new_v4().to_string(),
                    tool_name: tool_name.to_string(),
                    file_path: path.to_string(),
                    arguments: tool_args.to_string(),
                    reason: format!("路径 '{}' 不在白名单中，需要用户确认", path),
                }
            }
        }
    }
}

/// 非白名单路径的授权错误
pub fn path_not_whitelisted_error(tool: &str, path: &str) -> crate::error::AppError {
    crate::error::AppError::AuthorizationRequired {
        tool: tool.to_string(),
        reason: format!("路径 '{}' 不在白名单中", path),
    }
}

/// 检查工具授权（同步版本，旧 API）
///
/// 保留以确保与既有测试兼容；生产路径已改用 `check_authorization_with_session`。
pub fn check_authorization(
    level: AuthorizationLevel,
    path: &str,
    config: &PathWhitelistConfig,
    tool_name: &str,
) -> crate::error::AppResult<()> {
    match level {
        AuthorizationLevel::Always => Ok(()),
        AuthorizationLevel::PathWhitelist => {
            if is_path_allowed(path, config) {
                Ok(())
            } else {
                Err(path_not_whitelisted_error(tool_name, path))
            }
        }
        AuthorizationLevel::Confirm => Err(crate::error::AppError::AuthorizationRequired {
            tool: tool_name.to_string(),
            reason: "此工具需要用户确认授权（功能尚未实现）".into(),
        }),
    }
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn whitelist(paths: &[&str]) -> PathWhitelistConfig {
        PathWhitelistConfig {
            allowed_paths: paths.iter().map(|s| s.to_string()).collect(),
        }
    }

    // ----- 白名单逻辑 -----

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
        assert!(is_path_allowed("/tmp/test.txt.bak", &cfg));
        assert!(!is_path_allowed("/tmp/test2.txt", &cfg));
    }

    #[test]
    fn whitelist_partial_rejected() {
        let cfg = whitelist(&["/workspace/"]);
        assert!(!is_path_allowed("/workspace-secret/file.txt", &cfg));
    }

    // ----- 旧 check_authorization -----

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

    // ----- A2-3 新增测试 -----

    #[tokio::test]
    async fn decision_always_allows() {
        let cfg = whitelist(&["/home/"]);
        let session = PathAuthSession::new();
        let d = check_authorization_with_session(
            AuthorizationLevel::Always,
            "/etc/passwd",
            &cfg,
            "list_directory",
            "{}",
            &session,
        )
        .await;
        assert!(d.is_allowed());
        assert!(!d.needs_confirm());
    }

    #[tokio::test]
    async fn decision_pathwhitelist_in_whitelist_allows() {
        let cfg = whitelist(&["/workspace/"]);
        let session = PathAuthSession::new();
        let d = check_authorization_with_session(
            AuthorizationLevel::PathWhitelist,
            "/workspace/file.txt",
            &cfg,
            "read_file",
            r#"{"path":"/workspace/file.txt"}"#,
            &session,
        )
        .await;
        assert!(d.is_allowed());
    }

    #[tokio::test]
    async fn decision_pathwhitelist_outside_whitelist_needs_confirm() {
        let cfg = whitelist(&["/workspace/"]);
        let session = PathAuthSession::new();
        let d = check_authorization_with_session(
            AuthorizationLevel::PathWhitelist,
            "/etc/passwd",
            &cfg,
            "read_file",
            r#"{"path":"/etc/passwd"}"#,
            &session,
        )
        .await;
        assert!(d.needs_confirm());
        match d {
            AuthorizationDecision::Confirm {
                tool_name,
                file_path,
                request_id,
                reason,
                ..
            } => {
                assert_eq!(tool_name, "read_file");
                assert_eq!(file_path, "/etc/passwd");
                assert!(!request_id.is_empty());
                assert!(reason.contains("不在白名单"));
            }
            _ => panic!("应为 Confirm"),
        }
    }

    #[tokio::test]
    async fn decision_confirm_level_always_needs_confirm() {
        let cfg = whitelist(&["/workspace/"]);
        let session = PathAuthSession::new();
        let d = check_authorization_with_session(
            AuthorizationLevel::Confirm,
            "/workspace/file.txt",
            &cfg,
            "some_tool",
            "{}",
            &session,
        )
        .await;
        assert!(d.needs_confirm());
        match d {
            AuthorizationDecision::Confirm { tool_name, .. } => {
                assert_eq!(tool_name, "some_tool");
            }
            _ => panic!("应为 Confirm"),
        }
    }

    #[tokio::test]
    async fn session_marks_path_authorized_then_allows() {
        let cfg = whitelist(&["/workspace/"]);
        let session = PathAuthSession::new();

        let d1 = check_authorization_with_session(
            AuthorizationLevel::PathWhitelist,
            "/tmp/foo.txt",
            &cfg,
            "read_file",
            r#"{"path":"/tmp/foo.txt"}"#,
            &session,
        )
        .await;
        assert!(d1.needs_confirm());

        session.mark_authorized("/tmp/foo.txt").await;

        let d2 = check_authorization_with_session(
            AuthorizationLevel::PathWhitelist,
            "/tmp/foo.txt",
            &cfg,
            "read_file",
            r#"{"path":"/tmp/foo.txt"}"#,
            &session,
        )
        .await;
        assert!(d2.is_allowed());

        assert_eq!(session.len().await, 1);

        let d3 = check_authorization_with_session(
            AuthorizationLevel::PathWhitelist,
            "/tmp/bar.txt",
            &cfg,
            "read_file",
            r#"{"path":"/tmp/bar.txt"}"#,
            &session,
        )
        .await;
        assert!(d3.needs_confirm());
    }

    #[tokio::test]
    async fn session_clear_removes_authorizations() {
        let session = PathAuthSession::new();
        session.mark_authorized("/a").await;
        session.mark_authorized("/b").await;
        assert_eq!(session.len().await, 2);
        session.clear().await;
        assert_eq!(session.len().await, 0);
        assert!(!session.is_authorized("/a").await);
    }

    #[tokio::test]
    async fn session_clone_shares_state() {
        let session1 = PathAuthSession::new();
        let session2 = session1.clone();
        session1.mark_authorized("/shared").await;
        assert!(session2.is_authorized("/shared").await);
    }

    #[tokio::test]
    async fn decision_confirm_includes_arguments() {
        let cfg = PathWhitelistConfig::default();
        let session = PathAuthSession::new();
        let args = r#"{"path":"/etc/passwd","max_bytes":2048}"#;
        let d = check_authorization_with_session(
            AuthorizationLevel::PathWhitelist,
            "/etc/passwd",
            &cfg,
            "read_file",
            args,
            &session,
        )
        .await;
        match d {
            AuthorizationDecision::Confirm { arguments, .. } => {
                assert_eq!(arguments, args);
            }
            _ => panic!("应为 Confirm"),
        }
    }

    #[test]
    fn decision_is_allowed_helpers() {
        assert!(AuthorizationDecision::Allow.is_allowed());
        assert!(!AuthorizationDecision::Allow.needs_confirm());
        let c = AuthorizationDecision::Confirm {
            request_id: "x".into(),
            tool_name: "t".into(),
            file_path: "/p".into(),
            arguments: "{}".into(),
            reason: "r".into(),
        };
        assert!(!c.is_allowed());
        assert!(c.needs_confirm());
    }
}
