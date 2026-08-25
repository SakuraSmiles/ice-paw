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
use crate::harness::provider::{list_provider_infos, probe, ProviderInfo};

use super::agent_cmd::{AgentCmd, AgentWithCredentials};

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

/// 探测目标解析结果（纯函数产物，见 `resolve_probe_target`）
#[derive(Debug, PartialEq, Eq)]
pub enum ProbeTarget {
    /// `explicit_base_url`：表单入参或 agent 存量里的显式地址（None = 未显式
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
/// - base_url：表单入参 > agent 存量（作为「显式指定」透传，探测只测它）；
///   都没有 → 用注册表候选序列（默认 + 备选）。custom 等必填项为空 → Validation
/// - key：表单入参 > agent 存量（**仅当存量 agent 的 provider 与被测 provider
///   同名**——各家 key 互不通用的居多，GLM 标准/Coding 尤甚，拿旧 key 打新
///   端点只会报一个误导性的 401）> 空
/// - requires_key 且最终 key 为空 → `MissingKey`（调用方短路，不发请求）
pub fn resolve_probe_target(
    info: &ProviderInfo,
    base_url_input: Option<&str>,
    api_key_input: Option<&str>,
    stored: Option<&AgentWithCredentials>,
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
    let same_provider = stored.map(|c| c.agent.provider.as_str()) == Some(info.name.as_str());
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
/// - `base_url` / `api_key`：表单当前值，缺省时回退 agent 存量/注册表默认
/// - `agent_id`：编辑态传入——用存量 key 探测（key 密文永不回显给前端）
#[tauri::command]
pub async fn test_provider_connection(
    cmd: State<'_, std::sync::Arc<dyn AgentCmd>>,
    provider_name: String,
    base_url: Option<String>,
    api_key: Option<String>,
    agent_id: Option<String>,
) -> AppResult<ProviderConnectionResult> {
    let info = list_provider_infos()
        .into_iter()
        .find(|p| p.name == provider_name)
        .ok_or_else(|| AppError::Validation(format!("未知的 Provider: {}", provider_name)))?;

    // 编辑态：取存量 agent（key + 曾用 base_url）
    let stored = match agent_id.as_deref() {
        Some(id) if !id.is_empty() => Some(cmd.inner().get_with_credentials(id).await?),
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
    Ok(ProviderConnectionResult::failed(aggregate_probe_error(
        &candidates,
        &errors,
    )))
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

    /// 生产 `get_with_credentials` 返回的形态：外层 base_url = agent 行优先、
    /// vault 回退后的解析值（`resolve_probe_target` 读外层字段），fixture 同构
    fn stored_creds(provider: &str, api_key: &str, base_url: Option<&str>) -> AgentWithCredentials {
        AgentWithCredentials {
            agent: crate::db::models::AgentRow {
                id: "ag-1".into(),
                name: "n".into(),
                provider: provider.into(),
                model: "m".into(),
                system_prompt: String::new(),
                api_key_ref: "ref".into(),
                base_url: base_url.map(|s| s.into()),
                temperature: 0.7,
                max_tokens: 1024,
                extra_params: String::new(),
                sort_order: 0,
                cache_prompt: 1,
                max_history_messages: None,
                context_window: None,
                enabled_tools: None,
                supports_vision: 0,
                description: String::new(),
                avatar: None,
                workspace_path: None,
                created_at: String::new(),
                updated_at: String::new(),
            },
            api_key: api_key.into(),
            base_url: base_url.map(|s| s.into()),
            hooks: Default::default(),
            word_style_profile: None,
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
}
