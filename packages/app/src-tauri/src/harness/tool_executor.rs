//! L2 Tool Executor — 工具执行编排（W3.3 + A2-3 用户授权流程）
//!
//! 职责：对一批已完成的工具调用执行：
//! 1. 解析工具参数、提取待访问路径
//! 2. 通过 `check_authorization_with_session` 判断授权
//! 3. 直接执行 / emit `chat:tool-auth-request` 等待前端弹窗 / 拒绝
//! 4. 收集 tool_use + tool_result 的 ContentBlock，emit `chat:tool-result`
//!
//! **A2-3 变更**：
//! - `execute_tool_round` 现在接收 `PathAuthSession`（会话级已授权路径表）
//!   和 `whitelist_config`（白名单配置）。
//! - 当工具授权等级为 `PathWhitelist` 或 `Confirm` 且不在白名单 / 会话已授权
//!   集合内时，emit `chat:tool-auth-request` 事件，前端弹窗后通过
//!   `chat:tool-auth-response` 事件回传结果，Rust 侧用 oneshot 通道匹配
//!   request_id 并阻塞等待。
//! - 用户拒绝时，工具结果被写为「拒绝授权」错误（is_error=true），
//!   LLM 收到后可在下一轮调整策略。
//!
//! 设计取舍：
//! - oneshot 通道用 `Arc<Mutex<HashMap<String, oneshot::Sender<ToolAuthResponse>>>>`
//!   全局注册表（在 `ToolAuthRegistry` 中），这样 `chat:tool-auth-response`
//!   事件监听器（独立 task）能按 `request_id` 找到对应 sender 并 send。
//! - 监听器在 `ToolAuthRegistry::install_listener()` 中用 `tauri::Builder::on_page_load`
//!   或在 `lib.rs::run()` 注册。这里选择后者（独立模块 `auth_responder`），
//!   确保只有一份全局监听。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, Mutex};

use crate::db::models::{HookConfig, HookPoint};
use crate::db::repo;
use crate::infra::protocol::{
    ChatToolResultPayload, ContentBlock, ToolAuthRequestPayload, ToolAuthResponse,
};
use crate::harness::mcp::{AuthorizationLevel, McpRegistry, ToolContext};
use crate::harness::authority::{
    check_authorization_with_session, AuthorizationDecision, PathAuthSession, PathWhitelistConfig,
};
use crate::harness::hooks::{has_actions, run_hooks};

/// oneshot sender 的全局注册表类型
type AuthSenderMap = Arc<Mutex<HashMap<String, oneshot::Sender<ToolAuthResponse>>>>;

/// A2-3: 工具授权响应的全局注册表
///
/// 维护 `request_id → oneshot::Sender`，供前端响应事件按 request_id 解锁
/// 对应等待者。同时也持有应用侧的默认白名单配置（如果上层不显式传入）。
///
/// 生命周期：在 `lib.rs::run()` setup 阶段调用 `install_listener()`
/// 注册 Tauri 事件监听。
#[derive(Clone, Default)]
pub struct ToolAuthRegistry {
    inner: AuthSenderMap,
}

impl ToolAuthRegistry {
    /// 新建空注册表
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 注册一个新的等待者，返回 receiver
    pub async fn register(
        &self,
        request_id: String,
    ) -> oneshot::Receiver<ToolAuthResponse> {
        let (tx, rx) = oneshot::channel();
        let mut map = self.inner.lock().await;
        map.insert(request_id, tx);
        rx
    }

    /// 取出并删除一个等待者（用于取消时清理）
    pub async fn take(&self, request_id: &str) -> Option<oneshot::Sender<ToolAuthResponse>> {
        let mut map = self.inner.lock().await;
        map.remove(request_id)
    }

    /// 用响应唤醒一个等待者
    pub async fn respond(&self, response: ToolAuthResponse) -> bool {
        let mut map = self.inner.lock().await;
        if let Some(tx) = map.remove(&response.request_id) {
            // send 失败说明 receiver 已被 drop（例如上层取消）→ 忽略
            let _ = tx.send(response);
            true
        } else {
            false
        }
    }

    /// 当前等待者数量（仅供调试）
    #[cfg(test)]
    pub async fn pending_count(&self) -> usize {
        let map = self.inner.lock().await;
        map.len()
    }

