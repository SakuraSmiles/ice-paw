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
use crate::harness::provider::{
    list_provider_infos, probe, ProviderInfo, ProviderProtocol,
};

use super::agent_cmd::AgentCmd;

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

    // base_url 解析：入参 > agent 存量 > 注册表默认（custom 默认为空 = 必须显式填）
    let base_url = base_url
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            stored
                .as_ref()
                .and_then(|c| c.base_url.clone())
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| info.default_url.clone());
    if base_url.is_empty() {
        return Err(AppError::Validation(
            "自定义 Provider 必须填写 API URL（如 http://localhost:8000/v1）".into(),
        ));
    }

    // key 解析：入参 > agent 存量（免 key 的 provider 走空串探测，header 照发）
    let api_key = api_key
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| stored.as_ref().map(|c| c.api_key.clone()))
        .unwrap_or_default();

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
        Err(e) => Ok(ProviderConnectionResult {
            ok: false,
            model_count: 0,
            models: Vec::new(),
            error: Some(e.to_string()),
        }),
    }
}
