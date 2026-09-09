//! 新建 agent 手动模型配置 → ModelProfile 实体自动物化（Phase 3，2026-09-08
//! 拍板）。
//!
//! 产品决策：新建表单保留手写模型配置（厂商/模型/Key/URL，用户心智不变），
//! 保存时系统自动把这份配置物化为「设置-模型」里的实体并让 agent 引用——
//! 数据从出生就是统一形态（实体 + 引用）；编辑侧只允许选择实体（AgentForm
//! 合并 tag 选择器），手动入口不再暴露。
//!
//! 匹配语义与 boot 存量抽离（`agent_profile_migration`）同源（`profile_match`
//! 单一真相源）：同配置（厂商+模型+端点+Key）复用既有实体不重复建——两个
//! agent 用同一份凭据各建一次，天然共引同一个实体。执行序：Stronghold 槽位
//! → DB 行（DB 败回滚槽位防幽灵密钥，`model_profile_cmd::create` 同款）。
//!
//! 不物化的旁路由调用方判定（`agent_cmd::manual_materializable`）：凭据不齐
//! （需 Key 厂商无 Key / custom 缺端点，仅非 UI 调用方可达）不硬造 keyless
//! 实体——legacy 行照常可用，编辑时选实体即转正。

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::repo;
use crate::error::AppResult;
use crate::harness::profile_match::{
    group_key, normalize_url, scan_existing, trim_non_empty, unique_alias, GroupKey,
};
use crate::harness::provider;

/// 物化结果。
#[derive(Debug)]
pub(crate) struct Materialized {
    pub(crate) profile_id: String,
    /// false = 同配置复用既有实体（未新建）。
    pub(crate) created: bool,
}

