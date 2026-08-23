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
        // 平台差异写进描述（Codex A2）：Windows 侧告知代码页已统一 UTF-8，
        // 模型不必再猜测中文输出的编码形态
        if cfg!(windows) {
            "Execute a shell command line in the agent workspace and return combined \
stdout+stderr plus exit code. Runs via `cmd /C` with UTF-8 codepage (chcp 65001), so \
Chinese output decodes correctly. Use for builds, tests, git, etc. Non-zero exit is not \
an error—read exit_code and output to decide next steps."
        } else {
            "Execute a shell command line in the agent workspace and return combined \
stdout+stderr plus exit code. Use for builds, tests, git, etc. Non-zero exit is not an error—\
read exit_code and output to decide next steps."
        }
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

        // 走系统 shell：Unix 用 sh -c，Windows 用 cmd /C（支持 PATH/.cmd/管道）。
        // Windows 前置 `chcp 65001 >nul & `：中文系统默认 GBK 代码页，git/node 等子进程
        // 按控制台代码页编码输出 → 解码端混淆/乱码（诊断 2026-08-22：真乱码样本全部
        // 集中在 PowerShell/cmd 中文输出）。切到 UTF-8 代码页从源头统一，>nul 吞掉
        // 切换回显，& 串联原命令。
        let mut cmd = if cfg!(windows) {
            let mut c = tokio::process::Command::new("cmd");
            let full = format!("chcp 65001 >nul & {}", parsed.command);
            c.args(["/C", &full]);
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
        // Windows: 隐藏 cmd /C 弹出的控制台窗口（GUI 应用 spawn 子进程会闪窗）
        crate::infra::process::suppress_console_window(&mut cmd);

        let output = tokio::time::timeout(Duration::from_secs(parsed.timeout_secs), cmd.output())
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
            // String::truncate 按字节截断，落在新中文等多字节字符中间会 panic；
            // 走统一的安全截断（回退到 char 边界）。
            combined = crate::infra::strings::truncate_to_byte_boundary(
                &combined,
                MAX_OUTPUT,
                Some("\n...[输出已截断]"),
            );
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
            turn_id: None,
            cancel: None,
        }
    }

    #[tokio::test]
    async fn parse_valid_args() {
        let tool = RunCommandTool;
        let ctx = test_ctx().await;
        let result = tool
            .execute_with_context(r#"{"command":"echo hello","timeout_secs":5}"#, &ctx)
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

    /// Agent 质量拍（2026-08-23）：Windows 前置 chcp 65001 生效验证——
    /// 通过工具跑 `chcp` 查询活动代码页，应报 65001（包装是否真挂上以运行时为准）
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_codepage_is_utf8() {
        let tool = RunCommandTool;
        let ctx = test_ctx().await;
        let result = tool
            .execute_with_context(r#"{"command":"chcp","timeout_secs":10}"#, &ctx)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["exit_code"], 0);
        assert!(
            v["output"].as_str().unwrap().contains("65001"),
            "活动代码页应为 65001: {result}"
        );
    }
}
