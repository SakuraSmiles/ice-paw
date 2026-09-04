//! L2 Tool Executor — 工具执行编排（W3.3 + A2-3 用户授权流程）
//!
//! 职责：对一批已完成的工具调用执行：
//! 1. 解析工具参数、提取待访问路径
//! 2. 通过 `check_authorization_with_session` 判断授权
//! 3. 直接执行 / emit `chat:tool-auth-request` 等待前端弹窗 / 拒绝
//! 4. 收集 tool_use + tool_result 的 ContentBlock，emit `chat:tool-result`
//!
//! **A2-3 变更**：
//! - `execute_tool_round` 现在接收 `PathAuthSession`（会话级分层授权记忆，#11：
//!   精确路径/目录/工具三档 grant）和 `whitelist_config`（白名单配置）。
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

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::db::models::{HookConfig, HookPoint};
use crate::db::repo;
use crate::error::AppError;
use crate::harness::authority::{
    check_authorization_with_session, AuthorizationDecision, PathAuthSession, PathWhitelistConfig,
};
use crate::harness::hooks::{has_actions, run_hooks};
use crate::harness::mcp::client::ToolOutput;
use crate::harness::mcp::screen::channel as screen_channel;
use crate::harness::mcp::{AuthorizationLevel, McpRegistry, ToolContext};
pub use crate::harness::oneshot_registry::ToolAuthRegistry;
use crate::infra::protocol::{
    AuthScope, ChatToolResultPayload, ContentBlock, PendingRequestCancelPayload,
    ToolAuthRequestPayload, ToolAuthResponse,
};
use base64::Engine as _;

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
/// 调用工具并把 `AppResult<ToolOutput>` 统一转成 `Result<ToolOutput, String>`
///（execute_tool_round 内部「允许执行」与「用户授权后执行」两路的共用入口，含 panic 兜底）。
async fn invoke_tool(
    registry: &McpRegistry,
    name: &str,
    args: &str,
    ctx: &ToolContext,
) -> Result<ToolOutput, String> {
    // 批次④ 步骤 2（§4.2 purpose + B7 写操作避让）：通道 Active 且本会话附着时——
    // ① 家族工具执行前刷新「正在做什么」（当前回合用户指令摘要，HUD 使用者块）；
    // ② 写件执行期间登记 write_in_flight → HUD 收缩角部微条让路。包边覆盖执行
    // 全程（含 gate 排队/human park——排队期间收缩同样是给用户让路）；RAII
    // guard 取消安全（abort 丢弃 future 时 Drop 仍运行，计数不泄漏）。
    let screen_attached = screen_channel::SCREEN_TOOLS.contains(&name)
        && screen_channel::global().is_active_and_attached(&ctx.conv_id);
    if screen_attached {
        screen_channel::refresh_purpose(ctx).await;
    }
    let _write_bracket = (screen_attached && screen_channel::WRITE_TOOLS.contains(&name))
        .then(screen_channel::WriteBracket::enter);
    match registry.dispatch_catch_panic(name, args, ctx).await {
        Ok(out) => Ok(out),
        Err(e) => Err(e.to_string()),
    }
}

