//! 工具权限策略 — 路径白名单 + 用户确认授权
//!
//! Phase 1: 从 `tool_registry/authority.rs` 迁出，位于 `harness/` 顶层。
//!
//! 设计要点：
//! - `PathWhitelistConfig` 定义路径白名单配置
//! - `PathAuthSession` 跟踪「本次会话已授权的路径」
//! - `AuthorizationDecision` 表达「直接放行 / 需要用户确认 / 拒绝」三态

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::harness::mcp::AuthorizationLevel;
use crate::infra::path_norm;

// =========================================================================
// PathWhitelistConfig — 路径白名单配置
// =========================================================================

/// 路径白名单配置
#[derive(Debug, Clone, Default)]
pub struct PathWhitelistConfig {
    /// 允许的路径列表（目录或文件，例如 `["~/workspace/", "/tmp/"]`）
    pub allowed_paths: Vec<String>,
}

/// 判断给定路径是否在白名单内
///
/// 规则：
/// - 白名单为空 → 全部拒绝（安全默认）
/// - 成员判定经 `path_norm::path_within`（词法归一 + 组件级前缀 + `..` 穿越
///   消解 + Windows 大小写）——不再手搓字符串 starts_with
pub fn is_path_allowed(path: &str, config: &PathWhitelistConfig) -> bool {
    if config.allowed_paths.is_empty() {
        return false;
    }
    let target = Path::new(path);
    config
        .allowed_paths
        .iter()
        .any(|allowed| path_norm::path_within(target, Path::new(allowed)))
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
    Deny { reason: String },
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
// PathAuthSession — 会话级分层授权记忆
// =========================================================================

/// 会话级分层授权记忆（#11）
///
/// 跟踪「本次 LLM 流式循环中已被用户 `Allow`」的授权，三档 grant：
/// 精确路径 / 目录（含子目录）/ 工具，判定序 **tool > dir > path**。
/// 会话结束 / 流式取消时由上层调 `clear()` 清空——**永不跨会话持久**，
/// 跨会话的持久授权属于 agent 配置域（配置提案系统），审批通道不得升级。
#[derive(Debug, Clone, Default)]
pub struct PathAuthSession {
    inner: Arc<Mutex<AuthGrants>>,
}

/// 三档授权集合（均存归一化形式）
#[derive(Debug, Clone, Default)]
struct AuthGrants {
    /// 精确路径（auth_cache_key 归一）
    paths: HashSet<String>,
    /// 目录（含子目录，path_within 组件级判定；auth_cache_key 归一）
    dirs: Vec<String>,
    /// 工具名（原样，工具名区分大小写）
    tools: HashSet<String>,
}

impl PathAuthSession {
    /// 新建空会话
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(AuthGrants::default())),
        }
    }

    /// 本次工具调用是否已被会话授权覆盖（判定序 tool > dir > path）
    pub async fn is_authorized(&self, path: &str, tool_name: &str) -> bool {
        let grants = self.inner.lock().await;
        if grants.tools.contains(tool_name) {
            return true;
        }
        // 目录档：无路径（Confirm 级工具）时只有工具档能覆盖
        if !path.is_empty() {
            let target = Path::new(path);
            if grants
                .dirs
                .iter()
                .any(|dir| path_norm::path_within(target, Path::new(dir)))
            {
                return true;
            }
            if grants.paths.contains(&path_norm::auth_cache_key(path)) {
                return true;
            }
        }
        false
    }

    /// 精确路径入账（「允许本次」档——同一路径再次访问不再弹）
    pub async fn mark_authorized(&self, path: &str) {
        let key = path_norm::auth_cache_key(path);
        let mut grants = self.inner.lock().await;
        grants.paths.insert(key);
    }

    /// 目录档入账（「允许此目录」——含子目录，会话内免问）
    pub async fn mark_dir_authorized(&self, dir: &str) {
        let key = path_norm::auth_cache_key(dir);
        let mut grants = self.inner.lock().await;
        if !grants.dirs.contains(&key) {
            grants.dirs.push(key);
        }
    }

    /// 工具档是否已覆盖（Confirm 级工具的扩围判定——**只看工具档**：
    /// 用户授权的目录/路径属文件域 grant，拿来静默放行 Confirm 级工具
    /// 属范围升级，用户没批过这个）
    pub async fn is_tool_authorized(&self, tool_name: &str) -> bool {
        self.inner.lock().await.tools.contains(tool_name)
    }

    /// 工具档入账（「允许此工具」——会话内该工具所有调用免问；
    /// Confirm 级工具唯一可用的扩围档）
    pub async fn mark_tool_authorized(&self, tool: &str) {
        let mut grants = self.inner.lock().await;
        grants.tools.insert(tool.to_string());
    }

    /// 清空会话授权（流式结束 / 取消时调用）
    pub async fn clear(&self) {
        let mut grants = self.inner.lock().await;
        *grants = AuthGrants::default();
    }

    /// 已授权条目总数（仅供测试 / 调试）
    #[cfg(test)]
    pub async fn len(&self) -> usize {
        let grants = self.inner.lock().await;
        grants.paths.len() + grants.dirs.len() + grants.tools.len()
    }

    /// 是否为空
    #[cfg(test)]
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

