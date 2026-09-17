//! ModelProfile 降级链生产装配与凭据解析（U3-2 ① 从 `commands::model_profile_cmd`
//! 下沉至此，破除「harness 反向依赖 commands」的层级倒置）。
//!
//! `production_fallback_plan` / `delegation_fallback_plan` / `ProfileFallbackResolver`
//! 原住在 `commands::model_profile_cmd`，却被 `harness::channel` / `harness::inbox`
//! / `harness::mcp::delegate` 调用——业务逻辑层反向依赖命令层。本模块把它们连同
//! 其依赖的凭据解析（`resolve_profile_credentials`、`ModelProfileWithCredentials`、
//! `ResolveProfileError`）一并下沉；commands 层经 re-export 保留原调用路径，
//! `agent_cmd` / `chat_cmd` / `provider_cmd` 零改动。
//!
//! 层序（自上而下）：commands → harness → infra/protocol → db。本模块属 harness，
//! 允许 AppHandle 与 SqlitePool（生产装配需要 Stronghold 解密 key、读 profile/agent 行）。

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::crypto;
use crate::db::models::{AgentRow, ModelProfileRow};
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::r#loop::fallback::{
    effective_output_cap, parse_fallback_ids, FallbackPlan, FallbackResolver, ResolvedModel,
};

// ============================================================================
// 类型：带凭据的 profile 数据（测试命令 / 视觉链解析用）
// ============================================================================

/// 视觉链、`test_provider_connection` 等消费方拼装调用所需的全部信息。
///
/// base_url 解析规则与 agent 汇聚点一致：DB 行非空优先，vault 记录兜底。
#[derive(Debug, Clone)]
pub struct ModelProfileWithCredentials {
    pub profile: ModelProfileRow,
    /// 解密后的 api_key（明文，仅过内存）
    pub api_key: String,
    pub base_url: Option<String>,
}

// ============================================================================
// 解析内核（agent 引用路径共用）
// ============================================================================

/// profile 凭据解析失败的两种语义（调用方分流处置的依据）：
///
/// - [`ResolveProfileError::NotFound`]：**行已删**——引用悬空是已知终态，
///   守卫兜底下降级（agent 行内快照）/ 跳档（链路测试 Skipped）合理；
/// - [`ResolveProfileError::Corrupted`]：**行在但凭据读不出**（Stronghold 槽位
///   缺失/JSON 损坏/DB 故障）——数据坏了是真实故障：静默换旧 Key 照跑会掩盖
///   问题、健康归因记错主档，调用方须诚实上抛或记健康 Failed。
///
/// 注意 `crypto::fetch_api_key` 对「槽位无记录」也返回 `AppError::NotFound`
/// （资源是 api_key 槽位）——能走到那一步说明行已查到，故统一归 Corrupted，
/// 只有 `get_by_id` 的 NotFound 才是行缺。
pub(crate) enum ResolveProfileError {
    NotFound { id: String },
    Corrupted(AppError),
}

impl std::fmt::Display for ResolveProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { id } => write!(f, "模型配置行不存在: {id}"),
            Self::Corrupted(e) => write!(f, "模型配置凭据不可读: {e}"),
        }
    }
}

impl From<ResolveProfileError> for AppError {
    fn from(e: ResolveProfileError) -> Self {
        match e {
            ResolveProfileError::NotFound { id } => AppError::NotFound {
                resource: "model_profile",
                id,
            },
            // Corrupted 保留原始错误（Stronghold/Json/Database…），不糊成 Internal
            ResolveProfileError::Corrupted(e) => e,
        }
    }
}

/// 解析 profile 出可用凭据（自由函数——`SqlAgentCmd` 的 agent 引用解析直接
/// 调它，不注入 `Arc<dyn ModelProfileCmd>`，模块耦合面不扩大）。
///
/// base_url 解析规则与 agent 汇聚点一致：DB 行非空优先，vault 记录兜底。
/// 失败语义二分见 [`ResolveProfileError`]。
pub(crate) async fn resolve_profile_credentials(
    app: &AppHandle,
    pool: &SqlitePool,
    profile_id: &str,
) -> Result<ModelProfileWithCredentials, ResolveProfileError> {
    let row = repo::model_profile::get_by_id(pool, profile_id)
        .await
        .map_err(|e| match e {
            AppError::NotFound { id, .. } => ResolveProfileError::NotFound { id },
            other => ResolveProfileError::Corrupted(other),
        })?;
    let (api_key, vault_base_url) = crypto::fetch_api_key(app, &row.api_key_ref)
        .map_err(ResolveProfileError::Corrupted)?;
    let base_url = row
        .base_url
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(vault_base_url.as_deref())
        .map(String::from);
    Ok(ModelProfileWithCredentials {
        profile: row,
        api_key,
        base_url,
    })
}

