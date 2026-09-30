//! `ask_user` 工具 — 向用户提出结构化选择题（回合内人机决策点）
//!
//! 治「需要用户拍板时只能文本提问」的三重成本：打字输入成本 / 自由文本
//! 解析歧义（「好的」「用第一个」→ 澄清轮）/ 回合断裂（问完即结束回合，
//! 用户手打回复再起一轮完整 Pipeline）。工具化后：点选代替打字、结构化
//! 返回零歧义、**回合不中断**（结果作为 tool_result 回到在途回合）。
//!
//! ## 形态（2026-09-30 用户拍板：工具化回合内选择 + 常驻等待）
//! - Always 级（询问 ≠ 授权——全自动开关只放行 Confirm 授权，不影响本工具：
//!   恰恰是全自动模式下用户保留拍板权的口子）
//! - 等待语义：**常驻等待（无超时）**，仅用户「停止生成」时经 cancel 中止
//! - 无人值守语境在**调用时直接拒绝**（不进入等待）：定时任务载体会话 /
//!   MA-3 跨会话来件消费回合——挂死等待会卡住调度器（并发 1）或消费队列
//!
//! ## 注册边界
//! 仅 kind='chat' 1v1 会话（session_runner 组装期注册，与 delegate/relay
//! 同闸）：频道（用户在场但多成员语义未定，v1 不开）、委派子会话（无用户）
//! 拿不到本工具。
//!
//! ## 护栏
//! 每回合提问上限 [`MAX_PER_TURN`]——防弱模型把对话变成问卷；命中即 Err
//! 三段式勒令自行决策。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use serde::Deserialize;
use tauri::{Emitter, Manager};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::infra::protocol::{
    AskUserOptionOut, AskUserRequestPayload, AskUserResponse, PendingRequestCancelPayload,
};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

/// 选项数边界（min 防伪选择、max 防问卷轰炸）
const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 6;
/// 问题长度上限（字符）——卡片单行省略 + 全文 title 的合理边界
const MAX_QUESTION_CHARS: usize = 500;
/// 选项标签长度上限（字符）
const MAX_LABEL_CHARS: usize = 80;
/// 每回合提问上限（防连环问；多问题任务应合并为一次多选或自行拆分优先级）
const MAX_PER_TURN: u8 = 5;

pub struct AskUserTool;

#[derive(Deserialize, Debug)]
struct AskUserOptionArg {
    label: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize, Debug)]
struct AskUserArgs {
    question: String,
    options: Vec<AskUserOptionArg>,
    #[serde(default)]
    multiple: Option<bool>,
    #[serde(default)]
    allow_custom: Option<bool>,
}

// =========================================================================
// 纯函数：参数校验 / 答案格式化
// =========================================================================

fn validate_args(args: &AskUserArgs) -> AppResult<()> {
    let q = args.question.trim();
    if q.is_empty() {
        return Err(AppError::Validation(
            "ask_user 参数无效: question 不能为空。请给出要问用户的问题。".into(),
        ));
    }
    if q.chars().count() > MAX_QUESTION_CHARS {
        return Err(AppError::Validation(format!(
            "ask_user 参数无效: question 超过 {MAX_QUESTION_CHARS} 字符（当前 {}）。请精简问题。",
            q.chars().count()
        )));
    }
    if args.options.len() < MIN_OPTIONS || args.options.len() > MAX_OPTIONS {
        return Err(AppError::Validation(format!(
            "ask_user 参数无效: options 需要 {MIN_OPTIONS}-{MAX_OPTIONS} 个选项（当前 {} 个）。\
             不足两个不构成选择；超过六个改拆多轮或收窄问题。",
            args.options.len()
        )));
    }
    for (i, opt) in args.options.iter().enumerate() {
        let label = opt.label.trim();
        if label.is_empty() {
            return Err(AppError::Validation(format!(
                "ask_user 参数无效: 第 {} 个选项的 label 为空。",
                i + 1
            )));
        }
        if label.chars().count() > MAX_LABEL_CHARS {
            return Err(AppError::Validation(format!(
                "ask_user 参数无效: 第 {} 个选项的 label 超过 {MAX_LABEL_CHARS} 字符。\
                 选项是短标签，长说明放 description。",
                i + 1
            )));
        }
    }
    Ok(())
}

