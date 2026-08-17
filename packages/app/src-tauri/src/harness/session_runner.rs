//! Session Runner — 单次 agent 回合的可复用编排内核（多 agent 协作 MA-1）
//!
//! 从 `commands/chat_cmd.rs::send_message` 的编排主体抽出的「一次完整 agent 回合」：
//! 历史解析（read_route）→ Pipeline 拼装 → 落库（用户消息 + 事件 + assistant 占位）
//! → 预算/工具组装 → ConversationStart 钩子 → spawn 流式循环。
//!
//! 两个调用方：
//! - **用户发送**（`send_message` 命令）：fire-and-forget——拿到完成信号 Receiver
//!   后直接 drop，前端靠流式事件感知进度；
//! - **agent 委派**（`mcp::delegate`）：inline await 完成信号，取回 [`TurnSummary`]
//!   作为 tool_result 回传主 agent。
//!
//! ## 完成信号（TurnSummary）
//!
//! 不改 loop_engine 的任何返回路径：`stream_loop` 返回后从**事件日志**读取本 turn
//! 的 `turn_ended` 事件（termination/rounds/usage）+ 最终 assistant 正文（messages
//! 表）。这是「真相在产物」原则的直接应用——MA-2 任务台账读取同一事实源。
//! 保序保证：`turn_ended` 先于 cleanup()/unregister inline 落库是 event_log 的
//! 硬规则，`stream_loop` 返回时事件必已在库。
//!
//! ## 取消
//!
//! cancel_token 由**调用方**创建并注册 ChatState（两条路径语义不同：用户路径
//! `chat_state.start()` 新建并拒绝重入；委派路径 `parent.child_token()` 级联取消），
//! runner 只消费。spawn 后的注销责任在流式任务的 RAII 守卫（见 [`spawn_stream_loop`]）。

use std::sync::Arc;

use sqlx::SqlitePool;
use tauri::AppHandle;
use uuid::Uuid;

use crate::db::models::{ConversationRow, HookConfig, HookPoint, NewMessage};
use crate::db::repo;
use crate::error::AppResult;
use crate::harness::budget::LoopBudget;
use crate::harness::chat_state::CancellationToken;
use crate::harness::event_log::{self, EventCtx};
use crate::harness::hooks::{has_actions, run_hooks};
use crate::harness::mcp::{McpRegistry, McpServerManager};
use crate::harness::provider;
use crate::harness::read_route::ReadRouteRegistry;
use crate::harness::tool_executor::{build_tool_ctx, ToolAuthRegistry};
use crate::infra::protocol::{ContentBlock, LlmProvider, TokenUsage};

use crate::context::pipeline::{AssembledContext, PipelineContext, PipelineRunner};

/// 一次完整 agent 回合的终态摘要（完成信号负载）。
///
/// 数据源：`turn_ended` 事件 + 最终 assistant 消息正文（见模块注释）。
/// 消费方是 delegate v2 的 tool_result 回传（`mcp::delegate`）。
#[derive(Debug, Clone)]
pub(crate) struct TurnSummary {
    /// 终止原因（复用 finish_reason 词表：stop / tool_use / budget_exceeded / abort / ...）
    pub finish_reason: String,
    /// 最终 assistant 正文（回传给委派方的结果；异常路径为空串）
    pub final_text: String,
    /// 本 turn 完成的 LLM 轮数
    pub rounds: u32,
    /// 多轮 usage 合成（provider 未返回时 None）
    pub usage: Option<TokenUsage>,
}

/// 环境依赖（Tauri managed state 快照，调用方从 State 取出传入）。
pub(crate) struct TurnEnv<'a> {
    /// 对外进度事件出口（S6：编排与循环统一走 LoopEmitter，循环链零 AppHandle）。
    pub emitter: Arc<dyn crate::harness::r#loop::emitter::LoopEmitter>,
    /// 工具上下文注入用真句柄（ToolContext.app_handle；proposal/delegate 工具需要）。
    pub tool_app: Option<AppHandle>,
    pub pool: SqlitePool,
    /// 读路径路由缓存（全局共享实例的引用）
    pub route_registry: &'a ReadRouteRegistry,
    /// 全局 MCP 注册表（工具组装快照来源）
    pub global_registry: Arc<McpRegistry>,
    pub mcp_manager: Arc<McpServerManager>,
    /// 工具授权注册表（与 lib.rs install_listener 同一实例）
    pub auth_registry: ToolAuthRegistry,
}