// ============================================================================
// 降级链生产 resolver（B2-S2）——纯定义在 harness::loop::fallback，此处是
// Stronghold + create_provider 的生产装配（loop 链 Tauri-free 纪律）
// ============================================================================

/// 生产档位解析器：profile → 完整可换入档位。
///
/// 适配器恒重建（`create_provider` 按新 provider/model 组装，构造成本可忽略）；
/// max_tokens 按 `effective_output_cap` 公式以新模型重算。resolve 的 Err 由
/// 调用方按「跳过该档」处理（对齐视觉链逐候选语义）。
pub struct ProfileFallbackResolver {
    app: AppHandle,
    pool: SqlitePool,
}

impl ProfileFallbackResolver {
    pub fn new(app: AppHandle, pool: SqlitePool) -> Self {
        Self { app, pool }
    }
}

#[async_trait]
impl FallbackResolver for ProfileFallbackResolver {
    async fn resolve(
        &self,
        profile_id: &str,
        agent_max_tokens: i32,
        cache_prompt: bool,
    ) -> AppResult<ResolvedModel> {
        let cred = resolve_profile_credentials(&self.app, &self.pool, profile_id).await?;
        let provider = crate::harness::provider::create_provider(
            &cred.profile.provider,
            &cred.profile.model,
            cred.base_url.as_deref(),
            cache_prompt,
        )?;
        Ok(ResolvedModel {
            profile_id: profile_id.to_string(),
            alias: cred.profile.alias.clone(),
            provider,
            api_key: cred.api_key,
            model: cred.profile.model.clone(),
            max_tokens: effective_output_cap(
                agent_max_tokens,
                &cred.profile.provider,
                &cred.profile.model,
            ),
        })
    }
}

/// 生产降级链组装（chat_cmd / delegate 两调用方共用）：
/// agent 行 `fallback_profile_ids` 解析链 + ProfileFallbackResolver。
/// 空链 → `FallbackPlan::empty()`（不建 resolver，行为与 legacy 逐字节等价）。
pub(crate) fn production_fallback_plan(
    app: &AppHandle,
    pool: &SqlitePool,
    agent: &AgentRow,
) -> FallbackPlan {
    let chain = parse_fallback_ids(agent.fallback_profile_ids.as_deref());
    fallback_plan_from_chain(app, pool, chain, agent)
}

/// 链 → FallbackPlan 的组装内核（production / 委派继承共用）。
/// resolve 参数恒取**运行 agent**（child）的行值——max_tokens 原值与
/// cache_prompt 是 agent 级偏好，不随链来自谁而变。
fn fallback_plan_from_chain(
    app: &AppHandle,
    pool: &SqlitePool,
    chain: Vec<String>,
    agent: &AgentRow,
) -> FallbackPlan {
    if chain.is_empty() {
        return FallbackPlan::empty();
    }
    FallbackPlan::new(
        chain,
        Arc::new(ProfileFallbackResolver::new(app.clone(), pool.clone())),
        agent.model_profile_id.clone(),
        agent.max_tokens,
        agent.cache_prompt != 0,
    )
}

/// 委派子会话的降级链决策（纯函数，0.7 批 A）：
/// - 子 agent 显式配链 → 原样用自己的（自己的韧性选择不被覆盖）；
/// - 无链 → **继承父 agent 的降级链**，并摘除与子 agent 主档同 id 的档
///   （换到与当前主档相同的 profile = 重试一次已知失败，纯浪费）。
///
/// 语义边界：继承的是父的**配置链**（静态），非父会话换档后的剩余链——
/// 已知失败档由换档机制自行跳过（Quota 类即时换档只多一次循环），
/// 子会话 cursor=0 重新起跑（父的换档进度是父回合的运行时事实）。
fn delegation_fallback_chain(child: &AgentRow, parent: Option<&AgentRow>) -> Vec<String> {
    let own = parse_fallback_ids(child.fallback_profile_ids.as_deref());
    if !own.is_empty() {
        return own;
    }
    let Some(parent) = parent else {
        return Vec::new();
    };
    let mut chain = parse_fallback_ids(parent.fallback_profile_ids.as_deref());
    if let Some(child_primary) = child.model_profile_id.as_deref() {
        chain.retain(|id| id != child_primary);
    }
    chain
}

