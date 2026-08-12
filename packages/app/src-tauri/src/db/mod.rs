//! 数据库模块入口
//!
//! - `init_pool`：启动时初始化连接池，跑迁移，注入到 `tauri::State`
//! - 提供 `get_pool` 给非 setup 钩子处的代码借用

pub mod migrate;
pub mod models;
pub mod repo;

use std::path::PathBuf;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};
use tracing::{info, warn};

use crate::error::{AppError, AppResult};

/// 在 app 数据目录下创建（若不存在）`ice-paw.db`，返回连接池
/// 同时：
/// - 设置 `PRAGMA foreign_keys = ON`
/// - 设置 `synchronous = NORMAL`（性能/安全平衡）
/// - 启动时执行 `sqlx::migrate!()` 自动建表
pub async fn init_pool(app: &AppHandle) -> AppResult<SqlitePool> {
    // 1) 解析 app 数据目录
    let data_dir: PathBuf = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Tauri(format!("解析 app_data_dir 失败: {e}")))?;
    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("ice-paw.db");
    info!(target: "ice_paw.db", "SQLite 文件: {}", db_path.display());

    // 2) 构造连接选项
    let db_url = format!("sqlite://{}", db_path.display());
    let conn_opts = SqliteConnectOptions::from_str(&db_url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .synchronous(SqliteSynchronous::Normal)
        .journal_mode(SqliteJournalMode::Wal);

    // 3) 创建连接池（最多 5 个连接，足以支撑前端并发命令）
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect_with(conn_opts)
        .await?;

    // 4) 显式开启外键（双重保险，连接选项已开，但有些平台会失效）
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await?;
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await?;
    if fk != 1 {
        warn!(target: "ice_paw.db", "foreign_keys 未启用，fk={}", fk);
    }

    // 5) 跑迁移（V1__init.sql 等）
    // 路径相对 Cargo.toml（src-tauri）
    // 先自愈 _sqlx_migrations checksum 漂移：历史包曾把未提交的 migration 改动打进包，
    // 用户 db 记录的 checksum 与 commit 正版不一致会让 migrate!().run() 校验失败
    // → panic=abort 闪退。自愈在 run() 前同步 checksum（schema 不变，仅修字节漂移）。
    let migrator = sqlx::migrate!("./src/db/migrations");
    migrate::heal_checksum_drift(&pool, &migrator).await;
    migrator.run(&pool).await?;
    info!(target: "ice_paw.db", "数据库迁移完成");

    // 5.5) tool_result 孤儿迁移：把历史坏数据（tool_result 混进 assistant 消息）
    //      幂等地拆成独立 user 消息。失败不阻止启动（仅 warn）。
    if let Err(e) = migrate::fix_orphan_tool_results(&pool, &db_path).await {
        warn!(
            target: "ice_paw.db",
            "tool_result 孤儿迁移失败（不影响启动）: {}",
            e
        );
    }

    // 6) 注入到 tauri 状态（app 持有 + 返回给 setup 钩子）
    app.manage(pool.clone());

    Ok(pool)
}

/// 从 `AppHandle` 取连接池（已 manage 过的）
pub fn get_pool(app: &AppHandle) -> AppResult<SqlitePool> {
    Ok(app.state::<SqlitePool>().inner().clone())
}
