//! `harness::provider` — LLM provider adapters + 工厂函数
//!
//! - W2.1：从 `llm/adapters/openai.rs` 迁入 OpenAI 兼容 Adapter
//! - W2.2：从 `llm/adapters/anthropic.rs` 迁入 Anthropic 兼容 Adapter
//! - W2.3：合并 `llm/mod.rs` 中的 `create_provider()` + `default_base_url()` 工厂函数，
//!   删除整个 `llm/` 目录
//!
//! 设计要点：
//! - `LlmProvider` trait 定义统一的流式聊天接口，各厂商 Adapter 实现之
//! - `create_provider` 工厂根据 provider 名称返回对应 Adapter
//!   - OpenAI 兼容：openai / glm / deepseek
//!   - Anthropic 兼容：anthropic / minimax / minimax-cn
//! - API Key 不存储于 Adapter，每次调用时传入，降低泄露风险
//!
//! 详见 Sprint 计划 W2.1–W2.3。

pub mod anthropic;
pub mod embedding;
pub mod mock;
pub mod model_info;
pub mod openai;
pub mod probe;
pub use anthropic::AnthropicAdapter;
pub use mock::{MockProvider, MockScenario};
pub use model_info::{
    default_context_window, default_max_output_tokens, effective_supports_vision,
    model_capabilities, ModelCapabilities,
};
pub use openai::OpenAiAdapter;

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;

use crate::error::AppResult;
use crate::harness::chat_state::CancellationToken;
use crate::infra::protocol::{ChatDelta, ChatMessage, ToolDef};

/// LLM 提供方接口（从 `infra::protocol` 迁入，归属 provider 模块）
///
/// 实现方需提供 `stream_chat`，返回一个异步 Stream 逐块产出 `ChatDelta`。
/// 调用方在消费 Stream 时应定期检查 `cancel.is_cancelled()` 以支持用户停止。
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// 流式聊天
    #[allow(clippy::too_many_arguments)]
    async fn stream_chat(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<ToolDef>>,
        temperature: f64,
        max_tokens: i32,
        model: Option<&str>,
        cancel: CancellationToken,
    ) -> AppResult<Pin<Box<dyn futures::Stream<Item = AppResult<ChatDelta>> + Send>>>;

    /// 摘要专用通道（滚动摘要等内部小额度调用）。
    ///
    /// 根因（2026-08-15 生产诊断）：glm-5.2 等 thinking 模型会把小额度
    /// `max_tokens`（摘要仅 512）全部烧在思考通道（`reasoning_content`），
    /// `content` 恒为空 → 滚动摘要从未成功 → 全量历史每轮重发 → 预算熔断。
    /// 默认实现与 `stream_chat` 完全等价；OpenAI Adapter 覆写之，对支持
    /// 思考开关的 provider（GLM）显式注入 `thinking: {"type":"disabled"}`。
    /// 聊天主路径不走此方法，行为零变化。
    #[allow(clippy::too_many_arguments)]
    async fn stream_summary(
        &self,
        api_key: &str,
        messages: Vec<ChatMessage>,
        temperature: f64,
        max_tokens: i32,
        cancel: CancellationToken,
    ) -> AppResult<Pin<Box<dyn futures::Stream<Item = AppResult<ChatDelta>> + Send>>> {
        self.stream_chat(
            api_key,
            messages,
            None, // 摘要不启用工具
            temperature,
            max_tokens,
            None, // 摘要固定走 Adapter 默认 model
            cancel,
        )
        .await
    }

    /// 返回当前 Provider 实际使用的模型名（用于消息级记录）
    fn model_name(&self) -> &str;
}

// =========================================================================
// 工厂函数（从 llm/mod.rs 迁入）+ Provider 目录（单一真相源）
// =========================================================================

