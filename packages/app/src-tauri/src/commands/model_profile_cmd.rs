//! ModelProfile（模型配置实体）相关 Tauri Commands + trait 抽象
//!
//! ## 背景（Phase 1）
//!
//! 一条 profile = provider + model + key 引用 + base_url + 别名（「智谱主力」
//! 「智谱 Coding」）。视觉读取 / 语义检索改为引用本实体（preferences 新键
//! `vision_profile_ids` / `embedding_profile_id`）；agent 链路 Phase 2 接入，
//! 本模块与 `get_with_credentials`（agent 侧汇聚点）无交集。
//!
//! key 密文存 Stronghold 槽位 `profile:{id}`（`crypto` 的参数本质就是槽位
//! 字符串，直接复用）；表内 `api_key_ref` 建行时恒等填入。
//!
//! ## trait 抽象边界（照搬 REQ-XC-010 模式）
//!
//! - 生产实现：`SqlModelProfileCmd` —— sqlx + stronghold
//! - 测试实现：`MockModelProfileCmd` —— 内存状态，可注入预置数据
//! - trait object：`Arc<dyn ModelProfileCmd>` 在 Tauri State 里传递
//!
//! ## 校验三闸（agent_cmd 同款纪律）
//!
//! 1. `validate_new_profile`：alias/provider/model 非空；keyed 厂商空 key 拒
//!    （ollama/custom 免鉴权放行，空串仍写 Stronghold 占位记录）；custom 必填 URL
//! 2. 换厂商闸：update 时 provider 变更且新厂商要 key → 必须同批带新 key
//!    （旧 key 属于旧厂商，端点跟随后打到新端点必然鉴权失败——换厂商配置
//!    分裂的 profile 版根治，语义同 AgentForm 前端闸）
//! 3. 删除引用守卫：被 `vision_profile_ids` / `embedding_profile_id` 引用 → 拒

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tauri::{AppHandle, State};
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::crypto;
use crate::db::models::{
    ModelProfile, ModelProfileRow, ModelProfileUpdate, NewModelProfile, RotateProfileKey,
    UserPreferences,
};
use crate::db::repo;
use crate::db::repo::model_profile::NewProfileRow;
use crate::error::{AppError, AppResult};
use crate::harness::provider::{
    provider_default_url, provider_requires_base_url, provider_requires_key,
};

// ============================================================================
// 入参校验 + 纯函数
// ============================================================================

/// `NewModelProfile` 入参校验（Sql / Mock 共用，镜像 `validate_new_agent`）。
///
/// 空 key 语义同 agent：免鉴权厂商（ollama/custom）允许空——空串仍会经
/// `crypto::store_api_key` 存一条空记录占位（Stronghold 无记录时
/// `fetch_api_key` 返回 NotFound，消费链路会报错）；custom 必须显式填端点。
fn validate_new_profile(input: &NewModelProfile) -> AppResult<()> {
    if input.alias.trim().is_empty() {
        return Err(AppError::Validation("别名不能为空".into()));
    }
    if input.provider.trim().is_empty() {
        return Err(AppError::Validation("provider 不能为空".into()));
    }
    if input.model.trim().is_empty() {
        return Err(AppError::Validation("model 不能为空".into()));
    }
    if provider_requires_key(input.provider.trim()) && input.api_key.trim().is_empty() {
        return Err(AppError::Validation("api_key 不能为空".into()));
    }
    if provider_requires_base_url(input.provider.trim())
        && input.base_url.as_deref().unwrap_or("").trim().is_empty()
    {
        return Err(AppError::Validation("custom 厂商必须填写端点 URL".into()));
    }
    Ok(())
}

/// 换厂商是否强制伴随新 key（纯函数便于测试）：仅当厂商真的变了、且新厂商
/// 要 key 时才拦——ollama → custom 这类免鉴权切换无需新 key。
fn provider_switch_requires_key(provider_changed: bool, new_provider: Option<&str>) -> bool {
    provider_changed && new_provider.is_some_and(|p| provider_requires_key(p.trim()))
}

/// 换厂商时未显式提供 base_url → 新厂商注册表默认地址（端点跟随厂商）。
/// agent_cmd.rs:274 的 profile 版（纯函数；custom/未知无默认 → None 保持不改）。
fn default_url_on_provider_switch(
    provider_changed: bool,
    new_provider: Option<&str>,
) -> Option<String> {
    if !provider_changed {
        return None;
    }
    new_provider
        .map(provider_default_url)
        .filter(|url| !url.is_empty())
}

