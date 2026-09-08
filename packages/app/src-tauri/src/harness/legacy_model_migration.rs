//! 旧模型配置 → ModelProfile 实体迁移（ModelProfile Phase 1，boot 幂等）。
//!
//! ## 为什么
//!
//! 视觉读取（vision_config 条目链 / 旧四键）与语义检索（embedding 四键）的
//! API Key 此前以**明文**存 user_preferences JSON。实体化后两处改为引用
//! model_profiles 行（key 收编 Stronghold 槽位 `profile:{id}`）。本模块在 boot
//! 时把存量旧格式搬进实体——纯软迁移会让明文 key 永远留在 preferences（安全
//! 收益落空），纯硬搬无兜底；混合式最坏情况 = 数据维持旧格式继续工作（读侧
//! `Some=权威 / None=回落` 的软迁移语义），仅无法编辑，日志可查。
//!
//! ## 幂等基石（确定性 id + 全程可重放）
//!
//! - 迁移产物 id 确定性：`mp-vision-{i}`（有效条目在链内的序位）/ `mp-embed`；
//! - Stronghold 重写同槽位无害、DB 行 `INSERT OR IGNORE`、引用键 set 覆写、
//!   旧键 delete 幂等——任意一步崩溃，下次启动整腿重放收敛到同一终态。
//!
//! ## 次序铁律
//!
//! **Stronghold 全部落成功 → DB 行 → 引用键 → 才清旧键**。Stronghold 是唯一
//! 不可从 DB 重建的数据（key 密文只在旧键明文里）——先落它保证中途任何失败
//! 都不产生「引用键已指、key 却没有」的断链 profile（那会让读侧 warn+跳过，
//! 而旧键还没清又能回落，行为不坏但迁移永不收敛）。
//!
//! ## 腿语义（vision 与 embedding 同构）
//!
//! - 引用键已 `Some` → 腿跳过（Some=权威，含显式清空；重迁移 = 覆写用户选择）；
//! - vision_config 为 `Some`（含空数组）→ 腿必跑：显式清空等价迁移为
//!   `Some(vec![])`（语义保真），并顺手清掉被遮蔽的死旧键；
//! - 旧四键形态且字段不齐 → 腿不动（键留着：读侧回落后本就解析为空链/未启用，
//!   贸然清掉反而丢用户的半配置数据）。

use sqlx::SqlitePool;

use crate::db::models::UserPreferences;
use crate::db::repo;
use crate::error::AppResult;

/// 待建的单条 profile（从旧格式提取的字段齐全条目；api_key 明文仅过内存）。
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedProfile {
    pub id: String,
    pub alias: String,
    pub provider: String,
    pub model: String,
    pub api_key: String,
    /// 仅保留用户显式填过的端点；None = 运行时按注册表推导（读侧三层规则不变）
    pub base_url: Option<String>,
    pub sort_order: i32,
}

impl PlannedProfile {
    /// Stronghold 槽位（同 model_profiles 建行惯例：api_key_ref 恒等 `profile:{id}`）
    pub fn api_key_ref(&self) -> String {
        format!("profile:{}", self.id)
    }
}

/// 迁移计划：两条腿各自 `None` = 不动（已迁移 / 旧数据不完整）。
#[derive(Debug, Default, PartialEq)]
pub struct MigrationPlan {
    /// `Some(profiles)` = 视觉腿要跑；**含空列表** = vision_config 显式清空的
    /// 等价迁移（落 `Some(vec![])` 引用键保语义）。有效条目按链内序位取
    /// 确定性 id / 别名（视觉主模型 / 视觉降级{n}）。
    pub vision: Option<Vec<PlannedProfile>>,
    /// `Some(profile)` = embedding 腿要跑（旧四键齐全）
    pub embedding: Option<PlannedProfile>,
}

/// 旧 vision 四键 + 新格式键（迁移成功后清理）
const LEGACY_VISION_KEYS: &[&str] = &[
    "vision_config",
    "vision_provider",
    "vision_model",
    "vision_api_key",
    "vision_base_url",
];

/// 旧 embedding 四键（迁移成功后清理）
const LEGACY_EMBEDDING_KEYS: &[&str] = &[
    "embedding_provider",
    "embedding_model",
    "embedding_api_key",
    "embedding_base_url",
];