/// 一回合的输入（调用方完成输入预处理后打包传入）。
///
/// 预处理（附件 materialize / 视觉元提示 / cancel token 注册）留在调用方：
/// 用户路径的预处理面向前端输入（files/图片校验），委派路径是纯文本任务——
/// 两条路径在「历史解析 → Pipeline → 落库 → 循环」处汇合，正是本内核的边界。
pub(crate) struct AgentTurnInput {
    pub conv: ConversationRow,
    /// 目标 agent（委派时 = 专家 agent，与发起方无关——专家用自己的模型）
    pub agent: crate::db::models::AgentRow,
    /// agent.yaml 钩子配置
    pub hooks: HookConfig,
    pub provider: Arc<dyn LlmProvider>,
    pub api_key: String,
    /// 预生成的用户消息 ID（附件分页提示里已嵌此 id，必须复用）
    pub user_msg_id: String,
    /// 用户消息正文（落库 content + 检索 query）
    pub content_text: String,
    /// 发给 LLM 的用户 blocks（materialize + 视觉元提示后）
    pub llm_blocks: Vec<ContentBlock>,
    /// 落库/事件用的原始 blocks（用户真实发送内容，不含视觉适配视图）
    pub persist_blocks: Vec<ContentBlock>,
    /// 大附件分页块（Pipeline 成功后与消息行同批写入）
    pub attach_db_inputs: Vec<repo::message_attachment::AttachmentChunkInput>,
    /// 视觉候选原始字节（扫描件等）
    pub attach_file_inputs: Vec<repo::message_attachment_file::AttachmentFileInput>,
    /// chat:start 是否携带 user_content_blocks（含附件时 patch 前端乐观消息）
    pub emit_user_blocks: bool,
    pub tools_enabled: bool,
    pub model_override: Option<String>,
    /// 已注册到 ChatState 的取消令牌（用户=新建；委派=父 token 的子链）
    pub cancel_token: CancellationToken,
}