/// base_url 双层 Option 解析（agent_cmd.rs:291 的 profile 版）：
/// 显式 `Some(opt)` → 照设/照清；absent → 仅「换厂商且有注册表默认」时跟随。
fn resolve_base_url_arg<'a>(
    explicit: Option<Option<&'a str>>,
    switch_url: Option<&'a str>,
) -> Option<Option<&'a str>> {
    match explicit {
        Some(opt) => Some(opt),
        None => switch_url.map(Some),
    }
}

/// 删除守卫判定（纯函数）：该 profile 被哪些引用使用。返回 None = 可删。
///
/// 引用只在**新格式键**上有意义（`Some` 权威）；`None` = 旧格式仍在用，
/// 旧格式的 key 内嵌在 vision_config / embedding 四键里、不引用 profile 表，
/// 恒不构成引用。
fn referenced_by(prefs: &UserPreferences, id: &str) -> Option<String> {
    let mut uses: Vec<&str> = Vec::new();
    if prefs
        .vision_profile_ids
        .as_ref()
        .is_some_and(|ids| ids.iter().any(|x| x == id))
    {
        uses.push("视觉读取");
    }
    if prefs.embedding_profile_id.as_deref() == Some(id) {
        uses.push("语义检索");
    }
    if uses.is_empty() {
        None
    } else {
        Some(uses.join("、"))
    }
}

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

/// 解析 profile 出可用凭据（自由函数——`SqlAgentCmd` 的 agent 引用解析直接
/// 调它，不注入 `Arc<dyn ModelProfileCmd>`，模块耦合面不扩大）。
///
/// base_url 解析规则与 agent 汇聚点一致：DB 行非空优先，vault 记录兜底。
pub(crate) async fn resolve_profile_credentials(
    app: &AppHandle,
    pool: &SqlitePool,
    profile_id: &str,
) -> AppResult<ModelProfileWithCredentials> {
    let row = repo::model_profile::get_by_id(pool, profile_id).await?;
    let (api_key, vault_base_url) = crypto::fetch_api_key(app, &row.api_key_ref)?;
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
impl crate::harness::r#loop::fallback::FallbackResolver for ProfileFallbackResolver {
    async fn resolve(
        &self,
        profile_id: &str,
        agent_max_tokens: i32,
        cache_prompt: bool,
    ) -> AppResult<crate::harness::r#loop::fallback::ResolvedModel> {
        use crate::harness::r#loop::fallback::{effective_output_cap, ResolvedModel};
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
    agent: &crate::db::models::AgentRow,
) -> crate::harness::r#loop::fallback::FallbackPlan {
    use crate::harness::r#loop::fallback::{parse_fallback_ids, FallbackPlan};
    let chain = parse_fallback_ids(agent.fallback_profile_ids.as_deref());
    if chain.is_empty() {
        return FallbackPlan::empty();
    }
    FallbackPlan::new(
        chain,
        Arc::new(ProfileFallbackResolver::new(app.clone(), pool.clone())),
        agent.model_profile_id.clone(),
        // resolve 参数：agent 行 max_tokens **原值**（非主档策展抬升后的有效值）
        // + 主档 cache_prompt——见 FallbackPlan 字段注释
        agent.max_tokens,
        agent.cache_prompt != 0,
    )
}

// ============================================================================
// trait ModelProfileCmd
// ============================================================================

#[async_trait]
pub trait ModelProfileCmd: Send + Sync {
    /// 列出全部 profile（不含敏感字段，按 sort_order 排序）
    async fn list(&self) -> AppResult<Vec<ModelProfile>>;

    /// 取单条元数据（DTO）
    async fn get(&self, profile_id: &str) -> AppResult<ModelProfile>;

    /// 创建 profile（含 key 写入 Stronghold 槽位 profile:{id}）
    async fn create(&self, input: NewModelProfile) -> AppResult<ModelProfile>;

    /// 部分更新（provider 变更需同批带新 key——换厂商闸）
    async fn update(&self, input: ModelProfileUpdate) -> AppResult<ModelProfile>;

    /// 单独轮换 api_key
    async fn rotate_key(&self, input: RotateProfileKey) -> AppResult<ModelProfile>;

    /// 删除 profile（被视觉/检索引用时拒绝——删除引用守卫）
    async fn delete(&self, profile_id: &str) -> AppResult<()>;

