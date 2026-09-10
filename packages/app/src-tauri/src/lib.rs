#![warn(clippy::all)]
// edit_docx 的 oneOf 操作 schema（mcp/docx_tool.rs，14 个操作逐个展开）超
// serde_json::json! 宏默认递归上限 128——宏展开纯编译期数据，抬到 256 无运行时影响。
#![recursion_limit = "256"]

//! IcePaw Tauri 应用入口
//!
//! 模块组装：
//! - `error`     —— 统一错误类型
//! - `db`        —— sqlx 连接池与 migrations
//! - `crypto`    —— stronghold wrapper
//! - `commands`  —— 暴露给前端的 invoke 入口
//! - `context`   —— L2 Context 层（W1.1 建壳占位）
//! - `harness`   —— L2 Harness 层（W2.x 逐步填充）
//! - `infra`     —— L0/L1 基础设施层（W1.1 建壳占位）
//! - `loop`      —— L2 Loop 层占位（raw identifier `r#loop`）
//!
//! **W2.3 起**：`llm/` 目录已删除（provider / tool_registry / chat_state 全部迁入 `harness/`）。
//!
//! 启动顺序（setup）：
//!   1. 初始化 tracing（stdout + 磁盘日志）
//!   2. 启动数据库连接池 + 跑迁移
//!   3. 启动 stronghold（snapshot 落到 app_data_dir）
//!   4. 注册全部 commands

pub mod commands;
pub mod context;
pub mod crypto;
pub mod db;
pub mod error;
pub mod harness;
pub mod infra;
pub mod logging;
pub mod platform;

use std::sync::Arc;

use tauri::Manager;

use harness::mcp::McpRegistry;

