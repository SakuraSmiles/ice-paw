//! `spawn_worker` 工具——临时 Worker 原语（频道 Router 化 Phase 1，批 P1-2）
//!
//! 设计真相源：docs/channel-router-roadmap.md §6.2（WorkerSpec / 治理三件套）。
//!
//! 父 agent（1v1 会话内）按完整岗位说明书**现场创建临时 worker**：ephemeral
//! agent 行 + 独立子会话（kind='worker'）跑完整回合（与 delegate 同一执行引擎
//! `run_agent_turn`），完成后回传结果。worker 用完即走：不进用户 agent 列表
//! （批 P1-1 的 visibleAgents 过滤）、不可编辑、自动过期（过期清理批 P1-5）。
//!
//! ## 治理三件套（批内落地进度）
//!
//! - **权限闸（本批）**：worker 工具面 ⊆ 创建者工具面（`scope_allows` 逐工具
//!   判定，组展开语义复用 tool_scopes）；本批只放行**只读裸工具名单**
//!   （[`READONLY_WORKER_SCOPES`]），宽工具面（group:/server:/写类裸名）暂 Err
//!   拒绝——批 P1-3 接审批卡（delegation-auth 形态）后放开。
//! - **成本闸（批 P1-4）**：`max_total_tokens` 随 spec 落 worker 行 extra_params
//!   （AgentRow::max_total_tokens 语义位，零迁移），硬停分支下一批接线。
//! - **深度闸（本批，结构性）**：本工具仅 kind='chat' 会话注册（session_runner
//!   组装期，与 delegate 同闸）——worker 子会话（kind='worker'）拿不到
//!   spawn_worker/delegate，「外包不能再雇外包」在结构上不可能。
//!   ⚠️ 勿将 spawn_worker 加入 PLATFORM_TOOLS（收窄恒保留名单）——那会让
//!   worker 会话在任意收窄下仍拿回 spawn 权，深度闸被击穿。
//!
//! ## 密钥纪律
//!
//! `model_profile_id` 必填、api_key 恒空出生——凭据经既有 profile 引用解析
//! （`get_with_credentials`）取得，**说明书永不内联密钥**（继承提案卡
//! `__SLOT__` 精神：审批/创建界面不是凭据收集界面）。
//!
//! ## 与 delegate 的分工
//!
//! delegate = 派给**存量** agent（用户配置的专家，带记忆/KB 积累）；
//! spawn_worker = 系统无合适人选时**现场造人**（一次性/隔离/并行扇出场景）。
//! 两者执行面同构（子会话 + run_agent_turn + 活性护栏），Phase 2 起 start/stop
//! 异步变体共享同一底座。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tauri::{Emitter, Manager};
use uuid::Uuid;

use crate::commands::agent_cmd::AgentCmd;
use crate::db::models::{NewAgent, NewConversation};
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::chat_state::ChatState;
use crate::harness::read_route::ReadRouteRegistry;
use crate::harness::session_runner::{self, AgentTurnInput, TurnEnv};
use crate::infra::protocol::{ContentBlock, DelegationStartedPayload};

use super::client::{McpClient, ToolContext};
use super::manager::McpServerManager;
use super::tool_scopes::scope_allows;
use super::types::AuthorizationLevel;
use super::McpRegistry;

pub struct SpawnWorkerTool;

/// 本批放行的只读裸工具名单（缺省工具面 = 这份名单）。
/// 刻意不用 `group:` 条目——组是写读混装快照（files 含 write_file/run_command，
/// kb 含 save_to_kb），只读语义必须裸名枚举。
pub(crate) const READONLY_WORKER_SCOPES: &[&str] = &[
    "read_file",
    "list_directory",
    "directory_tree",
    "get_file_info",
    "read_multiple_files",
    "search_files",
    "view_image",
    "web_fetch",
    "search_kb",
    "read_kb_document",
];

#[derive(Deserialize)]
struct SpawnWorkerArgs {
    /// 创建理由（审计：为什么需要临时工而非存量 agent）
    purpose: String,
    /// worker 完整职责 spec（出生 system_prompt）
    role_prompt: String,
    /// 模型配置引用（必填——禁手填 provider/model/key）
    model_profile_id: String,
    /// 首个任务（自包含，worker 看到的全部输入，同 delegate 语义）
    task: String,
    /// 请求工具面（缺省 = 只读名单；本批只放行只读裸名）
    #[serde(default)]
    tool_scopes: Option<Vec<String>>,
    /// 预算硬帽（token；缺省 = 系统默认 3× context_window。批 P1-4 接线硬停）
    #[serde(default)]
    max_total_tokens: Option<i64>,
}