/// 把用户作答格式化为 tool_result（JSON 结构化 + message 自然语言行——
/// agent 好解析、历史好读双格式）。
fn format_answer(resp: &AskUserResponse) -> String {
    match resp.action.as_str() {
        "answered" => {
            let mut parts: Vec<String> = Vec::new();
            if !resp.selected.is_empty() {
                parts.push(format!("用户选择了：{}", resp.selected.join("、")));
            }
            if let Some(custom) = resp.custom_text.as_deref().filter(|s| !s.trim().is_empty()) {
                parts.push(format!("并补充：{custom}"));
            }
            let message = if parts.is_empty() {
                "用户提交了空答案（未选任何选项也未输入）。请自行决策并说明。".to_string()
            } else {
                parts.join("；")
            };
            serde_json::json!({
                "status": "answered",
                "selected": resp.selected,
                "custom_text": resp.custom_text,
                "message": message,
            })
            .to_string()
        }
        _ => serde_json::json!({
            "status": "dismissed",
            "message": "用户跳过了此问题。请基于已有信息自行判断并继续，\
                        同时在结果中说明该决策未经用户确认。",
        })
        .to_string(),
    }
}

// =========================================================================
// 护栏：每回合提问计数
// =========================================================================

/// turn_id → 已问次数。回合结束无钩子可清，以容量粗上限兜底（超限整体清空
/// ——最坏情形个别回合多问几次，内存恒有界）。
static TURN_ASK_COUNTS: OnceLock<Mutex<HashMap<String, u8>>> = OnceLock::new();

fn bump_turn_count(turn_id: &str) -> u8 {
    static CAP: usize = 256;
    let registry = TURN_ASK_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut map) = registry.lock() else {
        return 1;
    };
    if map.len() > CAP {
        map.clear();
    }
    let count = map.entry(turn_id.to_string()).or_insert(0);
    *count += 1;
    *count
}

// =========================================================================
// 无人值守语境判定（数据驱动，不新增 turn 管线字段）
// =========================================================================

