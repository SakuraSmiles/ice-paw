//! `update_plan` 工具 —— 会话计划（待办清单）的唯一维护通道（计划功能，C4）
//!
//! agent 的 TodoWrite 等价物：维护「本会话当前打算做什么、做到哪了」的意图文档。
//! 与任务（委派会话，执行单元）正交：计划是**会话内容**，以 `plan_updated` 事件
//! 快照形态存在（全量覆写，回放 last-wins）；条目可通过 `task_conversation_id`
//! 引用委派子会话（声明→执行的边），但勾选恒为 agent 的判断，不从任务终态映射。
//!
//! 注册在全局注册表（`register_builtin`）：委派子会话的专家也能维护自己的计划
//! ——事件天然落子会话日志，不上浮父会话（跨会话聚合是 MA-2 台账的事）。
//!
//! 授权：`Always`（无会话外副作用——只写本会话事件日志，不碰文件/网络）。

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::harness::event_log::{self, EventCtx, PlanItem, PlanUpdatedPayload};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

/// 快照上限：防失控长清单（截断历史回放与 payload 膨胀），30 步已是超长任务
const MAX_ITEMS: usize = 30;
/// 单条文本上限（字符）：清单条目是索引不是正文
const MAX_TEXT_CHARS: usize = 200;
const VALID_STATUSES: [&str; 3] = ["pending", "in_progress", "done"];

pub struct UpdatePlanTool;

#[derive(Deserialize)]
struct UpdatePlanArgs {
    steps: Vec<PlanStepArg>,
}

#[derive(Deserialize)]
struct PlanStepArg {
    text: String,
    status: String,
    #[serde(default)]
    task_conversation_id: Option<String>,
}

/// 校验 + 归一化（纯函数，便于单测）。错误文案面向 LLM：指出哪条违规 + 正确形状，
/// 模型可自修复重试（教学型错误，不是裸 400）。
fn validate(args: &UpdatePlanArgs) -> AppResult<Vec<PlanItem>> {
    if args.steps.len() > MAX_ITEMS {
        return Err(AppError::Validation(format!(
            "steps 超过上限（{} 条，最多 {MAX_ITEMS}）——请合并或精简条目",
            args.steps.len()
        )));
    }
    let mut items = Vec::with_capacity(args.steps.len());
    for (i, s) in args.steps.iter().enumerate() {
        let text = s.text.trim();
        if text.is_empty() {
            return Err(AppError::Validation(format!(
                "steps[{i}].text 为空——每条必须是自包含的步骤描述"
            )));
        }
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(AppError::Validation(format!(
                "steps[{i}].text 超过 {MAX_TEXT_CHARS} 字——条目是索引不是正文，请精简"
            )));
        }
        if !VALID_STATUSES.contains(&s.status.as_str()) {
            return Err(AppError::Validation(format!(
                "steps[{i}].status='{}' 非法——必须是 {} 之一",
                s.status,
                VALID_STATUSES.join("/")
            )));
        }
        items.push(PlanItem {
            text: text.to_string(),
            status: s.status.clone(),
            task_conversation_id: s.task_conversation_id.clone().filter(|id| !id.is_empty()),
        });
    }
    Ok(items)
}

#[async_trait]
impl McpClient for UpdatePlanTool {
    fn name(&self) -> &str {
        "update_plan"
    }

