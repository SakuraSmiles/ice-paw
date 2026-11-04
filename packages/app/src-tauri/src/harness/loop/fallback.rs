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
    /// 下一个待尝试的链下标（单调前进：失败的档本回合不再回头——**回探例外**：
    /// 主档冷却期满后经 [`ProbeState`] 重试，见模块尾「回探」节）
    pub cursor: usize,
    /// 解析器（生产 ProfileFallbackResolver / e2e MapResolver；None = 无链）
    pub resolver: Option<Arc<dyn FallbackResolver>>,
    /// 当前激活档位对应的 profile id（主档引用时 = model_profile_id；legacy
    /// 手动主档 = None——健康记录 no-op）。换档成功后指向新档。
    pub active_profile_id: Option<String>,
    /// 回合起跑时的主档引用（回探目标；legacy 手动主档 = None → 回探结构性
    /// 不可能，行为与旧版逐字节一致）。
    pub initial_profile_id: Option<String>,
    /// 回探状态（首次换档离开主档时创建；None = 从未换档 / legacy / 回探成功后清除）
    pub probe: Option<ProbeState>,
    /// resolve 参数一：agent 行 `max_tokens` **原值**（非主档有效值——主档的
    /// 策展抬升随主模型，跨档累积会把旧档的策展值强加给新档）。换档后
    /// `effective_output_cap` 以它为新基准重算。
    pub agent_max_tokens: i32,
    /// resolve 参数二：与主档一致的 cache_prompt（传给 create_provider——
    /// 前缀缓存开关是 agent 级偏好，不随档位变）。
    pub cache_prompt: bool,
    /// 回探冷却覆盖（**e2e 专用**——生产 None 走原因驱动默认表；压缩冷却让
    /// 「换档 → 下一轮回探」全链路在测试里秒级可驱动）。
    pub probe_cooldown_override: Option<std::time::Duration>,
}

/// 主档回探状态（换档后不回头原则的**瞬态例外**，2026-11-04）。
///
/// 生产实案：GLM Coding 端点在套餐配额/并发压力下返回 429+1113（分类 quota →
/// 零退避即时换档），而委派子任务 = 一个数十轮的长回合——一次瞬态 1113 把整个
/// 下午的全部工作量静默转移到降档模型（DeepSeek 余额烧穿）。quota/rate_limited/
/// network 三类触发都可能自愈，按冷却窗口回探主档：成功即切回（事件 reason=
/// `recovered`），失败还原到换出档并按 ×2 退避拉长冷却；不可恢复类（Auth 等）
/// 本回合不再回探。
pub(crate) struct ProbeState {
    /// 下次回探时刻（None = 本回合不再回探：不可恢复类失败，或主档解析失败循环）
    pub next_at: Option<tokio::time::Instant>,
    /// 当前回探冷却间隔（失败 ×2 增长到 cap）
    pub backoff: std::time::Duration,
    /// 冷却上限（按首次换档触发原因定档）
    pub cap: std::time::Duration,
    /// 回探目标：主档的完整档位（resolve 产物——切回时直接换入五字段）
    pub primary: Option<ResolvedModel>,
    /// 回退目标：换出档的完整快照（回探失败时直接还原，不靠 cursor 重走链——
    /// 链走法可能落进更深的死档，直接还原保证回到已知可用档）
    pub last_good: Option<ResolvedModel>,
    /// 还原后的 cursor 恢复位（换出档在链中下标 +1）
    pub restore_cursor: usize,
    /// 本轮正在回探主档（stream_with_retry 失败路径的分流依据）
    pub live: bool,
}

/// 按首次换档触发原因定回探冷却初值：quota（含 GLM 1113——套餐窗口/并发类
/// 多为分钟级自愈）短冷却快回；rate_limited（如 DeepSeek 1308 五小时窗）长冷却。
fn initial_probe_backoff(trigger: &str) -> std::time::Duration {
    match trigger {
        "quota" => std::time::Duration::from_secs(60),
        "rate_limited" => std::time::Duration::from_secs(600),
        _ => std::time::Duration::from_secs(300),
    }
}

/// 冷却上限（同上按原因定档）。
fn probe_backoff_cap(trigger: &str) -> std::time::Duration {
    match trigger {
        "quota" => std::time::Duration::from_secs(900),
        "rate_limited" => std::time::Duration::from_secs(3600),
        _ => std::time::Duration::from_secs(1800),
    }
}