/// Provider 描述符：名称 → 协议类型 + 默认 URL + 目录元数据
///
/// `label`/`note`/`requires_key`/`requires_base_url`/`models` 是给前端
/// Provider 目录用的（`list_providers` 命令下发），与工厂共用同一张表，
/// 前后端零硬编码漂移。
struct ProviderDesc {
    name: &'static str,
    protocol: ProviderProtocol,
    default_url: &'static str,
    /// 备选探测端点（标签, 地址）：未显式填地址时按 [默认, ...备选] 顺序探测，
    /// 走通的地址回传前端存进 agent——智谱标准/Coding 双端点 key 不通用，
    /// 让「测试连接」自动匹配，下拉里只保留一个厂商选项
    alt_urls: &'static [(&'static str, &'static str)],
    /// 展示名（下拉框主行）
    label: &'static str,
    /// 补充说明（下拉框副行，如「本地推理，无需 API Key」）
    note: Option<&'static str>,
    /// 该 provider 是否需要 API Key（ollama 本地服务无需）
    requires_key: bool,
    /// 是否必须显式填写 base_url（custom 无默认地址，必填）
    requires_base_url: bool,
    /// API Key 申请页地址（前端「去申请 ↗」直达；免 key 厂商为 None）。
    /// 单一真相源在此，GeneralSettings 的 embedding keyUrl 映射是本表子集的
    /// 前端旧副本（后续可顺手收敛）。
    key_url: Option<&'static str>,
    /// 隐藏条目：不进前端下拉（UI 已收敛/下线），但注册表仍可解析——
    /// 存量 agent 的创建/探测/徽标显示照常，零破坏
    hidden: bool,
    /// 静态模型目录（起点参考；在线「拉取」按钮拿实时列表，手输永远保留）
    models: &'static [&'static str],
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderProtocol {
    OpenAI,
    Anthropic,
}

/// 数据驱动的 provider 注册表（单一真相源，消除 create_provider 与
/// default_base_url 两份 match 语句的同步风险）。
///
/// 端点地址均已按官方文档核对（2026-08）：DeepSeek `api.deepseek.com` 是 API
/// 域（官网是 www.deepseek.com）；智谱双端点见 `glm` 的 alt_urls；MiniMax
/// 统一国内站（用户拍板：不区分国内/国际）；Ollama 不进下拉（本地服务地址
/// 因人而异，模型名也装不出来——手输模型名 + 填本机 URL 即覆盖，存量
/// `provider="ollama"` 的 agent 仍可解析）。
const PROVIDERS: &[ProviderDesc] = &[
    ProviderDesc {
        name: "openai", protocol: ProviderProtocol::OpenAI, default_url: "https://api.openai.com",
        alt_urls: &[], label: "OpenAI", note: None, requires_key: true, requires_base_url: false,
        key_url: Some("https://platform.openai.com/api-keys"),
        hidden: false,
        models: &["gpt-4o", "gpt-4o-mini", "o3-mini", "gpt-4.1", "gpt-4.1-mini"],
    },
    ProviderDesc {
        name: "glm", protocol: ProviderProtocol::OpenAI, default_url: "https://open.bigmodel.cn/api/paas/v4",
        alt_urls: &[("Coding 端点", "https://open.bigmodel.cn/api/coding/paas/v4")],
        label: "智谱", note: Some("GLM 系列；标准/Coding 端点可切换，Coding 套餐请选 Coding 端点；5.3 系思考常开不可关"),
        requires_key: true, requires_base_url: false,
        key_url: Some("https://open.bigmodel.cn/usercenter/proj-mgmt/apikeys"),
        hidden: false,
        models: &["glm-5.3", "glm-5.3-flash", "glm-5.2", "glm-5.1", "glm-5v-turbo", "glm-5-turbo", "glm-4-flash"],
    },
    ProviderDesc {
        name: "glm-coding", protocol: ProviderProtocol::OpenAI, default_url: "https://open.bigmodel.cn/api/coding/paas/v4",
        alt_urls: &[], label: "智谱 GLM Coding",
        note: Some("旧入口：新配置请选「智谱」，测试连接会自动匹配端点"),
        requires_key: true, requires_base_url: false,
        key_url: Some("https://open.bigmodel.cn/usercenter/proj-mgmt/apikeys"),
        hidden: true,
        models: &["glm-5.3", "glm-5.2", "glm-5.1", "glm-5-turbo"],
    },
    ProviderDesc {
        name: "deepseek", protocol: ProviderProtocol::OpenAI, default_url: "https://api.deepseek.com",
        alt_urls: &[], label: "DeepSeek", note: Some("V4 系 1M 窗口；vision-exp 为视觉实验模型；chat/reasoner 旧名已于 2026-07 弃用"),
        requires_key: true, requires_base_url: false,
        key_url: Some("https://platform.deepseek.com/api_keys"),
        hidden: false,
        models: &["deepseek-v4-pro", "deepseek-v4-flash", "deepseek-v4-flash-vision-exp"],
    },
    ProviderDesc {
        name: "anthropic", protocol: ProviderProtocol::Anthropic, default_url: "https://api.anthropic.com",
        alt_urls: &[], label: "Anthropic", note: None, requires_key: true, requires_base_url: false,
        key_url: Some("https://console.anthropic.com/settings/keys"),
        hidden: false,
        models: &[
            "claude-opus-5",
            "claude-sonnet-5",
            "claude-fable-5",
            "claude-haiku-4-5",
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
            "claude-haiku-3-5-20241022",
        ],
    },
    ProviderDesc {
        name: "minimax", protocol: ProviderProtocol::Anthropic, default_url: "https://api.minimaxi.com/anthropic",
        alt_urls: &[], label: "MiniMax", note: Some("国内站"), requires_key: true, requires_base_url: false,
        key_url: Some("https://platform.minimaxi.com/user-center/basic-information/interface-key"),
        hidden: false,
        models: &["MiniMax-M3", "MiniMax-M2.5", "MiniMax-M2.5-highspeed"],
    },
    ProviderDesc {
        name: "minimax-cn", protocol: ProviderProtocol::Anthropic, default_url: "https://api.minimaxi.com/anthropic",
        alt_urls: &[], label: "MiniMax（国内站·旧）",
        note: Some("旧入口：与 MiniMax 同端点"),
        requires_key: true, requires_base_url: false,
        key_url: Some("https://platform.minimaxi.com/user-center/basic-information/interface-key"),
        hidden: true,
        models: &["MiniMax-M3", "MiniMax-M2.5", "MiniMax-M2.5-highspeed"],
    },
    ProviderDesc {
        name: "ollama", protocol: ProviderProtocol::OpenAI, default_url: "http://localhost:11434/v1",
        alt_urls: &[], label: "Ollama 本地",
        note: Some("已下线：新配置请在模型框手输模型名 + API URL 填本机地址（默认 http://localhost:11434/v1），无需 Key"),
        requires_key: false, requires_base_url: false, key_url: None, hidden: true,
        models: &[],
    },
    ProviderDesc {
        name: "custom", protocol: ProviderProtocol::OpenAI, default_url: "",
        alt_urls: &[], label: "自定义（OpenAI 兼容）",
        note: Some("模型框手输目录外名字即落此处；必填 API URL（Ollama 等本机服务如 http://localhost:11434/v1），无需鉴权可留空 Key"),
        requires_key: false, requires_base_url: true, key_url: None, hidden: true,
        models: &[],
    },
];

