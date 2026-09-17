//! Agent 入参校验 + 换厂商端点跟随纯函数（从 `agent_cmd.rs` God module 拆出，U3-5 ③）。
//!
//! 全部纯函数（无 IO / 无 sqlx / 无 stronghold），便于单测回归。⚠️ 注意
//! `default_url_on_provider_switch` / `resolve_base_url_arg` 与 `model_profile_cmd.rs`
//! 的同名镜像函数是两份独立副本（`NewAgent` vs `NewModelProfile` 入参形态不同，
//! 但这两个纯函数的语义完全相同——未来若要收敛需两处同步，勿单边改语义）。

use crate::db::models::{AgentUpdate, NewAgent};
use crate::error::{AppError, AppResult};
use crate::harness::provider::{
    provider_default_url, provider_requires_base_url, provider_requires_key,
};

// ============================================================================
// 入参校验
// ============================================================================

/// `NewAgent` 入参校验（`SqlAgentCmd::create` / `MockAgentCmd::create` 共用）。
///
/// api_key 是否必填按 provider 目录判定（`provider_requires_key`）：
/// ollama / custom 等本地或免鉴权服务允许空 key——**空串仍会经
/// `crypto::store_api_key` 存一条空记录**（Stronghold 无记录时
/// `fetch_api_key` 返回 NotFound，聊天链路 `get_with_credentials` 会报错，
/// 所以必须占位）；OpenAI adapter 发空 Bearer，本地服务忽略。
///
/// ModelProfile Phase 2：引用模式（`model_profile_id` 非空）跳过 provider/
/// model/api_key 必填——模型身份来自 profile，api_key 空串占位（agent 自身
/// 槽位不再被引用路径读取）；降级链非空时要求主档存在。
pub(crate) fn validate_new_agent(input: &NewAgent) -> AppResult<()> {
    if input.id.trim().is_empty() {
        return Err(AppError::Validation("ID 不能为空".into()));
    }
    if input.name.trim().is_empty() {
        return Err(AppError::Validation("name 不能为空".into()));
    }
    if input
        .model_profile_id
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty())
    {
        validate_fallback_has_primary(
            true,
            input
                .fallback_profile_ids
                .as_ref()
                .is_some_and(|v| !v.is_empty()),
        )?;
        return Ok(());
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
    Ok(())
}

/// 降级链依赖主档：非空链必须搭配主档引用（链的起点是主 profile，legacy
/// agent 无主档链无处生效）。纯函数，create/update 共用（两处入参形态不同，
/// 链是否非空由调用方算好传入）。
fn validate_fallback_has_primary(has_primary: bool, chain_nonempty: bool) -> AppResult<()> {
    if chain_nonempty && !has_primary {
        return Err(AppError::Validation(
            "降级链需要先选择主模型（引用的模型配置）".into(),
        ));
    }
    Ok(())
}

/// 手动新建字段能否物化为 ModelProfile 实体（Phase 3，2026-09-08 拍板：新建
/// 表单保留手写配置、保存时自动转实体）。UI 路径经 `validate_new_agent` 后
/// 恒真；旁路（提案 CreateAgent 等）凭据不齐时为 false → legacy 行照常可用
/// （不硬造 keyless 实体），编辑时选实体即转正。判定与 boot 存量抽离
/// （agent_profile_migration）的跳过条件同构。
pub(crate) fn manual_materializable(input: &NewAgent) -> bool {
    let provider = input.provider.trim();
    if provider.is_empty() || input.model.trim().is_empty() {
        return false;
    }
    if provider_requires_key(provider) && input.api_key.trim().is_empty() {
        return false;
    }
    let explicit_url = input
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    !provider_requires_base_url(provider) || explicit_url.is_some()
}