/// 运行一次完整 agent 回合（不阻塞等待循环完成）。
///
/// 返回完成信号 Receiver：用户路径直接 drop；委派路径 `rx.await`（配壁钟护栏）
/// 取回 [`TurnSummary`]。spawn 失败前的任何早退返回 `Err`（不落任何 DB 行——
/// 与原 send_message 的「无孤儿用户消息」语义一致）。
pub(crate) async fn run_agent_turn(
    env: &TurnEnv<'_>,
    input: AgentTurnInput,
) -> AppResult<tokio::sync::oneshot::Receiver<TurnSummary>> {
    let AgentTurnInput {
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
        emit_user_blocks,
        tools_enabled,
        model_override,
        cancel_token,
    } = input;

    let conv_id = conv.id.clone();
    let pool = &env.pool;

    // --- 历史解析（session-events Phase 2B：事件派生是唯一读路径） ---
    // legacy 拼装已退役（S1）：resolve 仅作健康监控——非 Derive 说明该会话存在
    // 对账 diff / 混合纪元 / 零事件残留，error 后**照常派生**（历史可能缺行，
    // 不再静默回退）。排查走 reconcile_session / get_read_route_status；
    // 回滚 = revert 阶段 1 commit（messages 表双写持续，Legacy 可整体恢复）。
    let route = env.route_registry.resolve(pool, &conv_id).await?;
    if route.route != crate::harness::read_route::ReadRoute::Derive {
        tracing::error!(
            target: "ice_paw.read_route",
            conv = %conv_id,
            reason = %route.reason,
            diffs = route.diffs,
            "会话读路径非绿（已无 legacy 兜底）：派生历史可能缺行，排查 reconcile_session"
        );
    }
    let history = crate::harness::read_route::load_history_from_events(pool, &conv_id).await?;
    // 最近 10 条工具消息名（loop_engine 动态工具打分用）
    let tool_call_history = repo::message::list_recent_tool_names(pool, &conv_id, 10).await?;

    // 上下文窗口（agent 显式 → 模型表 → 128K 兜底）；max_input_tokens 由
    // TokenWindowStage（硬裁）+ MemoryStage（折叠）消费。
    let context_window = agent
        .context_window
        .map(|v| v as usize)
        .or_else(|| provider::default_context_window(&agent.provider, &agent.model))
        .unwrap_or(128_000);

    // --- Pipeline 拼装（Template → OsContext → SystemPrompt → History →
    //     ToolFailureFold → Memory → TokenWindow → Final） ---
    let mut pipeline_ctx = PipelineContext::new(
        pool.clone(),
        agent.clone(),
        None,
        history,
        final_blocks,
        tools_enabled,
        Some(content_text.clone()),
        tool_call_history.clone(),
        crate::context::token::ContextBudget {
            max_input_tokens: context_window,
        },
        conv_id.clone(),
        cancel_token.clone(),
    );
    // session-events（Phase 0）：turn 归属键，MemoryStage 的 summary_* 事件用
    pipeline_ctx.turn_id = Some(user_msg_id.clone());

    // 项目 workspace + 上下文目录注入 Pipeline
    if let Some(ref pid) = conv.project_id {
        if let Ok(proj) = repo::project::get_by_id(pool, pid).await {
            pipeline_ctx.project_workspace = proj.workspace_path;
        }
        // 项目上下文目录：{default_ws}/projects/{project_id}/
        if let Ok(prefs) = repo::preferences::get_all(pool).await {
            if let Some(ref ws) = prefs.default_workspace_path {
                let dir = format!("{}/projects/{}", ws.trim_end_matches(['/', '\\']), pid);
                pipeline_ctx.project_context_dir = Some(dir);
            }
        }
    }

    // 事2：注入已解析的 agent API key，供 ModalCapabilityStage 收集视觉凭据。
    // 空 key 也允许（agent 可能用 base_url 免 key）。
    pipeline_ctx.api_key = Some(api_key.clone());

    // MA-1：可调度清单注入——主 agent 感知「能调度谁」（项目成员优先，否则全部
    // agent，见 delegate::resolve_dispatchable）。仅用户会话：delegation 子会话没有
    // delegate 工具（下方组装期按 kind 注册），注入清单只会误导。解析失败降级为
    // 跳过注入（不阻塞发送；工具调用时还有集合校验兜底）。
    if tools_enabled && conv.kind == "chat" {
        match crate::harness::mcp::delegate::resolve_dispatchable(
            pool,
            conv.project_id.as_deref(),
            &conv.agent_id,
        )
        .await
        {
            Ok(list) if !list.is_empty() => {
                pipeline_ctx.delegation_hint =
                    Some(crate::harness::mcp::delegate::build_dispatch_hint(&list));
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(
                target: "ice_paw.delegate",
                "解析可调度集合失败（跳过清单注入）: {e}"
            ),
        }
    }

    // M1.5: 构造 LlmSummaryProvider 注入 Pipeline
    use crate::context::memory::SummaryProvider;
    use crate::harness::summary_provider::LlmSummaryProvider;
    let summary_provider: Box<dyn SummaryProvider> = Box::new(LlmSummaryProvider::new(
        llm_provider.clone(),
        api_key.clone(),
    ));
    PipelineRunner::default_pipeline(pool, Some(summary_provider))
        .run(&mut pipeline_ctx)
        .await?;

    // 摘要注入事件（前端暂未 listen，保留面向未来）
    if let Some(event) = pipeline_ctx.summary_event {
        crate::harness::r#loop::emitter::emit_ser(
            env.emitter.as_ref(),
            "chat:summary-injected",
            &event,
        );
    }

    let mut assembled = AssembledContext {
        messages: pipeline_ctx.messages,
    };

    // --- Pipeline 成功 → 落库（用户消息 + 分页块 + assistant 占位） ---
    // 全部 DB 写入推迟到此刻：前置任一失败都不落任何行，无孤儿用户消息。
    // content_text 快照供 user_message 事件（create 会 move）。
    let user_content_snapshot = content_text.clone();
    repo::message::create(
        pool,
        &user_msg_id,
        &NewMessage {
            conversation_id: conv_id.clone(),
            role: "user".into(),
            content: content_text.clone(),
            token_count: None,
            error: None,
            model: None,
        },
    )
    .await?;
    // 回填 content_blocks：用适配前的原始 persist_blocks（含原图 + 附件卡片/正文）。
    // 视觉适配只改发给 LLM 的 messages，不改用户落库内容——历史回看须保留用户真实发送的图片。
    let blocks_json = serde_json::to_string(&persist_blocks).unwrap_or_else(|_| "[]".into());
    repo::message::update_content_blocks(pool, &user_msg_id, &blocks_json).await?;
    // 大附件分页块：消息行已存在（FK 父满足）。幂等：先清旧再批量插。
    if !attach_db_inputs.is_empty() {
        repo::message_attachment::delete_by_message(pool, &user_msg_id).await?;
        repo::message_attachment::insert_batch(pool, &user_msg_id, &attach_db_inputs).await?;
    }
    // Phase B 视觉候选文件字节（扫描件等，文本提取为空）：同批写入。
    if !attach_file_inputs.is_empty() {
        repo::message_attachment_file::delete_by_message(pool, &user_msg_id).await?;
        repo::message_attachment_file::insert_batch(pool, &user_msg_id, &attach_file_inputs)
            .await?;
    }

    // --- session_events 影子写入（Phase 0）：turn 用户侧事实 ---
    let ev = EventCtx::new(&conv_id, &user_msg_id, &agent.id);
    event_log::log_user_message(
        pool,
        &ev,
        &user_msg_id,
        &event_log::UserMessagePayload {
            v: 1,
            content: user_content_snapshot,
            blocks: persist_blocks,
        },
    )
    .await;
    // 附件留存事实——仅元信息（正文在 messages/分页表，字节在 files 表；防三重冗余）。
    if !attach_db_inputs.is_empty() {
        event_log::log_attachment_stored(
            pool,
            &ev,
            &user_msg_id,
            &event_log::AttachmentStoredPayload::Pages {
                v: 1,
                items: attach_db_inputs
                    .iter()
                    .map(|c| event_log::AttachmentPageItem {
                        idx: c.idx,
                        name: c.name.clone(),
                        kind: c.kind.clone(),
                        label: c.label.clone(),
                        token_est: c.token_est,
                    })
                    .collect(),
            },
        )
        .await;
    }
    if !attach_file_inputs.is_empty() {
        event_log::log_attachment_stored(
            pool,
            &ev,
            &user_msg_id,
            &event_log::AttachmentStoredPayload::Bytes {
                v: 1,
                items: attach_file_inputs
                    .iter()
                    .map(|f| event_log::AttachmentBytesItem {
                        idx: f.idx,
                        name: f.name.clone(),
                        ext: f.ext.clone(),
                        bytes_len: f.bytes.len(),
                    })
                    .collect(),
            },
        )
        .await;
    }

    let asst_msg_id = Uuid::new_v4().to_string();
    // 助手消息的 model 字段使用 override（若有），否则回退 Agent 默认 model。
    let effective_model = model_override
        .clone()
        .unwrap_or_else(|| agent.model.clone());
    repo::message::create(
        pool,
        &asst_msg_id,
        &NewMessage {
            conversation_id: conv_id.clone(),
            role: "assistant".into(),
            content: String::new(),
            token_count: None,
            error: None,
            model: Some(effective_model.clone()),
        },
    )
    .await?;

    // --- emit chat:start（cancel_token 已由调用方注册） ---
    // 含附件时把 materialize 后的 content_blocks 带给前端，patch 乐观用户消息。
    // S6：emit 经 LoopEmitter（失败实现内 warn，不再向上传播——原 `?` 只因
    // tauri emit 返回 Result；事件发不出去不应让已落库的回合整体失败）。
    crate::harness::r#loop::emitter::emit_ser(
        env.emitter.as_ref(),
        "chat:start",
        &crate::infra::protocol::ChatStartPayload {
            conversation_id: conv_id.clone(),
            user_message_id: user_msg_id.clone(),
            assistant_message_id: asst_msg_id.clone(),
            user_content_blocks: if emit_user_blocks {
                Some(blocks_json.clone())
            } else {
                None
            },
        },
    );

    // --- 预算（B1 自动续期：显式=硬上限→额度 0；默认 model-aware 3×→可续期） ---
    let tool_max_rounds = agent.tool_max_rounds();
    let agent_max_tokens = agent.max_total_tokens();
    let budget_max_tokens = agent_max_tokens.unwrap_or_else(|| {
        let window = agent
            .context_window
            .map(|v| v as usize)
            .or_else(|| provider::default_context_window(&agent.provider, &agent.model))
            .unwrap_or(128_000);
        window.saturating_mul(3)
    });
    let budget_renewals: u32 = if agent_max_tokens.is_some() {
        0
    } else {
        crate::harness::budget::DEFAULT_AUTO_RENEWALS
    };

    // 单轮输出上限：agent.max_tokens 与模型策展表取 max（只抬不降）。
    let effective_max_tokens = agent.max_tokens.max(
        provider::default_max_output_tokens(&agent.provider, &agent.model).unwrap_or(16_384) as i32,
    );

    // --- 工具组装：全局注册表快照（boot 时已启动全部 server） ---
    // 默认全开：所有已注册工具（内置 + 全局/per_agent MCP server，含平台元工具）
    // 对每个 agent 可用。per_agent server 的 workspace 后台异步绑定，不阻塞。
    let tool_registry = if tools_enabled {
        let reg = McpRegistry::from_map(env.global_registry.snapshot().await);

        // MA-1：delegate 工具按会话类型注册——只有用户会话（kind='chat'）可发起
        // 委派。全局注册表不含此工具（register_builtin 不注入），组装期按 kind
        // 决定：delegation 子会话拿不到它 → 委派深度=1 的结构性护栏（接收方不能
        // 二次委派，「A委派B、B委派回A」的乒乓球在结构上不可能）。
        if conv.kind == "chat" {
            reg.register(Arc::new(crate::harness::mcp::delegate::DelegateTool))
                .await;
        }

        // 后台异步绑定 per_agent server workspace（不阻塞消息发送）
        if let Some(workspace) = agent.workspace_path.as_deref() {
            let mcp_configs = repo::mcp_server::list_all(pool).await.unwrap_or_else(|e| {
                tracing::warn!(target: "ice_paw.chat", "加载 MCP server 配置失败: {e}");
                Vec::new()
            });
            let mgr = Arc::clone(&env.mcp_manager);
            let reg = Arc::clone(&env.global_registry);
            let ws = workspace.to_string();
            tokio::spawn(async move {
                for cfg in mcp_configs
                    .iter()
                    .filter(|c| c.enabled && c.scope == "per_agent")
                {
                    mgr.rebind_workspace_if_needed(&cfg.id, &ws, &reg).await;
                }
            });
        }
        reg
    } else {
        McpRegistry::new()
    };

    // --- session_events：turn_context 快照 ---
    // 「模型用什么模型/看到什么工具/什么预算」——Phase 1 解释行为差异的锚点。
    {
        let tool_names: Vec<String> = tool_registry
            .list_tool_defs()
            .await
            .iter()
            .take(50)
            .map(|d| d.name.clone())
            .collect();
        event_log::log_turn_context(
            pool,
            &ev,
            &event_log::TurnContextPayload {
                v: 1,
                provider: agent.provider.clone(),
                effective_model: effective_model.clone(),
                model_override: model_override.clone(),
                tools_enabled,
                tool_names,
                temperature: Some(agent.temperature),
                max_tokens: Some(effective_max_tokens as i64),
                tool_max_rounds,
                budget_max_tokens: Some(budget_max_tokens as u64),
                context_window: Some(context_window as i64),
            },
        )
        .await;
    }

    // --- 钩子：ConversationStart（对话开始，inject_prompt 追加到 system_prompt）---
    // 在 tool_registry 组装完成后、spawn 前执行：注入结果随 messages 进入流式循环。
    if has_actions(&hooks, HookPoint::ConversationStart) {
        let hook_ctx = build_tool_ctx(
            pool,
            conv_id.clone(),
            conv.agent_id.clone(),
            conv.project_id.clone(),
            Some(api_key.clone()),
        )
        .await;
        match run_hooks(
            HookPoint::ConversationStart,
            &hooks,
            &hook_ctx,
            &tool_registry,
        )
        .await
        {
            Ok(outcome) => {
                if let Some(inj) = outcome.injected_prompt {
                    // session-events：钩子注入是模型可见事实（Model-visible means logged）。
                    event_log::log_hook_injected(
                        pool,
                        &ev,
                        &event_log::HookInjectedPayload {
                            v: 1,
                            point: "conversation_start".into(),
                            prompt: inj.clone(),
                        },
                    )
                    .await;
                    inject_into_system(&mut assembled.messages, &inj);
                }
            }
            Err(e) => tracing::warn!(
                target: "ice_paw.hooks",
                "ConversationStart 钩子执行失败（忽略）: {}", e
            ),
        }
    }

    // --- spawn 流式循环 + 完成信号 ---
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<TurnSummary>();
    spawn_stream_loop(StreamLoopInput {
        emitter: env.emitter.clone(),
        tool_app: env.tool_app.clone(),
        pool: pool.clone(),
        provider: llm_provider,
        api_key,
        messages: assembled.messages,
        temperature: agent.temperature,
        max_tokens: effective_max_tokens,
        cancel_token,
        conv_id,
        user_msg_id,
        asst_msg_id,
        tools_enabled,
        query: Some(content_text),
        call_history: tool_call_history,
        model_override,
        asst_model: Some(effective_model),
        tool_max_rounds,
        budget_max_tokens,
        budget_renewals,
        auth_registry: env.auth_registry.clone(),
        tool_registry,
        agent_id: conv.agent_id.clone(),
        project_id: conv.project_id.clone(),
        hooks,
        done_tx: Some(done_tx),
    });
    Ok(done_rx)
}