fn find_provider(name: &str) -> Option<&'static ProviderDesc> {
    PROVIDERS.iter().find(|p| p.name == name)
}

/// 按 provider 名称创建对应的 LLM Adapter。
///
/// 数据驱动：从 `PROVIDERS` 注册表查找协议类型 + 默认 URL，
/// 消除 create_provider / default_base_url 两份 match 同步风险。
///
/// 未识别的 provider 兜底走 OpenAI 兼容（向后兼容），并打 warn 日志。
pub fn create_provider(
    provider: &str,
    model: &str,
    base_url: Option<&str>,
    cache_prompt: bool,
) -> AppResult<Arc<dyn LlmProvider>> {
    let desc = find_provider(provider);
    let url = match base_url {
        Some(u) if !u.is_empty() => u.to_string(),
        _ => desc.map(|d| d.default_url).unwrap_or("").to_string(),
    };

    let protocol = desc.map(|d| d.protocol);

    tracing::info!(
        target: "ice_paw.llm",
        "创建 Provider: {} | model={} | base_url={} | protocol={}",
        provider,
        model,
        url,
        match protocol {
            Some(ProviderProtocol::OpenAI) => "openai",
            Some(ProviderProtocol::Anthropic) => "anthropic",
            None => "unknown(fallback=openai)",
        },
    );

    match protocol {
        Some(ProviderProtocol::OpenAI) => Ok(Arc::new(OpenAiAdapter::new(
            model.to_string(),
            url,
            provider,
        )?)),
        Some(ProviderProtocol::Anthropic) => Ok(Arc::new(AnthropicAdapter::new(
            model.to_string(),
            url,
            cache_prompt,
        )?)),
        None => {
            tracing::warn!(
                target: "ice_paw.llm",
                "未知 provider '{}'，兜底走 OpenAI 兼容",
                provider
            );
            Ok(Arc::new(OpenAiAdapter::new(
                model.to_string(),
                url,
                provider,
            )?))
        }
    }
}

/// 各 provider 的默认 base_url（委托给 PROVIDERS 注册表）。
/// 仅测试使用；生产代码内联读取 `PROVIDERS`。
#[allow(dead_code)]
fn default_base_url(provider: &str) -> String {
    find_provider(provider)
        .map(|d| d.default_url)
        .unwrap_or("")
        .to_string()
}