    /// 取 profile + 解密后的凭据（测试命令 / 视觉链解析专用）
    async fn get_with_credentials(
        &self,
        profile_id: &str,
    ) -> AppResult<ModelProfileWithCredentials>;

    /// 记录健康状态（状态监控，warn-only 旁路——测试命令结果回写用；slug 见
    /// `harness::profile_health::ProfileHealth::as_str`）。绝不返回 Err。
    async fn record_health(&self, profile_id: &str, health: &str, detail: Option<&str>);
}

// ============================================================================
// SqlModelProfileCmd —— 生产实现（sqlx + stronghold）
// ============================================================================

pub struct SqlModelProfileCmd {
    app: AppHandle,
    pool: SqlitePool,
}

impl SqlModelProfileCmd {
    pub fn new(app: AppHandle, pool: SqlitePool) -> Self {
        Self { app, pool }
    }

    /// 组装前端 DTO：`has_api_key` 真相实查（同 `agent_dto` 规则——免 key 厂商
    /// 恒 true；要求 key 的厂商查 Stronghold 记录存在）。
    fn profile_dto(&self, row: ModelProfileRow) -> ModelProfile {
        let has_api_key = !provider_requires_key(&row.provider)
            || crypto::has_api_key(&self.app, &row.api_key_ref).unwrap_or(false);
        let mut dto = ModelProfile::from(row);
        dto.has_api_key = has_api_key;
        dto
    }
}

#[async_trait]
impl ModelProfileCmd for SqlModelProfileCmd {
    async fn list(&self) -> AppResult<Vec<ModelProfile>> {
        let rows = repo::model_profile::list(&self.pool).await?;
        Ok(rows.into_iter().map(|row| self.profile_dto(row)).collect())
    }

    async fn get(&self, profile_id: &str) -> AppResult<ModelProfile> {
        let row = repo::model_profile::get_by_id(&self.pool, profile_id).await?;
        Ok(self.profile_dto(row))
    }

    async fn create(&self, input: NewModelProfile) -> AppResult<ModelProfile> {
        validate_new_profile(&input)?;
        let id = Uuid::new_v4().to_string();
        let slot = format!("profile:{id}");

        crypto::store_api_key(&self.app, &slot, &input.api_key, input.base_url.as_deref())?;

        // 追加到列表末尾（现有最大 sort_order + 1；表小，全量读无压力）
        let next_order = repo::model_profile::list(&self.pool)
            .await?
            .iter()
            .map(|r| r.sort_order)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);

