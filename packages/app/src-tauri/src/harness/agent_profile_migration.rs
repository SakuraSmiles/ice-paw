//! 存量 agent 模型配置 → ModelProfile 实体一次性 boot 迁移（2026-09-08 用户指令）。
//!
//! 与 `legacy_model_migration`（Phase 1，视觉/语义检索偏好腿）同族但数据源不同：
//! 那边读不可变的 preferences（确定性序号 id 即可幂等），这边读**会在迁移过程中
//! 变化的 agents 行**（第 3 步写引用列直接改变下一轮扫描的资格集）——所以幂等
//! 基石换成**内容哈希 id**：组的 id 只由组内容决定，与「还剩哪些 agent」无关，
//! 任意步崩溃后重放收敛到同一结果（部分完成后序号 id 会张冠李戴，禁用序号）。
//!
//! 语义：把 legacy 手动 agent（model_profile_id IS NULL）的模型配置抽离为
//! 「设置-模型」里的实体，agent 转为引用。同配置（厂商+模型+端点+Key）只建
//! 一个实体、多个 agent 引用同一个；与既有实体同配置时复用既有实体不重复建。
//!
//! 次序铁律（Phase 1 同款，`?` 短路整体中止）：
//! 1. Stronghold 槽位全落（`profile:{id}`）→ 2. DB 行（INSERT OR IGNORE）→
//! 3. agents 引用列（窄 UPDATE 只 SET model_profile_id，快照列不动——值本就
//!    相等，聊天行为逐字节保持）→ 4. preferences 幂等标记（最后落，此前任何
//!    失败下次启动整段重放收敛）。
//!
//! 幂等标记的必要性：不能只靠 model_profile_id IS NULL 当判据——迁移跑过之后
//! 用户手动新建的手动 agent（表单里「手动配置」是刻意选择）会在下次 boot 被
//! 误收编。标记在 → 永不再扫。
//!
//! 跳过条件（保持 legacy 手动形态照常可用，warn 披露）：provider/model 字段
//! 不齐 / 需 Key 的厂商但 Key 读不到 / custom 缺端点。agent 旧 key 槽位留着
//! 不读（批 1 惯例：引用 agent 的 Key 从 profile 槽位取）。

use std::collections::{HashMap, HashSet};

use sqlx::SqlitePool;

use crate::db::repo;
use crate::error::AppResult;
use crate::harness::provider;

/// 幂等标记（preferences 内部标记类键，backfill 版本号先例；跑过即不再扫）。
const MIGRATION_MARKER: &str = "agent_profile_migration_done";

// =========================================================================
// 数据形状
// =========================================================================

/// 一个待新建实体的完整规划（与 Phase 1 PlannedProfile 同族；id 是内容哈希）。
#[derive(Debug, Clone)]
pub struct PlannedProfile {
    pub id: String,
    pub alias: String,
    pub provider: String,
    pub model: String,
    /// 明文 Key（只进 Stronghold 槽位不落库；空串 = 免鉴权本地端点占位）。
    pub api_key: String,
    pub base_url: Option<String>,
    pub sort_order: i32,
}

impl PlannedProfile {
    fn api_key_ref(&self) -> String {
        format!("profile:{}", self.id)
    }
}

/// 去重键：同 (厂商, 模型, 生效端点, Key) = 同一配置 = 同一个实体。
/// 端点归一（显式与注册表默认等价 / 末尾斜杠）；Key 两侧都 trim。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GroupKey {
    provider: String,
    model: String,
    effective_url: String,
    api_key: String,
}

/// 迁移结果（boot 日志用）。
#[derive(Debug, Default)]
pub struct MigrationOutcome {
    /// 标记已在（本轮直接跳过）。
    pub done_before: bool,
    pub profiles_created: usize,
    pub profiles_reused: usize,
    pub agents_migrated: usize,
    pub agents_skipped: usize,
    pub skip_reasons: Vec<String>,
}

// =========================================================================
// 纯函数
// =========================================================================

fn trim_non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|v| !v.is_empty())
}

/// 首个非空（trim 后）；行值与 vault 副本的取值序。
fn first_non_empty(a: Option<&str>, b: Option<&str>) -> Option<String> {
    trim_non_empty(a)
        .map(str::to_string)
        .or_else(|| trim_non_empty(b).map(str::to_string))
}