// 11 个参数均为单次工具轮次的独立输入（8/11 来自 loop context，3 个本轮局部），
// 且仅 1 个调用点（loop_engine::stream_loop_inner）。彻底收敛需把 loop context 整体
// 传入，属跨层耦合的架构决策，不在 clippy 清理范围；此处显式 allow + 注明。
// `ev` 为 session-events 的 turn 上下文（tool_execution 事件归属用，Phase 0）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_tool_round(
    emitter: &dyn crate::harness::r#loop::emitter::LoopEmitter,
    tool_app: Option<&AppHandle>,
    registry: &McpRegistry,
    auth_registry: &ToolAuthRegistry,
    session: &PathAuthSession,
    whitelist: &PathWhitelistConfig,
    completed_calls: &[(String, String, String)],
    tool_ctx: &ToolContext,
    asst_msg_id: &str,
    ev: &crate::harness::event_log::EventCtx,
    cancel: &crate::harness::chat_state::CancellationToken,
    hooks: &HookConfig,
) -> crate::error::AppResult<Vec<ContentBlock>> {
    let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

    // 扩展 tool_ctx：注入 app_handle + proposal_registry（propose_config_change 等工具需要）
    // + turn_id（update_plan 等在工具执行点落 session-event 的工具需要——事件归组）
    let mut enriched_ctx = tool_ctx.clone();
    // S6：app_handle / proposal_registry 仅在真 Tauri 运行时（tool_app=Some）注入；
    // 测试（None）时依赖它们的工具（proposal / delegate）走既有「需要 App 上下文」降级。
    enriched_ctx.app_handle = tool_app.cloned();
    enriched_ctx.cancel = Some(cancel.clone());
    enriched_ctx.turn_id = Some(ev.turn_id.clone());
    enriched_ctx.proposal_registry = tool_app.and_then(|app| {
        app.try_state::<crate::harness::proposal_registry::ProposalRegistry>()
            .map(
                |s: tauri::State<'_, crate::harness::proposal_registry::ProposalRegistry>| {
                    s.inner().clone()
                },
            )
    });
    let tool_ctx = &enriched_ctx;

    // agent workspace 内的文件免授权（workspace 是 agent 的信任领地，
    // agent 读写自己 workspace 内的文件不需弹窗确认）。workspace 由 loop_engine
    // 解析后放进 tool_ctx，这里直接复用，避免重复查库。
    let workspace = tool_ctx.workspace.as_ref().map(PathBuf::from);

    for (tc_id, tc_name, tc_args) in completed_calls {
        let tool_start = std::time::Instant::now();
        let started_at = now_sql();
        // 1. 解析授权级别 + 路径（+ 路径字段名——「允许此目录」档判定 dir 本身 vs 父目录用）
        let (level, file_path, path_key) = inspect_tool_for_auth(registry, tc_name, tc_args).await;

        let decision = if matches!(level, AuthorizationLevel::PathWhitelist)
            && path_within_workspace(&file_path, &workspace)
        {
            // workspace 内 + PathWhitelist 级别（如 read_file）→ 免授权放行
            AuthorizationDecision::Allow
        } else {
            check_authorization_with_session(
                level, &file_path, whitelist, tc_name, tc_args, session,
            )
            .await
        };

        // Confirm 级工具若声明了 auth_reason（如截屏「画面将发送给模型服务商」），
        // 替换通用 reason 展示在审批卡上——用户点「允许」前必须知道数据去向。
        let mut decision = decision;
        if let AuthorizationDecision::Confirm { reason, tool_name, .. } = &mut decision {
            if let Some(custom) = registry
                .get(tool_name)
                .await
                .and_then(|t| t.auth_reason())
            {
                *reason = custom;
            }
        }

        // 批次④ 步骤 1：屏幕通道授权短路（§4.11）——通道 Active 且本会话已附着
        // 时，computer-use 家族工具的 Confirm 上收为 Allow（用户开启通道/批准
        // 加入的动作即知情同意）。只吃 Confirm，不碰 Deny（显式拒绝不被越权）。
        let decision = screen_channel::short_circuit(decision, tc_name, &tool_ctx.conv_id);

        // 2. 根据决策执行
        let final_result: Result<ToolOutput, String> = match decision {
            AuthorizationDecision::Allow => invoke_tool(registry, tc_name, tc_args, tool_ctx).await,
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
                let emit_result = serde_json::to_value(&payload)
                    .map_err(|e| AppError::Internal(format!("payload 序列化失败: {e}")))
                    .map(|v| {
                        emitter.emit("chat:tool-auth-request", v);
                    });
                if let Err(e) = emit_result {
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
                    // 2c. 等待响应（带取消支持 + 2 分钟超时防止永久挂起——
                    //     用户不在时快速超时释放会话，见 wait_for_auth_response）
                    let outcome = wait_for_auth_response(
                        rx,
                        cancel,
                        &request_id,
                        auth_registry,
                        emitter,
                        &tool_ctx.conv_id,
                    )
                    .await;
                    match outcome {
                        Some(resp) if resp.allowed => {
                            // 用户允许：按所选范围入会话级分层授权（#11），然后执行
                            match resp.scope {
                                AuthScope::Once => {
                                    session.mark_authorized(&file_path).await;
                                }
                                AuthScope::ThisTool => {
                                    session.mark_tool_authorized(&tool_name).await;
                                }
                                AuthScope::ThisDir => match dir_grant_target(&path_key, &file_path)
                                {
                                    Some(dir) => session.mark_dir_authorized(&dir).await,
                                    // 无路径（Confirm 级工具）→ 退化为工具档（协议注释已声明）
                                    None => session.mark_tool_authorized(&tool_name).await,
                                },
                            }
                            tracing::info!(
                                target: "ice_paw.tool_auth",
                                "用户允许工具调用: tool={} path={} scope={:?}",
                                tool_name,
                                file_path,
                                resp.scope,
                            );
                            // 批次④ 步骤 1：屏幕通道「首用加入即附着」（§4.1 B5）——
                            // 通道 Active + 本会话未附着 + 家族工具获批 → 附着，
                            // 本会话后续免卡。附着只发生在用户刚点过允许之后
                            //（每条附着路径都以一次用户点击为前提，无静默附着）；
                            // delegate 子会话同路径逐个点头。
                            if screen_channel::SCREEN_TOOLS.contains(&tc_name.as_str()) {
                                let ch = screen_channel::global();
                                if ch.is_active() && !ch.is_attached(&tool_ctx.conv_id) {
                                    let info = screen_channel::attach_info_from_db(
                                        &tool_ctx.pool,
                                        &tool_ctx.agent_id,
                                        &tool_ctx.conv_id,
                                    )
                                    .await;
                                    if ch.attach_if_active(&tool_ctx.conv_id, info.clone()) {
                                        tracing::info!(
                                            target: "ice_paw.screen_channel",
                                            conv = %tool_ctx.conv_id, agent = %info.agent_name,
                                            "会话首用批准后加入屏幕通道"
                                        );
                                        if let Some(app) = tool_app {
                                            screen_channel::emit_state(app);
                                        }
                                    }
                                }
                            }
                            invoke_tool(registry, tc_name, tc_args, tool_ctx).await
                        }
                        Some(_) => {
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
        let finished_at = now_sql();

        // 审计：写入 tool_calls 表（失败仅 warn，不影响主流程）。
        // 副作用——表一旦有数据，list_recent_tool_names 不再返回空 →
        // 工具打分的「历史权重」维度从此真正生效（见 scoring.rs）。
        let (audit_is_error, audit_result) = match &final_result {
            Ok(out) => (false, out.text.as_str()),
            Err(e) => (true, e.as_str()),
        };
        let tool_call_id = Uuid::new_v4().to_string();
        if let Err(e) = repo::tool_call::create(
            &tool_ctx.pool,
            &tool_call_id,
            asst_msg_id,
            tc_name,
            tc_args,
            Some(audit_result),
            audit_is_error,
            duration_ms,
            &started_at,
            &finished_at,
        )
        .await
        {
            tracing::warn!(
                target: "ice_paw.tool_audit",
                "写入 tool_calls 审计失败（忽略）: tool={} err={}",
                tc_name,
                e
            );
        }
        // session-events（Phase 0）：tool_execution 事件镜像同一审计事实
        //（同 id / 同截断策略），warn-only inline await，不阻断工具循环。
        crate::harness::event_log::log_tool_execution(
            &tool_ctx.pool,
            ev,
            asst_msg_id,
            &tool_call_id,
            Some(tc_id),
            tc_name,
            tc_args,
            Some(audit_result),
            audit_is_error,
            duration_ms,
        )
        .await;

        match final_result {
            Ok(out) => {
                crate::harness::r#loop::emitter::emit_ser(
                    emitter,
                    "chat:tool-result",
                    &ChatToolResultPayload {
                        conversation_id: tool_ctx.conv_id.clone(),
                        message_id: asst_msg_id.to_string(),
                        tool_use_id: tc_id.clone(),
                        content: out.text.clone(),
                        is_error: false,
                        duration_ms,
                    },
                );
                tool_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: tc_id.clone(),
                    content: out.text,
                    is_error: Some(false),
                });
                // 门②（事2）：工具回传 PNG → 按 agent「有效视觉能力」统一适配。
                // 有效视觉 → 编码 base64 Image 块注入（模型直读；Anthropic/GLM tool_result 原生
                //   支持 image block，OpenAI 适配层把 Image 拆为紧邻 role="user" image_url）。
                // 非视觉 → 走多级凭据代读成文本，追加到本 ToolResult 的 content（绝不向非视觉
                //   模型塞 Image → 400 / "看不到"）。当前仅 view_attachment_image 回传 PNG 且自身
                //   已按 effective_vision 路由，本守卫是其外层的防御纵深（防未来未路由的工具）。
                if let Some(png) = out.image_png {
                    let agent_opt = repo::agent::get_by_id(&tool_ctx.pool, &tool_ctx.agent_id)
                        .await
                        .map_err(|e| {
                            tracing::warn!(
                                target: "ice_paw.tool_image",
                                err = %e,
                                "取 agent 失败，按非视觉处理（代读/剥离）"
                            );
                            e
                        })
                        .ok();
                    let eff_vision = agent_opt
                        .as_ref()
                        .map(|a| {
                            crate::harness::provider::effective_supports_vision(
                                a.supports_vision,
                                &a.provider,
                                &a.model,
                            )
                        })
                        .unwrap_or(false);

                    if eff_vision {
                        let data = base64::engine::general_purpose::STANDARD.encode(&png);
                        tool_result_blocks.push(ContentBlock::Image {
                            data,
                            media_type: "image/png".to_string(),
                        });
                        tracing::info!(
                            target: "ice_paw.tool_image",
                            tool = %tc_name,
                            bytes = png.len(),
                            "工具回传图片，已注入 Image 块给视觉 agent"
                        );
                    } else {
                        // 非视觉：复用统一适配（两档制第二档——平台视觉配置链代读）。
                        let candidates = crate::harness::modal::gather_vision_candidates(&tool_ctx.pool)
                            .await;
                        let data = base64::engine::general_purpose::STANDARD.encode(&png);
                        let tmp = vec![ContentBlock::image(data, "image/png")];
                        let outcome = crate::harness::modal::adapt_blocks_for_vision(
                            &tmp,
                            false,
                            &candidates,
                            None,
                        )
                        .await;
                        // session-events：工具返图的投影期适配入日志（stage=tool_image，
                        // OCR 全文随 items 落库）。视觉直通分支不记——注入的 Image 块
                        // 已由 tool_result_message 事件镜像。
                        crate::harness::event_log::log_modal_adapted(
                            &tool_ctx.pool,
                            ev,
                            &crate::harness::event_log::ModalAdaptedPayload {
                                v: 1,
                                stage: "tool_image".into(),
                                mode: if outcome.ocr_replaced > 0 {
                                    "ocr_substitute".into()
                                } else {
                                    "strip_with_hint".into()
                                },
                                items: outcome.items.clone(),
                            },
                        )
                        .await;
                        // 适配产出的文本（代读文本或诚实提示）追加到本 ToolResult content。
                        let extra: String = outcome
                            .blocks
                            .iter()
                            .filter_map(|b| b.as_text().map(|s| s.to_string()))
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !extra.is_empty() {
                            if let Some(ContentBlock::ToolResult { content, .. }) =
                                tool_result_blocks.last_mut()
                            {
                                content.push_str("\n\n[工具回传图片，当前模型无视觉能力，");
                                content.push_str(if outcome.ocr_replaced > 0 {
                                    "经视觉凭据代读为文本]\n"
                                } else if outcome.drop_reason.is_some() {
                                    // 有凭据但调用失败（敏感拒/限流/key 错/网络）——具体原因在下方诚实提示
                                    "视觉凭据代读失败]\n"
                                } else {
                                    // 无凭据或 base64 损坏（未发起有效调用）
                                    "且无可用视觉凭据代读]\n"
                                });
                                content.push_str(&extra);
                            }
                        }
                        tracing::info!(
                            target: "ice_paw.tool_image",
                            tool = %tc_name,
                            ocr_replaced = outcome.ocr_replaced,
                            "工具回传图片，非视觉 agent 已代读/剥离（不再塞 Image 块）"
                        );
                    }
                }
            }
            Err(err_content) => {
                crate::harness::r#loop::emitter::emit_ser(
                    emitter,
                    "chat:tool-result",
                    &ChatToolResultPayload {
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
/// - `Some(resp)`  用户已决定（`resp.allowed` + `resp.scope` 供按档入账）
/// - `None`        被取消 / 超时
///
/// `cancel` 触发取消时，oneshot receiver 被丢弃，sender 端 send 失败被忽略。
///
/// 失效分支（取消/超时/通道关闭）会 emit `chat:tool-auth-request-cancel`，
/// 让前端按 `request_id` 清除对应授权弹窗，避免残留导致用户对已失效请求操作。
/// 等待授权响应（带取消支持 + 2 分钟超时）——tool_executor 通用授权与
/// delegate 委派授权（2026-09-03）共用，单一实现勿复制。
pub(crate) async fn wait_for_auth_response(
    rx: oneshot::Receiver<ToolAuthResponse>,
    cancel: &crate::harness::chat_state::CancellationToken,
    request_id: &str,
    auth_registry: &ToolAuthRegistry,
    emitter: &dyn crate::harness::r#loop::emitter::LoopEmitter,
    conv_id: &str,
) -> Option<ToolAuthResponse> {
    const TIMEOUT: Duration = Duration::from_secs(120); // 2 分钟——用户不在时快速超时释放会话

    tokio::select! {
        biased;
        // 用户主动取消（对话被停止）
        _ = wait_for_cancel(cancel) => {
            // 清理 sender，避免后续响应泄露
            let _ = auth_registry.take(request_id).await;
            emit_auth_cancel(emitter, request_id, conv_id, "abort");
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
            emit_auth_cancel(emitter, request_id, conv_id, "timeout");
            None
        }
        // 收到前端响应
        msg = rx => {
            match msg {
                Ok(resp) => Some(resp),
                Err(_) => {
                    // sender 被 drop → 视为取消
                    emit_auth_cancel(emitter, request_id, conv_id, "cancelled");
                    None
                }
            }
        }
    }
}

/// 通知前端清除对应授权弹窗（授权请求失效时调用）。
/// 与 `proposal_tool::emit_proposal_cancel` 对称。emit 失败仅 warn。
fn emit_auth_cancel(
    emitter: &dyn crate::harness::r#loop::emitter::LoopEmitter,
    request_id: &str,
    conv_id: &str,
    reason: &str,
) {
    crate::harness::r#loop::emitter::emit_ser(
        emitter,
        "chat:tool-auth-request-cancel",
        &PendingRequestCancelPayload {
            request_id: request_id.into(),
            conversation_id: conv_id.into(),
            reason: reason.into(),
        },
    );
}

/// 等待 CancellationToken 触发（包装成 future）
pub(crate) async fn wait_for_cancel(token: &crate::harness::chat_state::CancellationToken) {
    // 简单的轮询（100ms 粒度）。CancellationToken 当前未提供异步 wait，
    // 这里用低开销循环等价实现。后续如果改成 watch channel 可以直接 select。
    loop {
        if token.is_cancelled() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// UTC 墙钟，格式与 SQLite `datetime('now')` 一致（`YYYY-MM-DD HH:MM:SS`）。
/// 供 tool_calls 审计记录起止时间，便于 SQL 直接做时间差/排序。
fn now_sql() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 从工具实例提取 `AuthorizationLevel` + 待访问路径 + 路径字段名。
///
/// - `level` 直接调 `tool.authorization_level()`
/// - `file_path` 从参数 JSON 中提取（见 [`extract_path_from_args`]），
///   找不到则用空字符串（`Always` 工具不需要路径）
/// - `path_key` 命中的字段名（"path"/"dir"/…），「允许此目录」档判定
///   目录参数本身 vs 文件参数的父目录用
async fn inspect_tool_for_auth(
    registry: &McpRegistry,
    tool_name: &str,
    args: &str,
) -> (AuthorizationLevel, String, String) {
    let default_level = AuthorizationLevel::Always;

    let Some(tool) = registry.get(tool_name).await else {
        return (default_level, String::new(), String::new());
    };

    let level = tool.authorization_level();
    let (path_key, path) =
        extract_path_from_args(args).unwrap_or_else(|| (String::new(), String::new()));
    (level, path, path_key)
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
        app_handle: None,
        proposal_registry: None,
        // turn 归组键由 execute_tool_round 富化注入（工具轮内有 ev 上下文）；
        // 构造点（hooks 等 turn 语境之外）恒 None
        turn_id: None,
        cancel: None,
    }
}

/// 判断 file_path 是否在 agent workspace 内。
///
/// 授权免弹窗的单一判定，经 `infra::path_norm::path_within`（词法归一 +
/// 组件级比较 + Windows 大小写/`..`/verbatim 前缀安全）：
/// - 存在路径先 canonicalize（解析符号链接，防链接逃逸出 workspace）
/// - 不存在（新建文件/未建目录）回退词法归一——旧实现此处拿原始串与
///   canonicalize 的 `\\?\` verbatim 形直接 starts_with，Windows 恒 false，
///   workspace 内所有不存在路径 100% 误弹审批（0.3.5 生产反馈）
fn path_within_workspace(file_path: &str, workspace: &Option<PathBuf>) -> bool {
    let Some(ws) = workspace else {
        return false;
    };
    let target = match Path::new(file_path).canonicalize() {
        Ok(cfp) => cfp,
        Err(_) => crate::infra::path_norm::lexical_normalize(Path::new(file_path)),
    };
    crate::infra::path_norm::path_within(&target, ws)
}

/// 从工具参数 JSON 提取路径字段（`path` / `file_path` / `dir` / `source` / `destination`），
/// 返回 `(字段名, 路径)`。
///
/// `source`/`destination` 供 `move_file`/`copy_file`：tool_executor 只提取单个路径做白名单
/// 校验，故以 source 为代表路径（destination 由工具内 `reject_sensitive` 兜底）。
/// 多路径工具（如 `read_multiple_files` 的 paths 数组）无法提取，会回退到弹窗确认。
fn extract_path_from_args(args: &str) -> Option<(String, String)> {
    let parsed: Value = serde_json::from_str(args).ok()?;
    for key in ["path", "file_path", "dir", "source", "destination"] {
        if let Some(s) = parsed.get(key).and_then(|v| v.as_str()) {
            return Some((key.to_string(), s.to_string()));
        }
    }
    None
}

/// 「允许此目录」档的入账目标（#11）。
///
/// - 参数字段是 `dir` → 路径本身就是目录，原样入账（list_directory 等）
/// - 其它字段（path/file_path/source/destination）→ 入账父目录
///   （对 `C:\ws\a.txt` 批「此目录」意图是「a.txt 所在的 ws」，而非精确到文件）
/// - 无路径（Confirm 级工具）→ `None`，调用方退化为工具档
fn dir_grant_target(path_key: &str, file_path: &str) -> Option<String> {
    if file_path.is_empty() {
        return None;
    }
    if path_key == "dir" {
        return Some(file_path.to_string());
    }
    Path::new(file_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|p| !p.is_empty())
        .or_else(|| Some(file_path.to_string()))
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
            Some(("path".into(), "/etc/passwd".into()))
        );
    }

    #[test]
    fn extract_path_from_args_with_file_path() {
        assert_eq!(
            extract_path_from_args(r#"{"file_path":"/home/x.txt"}"#),
            Some(("file_path".into(), "/home/x.txt".into()))
        );
    }

    #[test]
    fn extract_path_from_args_with_dir() {
        assert_eq!(
            extract_path_from_args(r#"{"dir":"/var"}"#),
            Some(("dir".into(), "/var".into()))
        );
    }

    #[test]
    fn extract_path_from_args_with_source() {
        // move_file 用 source/destination；source 排在 destination 前，故提取 source
        // 作为代表路径用于白名单授权
        assert_eq!(
            extract_path_from_args(r#"{"source":"/a/b.txt","destination":"/c/b.txt"}"#),
            Some(("source".into(), "/a/b.txt".into()))
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

    // ===== #11「允许此目录」档入账目标推导 =====

    #[test]
    fn dir_grant_target_dir_arg_is_itself() {
        // dir 参数：路径就是目录，原样入账（不升到父级——那会连带兄弟目录）
        assert_eq!(
            dir_grant_target("dir", "/ws/project"),
            Some("/ws/project".into())
        );
    }

    #[test]
    fn dir_grant_target_file_arg_takes_parent() {
        assert_eq!(
            dir_grant_target("path", "/ws/project/a.txt"),
            Some("/ws/project".into())
        );
        assert_eq!(
            dir_grant_target("file_path", "/ws/project/sub/b.txt"),
            Some("/ws/project/sub".into())
        );
    }

    #[test]
    fn dir_grant_target_empty_path_falls_to_tool_grant() {
        // 无路径（Confirm 级工具）→ None → 调用方退化为工具档
        assert_eq!(dir_grant_target("", ""), None);
        assert_eq!(dir_grant_target("path", ""), None);
    }

    #[test]
    fn dir_grant_target_root_level_file_falls_to_itself() {
        // 无父可取的极端形态（词法 parent 为空）→ 文件本身入账，
        // 由 auth_cache_key 归一兜底
        assert_eq!(
            dir_grant_target("path", "bare.txt"),
            Some("bare.txt".into())
        );
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
                scope: AuthScope::ThisDir,
                delegation_grant: None,
            })
            .await;
        });

        let resp = rx.await.unwrap();
        assert!(resp.allowed);
        assert_eq!(resp.scope, AuthScope::ThisDir);
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
                scope: AuthScope::Once,
                delegation_grant: None,
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
        assert!(!path_within_workspace(
            "C:/Windows/System32/drivers/etc/hosts",
            &ws
        ));
    }

    #[test]
    fn path_within_workspace_none_workspace() {
        assert!(!path_within_workspace("/any/path", &None));
    }

    #[test]
    fn path_within_workspace_nonexistent_inside() {
        // 0.3.5 生产 bug 回归：workspace 走 canonicalize（Windows 得 \\?\ 形），
        // 目标是 workspace 内尚不存在的路径（新建文件/未建目录）——
        // 旧实现恒 false → 误弹审批
        let ws_path = std::env::temp_dir()
            .canonicalize()
            .expect("temp dir 应存在");
        let ws = Some(ws_path.clone());
        let inside_new = ws_path.join("no_such_dir_p1r12").join("new_file.txt");
        assert!(path_within_workspace(inside_new.to_str().unwrap(), &ws));
    }

    #[test]
    fn path_within_workspace_dotdot_traversal_rejected() {
        // 安全回归：字符串前缀曾被 `..` 穿越骗过（/tmp/../../x starts_with /tmp）
        let ws_path = std::env::temp_dir()
            .canonicalize()
            .expect("temp dir 应存在");
        let ws = Some(ws_path.clone());
        let escape = ws_path.join("..").join("..").join("no_such_top_p1r12");
        assert!(!path_within_workspace(escape.to_str().unwrap(), &ws));
    }
}
