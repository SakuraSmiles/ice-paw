//! Chat Tauri Commands 入口 — 仅编排，不含业务逻辑。
//!
//! - `send_message`：入参校验 → 取 agent/api_key → 拼装上下文 → 写库占位 → spawn stream_loop
//! - `stop_generation`：触发 ChatState 上的 CancellationToken
//!
//! 业务分布：protocol → infra::protocol | 附件物化 → harness::attachments
//!           上下文 → context::pipeline | 调度 → harness::loop_engine
//!           错误 → harness::error_mapping | 收尾 → harness::cleanup

use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::commands::agent_cmd::AgentCmd;
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::attachments::{build_modality_hint, materialize_file_blocks};
use crate::harness::chat_state::ChatState;
use crate::harness::mcp::{McpRegistry, McpServerManager};
use crate::harness::provider;
use crate::harness::session_runner;
use crate::infra::file_validation::validate_files;
use crate::infra::protocol::{
    strip_empty_image_blocks, validate_images, ConfigProposalResponse, ContentBlock,
    ProposalDecision, SendMessageInput, ToolAuthResponse,
};

/// 发送消息 — 触发 LLM 流式生成。
///
/// REQ-XC-010: 依赖 `Arc<dyn AgentCmd>` trait object，而非具体
/// `SqlAgentCmd` 类型。这样可以在测试中注入 `MockAgentCmd`。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_message(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    chat_state: State<'_, ChatState>,
    agent_cmd: State<'_, Arc<dyn AgentCmd>>,
    input: SendMessageInput,
    auth_registry: State<'_, crate::harness::tool_executor::ToolAuthRegistry>,
    global_registry: State<'_, Arc<McpRegistry>>,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    route_registry: State<'_, crate::harness::read_route::ReadRouteRegistry>,
) -> AppResult<()> {
    tracing::info!(target: "ice_paw.chat", "send_message 被调用: conv={} model={:?} tools={}",
        input.conversation_id, input.model, input.tools_enabled);
    // --- 1. 入参校验：content_blocks 优先，回退到 legacy content ---
    let final_blocks = {
        let blocks = input.content_blocks.clone().filter(|v| !v.is_empty());
        let legacy = input.content.as_ref().and_then(|s| {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_owned())
            }
        });
        match (blocks, legacy) {
            (Some(b), _) => b,
            (None, Some(t)) => vec![ContentBlock::text(t)],
            // 纯附件无文本：允许（materialize 会填充 Attachment + 提取正文）
            (None, None) if input.files.as_ref().map(|v| !v.is_empty()).unwrap_or(false) => {
                Vec::new()
            }
            (None, None) => {
                return Err(AppError::Validation(
                    "content 或 content_blocks 至少提供一个".into(),
                ))
            }
        }
    };
    validate_images(&final_blocks)?;
    // 0 字节图片能通过 validate_images（尺寸 0 ≤ 上限），但发给 LLM 会被 400 拒绝。
    // 软剥离：空图片块移除 + 注入诚实提示，绝不阻塞整条消息（与文档附件软失败同策略）。
    let final_blocks = strip_empty_image_blocks(final_blocks);

    let conv_id = input.conversation_id.clone();
    let tools_enabled = input.tools_enabled;
    // P0-3: 会话级 model override（None = 使用 Agent 默认 model）
    let model_override = input.model.clone();
    // 用户原始文本 query（仅 Text 块，**不含**附件提取文本）——用于标题/钩子/检索，
    // 避免整份文档灌进 query 噪声。附件内容在下方 materialize 后进 final_blocks 给 LLM。
    // （runner 内部以 content_text 同时作消息正文与检索 query。）
    let content_text = ContentBlock::join_text(&final_blocks);

    // --- 2. 取会话 + agent + api_key → 创建 provider ---
    // （先于写消息：conv/agent/provider 任一失败都应**不落任何 DB 行**，避免孤儿用户消息。）
    let conv = repo::conversation::get_by_id(pool.inner(), &conv_id).await?;
    let agent_with_creds = agent_cmd.get_with_credentials(&conv.agent_id).await?;
    let agent = agent_with_creds.agent;
    let api_key = agent_with_creds.api_key;
    let base_url = agent_with_creds.base_url.as_deref();
    // 钩子配置（来自 agent.yaml `hooks`；纯文件，不进 DB）
    let hooks = agent_with_creds.hooks;

    let llm_provider = provider::create_provider(
        &agent.provider,
        &agent.model,
        base_url,
        agent.cache_prompt != 0,
    )?;

    // Phase 3 + Phase A：office/pdf 附件 → 后端提取为 Text 块追加到 final_blocks 末尾。
    // 文件是输入模态（LLM 读不了 base64 二进制），物化为 Text：不进 ContentBlock 枚举、
    // base64 不落盘 DB、provider 适配层零改动。提取失败 → 整条消息拒绝（fail-fast，先于 Pipeline）。
    //
    // materialize 是**纯函数**（只解码+提取+拼 blocks+收集分页块数据），**不写 DB**。
    // 用户消息行与分页块行都推迟到 Pipeline 成功后一起写——这样 conv/agent/provider/
    // Pipeline/materialize 任一失败都不落任何 DB 行，**无孤儿用户消息**（分页块外键父
    // = 用户消息，二者同批写入，消息行先于块行）。
    //
    // user_msg_id 预生成（仅 UUID 字符串，无 IO）：materialize 需把它嵌进大文件首页的
    // read_attachment_page 工具提示里；真正落库时复用同一 id。
    let user_msg_id = Uuid::new_v4().to_string();
    let (final_blocks, attach_db_inputs, attach_file_inputs) =
        match input.files.as_ref().filter(|v| !v.is_empty()) {
            Some(files) => {
                validate_files(files)?;
                materialize_file_blocks(&user_msg_id, final_blocks, files)?
            }
            None => (final_blocks, Vec::new(), Vec::new()),
        };

    // @ 引用展开（Reference 块 → 快照 Text 块）：在 persist_blocks 落库快照
    // clone **之前**，落库消息 = 引用卡 + 展开快照（回放保真，session_events
    // 零特例）。失效降级为占位 Text，绝不阻塞整条消息。
    let mut final_blocks = crate::harness::references::materialize_reference_blocks(
        pool.inner(),
        &conv_id,
        final_blocks,
    )
    .await;

    // 事1 + 事2：视觉模态元信息注入。仅当 agent「有效支持视觉」时插入"你已直接收到 N 张图、
    // 无需调图片工具"的元提示——纠正视觉 agent「看到了却说没看到」的认知偏差。
    // 非视觉 agent 不插此提示（其图片由 ModalCapabilityStage 走代读/诚实剥离，提示由该 Stage 负责）。
    // 持久化用的原始 blocks：用户真实发送内容（含原图 + 附件卡片/正文）。
    // ⚠️ 视觉适配（ModalCapabilityStage 对非视觉 agent 把 Image 代读/剥离）只改发给 LLM 的
    // 视图（pipeline_ctx.final_blocks → user_blocks），**绝不能污染落库的用户消息**——否则
    // 非视觉 agent 的历史回看会丢图（用户气泡从 content_blocks 取 Image 块渲染；bug 表现：
    // 发送时看得到图、agent 回答后从 DB 刷新就消失）。故在此（materialize 后、build_modality_hint
    // 元提示前）clone 一份原始 blocks 专供落库，与发给 LLM 的适配视图彻底解耦。
    let persist_blocks: Vec<ContentBlock> = final_blocks.clone();

    let eff_vision = crate::harness::provider::effective_supports_vision(
        agent.supports_vision,
        &agent.provider,
        &agent.model,
    );
    if eff_vision {
        if let Some(hint) = build_modality_hint(&final_blocks) {
            final_blocks.insert(0, hint);
        }
    }

    // --- 3. 注册 cancel token（必须先于 Pipeline，让 MemoryStage 摘要也能响应取消）---
    let cancel_token = chat_state.start(&conv_id).inspect_err(|_| {
        tracing::warn!(target: "ice_paw.chat", "send_message: 会话 {} 已有在途生成任务", conv_id);
    })?;

    // RAII 守卫：到此 token 已注册，若下方任意 `?` 早返回（Pipeline/DB 写/emit/spawn 前失败），
    // 守卫 drop 时自动 unregister，避免 conv_id 永久残留 ChatState → is_streaming 永不翻转 →
    // 后续 send_message 全部命中「已有在途任务」、会话卡死需重启 App。
    // spawn 成功后在函数末尾 disarm，把注销责任移交给 stream_loop 的 finalize_*。
    let conv_id_guard = conv_id.clone();
    let cancel_guard = scopeguard::guard((), |_| chat_state.unregister(&conv_id_guard));

    // --- 4. 委派 session_runner：历史解析 → Pipeline → 落库 → spawn 流式循环 ---
    // MA-1 抽取：send_message 保留输入预处理（校验/附件物化/视觉提示），「一次完整
    // agent 回合」的编排内核复用 session_runner::run_agent_turn（agent 委派走同一
    // 内核——专家用自己的 provider/key 跑完整循环）。用户路径 fire-and-forget：
    // 完成信号 Receiver 直接 drop，前端靠流式事件感知进度。
    let _turn_done = session_runner::run_agent_turn(
        &session_runner::TurnEnv {
            emitter: crate::harness::r#loop::emitter::tauri_emitter(app.clone(), conv_id.clone()),
            tool_app: Some(app.clone()),
            pool: pool.inner().clone(),
            route_registry: route_registry.inner(),
            global_registry: Arc::clone(global_registry.inner()),
            mcp_manager: Arc::clone(mcp_manager.inner()),
            auth_registry: auth_registry.inner().clone(),
        },
        session_runner::AgentTurnInput {
            conv,
            agent,
            hooks,
            provider: llm_provider,
            api_key,
            user_msg_id,
            content_text,
            llm_blocks: final_blocks,
            persist_blocks,
            attach_db_inputs,
            attach_file_inputs,
            emit_user_blocks: input.files.as_ref().map(|v| !v.is_empty()).unwrap_or(false),
            tools_enabled,
            model_override,
            cancel_token,
        },
    )
    .await?;
    // spawn 成功：注销责任已移交 stream_loop（其 finalize_success/finalize_cancel → cleanup →
    // unregister），解除守卫，避免此处 Ok 返回时误注销导致 is_streaming 提前翻转。
    scopeguard::ScopeGuard::into_inner(cancel_guard);
    Ok(())
}