/// 从事件日志读取本 turn 终态（完成信号的数据源，见模块注释）。
///
/// 查不到 `turn_ended`（异常退出）/查询失败/payload 损坏 → 诚实的降级摘要
/// （finish_reason="aborted"、空正文），委派方据此向主 LLM 报错而非悬挂。
pub(crate) async fn read_turn_outcome(
    pool: &SqlitePool,
    conv_id: &str,
    turn_id: &str,
) -> TurnSummary {
    let fallback = TurnSummary {
        finish_reason: "aborted".into(),
        final_text: String::new(),
        rounds: 0,
        usage: None,
    };
    // turn_ended 先于 unregister 落库（event_log 硬规则）→ stream_loop 返回时必在库。
    let row: Option<(Option<String>, String)> = match sqlx::query_as(
        "SELECT message_id, payload FROM session_events \
         WHERE session_id = ? AND turn_id = ? AND kind = 'turn_ended' \
         ORDER BY seq DESC LIMIT 1",
    )
    .bind(conv_id)
    .bind(turn_id)
    .fetch_optional(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(target: "ice_paw.session_runner", "读取 turn_ended 失败: {e}");
            return fallback;
        }
    };
    let Some((message_id, payload_json)) = row else {
        tracing::warn!(
            target: "ice_paw.session_runner",
            "turn 无 turn_ended 事件（异常退出路径），降级为 aborted: conv={conv_id} turn={turn_id}"
        );
        return fallback;
    };
    let payload = match serde_json::from_str::<event_log::TurnEndedPayload>(&payload_json) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(target: "ice_paw.session_runner", "turn_ended payload 解析失败: {e}");
            return fallback;
        }
    };
    let final_text = match &message_id {
        Some(id) => sqlx::query_scalar::<_, String>("SELECT content FROM messages WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .unwrap_or_default(),
        None => String::new(),
    };
    TurnSummary {
        finish_reason: payload.termination,
        final_text,
        rounds: payload.rounds,
        usage: payload.usage,
    }
}