    fn description(&self) -> &str {
        "Maintain the session's plan (todo list) shown to the user. Each call REPLACES the \
         whole plan with a complete snapshot - never send a delta. Create it when starting a \
         multi-step task; after each step finishes, call again with that step marked 'done' \
         and the next 'in_progress'. Marking a step done is YOUR judgment: a delegated task \
         finishing does not automatically complete its step - if the result is unsatisfactory, \
         keep it in_progress and re-delegate. Keep at most ~8 steps; drop finished ones once \
         they no longer matter. If a step is executed via delegate_to_agent, put the returned \
         child_conversation_id in that step's task_conversation_id so the UI can link them."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "maxItems": MAX_ITEMS,
                    "description": "Complete plan snapshot (replaces the previous one). Order = execution order.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": {
                                "type": "string",
                                "description": "Self-contained step description (<=200 chars)"
                            },
                            "status": {
                                "type": "string",
                                "enum": VALID_STATUSES,
                                "description": "pending = not started, in_progress = currently working, done = completed"
                            },
                            "task_conversation_id": {
                                "type": "string",
                                "description": "Optional: child_conversation_id from delegate_to_agent if this step runs as a delegation"
                            }
                        },
                        "required": ["text", "status"]
                    }
                }
            },
            "required": ["steps"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "update_plan 必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: UpdatePlanArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("update_plan 参数解析失败: {e}")))?;
        let items = validate(&parsed)?;

        // turn_id 缺失则事件落 NULL turn → 轨迹错归「纪元前桶」。工具轮富化注入
        // （execute_tool_round），理论上必有；缺失属接线缺陷，诚实报错而非静默错分。
        let turn_id = ctx.turn_id.clone().ok_or_else(|| {
            AppError::Internal("update_plan 缺少 turn_id 上下文（工具轮富化未注入）".into())
        })?;

        // plan_updated 落库（warn-only inline await，与全体事件同一硬规则）。
        // emit 在返回前完成：前端收到工具结果时计划快照必已可查，无竞态。
        let ev = EventCtx::new(&ctx.conv_id, &turn_id, &ctx.agent_id);
        event_log::log_plan_updated(
            &ctx.pool,
            &ev,
            &PlanUpdatedPayload {
                v: 1,
                items: items.clone(),
            },
        )
        .await;

        let done = items.iter().filter(|i| i.status == "done").count();
        Ok(json!({
            "ok": true,
            "items": items.len(),
            "done": done,
            "note": "计划已更新（全量快照）"
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(steps_json: &str) -> UpdatePlanArgs {
        // LLM 实际输入形状是 {"steps":[...]}（parameters schema 定的）；此处传裸数组便于构造
        let wrapped = format!(r#"{{"steps":{steps_json}}}"#);
        serde_json::from_str(&wrapped).unwrap()
    }

    #[test]
    fn validate_accepts_normal_snapshot() {
        let a = args(
            r#"[{"text":"调研","status":"done"},{"text":"评审","status":"in_progress","task_conversation_id":"c1"}]"#,
        );
        let items = validate(&a).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].status, "done");
        assert_eq!(items[1].task_conversation_id.as_deref(), Some("c1"));
    }

    #[test]
    fn validate_allows_empty_steps_to_clear_plan() {
        let a = args("[]");
        assert!(
            validate(&a).unwrap().is_empty(),
            "空快照 = 清空计划（合法）"
        );
    }

    #[test]
    fn validate_rejects_bad_status_with_teaching_error() {
        let a = args(r#"[{"text":"x","status":"finished"}]"#);
        let err = validate(&a).unwrap_err().to_string();
        assert!(err.contains("steps[0].status"), "错误须指明哪条: {err}");
        assert!(
            err.contains("pending/in_progress/done"),
            "错误须给正确形状: {err}"
        );
    }

    #[test]
    fn validate_rejects_empty_and_overlong_text() {
        let a = args(r#"[{"text":"  ","status":"done"}]"#);
        assert!(validate(&a)
            .unwrap_err()
            .to_string()
            .contains("steps[0].text"));

        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        let a = args(&format!(r#"[{{"text":"{long}","status":"done"}}]"#));
        assert!(validate(&a).unwrap_err().to_string().contains("精简"));
    }

    #[test]
    fn validate_rejects_over_limit() {
        let steps: Vec<String> = (0..MAX_ITEMS + 1)
            .map(|i| format!(r#"{{"text":"s{i}","status":"pending"}}"#))
            .collect();
        let a = args(&format!("[{}]", steps.join(",")));
        assert!(validate(&a).unwrap_err().to_string().contains("上限"));
    }

    #[test]
    fn validate_trims_and_drops_empty_task_ref() {
        let a = args(r#"[{"text":"  步骤  ","status":"pending","task_conversation_id":""}]"#);
        let items = validate(&a).unwrap();
        assert_eq!(items[0].text, "步骤");
        assert_eq!(items[0].task_conversation_id, None, "空串引用归一为 None");
    }

    #[tokio::test]
    async fn execute_requires_turn_id_context() {
        let ctx = ToolContext {
            conv_id: "c1".into(),
            agent_id: "a1".into(),
            project_id: None,
            workspace: None,
            pool: sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap(),
            api_key: None,
            app_handle: None,
            proposal_registry: None,
            turn_id: None,
            cancel: None,
        };
        let err = UpdatePlanTool
            .execute_with_context(r#"{"steps":[{"text":"x","status":"pending"}]}"#, &ctx)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("turn_id"),
            "缺 turn_id 须诚实报错: {err}"
        );
    }
}