// =========================================================================
// Provider 目录下发（list_providers 命令的数据源）
// =========================================================================

/// 前端 Provider 目录条目（`list_providers` 命令的返回类型）。
/// 字段与 `ProviderDesc` 一一对应，serde 走 snake_case 透传（与 events 惯例一致）。
/// `hidden` 条目也会下发（AgentSettings 徽标/编辑态解析要用），前端下拉自行过滤。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub protocol: ProviderProtocol,
    pub default_url: String,
    /// 备选探测端点 [标签, 地址]（serde 元组序列化为数组）
    pub alt_urls: Vec<(String, String)>,
    pub label: String,
    pub note: Option<String>,
    pub requires_key: bool,
    pub requires_base_url: bool,
    pub key_url: Option<String>,
    pub hidden: bool,
    pub models: Vec<String>,
}

/// 全量 Provider 目录（`list_providers` 命令直接返回；含 hidden 条目）。
pub fn list_provider_infos() -> Vec<ProviderInfo> {
    PROVIDERS
        .iter()
        .map(|d| ProviderInfo {
            name: d.name.to_string(),
            protocol: d.protocol,
            default_url: d.default_url.to_string(),
            alt_urls: d
                .alt_urls
                .iter()
                .map(|(l, u)| (l.to_string(), u.to_string()))
                .collect(),
            label: d.label.to_string(),
            note: d.note.map(|s| s.to_string()),
            requires_key: d.requires_key,
            requires_base_url: d.requires_base_url,
            key_url: d.key_url.map(|s| s.to_string()),
            hidden: d.hidden,
            models: d.models.iter().map(|s| s.to_string()).collect(),
        })
        .collect()
}

/// 该 provider 是否必须配 API Key。未知 provider 保守返回 true
/// （按需要 key 的多数路径处理，宁可多要求也不静默发空 key）。
pub fn provider_requires_key(name: &str) -> bool {
    find_provider(name).map(|d| d.requires_key).unwrap_or(true)
}

/// 该 provider 是否必须显式填写 base_url（当前仅 custom）。
pub fn provider_requires_base_url(name: &str) -> bool {
    find_provider(name)
        .map(|d| d.requires_base_url)
        .unwrap_or(false)
}