/// 应用入口
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // boot 计时锚点（治看不见）：setup 段各节点有日志时间戳可查，但 exe 加载 →
    // 插件初始化 → WebView2 环境创建（冷启动白屏的真凶段）发生在日志第一行
    // 之前完全不可见。此 Instant 从进程最早点起算，setup 开始/结束各打一次
    // 总耗时，下次启动慢日志直接指出是哪一段。
    let boot_start = std::time::Instant::now();

    // boot 时刻（UTC，与 session_events.created_at 的 datetime('now') 同格式同语义）：
    // sweep 后台化后的「进程边界」——只补记本进程启动前遗留的未闭合 turn，结构上
    // 排除误杀刚开的新 turn（见 setup 2b）。取于进程最早期，本进程内新事件必然晚于它。
    let boot_at = chrono::Utc::now()
        .naive_utc()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    // WebView2 磁盘缓存瘦身（2026-09-09 生产实案：EBWebView 膨胀到 606MB——Cache
    // 343MB + Code Cache 199MB，Chromium 磁盘缓存默认上限 ≈ 磁盘容量 1/3，大盘上
    // 几百 MB 且淘汰懒散；WebView2 初始化加载巨型 profile = 冷启动白屏「未响应」
    // 的真凶，且发生在 index.html 骨架之前、骨架结构上覆盖不到）。必须跑在
    // WebView2 启动**之前**——启动后 Cache 文件被 msedgewebview2 进程持有锁死。
    let context = tauri::generate_context!();
    #[cfg(windows)]
    prune_webview_cache_on_version_change(&context);

    let mut builder = tauri::Builder::default();
    // 单实例：点审批 toast / 双击 exe 拉起第二进程时拦截并前置主实例（防双开）。
    // 必须最先注册——晚注册则第二实例可能在拦截生效前已起窗。
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            harness::approval_toast::focus_main_window(app);
        }));
    }
    builder
        // 仅保留 opener 业务插件
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        // 系统通知：审批请求后台提醒（发送侧在前端事件层 utils/systemNotify，按失焦判定）
        .plugin(tauri_plugin_notification::init())
        // 窗口状态记忆（尺寸/位置/最大化跨启动保持）：restore 发生在插件加载阶段，
        // 早于 setup —— setup 里的「首启动态默认尺寸」只在无保存状态时生效。
        .plugin(tauri_plugin_window_state::Builder::default().build())
        // 聊天全局状态（CancellationToken 注册表）。屏幕通道写令牌的活性回收
        //（§4.3）共用本注册表：同柄 Clone 注入，is_streaming 只读查询。
        .manage({
            let cs = harness::chat_state::ChatState::new();
            harness::mcp::screen::channel::global().set_liveness(cs.clone());
            cs
        })
        // REQ-XC-010: AgentCmd trait 抽象注入
        .manage::<Option<std::sync::Arc<dyn commands::agent_cmd::AgentCmd>>>(None)
        // Phase 2: 全局 MCP 工具注册表（外部 Server 管理器改在 setup 内注入，见下）。
        .manage(Arc::new(McpRegistry::with_builtin()))
        // session-events Phase 2A：读路径路由缓存（事件日志转新会话主读路径）。
        .manage(harness::read_route::ReadRouteRegistry::new())
        // 注：原 `tauri_plugin_stronghold::Builder::new(...).build()` 注册已移除。
        //
        // 理由（参见 dev2 评审方案 §3.2）：
        //   1. 前端 0 处使用 plugin JS API（已 grep 确认无
        //      `@tauri-apps/plugin-stronghold` 依赖）。
        //   2. plugin 的 `Builder::build()` 在 setup 阶段**不**自动创建
        //      Stronghold 实例，只 `app.manage(StrongholdCollection::default())`
        //      + 注册 password hash fn —— 当前事实上是死权限 +
        //      `capabilities/default.json` 中的 `stronghold:default` 死权限。
        //   3. Rust 侧由 `crypto::init` 自己维护一份 `Stronghold`，snapshot
        //      落盘到 `app_data_dir/stronghold.hold`，不依赖 plugin wrapper。
        //
        // 移除后攻击面减小，Phase 2 接 OS keyring 不依赖 plugin。
        // `tauri-plugin-stronghold` crate 依赖保留（`crypto::init` 仍借用其
        // `stronghold::Stronghold` 类型 wrapper）。
        // 全部 commands 注册
        .invoke_handler(tauri::generate_handler![
            commands::agent_cmd::list_agents,
            commands::agent_cmd::create_agent,
            commands::agent_cmd::update_agent,
            commands::agent_cmd::rotate_agent_api_key,
            commands::agent_cmd::delete_agent,
            commands::agent_yaml::get_agent_yaml_fields,
            commands::agent_yaml::set_agent_yaml_field,
            commands::agent_yaml::set_agent_system_prompt,
            commands::agent_yaml::set_agent_word_profile,
            commands::agent_yaml::set_agent_enabled_tools,
            commands::conversation_cmd::list_all_conversations,
            commands::conversation_cmd::list_conversations,
            commands::conversation_cmd::create_conversation,
            commands::conversation_cmd::rename_conversation,
            commands::conversation_cmd::pin_conversation,
            commands::conversation_cmd::delete_conversation,
            commands::conversation_cmd::update_conversation_tools_override,
            commands::conversation_cmd::export_session_trajectory,
            commands::conversation_cmd::list_session_events,
            commands::conversation_cmd::list_turn_anchors,
            commands::conversation_cmd::trajectory_turn_offset,
            commands::conversation_cmd::get_session_plan,
            commands::conversation_cmd::reconcile_session,
            commands::conversation_cmd::get_read_route_status,
            commands::message_cmd::list_messages,
            commands::message_cmd::create_message,
            // MA-3 收件箱（跨会话通讯）
            commands::channel_cmd::ensure_channel,
            commands::channel_cmd::get_channel,
            commands::channel_cmd::set_channel_coordinator,
            commands::channel_cmd::reelect_channel_coordinator,
            commands::inbox_cmd::list_inbox,
            commands::inbox_cmd::list_inbox_counts,
            commands::inbox_cmd::set_inbox_policy,
            commands::inbox_cmd::respond_inbox_item,
            commands::chat_cmd::send_message,
            commands::chat_cmd::stop_generation,
            commands::chat_cmd::is_conversation_streaming,
            commands::chat_cmd::respond_config_proposal,
            commands::chat_cmd::respond_tool_auth,
            commands::chat_cmd::notify_approval,
            commands::preferences_cmd::get_preferences,
            commands::preferences_cmd::set_preference,
            commands::preferences_cmd::test_vision_config,
            commands::model_profile_cmd::list_model_profiles,
            commands::model_profile_cmd::create_model_profile,
            commands::model_profile_cmd::update_model_profile,
            commands::model_profile_cmd::rotate_model_profile_key,
            commands::model_profile_cmd::delete_model_profile,
            commands::model_profile_cmd::test_model_profile_vision,
            commands::mcp_cmd::list_mcp_servers,
            commands::mcp_cmd::create_mcp_server,
            commands::mcp_cmd::update_mcp_server,
            commands::mcp_cmd::delete_mcp_server,
            commands::mcp_cmd::retry_mcp_server,
            commands::mcp_cmd::set_mcp_enabled,
            commands::mcp_cmd::check_nodejs,
            commands::mcp_cmd::list_builtin_tools,
            commands::kb_cmd::list_kb,
            commands::kb_cmd::create_kb,
            commands::kb_cmd::update_kb,
            commands::kb_cmd::delete_kb,
            commands::kb_cmd::reindex_kb,
            commands::kb_cmd::list_kb_documents,
            commands::kb_cmd::get_kb_stats,
            commands::kb_cmd::test_embedding_config,
            commands::kb_cmd::rebuild_all_embeddings,
            commands::log_cmd::get_logs,
            commands::log_cmd::get_data_dir,
            commands::log_cmd::open_data_dir,
            commands::provider_cmd::list_providers,
            commands::provider_cmd::test_provider_connection,
            commands::provider_cmd::test_agent_model_chain,
            // 屏幕共享通道（批次④ 步骤 1：开/关 + 状态拉取）
            commands::screen_cmd::screen_channel_open,
            commands::screen_cmd::screen_channel_stop,
            commands::screen_cmd::get_screen_channel_state,
            commands::screen_cmd::screen_channel_pause,
            commands::screen_cmd::screen_channel_resume,
            commands::screen_cmd::screen_channel_grant,
            commands::screen_cmd::screen_channel_detach,
            commands::screen_cmd::screen_channel_cycle_hud_monitor,
            commands::screen_cmd::screen_hud_set_form,
            // 项目管理
            commands::project_cmd::list_projects,
            commands::project_cmd::create_project,
            commands::project_cmd::update_project,
            commands::project_cmd::delete_project,
            commands::project_cmd::reorder_projects,
            commands::project_cmd::set_project_agents,
            commands::project_cmd::add_project_agent,
            commands::project_cmd::remove_project_agent,
            commands::project_cmd::list_conversations_by_project,
            commands::project_cmd::move_conversation_to_project,
            commands::project_cmd::archive_project,
            commands::project_cmd::unarchive_project,
            commands::project_cmd::permanent_delete_project,
            commands::project_cmd::get_project_context,
            commands::project_cmd::set_project_context,
            commands::project_cmd::open_project_context_dir,
            // MA-2 项目台账 / 项目轨迹 / 概览（纯只读派生）
            commands::project_cmd::list_project_tasks,
            commands::project_cmd::list_project_events,
            commands::project_cmd::get_project_overview,
        ])
        // 启动逻辑（move：闭包捕获 boot_at/boot_start，非 move 借用不满足 'static）
        .setup(move |app| {
            let handle = app.handle().clone();

            // 批次④ 步骤 2：屏幕通道状态广播器——gate 路径的令牌/队列变化
            // （不经命令层）由此 emit 到 HUD/主窗（channel::bump 内调用）。
            harness::mcp::screen::channel::global().set_broadcaster(handle.clone());

            // 0) 初始化日志（stdout + 文件 daily 轮转，非阻塞写）+ panic hook。
            //    WorkerGuard 托管到 app state，进程退出时随 state drop 自动 flush。
            match logging::init(&handle) {
                Ok(guard) => {
                    handle.manage(guard);
                }
                Err(e) => eprintln!("[ice_paw] logging init failed: {e}"),
            }

            // boot 锚点 1：setup 前段 = exe 加载 + 插件初始化 + WebView2 环境创建
            // （窗口已可见但 index.html 未加载 = 白屏段）。这段慢 = WebView2/profile
            // 层问题（缓存膨胀/杀毒扫描/冷盘），与 setup 内逻辑无关。
            tracing::info!(
                target: "ice_paw",
                "boot 锚点：setup 前段（exe + 插件 + WebView2 初始化）耗时 {}ms",
                boot_start.elapsed().as_millis()
            );

            // 窗口兜底显示：主窗 visible=false（消灭冷启动白屏窗口），常路径 =
            // 前端 main.ts 挂载完成即 show。前端 JS 异常（死循环/白屏）时窗口会
            // 永不可见 = 应用「打不开」——10s 后强制 show，恢复「有窗可见、问题
            // 可见」的诚实状态（show 幂等，正常路径已可见时无感）。
            {
                let win_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    if let Some(win) = win_handle.get_webview_window("main") {
                        let _ = win.show();
                    }
                });
            }

            // 0b) 首启动态默认窗口尺寸：固定 1200×800 在 1440p 上只占工作区
            //     47%（显小），在 1080p 上又已占 63%。按窗口所在屏工作区比例计算并
            //     夹紧：宽 66%∈[1200,1680]，高 72%∈[760,1000]，再各自不超工作区-40
            //     （小屏收敛防溢出）。仅首启生效：以 window-state 插件状态文件不存在
            //     为「首启」判据（restore 发生在插件加载阶段，早于本 setup——存在状态
            //     时窗口已被恢复为用户尺寸，此处不再介入；之后跨启动完全尊重用户选择）。
            let has_window_state = handle
                .path()
                .app_config_dir()
                .map(|d| d.join(tauri_plugin_window_state::DEFAULT_FILENAME).exists())
                .unwrap_or(false);
            if !has_window_state {
                if let Some(win) = app.get_webview_window("main") {
                    // `hwnd()` 是 Tauri 的 Windows-only API（CI 在 ubuntu 编译裸调用即 E0599）。
                    // 非 Windows：platform 层 Hwnd=() 且工作区恒 None → 跳过动态尺寸，
                    // 保留 tauri.conf 默认尺寸。
                    #[cfg(windows)]
                    let hwnd = win.hwnd().ok().map(|h| h.0).unwrap_or(std::ptr::null_mut());
                    #[cfg(not(windows))]
                    let hwnd = ();
                    if let Some(work) = platform::primary_monitor_work_area(hwnd) {
                        // 工作区是物理像素；窗口 API 用逻辑尺寸，按窗口缩放换算。
                        let scale = win.scale_factor().unwrap_or(1.0);
                        let work_w = (work.right - work.left) as f64 / scale;
                        let work_h = (work.bottom - work.top) as f64 / scale;
                        let w = (work_w * 0.66).clamp(1200.0, 1680.0).min((work_w - 40.0).max(900.0));
                        let h = (work_h * 0.72).clamp(760.0, 1000.0).min((work_h - 40.0).max(600.0));
                        let _ = win.set_size(tauri::LogicalSize::new(w, h));
                        let cx = (work.left as f64 / scale + (work_w - w) / 2.0).max(0.0);
                        let cy = (work.top as f64 / scale + (work_h - h) / 2.0).max(0.0);
                        let _ = win.set_position(tauri::LogicalPosition::new(cx, cy));
                    }
                }
            }

            // 1) stronghold（同步初始化）
            if let Err(e) = crypto::init(&handle) {
                eprintln!("[ice_paw] stronghold init failed: {e}");
                return Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>);
            }

            // 2) 数据库（async，需 block_on）
            let pool = match tauri::async_runtime::block_on(async {
                db::init_pool(&handle).await
            }) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[ice_paw] db init failed: {e}");
                    return Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>);
                }
            };
            tracing::info!(
                target: "ice_paw",
                "数据库连接池就绪，最大连接数 = {}",
                pool.size()
            );

            // 2b) + 2b-2) boot 自愈两件（崩溃扫尾 + 旧会话 backfill）——后台化：
            //      纯增量幂等扫尾，不阻塞首屏任何读路径（晚 1-2 秒完成无感）。
            //      曾在主线程 block_on 跑，大库上把 setup 拖长 → 窗口已显示而
            //      主线程冻结卡住 WebView 渲染 = 冷启动长时间白屏（2026-09-07
            //      生产反馈根治批；MCP boot / KB watcher 已是 spawn 先例）。
            //      sweep 带 boot_at 时间上界：与「用户立刻发消息开新 turn」并发
            //      时结构排除误杀（find_open_turns 只扫 created_at < boot_at）。
            {
                let sweep_pool = pool.clone();
                tauri::async_runtime::spawn(async move {
                    let swept =
                        harness::event_log::sweep_interrupted_turns(&sweep_pool, &boot_at).await;
                    if swept > 0 {
                        tracing::info!(
                            target: "ice_paw",
                            "崩溃自愈：补记 {swept} 个中断 turn 的 turn_ended(interrupted)"
                        );
                    }

                    // 旧会话事件 backfill（Phase 2B 前置，幂等、纯增量）：零事件
                    // 旧会话反向合成 session_events → 对账零 diff → read_route
                    // 自动路由 Derive。不碰 messages 行、不碰真实事件；就算合成
                    // 有错 → reconcile diff → 自动回退 Legacy（安全网）。
                    let bf = harness::backfill::backfill_legacy_sessions(&sweep_pool).await;
                    if bf.backfilled > 0 || bf.failed > 0 {
                        tracing::info!(
                            target: "ice_paw.backfill",
                            sessions = bf.backfilled,
                            events = bf.events_written,
                            bytes = bf.payload_bytes,
                            failed = bf.failed,
                            epoch_rows = bf.epoch_rows,
                            "旧会话事件 backfill 完成"
                        );
                    }
                });
            }

            // 3) A2-3: 安装工具授权响应全局监听器（前端 chat:tool-auth-response）
            let auth_registry = harness::tool_executor::ToolAuthRegistry::new();
            auth_registry.install_listener(
                &handle,
                "chat:tool-auth-response".to_string(),
                "[tool_auth] 收到未知 request_id 的授权响应（可能已超时）".to_string(),
                "[tool_auth] 授权响应解析失败".to_string(),
            );
            handle.manage(auth_registry);

            // 3a) L0: 会话级授权记忆注册表（conv_id → PathAuthSession，跨轮持久，
            //      app 进程内生命周期；委派预授权 seed 也走此处）
            handle.manage(harness::authority::AuthSessionRegistry::new());

            // 3b) 安装配置提案响应全局监听器（前端 chat:config-proposal-response）
            let proposal_registry = harness::proposal_registry::ProposalRegistry::new();
            proposal_registry.install_listener(
                &handle,
                "chat:config-proposal-response".to_string(),
                "[mgmt] 收到未知 request_id 的提案响应（可能已超时）".to_string(),
                "[mgmt] 提案响应解析失败".to_string(),
            );
            handle.manage(proposal_registry);

            // 3c) 会话事件通知总线（轨迹 live v2）：订阅 event_log 的 append 广播，
            //     转 Tauri event 推给前端——前端按 conversation_id 过滤后用已载
            //     max_seq 游标拉增量（list_after），替代纯轮询的固定延迟。
            //     事件在 append 落库成功后才广播，通知到达时行必可查，无竞态。
            {
                use tauri::Emitter;
                let mut rx = harness::event_log::event_bus().subscribe();
                let emit_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        // Lagged（容量 256 内消费不过来丢帧）不算致命：跳过继续收，
                        // 只有 Closed（全部发送端放下）才退出。旧写法 while let Ok
                        // 把 Lagged 当 Err 永久退出循环，session:event-appended 全局
                        // 停发到重启（2026-09-04 质检 Q4；前端有 5s 轮询兜底但 live
                        // 增量退化为轮询）。
                        match rx.recv().await {
                            Ok(note) => {
                                let _ = emit_handle.emit("session:event-appended", note);
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                tracing::warn!(
                                    target: "ice_paw.event_bus",
                                    "事件通知转发落后丢帧（live 增量由前端 5s 轮询兜底），继续接收"
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
            }

            // 3d) MA-3 收件箱排空观察者：accept 会话的 turn_ended 广播 → 等静默
            //     → pending 非空且配额未尽 → 消费下一条（链式排空）。零侵入
            //     loop 退出路径（与 3c 同一广播源的独立订阅）。必须在 3c 之后
            //     无依赖，独立 spawn 保证 lagged 互不传染。
            harness::inbox::spawn_drain_watcher(handle.clone());

            // 3e) 频道 v1 回合结束观察者：频道回合的 turn_ended 广播 → 等静默
            //     → sender sweep → 抢占检查/接力检查（成员终文 @ 解析）。
            //     与 3c/3d 同一广播源的独立订阅，lagged 互不传染。
            harness::channel::spawn_channel_watcher(handle.clone());

            // 4) REQ-XC-010: 注入 AgentCmd trait object (生产实现 SqlAgentCmd)
            // 覆盖 builder 阶段注入的 None 占位。
            let sql_agent_cmd: std::sync::Arc<dyn commands::agent_cmd::AgentCmd> =
                std::sync::Arc::new(commands::agent_cmd::SqlAgentCmd::new(
                    handle.clone(),
                    pool.clone(),
                ));
            handle.manage(sql_agent_cmd);

            // 4b) ModelProfile（模型配置实体，Phase 1）：注入 trait object。
            // 视觉读取/语义检索经它取凭据；agent 链路 Phase 2 接入。
            let sql_model_profile_cmd: std::sync::Arc<
                dyn commands::model_profile_cmd::ModelProfileCmd,
            > = std::sync::Arc::new(commands::model_profile_cmd::SqlModelProfileCmd::new(
                handle.clone(),
                pool.clone(),
            ));
            handle.manage(sql_model_profile_cmd);

            // 5) Phase 2: 种子默认 MCP Server + 启动已启用的外部 MCP Server
            // 注入 McpServerManager（携带 AppHandle —— bundled 运行时解析 resource_dir 需要）。
            handle.manage(Arc::new(harness::mcp::McpServerManager::new_with_handle(handle.clone())));
            let mcp_registry: Arc<McpRegistry> = handle.state::<Arc<McpRegistry>>().inner().clone();
            let mcp_manager: Arc<harness::mcp::McpServerManager> =
                handle.state::<Arc<harness::mcp::McpServerManager>>().inner().clone();
            // 后台启动 MCP Server（不阻塞应用启动）
            let boot_registry = mcp_registry.clone();
            let boot_manager = mcp_manager.clone();
            let boot_pool = pool.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = db::repo::mcp_server::seed_defaults(&boot_pool).await {
                    tracing::warn!(target: "ice_paw.mcp", "种子默认 MCP Server 失败: {e}");
                }
                let configs = match db::repo::mcp_server::list_all(&boot_pool).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(target: "ice_paw.mcp", "加载 MCP Server 配置失败: {e}");
                        return;
                    }
                };

                let boot_ws = std::env::temp_dir().to_string_lossy().to_string();
                let futures: Vec<_> = configs.iter().filter(|c| c.enabled).map(|cfg| {
                    let cfg = cfg.clone();
                    let registry = boot_registry.clone();
                    let manager = boot_manager.clone();
                    let ws = boot_ws.clone();
                    async move {
                        let workspace = if cfg.scope == "per_agent" { Some(ws.as_str()) } else { None };
                        tracing::info!(
                            target: "ice_paw.mcp",
                            "启动 MCP Server '{}' (scope={})",
                            cfg.name, cfg.scope,
                        );
                        if let Err(e) = manager.start_server(&cfg, workspace, &registry).await {
                            tracing::warn!(
                                target: "ice_paw.mcp",
                                "MCP Server '{}' 启动失败: {}",
                                cfg.name, e,
                            );
                        }
                    }
                }).collect();
                futures::future::join_all(futures).await;
                tracing::info!(target: "ice_paw.mcp", "所有 MCP Server 启动完成");
            });

            // 5b) 旧模型配置 → ModelProfile 实体迁移（ModelProfile Phase 1，幂等）。
            //     轻（1 读 + ≤5 行写 + Stronghold 槽位），同步跑保证「迁移完成」
            //     先于 KB watcher 首扫（watcher 即消费 embedding 引用配置）；
            //     失败 warn 不阻塞（读侧回落旧格式，下次启动重放）。
            tauri::async_runtime::block_on(async {
                harness::legacy_model_migration::migrate_legacy_model_prefs(&handle, &pool).await;
            });

            // 5c) 存量 agent 模型配置 → ModelProfile 实体（Phase 2 存量抽离，
            //     一次性）。同配置（厂商+模型+端点+Key）合并为一个实体多 agent
            //     共引；幂等标记跑过即不再扫（未来手动新建的 agent 保持手动形态）；
            //     失败 warn 下次重放收敛（agent 手动形态照常可用）。
            tauri::async_runtime::block_on(async {
                harness::agent_profile_migration::migrate_agent_models(&handle, &pool).await;
            });

            // 6) RAG: 启动知识库 watcher 管理器（运行时可增删监听 + 首次全量索引）。
            //    KbWatcherManager 注入 Tauri State，供 agent_cmd 在 create/update/delete
            //    时对账（运行期新建 agent 的 KB 目录不再需要重启即可被监听）。
            //    后台运行，失败仅 warn，不阻止应用启动。
            let pool_for_kb = pool.clone();
            let handle_for_kb = handle.clone();
            tauri::async_runtime::spawn(async move {
                let wm = match harness::kb::watcher_manager::KbWatcherManager::new(
                    pool_for_kb.clone(),
                    // AppHandle 通道：embedding 引用链（ModelProfile Phase 1）解
                    // Stronghold key 用——须在 migrate_legacy_model_prefs 落引用键
                    // 之后启动（首扫即消费新配置）。
                    Some(handle_for_kb.clone()),
                )
                {
                    Ok(w) => Arc::new(w),
                    Err(e) => {
                        tracing::warn!(target: "ice_paw.kb", "KbWatcherManager 创建失败: {e}");
                        return;
                    }
                };
                handle_for_kb.manage(wm.clone());

                // 先确保约定 KB 行存在（global + 各 agent），失败仅 warn 继续用现有 KB。
                if let Err(e) = harness::kb::ensure::ensure_default_kbs(&pool_for_kb).await {
                    tracing::warn!(target: "ice_paw.kb", "ensure 约定 KB 失败（继续用现有 KB）: {e}");
                }
                let kbs = match db::repo::kb::list_all(&pool_for_kb).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(target: "ice_paw.kb", "加载 KB 列表失败: {e}");
                        return;
                    }
                };
                for kb in kbs.into_iter().filter(|k| k.enabled) {
                    wm.add_watch(kb.id, kb.directory);
                }
                tracing::info!(target: "ice_paw.kb", "KB watcher 管理器启动完成");
            });

            // 共享模板目录（D17）：安装包模板资产落盘 <app_data_dir>/templates/。
            // 幂等、不覆盖用户改动；write_docx 解析链 = workspace templates/ →
            // 此目录 → 内置档位（失败仅 warn，内置档位仍可用）。
            if let Ok(data_dir) = crate::logging::data_dir(&handle) {
                let tpl_dir = data_dir.join("templates");
                match harness::doc::shared_templates::ensure_shared_templates(&tpl_dir) {
                    Ok(0) => {}
                    Ok(n) => {
                        tracing::info!(target: "ice_paw.doc", "共享模板落盘 {n} 份: {}", tpl_dir.display());
                    }
                    Err(e) => {
                        tracing::warn!(target: "ice_paw.doc", "共享模板落盘失败（内置档位仍可用）: {e}");
                    }
                }
            }

            let _ = pool;

            // boot 锚点 2：setup 全链完成。锚点 1 → 2 之差 = setup 内逻辑耗时
            // （Stronghold/迁移/注册表等），与 WebView2 层问题二分定位。
            tracing::info!(
                target: "ice_paw",
                "boot 锚点：setup 完成，进程 boot 总耗时 {}ms",
                boot_start.elapsed().as_millis()
            );
            Ok(())
        })
        .run(context)
        .expect("error while running tauri application");
}

