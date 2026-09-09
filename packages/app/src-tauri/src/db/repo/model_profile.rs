//! `model_profiles` 表的 SQL 操作（模型配置实体，migration 49）
//!
//! 约定（同 `agent.rs`）：
//! - `api_key_ref` 仅作引用 key（默认 = `profile:{id}`），密文存在 stronghold 里
//! - 上层负责调用 stronghold 写入/删除；本模块只更新 DB 这一行

use sqlx::SqlitePool;

use crate::db::models::ModelProfileRow;
use crate::error::{AppError, AppResult};

/// 列出全部 profile，按 sort_order asc, created_at asc
pub async fn list(pool: &SqlitePool) -> AppResult<Vec<ModelProfileRow>> {
    let rows = sqlx::query_as::<_, ModelProfileRow>(
        "SELECT id, alias, provider, model, api_key_ref, base_url,
                sort_order, created_at, updated_at,
                last_health, last_health_detail, last_health_at
           FROM model_profiles
          ORDER BY sort_order ASC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 按 id 取一条；找不到返回 `AppError::NotFound`
pub async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<ModelProfileRow> {
    let row = sqlx::query_as::<_, ModelProfileRow>(
        "SELECT id, alias, provider, model, api_key_ref, base_url,
                sort_order, created_at, updated_at,
                last_health, last_health_detail, last_health_at
           FROM model_profiles WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "model_profile",
        id: id.to_string(),
    })?;
    Ok(row)
}

/// 创建 profile。id 与 api_key_ref 由调用方生成（api_key_ref 恒 = `profile:{id}`）。
/// `insert_ignore`：id 冲突时静默跳过（boot 迁移幂等基石——确定性 id 重写无害）。
pub async fn create(pool: &SqlitePool, row: &NewProfileRow, insert_ignore: bool) -> AppResult<()> {
    let sql = if insert_ignore {
        "INSERT OR IGNORE INTO model_profiles
           (id, alias, provider, model, api_key_ref, base_url, sort_order)
         VALUES (?, ?, ?, ?, ?, ?, ?)"
    } else {
        "INSERT INTO model_profiles
           (id, alias, provider, model, api_key_ref, base_url, sort_order)
         VALUES (?, ?, ?, ?, ?, ?, ?)"
    };
    sqlx::query(sql)
        .bind(&row.id)
        .bind(&row.alias)
        .bind(&row.provider)
        .bind(&row.model)
        .bind(&row.api_key_ref)
        .bind(row.base_url.as_deref())
        .bind(row.sort_order)
        .execute(pool)
        .await?;
    Ok(())
}

/// 创建入参（repo 层形状：id/api_key_ref 已由命令层或迁移器生成）
pub struct NewProfileRow {
    pub id: String,
    pub alias: String,
    pub provider: String,
    pub model: String,
    pub api_key_ref: String,
    pub base_url: Option<String>,
    pub sort_order: i32,
}

/// 部分更新（None 字段不动；先读后合并，同 agent.rs 模式）
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    alias: Option<&str>,
    provider: Option<&str>,
    model: Option<&str>,
    base_url: Option<Option<&str>>,
    sort_order: Option<i32>,
) -> AppResult<ModelProfileRow> {
    let mut current = get_by_id(pool, id).await?;
    if let Some(v) = alias {
        current.alias = v.to_string();
    }
    if let Some(v) = provider {
        current.provider = v.to_string();
    }
    if let Some(v) = model {
        current.model = v.to_string();
    }
    if let Some(v) = base_url {
        current.base_url = v.map(String::from);
    }
    if let Some(v) = sort_order {
        current.sort_order = v;
    }
    sqlx::query(
        "UPDATE model_profiles
            SET alias = ?, provider = ?, model = ?, base_url = ?, sort_order = ?
          WHERE id = ?",
    )
    .bind(&current.alias)
    .bind(&current.provider)
    .bind(&current.model)
    .bind(&current.base_url)
    .bind(current.sort_order)
    .bind(id)
    .execute(pool)
    .await?;
    get_by_id(pool, id).await
}