/// 条目字段齐全（provider/model/api_key trim 后非空）——与
/// `vision::entry_to_credential` 的字段门一致；端点可推导性交读侧三层解析
/// （不可推导 = warn 跳过，与旧链行为等价且在设置页可见可修）。
fn vision_entry_fields_valid(e: &crate::db::models::VisionConfigEntry) -> bool {
    !e.provider.trim().is_empty()
        && !e.model.trim().is_empty()
        && !e.api_key.trim().is_empty()
}

/// 纯函数：从当前 preferences 推导迁移计划（单测全覆盖；不碰 DB / Stronghold）。
pub fn plan_legacy_migration(prefs: &UserPreferences) -> MigrationPlan {
    // ---- 视觉腿 ----
    let vision = if prefs.vision_profile_ids.is_some() {
        None // 已迁移（Some=权威）→ 不动
    } else {
        let entries: Vec<crate::db::models::VisionConfigEntry> = match &prefs.vision_config {
            Some(entries) => entries.clone(),
            None => vec![crate::db::models::VisionConfigEntry {
                provider: prefs.vision_provider.clone().unwrap_or_default(),
                model: prefs.vision_model.clone().unwrap_or_default(),
                api_key: prefs.vision_api_key.clone().unwrap_or_default(),
                base_url: prefs.vision_base_url.clone(),
            }],
        };
        let profiles: Vec<PlannedProfile> = entries
            .iter()
            .filter(|e| vision_entry_fields_valid(e))
            .enumerate()
            .map(|(i, e)| PlannedProfile {
                id: format!("mp-vision-{i}"),
                alias: if i == 0 {
                    "视觉主模型".into()
                } else {
                    format!("视觉降级{i}")
                },
                provider: e.provider.trim().to_string(),
                model: e.model.trim().to_string(),
                api_key: e.api_key.trim().to_string(),
                base_url: e
                    .base_url
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from),
                sort_order: i as i32,
            })
            .collect();
        // vision_config Some（含空）→ 腿必跑（清死旧键 + 落等价引用键）；
        // 旧四键形态且全无效 → 腿不动（半配置数据留给读侧回落）
        if prefs.vision_config.is_some() || !profiles.is_empty() {
            Some(profiles)
        } else {
            None
        }
    };

    // ---- embedding 腿 ----
    let embedding = if prefs.embedding_profile_id.is_some() {
        None // 已迁移 → 不动
    } else {
        let complete = [
            prefs.embedding_model.as_deref(),
            prefs.embedding_provider.as_deref(),
            prefs.embedding_api_key.as_deref(),
        ]
        .iter()
        .all(|s| s.is_some_and(|v| !v.trim().is_empty()));
        // 端点可推导性同读侧：显式 URL 或注册表 openai_url 任一成立才搬
        // （否则旧配置本就解析失败=未启用，搬过去是僵尸行）
        let url_derivable = prefs
            .embedding_base_url
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
            || crate::harness::provider::provider_openai_url(
                prefs.embedding_provider.as_deref().unwrap_or_default(),
            )
            .is_some();
        (complete && url_derivable).then(|| PlannedProfile {
            id: "mp-embed".into(),
            alias: "语义检索".into(),
            provider: prefs.embedding_provider.clone().unwrap().trim().to_string(),
            model: prefs.embedding_model.clone().unwrap().trim().to_string(),
            api_key: prefs.embedding_api_key.clone().unwrap().trim().to_string(),
            base_url: prefs
                .embedding_base_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
            sort_order: 0,
        })
    };

    MigrationPlan { vision, embedding }
}