    /// A2-3: 安装前端 `chat:tool-auth-response` 事件监听器
    ///
    /// 在 `lib.rs::run()` setup 阶段调用一次，监听前端响应事件，
    /// 通过 `self.respond()` 唤醒对应 request_id 的 oneshot。
    ///
    /// 注意：克隆 `self`（注册表内部是 `Arc`，克隆即共享）。
    pub fn install_listener(&self, app: &AppHandle) {
        let registry = self.clone();
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            use tauri::Listener;
            let _ = app_handle.listen("chat:tool-auth-response", move |event| {
                let registry = registry.clone();
                // payload 必须在 spawn 前 clone 为 String，因为 event 是 &Event
                let payload_str = event.payload().to_string();
                let response: Result<ToolAuthResponse, _> = serde_json::from_str(&payload_str);
                tauri::async_runtime::spawn(async move {
                    match response {
                        Ok(r) => {
                            let handled = registry.respond(r).await;
                            if !handled {
                                tracing::warn!(
                                    target: "ice_paw.tool_auth",
                                    "收到未知 request_id 的授权响应（可能已超时）",
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                target: "ice_paw.tool_auth",
                                "授权响应解析失败: {} (payload={})",
                                e,
                                payload_str,
                            );
                        }
                    }
                });
            });
        });
    }
}