/// 删除 profile 行（DB 行删除；Stronghold 槽位清理由命令层负责）
pub async fn delete(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let affected = sqlx::query("DELETE FROM model_profiles WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "model_profile",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 引用该 profile 的 agent（ModelProfile Phase 2 删除守卫 agent 腿）。
///
/// 主档引用（`model_profile_id = ?`）精确匹配；降级链（`fallback_profile_ids`
/// JSON 数组串）用 LIKE `"id"` 带引号匹配防前缀碰撞（`"mp-1"` 不会撞 `"mp-10"`）。
/// 返回 (id, name) 供守卫文案点名。
pub async fn agents_referencing(
    pool: &SqlitePool,
    profile_id: &str,
) -> AppResult<Vec<(String, String)>> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT id, name FROM agents
          WHERE model_profile_id = ?
             OR (fallback_profile_ids IS NOT NULL AND fallback_profile_ids LIKE ?)",
    )
    .bind(profile_id)
    .bind(format!("%\"{profile_id}\"%"))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 记录健康状态（状态监控）：slug 见 `harness::profile_health::ProfileHealth`。
///
/// 行不存在（记录瞬间 profile 已删的竞态）静默 Ok——状态是旁路数据不构成错误。
/// 只碰健康三列：migration 50 起的 updated_at 触发器按配置字段 WHEN 门控，
/// 此写入不伪造「编辑过」。
pub async fn record_health(
    pool: &SqlitePool,
    id: &str,
    health: &str,
    detail: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE model_profiles
            SET last_health = ?, last_health_detail = ?, last_health_at = datetime('now')
          WHERE id = ?",
    )
    .bind(health)
    .bind(detail)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// in-memory SQLite + 全量 migrations
    async fn test_pool() -> SqlitePool {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        pool
    }

    fn row(id: &str, alias: &str) -> NewProfileRow {
        NewProfileRow {
            id: id.into(),
            alias: alias.into(),
            provider: "glm".into(),
            model: "glm-5.3-flash".into(),
            api_key_ref: format!("profile:{id}"),
            base_url: None,
            sort_order: 0,
        }
    }

    #[tokio::test]
    async fn crud_roundtrip() {
        let pool = test_pool().await;
        create(&pool, &row("mp1", "智谱主力"), false)
            .await
            .expect("create");
        let got = get_by_id(&pool, "mp1").await.expect("get");
        assert_eq!(got.alias, "智谱主力");
        assert_eq!(got.api_key_ref, "profile:mp1");

        // partial update：None 不改
        let updated = update(&pool, "mp1", Some("智谱备用"), None, None, None, None)
            .await
            .expect("update alias");
        assert_eq!(updated.alias, "智谱备用");
        assert_eq!(updated.provider, "glm");

        // list 按 sort_order 排序
        create(
            &pool,
            &NewProfileRow {
                id: "mp2".into(),
                alias: "排在前面".into(),
                provider: "openai".into(),
                model: "gpt-4o".into(),
                api_key_ref: "profile:mp2".into(),
                base_url: None,
                sort_order: -1,
            },
            false,
        )
        .await
        .expect("create 2");
        let all = list(&pool).await.expect("list");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "mp2");

        delete(&pool, "mp1").await.expect("delete");
        assert!(get_by_id(&pool, "mp1").await.is_err());
    }

    #[tokio::test]
    async fn base_url_double_option_semantics() {
        let pool = test_pool().await;
        create(&pool, &row("mp1", "a"), false)
            .await
            .expect("create");

        // Some(Some) = 设定
        let r = update(
            &pool,
            "mp1",
            None,
            None,
            None,
            Some(Some("https://x.example")),
            None,
        )
        .await
        .expect("set");
        assert_eq!(r.base_url.as_deref(), Some("https://x.example"));

        // None = 不改
        let r = update(&pool, "mp1", None, None, None, None, None)
            .await
            .expect("keep");
        assert_eq!(r.base_url.as_deref(), Some("https://x.example"));

        // Some(None) = 清空
        let r = update(&pool, "mp1", None, None, None, Some(None), None)
            .await
            .expect("clear");
        assert_eq!(r.base_url, None);
    }

    /// boot 迁移幂等基石：确定性 id + INSERT OR IGNORE 重入无害
    #[tokio::test]
    async fn insert_ignore_idempotent() {
        let pool = test_pool().await;
        create(&pool, &row("mp-vision-0", "视觉主模型"), true)
            .await
            .expect("first");
        create(&pool, &row("mp-vision-0", "视觉主模型"), true)
            .await
            .expect("re-run no-op");
        let all = list(&pool).await.expect("list");
        assert_eq!(all.len(), 1);
    }

    /// 健康三列读写 + updated_at 触发器门控（状态写入不伪造「编辑过」）
    #[tokio::test]
    async fn record_health_updates_columns_without_touching_updated_at() {
        let pool = test_pool().await;
        create(&pool, &row("mp1", "智谱主力"), false)
            .await
            .expect("create");
        let before = get_by_id(&pool, "mp1").await.expect("get");
        assert_eq!(before.last_health, None, "新行未调用态 = NULL");

        // 睡 1s 保证 datetime('now') 秒级粒度能区分（同秒内触发器刷了也测不出）
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        record_health(&pool, "mp1", "quota", Some("余额不足或无可用资源包"))
            .await
            .expect("record");
        let after = get_by_id(&pool, "mp1").await.expect("get after");
        assert_eq!(after.last_health.as_deref(), Some("quota"));
        assert_eq!(
            after.last_health_detail.as_deref(),
            Some("余额不足或无可用资源包")
        );
        assert!(after.last_health_at.is_some(), "调用时间应落库");
        assert_eq!(
            after.updated_at, before.updated_at,
            "纯健康写入不应刷 updated_at（触发器按配置字段 WHEN 门控）"
        );

        // 配置字段更新仍正常刷 updated_at
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        update(&pool, "mp1", Some("改名"), None, None, None, None)
            .await
            .expect("rename");
        let renamed = get_by_id(&pool, "mp1").await.expect("get renamed");
        assert_ne!(
            renamed.updated_at, after.updated_at,
            "配置更新应刷 updated_at"
        );
        assert_eq!(
            renamed.last_health.as_deref(),
            Some("quota"),
            "配置更新不碰健康列"
        );

        // 行不存在（已删竞态）静默 Ok
        record_health(&pool, "ghost", "ok", None)
            .await
            .expect("ghost no-op");
    }

    /// 删除守卫 agent 腿（Phase 2）：主档引用 / 降级链命中 + 前缀不碰撞。
    #[tokio::test]
    async fn agents_referencing_covers_primary_and_fallback() {
        let pool = test_pool().await;
        // 最小 agents 行（守卫查询只读 id/name/两引用列）
        async fn seed(
            pool: &SqlitePool,
            id: &str,
            name: &str,
            primary: Option<&str>,
            fb: Option<&str>,
        ) {
            let q = format!(
                "INSERT INTO agents (id, name, provider, model, api_key_ref, model_profile_id, fallback_profile_ids)
                 VALUES ('{id}', '{name}', 'glm', 'm', 'x', {primary}, {fb})",
                primary = primary.map(|p| format!("'{p}'")).unwrap_or_else(|| "NULL".into()),
                fb = fb.map(|f| format!("'{f}'")).unwrap_or_else(|| "NULL".into()),
            );
            sqlx::query(&q).execute(pool).await.unwrap();
        }
        seed(&pool, "a1", "主档引用", Some("mp1"), None).await;
        seed(&pool, "a2", "链中引用", None, Some(r#"["mp9","mp1"]"#)).await;
        seed(
            &pool,
            "a3",
            "无关行",
            Some("mp10"),
            Some(r#"["mp10","mp11"]"#),
        )
        .await;
        seed(&pool, "a4", "legacy", None, None).await;

        let hits = agents_referencing(&pool, "mp1").await.expect("query");
        let mut names: Vec<&str> = hits.iter().map(|(_, n)| n.as_str()).collect();
        names.sort();
        // 前缀不碰撞：mp10/mp11 的行（a3）不得命中 mp1
        assert_eq!(names, vec!["主档引用", "链中引用"]);

        // 查 mp10 → 只命中 a3
        let hits = agents_referencing(&pool, "mp10").await.expect("query");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "无关行");

        // 无引用 → 空
        assert!(agents_referencing(&pool, "mp-nope")
            .await
            .unwrap()
            .is_empty());
    }

    /// 升级路径锁定（2026-09-08 实案）：dev 真机库已应用**旧版 49**（无健康
    /// 三列、无 WHEN 触发器，boot 迁移已落存量数据），健康三列后置进 50——
    /// 模拟旧库经 boot 同款次序（heal → run）升级后的终态：三列出现、存量行
    /// 原样、触发器已换 WHEN 门控版（健康写入不刷 updated_at）。
    #[tokio::test]
    async fn migration_50_upgrades_legacy_49_database() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;

        // 裸库：手建旧版 49 schema（与真机 sqlite_master 逐语句一致）
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE model_profiles (
              id TEXT PRIMARY KEY,
              alias TEXT NOT NULL,
              provider TEXT NOT NULL,
              model TEXT NOT NULL,
              api_key_ref TEXT NOT NULL,
              base_url TEXT,
              sort_order INTEGER NOT NULL DEFAULT 0,
              created_at TEXT NOT NULL DEFAULT (datetime('now')),
              updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        // agents 表（01_init 摘录，49 前的形状）：migration 51 起 ALTER agents，
        // 伪造库须有此表升级路径才跑得通（run 只看登记不看 schema，但 51 真执行）。
        sqlx::query(
            "CREATE TABLE agents (
              id TEXT PRIMARY KEY,
              name TEXT NOT NULL,
              provider TEXT NOT NULL,
              model TEXT NOT NULL,
              system_prompt TEXT NOT NULL DEFAULT '',
              api_key_ref TEXT NOT NULL,
              base_url TEXT,
              temperature REAL NOT NULL DEFAULT 0.7,
              max_tokens INTEGER NOT NULL DEFAULT 4096,
              extra_params TEXT NOT NULL DEFAULT '{}',
              sort_order INTEGER NOT NULL DEFAULT 0,
              created_at TEXT NOT NULL DEFAULT (datetime('now')),
              updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        // conversations 表（01_init 最小形状）：migration 52 起 ALTER conversations
        // 加 inbox_policy，伪造库须有此表（同上：run 只看登记，但 52 真执行）。
        sqlx::query(
            "CREATE TABLE conversations (
              id TEXT PRIMARY KEY,
              agent_id TEXT NOT NULL,
              title TEXT NOT NULL DEFAULT '',
              pinned INTEGER NOT NULL DEFAULT 0,
              created_at TEXT NOT NULL DEFAULT (datetime('now')),
              updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("CREATE INDEX idx_model_profiles_sort ON model_profiles(sort_order ASC, created_at ASC)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER trg_model_profiles_upd
               AFTER UPDATE ON model_profiles
               BEGIN
                 UPDATE model_profiles SET updated_at = datetime('now') WHERE id = NEW.id;
               END",
        )
        .execute(&pool)
        .await
        .unwrap();

        // 伪造「1..49 已应用」登记（checksum 乱填 = heal 自愈对象）。run() 只对
        // 未登记的 50 执行，1..48 的表不存在无妨——run 不校验 schema 只看登记。
        sqlx::query(
            "CREATE TABLE _sqlx_migrations (
                version BIGINT PRIMARY KEY,
                description TEXT NOT NULL,
                installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                success BOOLEAN NOT NULL,
                checksum BLOB NOT NULL,
                execution_time BIGINT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        for v in 1..=49i64 {
            sqlx::query(
                "INSERT INTO _sqlx_migrations
                   (version, description, installed_on, success, checksum, execution_time)
                 VALUES (?, 'legacy', '2026-09-08 00:00:00', 1, X'00', 0)",
            )
            .bind(v)
            .execute(&pool)
            .await
            .unwrap();
        }

        // 存量数据（真机 boot 迁移落的形状，created_at/updated_at 固定便于断言）
        sqlx::query(
            "INSERT INTO model_profiles
               (id, alias, provider, model, api_key_ref, base_url, sort_order, created_at, updated_at)
             VALUES ('mp-vision-0', '视觉主模型', 'glm', 'glm-5.3-flash', 'profile:mp-vision-0',
                     'https://x.example/v1', 0, '2026-09-08 03:22:12', '2026-09-08 03:22:12')",
        )
        .execute(&pool)
        .await
        .unwrap();

        // 模拟 boot 次序（db/mod.rs 同款）：heal 漂移 → heal 缺席 → run
        let migrator = sqlx::migrate!("./src/db/migrations");
        crate::db::migrate::heal_checksum_drift(&pool, &migrator).await;
        crate::db::migrate::heal_dropped_migrations(&pool, &migrator).await;
        migrator
            .run(&pool)
            .await
            .expect("旧 49 库升级应通过（50 加三列）");

        // 终态：三列出现 + 存量行原样（未调用 NULL 态 + 时间戳不动）
        let row = get_by_id(&pool, "mp-vision-0").await.unwrap();
        assert_eq!(row.alias, "视觉主模型");
        assert_eq!(row.last_health, None, "升级不虚构健康状态");
        assert_eq!(row.updated_at, "2026-09-08 03:22:12");

        // 51（agent 引用两列）同轮生效——查询成功本身即证明两列已在
        let n: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM agents WHERE model_profile_id IS NOT NULL OR fallback_profile_ids IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n.0, 0, "空表无引用行；查询成功即证明 51 两列已升级");

        // 触发器已换 WHEN 门控版：健康写入落三列但不刷 updated_at
        record_health(&pool, "mp-vision-0", "ok", None)
            .await
            .unwrap();
        let after = get_by_id(&pool, "mp-vision-0").await.unwrap();
        assert_eq!(after.last_health.as_deref(), Some("ok"));
        assert!(after.last_health_at.is_some());
        assert_eq!(
            after.updated_at, "2026-09-08 03:22:12",
            "健康写入不应刷 updated_at（50 换上的 WHEN 门控生效）"
        );
    }
}