/// 把手动模型配置物化为 profile 实体；同配置（去重键见 [`profile_match`]）
/// 复用既有实体。
///
/// Stronghold 读写以闭包注入（单测模拟 vault，不依赖 AppHandle）。id 用
/// `mp-{uuid}`（`model_profile_cmd::create` 同款——源是一次性用户输入，无
/// 崩溃重放收敛需求，不适用迁移的内容哈希 id）。
// 8 参各有其义（5 数据 + 3 槽位闭包），仓内同类先例见 agent_cmd/chat_cmd
#[allow(clippy::too_many_arguments)]
pub(crate) async fn materialize_from_manual<F, G, H>(
    pool: &SqlitePool,
    provider: &str,
    model: &str,
    api_key: &str,
    explicit_base_url: Option<&str>,
    fetch_key: F,
    store_key: G,
    delete_key: H,
) -> AppResult<Materialized>
where
    F: Fn(&str) -> Option<(String, Option<String>)>,
    G: Fn(&str, &str, Option<&str>) -> AppResult<()>,
    H: Fn(&str) -> AppResult<()>,
{
    let gk: GroupKey = group_key(provider, model, explicit_base_url, api_key);
    let existing = repo::model_profile::list(pool).await?;
    let (by_key, mut used_aliases, next_sort) = scan_existing(&existing, &fetch_key);
    if let Some(id) = by_key.get(&gk) {
        return Ok(Materialized {
            profile_id: id.clone(),
            created: false,
        });
    }

    let (label, default_url) = match provider::list_provider_infos()
        .into_iter()
        .find(|p| p.name == gk.provider)
    {
        Some(info) => (info.label, info.default_url),
        // 未注册厂商：label 退用厂商名，默认端点为空串
        None => (
            gk.provider.clone(),
            provider::provider_default_url(&gk.provider),
        ),
    };
    // 存储形态：显式值与注册表默认同端点 → None（运行时按注册表推导，与空
    // 等价）；真自定义端点原样保留（trim 后）
    let base_url = trim_non_empty(explicit_base_url)
        .filter(|u| normalize_url(u) != normalize_url(&default_url))
        .map(str::to_string);

    let id = format!("mp-{}", Uuid::new_v4());
    let slot = format!("profile:{id}");
    let alias = unique_alias(&mut used_aliases, &label, &gk.model);
    store_key(&slot, &gk.api_key, base_url.as_deref())?;
    let row = repo::model_profile::NewProfileRow {
        id: id.clone(),
        alias,
        provider: gk.provider.clone(),
        model: gk.model.clone(),
        api_key_ref: slot.clone(),
        base_url,
        sort_order: next_sort,
    };
    if let Err(e) = repo::model_profile::create(pool, &row, false).await {
        // DB 写失败回滚刚落的槽位（幽灵密钥防护；回滚失败只 warn——留在
        // vault 但无 DB 行指向属孤儿槽位，重试另起 uuid 不受影响）
        if let Err(ke) = delete_key(&slot) {
            tracing::warn!(target: "ice_paw.model_profile", "物化失败回滚槽位 {slot} 失败: {ke}");
        }
        return Err(e);
    }
    Ok(Materialized {
        profile_id: id,
        created: true,
    })
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use std::cell::RefCell;
    use std::collections::HashMap;
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

    fn fetch_fn(v: Vault) -> impl Fn(&str) -> Option<(String, Option<String>)> {
        move |slot| v.borrow().get(slot).cloned()
    }

    fn store_fn(v: Vault) -> impl Fn(&str, &str, Option<&str>) -> AppResult<()> {
        move |slot, key, url| {
            v.borrow_mut()
                .insert(slot.into(), (key.into(), url.map(String::from)));
            Ok(())
        }
    }

    fn delete_fn(v: Vault) -> impl Fn(&str) -> AppResult<()> {
        move |slot| {
            v.borrow_mut().remove(slot);
            Ok(())
        }
    }

    async fn profile_by_id(pool: &SqlitePool, id: &str) -> crate::db::models::ModelProfileRow {
        repo::model_profile::get_by_id(pool, id)
            .await
            .expect("profile row")
    }

    /// 完整手动字段 → 新建实体（Key trim 后入槽位、默认端点不显式存）
    #[tokio::test]
    async fn creates_profile_from_manual_fields() {
        let pool = test_pool().await;
        let vault = new_vault();
        let m = materialize_from_manual(
            &pool,
            " glm ",
            " glm-5.3 ",
            " sk-1 ",
            None,
            fetch_fn(vault.clone()),
            store_fn(vault.clone()),
            delete_fn(vault.clone()),
        )
        .await
        .unwrap();
        assert!(m.created);
        assert!(m.profile_id.starts_with("mp-"));

        let p = profile_by_id(&pool, &m.profile_id).await;
        assert_eq!(p.provider, "glm"); // trim 后落库
        assert_eq!(p.model, "glm-5.3");
        assert_eq!(p.base_url, None, "默认端点不显式存");
        assert_eq!(p.api_key_ref, format!("profile:{}", p.id));
        assert!(p.alias.contains("glm-5.3"), "别名含模型名: {}", p.alias);
        // Key 进实体槽位（trim 后）
        assert_eq!(
            vault.borrow().get(&p.api_key_ref),
            Some(&("sk-1".to_string(), None))
        );
    }

    /// 同配置（含端点等价形态）→ 复用既有实体不重复建
    #[tokio::test]
    async fn reuses_existing_profile_for_same_credentials() {
        let pool = test_pool().await;
        let vault = new_vault();
        let m1 = materialize_from_manual(
            &pool,
            "glm",
            "glm-5.3",
            "sk-1",
            None,
            fetch_fn(vault.clone()),
            store_fn(vault.clone()),
            delete_fn(vault.clone()),
        )
        .await
        .unwrap();
        assert!(m1.created);

        // 第二次同配置（Key 带空白、端点显式写默认值）→ 复用
        let default_url = provider::provider_default_url("glm");
        let m2 = materialize_from_manual(
            &pool,
            "glm",
            "glm-5.3",
            " sk-1 ",
            Some(&default_url),
            fetch_fn(vault.clone()),
            store_fn(vault.clone()),
            delete_fn(vault.clone()),
        )
        .await
        .unwrap();
        assert!(!m2.created);
        assert_eq!(m2.profile_id, m1.profile_id);
        assert_eq!(
            repo::model_profile::list(&pool).await.unwrap().len(),
            1,
            "不应重复建实体"
        );
    }

    /// 真自定义端点保留原样；末尾斜杠差异视为同端点（复用）
    #[tokio::test]
    async fn custom_url_kept_and_slash_equivalence_reuses() {
        let pool = test_pool().await;
        let vault = new_vault();
        let m1 = materialize_from_manual(
            &pool,
            "custom",
            "qwen3:8b",
            "",
            Some("http://localhost:11434/v1"),
            fetch_fn(vault.clone()),
            store_fn(vault.clone()),
            delete_fn(vault.clone()),
        )
        .await
        .unwrap();
        let p1 = profile_by_id(&pool, &m1.profile_id).await;
        assert_eq!(p1.base_url.as_deref(), Some("http://localhost:11434/v1"));

        // 末尾多一杠 = 同端点 → 复用
        let m2 = materialize_from_manual(
            &pool,
            "custom",
            "qwen3:8b",
            "",
            Some("http://localhost:11434/v1/"),
            fetch_fn(vault.clone()),
            store_fn(vault.clone()),
            delete_fn(vault.clone()),
        )
        .await
        .unwrap();
        assert!(!m2.created, "斜杠差异应复用同一实体");
        assert_eq!(m1.profile_id, m2.profile_id);
    }

    /// 同模型不同 Key → 新实体 + 别名加序号（同迁移惯例，可辨认）
    #[tokio::test]
    async fn distinct_key_same_model_gets_numbered_alias() {
        let pool = test_pool().await;
        let vault = new_vault();
        let m1 = materialize_from_manual(
            &pool,
            "glm",
            "glm-5.3",
            "sk-1",
            None,
            fetch_fn(vault.clone()),
            store_fn(vault.clone()),
            delete_fn(vault.clone()),
        )
        .await
        .unwrap();
        let m2 = materialize_from_manual(
            &pool,
            "glm",
            "glm-5.3",
            "sk-2",
            None,
            fetch_fn(vault.clone()),
            store_fn(vault.clone()),
            delete_fn(vault.clone()),
        )
        .await
        .unwrap();
        assert!(m1.created && m2.created);
        assert_ne!(m1.profile_id, m2.profile_id);

        let a1 = profile_by_id(&pool, &m1.profile_id).await.alias;
        let a2 = profile_by_id(&pool, &m2.profile_id).await.alias;
        assert_eq!(a2, format!("{a1} 2"), "重名追加序号: {a1} / {a2}");
    }

    /// Stronghold 写失败 → 诚实失败，DB 零行
    #[tokio::test]
    async fn store_failure_propagates_without_db_row() {
        let pool = test_pool().await;
        let vault = new_vault();
        let err = materialize_from_manual(
            &pool,
            "glm",
            "glm-5.3",
            "sk-1",
            None,
            fetch_fn(vault.clone()),
            |_, _, _| Err(AppError::Stronghold("模拟失败".into())),
            delete_fn(vault.clone()),
        )
        .await;
        assert!(err.is_err());
        assert!(
            repo::model_profile::list(&pool).await.unwrap().is_empty(),
            "DB 行不应先于 Stronghold 落"
        );
    }
}