        let base_url = input
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let new_row = NewProfileRow {
            id: id.clone(),
            alias: input.alias.trim().to_string(),
            provider: input.provider.trim().to_string(),
            model: input.model.trim().to_string(),
            api_key_ref: slot.clone(),
            base_url,
            sort_order: next_order,
        };
        if let Err(e) = repo::model_profile::create(&self.pool, &new_row, false).await {
            // DB 写失败回滚刚落的 Stronghold 槽位，避免幽灵密钥
            if let Err(ke) = crypto::delete_api_key(&self.app, &slot) {
                tracing::warn!(target: "ice_paw.model_profile", "回滚 profile 槽位 {slot} 失败: {ke}");
            }
            return Err(e);
        }
        let row = repo::model_profile::get_by_id(&self.pool, &id).await?;
        Ok(self.profile_dto(row))
    }

    async fn update(&self, input: ModelProfileUpdate) -> AppResult<ModelProfile> {
        let old = repo::model_profile::get_by_id(&self.pool, &input.id).await?;

        // 换厂商闸：旧 key 属于旧厂商，端点跟随后打到新端点必然鉴权失败——
        // 要求 key 与厂商同批到达（前端编辑表单在切厂商时显式展示 Key 输入框）
        let provider_changed = input
            .provider
            .as_deref()
            .is_some_and(|new| new != old.provider);
        if provider_switch_requires_key(provider_changed, input.provider.as_deref())
            && input
                .api_key
                .as_deref()
                .map(str::trim)
                .is_none_or(str::is_empty)
        {
            return Err(AppError::Validation(
                "更换厂商时必须同时提供新厂商的 API Key：旧 Key 属于原厂商，打到新端点必然鉴权失败。请填入新厂商的 Key 后保存".into(),
            ));
        }

        // 端点跟随厂商（absent + 换厂商有默认 → 跟随；显式设/清照办）
        let switch_url =
            default_url_on_provider_switch(provider_changed, input.provider.as_deref());
        let base_url_arg = resolve_base_url_arg(
            input.base_url.as_ref().map(|o| o.as_deref()),
            switch_url.as_deref(),
        );

        // 顺带换 Key（可选）：vault 的 base_url 副本跟随最终 DB 值，保持兜底可用
        if let Some(key) = input
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            let final_base_url = match &base_url_arg {
                Some(opt) => opt.map(String::from),
                None => old.base_url.clone(),
            };
            crypto::store_api_key(&self.app, &old.api_key_ref, key, final_base_url.as_deref())?;
        }

        let row = repo::model_profile::update(
            &self.pool,
            &input.id,
            input.alias.as_deref().map(str::trim),
            input.provider.as_deref().map(str::trim),
            input.model.as_deref().map(str::trim),
            base_url_arg,
            input.sort_order,
        )
        .await?;

        if provider_changed && switch_url.is_some() && input.base_url.is_none() {
            tracing::info!(
                target: "ice_paw.model_profile",
                "profile {} 换厂商（{} → {}）且未显式提供地址：base_url 已重置为注册表默认 {}",
                row.id,
                old.provider,
                row.provider,
                row.base_url.as_deref().unwrap_or("")
            );
        }

        Ok(self.profile_dto(row))
    }

    async fn rotate_key(&self, input: RotateProfileKey) -> AppResult<ModelProfile> {
        // key 是否必填按该 profile 的 provider 判定（免鉴权厂商允许空——
        // 空串仍走 store，清掉旧值）
        let row = repo::model_profile::get_by_id(&self.pool, &input.profile_id).await?;
        if provider_requires_key(&row.provider) && input.api_key.trim().is_empty() {
            return Err(AppError::Validation("api_key 不能为空".into()));
        }
        // vault 副本跟随显式值；未显式给则跟随 DB 行现值（保持兜底可用）
        let vault_base_url = input.base_url.as_deref().or(row.base_url.as_deref());
        crypto::store_api_key(&self.app, &row.api_key_ref, &input.api_key, vault_base_url)?;
        // DB 列仅在显式提供时更新（Some=设/清，absent=不改）
        if input.base_url.is_some() {
            repo::model_profile::update(
                &self.pool,
                &input.profile_id,
                None,
                None,
                None,
                Some(input.base_url.as_deref()),
                None,
            )
            .await?;
        }
        let fresh = repo::model_profile::get_by_id(&self.pool, &input.profile_id).await?;
        Ok(self.profile_dto(fresh))
    }

    async fn delete(&self, profile_id: &str) -> AppResult<()> {
        // 删除引用守卫：先于一切清理动作——被引用的 profile 删了会让视觉/
        // 检索链在解析时静默跳过（warn 一条用户看不见）
        let prefs = repo::preferences::get_all(&self.pool).await?;
        if let Some(where_used) = referenced_by(&prefs, profile_id) {
            return Err(AppError::Validation(format!(
                "该模型配置正在被「{where_used}」引用，无法删除。请先到「设置 → 模型」解除引用，再回来删除"
            )));
        }
        // agent 腿（Phase 2）：主档引用或降级链任一命中即拦——悬空引用虽读侧
        // 降级 legacy 快照继续可用，但「删除成功后 agent 悄悄用旧快照」对用户
        // 不可见，宁拦勿悬
        let agents = repo::model_profile::agents_referencing(&self.pool, profile_id).await?;
        if !agents.is_empty() {
            let names: Vec<&str> = agents.iter().map(|(_, name)| name.as_str()).collect();
            return Err(AppError::Validation(format!(
                "该模型配置正在被 Agent「{}」引用，无法删除。请先在 Agent 设置里解除引用或调整降级链，再回来删除",
                names.join("、")
            )));
        }
        let row = repo::model_profile::get_by_id(&self.pool, profile_id).await?;
        // 先清 Stronghold 槽位（容错：失败仅 warn，不阻断删除）
        if let Err(e) = crypto::delete_api_key(&self.app, &row.api_key_ref) {
            tracing::warn!(target: "ice_paw.model_profile", "清理 profile {profile_id} API key 失败: {e}");
        }
        repo::model_profile::delete(&self.pool, profile_id).await
    }

    async fn get_with_credentials(
        &self,
        profile_id: &str,
    ) -> AppResult<ModelProfileWithCredentials> {
        resolve_profile_credentials(&self.app, &self.pool, profile_id).await
    }

    async fn record_health(&self, profile_id: &str, health: &str, detail: Option<&str>) {
        let clipped = detail.map(crate::harness::profile_health::clip_detail);
        if let Err(e) =
            repo::model_profile::record_health(&self.pool, profile_id, health, clipped.as_deref())
                .await
        {
            tracing::warn!(target: "ice_paw.profile_health", "记录模型配置 {profile_id} 健康状态失败: {e}");
        }
    }
}