/// 工具面分级（批 P1-3 起：宽面不再参数层拒绝，改走审批卡）。
/// ReadOnly = 全部条目命中只读名单（含空缺省）；Wide = 含 group:/server:/
/// 只读名单外的裸名（写类）。Wide 在有人值守语境弹审批卡，无人值守直接拒。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeTier {
    ReadOnly,
    Wide,
}

fn classify_scope_tier(scopes: &[String]) -> ScopeTier {
    let wide = scopes
        .iter()
        .any(|raw| !READONLY_WORKER_SCOPES.contains(&raw.as_str()));
    if wide {
        ScopeTier::Wide
    } else {
        ScopeTier::ReadOnly
    }
}

/// 无人值守语境的 worker 家族短语（判定腿复用 ask_user_tool::unattended_reason，
/// 文案各挂各的家族前缀——完整三段式在调用点组装）。
fn unattended_reason_text(reason: super::ask_user_tool::UnattendedReason) -> &'static str {
    use super::ask_user_tool::UnattendedReason::*;
    match reason {
        NoTurn => "当前不在对话回合内",
        NonChatKind => "当前会话类型不支持用户审批",
        ScheduledCarrier => "本会话是定时任务的载体，回合由计划自动触发",
        IncomingConsumed => "本回合由跨会话来件触发，发起方是另一个 agent",
    }
}

/// 参数校验（纯函数，单测锁定）。
/// 返回 Err = 家族前缀稳定的三段式错误（doom_detect 依赖首行前缀稳定）。
fn validate_spawn_args(args: &SpawnWorkerArgs) -> AppResult<()> {
    if args.purpose.trim().is_empty() {
        return Err(AppError::Validation(
            "临时工作者创建失败: purpose 为空。请说明为什么需要临时 worker（审计与命名都用它）".into(),
        ));
    }
    if args.role_prompt.trim().is_empty() {
        return Err(AppError::Validation(
            "临时工作者创建失败: role_prompt 为空。worker 靠它知道自己的职责，必须完整自包含".into(),
        ));
    }
    if args.model_profile_id.trim().is_empty() {
        return Err(AppError::Validation(
            "临时工作者创建失败: model_profile_id 为空。worker 必须引用既有模型配置（设置-模型页管理），不接受手填厂商/模型/密钥".into(),
        ));
    }
    if args.task.trim().is_empty() {
        return Err(AppError::Validation(
            "临时工作者创建失败: task 为空。任务描述必须自包含（这是 worker 收到的全部输入）".into(),
        ));
    }
    Ok(())
}

/// 创建者工具面闸（纯函数）：请求条目逐工具 ⊆ 创建者有效面。
/// 创建者 scopes 空 = 全开（任何请求都过）；非空 = 每个请求工具都必须被
/// `scope_allows` 判定覆盖（组展开语义复用——创建者配了 group:files 时请求
/// read_file 裸名也覆盖）。返回被拒的工具名（空 = 全过）。
fn creator_scope_violations(
    creator_scopes: &[String],
    requested: &[String],
) -> Vec<String> {
    requested
        .iter()
        .filter(|tool| !scope_allows(creator_scopes, tool, None))
        .cloned()
        .collect()
}

/// worker 命名：`临时·{purpose 摘要}·{id 短码}`（委派卡/台账可读归属）
fn worker_display_name(purpose: &str, id: &str) -> String {
    let summary =
        crate::infra::strings::truncate_to_byte_boundary(purpose.trim(), 24, Some("…"));
    let short = &id.replace('-', "")[..4.min(id.replace('-', "").len())];
    format!("临时·{summary}·{short}")
}

#[async_trait]
impl McpClient for SpawnWorkerTool {
    fn name(&self) -> &str {
        "spawn_worker"
    }