// =========================================================================
// WebView2 磁盘缓存瘦身（版本升级时一次性，run() 最早期执行）
// =========================================================================

/// 版本变化 → 删除 WebView2 的 HTTP 磁盘缓存（Default/Cache）与 V8 代码缓存
/// （Default/Code Cache）。这两目录是纯派生缓存（不含 Cookies/登录态/Local
/// Storage 等用户数据，那些不碰），删了由 WebView2 按需重建。
///
/// 为什么按「版本变化」触发：发版后前端资产 hash 全变，旧缓存条目整体变垃圾；
/// 而版本不变时缓存仍有命中价值（同一资产二次加载免读盘）——只在升级后首启清
/// 一次，日常启动零开销（一次读小标记文件）。
///
/// 增长上限另由 tauri.conf `additionalBrowserArgs --disk-cache-size=50MB` 封顶
/// （Chromium 默认 ≈ 磁盘 1/3，大盘上几百 MB 淘汰懒散 = 本实案 606MB 的成因）。
#[cfg(windows)]
fn prune_webview_cache_on_version_change(context: &tauri::Context) {
    use std::path::PathBuf;

    let Some(version) = context.config().version.as_ref() else {
        return;
    };
    let version = version.to_string();
    let Ok(local_app) = std::env::var("LOCALAPPDATA") else {
        return;
    };
    let root: PathBuf = PathBuf::from(local_app)
        .join(&context.config().identifier)
        .join("EBWebView");
    prune_impl(&root, &version);
}