// ============================================================================
// MockModelProfileCmd —— 测试实现（内存状态）
// ============================================================================

/// 测试用 ModelProfileCmd 实现：内存 HashMap 存储 profile 行 + 凭据 +
/// 调用日志（照搬 MockAgentCmd 形态）。
type MockEntry = (ModelProfileRow, String, Option<String>);
type MockStore = HashMap<String, MockEntry>;
pub struct MockModelProfileCmd {
    inner: std::sync::Mutex<MockStore>,
    calls: std::sync::Mutex<Vec<String>>,
}

impl MockModelProfileCmd {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(HashMap::new()),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// 预置一条 profile（row + api_key + base_url 覆盖值）
    pub fn seed(&self, row: ModelProfileRow, api_key: String, base_url: Option<String>) {
        self.inner
            .lock()
            .unwrap()
            .insert(row.id.clone(), (row, api_key, base_url));
    }

    fn log(&self, entry: String) {
        self.calls.lock().unwrap().push(entry);
    }

    /// 取回调用日志（测试断言用）
    pub fn call_log(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Default for MockModelProfileCmd {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ModelProfileCmd for MockModelProfileCmd {
    async fn list(&self) -> AppResult<Vec<ModelProfile>> {
        self.log("list".into());
        let g = self.inner.lock().unwrap();
        let mut items: Vec<MockEntry> = g.values().cloned().collect();
        items.sort_by_key(|(row, _, _)| row.sort_order);
        Ok(items
            .into_iter()
            .map(|(row, key, _)| {
                let has = !provider_requires_key(&row.provider) || !key.is_empty();
                let mut dto = ModelProfile::from(row);
                dto.has_api_key = has;
                dto
            })
            .collect())
    }

    async fn get(&self, profile_id: &str) -> AppResult<ModelProfile> {
        self.log(format!("get({profile_id})"));
        let g = self.inner.lock().unwrap();
        let (row, key, _) = g.get(profile_id).ok_or_else(|| AppError::NotFound {
            resource: "model_profile",
            id: profile_id.to_string(),
        })?;
        let mut dto = ModelProfile::from(row.clone());
        dto.has_api_key = !provider_requires_key(&row.provider) || !key.is_empty();
        Ok(dto)
    }

    async fn create(&self, input: NewModelProfile) -> AppResult<ModelProfile> {
        self.log(format!("create({})", input.alias));
        validate_new_profile(&input)?;
        let id = Uuid::new_v4().to_string();
        let row = ModelProfileRow {
            api_key_ref: format!("profile:{id}"),
            id,
            alias: input.alias.trim().to_string(),
            provider: input.provider.trim().to_string(),
            model: input.model.trim().to_string(),
            base_url: input.base_url.clone(),
            sort_order: 0,
            created_at: "2024-01-01 00:00:00".to_string(),
            updated_at: "2024-01-01 00:00:00".to_string(),
            last_health: None,
            last_health_detail: None,
            last_health_at: None,
        };
        let dto = ModelProfile {
            has_api_key: !provider_requires_key(&row.provider) || !input.api_key.is_empty(),
            ..ModelProfile::from(row.clone())
        };
        self.inner
            .lock()
            .unwrap()
            .insert(row.id.clone(), (row, input.api_key, input.base_url));
        Ok(dto)
    }

    async fn update(&self, input: ModelProfileUpdate) -> AppResult<ModelProfile> {
        self.log(format!("update({})", input.id));
        let mut g = self.inner.lock().unwrap();
        let (row, key, base_url) = g.get_mut(&input.id).ok_or_else(|| AppError::NotFound {
            resource: "model_profile",
            id: input.id.clone(),
        })?;
        if let Some(v) = input.alias {
            row.alias = v.trim().to_string();
        }
        if let Some(v) = input.provider {
            row.provider = v.trim().to_string();
        }
        if let Some(v) = input.model {
            row.model = v.trim().to_string();
        }
        if let Some(v) = input.api_key {
            *key = v;
        }
        if let Some(v) = input.base_url {
            *base_url = v;
        }
        if let Some(v) = input.sort_order {
            row.sort_order = v;
        }
        let has = !provider_requires_key(&row.provider) || !key.is_empty();
        let mut dto = ModelProfile::from(row.clone());
        dto.has_api_key = has;
        Ok(dto)
    }

    async fn rotate_key(&self, input: RotateProfileKey) -> AppResult<ModelProfile> {
        self.log(format!("rotate_key({})", input.profile_id));
        let mut g = self.inner.lock().unwrap();
        let (row, key, base_url) =
            g.get_mut(&input.profile_id)
                .ok_or_else(|| AppError::NotFound {
                    resource: "model_profile",
                    id: input.profile_id.clone(),
                })?;
        *key = input.api_key.clone();
        if let Some(bu) = input.base_url.clone() {
            *base_url = Some(bu);
        }
        Ok(ModelProfile::from(row.clone()))
    }

    async fn delete(&self, profile_id: &str) -> AppResult<()> {
        self.log(format!("delete({profile_id})"));
        self.inner
            .lock()
            .unwrap()
            .remove(profile_id)
            .ok_or_else(|| AppError::NotFound {
                resource: "model_profile",
                id: profile_id.to_string(),
            })?;
        Ok(())
    }

    async fn get_with_credentials(
        &self,
        profile_id: &str,
    ) -> AppResult<ModelProfileWithCredentials> {
        self.log(format!("get_with_credentials({profile_id})"));
        let g = self.inner.lock().unwrap();
        let (row, key, base_url) = g.get(profile_id).ok_or_else(|| AppError::NotFound {
            resource: "model_profile",
            id: profile_id.to_string(),
        })?;
        Ok(ModelProfileWithCredentials {
            base_url: row
                .base_url
                .clone()
                .filter(|s| !s.is_empty())
                .or(base_url.clone()),
            profile: row.clone(),
            api_key: key.clone(),
        })
    }

    async fn record_health(&self, profile_id: &str, health: &str, _detail: Option<&str>) {
        self.log(format!("record_health({profile_id},{health})"));
    }
}

// ============================================================================
// Tauri command 包装
// ============================================================================

/// 列出全部模型配置
#[tauri::command]
pub async fn list_model_profiles(
    cmd: State<'_, Arc<dyn ModelProfileCmd>>,
) -> AppResult<Vec<ModelProfile>> {
    cmd.inner().list().await
}

/// 创建模型配置
#[tauri::command]
pub async fn create_model_profile(
    cmd: State<'_, Arc<dyn ModelProfileCmd>>,
    input: NewModelProfile,
) -> AppResult<ModelProfile> {
    cmd.inner().create(input).await
}

/// 部分更新模型配置
#[tauri::command]
pub async fn update_model_profile(
    cmd: State<'_, Arc<dyn ModelProfileCmd>>,
    input: ModelProfileUpdate,
) -> AppResult<ModelProfile> {
    cmd.inner().update(input).await
}

/// 轮换模型配置的 api_key
#[tauri::command]
pub async fn rotate_model_profile_key(
    cmd: State<'_, Arc<dyn ModelProfileCmd>>,
    input: RotateProfileKey,
) -> AppResult<ModelProfile> {
    cmd.inner().rotate_key(input).await
}

/// 删除模型配置（被引用时拒绝）
#[tauri::command]
pub async fn delete_model_profile(
    cmd: State<'_, Arc<dyn ModelProfileCmd>>,
    id: String,
) -> AppResult<()> {
    cmd.inner().delete(&id).await
}

/// 视觉链健康检查：用**已保存的模型配置**代读一张 1×1 探针图——服务端取存量
/// 凭据（key 密文永不回显前端，`test_provider_connection` 的 profile_id 腿同构）。
/// 视觉引用卡逐条「测试」走它；端点推导与正式代读同源（行显式 > vault 副本 >
/// 注册表 openai_url），测过 = 同参数正式链路可用。
///
/// 状态监控：探针是真实调用，结果沉淀为该 profile 的最新健康状态（模型页状态点）。
#[tauri::command]
pub async fn test_model_profile_vision(
    cmd: State<'_, Arc<dyn ModelProfileCmd>>,
    profile_id: String,
) -> AppResult<super::preferences_cmd::VisionTestResult> {
    let cred = cmd.inner().get_with_credentials(&profile_id).await?;
    let base_url = cred
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            crate::harness::provider::provider_openai_url(&cred.profile.provider)
                .map(String::from)
        })
        .ok_or_else(|| {
            AppError::Validation(format!(
                "模型配置「{}」的厂商（{}）无 OpenAI 兼容视觉端点——请在该配置里填写自定义端点后再试",
                cred.profile.alias, cred.profile.provider
            ))
        })?;
    let result = super::preferences_cmd::probe_describe_image(
        &cred.profile.provider,
        &cred.profile.model,
        &base_url,
        &cred.api_key,
    )
    .await;
    match &result {
        Ok(_) => cmd.inner().record_health(&profile_id, "ok", None).await,
        Err(e) => {
            use crate::harness::profile_health::{clip_detail, health_from_error};
            cmd.inner()
                .record_health(
                    &profile_id,
                    health_from_error(&e.to_string()).as_str(),
                    Some(&clip_detail(&e.to_string())),
                )
                .await;
        }
    }
    result
}

// ============================================================================
// 单元测试（纯函数 + Mock；DB 路径由 repo 测试与集成测试覆盖）
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(id: &str, alias: &str) -> ModelProfileRow {
        ModelProfileRow {
            id: id.to_string(),
            alias: alias.to_string(),
            provider: "glm".to_string(),
            model: "glm-5.3-flash".to_string(),
            api_key_ref: format!("profile:{id}"),
            base_url: None,
            sort_order: 0,
            created_at: "2024-01-01 00:00:00".to_string(),
            updated_at: "2024-01-01 00:00:00".to_string(),
            last_health: None,
            last_health_detail: None,
            last_health_at: None,
        }
    }

