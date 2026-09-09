//! 回合级上下文开销记录 + 缓存 miss 归因（③ 上下文开销可观测化）。
//!
//! 两个产物：
//! - `chat:budget` 瞬态 `miss_hint`（本轮全 miss 的归因 slug 数组 → BudgetPill chip）；
//! - `context_breakdown` 落库事件（段级组成 + 指纹 + 逐轮序列 → 轨迹页体检区）。
//!
//! 挂 [`LoopContext::turn_cost`]，由 session_runner 从 `ContextAnatomyInput`
//!（Pipeline 后 `build_anatomy` + 跨回合基线查询）构造；**emit 单点在
//! `stream_loop` wrapper**（inner 返回后）——19 个 finalize 调用点的公共唯一
//! 下游，零签名扰动；turn_ended 已在 inner finalize 内先落库，硬规则不破。
//!
//! ⚠️ 措辞诚实（硬约束）：归因是**本地推断**非 provider 报告——前端展示必须
//! 带披露（missHint.ts）。已知机理说明（TTL 过期 / 前缀块对齐）只住前端文案，
//! 后端只判「输入侧谁变了」这个可观测事实。

use crate::harness::event_log::{
    ContextBreakdownPayload, ContextBreakdownFingerprint, ContextBreakdownRound,
    ContextBreakdownSegment,
};
use crate::infra::protocol::ToolDef;

/// 归因门槛：prompt ≥ 此值且 `cached == 0` 才出 hint。
///
/// 部分未命中是常态（每轮追加后缀天然 miss 尾块），逐轮提示是噪声；OpenAI 系
/// 自动缓存按 1024 token 块对齐，低于此值几乎无缓存可言。
pub(crate) const MISS_HINT_MIN_PROMPT: u64 = 1024;

/// miss 归因 slug 词表（前端 `utils/missHint.ts` 镜像；改动两边同步）。
pub(crate) mod miss_slug {
    /// 无任何基线（会话首个请求）——与其他 slug 互斥
    pub const FIRST_REQUEST: &str = "first_request";
    /// 工具列表指纹变化（增删/描述改/相关性子集逐轮抖动）
    pub const TOOLS_CHANGED: &str = "tools_changed";
    /// system 稳定段变化（人设/工具提示/委派清单/样式档案）
    pub const SYSTEM_STABLE_CHANGED: &str = "system_stable_changed";
    /// 运行环境段变化（工作目录/时区/project.md/conventions.md）
    pub const OS_ENV_CHANGED: &str = "os_env_changed";
    /// 注入状态翻转（BeforeLlm 钩子 / 预算提醒）
    pub const INJECTION_CHANGED: &str = "injection_changed";
    /// 降级链换档（缓存命名空间切换）
    pub const MODEL_SWITCHED: &str = "model_switched";
    /// 以上均无差异（前端文案「疑缓存过期」+ 推断披露）
    pub const NO_DETECTABLE_CHANGE: &str = "no_detectable_change";
}

/// 跨回合比对基线：上一回合 `context_breakdown` 事件的指纹终态。
#[derive(Debug, Clone)]
pub(crate) struct PrevBaseline {
    /// 上回合轮 0 工具列表指纹（`fingerprint.tools`）
    pub tools: String,
    pub system_stable: String,
    pub os_stable: String,
    /// 上回合最后一轮是否注入（轮 0 与上回合末轮比对）
    pub last_injected: bool,
    /// 上回合任一轮是否换档（换档后缓存命名空间已切换）
    pub any_switched: bool,
}

impl PrevBaseline {
    /// 从上一条 breakdown payload JSON 解析基线。
    ///
    /// 坏 JSON / 空轮次返回 None——调用方 warn-only 消费：降级按无基线
    /// （归因 first_request），诚实降级不污染。
    pub(crate) fn from_payload(json: &str) -> Option<Self> {
        let p: ContextBreakdownPayload = serde_json::from_str(json).ok()?;
        if p.rounds.is_empty() {
            return None;
        }
        Some(Self {
            tools: p.fingerprint.tools,
            system_stable: p.fingerprint.system_stable,
            os_stable: p.fingerprint.os_stable,
            last_injected: p.rounds.last().map(|r| r.injected).unwrap_or(false),
            any_switched: p.rounds.iter().any(|r| r.model_switched),
        })
    }
}

