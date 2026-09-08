//! 降级链（B2）：主模型档失败后按序换备用模型配置（ModelProfile）。
//! 分层纪律（S6 同款）：本文件只放**纯净件**——解析结果结构、resolver trait、
//! 链游标状态、纯函数、换档编排（[`try_switch_model`]）。生产 resolver
//! （Stronghold 取 key + create_provider 重建适配器）在 `commands::model_profile_cmd`
//! （与 `resolve_profile_credentials` 同居）；e2e 注入内存 MapResolver。循环链零
//! Tauri 依赖。
//!
//! 三拦截点（Quota 即时 / RateLimited·Network 退避耗尽）的接线在
//! [`super::retry_round`]——它拥有重试循环与错误分类现场。本文件的
//! [`FallbackPlan`] 持有链状态（cursor 单调前进——失败的档不回头，换档成功
//! 后下一轮再失败从新档位继续向下）。

use std::sync::Arc;

use crate::error::LlmErrorKind;
use crate::harness::event_log::{log_model_switch, EventCtx, ModelSwitchPayload};
use crate::infra::protocol::{ChatModelSwitchedPayload, LlmProvider};

use super::context::LoopContext;

/// resolver 解析出的完整档位：可直接换入 `LoopContext` 的五个运行时字段值
/// （provider 适配器恒重建——构造成本可忽略，免同/跨协议分叉）。
pub(crate) struct ResolvedModel {
    pub profile_id: String,
    /// 档位别名（用户起的名，如「智谱主力」）——换档事件/toast 展示用
    pub alias: String,
    pub provider: Arc<dyn LlmProvider>,
    pub api_key: String,
    pub model: String,
    /// 输出上限：`effective_output_cap`（agent.max_tokens 与模型策展表取 max）
    /// 在换档后按新 provider/model 重算——策展值随模型不同。
    pub max_tokens: i32,
}

/// 降级链档位解析器。
///
/// `resolve` 的 Err 语义 = **跳过该档**（profile 已删 / key 解析失败 / 厂商
/// 不识别），对齐视觉链逐候选语义——单档解析失败不终止整条链。
#[async_trait::async_trait]
pub(crate) trait FallbackResolver: Send + Sync {
    async fn resolve(
        &self,
        profile_id: &str,
        agent_max_tokens: i32,
        cache_prompt: bool,
    ) -> crate::error::AppResult<ResolvedModel>;
}

/// 一次回合的降级链状态（挂 `LoopContext`，循环内可变——cursor 前进/激活档位
/// 更新都发生在换档时）。
///
/// `chain` 空 = 无降级链（legacy 行为，一切换档尝试天然 no-op）。
pub(crate) struct FallbackPlan {
    /// 备用 profile id 有序链（JSON 数组列解析产物；去重/排除主档在写入侧保证）
    pub chain: Vec<String>,
    /// 下一个待尝试的链下标（单调前进：失败的档本回合不再回头）
    pub cursor: usize,
    /// 解析器（生产 ProfileFallbackResolver / e2e MapResolver；None = 无链）
    pub resolver: Option<Arc<dyn FallbackResolver>>,
    /// 当前激活档位对应的 profile id（主档引用时 = model_profile_id；legacy
    /// 手动主档 = None——健康记录 no-op）。换档成功后指向新档。
    pub active_profile_id: Option<String>,
    /// resolve 参数一：agent 行 `max_tokens` **原值**（非主档有效值——主档的
    /// 策展抬升随主模型，跨档累积会把旧档的策展值强加给新档）。换档后
    /// `effective_output_cap` 以它为新基准重算。
    pub agent_max_tokens: i32,
    /// resolve 参数二：与主档一致的 cache_prompt（传给 create_provider——
    /// 前缀缓存开关是 agent 级偏好，不随档位变）。
    pub cache_prompt: bool,
}

impl FallbackPlan {
    /// 无降级链（默认形态——不构造 resolver，一切换档尝试 no-op）。
    pub(crate) fn empty() -> Self {
        Self {
            chain: Vec::new(),
            cursor: 0,
            resolver: None,
            active_profile_id: None,
            agent_max_tokens: 0,
            cache_prompt: false,
        }
    }

    /// e2e/测试组装：显式链 + 注入 resolver + 主档 profile id + resolve 参数。
    pub(crate) fn new(
        chain: Vec<String>,
        resolver: Arc<dyn FallbackResolver>,
        active_profile_id: Option<String>,
        agent_max_tokens: i32,
        cache_prompt: bool,
    ) -> Self {
        Self {
            chain,
            cursor: 0,
            resolver: Some(resolver),
            active_profile_id,
            agent_max_tokens,
            cache_prompt,
        }
    }

    /// 还有可尝试的备用档吗（链未尽且 resolver 在位）。
    pub(crate) fn has_next(&self) -> bool {
        self.cursor < self.chain.len() && self.resolver.is_some()
    }
}

