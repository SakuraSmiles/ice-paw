//! `run_command` 工具：执行 shell 命令（`Confirm` 授权——每次调用弹窗让用户确认）
//!
//! 在 agent workspace 内执行（`current_dir = ctx.workspace`）。走 `sh -c` / `cmd /C`，
//! 以支持 PATH 解析、管道、`&&`、Windows 的 `.cmd/.bat` 等。
//! 继承环境变量（命令需要 PATH 等；Confirm 级用户已逐条审批该命令）。
//! 非零退出码不算错误——把 stdout/stderr 原样返回给 LLM，让它据 exit_code 判断。

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{AppError, AppResult};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

/// 输出截断上限（避免超长输出撑爆 LLM 上下文）
const MAX_OUTPUT: usize = 20_000;

pub struct RunCommandTool;

#[derive(Deserialize)]
struct RunCommandArgs {
    /// 完整命令行（如 `npm test`、`git status`、`cargo build`）
    command: String,
    #[serde(default = "default_timeout")]
    timeout_secs: u64,
}

fn default_timeout() -> u64 {
    120
}

#[async_trait]
impl McpClient for RunCommandTool {
    fn name(&self) -> &str {
        "run_command"
    }

    fn description(&self) -> &str {
        "Execute a shell command line in the agent workspace and return combined \
stdout+stderr plus exit code. Use for builds, tests, git, etc. Non-zero exit is not an error—\
read exit_code and output to decide next steps."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Full command line to run (e.g. \"npm test\", \"git diff\")."
                },
                "timeout_secs": {
                    "type": "integer",
                    "default": 120,
                    "description": "Max seconds before killing the command."
                }
            },
            "required": ["command"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Confirm
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "run_command 必须通过 execute_with_context 调用（需要 workspace）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: RunCommandArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("run_command 参数解析失败: {e}")))?;

        // 走系统 shell：Unix 用 sh -c，Windows 用 cmd /C（支持 PATH/.cmd/管道）
        let mut cmd = if cfg!(windows) {
            let mut c = tokio::process::Command::new("cmd");
            c.args(["/C", &parsed.command]);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", &parsed.command]);
            c
        };
        if let Some(ws) = &ctx.workspace {
            cmd.current_dir(ws);
        }
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null());

        let output = tokio::time::timeout(
            Duration::from_secs(parsed.timeout_secs),
            cmd.output(),
        )
        .await
        .map_err(|_| {
            AppError::Internal(format!(
                "命令超时（{}s）: {}",
                parsed.timeout_secs, parsed.command
            ))
        })?
        .map_err(AppError::Io)?;

        // 统一解码 stdout/stderr（UTF-8 → GBK → lossy）
        let stdout = crate::infra::decode::decode_bytes(&output.stdout);
        let stderr = crate::infra::decode::decode_bytes(&output.stderr);
        let mut combined = String::new();
        if !stdout.text.is_empty() {
            combined.push_str(&stdout.text);
        }
        if !stderr.text.is_empty() {
            if !combined.is_empty() {
                combined.push_str("\n[stderr]\n");
            }
            combined.push_str(&stderr.text);
        }

        let truncated = combined.len() > MAX_OUTPUT;
        if truncated {
            combined.truncate(MAX_OUTPUT);
            combined.push_str("\n...[输出已截断]");
        }

        Ok(serde_json::json!({
            "command": parsed.command,
            "exit_code": output.status.code(),
            "output": combined,
            "encoding": if stdout.encoding == stderr.encoding { stdout.encoding } else { "mixed" },
            "truncated": truncated,
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
        }
    }

    #[tokio::test]
    async fn parse_valid_args() {
        let tool = RunCommandTool;
        let ctx = test_ctx().await;
        let result = tool
            .execute_with_context(
                r#"{"command":"echo hello","timeout_secs":5}"#,
                &ctx,
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["command"], "echo hello");
        assert_eq!(v["exit_code"], 0);
        assert!(v["output"].as_str().unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn non_zero_exit_code() {
        let tool = RunCommandTool;
        let ctx = test_ctx().await;
        let result = tool
            .execute_with_context(
                // 跨平台：用 exit 1 确保非零退出码
                if cfg!(windows) {
                    r#"{"command":"cmd /c exit 1","timeout_secs":5}"#
                } else {
                    r#"{"command":"sh -c 'exit 1'","timeout_secs":5}"#
                },
                &ctx,
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["exit_code"], 1);
    }

    #[tokio::test]
    async fn invalid_json_args_returns_validation_error() {
        let tool = RunCommandTool;
        let ctx = test_ctx().await;
        let result = tool.execute_with_context("not json", &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("参数解析失败") || err.contains("Validation"));
    }

    #[tokio::test]
    async fn missing_command_field_returns_error() {
        let tool = RunCommandTool;
        let ctx = test_ctx().await;
        let result = tool
            .execute_with_context(r#"{"timeout_secs":5}"#, &ctx)
            .await;
        assert!(result.is_err());
    }
}