/// 端点比较形态（trim + 去末尾斜杠——`x/v1` 与 `x/v1/` 同端点）。
fn normalize_url(u: &str) -> String {
    u.trim().trim_end_matches('/').to_string()
}

/// 实际生效端点：显式值 > 注册表默认（未知厂商默认为空串）。
fn effective_url(provider: &str, explicit: Option<&str>) -> String {
    let raw = trim_non_empty(explicit)
        .map(str::to_string)
        .unwrap_or_else(|| provider::provider_default_url(provider));
    normalize_url(&raw)
}

/// FNV-1a 64 → 12 hex。手写不依赖 std Hasher 的稳定性（跨版本重放同 id）。
fn content_hash(k: &GroupKey) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in [&k.provider, &k.model, &k.effective_url, &k.api_key] {
        for b in part.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        // 字段边界混入分隔字节：防相邻字段拼接歧义
        h ^= 0x1f;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:012x}")
}

/// 别名 = `{厂商展示名} {model}`；重名追加序号（同模型多 Key 场景可辨认）。
fn unique_alias(used: &mut HashSet<String>, label: &str, model: &str) -> String {
    let base = format!("{label} {model}");
    if used.insert(base.clone()) {
        return base;
    }
    for n in 2.. {
        let candidate = format!("{base} {n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

// =========================================================================
// 迁移本体
// =========================================================================

/// 核心迁移（Stronghold 读写以闭包注入，单测可模拟失败/缺失）。
pub async fn run_migration<F, G>(
    pool: &SqlitePool,
    fetch_key: F,
    store_key: G,
) -> AppResult<MigrationOutcome>
where
    F: Fn(&str) -> AppResult<(String, Option<String>)>,
    G: Fn(&PlannedProfile) -> AppResult<()>,
{
    // 0) 幂等标记：跑过即不再扫（手动形态是用户刻意选择，误收编 = 静默翻转）
    if repo::preferences::get(pool, MIGRATION_MARKER).await?.is_some() {
        return Ok(MigrationOutcome {
            done_before: true,
            ..MigrationOutcome::default()
        });
    }

    // 厂商目录：label（别名用）+ default_url（端点归一用）
    let provider_catalog: HashMap<String, (String, String)> =
        provider::list_provider_infos()
            .into_iter()
            .map(|p| (p.name, (p.label, p.default_url)))
            .collect();

    // 1) 逐 agent 判定资格（跳过条件就地记录；fetch 失败 = 无 Key 记录）
    let agents = repo::agent::list(pool).await?;
    let mut eligible: Vec<(String, GroupKey, Option<String>)> = vec![]; // (agent_id, 去重键, 显式端点原样)
    let mut outcome = MigrationOutcome::default();
    for a in &agents {
        if a.model_profile_id.as_deref().is_some_and(|s| !s.is_empty()) {
            continue; // 已是引用形态（含上轮重放完成的部分）
        }
        let (key, vault_url) = fetch_key(&a.api_key_ref).unwrap_or_default();
        let key = key.trim().to_string();
        let explicit = first_non_empty(a.base_url.as_deref(), vault_url.as_deref());
        let eff_url = effective_url(&a.provider, explicit.as_deref());
        let reason = if a.provider.trim().is_empty() || a.model.trim().is_empty() {
            Some("provider/model 字段不齐".to_string())
        } else if provider::provider_requires_key(&a.provider) && key.is_empty() {
            Some("需要 Key 的厂商但 Key 未配置".to_string())
        } else if provider::provider_requires_base_url(&a.provider) && eff_url.is_empty() {
            Some("自定义端点缺 base_url".to_string())
        } else {
            None
        };
        match reason {
            Some(r) => {
                outcome.agents_skipped += 1;
                outcome.skip_reasons.push(format!("{}: {r}", a.id));
            }
            None => eligible.push((
                a.id.clone(),
                GroupKey {
                    provider: a.provider.trim().to_string(),
                    model: a.model.trim().to_string(),
                    effective_url: eff_url,
                    api_key: key,
                },
                explicit,
            )),
        }
    }

    // 2) 既有实体：同键复用映射 + 别名占用集 + sort_order 起点
    let existing = repo::model_profile::list(pool).await?;
    let mut existing_by_key: HashMap<GroupKey, String> = HashMap::new();
    let mut used_aliases: HashSet<String> = HashSet::new();
    let mut next_sort = existing.iter().map(|p| p.sort_order).max().unwrap_or(-1) + 1;
    for p in &existing {
        used_aliases.insert(p.alias.clone());
        // key 读不到无法比对——不复用（该组新实体照建，不阻塞迁移）
        let Ok((key, vault_url)) = fetch_key(&p.api_key_ref) else {
            continue;
        };
        let explicit = first_non_empty(p.base_url.as_deref(), vault_url.as_deref());
        let gk = GroupKey {
            provider: p.provider.clone(),
            model: p.model.clone(),
            effective_url: effective_url(&p.provider, explicit.as_deref()),
            api_key: key.trim().to_string(),
        };
        existing_by_key.entry(gk).or_insert_with(|| p.id.clone());
    }

    // 3) 分组（首遇序 = list 的确定性序）与规划
    let mut groups: Vec<(GroupKey, Vec<String>, Option<String>)> = vec![];
    for (agent_id, gk, explicit) in eligible {
        match groups.iter_mut().find(|(k, _, _)| *k == gk) {
            Some(g) => g.1.push(agent_id),
            None => groups.push((gk, vec![agent_id], explicit)),
        }
    }
    let mut planned: Vec<PlannedProfile> = vec![];
    let mut refs: Vec<(String, String)> = vec![]; // (agent_id, profile_id)
    for (gk, agent_ids, explicit) in groups {
        let id = match existing_by_key.get(&gk) {
            Some(existing_id) => {
                outcome.profiles_reused += 1;
                existing_id.clone()
            }
            None => {
                let (label, default_url) = provider_catalog
                    .get(&gk.provider)
                    .cloned()
                    .unwrap_or_else(|| {
                        (
                            gk.provider.clone(),
                            provider::provider_default_url(&gk.provider),
                        )
                    });
                // 存储形态：显式值与注册表默认同端点 → None（运行时按注册表推导，
                // 与空等价）；真自定义端点 → 原样保留
                let base_url = match &explicit {
                    Some(u) if normalize_url(u) != normalize_url(&default_url) => Some(u.clone()),
                    _ => None,
                };
                let id = format!("mp-agent-{}", content_hash(&gk));
                planned.push(PlannedProfile {
                    id: id.clone(),
                    alias: unique_alias(&mut used_aliases, &label, &gk.model),
                    provider: gk.provider.clone(),
                    model: gk.model.clone(),
                    api_key: gk.api_key.clone(),
                    base_url,
                    sort_order: next_sort,
                });
                next_sort += 1;
                outcome.profiles_created += 1;
                id
            }
        };
        for aid in agent_ids {
            refs.push((aid, id.clone()));
        }
    }

    // 4) 执行（次序铁律；任一步失败 ? 短路——标记未落，下次整段重放收敛）
    for p in &planned {
        store_key(p)?;
    }
    for p in &planned {
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
    for (agent_id, profile_id) in &refs {
        repo::agent::set_model_profile_reference(pool, agent_id, Some(profile_id)).await?;
    }
    repo::preferences::set(pool, MIGRATION_MARKER, "1").await?;

    outcome.agents_migrated = refs.len();
    Ok(outcome)
}

/// boot 入口（挂点在 migrate_legacy_model_prefs 之后，同为轻量同步跑；两迁移
/// 互不依赖，先后皆可）。失败 warn 不阻塞——agent 保持手动形态照常可用，
/// 下次启动重放。跑过即静默（done_before）。
pub async fn migrate_agent_models(app: &tauri::AppHandle, pool: &SqlitePool) {
    let result = run_migration(
        pool,
        |slot| crate::crypto::fetch_api_key(app, slot),
        |p| {
            crate::crypto::store_api_key(app, &p.api_key_ref(), &p.api_key, p.base_url.as_deref())
        },
    )
    .await;
    match result {
        Ok(o) if o.done_before => {}
        Ok(o) => tracing::info!(
            target: "ice_paw.migrate",
            created = o.profiles_created,
            reused = o.profiles_reused,
            agents = o.agents_migrated,
            skipped = o.agents_skipped,
            reasons = o.skip_reasons.join("; "),
            "存量 agent 模型配置已抽离为 ModelProfile 实体（同配置合并，agent 转引用形态）"
        ),
        Err(e) => tracing::warn!(
            target: "ice_paw.migrate",
            err = %e,
            "存量 agent 模型抽离失败（agent 保持手动形态可用，下次启动重试）"
        ),
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::NewAgent;
    use crate::error::AppError;
    use std::cell::RefCell;
    use std::rc::Rc;

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

    type Vault = Rc<RefCell<HashMap<String, (String, Option<String>)>>>;

    fn new_vault() -> Vault {
        Rc::new(RefCell::new(HashMap::new()))
    }
    fn put_key(v: &Vault, slot: &str, key: &str, base_url: Option<&str>) {
        v.borrow_mut()
            .insert(slot.into(), (key.into(), base_url.map(String::from)));
    }
    fn fetch_fn(v: Vault) -> impl Fn(&str) -> AppResult<(String, Option<String>)> {
        move |slot| {
            v.borrow()
                .get(slot)
                .cloned()
                .ok_or_else(|| AppError::NotFound {
                    resource: "vault slot",
                    id: slot.to_string(),
                })
        }
    }
    fn store_fn(v: Vault) -> impl Fn(&PlannedProfile) -> AppResult<()> {
        move |p| {
            v.borrow_mut()
                .insert(p.api_key_ref(), (p.api_key.clone(), p.base_url.clone()));
            Ok(())
        }
    }

    async fn seed_agent(pool: &SqlitePool, id: &str, provider: &str, model: &str, base_url: Option<&str>) {
        let na = NewAgent {
            id: id.into(),
            name: id.into(),
            provider: provider.into(),
            model: model.into(),
            system_prompt: String::new(),
            api_key: String::new(),
            base_url: base_url.map(String::from),
            temperature: 0.7,
            max_tokens: 16384,
            extra_params: None,
            sort_order: 0,
            cache_prompt: true,
            supports_vision: false,
            max_history_messages: None,
            context_window: None,
            enabled_tools: None,
            workspace_path: None,
            avatar: None,
            model_profile_id: None,
            fallback_profile_ids: None,
        };
        repo::agent::create(pool, &na, id, id).await.expect("seed agent");
    }

    /// 同配置（同厂商/模型/Key/默认端点）两个 agent → 只建一个实体、共同引用
    #[tokio::test]
    async fn same_config_agents_share_one_profile() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "glm", "glm-5.3", None).await;
        seed_agent(&pool, "a2", "glm", "glm-5.3", None).await;
        let vault = new_vault();
        put_key(&vault, "a1", "sk-1", None);
        put_key(&vault, "a2", "sk-1", None);

        let out = run_migration(&pool, fetch_fn(vault.clone()), store_fn(vault.clone()))
            .await
            .unwrap();
        assert_eq!(out.profiles_created, 1);
        assert_eq!(out.agents_migrated, 2);
        assert_eq!(out.agents_skipped, 0);

        let profiles = repo::model_profile::list(&pool).await.unwrap();
        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        assert!(p.id.starts_with("mp-agent-"));
        assert_eq!(p.provider, "glm");
        assert_eq!(p.model, "glm-5.3");
        assert_eq!(p.base_url, None); // 默认端点不显式存
        assert_eq!(p.api_key_ref, format!("profile:{}", p.id));
        assert!(p.alias.contains("glm-5.3"));

        for aid in ["a1", "a2"] {
            assert_eq!(
                repo::agent::get_by_id(&pool, aid).await.unwrap().model_profile_id.as_deref(),
                Some(p.id.as_str())
            );
        }
        // Key 进了实体槽位（agent 转引用后从此处取）
        assert_eq!(
            vault.borrow().get(&format!("profile:{}", p.id)),
            Some(&("sk-1".to_string(), None))
        );
    }

    /// 同模型不同 Key → 两个实体（别名加序号），各 agent 引用各自的
    #[tokio::test]
    async fn different_key_same_model_creates_two_profiles() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "glm", "glm-5.3", None).await;
        seed_agent(&pool, "a2", "glm", "glm-5.3", None).await;
        let vault = new_vault();
        put_key(&vault, "a1", "sk-1", None);
        put_key(&vault, "a2", "sk-2", None);

        let out = run_migration(&pool, fetch_fn(vault.clone()), store_fn(vault.clone()))
            .await
            .unwrap();
        assert_eq!(out.profiles_created, 2);

        let profiles = repo::model_profile::list(&pool).await.unwrap();
        assert_eq!(profiles.len(), 2);
        assert_ne!(profiles[0].id, profiles[1].id);
        assert_ne!(profiles[0].alias, profiles[1].alias); // 重名加序号可辨认

        // 各自的 agent 引用持有自己 Key 的实体
        let id_of = |key: &str| {
            profiles
                .iter()
                .find(|p| vault.borrow().get(p.api_key_ref.as_str()).map(|(k, _)| k.as_str()) == Some(key))
                .map(|p| p.id.clone())
                .unwrap()
        };
        assert_eq!(
            repo::agent::get_by_id(&pool, "a1").await.unwrap().model_profile_id,
            Some(id_of("sk-1"))
        );
        assert_eq!(
            repo::agent::get_by_id(&pool, "a2").await.unwrap().model_profile_id,
            Some(id_of("sk-2"))
        );
    }

    /// 与既有实体同配置 → 复用不重建（延伸「不要重复添加」）
    #[tokio::test]
    async fn reuses_existing_profile_with_same_credentials() {
        let pool = test_pool().await;
        let vault = new_vault();
        put_key(&vault, "profile:mp-x", "sk-1", None);
        repo::model_profile::create(
            &pool,
            &repo::model_profile::NewProfileRow {
                id: "mp-x".into(),
                alias: "智谱主力".into(),
                provider: "glm".into(),
                model: "glm-5.3".into(),
                api_key_ref: "profile:mp-x".into(),
                base_url: None,
                sort_order: 0,
            },
            false,
        )
        .await
        .unwrap();
        seed_agent(&pool, "a1", "glm", "glm-5.3", None).await;
        put_key(&vault, "a1", "sk-1", None);

        let out = run_migration(&pool, fetch_fn(vault.clone()), store_fn(vault.clone()))
            .await
            .unwrap();
        assert_eq!(out.profiles_created, 0);
        assert_eq!(out.profiles_reused, 1);
        assert_eq!(out.agents_migrated, 1);
        assert_eq!(
            repo::model_profile::list(&pool).await.unwrap().len(),
            1,
            "不应新建实体"
        );
        assert_eq!(
            repo::agent::get_by_id(&pool, "a1").await.unwrap().model_profile_id.as_deref(),
            Some("mp-x")
        );
    }

    /// 显式默认端点与隐式（空）同端点 → 合并；存储为 None（与注册表推导等价）
    #[tokio::test]
    async fn explicit_default_url_merges_with_implicit() {
        let pool = test_pool().await;
        let glm_default = provider::provider_default_url("glm");
        seed_agent(&pool, "a1", "glm", "glm-5.3", Some(&glm_default)).await; // 显式写了默认地址
        seed_agent(&pool, "a2", "glm", "glm-5.3", None).await; // 留空走默认
        let vault = new_vault();
        put_key(&vault, "a1", "sk-1", None);
        put_key(&vault, "a2", "sk-1", None);

        let out = run_migration(&pool, fetch_fn(vault.clone()), store_fn(vault.clone()))
            .await
            .unwrap();
        assert_eq!(out.profiles_created, 1, "同端点应合并为一个实体");
        let profiles = repo::model_profile::list(&pool).await.unwrap();
        assert_eq!(profiles[0].base_url, None);
    }

    /// custom 本地端点（免 Key）→ 迁移为带显式 URL、空 Key 占位的实体
    #[tokio::test]
    async fn custom_local_endpoint_migrates_with_url() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "custom", "qwen3:8b", Some("http://localhost:11434/v1")).await;
        // custom 免鉴权：vault 无 key 槽位（fetch NotFound → 空 Key，合法）
        let vault = new_vault();

        let out = run_migration(&pool, fetch_fn(vault.clone()), store_fn(vault.clone()))
            .await
            .unwrap();
        assert_eq!(out.profiles_created, 1);
        assert_eq!(out.agents_migrated, 1);
        let profiles = repo::model_profile::list(&pool).await.unwrap();
        assert_eq!(profiles[0].base_url.as_deref(), Some("http://localhost:11434/v1"));
        assert_eq!(
            vault.borrow().get(&profiles[0].api_key_ref),
            Some(&("".to_string(), Some("http://localhost:11434/v1".to_string()))),
            "空 Key 占位 + URL 副本入槽位"
        );
    }

    /// 需 Key 的厂商但 Key 读不到 → 跳过该 agent（保持手动形态），但迁移整体完成落标记
    #[tokio::test]
    async fn keyed_provider_missing_key_agent_stays_manual() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "glm", "glm-5.3", None).await;
        let vault = new_vault(); // 无 a1 槽位

        let out = run_migration(&pool, fetch_fn(vault.clone()), store_fn(vault.clone()))
            .await
            .unwrap();
        assert_eq!(out.agents_skipped, 1);
        assert!(out.skip_reasons[0].contains("a1"));
        assert!(out.skip_reasons[0].contains("Key"));
        assert_eq!(out.agents_migrated, 0);
        assert!(repo::model_profile::list(&pool).await.unwrap().is_empty());
        assert!(repo::agent::get_by_id(&pool, "a1").await.unwrap().model_profile_id.is_none());
        // 跳过是终局决定：标记照落，不会下轮再扫
        assert!(repo::preferences::get(&pool, MIGRATION_MARKER).await.unwrap().is_some());
    }

    /// 标记在 → 第二轮零扫描零变化（空 vault 也无害——根本不读 Key）
    #[tokio::test]
    async fn second_run_is_noop_via_marker() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "glm", "glm-5.3", None).await;
        seed_agent(&pool, "a2", "glm", "glm-5.3", None).await;
        let vault = new_vault();
        put_key(&vault, "a1", "sk-1", None);
        put_key(&vault, "a2", "sk-1", None);
        run_migration(&pool, fetch_fn(vault.clone()), store_fn(vault.clone()))
            .await
            .unwrap();
        let pid = repo::model_profile::list(&pool).await.unwrap()[0].id.clone();

        let out = run_migration(&pool, fetch_fn(new_vault()), store_fn(new_vault()))
            .await
            .unwrap();
        assert!(out.done_before);
        assert_eq!(out.profiles_created, 0);
        assert_eq!(repo::model_profile::list(&pool).await.unwrap().len(), 1);
        assert_eq!(
            repo::agent::get_by_id(&pool, "a1").await.unwrap().model_profile_id,
            Some(pid)
        );
    }

    /// Stronghold 失败 → 次序铁律保证 DB 行/引用列都不发生；重放收敛
    #[tokio::test]
    async fn store_failure_keeps_state_clean_then_replay_converges() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "glm", "glm-5.3", None).await;
        seed_agent(&pool, "a2", "glm", "glm-5.3", None).await;
        let vault = new_vault();
        put_key(&vault, "a1", "sk-1", None);
        put_key(&vault, "a2", "sk-1", None);

        let err = run_migration(
            &pool,
            fetch_fn(vault.clone()),
            |_| Err(AppError::Stronghold("模拟失败".into())),
        )
        .await;
        assert!(err.is_err());
        assert!(repo::model_profile::list(&pool).await.unwrap().is_empty(), "DB 行不应先于 Stronghold 落");
        assert!(repo::agent::get_by_id(&pool, "a1").await.unwrap().model_profile_id.is_none());
        assert!(
            repo::preferences::get(&pool, MIGRATION_MARKER).await.unwrap().is_none(),
            "失败不落标记（下次重放）"
        );

        let out = run_migration(&pool, fetch_fn(vault.clone()), store_fn(vault.clone()))
            .await
            .unwrap();
        assert_eq!(out.profiles_created, 1);
        assert_eq!(out.agents_migrated, 2);
        let profiles = repo::model_profile::list(&pool).await.unwrap();
        assert_eq!(profiles.len(), 1);
        for aid in ["a1", "a2"] {
            assert_eq!(
                repo::agent::get_by_id(&pool, aid).await.unwrap().model_profile_id.as_deref(),
                Some(profiles[0].id.as_str())
            );
        }
    }

    /// 内容哈希 id 稳定：同键两次规划同 id（部分完成重放不张冠李戴的地基）
    #[test]
    fn content_hash_is_deterministic_and_distinct() {
        let k1 = GroupKey {
            provider: "glm".into(),
            model: "glm-5.3".into(),
            effective_url: "https://open.bigmodel.cn/api/paas/v4".into(),
            api_key: "sk-1".into(),
        };
        let k2 = k1.clone();
        let k3 = GroupKey { api_key: "sk-2".into(), ..k1.clone() };
        assert_eq!(content_hash(&k1), content_hash(&k2));
        assert_ne!(content_hash(&k1), content_hash(&k3));
        // 字段边界歧义防线：("ab","c") vs ("a","bc") 不同哈希
        let ka = GroupKey { provider: "ab".into(), model: "c".into(), ..k1.clone() };
        let kb = GroupKey { provider: "a".into(), model: "bc".into(), ..k1.clone() };
        assert_ne!(content_hash(&ka), content_hash(&kb));
    }
}
