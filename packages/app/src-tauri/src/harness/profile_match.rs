//! 模型配置匹配原语——「同配置 = 同一实体」的单一真相源。
//!
//! 两条路径共享，判定分叉 = 同一份凭据在迁移与新建两处得到不同的「是否
//! 同一实体」结论，复用失效重复建实体：
//!
//! - `agent_profile_migration`：boot 存量抽离（一次性，2026-09-08 批 3）
//! - `profile_materialize`：新建 agent 手动配置保存时自动物化（运行期增量）
//!
//! 去重键 = (厂商, 模型, 生效端点, Key)。端点归一：trim + 去末尾斜杠 +
//! 显式默认地址与注册表默认合并（存储 None、运行时按注册表推导，与空等价）；
//! Key 两侧 trim。

use std::collections::{HashMap, HashSet};

use crate::db::models::ModelProfileRow;
use crate::harness::provider;

/// 去重键：同 (厂商, 模型, 生效端点, Key) = 同一配置 = 同一个实体。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GroupKey {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) effective_url: String,
    pub(crate) api_key: String,
}

/// trim 后非空才有值。
pub(crate) fn trim_non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|v| !v.is_empty())
}

/// 首个非空（trim 后）；行值与 vault 副本的取值序。
pub(crate) fn first_non_empty(a: Option<&str>, b: Option<&str>) -> Option<String> {
    trim_non_empty(a)
        .map(str::to_string)
        .or_else(|| trim_non_empty(b).map(str::to_string))
}

/// 端点比较形态（trim + 去末尾斜杠——`x/v1` 与 `x/v1/` 同端点）。
pub(crate) fn normalize_url(u: &str) -> String {
    u.trim().trim_end_matches('/').to_string()
}

/// 实际生效端点：显式值 > 注册表默认（未知厂商默认为空串）。
pub(crate) fn effective_url(provider: &str, explicit: Option<&str>) -> String {
    let raw = trim_non_empty(explicit)
        .map(str::to_string)
        .unwrap_or_else(|| provider::provider_default_url(provider));
    normalize_url(&raw)
}

/// 规范构造：厂商/模型/Key 统一 trim，端点走 effective_url。
pub(crate) fn group_key(
    provider: &str,
    model: &str,
    explicit_url: Option<&str>,
    api_key: &str,
) -> GroupKey {
    GroupKey {
        provider: provider.trim().to_string(),
        model: model.trim().to_string(),
        effective_url: effective_url(provider, explicit_url),
        api_key: api_key.trim().to_string(),
    }
}

/// 别名 = `{厂商展示名} {model}`；重名追加序号（同模型多 Key 场景可辨认）。
pub(crate) fn unique_alias(used: &mut HashSet<String>, label: &str, model: &str) -> String {
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

/// 既有实体扫描：同键复用映射 + 别名占用集 + 下一 sort_order。
///
/// Key 读不到的实体不进复用映射（不阻塞流程——该组照建新实体；best-effort
/// 语义与 Stronghold 旁路降级一致）。fetch 以 `Option` 闭包注入（调用方的
/// `AppResult` 在入口 `.ok()` 掉——读不到与失败同归「不参与匹配」）。
pub(crate) fn scan_existing<F>(
    profiles: &[ModelProfileRow],
    fetch_key: F,
) -> (HashMap<GroupKey, String>, HashSet<String>, i32)
where
    F: Fn(&str) -> Option<(String, Option<String>)>,
{
    let mut by_key: HashMap<GroupKey, String> = HashMap::new();
    let mut used_aliases: HashSet<String> = HashSet::new();
    let next_sort = profiles.iter().map(|p| p.sort_order).max().unwrap_or(-1) + 1;
    for p in profiles {
        used_aliases.insert(p.alias.clone());
        let Some((key, vault_url)) = fetch_key(&p.api_key_ref) else {
            continue;
        };
        let explicit = first_non_empty(p.base_url.as_deref(), vault_url.as_deref());
        let gk = group_key(&p.provider, &p.model, explicit.as_deref(), &key);
        by_key.entry(gk).or_insert_with(|| p.id.clone());
    }
    (by_key, used_aliases, next_sort)
}