/// 回探失败后的冷却推进（纯函数）：可恢复类 ×2 增长到 cap；不可恢复类返回
/// None（本回合不再回探）。
fn probe_cooldown_after_failure(
    backoff: std::time::Duration,
    cap: std::time::Duration,
    kind: crate::error::LlmErrorKind,
) -> Option<std::time::Duration> {
    // fallback_trigger 覆盖的触发面 = 可能自愈的错误族；其余（Auth/Forbidden/
    // ContextTooLong/Sensitive/Unknown）换档也解决不了，主档同理不再试（None）
    fallback_trigger(kind).map(|_| backoff.saturating_mul(2).min(cap))
}

impl FallbackPlan {
    /// 无降级链（默认形态——不构造 resolver，一切换档尝试 no-op）。
    pub(crate) fn empty() -> Self {
        Self {
            chain: Vec::new(),
            cursor: 0,
            resolver: None,
            active_profile_id: None,
            initial_profile_id: None,
            probe: None,
            agent_max_tokens: 0,
            cache_prompt: false,
            probe_cooldown_override: None,
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
            active_profile_id: active_profile_id.clone(),
            // 回探目标 = 回合起跑档（首次换档前它就是主档）
            initial_profile_id: active_profile_id,
            probe: None,
            agent_max_tokens,
            cache_prompt,
            probe_cooldown_override: None,
        }
    }

