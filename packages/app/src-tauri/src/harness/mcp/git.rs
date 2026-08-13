//! `git` 工具：只读 git 操作（status / diff / log / show），跑 `git` CLI
//!
//! `Always` 授权（全是只读）。在 agent workspace 内执行。写操作（commit / add / push）
//! 不在此暴露——由 LLM 走 `run_command`（Confirm）。

use std::process::Stdio;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{AppError, AppResult};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

const MAX_OUTPUT: usize = 20_000;
const ALLOWED_OPS: &[&str] = &["status", "diff", "log", "show"];

pub struct GitTool;

#[derive(Deserialize)]
struct GitArgs {
    operation: String,
    /// 可选附加参数（如 log 的 "-n 20"、show 的 commit hash），按空白拆分传给 git
    #[serde(default)]
    args: Option<String>,
}

#[async_trait]
impl McpClient for GitTool {
    fn name(&self) -> &str {
        "git"
    }

    fn description(&self) -> &str {
        "Run read-only git operations (status/diff/log/show) in the agent workspace. \
For write operations (commit/add/push), use run_command instead."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "enum": ["status", "diff", "log", "show"] },
                "args": { "type": "string", "description": "Optional extra args, e.g. '-n 20' for log or a commit hash for show." }
            },
            "required": ["operation"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "git 必须通过 execute_with_context 调用（需要 workspace）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: GitArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("git 参数解析失败: {e}")))?;

        if !ALLOWED_OPS.contains(&parsed.operation.as_str()) {
            return Err(AppError::Validation(format!(
                "git 只支持只读操作 {:?}（写操作请用 run_command）",
                ALLOWED_OPS
            )));
        }

        let mut cmd = tokio::process::Command::new("git");
        // core.quotepath=false：禁用 git 对非 ASCII 路径的八进制转义
        //（默认会把中文文件名输出成 "\344\270\255"；关掉后直接吐原始 UTF-8）
        cmd.arg("-c").arg("core.quotepath=false");
        cmd.arg(&parsed.operation);
        if let Some(extra) = &parsed.args {
            for a in extra.split_whitespace() {
                cmd.arg(a);
            }
        }
        if let Some(ws) = &ctx.workspace {
            cmd.current_dir(ws);
        }
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        // Windows: 隐藏 git 子进程弹出的控制台窗口（GUI 应用 spawn 子进程会闪窗）
        crate::infra::process::suppress_console_window(&mut cmd);

        let output = cmd
            .output()
            .await
            .map_err(|e| AppError::Internal(format!("执行 git 失败（git 是否安装？）: {e}")))?;

        let stdout = crate::infra::decode::decode_bytes(&output.stdout);
        let stderr = crate::infra::decode::decode_bytes(&output.stderr);
        let mut out = stdout.text;
        if !stderr.text.is_empty() {
            out.push_str("\n[stderr]\n");
            out.push_str(&stderr.text);
        }
        if out.len() > MAX_OUTPUT {
            // String::truncate 按字节截断，落在新中文等多字节字符中间会 panic；
            // 走统一的安全截断（回退到 char 边界）。
            out = crate::infra::strings::truncate_to_byte_boundary(
                &out,
                MAX_OUTPUT,
                Some("\n...[输出已截断]"),
            );
        }

        Ok(serde_json::json!({
            "operation": parsed.operation,
            "exit_code": output.status.code(),
            "output": out,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn test_ctx() -> ToolContext {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        ToolContext {
            conv_id: "test".into(),
            agent_id: "test-agent".into(),
            project_id: None,
            workspace: None,
            pool,
            api_key: None,
            app_handle: None,
            proposal_registry: None,
            cancel: None,
        }
    }

    #[tokio::test]
    async fn reject_write_operation() {
        let tool = GitTool;
        let ctx = test_ctx().await;
        let result = tool
            .execute_with_context(r#"{"operation":"commit"}"#, &ctx)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("只读"));
    }

    #[tokio::test]
    async fn reject_unknown_operation() {
        let tool = GitTool;
        let ctx = test_ctx().await;
        let result = tool
            .execute_with_context(r#"{"operation":"push"}"#, &ctx)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn invalid_json_args_returns_validation_error() {
        let tool = GitTool;
        let ctx = test_ctx().await;
        let result = tool.execute_with_context("not json", &ctx).await;
        assert!(result.is_err());
    }
}