/// 委派路径的降级链组装（delegate.rs 调用）：子 agent 无链时回退读父 agent
/// 行继承。父行读取失败**降级为无链**（warn 留痕）——链是韧性增强不是委派
/// 前提，绝不让继承失败阻塞委派本身。
pub(crate) async fn delegation_fallback_plan(
    app: &AppHandle,
    pool: &SqlitePool,
    child: &AgentRow,
    parent_agent_id: &str,
) -> FallbackPlan {
    if !parse_fallback_ids(child.fallback_profile_ids.as_deref()).is_empty() {
        return production_fallback_plan(app, pool, child);
    }
    let parent = repo::agent::get_by_id(pool, parent_agent_id).await.ok();
    if parent.is_none() {
        tracing::warn!(
            target: "ice_paw.fallback",
            parent = parent_agent_id,
            "父 agent 行读取失败，委派子会话降级为无链（不阻塞委派）"
        );
    }
    let chain = delegation_fallback_chain(child, parent.as_ref());
    fallback_plan_from_chain(app, pool, chain, child)
}

// ============================================================================
// 单元测试（委派降级链继承的纯函数决策，0.7 批 A 从 model_profile_cmd 随迁）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 委派降级链测试夹具：只关心 model_profile_id / fallback_profile_ids 两列
    fn deleg_agent_row(
        model_profile_id: Option<&str>,
        fallback_ids: Option<&str>,
    ) -> AgentRow {
        AgentRow {
            id: "a1".into(),
            name: "n".into(),
            provider: "glm".into(),
            model: "glm-5.3-flash".into(),
            system_prompt: String::new(),
            api_key_ref: String::new(),
            base_url: None,
            temperature: 0.7,
            max_tokens: 4096,
            extra_params: String::new(),
            sort_order: 0,
            cache_prompt: 0,
            max_history_messages: None,
            context_window: None,
            enabled_tools: None,
            tool_scopes: None,
            supports_vision: 0,
            description: String::new(),
            avatar: None,
            workspace_path: None,
            model_profile_id: model_profile_id.map(Into::into),
            fallback_profile_ids: fallback_ids.map(Into::into),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn delegation_chain_child_own_wins_over_parent() {
        // 子 agent 显式配链 → 原样用自己的，父链不参与
        let child = deleg_agent_row(Some("mp-child"), Some(r#"["mp-a","mp-b"]"#));
        let parent = deleg_agent_row(Some("mp-parent"), Some(r#"["mp-x"]"#));
        let chain = delegation_fallback_chain(&child, Some(&parent));
        assert_eq!(chain, vec!["mp-a".to_string(), "mp-b".to_string()]);
    }

    #[test]
    fn delegation_chain_inherits_parent_when_child_empty() {
        let child = deleg_agent_row(Some("mp-child"), None);
        let parent = deleg_agent_row(Some("mp-parent"), Some(r#"["mp-x","mp-y"]"#));
        let chain = delegation_fallback_chain(&child, Some(&parent));
        assert_eq!(chain, vec!["mp-x".to_string(), "mp-y".to_string()]);
    }

    #[test]
    fn delegation_chain_inheritance_drops_child_primary() {
        // 父链含子主档 → 摘除（换到当前主档 = 重试一次已知失败）
        let child = deleg_agent_row(Some("mp-a"), None);
        let parent = deleg_agent_row(None, Some(r#"["mp-a","mp-b","mp-a"]"#));
        let chain = delegation_fallback_chain(&child, Some(&parent));
        assert_eq!(chain, vec!["mp-b".to_string()]);
    }

    #[test]
    fn delegation_chain_parent_missing_or_chain_drained() {
        let child = deleg_agent_row(Some("mp-a"), None);
        // 父行读不到（None）→ 无链
        assert!(delegation_fallback_chain(&child, None).is_empty());
        // 父也无链 → 无链
        let no_chain_parent = deleg_agent_row(None, None);
        assert!(delegation_fallback_chain(&child, Some(&no_chain_parent)).is_empty());
        // 摘除后空链（父链只有子的主档）
        let only_primary = deleg_agent_row(None, Some(r#"["mp-a"]"#));
        assert!(delegation_fallback_chain(&child, Some(&only_primary)).is_empty());
    }

    #[test]
    fn delegation_chain_legacy_child_inherits_too() {
        // legacy 子 agent（手动主档、model_profile_id NULL）同样继承——
        // 换档机制对 legacy 主档 + 链 profile 的组合本就成立
        let child = deleg_agent_row(None, None);
        let parent = deleg_agent_row(None, Some(r#"["mp-x"]"#));
        let chain = delegation_fallback_chain(&child, Some(&parent));
        assert_eq!(chain, vec!["mp-x".to_string()]);
    }
}