    fn new_profile(provider: &str, api_key: &str) -> NewModelProfile {
        NewModelProfile {
            alias: "智谱主力".into(),
            provider: provider.into(),
            model: "glm-5.3-flash".into(),
            api_key: api_key.into(),
            base_url: None,
        }
    }

    // ---------------- validate_new_profile ----------------

    #[test]
    fn validate_allows_empty_key_for_ollama_and_custom() {
        let mut ollama = new_profile("ollama", "");
        ollama.base_url = None;
        assert!(validate_new_profile(&ollama).is_ok());
        // custom 必填 URL
        let mut custom = new_profile("custom", "");
        custom.base_url = None;
        assert!(matches!(
            validate_new_profile(&custom),
            Err(AppError::Validation(_))
        ));
        custom.base_url = Some("http://127.0.0.1:8000/v1".into());
        assert!(validate_new_profile(&custom).is_ok());
    }

    #[test]
    fn validate_rejects_empty_key_for_keyed_providers() {
        for p in [
            "openai",
            "glm",
            "glm-coding",
            "deepseek",
            "anthropic",
            "minimax",
        ] {
            let err = validate_new_profile(&new_profile(p, "  ")).unwrap_err();
            assert!(matches!(err, AppError::Validation(_)), "{p} 空 key 应被拒");
        }
        // 未知 provider 保守按需要 key 处理
        assert!(validate_new_profile(&new_profile("totally-unknown", "")).is_err());
    }