    /// e2e 专用：压缩回探冷却（生产勿用——默认表按触发原因驱动；仅测试构建可见）。
    #[cfg(test)]
    pub(crate) fn with_probe_cooldown(mut self, d: std::time::Duration) -> Self {
        self.probe_cooldown_override = Some(d);
        self
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
                // 换出档完整快照（回探回退目标）：五字段此刻还是换出档的值
                let from_tier = ResolvedModel {
                    profile_id: from_profile_id.clone().unwrap_or_default(),
                    alias: String::new(),
                    provider: ctx.provider.clone(),
                    api_key: ctx.api_key.clone(),
                    model: from_model.clone(),
                    max_tokens: ctx.max_tokens,
                };

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

                // 回探登记：离开主档后按触发原因起冷却窗口（瞬态失败——尤其
                // GLM 1113 的套餐窗口/并发语义——不再把整个长回合钉死在降档）
                register_probe(ctx, trigger, from_tier);
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
                            error_text,
                            200,
                            Some("…"),
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

// ============================================================================
// 主档回探（2026-11-04）：换档「不回头」原则的瞬态例外
// ============================================================================

/// 换档成功后登记/更新回探状态（[`try_switch_model`] 成功臂调用）。
///
/// - 首次离开主档：按触发原因建 `ProbeState`（冷却初值 + 上限 + 回退目标）；
/// - 已在回探循环（回探失败后经链换向更深档）：只更新回退目标与 cursor 恢复位；
/// - legacy 主档（`initial_profile_id` None）：结构性不可能回探，零登记。
fn register_probe(ctx: &mut LoopContext, trigger: &str, from_tier: ResolvedModel) {
    let Some(initial) = ctx.fallback.initial_profile_id.clone() else {
        return;
    };
    let to_profile_id = ctx.fallback.active_profile_id.clone().unwrap_or_default();
    if initial == to_profile_id {
        // 换向主档本身：仅 probe 专用路径之外不该发生；防御性清除回探状态
        ctx.fallback.probe = None;
        return;
    }
    let restore_cursor = ctx.fallback.cursor;
    match ctx.fallback.probe.as_mut() {
        None => {
            let backoff = ctx
                .fallback
                .probe_cooldown_override
                .unwrap_or_else(|| initial_probe_backoff(trigger));
            ctx.fallback.probe = Some(ProbeState {
                next_at: Some(tokio::time::Instant::now() + backoff),
                backoff,
                cap: probe_backoff_cap(trigger).max(backoff),
                primary: None,
                last_good: Some(from_tier),
                restore_cursor,
                live: false,
            });
        }
        Some(p) => {
            // 回探失败后走链换向更深档：回退目标跟到新档（回探失败时还原到
            // 「最近已知可用档」，不是当初第一次换出的那个）
            p.last_good = Some(from_tier);
            p.restore_cursor = restore_cursor;
        }
    }
}

/// 本轮是否正在回探主档（retry_round 失败路径分流依据）。
pub(crate) fn probe_live(ctx: &LoopContext) -> bool {
    ctx.fallback
        .probe
        .as_ref()
        .is_some_and(|p| p.live && p.primary.is_some())
}

/// 冷却期满则起一次回探：resolve 主档 → 快照当前档为回退目标 → 换入主档。
/// resolve 失败 = 顺延冷却（主档配置坏了不该每轮都去撞）。
/// [`stream_with_retry`] 每轮顶部调用。
pub(crate) async fn maybe_start_probe(ctx: &mut LoopContext) {
    // 门槛短路（序：无状态 → 在途 → 已熄灭 → 未到期）
    {
        let Some(p) = ctx.fallback.probe.as_ref() else {
            return;
        };
        if p.live {
            return;
        }
        let Some(next_at) = p.next_at else {
            return; // 本回合不再回探（不可恢复类）
        };
        if next_at > tokio::time::Instant::now() {
            return;
        }
    }
    let Some(primary_id) = ctx.fallback.initial_profile_id.clone() else {
        return;
    };
    // 主档已是激活档（回探成功的后续轮次）：清除回探状态（防御路径）
    if ctx.fallback.active_profile_id.as_deref() == Some(primary_id.as_str()) {
        ctx.fallback.probe = None;
        return;
    }
    let Some(resolver) = ctx.fallback.resolver.clone() else {
        return;
    };
    match resolver
        .resolve(
            &primary_id,
            ctx.fallback.agent_max_tokens,
            ctx.fallback.cache_prompt,
        )
        .await
    {
        Ok(primary) => {
            // 当前档完整快照 = 回退目标（五字段此刻是换出档的值）
            let last_good = ResolvedModel {
                profile_id: ctx.fallback.active_profile_id.clone().unwrap_or_default(),
                alias: String::new(),
                provider: ctx.provider.clone(),
                api_key: ctx.api_key.clone(),
                model: ctx.model.clone().unwrap_or_default(),
                max_tokens: ctx.max_tokens,
            };
            let restore_cursor = ctx.fallback.cursor;
            let to_model = primary.model.clone();
            let to_profile_id = primary.profile_id.clone();

            ctx.provider = primary.provider.clone();
            ctx.api_key = primary.api_key.clone();
            ctx.model = Some(primary.model.clone());
            ctx.asst_model = Some(primary.model.clone());
            ctx.max_tokens = primary.max_tokens;
            ctx.fallback.active_profile_id = Some(primary.profile_id.clone());
            // cursor 归零：回探若失败走链重换，从链头重新起跑（对齐回合起跑语义）
            ctx.fallback.cursor = 0;

            if let Some(p) = ctx.fallback.probe.as_mut() {
                p.primary = Some(primary);
                p.last_good = Some(last_good);
                p.restore_cursor = restore_cursor;
                p.live = true;
            }
            // 回探也是缓存命名空间切换（usage 归因的 model_switched 位）
            ctx.turn_cost.mark_model_switched();
            tracing::info!(target: "ice_paw.fallback",
                conv = %ctx.conv_id,
                "冷却期满，回探主档 {to_profile_id}（{to_model}）——成功即切回，失败还原");
        }
        Err(e) => {
            if let Some(p) = ctx.fallback.probe.as_mut() {
                if let Some(next_at) = p.next_at {
                    p.next_at = Some(next_at + p.backoff);
                }
            }
            tracing::warn!(target: "ice_paw.fallback",
                conv = %ctx.conv_id,
                "回探主档解析失败（顺延冷却）: {e}");
        }
    }
}

/// 回探失败：还原到回退目标（最近已知可用档）+ 冷却推进。
/// 可恢复类 ×2 增长到 cap；不可恢复类 `next_at = None`（本回合不再回探）。
/// 回退目标缺失（防御）= 仅结束回探态，保持当前档（走原终态语义）。
/// 返回 true = 已还原可继续本轮（调用方 continue）；false = 无法还原。
pub(crate) fn abort_probe(
    ctx: &mut LoopContext,
    retry_state: &mut crate::harness::retry::RetryState,
    error_text: &str,
) -> bool {
    let Some(mut probe) = ctx.fallback.probe.take() else {
        return false;
    };
    if !probe.live {
        ctx.fallback.probe = Some(probe);
        return false;
    }
    let kind = crate::error::classify_llm_error(error_text);
    let restored_model = probe
        .last_good
        .as_ref()
        .map(|lg| lg.model.clone())
        .unwrap_or_default();
    // 还原五字段 + cursor
    if let Some(lg) = probe.last_good.take() {
        ctx.provider = lg.provider;
        ctx.api_key = lg.api_key;
        ctx.model = Some(lg.model.clone());
        ctx.asst_model = Some(lg.model);
        ctx.max_tokens = lg.max_tokens;
        ctx.fallback.active_profile_id = Some(lg.profile_id);
        ctx.fallback.cursor = probe.restore_cursor;
    }
    // 冷却推进（不可恢复类熄灭回探）
    match probe_cooldown_after_failure(probe.backoff, probe.cap, kind) {
        Some(grown) => {
            probe.backoff = grown;
            probe.next_at = Some(tokio::time::Instant::now() + grown);
        }
        None => probe.next_at = None,
    }
    probe.live = false;
    probe.primary = None;
    ctx.fallback.probe = Some(probe);
    // 回探尝试不计入退避预算（还原后等效全新一轮）
    *retry_state = crate::harness::retry::RetryState::new();
    // 还原也是缓存命名空间切换
    ctx.turn_cost.mark_model_switched();
    tracing::info!(target: "ice_paw.fallback",
        conv = %ctx.conv_id,
        kind = ?kind,
        "回探主档失败，还原 {restored_model}；下次回探冷却已推进");
    true
}

/// 回探成功（本轮在主档上产出内容）：清除回探状态 + 落 `recovered` 换档事件
/// + emit toast（用户可见「已切回主模型」）。[`stream_with_retry`] Ok 臂调用。
pub(crate) async fn probe_succeeded(ctx: &mut LoopContext, current_asst_msg_id: &str) {
    let Some(probe) = ctx.fallback.probe.take() else {
        return;
    };
    if !probe.live {
        ctx.fallback.probe = Some(probe);
        return;
    }
    let Some(primary) = probe.primary else {
        return;
    };
    let Some(last_good) = probe.last_good else {
        return;
    };
    let from_model = last_good.model.clone();
    let from_profile_id = last_good.profile_id.clone();
    let to_alias = primary.alias.clone();
    let to_model = primary.model.clone();
    let to_profile_id = primary.profile_id.clone();
    tracing::info!(target: "ice_paw.fallback",
        conv = %ctx.conv_id,
        "回探主档成功 → {to_profile_id}（{to_model}），已切回");
    let ev = EventCtx::new(&ctx.conv_id, &ctx.user_msg_id, &ctx.agent_id);
    log_model_switch(
        &ctx.pool,
        &ev,
        &ModelSwitchPayload {
            v: 1,
            from_profile_id: Some(from_profile_id),
            from_model: from_model.clone(),
            to_profile_id,
            to_alias: to_alias.clone(),
            to_model: to_model.clone(),
            reason: "recovered".to_string(),
            attempt: 0,
            error: None,
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
            reason: "recovered".to_string(),
        },
    );
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

    /// 回探冷却表：quota 短冷却快回（GLM 1113 的套餐窗口/并发多为分钟级自愈）；
    /// rate_limited 长冷却（DeepSeek 1308 类五小时窗）；cap 同序。
    #[test]
    fn probe_backoff_table_by_trigger() {
        assert_eq!(initial_probe_backoff("quota"), std::time::Duration::from_secs(60));
        assert_eq!(initial_probe_backoff("rate_limited"), std::time::Duration::from_secs(600));
        assert_eq!(initial_probe_backoff("network"), std::time::Duration::from_secs(300));
        assert_eq!(probe_backoff_cap("quota"), std::time::Duration::from_secs(900));
        assert_eq!(probe_backoff_cap("rate_limited"), std::time::Duration::from_secs(3600));
    }

    /// 回探失败冷却推进：可恢复类 ×2 封顶；不可恢复类（Auth 等）熄灭回探。
    #[test]
    fn probe_cooldown_grows_and_extinguishes() {
        use crate::error::LlmErrorKind as K;
        use std::time::Duration;
        // quota 60s → 120s → 240s → … cap 900s
        let cap = probe_backoff_cap("quota");
        let mut b = initial_probe_backoff("quota");
        b = probe_cooldown_after_failure(b, cap, K::RateLimited).unwrap();
        assert_eq!(b, Duration::from_secs(120));
        b = probe_cooldown_after_failure(b, cap, K::GlmResourcePack).unwrap();
        assert_eq!(b, Duration::from_secs(240));
        b = probe_cooldown_after_failure(Duration::from_secs(600), cap, K::Network).unwrap();
        assert_eq!(b, Duration::from_secs(900), "封顶不越界");
        // 不可恢复类 → None（本回合不再回探）
        assert!(probe_cooldown_after_failure(b, cap, K::Auth).is_none());
        assert!(probe_cooldown_after_failure(b, cap, K::ContextTooLong).is_none());
    }
}