/// 执行迁移（次序铁律见模块头）。`store_key` 注入 Stronghold 写（测试可模拟
/// 失败路径）；返回建出的 profile 总数。
async fn execute_migration<F>(
    pool: &SqlitePool,
    plan: &MigrationPlan,
    mut store_key: F,
) -> AppResult<usize>
where
    F: FnMut(&PlannedProfile) -> AppResult<()>,
{
    let all: Vec<&PlannedProfile> = plan
        .vision
        .as_ref()
        .map(|v| v.iter().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(plan.embedding.iter())
        .collect();

    // 1. Stronghold 全部落成功（? 短路：失败即中止，引用键/清旧键都不发生）
    for p in &all {
        store_key(p)?;
    }
    // 2. DB 行（INSERT OR IGNORE 幂等；重放不重复不覆盖用户后续编辑）
    for p in &all {
        repo::model_profile::create(
            pool,
            &repo::model_profile::NewProfileRow {
                id: p.id.clone(),
                alias: p.alias.clone(),
                provider: p.provider.clone(),
                model: p.model.clone(),
                api_key_ref: p.api_key_ref(),
                base_url: p.base_url.clone(),
                sort_order: p.sort_order,
            },
            true,
        )
        .await?;
    }
    // 3. 引用键（set 覆写；值形态 = 前端 JSON.stringify 惯例）
    if let Some(vision) = &plan.vision {
        let ids: Vec<&str> = vision.iter().map(|p| p.id.as_str()).collect();
        repo::preferences::set(pool, "vision_profile_ids", &serde_json::to_string(&ids)?).await?;
    }
    if let Some(e) = &plan.embedding {
        repo::preferences::set(
            pool,
            "embedding_profile_id",
            &serde_json::to_string(&e.id)?,
        )
        .await?;
    }
    // 4. 清旧键（各自仅当该腿跑了；delete 幂等）
    if plan.vision.is_some() {
        for key in LEGACY_VISION_KEYS {
            repo::preferences::delete(pool, key).await?;
        }
    }
    if plan.embedding.is_some() {
        for key in LEGACY_EMBEDDING_KEYS {
            repo::preferences::delete(pool, key).await?;
        }
    }
    Ok(all.len())
}

/// boot 入口：读 preferences → 计划 → 执行。失败 warn 不阻塞启动（读侧回落
/// 旧格式继续工作，下次启动整腿重放）。挂点在 crypto::init 之后、KB watcher
/// 之前（watcher 首扫即消费新引用配置）。
pub async fn migrate_legacy_model_prefs(app: &tauri::AppHandle, pool: &SqlitePool) {
    let prefs = match repo::preferences::get_all(pool).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(target: "ice_paw.migrate", err = %e, "读取 preferences 失败，跳过旧模型配置迁移");
            return;
        }
    };
    let plan = plan_legacy_migration(&prefs);
    if plan.vision.is_none() && plan.embedding.is_none() {
        return; // 无事可做（已迁移 / 旧数据不完整）
    }
    let n_vision = plan.vision.as_ref().map(|v| v.len());
    let has_embed = plan.embedding.is_some();
    let result = execute_migration(pool, &plan, |p| {
        crate::crypto::store_api_key(app, &p.api_key_ref(), &p.api_key, p.base_url.as_deref())
    })
    .await;
    match result {
        Ok(n) => tracing::info!(
            target: "ice_paw.migrate",
            profiles = n,
            vision = ?n_vision,
            embedding = has_embed,
            "旧模型配置已迁移为 ModelProfile 实体（vision_config/旧四键 → 引用键）"
        ),
        Err(e) => tracing::warn!(
            target: "ice_paw.migrate",
            err = %e,
            "旧模型配置迁移失败（读侧回落旧格式继续工作，下次启动重试）"
        ),
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{UserPreferences, VisionConfigEntry};

    fn entry(provider: &str, model: &str, key: &str) -> VisionConfigEntry {
        VisionConfigEntry {
            provider: provider.into(),
            model: model.into(),
            api_key: key.into(),
            base_url: None,
        }
    }

    /// 全空 preferences（全新安装 / 全未配置）→ 两腿都 None
    #[test]
    fn plan_noop_on_fresh_prefs() {
        let plan = plan_legacy_migration(&UserPreferences::default());
        assert_eq!(plan, MigrationPlan::default());
    }

    /// 新格式条目链 → 逐条规划，确定性 id / 别名 / base_url 保留
    #[test]
    fn plan_vision_from_new_format_entries() {
        let mut prefs = UserPreferences::default();
        let mut e0 = entry("glm", "glm-5.3-flash", "sk-a");
        e0.base_url = Some("https://custom.example/v1".into());
        prefs.vision_config = Some(vec![e0, entry("deepseek", "ds-vision", "sk-b")]);

        let plan = plan_legacy_migration(&prefs);
        let vision = plan.vision.expect("vision_config=Some → 腿必跑");
        assert_eq!(vision.len(), 2);
        assert_eq!(vision[0].id, "mp-vision-0");
        assert_eq!(vision[0].alias, "视觉主模型");
        assert_eq!(vision[0].base_url.as_deref(), Some("https://custom.example/v1"));
        assert_eq!(vision[1].id, "mp-vision-1");
        assert_eq!(vision[1].alias, "视觉降级1");
        assert_eq!(vision[1].base_url, None, "未显式填端点 → None 走运行时推导");
        assert!(plan.embedding.is_none());
    }

    /// 无效条目（字段不齐）被过滤，有效条目序位重排（id 无空洞）
    #[test]
    fn plan_vision_filters_invalid_and_reindexes() {
        let prefs = UserPreferences {
            vision_config: Some(vec![
                entry("", "m", "k"),        // provider 空 → 无效
                entry("glm", "glm-a", " "), // key 空白 → 无效
                entry("glm", "glm-b", "sk-ok"),
                entry("deepseek", "ds-c", "sk-ok2"),
            ]),
            ..Default::default()
        };

        let vision = plan_legacy_migration(&prefs).vision.unwrap();
        assert_eq!(vision.len(), 2);
        assert_eq!(vision[0].id, "mp-vision-0");
        assert_eq!(vision[0].model, "glm-b");
        assert_eq!(vision[1].id, "mp-vision-1");
        assert_eq!(vision[1].model, "ds-c");
    }

    /// vision_config 显式清空（Some(vec![])）→ 等价迁移为空引用链（语义保真）
    #[test]
    fn plan_vision_explicit_empty_preserved() {
        let prefs = UserPreferences {
            vision_config: Some(vec![]),
            // 死旧键残留（被 Some 遮蔽）也该被清 → 腿必跑
            vision_provider: Some("glm".into()),
            ..Default::default()
        };

        let plan = plan_legacy_migration(&prefs);
        assert_eq!(plan.vision, Some(vec![]), "空链等价迁移，腿跑但零 profile");
    }

    /// 旧四键齐全 → 单条「视觉主模型」；不全 → 腿不动（键留给读侧回落）
    #[test]
    fn plan_vision_from_legacy_four_keys() {
        let prefs = UserPreferences {
            vision_provider: Some("glm".into()),
            vision_model: Some("glm-5.3-flash".into()),
            vision_api_key: Some("sk-a".into()),
            vision_base_url: Some("https://x.example".into()),
            ..Default::default()
        };

        let plan = plan_legacy_migration(&prefs);
        let vision = plan.vision.expect("旧四键齐全 → 腿跑");
        assert_eq!(vision.len(), 1);
        assert_eq!(vision[0].id, "mp-vision-0");
        assert_eq!(vision[0].alias, "视觉主模型");
        assert_eq!(vision[0].base_url.as_deref(), Some("https://x.example"));

        // 缺 key → 腿不动
        let partial = UserPreferences {
            vision_provider: Some("glm".into()),
            vision_model: Some("glm-5.3-flash".into()),
            ..Default::default()
        };
        assert!(plan_legacy_migration(&partial).vision.is_none());
    }

    /// 引用键已 Some（含显式清空）→ 腿跳过（Some=权威，不覆写用户选择）
    #[test]
    fn plan_vision_skips_when_already_migrated() {
        let prefs = UserPreferences {
            vision_profile_ids: Some(vec![]), // 显式清空也是权威
            vision_config: Some(vec![entry("glm", "m", "k")]),
            ..Default::default()
        };
        assert!(plan_legacy_migration(&prefs).vision.is_none());
    }

    /// embedding 四键齐全 → 单条 mp-embed「语义检索」；不全 / 无端点 → 腿不动
    #[test]
    fn plan_embedding_from_legacy_four_keys() {
        let prefs = UserPreferences {
            embedding_provider: Some("glm".into()),
            embedding_model: Some("embedding-3".into()),
            embedding_api_key: Some("sk-e".into()),
            ..Default::default()
        };

        let e = plan_legacy_migration(&prefs)
            .embedding
            .expect("四键齐全且 glm 有 openai_url → 腿跑");
        assert_eq!(e.id, "mp-embed");
        assert_eq!(e.alias, "语义检索");
        assert_eq!(e.model, "embedding-3");
        assert_eq!(e.base_url, None, "无显式 URL → None 走运行时推导");

        // 缺 provider → 不搬
        let partial = UserPreferences {
            embedding_model: Some("embedding-3".into()),
            embedding_api_key: Some("sk-e".into()),
            ..Default::default()
        };
        assert!(plan_legacy_migration(&partial).embedding.is_none());

        // 端点不可推导（未知厂商且无显式 URL）→ 不搬（旧配置本就未启用）
        let no_url = UserPreferences {
            embedding_provider: Some("totally-unknown".into()),
            embedding_model: Some("m".into()),
            embedding_api_key: Some("k".into()),
            ..Default::default()
        };
        assert!(plan_legacy_migration(&no_url).embedding.is_none());

        // 显式 URL 的自定义厂商 → 可搬且保留显式值
        let mut custom = no_url;
        custom.embedding_base_url = Some("https://my-proxy.example/v1".into());
        let e2 = plan_legacy_migration(&custom).embedding.expect("显式 URL → 可搬");
        assert_eq!(e2.base_url.as_deref(), Some("https://my-proxy.example/v1"));

        // 已迁移 → 腿跳过
        let mut done = prefs.clone();
        done.embedding_profile_id = Some("mp-embed".into());
        assert!(plan_legacy_migration(&done).embedding.is_none());
    }

    // ---- 执行器（内存库；Stronghold 以注入闭包模拟）----

    async fn fresh_pool() -> SqlitePool {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
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

    /// seed 完整 legacy 形态：vision_config 2 条 + embedding 四键（JSON.stringify 形态）
    async fn seed_legacy(pool: &SqlitePool) {
        let entries = vec![entry("glm", "glm-5.3-flash", "sk-a"), entry("deepseek", "ds-v", "sk-b")];
        repo::preferences::set(
            pool,
            "vision_config",
            &serde_json::to_string(&entries).unwrap(),
        )
        .await
        .unwrap();
        // 死旧四键（被 vision_config=Some 遮蔽）
        repo::preferences::set(pool, "vision_provider", "\"glm\"")
            .await
            .unwrap();
        repo::preferences::set(pool, "embedding_provider", "\"glm\"")
            .await
            .unwrap();
        repo::preferences::set(pool, "embedding_model", "\"embedding-3\"")
            .await
            .unwrap();
        repo::preferences::set(pool, "embedding_api_key", "\"sk-e\"")
            .await
            .unwrap();
    }

    /// 全链：建行 + 引用键 + 清旧键 + 幂等重放
    #[tokio::test]
    async fn execute_migrates_and_is_idempotent() {
        let pool = fresh_pool().await;
        seed_legacy(&pool).await;
        let prefs = repo::preferences::get_all(&pool).await.unwrap();
        let plan = plan_legacy_migration(&prefs);
        assert_eq!(plan.vision.as_ref().unwrap().len(), 2);
        assert!(plan.embedding.is_some());

        let n = execute_migration(&pool, &plan, |_| Ok(()))
            .await
            .unwrap();
        assert_eq!(n, 3, "vision 2 + embedding 1");

        // DB 行落位（含 api_key_ref 惯例）
        let rows = repo::model_profile::list(&pool).await.unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.api_key_ref == format!("profile:{}", r.id)));

        // 引用键落位 + 旧键全清
        let after = repo::preferences::get_all(&pool).await.unwrap();
        assert_eq!(
            after.vision_profile_ids,
            Some(vec!["mp-vision-0".into(), "mp-vision-1".into()])
        );
        assert_eq!(after.embedding_profile_id, Some("mp-embed".into()));
        for key in LEGACY_VISION_KEYS.iter().chain(LEGACY_EMBEDDING_KEYS) {
            assert!(
                repo::preferences::get(&pool, key).await.unwrap().is_none(),
                "旧键 {key} 应已清除"
            );
        }

        // 重放（模拟下次 boot 再跑同形态——实际会因引用键 Some 而 plan 为空，
        // 此处直接重执行同 plan 验证 INSERT OR IGNORE 不重复不炸）
        execute_migration(&pool, &plan, |_| Ok(()))
            .await
            .unwrap();
        assert_eq!(repo::model_profile::list(&pool).await.unwrap().len(), 3);
    }

    /// 次序铁律：Stronghold 失败 → 引用键不落、旧键不清、DB 行不建
    #[tokio::test]
    async fn execute_keystore_failure_leaves_legacy_intact() {
        let pool = fresh_pool().await;
        seed_legacy(&pool).await;
        let prefs = repo::preferences::get_all(&pool).await.unwrap();
        let plan = plan_legacy_migration(&prefs);

        let err = execute_migration(&pool, &plan, |_| {
            Err(crate::error::AppError::Stronghold("模拟失败".into()))
        })
        .await;
        assert!(err.is_err(), "Stronghold 失败必须整体 Err");

        assert!(
            repo::model_profile::list(&pool).await.unwrap().is_empty(),
            "Stronghold 先落铁律：DB 行不应存在"
        );
        let after = repo::preferences::get_all(&pool).await.unwrap();
        assert!(after.vision_profile_ids.is_none(), "引用键不落");
        assert!(after.embedding_profile_id.is_none());
        assert!(
            after.embedding_api_key.is_some(),
            "旧键保留（读侧回落继续工作）"
        );
    }
}
