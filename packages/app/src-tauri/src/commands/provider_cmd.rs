//! `commands::provider_cmd` — Provider 目录下发 + 连通性测试
//!
//! - `list_providers`：注册表直出（前端目录/校验规则的唯一数据源）
//! - `test_provider_connection`：一次 GET /models 同时回答「通不通」和
//!   「有哪些模型」。「测试连接」与「拉取模型」两个按钮共用，不做两次往返。
//!
//! 解析优先级（base_url 与 api_key 同构）：
//! 入参（表单当前值）> agent 存量（编辑态，密文不回显、后端代取）> 注册表默认。

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::harness::provider::{create_provider, list_provider_infos, probe, ProviderInfo};
use crate::infra::cancel::CancellationToken;
use crate::infra::protocol::ChatMessage;

use super::agent_cmd::AgentCmd;
use super::model_profile_cmd::ModelProfileCmd;

/// Provider 目录（注册表快照，前端下拉框数据源）
#[tauri::command]
pub async fn list_providers() -> AppResult<Vec<ProviderInfo>> {
    // async 化：同步命令跑 Tauri 主线程，生成中事件注入洪泛时会被排队十几秒
    // （模型下拉数据源）。纯内存快照，async 化零成本。
    Ok(list_provider_infos())
}

/// 连通性测试结果。**探测失败不是命令失败**：`ok:false + error` 结构化返回，
/// 前端行内展示具体原因（HTTP 状态/网络错误），不弹通用错误框。
/// `matched_url`：实际走通的端点地址（多端点回退探测时可能是备选端点）——
/// 前端据此回填 API URL，把「这次测通了」固化成「以后都走它」。
#[derive(Debug, serde::Serialize)]
pub struct ProviderConnectionResult {
    pub ok: bool,
    pub model_count: usize,
    pub models: Vec<String>,
    pub error: Option<String>,
    pub matched_url: Option<String>,
}

impl ProviderConnectionResult {
    fn failed(error: String) -> Self {
        Self {
            ok: false,
            model_count: 0,
            models: Vec::new(),
            error: Some(error),
            matched_url: None,
        }
    }
}

/// 存量凭据三元组（agent / model profile 两个编辑态来源归一；key 密文不回显
/// 前端，命令层代取后传此处）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCreds {
    pub provider: String,
    pub api_key: String,
    /// 已固化的显式端点（agent 行 / profile 行或其 vault 副本；None = 走注册表推导）
    pub base_url: Option<String>,
}

/// 探测目标解析结果（纯函数产物，见 `resolve_probe_target`）
#[derive(Debug, PartialEq, Eq)]
pub enum ProbeTarget {
    /// `explicit_base_url`：表单入参或存量里的显式地址（None = 未显式
    /// 指定，探测时按注册表 [默认, ...备选] 顺序回退）
    Ready {
        explicit_base_url: Option<String>,
        api_key: String,
    },
    /// 需鉴权的 provider 但没有任何可用 key——不发注定 401 的请求，
    /// 直接给「先选内置目录 / 填 Key 再拉」的引导（模型浏览与拉取分离）
    MissingKey,
}

