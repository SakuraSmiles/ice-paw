//! 内置 sequential thinking 工具（原生替代 bundled `server-sequential-thinking`）。
//!
//! 原实现走内置 Node + `@modelcontextprotocol/server-sequential-thinking`（连同
//! zod/sdk 等传递依赖约 124M）。本模块用 Rust 原生复刻其唯一工具 `sequentialthinking`，
//! 接口 / 参数 / 返回形状逐项对齐（camelCase 字段名 + 相同 JSON 返回），零行为变化。
//!
//! 状态语义：原 Node 版在进程内维护 `thoughtHistory`（单会话，进程生命周期内累计）。
//! 内置化后同一注册表实例被多会话共享，故按 `conv_id` 隔离（多会话并行不串味），
//! 仅用于回显 `thoughtHistoryLength` / `branches`——本工具是推理辅助，历史不落盘。

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{AppError, AppResult};

use super::client::{McpClient, ToolContext};

/// 单会话的思考状态（纯内存，进程重启清零）。
#[derive(Default)]
struct SessionThinkingState {
    history_len: usize,
    branches: Vec<String>,
}

pub struct SequentialThinkingTool {
    states: Mutex<HashMap<String, SessionThinkingState>>,
}

impl SequentialThinkingTool {
    pub fn new() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for SequentialThinkingTool {
    fn default() -> Self {
        Self::new()
    }
}

/// camelCase 对齐 Node 版 inputSchema（zod）字段名。
///
/// `thought` / `is_revision` / `revises_thought` / `needs_more_thoughts` 仅作
/// schema 保真解析（LLM 会原样下发，需能反序列化不报错），本实现只追踪历史
/// 计数与分支、不回显思考正文，故这些字段不被读取——保留以锁定接口形状。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct SequentialThinkingArgs {
    thought: String,
    #[serde(default)]
    next_thought_needed: bool,
    thought_number: i64,
    total_thoughts: i64,
    #[serde(default)]
    is_revision: bool,
    #[serde(default)]
    revises_thought: Option<i64>,
    #[serde(default)]
    branch_from_thought: Option<i64>,
    #[serde(default)]
    branch_id: Option<String>,
    #[serde(default)]
    needs_more_thoughts: bool,
}

#[async_trait]
impl McpClient for SequentialThinkingTool {
    fn name(&self) -> &str {
        "sequentialthinking"
    }

    fn description(&self) -> &str {
        "A detailed tool for dynamic and reflective problem-solving through thoughts. \
Each thought can build on, question, or revise previous insights as understanding deepens. \
Use for breaking down complex problems into steps, planning with room for revision, and \
analysis that may need course correction. You can adjust totalThoughts as you progress, \
mark revisions (isRevision/revisesThought), branch into alternative paths \
(branchFromThought/branchId), and add more thoughts even after reaching what seemed like \
the end. Set nextThoughtNeeded=false only when truly done and a satisfactory answer is reached."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "thought": {
                    "type": "string",
                    "description": "Your current thinking step, which can include regular analytical steps, revisions of previous thoughts, questions about previous decisions, hypothesis generation, or hypothesis verification."
                },
                "nextThoughtNeeded": {
                    "type": "boolean",
                    "description": "Whether another thought step is needed"
                },
                "thoughtNumber": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Current thought number (numeric value, e.g., 1, 2, 3)"
                },
                "totalThoughts": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Estimated total thoughts needed (numeric value, e.g., 5, 10)"
                },
                "isRevision": {
                    "type": "boolean",
                    "description": "Whether this revises previous thinking"
                },
                "revisesThought": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Which thought is being reconsidered"
                },
                "branchFromThought": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Branching point thought number"
                },
                "branchId": {
                    "type": "string",
                    "description": "Branch identifier"
                },
                "needsMoreThoughts": {
                    "type": "boolean",
                    "description": "If more thoughts are needed"
                }
            },
            "required": ["thought", "nextThoughtNeeded", "thoughtNumber", "totalThoughts"]
        })
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "sequentialthinking 必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let mut parsed: SequentialThinkingArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("sequentialthinking 参数解析失败: {e}")))?;

        // 对齐 Node 版 zod 校验：序号下限 1
        if parsed.thought_number < 1 {
            return Err(AppError::Validation(
                "thoughtNumber 必须 ≥ 1".to_string(),
            ));
        }
        if parsed.total_thoughts < 1 {
            return Err(AppError::Validation(
                "totalThoughts 必须 ≥ 1".to_string(),
            ));
        }

        // 对齐 Node 版：序号超过预估总数时自动上修总数
        if parsed.thought_number > parsed.total_thoughts {
            parsed.total_thoughts = parsed.thought_number;
        }

        // 更新 per-conv 状态（仅回显用）
        let mut states = self.states.lock().expect("sequential thinking state poisoned");
        let state = states.entry(ctx.conv_id.clone()).or_default();
        state.history_len += 1;
        if let (Some(_from), Some(id)) = (parsed.branch_from_thought, &parsed.branch_id) {
            if !state.branches.contains(id) {
                state.branches.push(id.clone());
            }
        }

        let result = serde_json::json!({
            "thoughtNumber": parsed.thought_number,
            "totalThoughts": parsed.total_thoughts,
            "nextThoughtNeeded": parsed.next_thought_needed,
            "branches": state.branches,
            "thoughtHistoryLength": state.history_len,
        });
        Ok(result.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        // 最小构造（仅 execute_with_context 用到的 conv_id 有意义；pool 不触碰）
        ToolContext {
            conv_id: "c1".into(),
            agent_id: "a1".into(),
            project_id: None,
            workspace: None,
            pool: sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap(),
            api_key: None,
            app_handle: None,
            proposal_registry: None,
            turn_id: None,
            cancel: None,
            tool_use_id: None,
        }
    }

    #[tokio::test]
    async fn returns_history_length_and_adjusts_total() {
        let tool = SequentialThinkingTool::new();
        let out = tool
            .execute_with_context(
                r#"{"thought":"step 1","nextThoughtNeeded":true,"thoughtNumber":3,"totalThoughts":2}"#,
                &ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["thoughtNumber"], 3);
        assert_eq!(v["totalThoughts"], 3); // 上修
        assert_eq!(v["thoughtHistoryLength"], 1);
    }

    #[tokio::test]
    async fn accumulates_per_conv() {
        let tool = SequentialThinkingTool::new();
        let args = r#"{"thought":"x","nextThoughtNeeded":true,"thoughtNumber":1,"totalThoughts":2}"#;
        let _ = tool.execute_with_context(args, &ctx()).await.unwrap();
        let out = tool.execute_with_context(args, &ctx()).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["thoughtHistoryLength"], 2);
    }

    #[tokio::test]
    async fn isolates_between_convs() {
        let tool = SequentialThinkingTool::new();
        let args = r#"{"thought":"x","nextThoughtNeeded":true,"thoughtNumber":1,"totalThoughts":2}"#;
        let _ = tool.execute_with_context(args, &ctx()).await.unwrap();
        let mut c2 = ctx();
        c2.conv_id = "c2".into();
        let out = tool.execute_with_context(args, &c2).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["thoughtHistoryLength"], 1);
    }

    #[tokio::test]
    async fn rejects_zero_thought_number() {
        let tool = SequentialThinkingTool::new();
        let err = tool
            .execute_with_context(
                r#"{"thought":"x","nextThoughtNeeded":true,"thoughtNumber":0,"totalThoughts":2}"#,
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("thoughtNumber"));
    }
}