/// 返回拒绝原因（None = 前台用户语境，可问）。三段式文案家族前缀恒
/// 「用户选择不可用:」——稳定家族前缀是 doom_loop 签名的地基。
async fn unattended_rejection(
    pool: &sqlx::SqlitePool,
    conv_id: &str,
    turn_id: Option<&str>,
) -> Option<AppError> {
    // ① 工具轮外（hooks 等语境）无回合身份——问也无处归属
    let turn_id = match turn_id {
        Some(t) => t,
        None => {
            return Some(AppError::Validation(
                "用户选择不可用: 当前不在对话回合内（无回合身份）。请自行决策并在结果中说明。"
                    .into(),
            ))
        }
    };
    // ② 会话类型防御（组装期已按 kind 注册，此处挡直调/漂移）
    let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM conversations WHERE id = ?")
        .bind(conv_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    if kind.as_deref() != Some("chat") {
        return Some(AppError::Validation(
            "用户选择不可用: 当前会话类型不支持用户选择（仅 1v1 对话可用）。\
             请自行决策并在结果中说明。"
                .into(),
        ));
    }
    // ③ 定时任务载体（载体会话 kind 仍是 chat——按 scheduled_tasks 表判定；
    // 挂死等待会卡住全局串行调度器，必须在调用时拒绝）
    let task_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM scheduled_tasks WHERE target_conv_id = ?")
            .bind(conv_id)
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    if task_count > 0 {
        return Some(AppError::Validation(
            "用户选择不可用: 本会话是定时任务的载体，回合由计划自动触发，用户不在场。\
             请基于任务描述自行决策，并在最终结果中说明该决策未经用户确认。"
                .into(),
        ));
    }
    // ④ MA-3 跨会话来件消费回合（turn 锚消息带 incoming_source = 投递物化，
    // 发起方是另一个 agent 而非用户）
    let incoming: Option<Option<String>> =
        sqlx::query_scalar("SELECT incoming_source FROM messages WHERE id = ?")
            .bind(turn_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    if incoming.flatten().is_some() {
        return Some(AppError::Validation(
            "用户选择不可用: 本回合由跨会话来件触发，发起方是另一个 agent 而非用户。\
             请自行决策并说明；如需用户拍板，在最终回复里给出建议即可。"
                .into(),
        ));
    }
    None
}

/// 通知前端清除对应选择卡（等待失效时调用）。与 emit_auth_cancel 对称。
fn emit_ask_cancel(app: &tauri::AppHandle, request_id: &str, conv_id: &str, reason: &str) {
    let _ = app.emit(
        "chat:ask-user-request-cancel",
        PendingRequestCancelPayload {
            request_id: request_id.into(),
            conversation_id: conv_id.into(),
            reason: reason.into(),
        },
    );
}

#[async_trait]
impl McpClient for AskUserTool {
    fn name(&self) -> &str {
        "ask_user"
    }

    fn description(&self) -> &str {
        "向用户提出一个选择题并等待作答（界面呈现为可点选的选项卡）。\
         \
         适用场景：任务出现需要用户拍板的分叉——方案二选一/多选一、\
         是否继续执行有风险或不可逆的步骤、缺关键信息且无法从上下文推断。\
         用本工具代替在回复文本里提问：文本提问会结束本轮对话、用户需手打\
         回复再起一轮；本工具的作答直接返回当前回合，你据此立即继续。\
         \
         规则：question 一句话问清（不带客套）；options 2-6 个短标签选项\
         （差异说明放 description）；multiple=true 允许多选；用户可点「其他」\
         自由输入。\
         \
         不要滥用：能从上下文、项目说明或既有结论推断的不要问；每回合提问\
         不超过几次（有上限，超限会被拒绝）；开放式讨论、需要用户长文描述\
         的场景仍用正常文本回复。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "要问用户的问题（一句话，直接了当，如「用哪个方案？」）"
                },
                "options": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": {
                                "type": "string",
                                "description": "选项短标签（≤80 字符，如「方案 A：直改现有文档」）"
                            },
                            "description": {
                                "type": "string",
                                "description": "选项说明（可选，一行，说明差异/代价）"
                            }
                        },
                        "required": ["label"]
                    },
                    "description": "2-6 个选项"
                },
                "multiple": {
                    "type": "boolean",
                    "description": "是否允许多选（默认 false = 单选，点击即答）"
                },
                "allow_custom": {
                    "type": "boolean",
                    "description": "是否提供「其他」自由输入（默认 true）"
                }
            },
            "required": ["question", "options"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        // 询问不是授权面：Confirm 级会走审批卡语义（allowed/scope），与本工具
        // 的「作答」语义不符；全自动开关只翻 Confirm，本工具恒直达用户。
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "ask_user 必须通过 execute_with_context 调用（需要 conv/turn 上下文）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: AskUserArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("ask_user 参数解析失败: {e}")))?;
        validate_args(&parsed)?;

        // 无人值守语境：调用即拒（不进入等待——挂死会卡调度器/消费队列）
        if let Some(err) =
            unattended_rejection(&ctx.pool, &ctx.conv_id, ctx.turn_id.as_deref()).await
        {
            return Err(err);
        }

        // 每回合护栏
        let turn_key = ctx.turn_id.clone().unwrap_or_default();
        if bump_turn_count(&turn_key) > MAX_PER_TURN {
            return Err(AppError::Validation(format!(
                "ask_user 提问过于频繁: 本回合已达到提问上限（{MAX_PER_TURN} 次）。\
                 请停止提问，基于已有信息自行决策并在结果中说明。"
            )));
        }

        let app = ctx.app_handle.as_ref().ok_or_else(|| {
            AppError::Internal("ask_user: app_handle 未注入到 ToolContext".into())
        })?;
        let registry = app
            .try_state::<crate::harness::oneshot_registry::AskUserRegistry>()
            .map(|s| s.inner().clone())
            .ok_or_else(|| {
                AppError::Internal("ask_user: AskUserRegistry 未注册到应用状态".into())
            })?;

        let request_id = Uuid::new_v4().to_string();
        tracing::info!(
            target: "ice_paw.chat",
            request_id = %request_id,
            conv_id = %ctx.conv_id,
            options = parsed.options.len(),
            multiple = parsed.multiple.unwrap_or(false),
            "ask_user: 已向用户发出选择请求"
        );

        let rx = registry.register(request_id.clone()).await;
        let payload = AskUserRequestPayload {
            request_id: request_id.clone(),
            conversation_id: ctx.conv_id.clone(),
            tool_use_id: ctx.tool_use_id.clone().unwrap_or_default(),
            question: parsed.question.trim().to_string(),
            options: parsed
                .options
                .iter()
                .map(|o| AskUserOptionOut {
                    label: o.label.trim().to_string(),
                    description: o
                        .description
                        .as_deref()
                        .map(str::trim)
                        .filter(|d| !d.is_empty())
                        .map(str::to_string),
                })
                .collect(),
            multiple: parsed.multiple.unwrap_or(false),
            allow_custom: parsed.allow_custom.unwrap_or(true),
        };
        if let Err(e) = app.emit("chat:ask-user-request", payload) {
            let _ = registry.take(&request_id).await;
            return Err(AppError::Internal(format!(
                "ask_user: 无法发送选择请求事件: {e}"
            )));
        }

        // 常驻等待（无超时，2026-09-30 拍板）：仅用户停止生成（cancel）中止。
        // 用户长时间不答 = 回合停着——与「用户半天不回打字问题」同语义。
        let outcome = match ctx.cancel.as_ref() {
            Some(cancel) => tokio::select! {
                biased;
                _ = crate::harness::tool_executor::wait_for_cancel(cancel) => {
                    let _ = registry.take(&request_id).await;
                    emit_ask_cancel(app, &request_id, &ctx.conv_id, "abort");
                    tracing::info!(
                        target: "ice_paw.chat",
                        request_id = %request_id,
                        "ask_user: 对话被用户停止，问题未获回答"
                    );
                    return Ok(serde_json::json!({
                        "status": "cancelled",
                        "message": "用户停止了生成，问题未获回答。"
                    })
                    .to_string());
                }
                r = rx => r,
            },
            None => rx.await,
        };

        match outcome {
            Ok(resp) => {
                tracing::info!(
                    target: "ice_paw.chat",
                    request_id = %request_id,
                    action = %resp.action,
                    selected = resp.selected.len(),
                    "ask_user: 用户已作答"
                );
                Ok(format_answer(&resp))
            }
            Err(_) => {
                // sender 被 drop（等待者被清理）→ 视为取消
                emit_ask_cancel(app, &request_id, &ctx.conv_id, "cancelled");
                tracing::warn!(
                    target: "ice_paw.chat",
                    request_id = %request_id,
                    "ask_user: 响应通道已关闭（可能被取消）"
                );
                Ok(serde_json::json!({
                    "status": "cancelled",
                    "message": "问题等待被取消，未获回答。"
                })
                .to_string())
            }
        }
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn args(question: &str, labels: &[&str]) -> AskUserArgs {
        AskUserArgs {
            question: question.into(),
            options: labels
                .iter()
                .map(|l| AskUserOptionArg {
                    label: (*l).into(),
                    description: None,
                })
                .collect(),
            multiple: None,
            allow_custom: None,
        }
    }

    #[test]
    fn validate_bounds() {
        assert!(validate_args(&args("选哪个？", &["A", "B"])).is_ok());
        assert!(validate_args(&args("", &["A", "B"])).is_err(), "空问题拒绝");
        assert!(
            validate_args(&args("选哪个？", &["A"])).is_err(),
            "单选项不构成选择"
        );
        let seven = ["1", "2", "3", "4", "5", "6", "7"];
        assert!(
            validate_args(&args("选哪个？", &seven)).is_err(),
            "超六选项拒绝"
        );
        let long: String = "字".repeat(501);
        assert!(
            validate_args(&args(&long, &["A", "B"])).is_err(),
            "超长问题拒绝"
        );
        let mut a = args("选哪个？", &["A", "B"]);
        a.options[0].label = "  ".into();
        assert!(validate_args(&a).is_err(), "空 label 拒绝");
    }

    #[test]
    fn format_answer_shapes() {
        let answered = AskUserResponse {
            request_id: "r1".into(),
            action: "answered".into(),
            selected: vec!["方案 A".into()],
            custom_text: None,
        };
        let v: serde_json::Value = serde_json::from_str(&format_answer(&answered)).unwrap();
        assert_eq!(v["status"], "answered");
        assert_eq!(v["message"], "用户选择了：方案 A");

        let both = AskUserResponse {
            request_id: "r2".into(),
            action: "answered".into(),
            selected: vec!["A".into(), "B".into()],
            custom_text: Some("先做 A".into()),
        };
        let v: serde_json::Value = serde_json::from_str(&format_answer(&both)).unwrap();
        assert_eq!(v["message"], "用户选择了：A、B；并补充：先做 A");

        let dismissed = AskUserResponse {
            request_id: "r3".into(),
            action: "dismissed".into(),
            selected: vec![],
            custom_text: None,
        };
        let v: serde_json::Value = serde_json::from_str(&format_answer(&dismissed)).unwrap();
        assert_eq!(v["status"], "dismissed");
        assert!(v["message"].as_str().unwrap().contains("未经用户确认"));
    }

    #[test]
    fn turn_count_caps() {
        // 独立 key 防并发测试互染
        let key = format!("test-turn-{}", uuid::Uuid::new_v4());
        for i in 1..=MAX_PER_TURN {
            assert_eq!(bump_turn_count(&key), i);
        }
        assert!(
            bump_turn_count(&key) > MAX_PER_TURN,
            "超限后计数继续涨（由调用方拒绝）"
        );
    }

    // ---- 无人值守判定（in-memory sqlite + migrations）----

    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;
    use std::str::FromStr;

    async fn gate_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES ('agent-1', 't', 'anthropic', 'claude-test', '', '', 0.7, 1024, '{}', 0, 0)",
        )
        .execute(&pool)
        .await
        .expect("seed agent");
        sqlx::query(
            "INSERT INTO conversations (id, agent_id, title, kind) VALUES ('conv-1', 'agent-1', 't', 'chat')",
        )
        .execute(&pool)
        .await
        .expect("seed conversation");
        pool
    }

    #[tokio::test]
    async fn gate_allows_foreground_chat_turn() {
        let pool = gate_pool().await;
        sqlx::query("INSERT INTO messages (id, conversation_id, role, content) VALUES ('m1', 'conv-1', 'user', 'hi')")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            unattended_rejection(&pool, "conv-1", Some("m1"))
                .await
                .is_none(),
            "普通 1v1 用户回合应放行"
        );
    }

    #[tokio::test]
    async fn gate_rejects_scheduled_carrier() {
        let pool = gate_pool().await;
        sqlx::query("INSERT INTO messages (id, conversation_id, role, content) VALUES ('m1', 'conv-1', 'user', 'hi')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO scheduled_tasks (id, name, agent_id, schedule_kind, schedule_data, prompt, target_conv_id, miss_policy, next_run)
             VALUES ('task-1', 't', 'agent-1', 'once', '{}', '汇总进展', 'conv-1', 'skip', '2030-01-01 00:00:00')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let err = unattended_rejection(&pool, "conv-1", Some("m1"))
            .await
            .expect("定时任务载体应拒绝");
        assert!(err.to_string().contains("用户选择不可用"));
    }

    #[tokio::test]
    async fn gate_rejects_incoming_consumption() {
        let pool = gate_pool().await;
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, role, content, incoming_source) \
             VALUES ('m1', 'conv-1', 'user', 'hi', '{\"from\":\"a\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let err = unattended_rejection(&pool, "conv-1", Some("m1"))
            .await
            .expect("跨会话来件消费回合应拒绝");
        assert!(err.to_string().contains("跨会话来件"));
    }

    #[tokio::test]
    async fn gate_rejects_non_chat_kind_and_no_turn() {
        let pool = gate_pool().await;
        assert!(
            unattended_rejection(&pool, "conv-1", None).await.is_some(),
            "工具轮外（无 turn）应拒绝"
        );
        sqlx::query(
            "INSERT INTO conversations (id, agent_id, title, kind) VALUES ('conv-ch', 'agent-1', 't', 'channel')",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            unattended_rejection(&pool, "conv-ch", Some("x"))
                .await
                .is_some(),
            "频道 kind 应拒绝（防御位）"
        );
    }
}