/// 解析 agent 行的 `fallback_profile_ids` 列（JSON 数组串，enabled_tools 先例）。
///
/// None / 空串 / 空数组 / 坏 JSON → 空链（坏 JSON 附 warn——数据损坏要留痕，
/// 行为上降级为无链，不阻断对话）。
pub(crate) fn parse_fallback_ids(raw: Option<&str>) -> Vec<String> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<String>>(s) {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(
                target: "ice_paw.fallback",
                "fallback_profile_ids 列损坏（按无降级链处理）: {e}"
            );
            Vec::new()
        }
    }
}

/// 单轮输出上限公式（从 session_runner 抽出共用）：agent.max_tokens 与模型
/// 策展表取 max（只抬不降），策展缺档兜底 16384。
///
/// 换档后模型变了，策展值可能不同——resolver 用它按新 (provider, model) 重算。
pub(crate) fn effective_output_cap(agent_max_tokens: i32, provider: &str, model: &str) -> i32 {
    agent_max_tokens.max(
        crate::harness::provider::default_max_output_tokens(provider, model).unwrap_or(16_384)
            as i32,
    )
}

/// 错误类别 → 换档触发原因 slug（None = 不换档）。
///
/// 决策 7 分类表：
/// - **quota**（余额/资源包，`InsufficientBalance` | `GlmResourcePack`）：确定性
///   失败，两处不可重试分支**即时**拦截——不退避白等；
/// - **rate_limited** / **network**：瞬时错误，先走完退避重试（可能自愈），
///   耗尽后才在 loop-top 拦截换档；
/// - 其余（Auth/Forbidden/ContextTooLong/Sensitive/Unknown）不换：鉴权/超长换
///   模型不解决（key 与 profile 绑定倒可能解决 Auth，但多数 401 是 key 本身
///   失效——宁停勿换，让用户看到真实错误）；Unknown 可能是 prompt 层问题，
///   换档无效且会掩盖诊断信息。
pub(crate) fn fallback_trigger(kind: LlmErrorKind) -> Option<&'static str> {
    match kind {
        LlmErrorKind::InsufficientBalance | LlmErrorKind::GlmResourcePack => Some("quota"),
        LlmErrorKind::RateLimited => Some("rate_limited"),
        LlmErrorKind::Network => Some("network"),
        LlmErrorKind::Sensitive
        | LlmErrorKind::Auth
        | LlmErrorKind::Forbidden
        | LlmErrorKind::ContextTooLong
        | LlmErrorKind::Unknown => None,
    }
}

