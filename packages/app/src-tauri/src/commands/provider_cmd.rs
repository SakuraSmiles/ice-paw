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
pub fn list_providers() -> AppResult<Vec<ProviderInfo>> {
    Ok(list_provider_infos())
}

/// 连通性测试结果。**探测失败不是命令失败**：`ok:false + error` 结构化返回，
/// 前端行内展示具体原因（HTTP 状态/网络错误），不弹通用错误框。
#[derive(Debug, serde::Serialize)]
pub struct ProviderConnectionResult {
    pub ok: bool,
    pub model_count: usize,
    pub models: Vec<String>,
    pub error: Option<String>,
}

impl ProviderConnectionResult {
    fn failed(error: String) -> Self {
        Self { ok: false, model_count: 0, models: Vec::new(), error: Some(error) }
    }
}

/// 探测目标解析结果（纯函数产物，见 `resolve_probe_target`）
#[derive(Debug, PartialEq, Eq)]
pub enum ProbeTarget {
    Ready { base_url: String, api_key: String },
    /// 需鉴权的 provider 但没有任何可用 key——不发注定 401 的请求，
    /// 直接给「先选内置目录 / 填 Key 再拉」的引导（模型浏览与拉取分离）
    MissingKey,
}

/// 解析探测目标（纯函数，可单测）。规则：
///
/// - base_url：表单入参 > agent 存量 > 注册表默认；custom 等必填项为空 → Validation
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
    // base_url 解析：入参 > agent 存量 > 注册表默认（custom 默认为空 = 必须显式填）
    let base_url = base_url_input
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            stored
                .and_then(|c| c.base_url.clone())
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| info.default_url.clone());
    if base_url.is_empty() {
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
    Ok(ProbeTarget::Ready { base_url, api_key })
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
    let (base_url, api_key) = match target {
        ProbeTarget::Ready { base_url, api_key } => (base_url, api_key),
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

    tracing::info!(
        target: "ice_paw.llm",
        "探测 Provider: {} | base_url={} | has_key={} | protocol={:?}",
        provider_name,
        base_url,
        !api_key.is_empty(),
        info.protocol,
    );

    match probe::probe_models(info.protocol, &base_url, &api_key).await {
        Ok(models) => {
            let count = models.len();
            Ok(ProviderConnectionResult {
                ok: true,
                model_count: count,
                models,
                error: None,
            })
        }
        Err(e) => Ok(ProviderConnectionResult::failed(e.to_string())),
    }
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
                tool_trim_threshold: None,
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
            base_url: None,
            hooks: Default::default(),
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
                base_url: "https://x/v1".into(),
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
            ProbeTarget::Ready { base_url: info("glm").default_url.clone(), api_key: "glm-key".into() }
        );
        // 跨 provider：glm 的存量 key 不拿去打 deepseek（各家 key 多不通用）
        let cross = stored_creds("glm", "glm-key", None);
        assert_eq!(
            resolve_probe_target(&info("deepseek"), None, None, Some(&cross)).unwrap(),
            ProbeTarget::MissingKey
        );
    }

    #[test]
    fn keyless_provider_probes_with_empty_key() {
        // ollama：无 key 照常探测（本地服务忽略空 Bearer）
        let i = info("ollama");
        assert_eq!(
            resolve_probe_target(&i, None, None, None).unwrap(),
            ProbeTarget::Ready {
                base_url: i.default_url.clone(),
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
}