/// 解析探测目标（纯函数，可单测）。规则：
///
/// - base_url：表单入参 > 存量（作为「显式指定」透传，探测只测它）；
///   都没有 → 用注册表候选序列（默认 + 备选）。custom 等必填项为空 → Validation
/// - key：表单入参 > 存量（**仅当存量的 provider 与被测 provider 同名**——
///   各家 key 互不通用的居多，GLM 标准/Coding 尤甚，拿旧 key 打新端点只会
///   报一个误导性的 401）> 空
/// - requires_key 且最终 key 为空 → `MissingKey`（调用方短路，不发请求）
pub fn resolve_probe_target(
    info: &ProviderInfo,
    base_url_input: Option<&str>,
    api_key_input: Option<&str>,
    stored: Option<&StoredCreds>,
) -> AppResult<ProbeTarget> {
    // 显式地址：入参 > agent 存量（两者都是用户/系统明确选定的，探测只测它）
    let explicit_base_url = base_url_input
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            stored
                .and_then(|c| c.base_url.clone())
                .filter(|s| !s.trim().is_empty())
        });
    // 未显式指定时必须有注册表默认（custom 无默认 = 必须显式填）
    if explicit_base_url.is_none() && info.default_url.is_empty() {
        return Err(AppError::Validation(
            "自定义 Provider 必须填写 API URL（如 http://localhost:8000/v1）".into(),
        ));
    }

    // key 解析：入参 > 同 provider 的存量（跨 provider 的存量 key 不混用）
    let same_provider = stored.map(|c| c.provider.as_str()) == Some(info.name.as_str());
    let api_key = api_key_input
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| stored.filter(|_| same_provider).map(|c| c.api_key.clone()))
        .unwrap_or_default();

    if info.requires_key && api_key.is_empty() {
        return Ok(ProbeTarget::MissingKey);
    }
    Ok(ProbeTarget::Ready {
        explicit_base_url,
        api_key,
    })
}

/// 与存量等值的入参降级为 None（纯函数，可单测）：
/// - base_url 按 trim 比较——表单回显的同一地址与存量语义等同；
/// - api_key 明文等值——重贴原 Key = 未更换。
///
/// 降级**不改探测行为**（显式值=存量值，resolve 落到同一路径），变化的是健康
/// 归因：降级后「纯存量测试」判据（两入参皆 absent）恢复成立，探测结果如实
/// 沉淀为存量配置状态。生产实案（2026-09-09）：模型页把未改动的 base_url
/// 回显值当覆盖值传，存了自定义端点的档位状态点/最后调用时间恒不更新。
fn demote_unchanged_inputs(
    base_url: Option<String>,
    api_key: Option<String>,
    stored: Option<&StoredCreds>,
) -> (Option<String>, Option<String>) {
    let Some(c) = stored else {
        return (base_url, api_key);
    };
    let base_url = match base_url.as_deref() {
        Some(v) if Some(v.trim()) == c.base_url.as_deref().map(str::trim) => None,
        _ => base_url,
    };
    let api_key = match api_key.as_deref() {
        Some(v) if v.trim() == c.api_key.trim() => None,
        _ => api_key,
    };
    (base_url, api_key)
}

/// 探测候选端点序列（纯函数，可单测）：显式地址只测它自己；未显式时
/// [注册表默认, ...备选] 按序回退（智谱：标准端点 → Coding 端点自动匹配）。
pub fn probe_candidates(
    info: &ProviderInfo,
    explicit_base_url: Option<&str>,
) -> Vec<(String, String)> {
    match explicit_base_url {
        Some(u) => vec![("指定地址".to_string(), u.to_string())],
        None => std::iter::once(("标准端点".to_string(), info.default_url.clone()))
            .chain(
                info.alt_urls
                    .iter()
                    .map(|(l, u)| (l.to_string(), u.to_string())),
            )
            .collect(),
    }
}

/// 全部候选失败时的聚合文案：多端点时逐个标注失败原因，单端点保持原样
pub fn aggregate_probe_error(candidates: &[(String, String)], errors: &[String]) -> String {
    if candidates.len() <= 1 {
        return errors.join("");
    }
    let labeled: Vec<String> = candidates
        .iter()
        .zip(errors.iter())
        .map(|((label, _), e)| format!("{}：{}", label, e))
        .collect();
    format!("全部端点未通过——{}", labeled.join("；"))
}

