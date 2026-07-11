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
        // 注册 stronghold（用于 api_key 加密）
        //
        // 插件自带的 password hash fn 仅供前端插件 JS API 使用；
        // Rust 端我们走 `crypto::init` 自己管理一份 `Stronghold`，
        // snapshot 落到 app_data_dir/stronghold.hold。
        //
        // hash fn 同样走 blake2b256 派生到 32 字节，避免前端 JS API
        // 触发与 Rust 侧一样的 "illegal non-contiguous size" 错误。
        // （Phase 2 切 OS keyring 时，hash fn 与 Rust 侧可共用同一份 passphrase 源。）
        .plugin(tauri_plugin_stronghold::Builder::new(|pwd| {
            crypto::derive_stronghold_key(pwd.as_bytes()).to_vec()
        }).build())
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