/// AgentUpdate 冲突校验（纯函数，便于测试回归）。
///
/// 设引用（Some(Some)）与 provider/model/base_url 同批 → 拒：快照列族由
/// profile 解析产生，同批手填会被下一轮解析覆盖，形成「看似改了实则没改」
/// 的假象。解除引用（Some(None)）与手填同批**合法**——切回手动模式一并完成
/// （AgentForm 手动模式提交正是这个形状）。降级链检查走
/// [`validate_fallback_has_primary`]（目标态 = input 显式值优先、否则旧行值）。
///
/// 引用态行（旧行 model_profile_id 非空）未显式解除引用（字段缺席 = 引用
/// 保持）+ 手填快照族 → 同理拒：行仍挂在引用上，快照列入库后下轮解析即覆盖
/// 回实体值，用户批准的换模型静默失效，agent.yaml 镜像还会把永不生效的值写
/// 进文件（审查实案：提案卡批准 update_agent 直传快照族、不带
/// model_profile_id——绕过前端表单的引用态锁定）。
pub(crate) fn validate_update_model_fields_conflict(
    input: &AgentUpdate,
    old_has_primary: bool,
) -> AppResult<()> {
    if matches!(input.model_profile_id, Some(Some(_)))
        && (input.provider.is_some() || input.model.is_some() || input.base_url.is_some())
    {
        return Err(AppError::Validation(
            "设模型引用时不能同时修改厂商/模型/端点——请先保存引用，再单独调整".into(),
        ));
    }
    // 引用态行 + 引用字段缺席 + 手填快照族 → 拒。Some(None)（显式解除回
    // legacy）+ 手填 = 合法（切回手动模式一并完成），不进本分支。
    if old_has_primary
        && input.model_profile_id.is_none()
        && (input.provider.is_some() || input.model.is_some() || input.base_url.is_some())
    {
        return Err(AppError::Validation(
            "该 Agent 挂着模型配置引用，厂商/模型/端点由所引用的模型配置决定，本次修改不会生效——这些列会在下轮对话被实体值覆盖。请到「设置 → 模型」修改对应模型配置，或先解除引用改回手动模式后再修改".into(),
        ));
    }
    // 目标态主档：显式传了以传值为准（Some(None)=解除 → 无主档），否则沿用旧行
    let target_has_primary = match &input.model_profile_id {
        Some(opt) => opt.as_deref().is_some_and(|v| !v.trim().is_empty()),
        None => old_has_primary,
    };
    let chain_nonempty = matches!(
        &input.fallback_profile_ids,
        Some(Some(v)) if !v.is_empty()
    );
    validate_fallback_has_primary(target_has_primary, chain_nonempty)?;
    Ok(())
}

// ============================================================================
// 换厂商端点跟随（纯函数）
// ============================================================================

/// B：换厂商时未显式提供 base_url → 新厂商注册表默认地址（端点跟随厂商）。
/// 纯函数便于测试；custom/未知 provider 无默认（空串）→ None（保持不改）。
pub(crate) fn default_url_on_provider_switch(
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

/// base_url 双层 Option 解析（纯函数，便于测试回归）。
///
/// 显式 `Some(opt)` → 照设/照清（Some(None)=清空端点是合法显式意图）；
/// absent → 仅「换厂商且有注册表默认」时跟随（端点跟随厂商）；否则 `None`
/// = 保持不改。此前 absent 一律映射成 `Some(switch_url)`，未换厂商时变成
/// `Some(None)` = **静默清空端点**——任何不带 base_url 的 update（含提案卡）
/// 都会抹掉测试连接选定的端点；custom 换入无默认也被清空（与 B 修注释
/// 「保持不改」矛盾）。与 MockAgentCmd 的 if-let-Some 语义对齐。
pub(crate) fn resolve_base_url_arg<'a>(
    explicit: Option<Option<&'a str>>,
    switch_url: Option<&'a str>,
) -> Option<Option<&'a str>> {
    match explicit {
        Some(opt) => Some(opt),
        None => switch_url.map(Some),
    }
}
