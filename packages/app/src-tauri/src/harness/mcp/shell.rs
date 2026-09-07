//! `run_command` 工具：执行 shell 命令（`Confirm` 授权——每次调用弹窗让用户确认）
//!
//! 在 agent workspace 内执行（`current_dir = ctx.workspace`）。走 `sh -c` / `cmd /C`，
//! 以支持 PATH 解析、管道、`&&`、Windows 的 `.cmd/.bat` 等。
//! 继承环境变量（命令需要 PATH 等；Confirm 级用户已逐条审批该命令）。
//! 非零退出码不算错误——把 stdout/stderr 原样返回给 LLM，让它据 exit_code 判断。
//!
//! Windows 引号直通道（2026-09-07 生产根治批）：命令经 `raw_arg` 原样传给
//! `cmd /C`，语义 = 在 cmd 窗口手敲完全一致。此前走 `.args()` 的
//! CommandLineToArgvW 转义层（整体包引号 + 内层 `"` 转义成 `\"`），cmd 剥外层
//! 引号后 PowerShell 收到字面引号 → `-Command` 得到字符串字面量 → **命令被
//! 原样回显且 exit_code=0 的静默失败**（生产 1060 条记录 74 起实锤，agent 被迫
//! 84% 改写 .ps1 规避）。回显防御层见 [`looks_like_echo`]。

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

/// 回显检测（保守，宁漏勿误）：PowerShell 家族 + 输出主体是命令原文子串 + 够长。
///
/// 命中形态 = 直通道修复前的历史坑 / 模型自己写错引号：`-Command` 收到被引号
/// 包裹的**字符串字面量** → PowerShell 求值并输出它、exit_code=0 全静默。回显
/// 里 `$_` 已被字符串插值吞掉（`{ $_.Name }` 回显成 `{ .Name }`）——所以子串
/// 判据对命令本体与去 `$_` 形态都匹配。echo/type 天然不误报：输出通常 <15 字符
/// 且命令不含 powershell。
fn looks_like_echo(command: &str, output: &str) -> bool {
    let out = output.trim();
    if out.chars().count() < 15 {
        return false;
    }
    let lower = command.to_lowercase();
    if !(lower.contains("powershell") || lower.contains("pwsh")) {
        return false;
    }
    command.contains(out) || command.replace("$_", "").contains(out)
}

#[async_trait]
impl McpClient for RunCommandTool {
    fn name(&self) -> &str {
        "run_command"
    }

    fn description(&self) -> &str {
        // 平台差异写进描述（Codex A2）：Windows 侧告知代码页已统一 UTF-8 +
        // 引号/PowerShell/Unix 命令三条纪律（2026-09-07 生产 1060 条记录挖出的
        // 高频踩坑：回显 74 起 / Unix 命令全灭 20 起 / findstr 分词）
        if cfg!(windows) {
            "Execute a shell command line in the agent workspace and return combined \
stdout+stderr plus exit code. Runs via `cmd /C` with UTF-8 codepage (chcp 65001); quote \
semantics match typing the same line in a cmd window. PowerShell inline: \
`powershell -NoProfile -Command \"<code>\"` — wrap the whole code in double quotes and use \
single quotes for strings inside the code ($_ and pipes are safe inside the double quotes). \
For long scripts, write a .ps1 file and run `powershell -NoProfile -File <path>`. Unix \
commands (head/tail/grep/cat/ls) do not exist — use findstr (phrase: `/c:\"...\"`) or \
PowerShell. Non-zero exit is not an error—read exit_code and output to decide next steps. \
Long tasks: pass a larger timeout_secs."
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
        //
        // Windows 必须 raw_arg 原样直传：`.args()` 按 CommandLineToArgvW 转义（含引号/
        // 空格 → 整体包引号 + 内层 `"` 变 `\"`），cmd /C 剥外层引号后 PowerShell 收到
        // 字面引号 → -Command 得到字符串字面量 → 命令被原样回显且 exit_code=0（见
        // 模块头）。raw_arg 是 tokio Command 的 Windows 固有方法（不实现 std
        // CommandExt，infra/process.rs 注释先例）→ 必须编译期分支（CI Linux 跳过，
        // lib.rs hwnd 同款）。
        #[cfg(windows)]
        let mut cmd = {
            let full = format!("chcp 65001 >nul & {}", parsed.command);
            let mut c = tokio::process::Command::new("cmd");
            c.raw_arg("/C").raw_arg(&full);
            c
        };
        // Unix sh -c 的单参数语义与 argv 转义吻合（args 恰好包成单参），无回显问题。
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", &parsed.command]);
            c
        };
        if let Some(ws) = &ctx.workspace {
            cmd.current_dir(ws);
        }
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null())
            // 超时 drop output() future 时杀子进程：kill_on_drop 默认 false → 每次
            // 超时泄漏一个仍在跑的孤儿进程（生产库 51 起超时实锤盘点）。只杀直接
            // 子进程 cmd，孙进程不级联（树杀需 Job Object，暂不做）。
            .kill_on_drop(true);
        // Windows: 隐藏 cmd /C 弹出的控制台窗口（GUI 应用 spawn 子进程会闪窗）
        crate::infra::process::suppress_console_window(&mut cmd);