/// 工具执行编排（A2-3 升级版）
///
/// 与旧版的差异：
/// - 新增参数 `auth_registry: &ToolAuthRegistry`、`session: &PathAuthSession`、
///   `whitelist: &PathWhitelistConfig`、`cancel: &CancellationToken`、
///   `tool_ctx` / `asst_msg_id`（tool_ctx 提供 conv_id/agent_id/project_id/pool，
///   供 emit auth request 携带对话上下文 + 透传给 `dispatch` → `execute_with_context`）。
/// - 工具执行前先做授权判断；如果需要确认则 emit + 阻塞等待前端响应。
///
/// 返回 `tool_result_blocks`（每个已完成工具调用对应一个 ToolResult，emit `chat:tool-result`）。
/// tool_use blocks 由调用方（loop_engine）从 completed_calls 自行组装，消除重复来源。
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool_round(
    app: &AppHandle,
    registry: &McpRegistry,
    auth_registry: &ToolAuthRegistry,
    session: &PathAuthSession,
    whitelist: &PathWhitelistConfig,
    completed_calls: &[(String, String, String)],
    tool_ctx: &ToolContext,
    asst_msg_id: &str,
    cancel: &crate::harness::chat_state::CancellationToken,
    hooks: &HookConfig,
) -> crate::error::AppResult<Vec<ContentBlock>> {
    let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

    // agent workspace 内的文件免授权（workspace 是 agent 的信任领地，
    // agent 读写自己 workspace 内的文件不需弹窗确认）。workspace 由 loop_engine
    // 解析后放进 tool_ctx，这里直接复用，避免重复查库。
    let workspace = tool_ctx.workspace.as_ref().map(PathBuf::from);

    for (tc_id, tc_name, tc_args) in completed_calls {
        let tool_start = std::time::Instant::now();
        // 1. 解析授权级别 + 路径
        let (level, file_path) = inspect_tool_for_auth(registry, tc_name, tc_args).await;

        let decision = if matches!(level, AuthorizationLevel::PathWhitelist)
            && path_within_workspace(&file_path, &workspace)
        {
            // workspace 内 + PathWhitelist 级别（如 read_file）→ 免授权放行
            AuthorizationDecision::Allow
        } else {
            check_authorization_with_session(
                level,
                &file_path,
                whitelist,
                tc_name,
                tc_args,
                session,
            )
            .await
        };

        // 2. 根据决策执行
        let final_result: Result<String, String> = match decision {
            AuthorizationDecision::Allow => match registry.dispatch(tc_name, tc_args, tool_ctx).await {
                Ok(s) => Ok(s),
                Err(e) => Err(e.to_string()),
            },
            AuthorizationDecision::Confirm {
                request_id,
                tool_name,
                file_path,
                arguments,
                reason,
            } => {
                // 2a. 注册 oneshot receiver
                let rx = auth_registry.register(request_id.clone()).await;
                // 2b. emit 事件给前端
                let payload = ToolAuthRequestPayload {
                    request_id: request_id.clone(),
                    tool_use_id: tc_id.clone(),
                    tool_name: tool_name.clone(),
                    file_path: file_path.clone(),
                    arguments: arguments.clone(),
                    conversation_id: tool_ctx.conv_id.clone(),
                    message_id: asst_msg_id.to_string(),
                    reason: reason.clone(),
                };
                if let Err(e) = app.emit("chat:tool-auth-request", payload) {
                    // emit 失败：清理 receiver，视为拒绝
                    let _ = auth_registry.take(&request_id).await;
                    Err(format!("无法通知前端授权弹窗：{}", e))
                } else {
                    tracing::info!(
                        target: "ice_paw.tool_auth",
                        "已发送授权请求: tool={} path={} request_id={}",
                        tool_name,
                        file_path,
                        request_id,
                    );
                    // 2c. 等待响应（带取消支持 + 30 分钟超时防止永久挂起）
                    let outcome = wait_for_auth_response(rx, cancel, &request_id, auth_registry)
                        .await;
                    match outcome {
                        Some(true) => {
                            // 用户允许：标记会话级授权，然后执行
                            session.mark_authorized(&file_path).await;
                            tracing::info!(
                                target: "ice_paw.tool_auth",
                                "用户允许工具调用: tool={} path={}",
                                tool_name,
                                file_path,
                            );
                            match registry.dispatch(tc_name, tc_args, tool_ctx).await {
                                Ok(s) => Ok(s),
                                Err(e) => Err(e.to_string()),
                            }
                        }
                        Some(false) => {
                            // 用户拒绝：写工具结果为拒绝错误
                            tracing::info!(
                                target: "ice_paw.tool_auth",
                                "用户拒绝工具调用: tool={} path={}",
                                tool_name,
                                file_path,
                            );
                            Err(format!(
                                "用户拒绝了工具 '{}' 的调用（路径：{}）",
                                tool_name, file_path
                            ))
                        }
                        None => {
                            // 取消 / 超时：写工具结果为取消错误
                            tracing::warn!(
                                target: "ice_paw.tool_auth",
                                "授权请求被取消或超时: request_id={}",
                                request_id,
                            );
                            Err("授权请求被取消或超时".to_string())
                        }
                    }
                }
            }
            AuthorizationDecision::Deny { reason } => {
                tracing::warn!(
                    target: "ice_paw.tool_auth",
                    "工具被永久拒绝: tool={} reason={}",
                    tc_name,
                    reason,
                );
                Err(reason)
            }
        };

        // 3. emit tool-result + 收集 blocks
        let duration_ms = tool_start.elapsed().as_millis() as u64;
        match final_result {
            Ok(content) => {
                let _ = app.emit(
                    "chat:tool-result",
                    ChatToolResultPayload {
                        conversation_id: tool_ctx.conv_id.clone(),
                        message_id: asst_msg_id.to_string(),
                        tool_use_id: tc_id.clone(),
                        content: content.clone(),
                        is_error: false,
                        duration_ms,
                    },
                );
                tool_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: tc_id.clone(),
                    content,
                    is_error: Some(false),
                });
            }
            Err(err_content) => {
                let _ = app.emit(
                    "chat:tool-result",
                    ChatToolResultPayload {
                        conversation_id: tool_ctx.conv_id.clone(),
                        message_id: asst_msg_id.to_string(),
                        tool_use_id: tc_id.clone(),
                        content: err_content.clone(),
                        is_error: true,
                        duration_ms,
                    },
                );
                tool_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: tc_id.clone(),
                    content: err_content,
                    is_error: Some(true),
                });
            }
        }

        // === Hook: AfterTool（每次工具执行后；Log/CallTool，失败仅 warn 不中断）===
        if has_actions(hooks, HookPoint::AfterTool) {
            if let Err(e) = run_hooks(HookPoint::AfterTool, hooks, tool_ctx, registry).await {
                tracing::warn!(
                    target: "ice_paw.hooks",
                    "AfterTool 钩子执行失败（忽略）: tool={} err={}",
                    tc_name,
                    e
                );
            }
        }
    }

    Ok(tool_result_blocks)
}