/// 停止指定会话的流式生成。
#[tauri::command]
pub async fn stop_generation(
    chat_state: State<'_, ChatState>,
    conversation_id: String,
) -> AppResult<()> {
    if !chat_state.stop(&conversation_id) {
        tracing::warn!(target: "ice_paw.chat",
            "stop_generation: 会话 {} 无在途生成任务", conversation_id);
    }
    Ok(())
}

// ============================================================================
// 前端 → 后端 响应通道（invoke 命令）
//
// 设计说明（重要）：
// 原先审批/授权响应用 chat:config-proposal-response / chat:tool-auth-response
// 事件（前端 emit → 后端 app.listen）。但 Tauri v2 中前端 emit 是 webview 作用域、
// 后端 listen 是全局监听器——作用域不匹配导致事件被静默丢弃，双通道从未工作，
// 表现为「点批准后 agent 仍等满 120s 超时」。
// 改为 invoke 命令（Tauri 推荐的前端→后端请求-响应 IPC），可靠送达。
// ============================================================================

/// 前端审批结果 → 唤醒后端 propose_config_change 的 oneshot 等待者。
///
/// 扁平入参（与前端 `ConfigProposalResponse` 类型一致：decision 为字符串），
/// 命令内转 [`ProposalDecision`] enum 再交由 registry。
#[tauri::command]
pub async fn respond_config_proposal(
    proposal_registry: State<'_, crate::harness::proposal_registry::ProposalRegistry>,
    input: ConfigProposalResponseInput,
) -> AppResult<()> {
    let decision = match input.decision.as_str() {
        "approved" => ProposalDecision::Approved,
        "modified" => ProposalDecision::Modified {
            changes: input.changes.unwrap_or_default(),
        },
        "rejected" => ProposalDecision::Rejected {
            reason: input.reason,
        },
        other => {
            return Err(AppError::Validation(format!(
                "未知提案 decision: '{other}'（需 approved/modified/rejected）"
            )))
        }
    };
    let handled = proposal_registry
        .respond(ConfigProposalResponse {
            request_id: input.request_id,
            decision,
        })
        .await;
    if !handled {
        tracing::warn!(
            target: "ice_paw.mgmt",
            "提案 respond：未找到匹配的 request_id（可能已超时/取消）"
        );
    }
    Ok(())
}

/// 前端工具授权结果 → 唤醒后端 wait_for_auth_response 的 oneshot 等待者。
#[tauri::command]
pub async fn respond_tool_auth(
    auth_registry: State<'_, crate::harness::tool_executor::ToolAuthRegistry>,
    input: ToolAuthResponse,
) -> AppResult<()> {
    let handled = auth_registry.respond(input).await;
    if !handled {
        tracing::warn!(
            target: "ice_paw.tool_auth",
            "授权 respond：未找到匹配的 request_id（可能已超时/取消）"
        );
    }
    Ok(())
}

/// 前端审批响应的扁平入参（decision 为字符串，匹配前端类型）。
/// 命令内转为 [`ProposalDecision`] enum 再交由 registry。
#[derive(serde::Deserialize)]
pub struct ConfigProposalResponseInput {
    pub request_id: String,
    /// `"approved"` | `"modified"` | `"rejected"`
    pub decision: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub changes: Option<HashMap<String, String>>,
}