/// 测试 provider 连通性并拉取模型列表。
///
/// - `provider_name`：注册表内的 provider 名（未知 → Validation）
/// - `base_url` / `api_key`：表单当前值，缺省时回退存量/注册表默认
/// - `agent_id`：Agent 表单编辑态——用存量 agent 凭据探测（key 密文不回显前端）
/// - `profile_id`：模型配置卡编辑态——用存量 ModelProfile 凭据探测（同构语义，
///   ModelProfile Phase 1）；与 `agent_id` 同时传时 profile 优先（前端不会同传）
#[tauri::command]
pub async fn test_provider_connection(
    cmd: State<'_, std::sync::Arc<dyn AgentCmd>>,
    profiles: State<'_, std::sync::Arc<dyn ModelProfileCmd>>,
    provider_name: String,
    base_url: Option<String>,
    api_key: Option<String>,
    agent_id: Option<String>,
    profile_id: Option<String>,
) -> AppResult<ProviderConnectionResult> {
    let info = list_provider_infos()
        .into_iter()
        .find(|p| p.name == provider_name)
        .ok_or_else(|| AppError::Validation(format!("未知的 Provider: {}", provider_name)))?;

    // 编辑态：取存量凭据（key + 曾用 base_url，后端代取）归一为三元组
    let stored = match (
        profile_id.as_deref().filter(|s| !s.is_empty()),
        agent_id.as_deref().filter(|s| !s.is_empty()),
    ) {
        (Some(pid), _) => {
            let c = profiles.inner().get_with_credentials(pid).await?;
            Some(StoredCreds {
                provider: c.profile.provider,
                api_key: c.api_key,
                base_url: c.base_url,
            })
        }
        (None, Some(aid)) => {
            let c = cmd.inner().get_with_credentials(aid).await?;
            Some(StoredCreds {
                provider: c.agent.provider,
                api_key: c.api_key,
                base_url: c.base_url,
            })
        }
        (None, None) => None,
    };

    // 入参与存量等值 ≠ 覆盖——降级后再走归因/探测（见函数注释；不改探测值，
    // 只恢复存量测试的健康归因）。
    let (base_url, api_key) = demote_unchanged_inputs(base_url, api_key, stored.as_ref());

    // 状态监控归因（warn-only 旁路）：仅「纯存量测试」才记——profile 腿且表单
    // 未传任何覆盖值（base_url / api_key 都 absent）。草稿测试测的是未保存的
    // 新值，结果不代表存量配置状态，不冒充。
    let tracked_profile = match (
        profile_id.as_deref().filter(|s| !s.is_empty()),
        base_url.is_none(),
        api_key.is_none(),
    ) {
        (Some(pid), true, true) => Some(pid.to_string()),
        _ => None,
    };

    let target = resolve_probe_target(
        &info,
        base_url.as_deref(),
        api_key.as_deref(),
        stored.as_ref(),
    )?;
    let (explicit_base_url, api_key) = match target {
        ProbeTarget::Ready {
            explicit_base_url,
            api_key,
        } => (explicit_base_url, api_key),
        ProbeTarget::MissingKey => {
            tracing::info!(
                target: "ice_paw.llm",
                "探测 Provider: {} | 短路：需鉴权但无可用 Key",
                provider_name,
            );
            return Ok(ProviderConnectionResult::failed(
                "在线拉取需要 API Key（该服务的模型列表接口需鉴权）。可先从下拉内置目录选择常用模型，填好 Key 后再拉取完整列表".into(),
            ));
        }
    };

    let candidates = probe_candidates(&info, explicit_base_url.as_deref());
    tracing::info!(
        target: "ice_paw.llm",
        "探测 Provider: {} | 候选端点={} | has_key={} | protocol={:?}",
        provider_name,
        candidates.iter().map(|(_, u)| u.as_str()).collect::<Vec<_>>().join(" → "),
        !api_key.is_empty(),
        info.protocol,
    );

    // 按序回退：任一端点走通即返回（matched_url 让前端把走通的固化下来）；
    // 全部失败 → 聚合各端点原因（多端点逐个标注）
    let mut errors: Vec<String> = Vec::new();
    for (_, url) in &candidates {
        match probe::probe_models(info.protocol, url, &api_key).await {
            Ok(models) => {
                if let Some(pid) = &tracked_profile {
                    profiles.inner().record_health(pid, "ok", None).await;
                }
                return Ok(ProviderConnectionResult {
                    ok: true,
                    model_count: models.len(),
                    models,
                    error: None,
                    matched_url: Some(url.clone()),
                });
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    let aggregated = aggregate_probe_error(&candidates, &errors);
    if let Some(pid) = &tracked_profile {
        use crate::harness::profile_health::{clip_detail, health_from_error};
        profiles
            .inner()
            .record_health(
                pid,
                health_from_error(&aggregated).as_str(),
                Some(&clip_detail(&aggregated)),
            )
            .await;
    }
    Ok(ProviderConnectionResult::failed(aggregated))
}

// ============================================================================
// 链路测试（agent 引用态「测试连接」）：模拟 agent 发送路径
// ============================================================================

/// 链路测试结果。ok=true 时 profile_id/alias/model = 命中档位（非首档即主模型
/// 故障后降级命中，前端文案区分）；ok=false = 全链不可用，error 为最后失败
/// 原文（档位别名前置）。
#[derive(Debug, serde::Serialize)]
pub struct ChainTestResult {
    pub ok: bool,
    pub profile_id: Option<String>,
    pub alias: Option<String>,
    pub model: Option<String>,
    pub error: Option<String>,
}

impl ChainTestResult {
    fn failed(error: String) -> Self {
        Self {
            ok: false,
            profile_id: None,
            alias: None,
            model: None,
            error: Some(error),
        }
    }
}

/// 单档探测结果（`walk_chain_for_test` 消费）：
/// - `Ok`：真发成功（携带别名/模型供回显——命中档位即答案）
/// - `Failed`：真发失败（是否推进下一档由 `slot_error_advances` 分类决定）
/// - `Skipped`：档位不可解析（配置已删 / 厂商不识别）——恒推进，与运行时
///   「resolve Err 跳档」同语义
enum SlotOutcome {
    Ok { alias: String, model: String },
    Failed(String),
    Skipped(String),
}

/// 失败是否推进下一档（纯函数）：与运行时降级链**同一张分类表**
/// （`fallback_trigger(classify_llm_error)`）——quota / rate_limited / network
/// 推进，Auth/Forbidden/ContextTooLong/Sensitive/Unknown 终止（换模型不解决，
/// 让用户看到真实错误）。测试所见即运行时所得。
fn slot_error_advances(msg: &str) -> bool {
    crate::harness::r#loop::fallback::fallback_trigger(crate::error::classify_llm_error(msg))
        .is_some()
}

/// 逐档走链（泛型内核，probe 直传——闭包经泛型结构体间接传入会触发 rustc
/// HRTB bug，直传是终态，勿收拢）：命中即返回；推进族失败换下一档；终止族
/// 失败原样返回；链尽聚合最后错误。
async fn walk_chain_for_test<F, Fut>(chain: &[String], mut probe: F) -> ChainTestResult
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = SlotOutcome>,
{
    let mut last_error: Option<String> = None;
    for pid in chain {
        match probe(pid.clone()).await {
            SlotOutcome::Ok { alias, model } => {
                return ChainTestResult {
                    ok: true,
                    profile_id: Some(pid.clone()),
                    alias: Some(alias),
                    model: Some(model),
                    error: None,
                };
            }
            SlotOutcome::Failed(msg) => {
                if slot_error_advances(&msg) {
                    tracing::info!(
                        target: "ice_paw.llm",
                        "链路测试：档位 {pid} 失败（推进）——{msg}"
                    );
                    last_error = Some(msg);
                    continue;
                }
                tracing::info!(
                    target: "ice_paw.llm",
                    "链路测试：档位 {pid} 失败（终止，不换档）——{msg}"
                );
                return ChainTestResult::failed(msg);
            }
            SlotOutcome::Skipped(msg) => {
                tracing::info!(target: "ice_paw.llm", "链路测试：跳过档位 {pid}——{msg}");
                last_error = Some(msg);
            }
        }
    }
    ChainTestResult::failed(format!(
        "链上 {} 个档位全部不可用——最后错误：{}",
        chain.len(),
        last_error.unwrap_or_else(|| "未发生调用".into())
    ))
}

/// 单档真发探测：走 `stream_summary` utility 通道（GLM 5.3 thinking low / 其余
/// disabled 策略内建，与滚动摘要/视觉代读同源）发一条 16 token 请求并消费到
/// 结束——列模型是鉴权层动作（智谱 Coding 1113 假绿实案），对话权益只有真发
/// 一条才验得出。30s 超时，文案含「超时」→ Network 类 → 可推进（分类表一致）。
///
/// 健康记录逐档在此做（warn-only）：Ok → ok；Failed → health_from_error +
/// clip_detail；Skipped → 不记（配置问题不是模型健康问题）。
async fn probe_chat_slot(
    profiles: &std::sync::Arc<dyn ModelProfileCmd>,
    pid: String,
) -> SlotOutcome {
    use futures::StreamExt;

    let cred = match profiles.get_with_credentials(&pid).await {
        Ok(c) => c,
        Err(_) => {
            return SlotOutcome::Skipped("无法解析（配置已删除或凭据不可读），已跳过该档".into());
        }
    };
    let alias = cred.profile.alias.clone();
    let provider = match create_provider(
        &cred.profile.provider,
        &cred.profile.model,
        cred.base_url.as_deref(),
        false,
    ) {
        Ok(p) => p,
        Err(e) => {
            return SlotOutcome::Skipped(format!("无法创建 Provider：{e}，已跳过该档"));
        }
    };
    let cancel = CancellationToken::new();
    let attempt = async {
        let mut stream = provider
            .stream_summary(
                &cred.api_key,
                vec![ChatMessage::from_text("user", "连接测试".to_string())],
                0.0,
                16,
                cancel,
            )
            .await?;
        while let Some(delta) = stream.next().await {
            delta?;
        }
        Ok::<(), AppError>(())
    };
    let outcome = match tokio::time::timeout(std::time::Duration::from_secs(30), attempt).await {
        Ok(Ok(())) => {
            profiles.record_health(&pid, "ok", None).await;
            SlotOutcome::Ok {
                alias,
                model: cred.profile.model.clone(),
            }
        }
        Ok(Err(e)) => {
            let raw = e.to_string();
            record_health_from_error(profiles, &pid, &raw).await;
            SlotOutcome::Failed(format!("「{alias}」：{raw}"))
        }
        Err(_) => {
            let raw = "请求超时（30 秒内未完成响应）".to_string();
            record_health_from_error(profiles, &pid, &raw).await;
            SlotOutcome::Failed(format!("「{alias}」：{raw}"))
        }
    };
    outcome
}

/// 失败档健康沉淀（warn-only 旁路，与 test_provider_connection 同规则）
async fn record_health_from_error(
    profiles: &std::sync::Arc<dyn ModelProfileCmd>,
    pid: &str,
    raw: &str,
) {
    use crate::harness::profile_health::{clip_detail, health_from_error};
    profiles
        .record_health(
            pid,
            health_from_error(raw).as_str(),
            Some(&clip_detail(raw)),
        )
        .await;
}

/// 链路测试（agent 引用态「测试连接」）：模拟 agent 发送路径——主模型起按序
/// 逐档真发一条极小额度摘要请求，按运行时降级链同一张分类表决定推进/终止，
/// 报告链上首个可对话档位。与 `test_provider_connection`（GET /models）分工：
/// 那个答「通不通、有哪些模型」，这个答「照这条链发消息能不能成」。
#[tauri::command]
pub async fn test_agent_model_chain(
    profiles: State<'_, std::sync::Arc<dyn ModelProfileCmd>>,
    chain: Vec<String>,
) -> AppResult<ChainTestResult> {
    if chain.is_empty() {
        return Ok(ChainTestResult::failed(
            "请先选择至少一个模型配置（第一个为主模型）".into(),
        ));
    }
    tracing::info!(
        target: "ice_paw.llm",
        "链路测试：{} 档，主模型 = 首档",
        chain.len()
    );
    let profiles = profiles.inner().clone();
    // 逐档克隆 Arc 进 async 块（FnMut 捕获变量的引用不可逃逸出闭包体）
    Ok(walk_chain_for_test(&chain, move |pid| {
        let profiles = profiles.clone();
        async move { probe_chat_slot(&profiles, pid).await }
    })
    .await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str) -> ProviderInfo {
        // 从真实注册表取（顺带保证测试与注册表不脱钩）
        list_provider_infos()
            .into_iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("注册表缺少 {}", name))
    }

    /// 存量凭据三元组（命令层从 agent / model profile 两个来源归一后的形态，
    /// `resolve_probe_target` 消费它）
    fn stored_creds(provider: &str, api_key: &str, base_url: Option<&str>) -> StoredCreds {
        StoredCreds {
            provider: provider.into(),
            api_key: api_key.into(),
            base_url: base_url.map(|s| s.into()),
        }
    }

    #[test]
    fn requires_key_without_any_key_short_circuits() {
        // 新建态没填 key：不发注定 401 的请求，交给上层给引导文案
        let i = info("deepseek");
        assert_eq!(
            resolve_probe_target(&i, None, None, None).unwrap(),
            ProbeTarget::MissingKey
        );
    }

    #[test]
    fn input_key_and_url_win() {
        let i = info("deepseek");
        assert_eq!(
            resolve_probe_target(&i, Some(" https://x/v1 "), Some(" sk-abc "), None).unwrap(),
            ProbeTarget::Ready {
                explicit_base_url: Some("https://x/v1".into()),
                api_key: "sk-abc".into()
            }
        );
    }

    #[test]
    fn stored_key_used_only_for_same_provider() {
        let glm = info("glm");
        // 同 provider：编辑态 key 留空，用存量 key 探测（密文不回显）
        let same = stored_creds("glm", "glm-key", None);
        assert_eq!(
            resolve_probe_target(&glm, None, None, Some(&same)).unwrap(),
            ProbeTarget::Ready {
                explicit_base_url: None,
                api_key: "glm-key".into()
            }
        );
        // 跨 provider：glm 的存量 key 不拿去打 deepseek（各家 key 多不通用）
        let cross = stored_creds("glm", "glm-key", None);
        assert_eq!(
            resolve_probe_target(&info("deepseek"), None, None, Some(&cross)).unwrap(),
            ProbeTarget::MissingKey
        );
    }

    #[test]
    fn stored_base_url_counts_as_explicit() {
        // 存量 base_url 是已固化的选择：只测它自己，不做多端点回退
        let glm = info("glm");
        let stored = stored_creds(
            "glm",
            "k",
            Some("https://open.bigmodel.cn/api/coding/paas/v4"),
        );
        assert_eq!(
            resolve_probe_target(&glm, None, None, Some(&stored)).unwrap(),
            ProbeTarget::Ready {
                explicit_base_url: Some("https://open.bigmodel.cn/api/coding/paas/v4".into()),
                api_key: "k".into()
            }
        );
    }

    #[test]
    fn keyless_provider_probes_with_empty_key() {
        // ollama：无 key 照常探测（本地服务忽略空 Bearer）
        let i = info("ollama");
        assert_eq!(
            resolve_probe_target(&i, None, None, None).unwrap(),
            ProbeTarget::Ready {
                explicit_base_url: None,
                api_key: String::new()
            }
        );
    }

    #[test]
    fn custom_without_url_rejected() {
        let i = info("custom");
        let err = resolve_probe_target(&i, None, None, None).unwrap_err();
        assert!(err.to_string().contains("必须填写 API URL"));
    }

    #[test]
    fn probe_candidates_explicit_is_single() {
        // 显式地址（表单/存量）：只测它自己
        let glm = info("glm");
        let c = probe_candidates(&glm, Some("https://my-proxy/v1"));
        assert_eq!(c.len(), 1);
        assert_eq!(
            c[0],
            ("指定地址".to_string(), "https://my-proxy/v1".to_string())
        );
    }

    #[test]
    fn probe_candidates_glm_falls_back_to_coding() {
        // 未显式指定：智谱按 标准 → Coding 顺序回退（key 不通用，自动匹配）
        let glm = info("glm");
        let c = probe_candidates(&glm, None);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].1, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(
            c[1],
            (
                "Coding 端点".to_string(),
                "https://open.bigmodel.cn/api/coding/paas/v4".to_string()
            )
        );
        // 无备选的 provider：单候选
        assert_eq!(probe_candidates(&info("deepseek"), None).len(), 1);
    }

    #[test]
    fn aggregate_error_labels_each_endpoint() {
        // 多端点全败：逐个标注 + 总前缀；单端点：原样透传
        let candidates = vec![
            ("标准端点".to_string(), "https://a".to_string()),
            ("Coding 端点".to_string(), "https://b".to_string()),
        ];
        let msg = aggregate_probe_error(
            &candidates,
            &["HTTP 401: 认证失败".into(), "HTTP 404".into()],
        );
        assert!(msg.contains("全部端点未通过"));
        assert!(msg.contains("标准端点：HTTP 401: 认证失败"));
        assert!(msg.contains("Coding 端点：HTTP 404"));
        let single = vec![("指定地址".to_string(), "https://a".to_string())];
        assert_eq!(aggregate_probe_error(&single, &["boom".into()]), "boom");
    }

    #[test]
    fn demote_unchanged_inputs_matches_stored_only() {
        let stored = stored_creds("glm", "sk-a", Some("https://x/v1"));
        // 同值（含首尾空白差异）→ 降级：健康归因恢复「纯存量测试」
        let (b, k) = demote_unchanged_inputs(
            Some(" https://x/v1 ".into()),
            Some("sk-a".into()),
            Some(&stored),
        );
        assert_eq!(b, None);
        assert_eq!(k, None);
        // 异值保留（草稿覆盖语义——测未保存的新值，不冒充存量状态）
        let (b, k) = demote_unchanged_inputs(
            Some("https://y/v1".into()),
            Some("sk-b".into()),
            Some(&stored),
        );
        assert_eq!(b.as_deref(), Some("https://y/v1"));
        assert_eq!(k.as_deref(), Some("sk-b"));
        // 存量无端点：空串入参不与 None 判等（保留——resolve 侧过滤为「未显式」）
        let stored_no_url = stored_creds("glm", "sk-a", None);
        let (b, _) = demote_unchanged_inputs(Some("".into()), None, Some(&stored_no_url));
        assert_eq!(b, Some("".into()));
        // 无存量（纯表单新建）原样透传
        let (b, k) = demote_unchanged_inputs(Some("https://z".into()), Some("sk-c".into()), None);
        assert_eq!(b.as_deref(), Some("https://z"));
        assert_eq!(k.as_deref(), Some("sk-c"));
    }

    // ---------------- 链路测试（walk 内核 + 换档分类表） ----------------

    fn chain(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    /// 分类表镜像断言：推进族（quota/限流/网络含超时）与终止族（鉴权等）
    /// 与运行时 fallback_trigger 同表——测试所见即运行时所得
    #[test]
    fn slot_error_advances_matches_fallback_table() {
        // 推进族
        assert!(slot_error_advances("code:1113 余额不足或无可用资源包"));
        assert!(slot_error_advances("insufficient balance"));
        assert!(slot_error_advances("HTTP 429: Too Many Requests"));
        assert!(slot_error_advances("请求超时（30 秒内未完成响应）"));
        assert!(slot_error_advances("connection refused"));
        // 终止族：换模型不解决，原样终止
        assert!(!slot_error_advances("HTTP 401: unauthorized"));
        assert!(!slot_error_advances("HTTP 403: forbidden"));
        assert!(!slot_error_advances("context_length_exceeded"));
        assert!(!slot_error_advances("image is sensitive"));
    }

    #[tokio::test]
    async fn chain_test_first_slot_ok_reports_it_and_stops() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen = calls.clone();
        let r = walk_chain_for_test(&chain(&["mp-1", "mp-2"]), move |pid| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(pid);
                SlotOutcome::Ok {
                    alias: "智谱主力".into(),
                    model: "glm-5.3-flash".into(),
                }
            }
        })
        .await;
        assert!(r.ok);
        assert_eq!(r.profile_id.as_deref(), Some("mp-1"));
        assert_eq!(r.alias.as_deref(), Some("智谱主力"));
        assert_eq!(r.model.as_deref(), Some("glm-5.3-flash"));
        assert!(r.error.is_none());
        // 命中即停：第二档不被调用
        assert_eq!(*calls.lock().unwrap(), vec!["mp-1".to_string()]);
    }

    #[tokio::test]
    async fn chain_test_rate_limited_advances_to_next_slot() {
        // 429 推进族：主档限流 → 降级档命中，ok=true + 命中第二档
        let r = walk_chain_for_test(&chain(&["mp-1", "mp-2"]), |pid| async move {
            if pid == "mp-1" {
                SlotOutcome::Failed("「主力」：HTTP 429: Too Many Requests".into())
            } else {
                SlotOutcome::Ok {
                    alias: "备用".into(),
                    model: "deepseek-v4".into(),
                }
            }
        })
        .await;
        assert!(r.ok);
        assert_eq!(r.profile_id.as_deref(), Some("mp-2"));
    }

    #[tokio::test]
    async fn chain_test_auth_error_stops_without_trying_next() {
        // 401 终止族：换档不解决（多数 401 是 key 本身失效），原样返回 + 不打后续档
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen = calls.clone();
        let r = walk_chain_for_test(&chain(&["mp-1", "mp-2"]), move |pid| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(pid);
                SlotOutcome::Failed("「主力」：HTTP 401: unauthorized".into())
            }
        })
        .await;
        assert!(!r.ok);
        assert!(r.profile_id.is_none());
        assert!(r.error.unwrap().contains("401"));
        assert_eq!(*calls.lock().unwrap(), vec!["mp-1".to_string()]);
    }

    #[tokio::test]
    async fn chain_test_timeout_advances_and_exhaustion_reports_last_error() {
        // 超时（Network 推进族）换档；链尽聚合最后错误（第二档的 429）
        let r = walk_chain_for_test(&chain(&["mp-1", "mp-2"]), |pid| async move {
            if pid == "mp-1" {
                SlotOutcome::Failed("「主力」：请求超时（30 秒内未完成响应）".into())
            } else {
                SlotOutcome::Failed("「备用」：HTTP 429: Too Many Requests".into())
            }
        })
        .await;
        assert!(!r.ok);
        let err = r.error.unwrap();
        assert!(err.contains("全部不可用"), "链尽聚合文案：{err}");
        assert!(err.contains("429"), "最后错误取链尾：{err}");
    }

    #[tokio::test]
    async fn chain_test_skipped_slots_advance() {
        // 跳档（配置已删等）恒推进——运行时 resolve Err 跳档同语义；链尽聚合
        let r = walk_chain_for_test(&chain(&["mp-1"]), |pid| async move {
            SlotOutcome::Skipped(format!("档位 {pid} 无法解析（配置已删除）"))
        })
        .await;
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("全部不可用"));
    }
}
