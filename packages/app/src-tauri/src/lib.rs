#![warn(clippy::all)]

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
pub mod r#loop;

use std::sync::Arc;

use tauri::Manager;

use harness::mcp::McpRegistry;

/// 应用入口
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 仅保留 opener 业务插件
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        // 聊天全局状态（CancellationToken 注册表）
        .manage(harness::chat_state::ChatState::new())
        // REQ-XC-010: AgentCmd trait 抽象注入
        .manage::<Option<std::sync::Arc<dyn commands::agent_cmd::AgentCmd>>>(None)
        // Phase 2: 全局 MCP 工具注册表 + 外部 Server 管理器
        .manage(Arc::new(McpRegistry::with_builtin()))
        .manage(Arc::new(harness::mcp::McpServerManager::new()))
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
            commands::conversation_cmd::list_all_conversations,
            commands::conversation_cmd::list_conversations,
            commands::conversation_cmd::create_conversation,
            commands::conversation_cmd::rename_conversation,
            commands::conversation_cmd::pin_conversation,
            commands::conversation_cmd::delete_conversation,
            commands::conversation_cmd::update_conversation_tools_override,
            commands::message_cmd::list_messages,
            commands::message_cmd::create_message,
            commands::chat_cmd::send_message,
            commands::chat_cmd::stop_generation,
            commands::preferences_cmd::get_preferences,
            commands::preferences_cmd::set_preference,
            commands::mcp_cmd::list_mcp_servers,
            commands::mcp_cmd::create_mcp_server,
            commands::mcp_cmd::update_mcp_server,
            commands::mcp_cmd::delete_mcp_server,
            commands::mcp_cmd::restart_mcp_server,
            commands::mcp_cmd::list_active_mcp_servers,
            commands::mcp_cmd::list_mcp_server_tools,
            commands::kb_cmd::list_kb,
            commands::kb_cmd::create_kb,
            commands::kb_cmd::update_kb,
            commands::kb_cmd::delete_kb,
            commands::kb_cmd::reindex_kb,
            commands::kb_cmd::list_kb_documents,
            commands::log_cmd::get_logs,
            commands::log_cmd::get_data_dir,
            commands::log_cmd::open_data_dir,
        ])
        // 启动逻辑
        .setup(|app| {
            let handle = app.handle().clone();

            // 0) 初始化日志（stdout + 文件 daily 轮转，非阻塞写）+ panic hook。
            //    WorkerGuard 托管到 app state，进程退出时随 state drop 自动 flush。
            match logging::init(&handle) {
                Ok(guard) => {
                    handle.manage(guard);
                }
                Err(e) => eprintln!("[ice_paw] logging init failed: {e}"),
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

            // 3) A2-3: 安装工具授权响应全局监听器（前端 chat:tool-auth-response）
            let auth_registry = harness::tool_executor::ToolAuthRegistry::new();
            auth_registry.install_listener(&handle);
            // 把注册表 manage 起来，方便后续扩展（当前主要给 setup 用）
            handle.manage(auth_registry);

            // 4) REQ-XC-010: 注入 AgentCmd trait object (生产实现 SqlAgentCmd)
            // 覆盖 builder 阶段注入的 None 占位。
            let sql_agent_cmd: std::sync::Arc<dyn commands::agent_cmd::AgentCmd> =
                std::sync::Arc::new(commands::agent_cmd::SqlAgentCmd::new(
                    handle.clone(),
                    pool.clone(),
                ));
            handle.manage(sql_agent_cmd);

            // 5) Phase 2: 启动已启用的外部 MCP Server
            let mcp_registry: Arc<McpRegistry> = handle.state::<Arc<McpRegistry>>().inner().clone();
            let mcp_manager: Arc<harness::mcp::McpServerManager> =
                handle.state::<Arc<harness::mcp::McpServerManager>>().inner().clone();
            match tauri::async_runtime::block_on(async {
                let configs = db::repo::mcp_server::list_all(&pool).await?;
                for cfg in &configs {
                    // per-agent 架构：仅全局启动 scope=global 的 server；
                    // scope=per_agent 在 send_message 时按 agent 启动（args 替换 workspace）
                    if cfg.enabled && cfg.scope == "global" {
                        tracing::info!(
                            target: "ice_paw.mcp",
                            "正在启动 MCP Server '{}' (command: {})",
                            cfg.name, cfg.command,
                        );
                        if let Err(e) = mcp_manager.start(cfg, &mcp_registry).await {
                            tracing::error!(
                                target: "ice_paw.mcp",
                                "MCP Server '{}' 启动失败: {}",
                                cfg.name, e,
                            );
                        }
                    } else if cfg.enabled && cfg.scope == "per_agent" {
                        // per_agent server：探测工具清单（临时启动+关闭，供工具集页展示能力）
                        if let Err(e) = mcp_manager.probe_tools(cfg).await {
                            tracing::warn!(
                                target: "ice_paw.mcp",
                                "per-agent MCP Server '{}' 工具探测失败: {}",
                                cfg.name, e,
                            );
                        }
                    }
                }
                Ok::<_, crate::error::AppError>(())
            }) {
                Ok(_) => tracing::info!(target: "ice_paw.mcp", "MCP Server 启动完成"),
                Err(e) => tracing::warn!(target: "ice_paw.mcp", "MCP Server 启动异常: {}", e),
            }

            // 6) RAG: 启动知识库 watcher（监听目录变更自动索引 + 首次全量扫描）。
            //    后台运行，失败仅 warn，不阻止应用启动。
            let pool_for_kb = pool.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = harness::kb::watcher::start(pool_for_kb).await {
                    tracing::warn!(target: "ice_paw.kb", "知识库 watcher 启动失败: {}", e);
                }
            });

            let _ = pool;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
