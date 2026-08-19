//! 启动时数据迁移：修复历史坏数据。
//!
//! tool_result 持久化彻底重构前，一次工具轮次的 `tool_use` + `tool_result` 被
//! 合并存进**同一条 assistant 消息**的 `content_blocks`（违反 Anthropic 协议——
//! tool_result 必须在 user 消息里）。`fix_orphan_tool_results` 在 `init_pool`
//! 跑完 sqlx 迁移后幂等地把 tool_result 拆成独立 user 消息。
//!
//! 迁移策略（幂等）：
//! 1. 扫描所有 `content_blocks` 含 `tool_result` 的 assistant 消息
//! 2. 对每条：partition 出 ToolResult 块 → 新建 role=user 消息存放（`created_at`
//!    沿用原 assistant，靠 rowid tie-break 排在其后）→ 原 assistant 去掉 ToolResult
//! 3. 再跑时 assistant 已无 ToolResult → no-op
//!
//! 排序说明：消息列表按 `(created_at ASC, rowid ASC)` 输出。新 user 消息 rowid
//! 大于原 assistant、created_at 相同，因此排在原 assistant 之后；下一条 assistant
//! 通常 created_at 更晚（多轮 LLM 生成耗时 > 1s），故 user 排在它之前 —— 顺序正确。

use std::path::Path;

use sqlx::migrate::Migrator;
use sqlx::SqlitePool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::AppResult;
use crate::infra::protocol::ContentBlock;

