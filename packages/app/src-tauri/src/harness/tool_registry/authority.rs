//! 工具权限策略 — 路径白名单 + 用户确认授权 (A2-3)
//!
//! W5.5: 实现 `PathWhitelistConfig` + `is_path_allowed()` 判断逻辑。
//! A2-3: 新增 `AuthorizationDecision` 枚举表达「直接放行 / 需要用户确认 / 拒绝」
//!       三态授权结果，配套 `PathAuthSession` 跟踪「本次会话已授权的路径」。
//!
//! 设计要点：
//! - `AuthorizationDecision::Allow` — 路径在白名单内 / 工具级别为 `Always`，直接放行
//! - `AuthorizationDecision::Confirm { .. }` — 需要前端弹窗确认
//!   - 路径白名单校验未通过（`PathWhitelist` 工具请求非白名单路径）
//!   - 工具级别本身就是 `Confirm`
//! - `AuthorizationDecision::Deny { .. }` — 永久拒绝（保留语义，当前未使用，留作扩展）
//!
//! **会话级记忆**：本模块的 `PathAuthSession`（独立类型，本文件内）保存「本轮
//! 流式生成中已被用户 `Allow` 的路径」。同一会话后续相同路径不再弹窗。
//! 会话结束由上层（`tool_executor::execute_tool_round` 结束或取消）调用
//! `clear()` 清空。

use std::collections::HashSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::AuthorizationLevel;

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
// AuthorizationDecision — 授权决策 (A2-3)
// =========================================================================

/// A2-3: 授权决策结果
///
/// 由 `check_authorization_with_session()` 返回，调用方根据结果决定
/// 直接执行 / 弹窗 / 拒绝。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthorizationDecision {
    /// 直接放行（路径在白名单 / 工具 Always）
    Allow,
    /// 需要用户在前端弹窗确认（已携带上下文：tool_name / path / request_id / args）
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
    /// 永久拒绝（保留：未来扩展按 allowlist / 工具级别做硬阻断）
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
// PathAuthSession — 会话级已授权路径表 (A2-3)
// =========================================================================

/// 会话级已授权路径表（A2-3）
///
/// 跟踪「本次 LLM 流式循环中已被用户 `Allow` 的路径」，
/// 同一会话内再次访问相同路径不再弹窗。
/// 会话结束 / 流式取消时由上层调 `clear()` 清空。
///
/// 线程安全：用 `tokio::sync::Mutex` 保护内部 `HashSet`。
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

    /// 同步插入（测试便利）
    #[cfg(test)]
    pub async fn mark_authorized_sync_for_test(&self, path: &str) {
        self.mark_authorized(path).await;
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
// check_authorization_with_session — 升级版授权判断 (A2-3)
// =========================================================================

/// A2-3: 检查工具授权，结合会话级已授权路径表。
///
/// 决策逻辑：
/// - `Always` → `Allow`
/// - `Confirm` → 总是 `Confirm`（前端弹窗；同会话内同 tool_name 不会重复触发，
///   因为每次工具调用都会生成新的 `request_id`）
/// - `PathWhitelist`:
///   - 路径在白名单 → `Allow`
///   - 路径在会话已授权集合中 → `Allow`（不重复弹窗）
///   - 都不满足 → `Confirm`（前端弹窗询问用户是否本次会话内放行）
///
/// `tool_args` 用于把当前工具调用的参数 JSON 透传给前端展示。
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
            if is_path_allowed(path, config) {
                AuthorizationDecision::Allow
            } else if session.is_authorized(path).await {
                // 本会话内用户已确认过该路径 → 免弹窗放行
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

// =========================================================================
// 兼容旧 API — 同步 / 无会话版本 (保留以避免破坏已有测试)
// =========================================================================

/// 非白名单路径的授权错误（保留 — 旧 API / 测试用）
pub fn path_not_whitelisted_error(tool: &str, path: &str) -> crate::error::AppError {
    crate::error::AppError::AuthorizationRequired {
        tool: tool.to_string(),
        reason: format!("路径 '{}' 不在白名单中", path),
    }
}

/// 检查工具授权（同步版本，旧 API）
///
/// - `Always`：直接放行
/// - `PathWhitelist`：检查路径白名单
/// - `Confirm`：暂未实现，视为拒绝（保留旧行为）
///
/// **注意**：A2-3 起，工具执行路径（`tool_executor::execute_tool_round`）
/// 不再依赖此函数；改用 `check_authorization_with_session()`。
/// 本函数保留是为了不破坏 `tests` 模块里的既有测试用例。
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

    // ----- 旧白名单逻辑（兼容） -----

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

        // 第一次：路径不在白名单 → Confirm
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

        // 用户确认后写入会话
        session.mark_authorized("/tmp/foo.txt").await;

        // 第二次：相同路径 → Allow（不再弹窗）
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

        // 已授权集合大小
        assert_eq!(session.len().await, 1);

        // 但不同路径仍需 Confirm
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
        // 验证 PathAuthSession 的 Clone 语义是共享同一 HashSet
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