/// session_runner → LoopConfig 的注入件：anatomy 段 + 稳定指纹 + 跨回合基线。
///
/// `Default` 全空 = 散落构造（测试）——recorder 照常工作，只是 system 段
/// 缺失、跨回合归因恒 first_request（诚实降级）。
#[derive(Debug, Clone, Default)]
pub(crate) struct ContextAnatomyInput {
    pub segments: Vec<ContextBreakdownSegment>,
    pub system_stable: String,
    pub os_stable: String,
    pub prev: Option<PrevBaseline>,
}

impl ContextAnatomyInput {
    /// 从 Pipeline 终态聚合产物转换（段 label String 化进 payload 形态）。
    pub(crate) fn from_anatomy(
        a: crate::context::anatomy::ContextAnatomy,
        prev: Option<PrevBaseline>,
    ) -> Self {
        Self {
            segments: a
                .segments
                .into_iter()
                .map(|s| ContextBreakdownSegment {
                    label: s.label.to_string(),
                    est: s.est,
                    count: s.count,
                })
                .collect(),
            system_stable: a.system_stable_hash,
            os_stable: a.os_stable_hash,
            prev,
        }
    }
}

/// 判定表输入（当前请求侧）。
pub(crate) struct MissInput<'a> {
    pub tools: &'a str,
    pub system_stable: &'a str,
    pub os_stable: &'a str,
    pub injected: bool,
    pub model_switched: bool,
    pub is_round_zero: bool,
}

