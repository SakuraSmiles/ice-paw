//! REQ-AGENT-029 / REQ-AGENT-030: Provider / Model 动态加载与级联命令。
//!
//! 设计要点：
//! - `list_providers()`  —— 列出全部可用 Provider（id + 展示名）。数据来源
//!   为 `harness::provider::list_providers()`，对应前端下拉。
//! - `list_models(provider_id)` —— 列出某 Provider 下的可选模型。前端在
//!   用户切换 Provider 后调用，重置模型选择。
//!
//! 两个命令均为「纯元信息查询」，不涉及 API Key / 网络 / 数据库，可直接同步返回。
//! 命名空间：commands::provider_cmd。

use crate::error::AppResult;
use crate::harness::provider::{self, ModelInfo, ProviderInfo};

/// REQ-AGENT-029: 列出全部可用 Provider。
///
/// 数据来源：编译期 `harness::provider::PROVIDER_INFO`（按 KNOWN_PROVIDERS
/// 顺序排列）。返回空数组时前端展示「暂无可用 Provider」。
///
/// 对应前端 `bridge.providers.list()`。
#[tauri::command]
pub async fn list_providers() -> AppResult<Vec<ProviderInfo>> {
    Ok(provider::list_providers())
}

/// REQ-AGENT-030: 列出某 Provider 的可选模型列表。
///
/// - `provider_id`：Provider id（与 `list_providers()` 返回的 `id` 字段对齐）
/// - 未知 provider 返回空数组（前端应回退为「自定义模型」输入态）
///
/// 对应前端 `bridge.providers.listModels(providerId)`。
#[tauri::command]
pub async fn list_models(provider_id: String) -> AppResult<Vec<ModelInfo>> {
    Ok(provider::list_provider_models(&provider_id))
}