/// 等待前端授权响应。
///
/// 返回值：
/// - `Some(true)`  用户允许
/// - `Some(false)` 用户拒绝
/// - `None`        被取消 / 超时
///
/// `cancel` 触发取消时，oneshot receiver 被丢弃，sender 端 send 失败被忽略。
async fn wait_for_auth_response(
    rx: oneshot::Receiver<ToolAuthResponse>,
    cancel: &crate::harness::chat_state::CancellationToken,
    request_id: &str,
    auth_registry: &ToolAuthRegistry,
) -> Option<bool> {
    const TIMEOUT: Duration = Duration::from_secs(120); // 2 分钟——用户不在时快速超时释放会话

    tokio::select! {
        biased;
        // 用户主动取消
        _ = wait_for_cancel(cancel) => {
            // 清理 sender，避免后续响应泄露
            let _ = auth_registry.take(request_id).await;
            None
        }
        // 超时自动拒绝（防止前端崩溃/用户离开导致会话永久锁死）
        _ = tokio::time::sleep(TIMEOUT) => {
            tracing::warn!(
                target: "ice_paw.tool_auth",
                "授权请求超时（{} 秒）: request_id={}",
                TIMEOUT.as_secs(),
                request_id,
            );
            let _ = auth_registry.take(request_id).await;
            None
        }
        // 收到前端响应
        msg = rx => {
            match msg {
                Ok(resp) => Some(resp.allowed),
                Err(_) => None, // sender 被 drop → 视为取消
            }
        }
    }
}