/// miss 归因判定表（纯函数；后端判事实、前端讲机理）。
///
/// 多 slug 并存（顺序 = 词表序，展示稳定）；`first_request` 与其他互斥。
/// system / os 稳定段只在轮 0 与上回合比——Pipeline 每回合只跑一次，回合内
/// 逐轮 system 恒同，轮 ≥1 比对是恒真噪声。
pub(crate) fn attribute_miss(cur: &MissInput<'_>, prev: Option<&PrevBaseline>) -> Vec<&'static str> {
    let Some(prev) = prev else {
        return vec![miss_slug::FIRST_REQUEST];
    };
    let mut out = Vec::new();
    // 工具列表变化（含 Some↔None：无工具的指纹是空串，与非空自然不等）
    if cur.tools != prev.tools {
        out.push(miss_slug::TOOLS_CHANGED);
    }
    if cur.is_round_zero && cur.system_stable != prev.system_stable {
        out.push(miss_slug::SYSTEM_STABLE_CHANGED);
    }
    if cur.is_round_zero && cur.os_stable != prev.os_stable {
        out.push(miss_slug::OS_ENV_CHANGED);
    }
    // 注入状态翻转（注入内容本身变化也算——钩子注入必经 system 前缀外追加）
    if cur.injected != prev.last_injected {
        out.push(miss_slug::INJECTION_CHANGED);
    }
    // 本轮或基线侧换档（缓存命名空间切换）
    if cur.model_switched || prev.any_switched {
        out.push(miss_slug::MODEL_SWITCHED);
    }
    if out.is_empty() {
        out.push(miss_slug::NO_DETECTABLE_CHANGE);
    }
    out
}

/// 回合级记录器。
///
/// 生命周期 = 一个对话回合（LoopContext 构造 → wrapper emit 后丢弃）。
/// - `begin_round`：每轮工具组装后（loop_engine）记工具指纹；首次调用兼记
///   `tool_defs` 段估算（工具定义 token 是历史估算盲区，token.rs 单点补上）。
/// - `record_usage`：provider 回传 usage 时记逐轮序列，全 miss 时跑判定表，
///   返回 slug 数组给调用方随 `chat:budget` 推前端。
/// - `mark_model_switched`：换档挂起标记（switch_and_reset 成功换档后），
///   落到下一个 usage 记录的本轮 `model_switched` 位。
pub(crate) struct TurnCostRecorder {
    anatomy: ContextAnatomyInput,
    /// 轮 0 工具定义估算（est, count）——`tool_defs` 段
    tool_defs: Option<(u64, u32)>,
    rounds: Vec<ContextBreakdownRound>,
    /// 当前轮工具指纹（begin_round 写，record_usage 消费）
    cur_tools_hash: String,
    /// 换档挂起（mark_model_switched 置位，record_usage 清）
    pending_switch: bool,
    /// 上一请求快照（同回合上一轮基线；轮 0 前为 None → 跨回合 anatomy.prev）
    prev_round: Option<PrevBaseline>,
}

impl TurnCostRecorder {
    pub(crate) fn new(anatomy: ContextAnatomyInput) -> Self {
        Self {
            anatomy,
            tool_defs: None,
            rounds: Vec::new(),
            cur_tools_hash: String::new(),
            pending_switch: false,
            prev_round: None,
        }
    }

    /// 每轮工具组装后调用（`tools=None` = 本轮禁用工具，指纹空串）。
    pub(crate) fn begin_round(&mut self, tools: Option<&[ToolDef]>) {
        self.cur_tools_hash = tools
            .map(crate::context::anatomy::hash_tool_defs)
            .unwrap_or_default();
        // 首轮兼记 tool_defs 段（出口恒按 name 排序、全量发送；后续轮相关性
        // 子集若抖动，指纹比对自会归因 tools_changed）
        if self.rounds.is_empty() {
            self.tool_defs = tools.map(|defs| {
                let est: u64 = defs
                    .iter()
                    .map(crate::context::token::estimate_tool_def_tokens)
                    .sum::<usize>() as u64;
                (est, defs.len() as u32)
            });
        }
    }

    /// provider 回传 usage 后调用。返回 miss 归因 slug 数组（非全 miss 返回 None）。
    pub(crate) fn record_usage(&mut self, prompt: u64, cached: u64, injected: bool) -> Option<Vec<String>> {
        let switched = self.pending_switch;
        self.pending_switch = false;
        let round_zero = self.rounds.is_empty();
        let prev: Option<&PrevBaseline> = self.prev_round.as_ref().or(self.anatomy.prev.as_ref());

        let miss = prompt >= MISS_HINT_MIN_PROMPT && cached == 0;
        let slugs: Option<Vec<String>> = miss.then(|| {
            let input = MissInput {
                tools: &self.cur_tools_hash,
                system_stable: &self.anatomy.system_stable,
                os_stable: &self.anatomy.os_stable,
                injected,
                model_switched: switched,
                is_round_zero: round_zero,
            };
            attribute_miss(&input, prev)
                .into_iter()
                .map(str::to_string)
                .collect()
        });

        self.rounds.push(ContextBreakdownRound {
            prompt,
            cached,
            tools_hash: self.cur_tools_hash.clone(),
            injected,
            model_switched: switched,
        });
        // 基线推进为「本请求快照」：下一轮（或下回合轮 0 若这是末轮）与之比对
        self.prev_round = Some(PrevBaseline {
            tools: self.cur_tools_hash.clone(),
            system_stable: self.anatomy.system_stable.clone(),
            os_stable: self.anatomy.os_stable.clone(),
            last_injected: injected,
            any_switched: switched,
        });
        slugs
    }

    /// 降级链换档标记（`switch_and_reset` 成功换档后调用）。
    pub(crate) fn mark_model_switched(&mut self) {
        self.pending_switch = true;
    }

    /// wrapper 单点取件：取出 payload 并复位记录器（零请求返回 None）。
    /// `stream_loop` 在 inner 返回后调用——19 个 finalize 调用点的公共唯一下游。
    pub(crate) fn take_payload(&mut self) -> Option<ContextBreakdownPayload> {
        std::mem::replace(self, TurnCostRecorder::new(ContextAnatomyInput::default()))
            .into_payload()
    }

    /// 组装 `context_breakdown` payload。零请求（无任何 usage 记录）返回 None
    /// ——不产事件（provider 间歇不回 usage 的回合诚实缺席，Inspector 对缺失不渲染）。
    pub(crate) fn into_payload(mut self) -> Option<ContextBreakdownPayload> {
        if self.rounds.is_empty() {
            return None;
        }
        if let Some((est, count)) = self.tool_defs {
            self.anatomy.segments.push(ContextBreakdownSegment {
                label: crate::context::anatomy::LABEL_TOOL_DEFS.to_string(),
                est,
                count: Some(count),
            });
        }
        let est_total: u64 = self.anatomy.segments.iter().map(|s| s.est).sum();
        let actual = self.rounds.first().map(|r| r.prompt);
        // fingerprint.tools 取**轮 0** 指纹：跨回合比对基线是「回合起点的工具
        // 列表」——末轮可能是相关性裁剪后的子集，拿它比对会把正常回合判成
        // tools_changed（恒真噪声）。
        let round0_tools = self.rounds[0].tools_hash.clone();
        Some(ContextBreakdownPayload {
            v: 1,
            segments: self.anatomy.segments,
            est_total,
            actual_prompt_tokens: actual,
            fingerprint: ContextBreakdownFingerprint {
                tools: round0_tools,
                system_stable: self.anatomy.system_stable,
                os_stable: self.anatomy.os_stable,
            },
            rounds: self.rounds,
        })
    }
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn prev(tools: &str, sys: &str, os: &str, inj: bool, sw: bool) -> PrevBaseline {
        PrevBaseline {
            tools: tools.into(),
            system_stable: sys.into(),
            os_stable: os.into(),
            last_injected: inj,
            any_switched: sw,
        }
    }

    fn input<'a>(
        tools: &'a str,
        sys: &'a str,
        os: &'a str,
        inj: bool,
        sw: bool,
        zero: bool,
    ) -> MissInput<'a> {
        MissInput {
            tools,
            system_stable: sys,
            os_stable: os,
            injected: inj,
            model_switched: sw,
            is_round_zero: zero,
        }
    }

    const T: &str = "tools-hash-a";
    const S: &str = "sys-hash-a";
    const O: &str = "os-hash-a";

    #[test]
    fn attribute_first_request_is_exclusive() {
        // 无基线：仅 first_request（哪怕其他输入全在变）
        let out = attribute_miss(&input("x", "y", "z", true, true, true), None);
        assert_eq!(out, vec![miss_slug::FIRST_REQUEST]);
    }

    #[test]
    fn attribute_all_change_slugs_coexist() {
        let p = prev("other", "other", "other", false, false);
        let out = attribute_miss(&input(T, S, O, true, true, true), Some(&p));
        assert_eq!(
            out,
            vec![
                miss_slug::TOOLS_CHANGED,
                miss_slug::SYSTEM_STABLE_CHANGED,
                miss_slug::OS_ENV_CHANGED,
                miss_slug::INJECTION_CHANGED,
                miss_slug::MODEL_SWITCHED,
            ]
        );
    }

    #[test]
    fn attribute_no_change_falls_to_no_detectable() {
        let p = prev(T, S, O, false, false);
        let out = attribute_miss(&input(T, S, O, false, false, true), Some(&p));
        assert_eq!(out, vec![miss_slug::NO_DETECTABLE_CHANGE]);
    }

    #[test]
    fn attribute_round_ge1_skips_system_os_compare() {
        // 同回合内 system/os 恒同；轮 ≥1 即使传不同值也不归因（防恒真噪声）
        let p = prev(T, S, O, false, false);
        let out = attribute_miss(&input(T, "changed", "changed", false, false, false), Some(&p));
        assert_eq!(out, vec![miss_slug::NO_DETECTABLE_CHANGE]);
    }

    #[test]
    fn attribute_baseline_switch_alone_counts() {
        // 上回合换过档：本回合轮 0 即使一切相同也归 model_switched
        let p = prev(T, S, O, false, true);
        let out = attribute_miss(&input(T, S, O, false, false, true), Some(&p));
        assert_eq!(out, vec![miss_slug::MODEL_SWITCHED]);
    }

    #[test]
    fn attribute_empty_tools_hash_means_disabled_tools() {
        // 工具禁用（空指纹）vs 上回合有工具 → tools_changed
        let p = prev(T, S, O, false, false);
        let out = attribute_miss(&input("", S, O, false, false, true), Some(&p));
        assert_eq!(out, vec![miss_slug::TOOLS_CHANGED]);
    }

    #[test]
    fn prev_baseline_from_payload_parses_and_degrades() {
        let json = r#"{"v":1,"segments":[],"est_total":0,
            "fingerprint":{"tools":"t1","system_stable":"s1","os_stable":"o1"},
            "rounds":[
              {"prompt":100,"cached":0,"tools_hash":"t1","injected":false,"model_switched":false},
              {"prompt":200,"cached":200,"tools_hash":"t1","injected":true,"model_switched":true}
            ]}"#;
        let b = PrevBaseline::from_payload(json).unwrap();
        assert_eq!(b.tools, "t1");
        assert_eq!(b.system_stable, "s1");
        assert!(b.last_injected, "取最后一轮的注入位");
        assert!(b.any_switched, "任一轮换档即真");

        // 空轮次 / 坏 JSON → None（调用方降级 first_request）
        let empty_rounds = r#"{"v":1,"segments":[],"est_total":0,
            "fingerprint":{"tools":"t","system_stable":"s","os_stable":"o"},"rounds":[]}"#;
        assert!(PrevBaseline::from_payload(empty_rounds).is_none());
        assert!(PrevBaseline::from_payload("not json").is_none());
    }

    fn tool_def(name: &str) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: "d".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn recorder_round_trip_and_miss_gates() {
        let defs = [tool_def("a"), tool_def("b")];
        // 轮 0 不归 tools_changed 的前提 = 基线指纹与真实计算值相等
        //（跨回合基线的 tools 用 hash_tool_defs 真实值而非手写串）
        let full_hash = crate::context::anatomy::hash_tool_defs(&defs);
        let mut rec = TurnCostRecorder::new(ContextAnatomyInput {
            segments: vec![ContextBreakdownSegment {
                label: "history".into(),
                est: 900,
                count: Some(4),
            }],
            system_stable: S.into(),
            os_stable: O.into(),
            prev: Some(prev(&full_hash, S, O, false, false)),
        });
        rec.begin_round(Some(&defs));
        // 轮 0 全 miss（prompt 达门槛）→ 无变化 → no_detectable_change
        let hint = rec.record_usage(4_096, 0, false).unwrap();
        assert_eq!(hint, vec![miss_slug::NO_DETECTABLE_CHANGE.to_string()]);

        // 轮 1：换档 + 工具子集变化 + 注入翻转 → 三 slug
        rec.mark_model_switched();
        rec.begin_round(Some(&defs[..1]));
        let hint = rec.record_usage(6_000, 0, true).unwrap();
        assert_eq!(
            hint,
            vec![
                miss_slug::TOOLS_CHANGED.to_string(),
                miss_slug::INJECTION_CHANGED.to_string(),
                miss_slug::MODEL_SWITCHED.to_string(),
            ]
        );

        // 部分 miss（cached>0）→ None（门槛：全 miss 才出 hint）
        rec.begin_round(Some(&defs[..1]));
        assert!(rec.record_usage(8_000, 6_000, false).is_none());
        // prompt 低于门槛 → None
        assert!(rec.record_usage(512, 0, false).is_none());

        let p = rec.into_payload().unwrap();
        assert_eq!(p.segments.len(), 2, "anatomy 段 + tool_defs 段");
        assert_eq!(p.segments[1].label, "tool_defs");
        assert_eq!(p.segments[1].count, Some(2));
        assert!(p.segments[1].est > 0);
        assert_eq!(p.est_total, 900 + p.segments[1].est);
        assert_eq!(p.actual_prompt_tokens, Some(4_096), "首轮 prompt 真值");
        // fingerprint.tools = 轮 0 指纹（跨回合基线），非末轮子集
        assert_eq!(p.fingerprint.tools, p.rounds[0].tools_hash);
        assert_ne!(p.fingerprint.tools, p.rounds[1].tools_hash);
        assert_eq!(p.rounds.len(), 4, "每次 record_usage 一条（含低门槛轮）");
        assert!(p.rounds[1].model_switched);
        assert!(p.rounds[1].injected);
        assert_eq!(p.rounds[2].cached, 6_000, "部分命中轮照记序列");
    }

    #[test]
    fn recorder_round_zero_uses_cross_turn_prev_then_advances() {
        // 无跨回合基线 + 无记录 → 轮 0 归 first_request；轮 1 起用内存上一轮
        let mut rec = TurnCostRecorder::new(ContextAnatomyInput {
            segments: vec![],
            system_stable: S.into(),
            os_stable: O.into(),
            prev: None,
        });
        rec.begin_round(None);
        let hint = rec.record_usage(2_000, 0, false).unwrap();
        assert_eq!(hint, vec![miss_slug::FIRST_REQUEST.to_string()]);
        // 轮 1 同输入 → 与上一轮快照比对 → no_detectable_change
        rec.begin_round(None);
        let hint = rec.record_usage(2_000, 0, false).unwrap();
        assert_eq!(hint, vec![miss_slug::NO_DETECTABLE_CHANGE.to_string()]);
    }

    #[test]
    fn recorder_zero_requests_yields_none() {
        let rec = TurnCostRecorder::new(ContextAnatomyInput::default());
        assert!(rec.into_payload().is_none());
    }
}