        let output = tokio::time::timeout(Duration::from_secs(parsed.timeout_secs), cmd.output())
            .await
            .map_err(|_| {
                AppError::Internal(format!(
                    "命令超时（{}s）: {}。长任务请提高 timeout_secs 参数后重试",
                    parsed.timeout_secs, parsed.command
                ))
            })?
            .map_err(|e| {
                // spawn 失败 = shell 本体没起来（cmd/sh 不在 PATH、权限问题），与
                // 命令内容无关——重试同样命令救不了，如实告知用户
                AppError::Io(std::io::Error::other(format!(
                    "run_command 启动 shell 失败: {e}。这不是命令本身的问题——\
                     系统找不到或无法执行 shell（cmd/sh），请如实告知用户检查\
                     系统环境，勿反复重试"
                )))
            })?;

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

        // 回显防御（doom nudge 尾注先例）：Ok 结果不进 doom 签名、is_error 维持
        // 事实（exit_code=0），只补纠正指引——模型看到警告即改写命令。
        if looks_like_echo(&parsed.command, &combined) {
            combined.push_str(
                "\n\n[警告] 输出与命令原文相同：命令很可能未被真正执行（PowerShell \
-Command 收到被引号包裹的字符串字面量时会原样回显且退出码为 0）。请改写：\
PowerShell 代码整体用双引号包裹、代码内部的字符串改用单引号；复杂脚本写入 \
.ps1 文件后用 powershell -File 执行。",
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
            tool_use_id: None,
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

    /// 引号直通道回归锁（2026-09-07 生产根治批）：`.args()` 转义层会让
    /// PowerShell -Command 收到字符串字面量 → 命令原样回显、exit_code=0 静默
    /// 失败（生产 1060 条记录 74 起实锤）。raw_arg 修复后含 `$_` 的内联管道
    /// 必须真正执行。修复前本测试必挂（输出为命令原文回显形态）。
    #[cfg(windows)]
    #[tokio::test]
    async fn powershell_inline_quotes_execute_not_echo() {
        let tool = RunCommandTool;
        let ctx = test_ctx().await;
        let result = tool
            .execute_with_context(
                r#"{"command":"powershell -NoProfile -Command \"Write-Output a,b | ForEach-Object { $_.ToUpper() }\"","timeout_secs":30}"#,
                &ctx,
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["exit_code"], 0);
        let out = v["output"].as_str().unwrap();
        assert!(
            out.contains('A') && out.contains('B'),
            "应真正执行并输出 A/B: {result}"
        );
        assert!(!out.contains(".ToUpper()"), "不得出现回显形态: {result}");
        assert!(
            !out.contains("[警告]"),
            "引号写法正确不应触发回显检测: {result}"
        );
    }

    /// 回显检测纯函数：生产实测形态命中、echo/短输出/无 powershell 不误报。
    #[test]
    fn looks_like_echo_detects_and_rejects() {
        // 生产实测样本形态：$_ 被字符串插值吞掉后回显原文
        let cmd = r#"powershell -NoProfile -Command "Get-ChildItem -Recurse | ForEach-Object { $_.FullName }""#;
        let echo = "Get-ChildItem -Recurse | ForEach-Object { .FullName }";
        assert!(looks_like_echo(cmd, echo));
        // 首尾空白（CRLF 等）容忍
        assert!(looks_like_echo(cmd, &format!("  {echo}\r\n")));
        // echo 命令：输出短（<15）且命令不含 powershell
        assert!(!looks_like_echo("echo hello world", "hello world"));
        // 命令不含 powershell 家族
        assert!(!looks_like_echo("findstr foo big.txt", "Get-ChildItem -Recurse"));
        // 输出非命令子串（正常命令输出）
        assert!(!looks_like_echo(cmd, "Directory: D:\\workspace"));
    }
}