/// 等待 CancellationToken 触发（包装成 future）
async fn wait_for_cancel(token: &crate::harness::chat_state::CancellationToken) {
    // 简单的轮询（100ms 粒度）。CancellationToken 当前未提供异步 wait，
    // 这里用低开销循环等价实现。后续如果改成 watch channel 可以直接 select。
    loop {
        if token.is_cancelled() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 从工具实例提取 `AuthorizationLevel` + 待访问路径。
///
/// - `level` 直接调 `tool.authorization_level()`
/// - `file_path` 从参数 JSON 中尝试 `path` / `file_path` / `dir` 字段
///   提取，找不到则用空字符串（`Always` 工具不需要路径）
async fn inspect_tool_for_auth(
    registry: &McpRegistry,
    tool_name: &str,
    args: &str,
) -> (AuthorizationLevel, String) {
    let default_level = AuthorizationLevel::Always;
    let default_path = String::new();

    let Some(tool) = registry.get(tool_name).await else {
        return (default_level, default_path);
    };

    let level = tool.authorization_level();
    let path = extract_path_from_args(args).unwrap_or(default_path);
    (level, path)
}

/// 解析 agent 的 workspace 根路径（canonicalize，用于 workspace 内免授权判断）。
/// agent 无 workspace 或路径不存在 → None（回退到正常授权流程）。
pub(crate) async fn resolve_agent_workspace(pool: &SqlitePool, agent_id: &str) -> Option<PathBuf> {
    let agent = repo::agent::get_by_id(pool, agent_id).await.ok()?;
    let ws = agent.workspace_path?;
    Path::new(&ws).canonicalize().ok()
}

/// 构造工具执行上下文（解析 workspace：project 优先，回退 agent workspace）。
///
/// 供钩子接入点（BeforeLlm / ConversationStart / ConversationEnd）与 loop_engine
/// 主流程复用，统一 workspace 解析逻辑。execute_tool_round 主流程的 tool_ctx
/// 原先在 loop_engine 内联构造，现已改为调用本函数。
pub(crate) async fn build_tool_ctx(
    pool: &SqlitePool,
    conv_id: String,
    agent_id: String,
    project_id: Option<String>,
    api_key: Option<String>,
) -> ToolContext {
    // workspace 解析：project 绑定了 workspace_path → 用项目源码根；
    // 否则回退 agent workspace。
    let project_ws: Option<String> = match &project_id {
        Some(pid) => repo::project::get_by_id(pool, pid)
            .await
            .ok()
            .and_then(|p| p.workspace_path)
            .and_then(|ws| Path::new(&ws).canonicalize().ok())
            .map(|p| p.to_string_lossy().to_string()),
        None => None,
    };
    let agent_ws = resolve_agent_workspace(pool, &agent_id)
        .await
        .map(|p| p.to_string_lossy().to_string());
    ToolContext {
        conv_id,
        agent_id,
        project_id,
        workspace: project_ws.or(agent_ws),
        pool: pool.clone(),
        api_key,
    }
}

/// 判断 file_path 是否在 agent workspace 内（规范化后 starts_with）。
/// 路径不存在时回退到直接 starts_with（read_file 读前判断，文件可能还没 canonicalize）。
fn path_within_workspace(file_path: &str, workspace: &Option<PathBuf>) -> bool {
    let Some(ws) = workspace else {
        return false;
    };
    match Path::new(file_path).canonicalize() {
        Ok(cfp) => cfp.starts_with(ws),
        Err(_) => Path::new(file_path).starts_with(ws),
    }
}

/// 从工具参数 JSON 提取路径字段（`path` / `file_path` / `dir`）
fn extract_path_from_args(args: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(args).ok()?;
    for key in ["path", "file_path", "dir"] {
        if let Some(s) = parsed.get(key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

// ==========================================================================
// 单元测试
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_path_from_args_with_path() {
        assert_eq!(
            extract_path_from_args(r#"{"path":"/etc/passwd"}"#),
            Some("/etc/passwd".into())
        );
    }

    #[test]
    fn extract_path_from_args_with_file_path() {
        assert_eq!(
            extract_path_from_args(r#"{"file_path":"/home/x.txt"}"#),
            Some("/home/x.txt".into())
        );
    }

    #[test]
    fn extract_path_from_args_with_dir() {
        assert_eq!(
            extract_path_from_args(r#"{"dir":"/var"}"#),
            Some("/var".into())
        );
    }

    #[test]
    fn extract_path_from_args_missing() {
        assert_eq!(extract_path_from_args(r#"{"other":"x"}"#), None);
    }

    #[test]
    fn extract_path_from_args_invalid_json() {
        assert_eq!(extract_path_from_args("not json"), None);
    }

    #[tokio::test]
    async fn registry_register_and_respond() {
        let reg = ToolAuthRegistry::new();
        let req_id = "req-1".to_string();
        let rx = reg.register(req_id.clone()).await;
        assert_eq!(reg.pending_count().await, 1);

        // 异步响应
        let reg2 = reg.clone();
        tokio::spawn(async move {
            reg2.respond(ToolAuthResponse {
                request_id: req_id.clone(),
                allowed: true,
            })
            .await;
        });

        let resp = rx.await.unwrap();
        assert!(resp.allowed);
        // 响应后 sender 已被取出
        assert_eq!(reg.pending_count().await, 0);
    }

    #[tokio::test]
    async fn registry_respond_unknown_id_returns_false() {
        let reg = ToolAuthRegistry::new();
        let handled = reg
            .respond(ToolAuthResponse {
                request_id: "nope".into(),
                allowed: false,
            })
            .await;
        assert!(!handled);
    }

    #[tokio::test]
    async fn registry_take_removes_sender() {
        let reg = ToolAuthRegistry::new();
        let req_id = "req-take".to_string();
        let _rx = reg.register(req_id.clone()).await;
        assert_eq!(reg.pending_count().await, 1);
        let taken = reg.take(&req_id).await;
        assert!(taken.is_some());
        assert_eq!(reg.pending_count().await, 0);
    }

    #[tokio::test]
    async fn registry_clone_shares_state() {
        let reg1 = ToolAuthRegistry::new();
        let reg2 = reg1.clone();
        let _rx = reg1.register("shared".into()).await;
        assert_eq!(reg2.pending_count().await, 1);
    }

    // ===== workspace 内免授权：path_within_workspace =====

    #[test]
    fn path_within_workspace_matches_inside() {
        let ws = Some(std::env::temp_dir());
        let inside = std::env::temp_dir().join("some_file.txt");
        assert!(path_within_workspace(inside.to_str().unwrap(), &ws));
    }

    #[test]
    fn path_within_workspace_rejects_outside() {
        let ws = Some(std::env::temp_dir());
        assert!(!path_within_workspace("C:/Windows/System32/drivers/etc/hosts", &ws));
    }

    #[test]
    fn path_within_workspace_none_workspace() {
        assert!(!path_within_workspace("/any/path", &None));
    }
}