/// 内核与路径解耦：`root` = EBWebView 目录（测试注入 tmp 目录缩样布局）。
#[cfg(windows)]
fn prune_impl(root: &std::path::Path, version: &str) {
    let marker = root.join(".webview-cache-gen");

    // 同版本快路径：不动（一次读小标记文件，<1ms）
    if std::fs::read_to_string(&marker).ok().as_deref() == Some(version) {
        return;
    }

    let profile = root.join("Default");
    let mut freed: u64 = 0;
    let mut failed = false;
    for sub in ["Cache", "Code Cache"] {
        let dir = profile.join(sub);
        if !dir.is_dir() {
            continue;
        }
        let size: u64 = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => freed += size,
            Err(e) => {
                failed = true;
                eprintln!("[ice_paw] WebView2 缓存清理失败（{sub}）: {e}");
            }
        }
    }
    if freed > 0 || !failed {
        eprintln!(
            "[ice_paw] 版本变化（{version}），清理 WebView2 缓存 {} 字节",
            freed
        );
    }
    // 尽力而为语义：无论成败都落标记（失败常为暂时性文件占用，但每次启动重删
    // 几百 MB 同样拖启动；真删不掉由 disk-cache-size 上限兜住增长）
    let _ = std::fs::write(&marker, version);
}