/// 换档编排（三拦截点共用）：健康记录失败档 → 按链逐档 resolve（Err 跳档）→
/// 成功即 swap `LoopContext` 五字段 + 落 `model_switch` 事件 + emit
/// `chat:model-switched`。
///
/// 返回 false = 无链/链尽/全部跳档（调用方走原终态路径：emit_round_error 或
/// RetryExhausted）。空链（legacy）在第一步 `has_next` 即返回，行为零变化。
pub(crate) async fn try_switch_model(
    ctx: &mut LoopContext,
    current_asst_msg_id: &str,
    trigger: &str,
    error_text: &str,
    switch_attempt: u32,
) -> bool {
    // 健康记录（决策 6，先于 has_next——无链的引用 agent 主档失败也要留痕）：
    // legacy 手动主档 active=None → no-op；warn-only 旁路绝不影响换档本身。
    crate::harness::profile_health::record_error(
        Some(&ctx.pool),
        ctx.fallback.active_profile_id.as_deref(),
        error_text,
    )
    .await;

    let Some(resolver) = ctx.fallback.resolver.clone() else {
        return false;
    };
    while ctx.fallback.has_next() {
        let pid = ctx.fallback.chain[ctx.fallback.cursor].clone();
        match resolver
            .resolve(
                &pid,
                ctx.fallback.agent_max_tokens,
                ctx.fallback.cache_prompt,
            )
            .await
        {
            // 解析失败 = 跳过该档（profile 已删 / key 解析失败 / 厂商不识别），
            // 对齐视觉链逐候选语义——单档解析失败不终止整条链
            Err(e) => {
                tracing::warn!(target: "ice_paw.fallback",
                    conv = %ctx.conv_id,
                    profile = %pid,
                    "降级链跳档（解析失败）: {e}");
                ctx.fallback.cursor += 1;
            }
            Ok(resolved) => {
                // from 快照先于 swap（换出档位信息在覆盖后即丢）
                let from_profile_id = ctx.fallback.active_profile_id.clone();
                let from_model = ctx.model.clone().unwrap_or_default();
                let to_alias = resolved.alias.clone();
                let to_model = resolved.model.clone();
                let to_profile_id = resolved.profile_id.clone();

                // swap 五字段（决策 8）：asst_model 跟随新档位 model——会话级
                // model_override 是针对原模型的意图，跨档失效；预算 cap 不重置
                //（用户侧开销连续计量）。
                ctx.provider = resolved.provider;
                ctx.api_key = resolved.api_key;
                ctx.model = Some(resolved.model);
                ctx.asst_model = Some(to_model.clone());
                ctx.max_tokens = resolved.max_tokens;
                ctx.fallback.active_profile_id = Some(to_profile_id.clone());
                ctx.fallback.cursor += 1;

                tracing::info!(target: "ice_paw.fallback",
                    conv = %ctx.conv_id,
                    attempt = switch_attempt,
                    reason = trigger,
                    "降级换档成功 → {to_profile_id}（{to_model}）");
                let ev = EventCtx::new(&ctx.conv_id, &ctx.user_msg_id, &ctx.agent_id);
                log_model_switch(
                    &ctx.pool,
                    &ev,
                    &ModelSwitchPayload {
                        v: 1,
                        from_profile_id,
                        from_model: from_model.clone(),
                        to_profile_id,
                        to_alias: to_alias.clone(),
                        to_model: to_model.clone(),
                        reason: trigger.to_string(),
                        attempt: switch_attempt,
                        error: Some(crate::infra::strings::truncate_to_byte_boundary(
                            error_text, 200, Some("…"),
                        )),
                    },
                )
                .await;
                super::emitter::emit_ser(
                    ctx.emitter.as_ref(),
                    "chat:model-switched",
                    &ChatModelSwitchedPayload {
                        conversation_id: ctx.conv_id.clone(),
                        message_id: current_asst_msg_id.to_string(),
                        from_model,
                        to_alias,
                        to_model,
                        reason: trigger.to_string(),
                    },
                );
                return true;
            }
        }
    }
    tracing::warn!(target: "ice_paw.fallback",
        conv = %ctx.conv_id,
        reason = trigger,
        "降级链已尽，走原终态路径");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fallback_ids_handles_all_column_forms() {
        // None / 空串 / 空白 → 空链
        assert!(parse_fallback_ids(None).is_empty());
        assert!(parse_fallback_ids(Some("")).is_empty());
        assert!(parse_fallback_ids(Some("  ")).is_empty());
        // 合法 JSON 数组
        assert_eq!(
            parse_fallback_ids(Some(r#"["mp-1","mp-2"]"#)),
            vec!["mp-1".to_string(), "mp-2".to_string()]
        );
        // 空数组 = 显式清空 → 空链
        assert!(parse_fallback_ids(Some("[]")).is_empty());
        // 坏 JSON → 降级空链（warn 留痕）
        assert!(parse_fallback_ids(Some("not-json")).is_empty());
    }

    #[test]
    fn empty_plan_is_inert() {
        let plan = FallbackPlan::empty();
        assert!(!plan.has_next());
        assert!(plan.chain.is_empty());
        assert!(plan.resolver.is_none());
    }

    #[test]
    fn has_next_tracks_cursor_and_resolver() {
        struct NoopResolver;
        #[async_trait::async_trait]
        impl FallbackResolver for NoopResolver {
            async fn resolve(
                &self,
                _profile_id: &str,
                _agent_max_tokens: i32,
                _cache_prompt: bool,
            ) -> crate::error::AppResult<ResolvedModel> {
                unreachable!("本用例不调用 resolve")
            }
        }
        let mut plan = FallbackPlan::new(
            vec!["mp-1".into(), "mp-2".into()],
            Arc::new(NoopResolver),
            Some("mp-main".into()),
            1024,
            false,
        );
        assert!(plan.has_next());
        plan.cursor = 2; // 链尽
        assert!(!plan.has_next());
    }

    /// 决策 7 分类全表：九 kind 逐一断言（含不换档五档——漏档不会有编译错，
    /// 表驱动防「加枚举变体忘查此表」）。
    #[test]
    fn fallback_trigger_covers_all_nine_kinds() {
        use crate::error::LlmErrorKind as K;
        // quota 族（确定性失败，两处不可重试分支即时拦截）
        assert_eq!(fallback_trigger(K::InsufficientBalance), Some("quota"));
        assert_eq!(fallback_trigger(K::GlmResourcePack), Some("quota"));
        // 瞬时族（退避耗尽后 loop-top 拦截）
        assert_eq!(fallback_trigger(K::RateLimited), Some("rate_limited"));
        assert_eq!(fallback_trigger(K::Network), Some("network"));
        // 不换档族：鉴权/权限/超长/审核是配置或输入问题，Unknown 可能是 prompt 层
        assert_eq!(fallback_trigger(K::Auth), None);
        assert_eq!(fallback_trigger(K::Forbidden), None);
        assert_eq!(fallback_trigger(K::ContextTooLong), None);
        assert_eq!(fallback_trigger(K::Sensitive), None);
        assert_eq!(fallback_trigger(K::Unknown), None);
    }

    #[test]
    fn output_cap_takes_max_of_agent_and_curated() {
        // agent 值更高（策展缺档兜底 16384）→ 保持 agent 值（只抬不降）
        assert_eq!(effective_output_cap(32_000, "totally-unknown", "m"), 32_000);
        // 策展值更高 → 抬到策展（断言只抬不降的语义，不锁具体策展值——表会演进）
        assert!(effective_output_cap(1_000, "glm", "glm-5.3") > 1_000);
        assert_eq!(effective_output_cap(1_000, "totally-unknown", "m"), 16_384);
    }
}