    #[test]
    fn validate_rejects_empty_fields() {
        let mut a = new_profile("ollama", "");
        a.alias = "  ".into();
        assert!(matches!(
            validate_new_profile(&a),
            Err(AppError::Validation(_))
        ));
        let mut b = new_profile("ollama", "");
        b.provider = "".into();
        assert!(matches!(
            validate_new_profile(&b),
            Err(AppError::Validation(_))
        ));
        let mut c = new_profile("ollama", "");
        c.model = "".into();
        assert!(matches!(
            validate_new_profile(&c),
            Err(AppError::Validation(_))
        ));
    }

    // ---------------- 换厂商闸 + 端点跟随 ----------------

    #[test]
    fn provider_switch_gate_only_for_keyed_providers() {
        // 换到要 key 的厂商 → 必须伴随新 key
        assert!(provider_switch_requires_key(true, Some("glm")));
        // 免鉴权厂商切换 → 不拦
        assert!(!provider_switch_requires_key(true, Some("ollama")));
        assert!(!provider_switch_requires_key(true, Some("custom")));
        // 未换厂商 → 不拦（改别名/模型/排序不该被 key 闸挡住）
        assert!(!provider_switch_requires_key(false, Some("glm")));
        assert!(!provider_switch_requires_key(true, None));
    }