/// `spawn_stream_loop` 的入参收拢（S4）：编排层（[`run_agent_turn`]）分散收集的
/// 各 Tauri State / 输入一次性成袋传递，消除 26 参数超长签名与
/// `#[allow(clippy::too_many_arguments)]`。字段与原形参平移；进入循环后的语义
/// 分组见 `LoopConfig` 的注释段与 `LoopContext` 的运行时件。
pub(crate) struct StreamLoopInput {
    /// 对外进度事件出口（S6：Tauri 世界 → 循环抽象世界的转换已在 TurnEnv 完成，
    /// 此处只透传；spawn 内的 RAII 守卫复用其 on_loop_exit 注销 ChatState）。
    pub emitter: Arc<dyn crate::harness::r#loop::emitter::LoopEmitter>,
    /// 工具上下文注入用真句柄（生产 `Some(app)`；测试 `None`）。
    pub tool_app: Option<AppHandle>,
    pub pool: SqlitePool,
    pub provider: Arc<dyn LlmProvider>,
    pub api_key: String,
    pub messages: Vec<crate::infra::protocol::ChatMessage>,
    pub temperature: f64,
    pub max_tokens: i32,
    pub cancel_token: CancellationToken,
    pub conv_id: String,
    pub user_msg_id: String,
    pub asst_msg_id: String,
    pub tools_enabled: bool,
    pub query: Option<String>,
    pub call_history: Vec<String>,
    pub model_override: Option<String>,
    pub asst_model: Option<String>,
    pub tool_max_rounds: Option<u32>,
    pub budget_max_tokens: usize,
    pub budget_renewals: u32,
    pub auth_registry: ToolAuthRegistry,
    pub tool_registry: McpRegistry,
    pub agent_id: String,
    pub project_id: Option<String>,
    pub hooks: HookConfig,
    pub done_tx: Option<tokio::sync::oneshot::Sender<TurnSummary>>,
}

