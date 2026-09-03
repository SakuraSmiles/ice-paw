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
            commands::chat_cmd::send_message,
            commands::chat_cmd::stop_generation,
            commands::chat_cmd::is_conversation_streaming,
            commands::chat_cmd::respond_config_proposal,
            commands::chat_cmd::respond_tool_auth,
            commands::chat_cmd::notify_approval,
            commands::preferences_cmd::get_preferences,
            commands::preferences_cmd::set_preference,
            commands::preferences_cmd::test_vision_config,
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
        // 启动逻辑
        .setup(|app| {
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

            // 2b) 崩溃自愈扫尾（幂等）：上次进程死亡（崩溃/kill/断电/关窗时在途）
            //     绕过所有退出路径，未闭合的 turn 会永远「进行中」并毒害
            //     turn_ended 派生状态机（MA-2 台账）。本地单进程 → 启动时任何
            //     未闭合 turn 定义上已死，补记 truthful 终态 interrupted。
            let swept =
                tauri::async_runtime::block_on(harness::event_log::sweep_interrupted_turns(&pool));
            if swept > 0 {
                tracing::info!(
                    target: "ice_paw",
                    "崩溃自愈：补记 {swept} 个中断 turn 的 turn_ended(interrupted)"
                );
            }

            // 2b-2) 旧会话事件 backfill（Phase 2B 前置，幂等、纯增量）：零事件
            //        旧会话反向合成 session_events → 对账零 diff → read_route
            //        自动路由 Derive。不碰 messages 行、不碰真实事件；就算合成
            //        有错 → reconcile diff → 自动回退 Legacy（安全网）。
            let bf = tauri::async_runtime::block_on(harness::backfill::backfill_legacy_sessions(
                &pool,
            ));
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

            // 3) A2-3: 安装工具授权响应全局监听器（前端 chat:tool-auth-response）
            let auth_registry = harness::tool_executor::ToolAuthRegistry::new();
            auth_registry.install_listener(
                &handle,
                "chat:tool-auth-response".to_string(),
                "[tool_auth] 收到未知 request_id 的授权响应（可能已超时）".to_string(),
                "[tool_auth] 授权响应解析失败".to_string(),
            );
            handle.manage(auth_registry);

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
                    while let Ok(note) = rx.recv().await {
                        let _ = emit_handle.emit("session:event-appended", note);
                    }
                });
            }

            // 4) REQ-XC-010: 注入 AgentCmd trait object (生产实现 SqlAgentCmd)
            // 覆盖 builder 阶段注入的 None 占位。
            let sql_agent_cmd: std::sync::Arc<dyn commands::agent_cmd::AgentCmd> =
                std::sync::Arc::new(commands::agent_cmd::SqlAgentCmd::new(
                    handle.clone(),
                    pool.clone(),
                ));
            handle.manage(sql_agent_cmd);

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

            // 6) RAG: 启动知识库 watcher 管理器（运行时可增删监听 + 首次全量索引）。
            //    KbWatcherManager 注入 Tauri State，供 agent_cmd 在 create/update/delete
            //    时对账（运行期新建 agent 的 KB 目录不再需要重启即可被监听）。
            //    后台运行，失败仅 warn，不阻止应用启动。
            let pool_for_kb = pool.clone();
            let handle_for_kb = handle.clone();
            tauri::async_runtime::spawn(async move {
                let wm = match harness::kb::watcher_manager::KbWatcherManager::new(pool_for_kb.clone())
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
