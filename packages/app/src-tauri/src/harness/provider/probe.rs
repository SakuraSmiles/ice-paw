//! `provider::probe` — 连通性探测 / 模型列表拉取
//!
//! 「测试连接」与「拉取模型」共用一次 GET /models 往返（`test_provider_connection`
//! 命令的数据层）。与聊天 Adapter 的关键差异：
//!
//! - **短超时**（connect 5s / 总 15s）：探测要快速反馈，不能套用 Adapter 的
//!   300s 总超时（长 prompt 视觉调用可到分钟级，但那是聊天路径的事）。
//! - **错误即结果**：调用方（provider_cmd）把 `Err` 转成 `ok:false + error`
//!   结构化返回，不向上抛——前端要展示具体失败原因而非通用错误弹窗。
//!
//! OpenAI 与 Anthropic 的 /models 响应外层同构（`{"data":[{"id":...}]}`），
//! 解析共用 `parse_model_ids`。

use std::time::Duration;

use serde_json::Value;

use super::anthropic;
use super::openai;
use super::ProviderProtocol;
use crate::error::{AppError, AppResult};

/// 探测专用 HTTP 客户端：短超时（connect 5s / 总 15s）。
fn probe_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Internal(format!("构建探测 HTTP 客户端失败: {e}")))
}

/// 探测 provider 连通性并拉取模型列表（一次 GET /models）。
///
/// - `base_url`：已是最终生效地址（入参 > 注册表默认值的解析在命令层完成）
/// - `api_key`：可空（ollama / 无鉴权本地服务）；按协议发对应 header，
///   空 key 时 header 照发（服务端忽略空 Bearer / 空 x-api-key 即通过，
///   需鉴权的服务返回 401 → 如实报错）
///
/// 非 2xx → `Llm("HTTP {status}: {body 前 500 字}")`，让用户看到服务端的
/// 真实错误信息（如 key 无效 / 端点不存在）。
pub async fn probe_models(
    protocol: ProviderProtocol,
    base_url: &str,
    api_key: &str,
) -> AppResult<Vec<String>> {
    let client = probe_client()?;
    let request = match protocol {
        ProviderProtocol::OpenAI => client
            .get(openai::build_models_url(base_url))
            .header("Authorization", format!("Bearer {}", api_key)),
        ProviderProtocol::Anthropic => client
            .get(anthropic::build_models_url(base_url))
            .header("x-api-key", api_key)
            .header("anthropic-version", anthropic::ANTHROPIC_VERSION),
    };

    let response = request
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("无法连接到 {}: {e}", base_url)))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(500).collect();
        return Err(AppError::Llm(format!("HTTP {}: {}", status.as_u16(), snippet)));
    }

    let payload: Value = response
        .json()
        .await
        .map_err(|e| AppError::Llm(format!("响应不是有效 JSON: {e}")))?;

    Ok(parse_model_ids(&payload))
}

/// 从 OpenAI/Anthropic 同构的 models 响应 `{"data":[{"id":"..."}]}` 提取模型 id。
///
/// 宽容解析：`data` 缺失/非数组/元素无 id → 返回已收集到的部分（可能为空），
/// 不报错——空列表配合外层「发现 0 个模型」的文案，比抛错更接近真实语义
/// （服务通了，只是列表格式没见过）。
pub fn parse_model_ids(payload: &Value) -> Vec<String> {
    let mut ids: Vec<String> = payload
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    ids.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids_json(ids: &[&str]) -> Value {
        serde_json::json!({ "data": ids.iter().map(|i| serde_json::json!({ "id": i })).collect::<Vec<_>>() })
    }

    #[test]
    fn parse_normal_list_sorted_case_insensitive() {
        let v = serde_json::json!({ "data": [
            { "id": "qwen3:32b" },
            { "id": "DeepSeek-V4" },
            { "id": "llama3" },
        ]});
        assert_eq!(
            parse_model_ids(&v),
            vec!["DeepSeek-V4".to_string(), "llama3".to_string(), "qwen3:32b".to_string()]
        );
    }

    #[test]
    fn parse_empty_data() {
        let v = serde_json::json!({ "data": [] });
        assert_eq!(parse_model_ids(&v), Vec::<String>::new());
    }

    #[test]
    fn parse_dedups_repeated_ids() {
        let v = ids_json(&["m1", "m1", "m2"]);
        assert_eq!(parse_model_ids(&v), vec!["m1".to_string(), "m2".to_string()]);
    }

    #[test]
    fn parse_malformed_payloads_return_empty_not_error() {
        // 缺 data / data 非数组 / 元素缺 id：宽容降级为部分/空列表
        assert_eq!(parse_model_ids(&serde_json::json!({})), Vec::<String>::new());
        assert_eq!(
            parse_model_ids(&serde_json::json!({ "data": "oops" })),
            Vec::<String>::new()
        );
        let v = serde_json::json!({ "data": [{ "name": "no-id-field" }, { "id": "ok" }] });
        assert_eq!(parse_model_ids(&v), vec!["ok".to_string()]);
    }
}