    #[test]
    fn profile_url_follows_provider_on_switch() {
        // 未换厂商：不注入默认地址（保持「不改」语义）
        assert_eq!(default_url_on_provider_switch(false, Some("glm")), None);
        // 换厂商：注册表默认地址
        assert_eq!(
            default_url_on_provider_switch(true, Some("glm")).as_deref(),
            Some("https://open.bigmodel.cn/api/paas/v4")
        );
        // custom / 未知 → None（保持不改）
        assert_eq!(default_url_on_provider_switch(true, Some("custom")), None);
        assert_eq!(default_url_on_provider_switch(true, Some("nope")), None);
    }

    #[test]
    fn profile_base_url_absent_keeps_endpoint() {
        assert_eq!(resolve_base_url_arg(None, None), None);
        assert_eq!(
            resolve_base_url_arg(None, Some("https://api.deepseek.com")),
            Some(Some("https://api.deepseek.com"))
        );
        assert_eq!(
            resolve_base_url_arg(Some(Some("https://x.example")), None),
            Some(Some("https://x.example"))
        );
        assert_eq!(resolve_base_url_arg(Some(None), None), Some(None));
    }

    /// 换厂商闸的 serde 面：api_key 缺席 = None（不带 key 的普通更新）
    #[test]
    fn update_payload_without_key_deserializes() {
        let u: ModelProfileUpdate =
            serde_json::from_str(r#"{"id":"mp1","alias":"新别名"}"#).unwrap();
        assert_eq!(u.api_key, None);
        assert_eq!(u.alias.as_deref(), Some("新别名"));
    }

    // ---------------- 删除引用守卫 ----------------

    #[test]
    fn referenced_by_detects_vision_and_embedding() {
        let mut prefs = UserPreferences::default();
        // 旧格式（None）恒不构成引用
        assert_eq!(referenced_by(&prefs, "mp1"), None);

        prefs.vision_profile_ids = Some(vec!["mp1".into(), "mp2".into()]);
        assert_eq!(referenced_by(&prefs, "mp1").as_deref(), Some("视觉读取"));
        assert_eq!(referenced_by(&prefs, "mp3"), None);

        // 双引用 → 合并文案
        prefs.embedding_profile_id = Some("mp1".into());
        assert_eq!(
            referenced_by(&prefs, "mp1").as_deref(),
            Some("视觉读取、语义检索")
        );

        // 显式清空（Some(vec![])）= 权威无引用
        prefs.vision_profile_ids = Some(vec![]);
        assert_eq!(referenced_by(&prefs, "mp1").as_deref(), Some("语义检索"));
    }

    // ---------------- Mock ----------------

    #[tokio::test]
    async fn mock_list_and_get_roundtrip() {
        let mock = MockModelProfileCmd::new();
        mock.seed(sample_row("mp1", "智谱主力"), "k1".into(), None);

        let list = mock.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].alias, "智谱主力");
        // keyed 厂商 + 已 seed key → 徽标真
        assert!(list[0].has_api_key);
    }

    #[tokio::test]
    async fn mock_get_with_credentials_prefers_row_base_url() {
        let mock = MockModelProfileCmd::new();
        let mut row = sample_row("mp1", "智谱主力");
        row.base_url = Some("https://row.example".into());
        mock.seed(row, "secret".into(), Some("https://vault.example".into()));

        let r = mock.get_with_credentials("mp1").await.unwrap();
        assert_eq!(r.api_key, "secret");
        // DB 行非空优先，vault 兜底
        assert_eq!(r.base_url.as_deref(), Some("https://row.example"));
    }

    #[tokio::test]
    async fn mock_rotate_and_delete() {
        let mock = MockModelProfileCmd::new();
        mock.seed(sample_row("mp1", "智谱主力"), "old".into(), None);

        mock.rotate_key(RotateProfileKey {
            profile_id: "mp1".into(),
            api_key: "new".into(),
            base_url: None,
        })
        .await
        .unwrap();
        assert_eq!(
            mock.get_with_credentials("mp1").await.unwrap().api_key,
            "new"
        );

        mock.delete("mp1").await.unwrap();
        assert!(mock.get("mp1").await.is_err());

        let log = mock.call_log();
        assert!(log.contains(&"rotate_key(mp1)".to_string()));
        assert!(log.contains(&"delete(mp1)".to_string()));
    }

    #[tokio::test]
    async fn trait_object_works_for_mock() {
        async fn exercise(cmd: Arc<dyn ModelProfileCmd>) -> AppResult<usize> {
            Ok(cmd.list().await?.len())
        }
        let mock: Arc<dyn ModelProfileCmd> = Arc::new(MockModelProfileCmd::new());
        assert_eq!(exercise(mock).await.unwrap(), 0);
    }
}
