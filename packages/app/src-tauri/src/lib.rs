#![warn(clippy::all)]

//! IcePaw Tauri 应用入口
//!
//! 模块组装：
//! - `error`     —— 统一错误类型
//! - `db`        —— sqlx 连接池与 migrations
//! - `crypto`    —— stronghold wrapper
//! - `commands`  —— 暴露给前端的 invoke 入口
//!
//! 启动顺序（setup）：
//!   1. 初始化 tracing
//!   2. 启动数据库连接池 + 跑迁移
//!   3. 启动 stronghold（snapshot 落到 app_data_dir）
//!   4. 注册全部 commands

pub mod commands;
pub mod crypto;
pub mod db;
pub mod error;
pub mod llm;

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// 应用入口
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 初始化日志：RUST_LOG 缺省给 info
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().with_target(true))
        .init();

    tauri::Builder::default()
        // 仅保留 opener 业务插件
        .plugin(tauri_plugin_opener::init())
        // 聊天全局状态（CancellationToken 注册表）
        .manage(llm::ChatState::new())
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
            commands::conversation_cmd::list_conversations,
            commands::conversation_cmd::create_conversation,
            commands::conversation_cmd::rename_conversation,
            commands::conversation_cmd::pin_conversation,
            commands::conversation_cmd::delete_conversation,
            commands::message_cmd::list_messages,
            commands::message_cmd::create_message,
            commands::template_cmd::list_templates,
            commands::template_cmd::get_template,
            commands::template_cmd::create_template,
            commands::template_cmd::update_template,
            commands::template_cmd::delete_template,
            commands::chat_cmd::send_message,
            commands::chat_cmd::stop_generation,
        ])
        // 启动逻辑
        .setup(|app| {
            let handle = app.handle().clone();

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
            let _ = pool;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