// =========================================================================
// check_authorization_with_session — 升级版授权判断
// =========================================================================

/// 检查工具授权，结合会话级分层授权记忆（#11）。
///
/// 决策逻辑：
/// - `Always` → `Allow`
/// - `Confirm` → 会话工具档已覆盖 → `Allow`；否则 `Confirm`
///   （「允许此工具」是 Confirm 级工具唯一可用的扩围档——路径/目录档
///   对其不生效，避免「授权过目录就静默放行高危工具」的范围升级）
/// - `PathWhitelist`:
///   - 路径在白名单 → `Allow`
///   - 会话三档任一覆盖（判定序 tool > dir > path）→ `Allow`
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
        AuthorizationLevel::Confirm => {
            if session.is_tool_authorized(tool_name).await {
                AuthorizationDecision::Allow
            } else {
                AuthorizationDecision::Confirm {
                    request_id: Uuid::new_v4().to_string(),
                    tool_name: tool_name.to_string(),
                    file_path: path.to_string(),
                    arguments: tool_args.to_string(),
                    reason: "此工具需要用户确认授权".to_string(),
                }
            }
        }
        AuthorizationLevel::PathWhitelist => {
            if is_path_allowed(path, config) || session.is_authorized(path, tool_name).await {
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
        // 语义收紧（组件级成员判定）：.bak 是另一个文件，不再被字符串
        // 前缀连带放行——白名单项精确到文件就只覆盖该文件
        assert!(!is_path_allowed("/tmp/test.txt.bak", &cfg));
        assert!(!is_path_allowed("/tmp/test2.txt", &cfg));
    }

    #[test]
    fn whitelist_dotdot_traversal_rejected() {
        // 安全：`..` 词法消解后再判归属，字符串前缀骗不过白名单
        let cfg = whitelist(&["/workspace/"]);
        assert!(!is_path_allowed("/workspace/../../etc/passwd", &cfg));
        assert!(is_path_allowed("/workspace/sub/../file.txt", &cfg));
    }

    #[test]
    fn whitelist_curdir_and_trailing_slash() {
        let cfg = whitelist(&["/workspace/"]);
        assert!(is_path_allowed("/workspace/./sub/file.txt", &cfg));
        assert!(is_path_allowed("/workspace", &cfg)); // 根本身也在内
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
        assert!(
            check_authorization(AuthorizationLevel::Always, "/etc/passwd", &cfg, "test").is_ok()
        );
    }

    #[test]
    fn check_authorization_whitelist_allowed() {
        let cfg = whitelist(&["/home/"]);
        assert!(check_authorization(
            AuthorizationLevel::PathWhitelist,
            "/home/doc.txt",
            &cfg,
            "read_file"
        )
        .is_ok());
    }

    #[test]
    fn check_authorization_whitelist_denied() {
        let cfg = whitelist(&["/home/"]);
        let result = check_authorization(
            AuthorizationLevel::PathWhitelist,
            "/etc/passwd",
            &cfg,
            "read_file",
        );
        assert!(result.is_err());
    }

    #[test]
    fn check_authorization_confirm_rejected() {
        let cfg = whitelist(&["/home/"]);
        let result = check_authorization(
            AuthorizationLevel::Confirm,
            "/home/doc.txt",
            &cfg,
            "some_tool",
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
        session.mark_dir_authorized("/d").await;
        session.mark_tool_authorized("t").await;
        assert_eq!(session.len().await, 3);
        session.clear().await;
        assert_eq!(session.len().await, 0);
        assert!(!session.is_authorized("/a", "read_file").await);
        assert!(!session.is_authorized("/d/x", "read_file").await);
        assert!(!session.is_authorized("/any", "t").await);
    }

    #[tokio::test]
    async fn session_authorization_normalized() {
        // 归一缓存键：同一路径不同写法（./ 段、尾斜杠）不再重复弹窗
        let session = PathAuthSession::new();
        session.mark_authorized("/ws/./sub/../file.txt").await;
        assert!(session.is_authorized("/ws/file.txt", "read_file").await);
        assert!(session.is_authorized("/ws/file.txt/", "read_file").await);
        assert!(!session.is_authorized("/ws/other.txt", "read_file").await);
    }

    #[tokio::test]
    async fn session_clone_shares_state() {
        let session1 = PathAuthSession::new();
        let session2 = session1.clone();
        session1.mark_authorized("/shared").await;
        assert!(session2.is_authorized("/shared", "read_file").await);
    }

    // ----- #11 分层授权记忆：三档 grant + 判定序 -----

    #[tokio::test]
    async fn dir_grant_covers_subpaths_not_siblings() {
        let session = PathAuthSession::new();
        session.mark_dir_authorized("/ws/project").await;
        assert!(session.is_authorized("/ws/project", "write_file").await);
        assert!(
            session
                .is_authorized("/ws/project/sub/deep/a.txt", "write_file")
                .await
        );
        // 归一化成员判定：./ 与尾斜杠写法同样覆盖
        assert!(
            session
                .is_authorized("/ws/project/./x/../b.txt", "write_file")
                .await
        );
        // 兄弟目录与上级不覆盖
        assert!(
            !session
                .is_authorized("/ws/project-other/a.txt", "write_file")
                .await
        );
        assert!(!session.is_authorized("/ws/a.txt", "write_file").await);
        // 目录档不覆盖其它工具的无路径调用
        assert!(!session.is_authorized("", "run_command").await);
    }

    #[tokio::test]
    async fn tool_grant_covers_any_path_this_tool_only() {
        let session = PathAuthSession::new();
        session.mark_tool_authorized("read_file").await;
        assert!(session.is_authorized("/etc/passwd", "read_file").await);
        assert!(session.is_authorized("", "read_file").await);
        // 其它工具不沾光
        assert!(!session.is_authorized("/etc/passwd", "write_file").await);
    }

    #[tokio::test]
    async fn confirm_level_tool_grant_skips_prompt() {
        // Confirm 级工具：会话工具档已覆盖 → 直接 Allow
        let cfg = whitelist(&["/workspace/"]);
        let session = PathAuthSession::new();
        session.mark_tool_authorized("run_command").await;
        let d = check_authorization_with_session(
            AuthorizationLevel::Confirm,
            "",
            &cfg,
            "run_command",
            r#"{"cmd":"ls"}"#,
            &session,
        )
        .await;
        assert!(d.is_allowed());
    }

    #[tokio::test]
    async fn confirm_level_ignores_path_and_dir_grants() {
        // 范围升级防线：文件域 grant（path/dir）不得静默放行 Confirm 级工具
        let cfg = whitelist(&["/workspace/"]);
        let session = PathAuthSession::new();
        session.mark_authorized("/workspace/run.sh").await;
        session.mark_dir_authorized("/workspace").await;
        let d = check_authorization_with_session(
            AuthorizationLevel::Confirm,
            "/workspace/run.sh",
            &cfg,
            "run_command",
            r#"{"path":"/workspace/run.sh"}"#,
            &session,
        )
        .await;
        assert!(d.needs_confirm());
    }

    #[tokio::test]
    async fn pathwhitelist_tool_grant_allows_outside_whitelist() {
        // 工具档优先级最高：白名单外路径也免问（用户显式批过此工具）
        let cfg = whitelist(&["/workspace/"]);
        let session = PathAuthSession::new();
        session.mark_tool_authorized("read_file").await;
        let d = check_authorization_with_session(
            AuthorizationLevel::PathWhitelist,
            "/etc/passwd",
            &cfg,
            "read_file",
            r#"{"path":"/etc/passwd"}"#,
            &session,
        )
        .await;
        assert!(d.is_allowed());
    }

    #[tokio::test]
    async fn pathwhitelist_dir_grant_allows_inside() {
        // 目录档：会话内授权目录下（含子目录）免问——连续审批的核心减负
        let cfg = whitelist(&["/workspace/"]);
        let session = PathAuthSession::new();
        session.mark_dir_authorized("/ws/proj").await;
        let d = check_authorization_with_session(
            AuthorizationLevel::PathWhitelist,
            "/ws/proj/src/main.rs",
            &cfg,
            "write_file",
            r#"{"path":"/ws/proj/src/main.rs"}"#,
            &session,
        )
        .await;
        assert!(d.is_allowed());
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