#[cfg(test)]
mod tests {
    // prune 的目录副作用在 tmp 目录验证（真实 EBWebView 布局缩样）
    #[cfg(windows)]
    #[test]
    fn prune_clears_caches_on_version_change_and_skips_same_version() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("icepaw_prune_{}", uuid::Uuid::new_v4()));
        // prune_impl 的 root 语义 = EBWebView 目录本身；tmp 下缩样真实布局
        let webview = root.join("EBWebView");
        let profile = webview.join("Default");
        fs::create_dir_all(profile.join("Cache/Cache_Data")).unwrap();
        fs::create_dir_all(profile.join("Code Cache/Code Cache")).unwrap();
        fs::write(profile.join("Cache/Cache_Data/f_000001"), vec![0u8; 1024]).unwrap();
        fs::write(profile.join("Code Cache/Code Cache/abc"), vec![0u8; 512]).unwrap();
        // 用户数据目录（Cookies 等）必须不碰
        fs::write(profile.join("Cookies"), b"user-data").unwrap();

        let marker = webview.join(".webview-cache-gen");
        // 版本 v1 → 清两缓存 + 落标记
        super::prune_impl(&webview, "1.0.0");
        assert!(!profile.join("Cache").exists());
        assert!(!profile.join("Code Cache").exists());
        assert!(profile.join("Cookies").exists(), "用户数据不得被清");
        assert_eq!(fs::read_to_string(&marker).unwrap(), "1.0.0");

        // 重建缓存后同版本再跑 → 快路径不删（缓存条目保留）
        fs::create_dir_all(profile.join("Cache")).unwrap();
        fs::write(profile.join("Cache/keep"), b"x").unwrap();
        super::prune_impl(&webview, "1.0.0");
        assert!(profile.join("Cache/keep").exists(), "同版本不得重删");

        // 版本 v2 → 再清
        super::prune_impl(&webview, "2.0.0");
        assert!(!profile.join("Cache").exists());
        assert_eq!(fs::read_to_string(&marker).unwrap(), "2.0.0");

        let _ = fs::remove_dir_all(&root);
    }
}