    fn description(&self) -> &str {
        "Create an ephemeral temporary worker agent to execute a one-off task. \
Use when NO existing agent fits (existing agents are preferred - they carry memory/context). \
The worker gets its own session, runs the task with the model profile you reference, \
and disappears afterwards (not user-visible, auto-expiring). Read-only tools only in this version."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "purpose": {
                    "type": "string",
                    "description": "Why a temporary worker is needed instead of an existing agent (audit trail, also used for naming)."
                },
                "role_prompt": {
                    "type": "string",
                    "description": "Complete role specification for the worker (becomes its system prompt). Must be self-contained."
                },
                "model_profile_id": {
                    "type": "string",
                    "description": "Model profile id to use (see list_model_profiles tool output). Required - provider/model/key cannot be specified inline."
                },
                "task": {
                    "type": "string",
                    "description": "Self-contained first task - the only input the worker receives."
                },
                "tool_scopes": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Requested tool scope entries. Defaults to a read-only set. Only read-only tool names are accepted in this version."
                },
                "max_total_tokens": {
                    "type": "integer",
                    "description": "Optional hard budget cap in tokens for this worker."
                }
            },
            "required": ["purpose", "role_prompt", "model_profile_id", "task"]
        })
    }

    /// 与 delegate 同款先例：授权决策点是工具内部治理层（宽工具面审批卡，
    /// 批 P1-3），工具本身恒 Allow 不进通用 Confirm 通道。
    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "spawn_worker 必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: SpawnWorkerArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("spawn_worker 参数解析失败: {e}")))?;
        validate_spawn_args(&parsed)?;

        let app = ctx.app_handle.clone().ok_or_else(|| {
            AppError::Internal("spawn_worker 需要 App 上下文（app_handle）".into())
        })?;

        // --- 1. 模型配置引用解析（密钥纪律：只认 profile，禁内联） ---
        let profile = repo::model_profile::get_by_id(&ctx.pool, &parsed.model_profile_id)
            .await
            .map_err(|_| {
                AppError::Validation(format!(
                    "临时工作者创建失败: 模型配置「{}」不存在。请先用 list_model_profiles 查有效 id",
                    parsed.model_profile_id
                ))
            })?;

        // --- 1.5 宽工具面审批闸（批 P1-3：只读自动放行 / 宽面弹卡 / 无人值守拒） ---
        // 复用委派授权通道（chat:delegation-auth-request 同款卡 + oneshot 120s +
        // 后台栈/toast 全复用，前端零改动）。worker_id 先生成供卡显示，批准才落库
        // （拒绝 = 零残留）。DelegationGrant 两档对 worker 均按放行处理（命令免问
        // 档的子会话 run_command 预授权语义不适用于 scopes 面本身，忽略附加档）。
        let requested_scopes: Vec<String> = parsed
            .tool_scopes
            .clone()
            .unwrap_or_else(|| READONLY_WORKER_SCOPES.iter().map(|s| s.to_string()).collect());
        let worker_id = Uuid::new_v4().to_string();
        let worker_name = worker_display_name(&parsed.purpose, &worker_id);
        if classify_scope_tier(&requested_scopes) == ScopeTier::Wide {
            // 无人值守语境宽面即拒（120s 必超时挂死工具轮——同 ask_user 治理：
            // 挂死会卡调度器/消费队列；判定四腿复用 ask_user_tool::unattended_reason）
            if let Some(reason) = super::ask_user_tool::unattended_reason(
                &ctx.pool,
                &ctx.conv_id,
                ctx.turn_id.as_deref(),
            )
            .await
            {
                return Err(AppError::AuthorizationRequired {
                    tool: "spawn_worker".into(),
                    reason: format!(
                        "临时工作者宽工具面需用户批准，但当前语境无人值守（{}）——\
                         请改用只读工具面（缺省即是），或改为在最终回复中向用户建议创建",
                        unattended_reason_text(reason)
                    ),
                });
            }
            let auth_registry = app
                .state::<crate::harness::tool_executor::ToolAuthRegistry>()
                .inner()
                .clone();
            let auth_request_id = Uuid::new_v4().to_string();
            let rx = auth_registry.register(auth_request_id.clone()).await;
            let auth_payload = crate::infra::protocol::DelegationAuthRequestPayload {
                request_id: auth_request_id.clone(),
                conversation_id: ctx.conv_id.clone(),
                message_id: ctx.turn_id.clone().unwrap_or_default(),
                agent_name: worker_name.clone(),
                agent_id: worker_id.clone(),
                // 卡主体呈现：创建理由 + 请求的权限面（用户批的是这份清单）
                task: format!(
                    "创建临时工作者：{}\n请求工具面：{}",
                    parsed.purpose.trim(),
                    requested_scopes.join("、")
                ),
            };
            if let Err(e) = app.emit("chat:delegation-auth-request", &auth_payload) {
                let _ = auth_registry.take(&auth_request_id).await;
                return Err(AppError::Internal(format!("无法通知前端审批请求: {e}")));
            }
            tracing::info!(
                target: "ice_paw.worker",
                parent_conv = %ctx.conv_id,
                request_id = %auth_request_id,
                scopes = requested_scopes.join(","),
                "等待用户批准宽工具面 worker（120s 超时）"
            );
            let auth_emitter =
                crate::harness::r#loop::emitter::tauri_emitter(app.clone(), ctx.conv_id.clone());
            let parent_cancel = ctx.cancel.clone().ok_or_else(|| {
                AppError::Internal("spawn_worker 缺少父会话取消令牌".into())
            })?;
            match crate::harness::tool_executor::wait_for_auth_response(
                rx,
                &parent_cancel,
                &auth_request_id,
                &auth_registry,
                auth_emitter.as_ref(),
                &ctx.conv_id,
            )
            .await
            {
                Some(resp) if resp.allowed => { /* 放行（附加档忽略，见上注） */ }
                Some(_) => {
                    return Err(AppError::AuthorizationRequired {
                        tool: "spawn_worker".into(),
                        reason: format!(
                            "用户拒绝了这次临时工作者创建（{}，工具面：{}）——\
                             请与用户确认意图，或改用缺省只读工具面重试",
                            parsed.purpose.trim(),
                            requested_scopes.join("、")
                        ),
                    });
                }
                None => {
                    return Err(AppError::AuthorizationRequired {
                        tool: "spawn_worker".into(),
                        reason: "临时工作者创建授权被取消或超时（120 秒内未获用户批准）\
                                 ——请改用缺省只读工具面，或在回复中向用户说明需求"
                            .into(),
                    });
                }
            }
        }

        // --- 2. 创建者工具面闸（治理①：worker ⊆ 创建者；审批不能越权——
        //        用户批准的是「请求面」，创建者面闸恒在此后独立执行） ---
        let creator = repo::agent::get_by_id(&ctx.pool, &ctx.agent_id).await?;
        let violations = creator_scope_violations(
            creator
                .tool_scopes
                .as_deref()
                .map(|s| serde_json::from_str::<Vec<String>>(s).unwrap_or_default())
                .unwrap_or_default()
                .as_slice(),
            &requested_scopes,
        );
        if !violations.is_empty() {
            return Err(AppError::Validation(format!(
                "临时工作者创建失败: 请求的工具超出你自己的工具面（{}）。\
                 worker 权限只能是创建者的子集——请去掉这些条目后重试",
                violations.join("、")
            )));
        }

        // --- 3. 建 ephemeral agent 行（内部路径直调 repo：不走 create_agent 命令层，
        //        无 yaml 生成、无物化——引用已显式；worker_id/name 已在审批段生成） ---
        let mut extra = serde_json::Map::new();
        if let Some(cap) = parsed.max_total_tokens {
            extra.insert(
                "max_total_tokens".into(),
                serde_json::json!(cap),
            );
        }
        extra.insert(
            "worker_purpose".into(),
            serde_json::json!(parsed.purpose.trim()),
        );
        let worker_row = repo::agent::create(
            &ctx.pool,
            &NewAgent {
                id: worker_id.clone(),
                name: worker_name.clone(),
                // 快照列出生值来自 profile（引用模式的解析回写由
                // get_with_credentials 每轮对齐，与存量 agent 同机制）
                provider: profile.provider.clone(),
                model: profile.model.clone(),
                system_prompt: parsed.role_prompt.clone(),
                api_key: String::new(),
                base_url: profile.base_url.clone(),
                temperature: 0.7,
                max_tokens: 8192,
                extra_params: Some(serde_json::Value::Object(extra)),
                sort_order: 0,
                cache_prompt: true,
                supports_vision: false,
                max_history_messages: None,
                context_window: None,
                enabled_tools: None,
                workspace_path: None,
                avatar: None,
                model_profile_id: Some(profile.id.clone()),
                fallback_profile_ids: None,
                // 内部字段（serde skip）：用户路径恒 None，此处引擎显式 ephemeral
                agent_kind: Some("ephemeral".into()),
            },
            &worker_id,
            &worker_id,
        )
        .await?;
        // tool_scopes 不进出生创建（NewAgent 无此字段——旋钮唯一写入通道语义）；
        // 组装期收窄读 DB 行（镜像不变式），worker 无 yaml → 直接 repo UPDATE 落列
        if !requested_scopes.is_empty() {
            repo::agent::update(
                &ctx.pool,
                &worker_id,
                &repo::agent::AgentRepoUpdate {
                    tool_scopes: Some(Some(requested_scopes.clone())),
                    ..Default::default()
                },
            )
            .await?;
        }

        // --- 4. 凭据经 profile 引用解析（worker 行 api_key 恒空出生） ---
        let agent_cmd = app.state::<Arc<dyn AgentCmd>>().inner().clone();
        let creds = agent_cmd.get_with_credentials(&worker_id).await.map_err(|e| {
            AppError::Internal(format!(
                "临时工作者凭据解析失败（模型配置「{}」可能缺 Key）: {e}——\
                 请如实告知用户检查该模型配置",
                profile.alias
            ))
        })?;
        let worker_provider = crate::harness::provider::create_provider(
            &creds.agent.provider,
            &creds.agent.model,
            creds.base_url.as_deref(),
            creds.agent.cache_prompt != 0,
        )
        .map_err(|e| {
            AppError::Internal(format!(
                "为临时工作者创建 provider 失败（{}/{}）: {e}",
                creds.agent.provider, creds.agent.model
            ))
        })?;

        // --- 5. 建子会话（kind='worker'：侧栏 isUserChat 正向白名单天然排除；
        //        委派候选/台账过滤见批 P1-4 收录） ---
        let parent = repo::conversation::get_by_id(&ctx.pool, &ctx.conv_id).await?;
        let title = crate::infra::strings::truncate_to_byte_boundary(parsed.task.trim(), 60, Some("…"));
        let child_conv_id = Uuid::new_v4().to_string();
        let child_conv = repo::conversation::create(
            &ctx.pool,
            &child_conv_id,
            &NewConversation {
                agent_id: worker_id.clone(),
                title: Some(title.clone()),
                project_id: parent.project_id.clone(),
                kind: Some("worker".into()),
                initiator_agent_id: Some(ctx.agent_id.clone()),
                parent_conversation_id: Some(ctx.conv_id.clone()),
                auto_approve: (parent.auto_approve != 0).then_some(1),
            },
        )
        .await?;
        crate::harness::auto_approve::global()
            .inherit(&ctx.conv_id, &child_conv_id)
            .await;

        // --- 6. cancel 级联 + ChatState 注册（RAII 兜底注销，同 delegate） ---
        let parent_cancel = ctx
            .cancel
            .clone()
            .ok_or_else(|| AppError::Internal("spawn_worker 缺少父会话取消令牌".into()))?;
        let child_cancel = parent_cancel.child_token();
        let chat_state = app.state::<ChatState>().inner().clone();
        chat_state.register(&child_conv_id, child_cancel.clone());
        let guard_conv_id = child_conv_id.clone();
        let cancel_guard =
            scopeguard::guard((), |_| chat_state.unregister(&guard_conv_id));

        // 复用委派卡通道（前端零改动即可呈现运行中 worker 卡；批 P1-4 换
        // worker 专属事件词表）
        let _ = app.emit(
            "chat:delegation-started",
            DelegationStartedPayload {
                conversation_id: ctx.conv_id.clone(),
                child_conversation_id: child_conv_id.clone(),
                agent_name: worker_name.clone(),
                title: title.clone(),
                tool_use_id: ctx.tool_use_id.clone(),
            },
        );

        tracing::info!(
            target: "ice_paw.worker",
            parent_conv = %ctx.conv_id,
            worker_conv = %child_conv_id,
            worker_agent = %worker_id,
            creator = %ctx.agent_id,
            profile = %profile.id,
            scopes = requested_scopes.join(","),
            "临时工作者创建：worker 跑完整子会话（ephemeral）"
        );

        // --- 7. worker 跑完整回合 + inline await（同步形态；异步 start/stop 是
        //        Phase 2 批 P2-a——届时本段拆「受理回执 + 回投」两半） ---
        let mut done_rx = session_runner::run_agent_turn(
            &TurnEnv {
                emitter: crate::harness::r#loop::emitter::tauri_emitter(
                    app.clone(),
                    child_conv_id.clone(),
                ),
                tool_app: Some(app.clone()),
                pool: ctx.pool.clone(),
                route_registry: app.state::<ReadRouteRegistry>().inner(),
                chat_state: chat_state.clone(),
                global_registry: Arc::clone(app.state::<Arc<McpRegistry>>().inner()),
                mcp_manager: Arc::clone(app.state::<Arc<McpServerManager>>().inner()),
                auth_registry: app
                    .state::<crate::harness::tool_executor::ToolAuthRegistry>()
                    .inner()
                    .clone(),
                auth_sessions: app
                    .state::<crate::harness::authority::AuthSessionRegistry>()
                    .inner()
                    .clone(),
            },
            AgentTurnInput {
                conv: child_conv,
                agent: creds.agent.clone(),
                hooks: creds.hooks,
                word_style_profile: creds.word_style_profile,
                provider: worker_provider,
                api_key: creds.api_key,
                user_msg_id: Uuid::new_v4().to_string(),
                content_text: parsed.task.clone(),
                relevance_query: None,
                llm_blocks: vec![ContentBlock::text(parsed.task.clone())],
                persist_blocks: vec![ContentBlock::text(parsed.task.clone())],
                attach_db_inputs: Vec::new(),
                attach_file_inputs: Vec::new(),
                emit_user_blocks: false,
                incoming_source: None,
                sender_name: None,
                pre_materialized: false,
                tools_enabled: true,
                model_override: None,
                cancel_token: child_cancel,
                // 无链 worker 继承创建者配置链（同委派继承语义；worker 行
                // 自身 fallback_profile_ids 恒空出生）
                fallback: crate::harness::fallback_plan::delegation_fallback_plan(
                    &app,
                    &ctx.pool,
                    &worker_row,
                    &ctx.agent_id,
                )
                .await,
            },
        )
        .await?;
        let _ = scopeguard::ScopeGuard::into_inner(cancel_guard);

        // --- 8. 活性护栏 await（与 delegate 同款：5min 窗口查注册表续期） ---
        const ACTIVITY_POLL_SECS: u64 = 5 * 60;
        let summary = loop {
            match tokio::time::timeout(Duration::from_secs(ACTIVITY_POLL_SECS), &mut done_rx)
                .await
            {
                Ok(Ok(summary)) => break summary,
                Err(_) if chat_state.is_streaming(&child_conv_id) => {
                    tracing::info!(
                        target: "ice_paw.worker",
                        worker_conv = %child_conv_id,
                        "worker 护栏续期（子会话仍在处理）"
                    );
                    continue;
                }
                Err(_) => {
                    return Err(AppError::Internal(format!(
                        "临时工作者子会话异常中止（注册状态消失且完成信号未到达）——\
                         请如实告知用户；worker 会话 {child_conv_id} 已落库的部分可在轨迹页查看"
                    )));
                }
                Ok(Err(_)) => {
                    return Err(AppError::Internal(
                        "临时工作者子会话流式循环异常退出（未产出完成信号）——请如实告知用户该任务失败"
                            .into(),
                    ));
                }
            }
        };

        // --- 9. tool_result（结构对齐 delegate：模型可引用 child 会话续追） ---
        let progress = crate::harness::mcp::delegate::collect_progress(
            &ctx.pool,
            &child_conv_id,
        )
        .await;
        let normal = crate::harness::mcp::delegate::is_normal_completion(&summary.finish_reason);
        let mut out = serde_json::json!({
            "worker_conversation_id": child_conv_id,
            "worker_agent_id": worker_id,
            "finish_reason": summary.finish_reason,
            "rounds": summary.rounds,
            "response": summary.final_text,
            "progress": progress,
        });
        if !normal {
            out["note"] = serde_json::json!(
                "worker 可能未完整完成任务（非正常终止）——结果可能不完整，请核对后决定是否重试"
            );
        }
        Ok(out.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(purpose: &str, scopes: Option<Vec<&str>>) -> SpawnWorkerArgs {
        SpawnWorkerArgs {
            purpose: purpose.into(),
            role_prompt: "你是文档统计员".into(),
            model_profile_id: "mp-1".into(),
            task: "统计目录文件数".into(),
            tool_scopes: scopes.map(|v| v.into_iter().map(String::from).collect()),
            max_total_tokens: None,
        }
    }

    #[test]
    fn validate_rejects_empty_fields() {
        // purpose 空
        let mut a = args("", None);
        let err = validate_spawn_args(&a).expect_err("应拒绝");
        assert!(err.to_string().contains("purpose"), "{err}");
        // role_prompt 空白
        a = args("p", None);
        a.role_prompt = "  ".into();
        let err = validate_spawn_args(&a).expect_err("应拒绝");
        assert!(err.to_string().contains("role_prompt"), "{err}");
        // model_profile_id 空
        a = args("p", None);
        a.model_profile_id = " ".into();
        let err = validate_spawn_args(&a).expect_err("应拒绝");
        assert!(err.to_string().contains("model_profile_id"), "{err}");
        // task 空
        a = args("p", None);
        a.task = " ".into();
        assert!(validate_spawn_args(&a).is_err());
    }

    /// 批 P1-3 分层判定：只读名单全命中 = ReadOnly（自动放行）；任一宽条目
    /// （group:/server:/写类裸名）= Wide（走审批卡，无人值守拒）。
    #[test]
    fn classify_scope_tier_splits_readonly_and_wide() {
        let ro: Vec<String> = vec!["read_file".into(), "search_kb".into()];
        assert_eq!(classify_scope_tier(&ro), ScopeTier::ReadOnly);
        // 缺省（空清单）由调用方展开为只读名单——空本身视为 ReadOnly
        assert_eq!(classify_scope_tier(&[]), ScopeTier::ReadOnly);

        for bad in ["group:files", "server:srv-1", "write_file", "run_command"] {
            let mixed: Vec<String> = vec!["read_file".into(), bad.into()];
            assert_eq!(
                classify_scope_tier(&mixed),
                ScopeTier::Wide,
                "宽条目「{bad}」应判 Wide"
            );
        }
        // 只读条目已不进参数层校验（宽面走审批，validate 只管空值）
        assert!(validate_spawn_args(&args("p", Some(vec!["write_file"]))).is_ok());
    }

    /// 无人值守文案映射：四腿全覆盖、家族语义到位（完整三段式在调用点）。
    #[test]
    fn unattended_reason_text_covers_all_legs() {
        use crate::harness::mcp::ask_user_tool::UnattendedReason::*;
        assert!(unattended_reason_text(NoTurn).contains("回合"));
        assert!(unattended_reason_text(NonChatKind).contains("会话类型"));
        assert!(unattended_reason_text(ScheduledCarrier).contains("定时任务"));
        assert!(unattended_reason_text(IncomingConsumed).contains("跨会话来件"));
    }

    /// 治理①：worker 工具面 ⊆ 创建者（scope_allows 组展开——creator 配了
    /// group:files 时请求 read_file 裸名也覆盖；creator 全开时任何请求都过）。
    #[test]
    fn creator_scope_gate_semantics() {
        let open: Vec<String> = vec![];
        assert!(creator_scope_violations(&open, &["read_file".into()]).is_empty());

        let files_group: Vec<String> = vec!["group:files".into()];
        assert!(creator_scope_violations(&files_group, &["read_file".into()]).is_empty());
        assert_eq!(
            creator_scope_violations(&files_group, &["search_kb".into()]),
            vec!["search_kb".to_string()],
            "组外工具被拒"
        );

        let narrow: Vec<String> = vec!["read_file".into(), "search_files".into()];
        assert_eq!(
            creator_scope_violations(&narrow, &["read_file".into(), "web_fetch".into()]),
            vec!["web_fetch".to_string()],
            "裸名精确匹配"
        );
    }

    #[test]
    fn worker_name_is_readable_and_bounded() {
        let name = worker_display_name("统计项目目录里的文档数量与类型", "ab12cd34-0000");
        assert!(name.starts_with("临时·"), "{name}");
        assert!(name.ends_with("·ab12"), "{name}");
        // 超长 purpose 截断到有界摘要
        let long = "x".repeat(200);
        let name = worker_display_name(&long, "ffff0000-0000");
        assert!(name.chars().count() < 40, "{name}");
    }
}