/// 启动时幂等迁移：把 assistant 消息里混入的 tool_result 拆成独立 user 消息。
///
/// 见模块级文档。`db_path` 用于在确有孤儿时先备份 DB（WAL checkpoint 后复制）。
pub async fn fix_orphan_tool_results(pool: &SqlitePool, db_path: &Path) -> AppResult<()> {
    // LIKE 快速过滤；JSON 里 ToolResult 的 type 字段是 "tool_result"
    let orphans: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, conversation_id, content_blocks, created_at
           FROM messages
          WHERE role = 'assistant'
            AND content_blocks LIKE '%tool_result%'",
    )
    .fetch_all(pool)
    .await?;

    if orphans.is_empty() {
        info!(target: "ice_paw.migrate", "无需迁移：未发现 tool_result 孤儿");
        return Ok(());
    }

    // 备份：先 checkpoint WAL（确保 -wal 内容落盘），再复制 db 文件
    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await;
    let backup = db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("ice-paw.db.pre-tool-result-migration.bak");
    match std::fs::copy(db_path, &backup) {
        Ok(n) => info!(
            target: "ice_paw.migrate",
            "已备份数据库到 {} ({} 字节)，开始 tool_result 孤儿迁移",
            backup.display(),
            n
        ),
        Err(e) => warn!(
            target: "ice_paw.migrate",
            "备份数据库失败: {}，仍继续迁移（建议手动备份）",
            e
        ),
    }

    let mut fixed = 0usize;
    for (msg_id, conv_id, blocks_json, created_at) in orphans {
        let Ok(blocks) = serde_json::from_str::<Vec<ContentBlock>>(&blocks_json) else {
            warn!(
                target: "ice_paw.migrate",
                "解析 content_blocks 失败，跳过: msg_id={}",
                msg_id
            );
            continue;
        };
        let (asst_blocks, result_blocks): (Vec<ContentBlock>, Vec<ContentBlock>) = blocks
            .into_iter()
            .partition(|b| !matches!(b, ContentBlock::ToolResult { .. }));
        if result_blocks.is_empty() {
            // LIKE 误匹配（如文本内容含 "tool_result" 字样）→ 跳过
            continue;
        }

        // 原子化：INSERT user(tool_result) + UPDATE assistant 去 tool_result 包进一个事务，
        // 避免崩溃在两步之间 → 下次启动 LIKE 仍命中 assistant → 重复插 user 行。
        let user_id = Uuid::new_v4().to_string();
        let result_json = serde_json::to_string(&result_blocks).unwrap_or_else(|_| "[]".into());
        let asst_json = serde_json::to_string(&asst_blocks).unwrap_or_else(|_| "[]".into());
        let tx_result: Result<(), sqlx::Error> = async {
            let mut tx = pool.begin().await?;
            sqlx::query(
                "INSERT INTO messages
                    (id, conversation_id, role, content, content_blocks, token_count, error, model, created_at)
                 VALUES (?, ?, 'user', '', ?, NULL, NULL, NULL, ?)",
            )
            .bind(&user_id)
            .bind(&conv_id)
            .bind(&result_json)
            .bind(&created_at)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE messages SET content_blocks = ? WHERE id = ?")
                .bind(&asst_json)
                .bind(&msg_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        if let Err(e) = tx_result {
            warn!(
                target: "ice_paw.migrate",
                "迁移事务失败（已回滚，下次启动重试）: msg_id={}, err={}",
                msg_id,
                e
            );
            continue;
        }
        fixed += 1;
    }

    info!(
        target: "ice_paw.migrate",
        "tool_result 孤儿迁移完成：修复 {} 条 assistant 消息",
        fixed
    );
    Ok(())
}

/// 启动时自愈 `_sqlx_migrations` 表的 checksum 漂移。
///
/// **背景**：历史安装包曾把 dev 工作区的未提交 migration 改动打进包（如某 migration
/// 的注释/空白调整），用户机器 db 记录了那个改动版的 checksum；新版包改回 git commit
/// 正版（checksum 不同）→ `sqlx::migrate!().run()` 校验失败报
/// "migration N was previously applied but has been modified" → `panic=abort` 闪退
/// （生产 release profile panic=abort，setup hook Err 直接退出无回溯）。
///
/// 本函数在 `migrate!().run()` **之前**跑：对 db 里每个已 apply 的 migration，若其
/// checksum 与编译进二进制的正版不一致，同步为正版。schema 实际一致（同一
/// ALTER/CREATE 语句，仅文本字节差），**不重新执行 SQL**，安全。首次安装（无
/// `_sqlx_migrations` 表）直接跳过——交给 `migrate!().run()` 全新建。
///
/// **风险**：若未来真有人改了已发布 migration 的 schema 语义（违反不可变原则），本
/// 函数会掩盖该不一致。故仍须坚守「已发布 migration 文件不可变，要改 schema 加新
/// migration」纪律。本函数只治「文本字节漂移」（无害），不掩盖 schema 语义变化。
pub async fn heal_checksum_drift(pool: &SqlitePool, migrator: &Migrator) {
    // 首次安装：_sqlx_migrations 尚不存在（migrate!().run() 会建），无需自愈
    let table_exists: Option<(i64,)> = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let exists = table_exists.map(|(c,)| c > 0).unwrap_or(false);
    if !exists {
        return;
    }

    let mut healed = 0u32;
    for m in migrator.migrations.iter() {
        let row: Option<(Vec<u8>,)> =
            sqlx::query_as("SELECT checksum FROM _sqlx_migrations WHERE version = ?")
                .bind(m.version)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();
        let Some((db_ck,)) = row else { continue };
        if db_ck.as_slice() == m.checksum.as_ref() {
            continue;
        }
        warn!(
            target: "ice_paw.migrate",
            "migration {} checksum 漂移（历史包污染），自愈同步为 commit 正版",
            m.version
        );
        let _ = sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
            .bind(m.checksum.as_ref())
            .bind(m.version)
            .execute(pool)
            .await;
        healed += 1;
    }
    if healed > 0 {
        info!(
            target: "ice_paw.migrate",
            "checksum 自愈完成：同步 {} 个 migration（schema 不变，仅修字节漂移）",
            healed
        );
    }
}

/// 启动时自愈 `_sqlx_migrations` 表的「已 apply 但解析集缺席」记录。
///
/// **背景**：未发布（未进安装包）的 migration 在开发期可能被删除/改号——本地 dev
/// 库已 apply 过该版本，新版二进制的解析集里没有它 → `sqlx::migrate!().run()` 报
/// "migration N was previously applied but is missing in the resolved migrations"
/// → boot 闪退（实例：migration 47 agent emoji 列随 emoji 档移除而删，2026-08-19）。
///
/// 本函数在 `migrate!().run()` **之前**跑：删除 `_sqlx_migrations` 里版本号不在
/// 编译集内的行。**只删登记记录，不触碰 schema**——本仓库 migration 全是
/// append-only（ALTER ADD COLUMN / CREATE TABLE），缺席版本意味着 db 里多一个
/// 惰性列/表，SELECT 均显式列名，无害；不会出现「缺列」（那来自「db 没有 + 二进制
/// 有」的反向情形，由 run() 正常补跑）。全新安装无缺席记录，no-op。
///
/// **风险**：若未来误删**已发布** migration 文件，本函数会把 db 登记记录一并抹掉、
/// 掩盖事故（schema 残留但不报错）。缓解：缺席删除必打 warn 日志（版本号可追溯），
/// 且发布纪律本身要求已发布 migration 不可变；两害相权（闪退 all users vs 静默
/// 残留 schema），取自愈。
pub async fn heal_dropped_migrations(pool: &SqlitePool, migrator: &Migrator) {
    // 首次安装：_sqlx_migrations 尚不存在（migrate!().run() 会建），无需自愈
    let table_exists: Option<(i64,)> = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let exists = table_exists.map(|(c,)| c > 0).unwrap_or(false);
    if !exists {
        return;
    }

    let applied: Vec<(i64,)> = sqlx::query_as("SELECT version FROM _sqlx_migrations")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    let dropped: Vec<i64> = applied
        .into_iter()
        .map(|(v,)| v)
        .filter(|v| !migrator.migrations.iter().any(|m| m.version == *v))
        .collect();
    if dropped.is_empty() {
        return;
    }
    for v in &dropped {
        warn!(
            target: "ice_paw.migrate",
            "migration {} 已 apply 但二进制解析集缺席（未发布 migration 被删），清除登记记录（schema 残留惰性无害）",
            v
        );
        let _ = sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ?")
            .bind(v)
            .execute(pool)
            .await;
    }
    info!(
        target: "ice_paw.migrate",
        "缺席 migration 自愈完成：清除 {} 个登记记录（{:?}）",
        dropped.len(),
        dropped
    );
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    async fn seed_conv(pool: &SqlitePool) {
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("a1").bind("t").bind("anthropic").bind("claude")
        .bind("").bind("").bind(0.7).bind(1024).bind("{}").bind(0).bind(0)
        .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES (?, ?, ?)")
            .bind("c1")
            .bind("a1")
            .bind("t")
            .execute(pool)
            .await
            .unwrap();
    }

    /// 插入一条 assistant 消息，content_blocks 由调用方指定
    async fn insert_asst(pool: &SqlitePool, id: &str, blocks_json: &str) {
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, role, content, content_blocks, model)
             VALUES (?, 'c1', 'assistant', '', ?, 'claude')",
        )
        .bind(id)
        .bind(blocks_json)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn fetch_blocks(pool: &SqlitePool, id: &str) -> Vec<ContentBlock> {
        let row: (String,) = sqlx::query_as("SELECT content_blocks FROM messages WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        serde_json::from_str(&row.0).unwrap_or_default()
    }

    #[tokio::test]
    async fn migrates_tool_result_out_of_assistant() {
        let pool = fresh_pool().await;
        seed_conv(&pool).await;
        // 坏数据：assistant 同时含 tool_use + tool_result
        insert_asst(
            &pool,
            "m1",
            r#"[{"type":"tool_use","id":"tu1","name":"read","input":"{}"},
               {"type":"tool_result","tool_use_id":"tu1","content":"ok","is_error":false}]"#,
        )
        .await;

        fix_orphan_tool_results(&pool, std::path::Path::new("/tmp/nonexistent.db"))
            .await
            .unwrap();

        // 原 assistant 只剩 tool_use
        let asst = fetch_blocks(&pool, "m1").await;
        assert_eq!(asst.len(), 1, "assistant 应只剩 tool_use");
        assert!(matches!(asst[0], ContentBlock::ToolUse { .. }));

        // 新增 user 消息含 tool_result
        let user_row: (String, String) = sqlx::query_as(
            "SELECT role, content_blocks FROM messages WHERE role='user' AND content_blocks LIKE '%tool_result%'",
        )
        .fetch_one(&pool).await.unwrap();
        assert_eq!(user_row.0, "user");
        let user_blocks: Vec<ContentBlock> = serde_json::from_str(&user_row.1).unwrap();
        assert_eq!(user_blocks.len(), 1);
        assert!(matches!(user_blocks[0], ContentBlock::ToolResult { .. }));
    }

    #[tokio::test]
    async fn idempotent_second_run_is_noop() {
        let pool = fresh_pool().await;
        seed_conv(&pool).await;
        insert_asst(
            &pool,
            "m1",
            r#"[{"type":"tool_use","id":"tu1","name":"read","input":"{}"},
               {"type":"tool_result","tool_use_id":"tu1","content":"ok","is_error":false}]"#,
        )
        .await;

        fix_orphan_tool_results(&pool, std::path::Path::new("/tmp/nonexistent.db"))
            .await
            .unwrap();
        // 第二次：应 no-op（assistant 已无 tool_result）
        fix_orphan_tool_results(&pool, std::path::Path::new("/tmp/nonexistent.db"))
            .await
            .unwrap();

        // user 消息仍然只有 1 条（没重复创建）
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM messages WHERE role='user' AND content_blocks LIKE '%tool_result%'",
        )
        .fetch_one(&pool).await.unwrap();
        assert_eq!(count.0, 1, "第二次迁移不应重复创建 user 消息");
    }

    #[tokio::test]
    async fn no_orphans_is_noop() {
        let pool = fresh_pool().await;
        seed_conv(&pool).await;
        // 干净的 assistant（纯文本 + tool_use，无 tool_result）
        insert_asst(
            &pool,
            "m1",
            r#"[{"type":"text","text":"hi"},{"type":"tool_use","id":"tu1","name":"x","input":"{}"}]"#,
        )
        .await;

        fix_orphan_tool_results(&pool, std::path::Path::new("/tmp/nonexistent.db"))
            .await
            .unwrap();

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE role='user'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "无孤儿时不应创建任何 user 消息");
    }

    #[tokio::test]
    async fn heal_checksum_drift_repairs_modified_migration() {
        let pool = fresh_pool().await;
        let migrator = sqlx::migrate!("./src/db/migrations");
        // 模拟历史包污染：篡改 migration 24 的 checksum（与 commit 正版不同）
        sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 24")
            .execute(&pool)
            .await
            .unwrap();
        // 自愈后应恢复为 commit 正版（否则 migrate!().run() 二次校验会 panic）
        heal_checksum_drift(&pool, &migrator).await;
        let row: (Vec<u8>,) =
            sqlx::query_as("SELECT checksum FROM _sqlx_migrations WHERE version = 24")
                .fetch_one(&pool)
                .await
                .unwrap();
        let m24 = migrator
            .migrations
            .iter()
            .find(|m| m.version == 24)
            .expect("migration 24 应存在");
        assert_eq!(
            row.0.as_slice(),
            m24.checksum.as_ref(),
            "自愈后 migration 24 checksum 应恢复为 commit 正版"
        );
    }

    #[tokio::test]
    async fn heal_checksum_drift_noop_on_fresh_db() {
        // fresh_pool 已对全新 db 跑过 migrate，_sqlx_migrations 全是正版 checksum，
        // 自愈应 no-op（checksum 不变即可）
        let pool = fresh_pool().await;
        let migrator = sqlx::migrate!("./src/db/migrations");
        let before: (Vec<u8>,) =
            sqlx::query_as("SELECT checksum FROM _sqlx_migrations WHERE version = 24")
                .fetch_one(&pool)
                .await
                .unwrap();
        heal_checksum_drift(&pool, &migrator).await;
        let after: (Vec<u8>,) =
            sqlx::query_as("SELECT checksum FROM _sqlx_migrations WHERE version = 24")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(before.0, after.0, "干净 db 自愈不应改动 checksum");
    }

    #[tokio::test]
    async fn heal_dropped_migrations_clears_missing_version_records() {
        // 复现 2026-08-19 实况：migration 47 被删后启动即
        // "migration 47 was previously applied but is missing in the resolved migrations"
        let pool = fresh_pool().await;
        let migrator = sqlx::migrate!("./src/db/migrations");
        sqlx::query("INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time)
                     VALUES (47, 'ghost', '2026-08-19 00:00:00', 1, X'00', 0)")
            .execute(&pool)
            .await
            .unwrap();

        heal_dropped_migrations(&pool, &migrator).await;

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 47")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "缺席版本的登记记录应被清除");
        // 自愈后 run() 不再报 missing（这是 boot 不闪退的行为锁）
        migrator.run(&pool).await.expect("自愈后 run 应通过");
    }

    #[tokio::test]
    async fn heal_dropped_migrations_noop_when_all_present() {
        let pool = fresh_pool().await;
        let migrator = sqlx::migrate!("./src/db/migrations");
        let before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap();
        heal_dropped_migrations(&pool, &migrator).await;
        let after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(before.0, after.0, "无缺席记录时不应改动 _sqlx_migrations");
    }
}