/// spawn LLM 流式协程，把编排结果交给 `harness::loop_engine::stream_loop`。
///
/// MA-1 从 `commands/chat_cmd.rs` 迁入（唯一调用方是 [`run_agent_turn`]），并新增
/// `done_tx` 完成信号：循环退出后从事件日志读取 [`TurnSummary`] 发送。接收方已
/// drop（用户路径不关心结果）时 send 失败，静默忽略。
pub(crate) fn spawn_stream_loop(input: StreamLoopInput) {
    // 解构还原原形参：函数体零改动（S4 仅收袋，不改行为）。
    let StreamLoopInput {
        emitter,
        tool_app,
        pool,
        provider,
        api_key,
        messages,
        temperature,
        max_tokens,
        cancel_token,
        conv_id,
        user_msg_id,
        asst_msg_id,
        tools_enabled,
        query,
        call_history,
        model_override,
        asst_model,
        tool_max_rounds,
        budget_max_tokens,
        budget_renewals,
        auth_registry,
        tool_registry,
        agent_id,
        project_id,
        hooks,
        done_tx,
    } = input;
    tokio::spawn(async move {
        // ★ RAII Drop 守卫：无论此任务如何退出（正常完成 / panic / runtime 关闭时被 drop），
        // 都保证注销 ChatState 中的 cancel_token。这消除了 scopeguard disarm 后
        // 唯一清理路径失效的风险——之前若 stream_loop panic 或 runtime 关闭时 future 被
        // drop 而未执行到 finalize_* → cleanup → unregister，token 永久残留导致会话卡死。
        // S6：经 LoopEmitter::on_loop_exit（TauriEmitter 实现内注销；与 cleanup() 双保险，
        // unregister 幂等）。
        let _cleanup_guard = scopeguard::guard((), {
            let emitter = emitter.clone();
            move |_| emitter.on_loop_exit()
        });

        // tool_registry 由 run_agent_turn 组装（global server + per-agent server），直接使用
        let mut observable = crate::harness::observable::RoundState::default();
        let budget = LoopBudget {
            max_tool_rounds: tool_max_rounds.unwrap_or(LoopBudget::default().max_tool_rounds),
            max_total_tokens: budget_max_tokens,
            // B1：预算续期额度由调用方按「显式=硬上限」策略算好传入；
            // 轮数续期同策略——显式 tool_max_rounds → 0，默认 → DEFAULT_AUTO_RENEWALS。
            max_budget_renewals: budget_renewals,
            max_round_renewals: if tool_max_rounds.is_some() {
                0
            } else {
                crate::harness::budget::DEFAULT_AUTO_RENEWALS
            },
            ..LoopBudget::default()
        };

        // A2-3: 使用共享的工具授权注册表（与 lib.rs install_listener 同一个实例）
        // 这样前端 chat:tool-auth-response 事件能匹配到正确的 oneshot sender。
        let auth_registry = auth_registry;
        // A2-3: 本次会话级已授权路径表
        let auth_session = crate::harness::authority::PathAuthSession::new();
        // A2-3: 路径白名单配置（当前为空 → 全部走 Confirm 流程）
        let whitelist = crate::harness::authority::PathWhitelistConfig::default();

        // A6/S4: LoopConfig(不可变配置) + LoopContext(配置 + 可变运行时件 + 可变消息缓冲)
        let config = crate::harness::loop_engine::LoopConfig {
            conv_id: conv_id.clone(),
            asst_msg_id,
            user_msg_id,
            agent_id,
            project_id,
            emitter,
            tool_app,
            pool,
            provider,
            api_key,
            temperature,
            max_tokens,
            tool_registry,
            tools_enabled,
            whitelist,
            cancel: cancel_token,
            budget,
            query,
            call_history,
            model: model_override,
            asst_model,
            hooks,
        };
        let mut ctx = crate::harness::loop_engine::LoopContext::new(
            config,
            auth_registry,
            auth_session,
            messages,
        );
        crate::harness::loop_engine::stream_loop(&mut ctx, &mut observable).await;
        // W2.4: emit final round-state after stream_loop completes（S6 起经 LoopEmitter，
        // 与循环内的中间发射同一出口）
        crate::harness::r#loop::events::emit_intermediate_round_state(
            ctx.emitter.as_ref(),
            &ctx.conv_id,
            &observable,
        );
        // MA-1 完成信号：从事件日志读取终态后送达委派方（见模块注释）。
        if let Some(tx) = done_tx {
            let summary = read_turn_outcome(&ctx.pool, &ctx.conv_id, &ctx.user_msg_id).await;
            let _ = tx.send(summary);
        }
    });
}