/// 该 provider 的默认 base_url（未知返回空串——custom 的默认本就是空）。
pub fn provider_default_url(name: &str) -> String {
    find_provider(name)
        .map(|d| d.default_url)
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 工厂应当返回 Ok（不应报错；URL / 模型透传）
    #[test]
    fn factory_returns_ok_for_known_providers() {
        for p in [
            "openai",
            "glm",
            "glm-coding",
            "deepseek",
            "anthropic",
            "minimax",
            "minimax-cn",
            "ollama",
            "custom",
        ] {
            let r = create_provider(p, "model-x", Some("https://x.com"), true);
            assert!(
                r.is_ok(),
                "known provider '{}' 应返回 Ok，实际: {:?}",
                p,
                r.err()
            );
            // 模型名应透传（以 URL 为 url, 验证创建不 panic）
            let _ = r.unwrap();
        }
    }

    /// 未知 provider 应当被接收（兑底走 OpenAI 兼容），不报错（向后兼容）
    #[test]
    fn factory_unknown_provider_falls_back() {
        let r = create_provider("totally-unknown-thing", "m", Some("https://x.com"), false);
        assert!(r.is_ok());
    }

    /// provider 未传 base_url 时会用 provider 默认值
    #[test]
    fn factory_uses_default_url_when_base_url_missing() {
        // glm 的默认 URL 应被使用（不需 Ok 中解析出 URL，但不应报错且不应 panic）
        let r = create_provider("glm", "m", None, false);
        assert!(r.is_ok());
        let r = create_provider("minimax-cn", "m", None, false);
        assert!(r.is_ok());
    }

    /// 默认 URL 表必须准确（端点已按官方文档核对，2026-08）：
    /// MiniMax 统一国内站（api.minimaxi.com，多一个 i——国际站 .io 已下线）
    #[test]
    fn default_base_urls() {
        assert_eq!(default_base_url("anthropic"), "https://api.anthropic.com");
        assert_eq!(
            default_base_url("minimax"),
            "https://api.minimaxi.com/anthropic"
        );
        assert_eq!(
            default_base_url("minimax-cn"),
            "https://api.minimaxi.com/anthropic"
        );
        // GLM 双端点：标准 paas 与 Coding Plan 订阅端点必须可区分（key 不通用）
        assert_eq!(
            default_base_url("glm"),
            "https://open.bigmodel.cn/api/paas/v4"
        );
        assert_eq!(
            default_base_url("glm-coding"),
            "https://open.bigmodel.cn/api/coding/paas/v4"
        );
        // 回归：原有三个不变（api.deepseek.com 是 API 域，官网是 www.）
        assert_eq!(default_base_url("openai"), "https://api.openai.com");
        assert_eq!(default_base_url("deepseek"), "https://api.deepseek.com");
        // ollama 默认指向本地服务；custom 无默认地址
        assert_eq!(default_base_url("ollama"), "http://localhost:11434/v1");
        assert_eq!(default_base_url("custom"), "");
        // 兑底返回空串
        assert_eq!(default_base_url(""), "");
        assert_eq!(default_base_url("totally-unknown"), "");
    }

    /// 可见目录（前端下拉数据源）：hidden 条目（旧入口/已下线）不进下拉，
    /// 但仍在注册表内可解析（存量 agent 零破坏）
    #[test]
    fn visible_catalog_excludes_hidden() {
        let infos = list_provider_infos();
        let visible: Vec<&str> = infos
            .iter()
            .filter(|i| !i.hidden)
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(
            visible,
            vec!["openai", "glm", "deepseek", "anthropic", "minimax"]
        );
        // hidden 条目仍可解析（工厂/目录元数据照常）
        for legacy in ["glm-coding", "minimax-cn", "ollama", "custom"] {
            assert!(find_provider(legacy).is_some(), "{} 应保留在注册表", legacy);
            assert!(find_provider(legacy).unwrap().hidden);
        }
    }

    /// 智谱双端点：可见条目 glm 携带 Coding 备选端点（探测回退用）；
    /// 其余 provider 无备选（单端点直测）
    #[test]
    fn glm_carries_coding_alt_url() {
        let infos = list_provider_infos();
        let glm = infos.iter().find(|i| i.name == "glm").unwrap();
        assert_eq!(
            glm.alt_urls,
            vec![(
                "Coding 端点".to_string(),
                "https://open.bigmodel.cn/api/coding/paas/v4".to_string()
            )]
        );
        for i in &infos {
            if i.name != "glm" {
                assert!(i.alt_urls.is_empty(), "{} 不应有备选端点", i.name);
            }
        }
    }

    /// 目录质量：label 全部唯一且非空（前端下拉显示名，重复即无法反查）
    #[test]
    fn provider_labels_unique_and_nonempty() {
        let infos = list_provider_infos();
        let mut labels: Vec<&str> = infos.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.iter().all(|l| !l.trim().is_empty()));
        labels.sort();
        let n = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), n, "label 不应有重复");
    }

    /// requires_key 标志：ollama/custom 免 key，其余全要；未知名保守返回 true
    #[test]
    fn provider_requires_key_flags() {
        assert!(!provider_requires_key("ollama"));
        assert!(!provider_requires_key("custom"));
        for p in [
            "openai",
            "glm",
            "glm-coding",
            "deepseek",
            "anthropic",
            "minimax",
            "minimax-cn",
        ] {
            assert!(provider_requires_key(p), "{p} 应要求 key");
        }
        assert!(provider_requires_key("totally-unknown"));
    }

    /// requires_base_url：仅 custom（无默认地址，必须显式填）
    #[test]
    fn provider_requires_base_url_flags() {
        let infos = list_provider_infos();
        let required: Vec<&str> = infos
            .iter()
            .filter(|i| i.requires_base_url)
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(required, vec!["custom"]);
    }

    /// 目录下发与注册表逐字段一致（names 对齐 + 元数据透传）
    #[test]
    fn list_provider_infos_matches_registry() {
        let infos = list_provider_infos();
        assert_eq!(infos.len(), PROVIDERS.len());
        for (info, desc) in infos.iter().zip(PROVIDERS.iter()) {
            assert_eq!(info.name, desc.name);
            assert_eq!(info.protocol, desc.protocol);
            assert_eq!(info.default_url, desc.default_url);
            assert_eq!(
                info.alt_urls,
                desc.alt_urls
                    .iter()
                    .map(|(l, u)| (l.to_string(), u.to_string()))
                    .collect::<Vec<_>>()
            );
            assert_eq!(info.label, desc.label);
            assert_eq!(info.note.as_deref(), desc.note);
            assert_eq!(info.requires_key, desc.requires_key);
            assert_eq!(info.requires_base_url, desc.requires_base_url);
            assert_eq!(info.hidden, desc.hidden);
            assert_eq!(
                info.models,
                desc.models
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            );
        }
    }
}