/// 把钩子注入的 prompt 追加到 system 消息（无 system 则创建）。
///
/// 从 `commands/chat_cmd.rs` 迁入（唯一调用方是本模块的 ConversationStart 钩子段）。
pub(crate) fn inject_into_system(
    messages: &mut Vec<crate::infra::protocol::ChatMessage>,
    injected: &str,
) {
    if let Some(sys_msg) = messages.iter_mut().find(|m| m.role == "system") {
        sys_msg
            .content
            .push(ContentBlock::text(injected.to_string()));
    } else {
        messages.insert(
            0,
            crate::infra::protocol::ChatMessage::from_text("system", injected),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_into_system_appends_block_to_existing_system() {
        let mut messages = vec![
            crate::infra::protocol::ChatMessage::from_text("system", "You are X."),
            crate::infra::protocol::ChatMessage::from_text("user", "hi"),
        ];
        inject_into_system(&mut messages, "ALWAYS use JSON.");
        // 仍是同一条 system 消息（追加块，非新增消息）
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content.len(), 2);
        let joined = messages[0].content_text();
        assert!(joined.contains("You are X."));
        assert!(joined.contains("ALWAYS use JSON."));
    }

    #[test]
    fn inject_into_system_creates_system_when_absent() {
        let mut messages = vec![crate::infra::protocol::ChatMessage::from_text("user", "hi")];
        inject_into_system(&mut messages, "system rule");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content_text(), "system rule");
        // 原 user 消息被推到第二位
        assert_eq!(messages[1].role, "user");
    }
}
