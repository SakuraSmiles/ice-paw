//! `harness::provider::embedding` — Embedding 向量生成后端（REQ-CHAT-047）
//!
//! ## 设计目标
//!
//! 提供统一的 `EmbeddingBackend` trait 用于把字符串映射为 `Vec<f32>` 向量，
//! 让上层（`MemoryStage` / 记忆检索）可以解耦于具体厂商实现。
//!
//! ## 模块组成
//!
//! - [`EmbeddingBackend`] trait — 异步 `embed()` 方法
//! - [`OpenAiEmbeddingBackend`] — 调用 OpenAI 兼容 `/v1/embeddings` 端点
//!   （OpenAI / GLM / DeepSeek / 自部署 vllm 等都兼容）
//! - [`NoopEmbeddingBackend`] — 测试用 noop（返回全 0 向量，便于离线测试）
//! - [`embedding_url_for`] — URL 智能拼接（与 `provider/openai::build_chat_url` 对齐）
//!
//! ## trait 签名
//!
//! ```ignore
//! async fn embed(&self, texts: Vec<&str>, api_key: &str) -> AppResult<Vec<Vec<f32>>>
//! ```
//!
//! - `texts`  批量文本（一次请求多个，减少 HTTP 开销）
//! - 返回 `Vec<Vec<f32>>` —— 每个文本一条 embedding，**长度可能因 model 而异**
//!   （text-embedding-3-small 默认 1536，可通过 `dimensions` 参数缩小）
//!
//! ## 与 LLM provider 的差异
//!
//! - LLM provider 流式返回；embedding 是单次 RPC 返回
//! - LLM provider 关心 SSE chunk 解析；embedding 关心 JSON 反序列化
//! - LLM provider 用 `LlmProvider` trait；embedding 用 `EmbeddingBackend` trait
//!   （语义不同，避免 trait 名冲突）
//!
//! ## 模型与 URL 推断
//!
//! - OpenAI 兼容厂商（openai / glm / deepseek）→ `POST {base_url}/v1/embeddings`
//! - Anthropic 厂商当前**不支持** embedding（Anthropic 没有 embedding API）
//! - 用户的 agent 配置中有 `embedding_model` 字段（可选，默认 `text-embedding-3-small`）
//! - agent.provider 决定走 OpenAI 兼容还是 Anthropic；当前实现只支持前者
//!   （Anthropic provider 调 embedding 时会返回 `AppError::Validation`，
//!    因为 Anthropic 没有官方 embedding API）

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::{AppError, AppResult};

// =========================================================================
// Trait 定义
// =========================================================================

/// Embedding 向量生成后端 trait（REQ-CHAT-047）
///
/// 把一批文本映射为对应的 `Vec<f32>` 向量。
///
/// # 参数
///
/// - `texts`    批量文本（至少 1 条；空 slice 由调用方拦截，这里不报错）
/// - `api_key`  当前 agent 的 API key（每次调用传入，不在 backend 中持久化）
///
/// # 返回值
///
/// `Vec<Vec<f32>>` —— 长度等于 `texts.len()`，每个内层 Vec 是一条 embedding。
/// 各 embedding 维度**必须一致**（同一 backend 用同一模型时恒成立）。
///
/// # 错误
///
/// - HTTP / 网络错误 → `AppError::Llm`
/// - API Key 无效 → `AppError::ProviderNotConfigured`
/// - 当前 provider 不支持 embedding（如 anthropic）→ `AppError::Validation`
#[async_trait]
pub trait EmbeddingBackend: Send + Sync {
    /// 把 `texts` 批量转成 embedding 向量
    async fn embed(
        &self,
        texts: Vec<&str>,
        api_key: &str,
    ) -> AppResult<Vec<Vec<f32>>>;

    /// 返回 backend 当前使用的 embedding 模型名（调试 / 审计用）
    fn model_name(&self) -> &str;

    /// 返回 backend 当前使用的 base URL（调试 / 审计用）
    fn base_url(&self) -> &str;
}

// =========================================================================
// 默认模型常量
// =========================================================================

/// 默认 embedding 模型：OpenAI text-embedding-3-small（1536 维）
///
/// 当前唯一默认：所有 OpenAI 兼容厂商默认都走这个模型（GLM / DeepSeek 也兼容）。
/// 用户可通过 agent.embedding_model 字段覆盖。
pub const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-3-small";

/// 默认 embedding 维度（用于「维度不匹配」时跳过该记录）
///
/// text-embedding-3-small 默认输出 1536 维，但 OpenAI 允许 `dimensions` 参数
/// 缩小到 256/512/1024/1536。此处固定 1536 作为「期望值」，实际 recall
/// 时按 query 维度动态比对。
pub const DEFAULT_EMBEDDING_DIM: usize = 1536;

/// 余弦相似度阈值（REQ-CHAT-047 明确要求）
///
/// 低于此阈值的记录不进 top-5。
pub const RECALL_SIMILARITY_THRESHOLD: f32 = 0.7;

/// 返回 top-K 数量（REQ-CHAT-047 明确要求）
pub const RECALL_TOP_K: usize = 5;

// =========================================================================
// URL 推断
// =========================================================================

/// 智能拼接 Embedding 端点 URL。
///
/// 与 `provider/openai::build_chat_url` 规则一致：
/// - 若末尾路径段匹配 `/vN`（`v` 后跟 ≥1 位数字），则不再追加 `/v1`
/// - 否则按 OpenAI 标准路径补 `/v1/embeddings`
///
/// # 示例
/// - `https://api.openai.com` → `https://api.openai.com/v1/embeddings`
/// - `https://open.bigmodel.cn/api/paas/v4` → `https://open.bigmodel.cn/api/paas/v4/embeddings`
pub fn embedding_url_for(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    let last_segment = trimmed.rsplit('/').next().unwrap_or("");
    if is_version_segment(last_segment) {
        format!("{}/embeddings", trimmed)
    } else {
        format!("{}/v1/embeddings", trimmed)
    }
}

fn is_version_segment(seg: &str) -> bool {
    if seg.len() < 2 {
        return false;
    }
    let bytes = seg.as_bytes();
    if bytes[0] != b'v' {
        return false;
    }
    seg[1..].chars().all(|c| c.is_ascii_digit())
}

// =========================================================================
// NoopEmbeddingBackend（测试用）
// =========================================================================

/// Noop 实现：永远返回全 0 向量（维度 = DEFAULT_EMBEDDING_DIM）。
///
/// 用途：
/// - 单元测试 / 离线场景
/// - 「embedding 后端未注入」时的兜底（不允许真实 LLM 调用）
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopEmbeddingBackend;

#[async_trait]
impl EmbeddingBackend for NoopEmbeddingBackend {
    async fn embed(
        &self,
        texts: Vec<&str>,
        _api_key: &str,
    ) -> AppResult<Vec<Vec<f32>>> {
        debug!(
            target: "ice_paw.embedding",
            "NoopEmbeddingBackend: 返回 {} 条全 0 向量",
            texts.len()
        );
        Ok(texts
            .iter()
            .map(|_| vec![0.0_f32; DEFAULT_EMBEDDING_DIM])
            .collect())
    }

    fn model_name(&self) -> &str {
        DEFAULT_EMBEDDING_MODEL
    }

    fn base_url(&self) -> &str {
        ""
    }
}

// =========================================================================
// OpenAiEmbeddingBackend（OpenAI 兼容实现）
// =========================================================================

/// OpenAI 兼容 Embedding Backend
///
/// 适用于：OpenAI / GLM (智谱) / DeepSeek / 自部署 vllm + OpenAI 协议等
///
/// ## 请求格式
///
/// ```text
/// POST {base_url}/v1/embeddings
/// Authorization: Bearer {api_key}
/// Content-Type: application/json
///
/// {
///   "input": ["text1", "text2", ...],
///   "model": "text-embedding-3-small"
/// }
/// ```
///
/// ## 响应格式（OpenAI 官方）
///
/// ```text
/// {
///   "object": "list",
///   "data": [
///     { "object": "embedding", "embedding": [0.1, 0.2, ...], "index": 0 },
///     ...
///   ],
///   "model": "text-embedding-3-small",
///   "usage": { "prompt_tokens": 10, "total_tokens": 10 }
/// }
/// ```
///
/// GLM / DeepSeek 等兼容厂商响应格式一致。
pub struct OpenAiEmbeddingBackend {
    /// 模型名称（如 `text-embedding-3-small`）
    model: String,
    /// API base URL（不含 `/v1/embeddings` 后缀）
    base_url: String,
    /// HTTP 客户端（复用连接池，与 `OpenAiAdapter` 共享 builder 模式）
    client: reqwest::Client,
}

impl OpenAiEmbeddingBackend {
    /// 创建 OpenAiEmbeddingBackend
    ///
    /// - `model`    embedding 模型名（默认 `text-embedding-3-small`）
    /// - `base_url` API 根地址（如 `https://api.openai.com`）
    pub fn new(model: String, base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client build");
        Self {
            model,
            base_url,
            client,
        }
    }

    /// 返回 backend 当前使用的模型名
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 返回 backend 当前使用的 base URL
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

// ---- 请求 / 响应结构 ----

#[derive(Debug, Serialize)]
struct EmbeddingRequest<'a> {
    input: Vec<&'a str>,
    model: &'a str,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
    #[allow(dead_code)]
    index: usize,
}

#[async_trait]
impl EmbeddingBackend for OpenAiEmbeddingBackend {
    async fn embed(
        &self,
        texts: Vec<&str>,
        api_key: &str,
    ) -> AppResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            // 空输入 → 返回空结果（不报错）
            return Ok(Vec::new());
        }
        if api_key.trim().is_empty() {
            return Err(AppError::ProviderNotConfigured(self.model.clone()));
        }

        let url = embedding_url_for(&self.base_url);
        let body = EmbeddingRequest {
            input: texts.clone(),
            model: &self.model,
        };

        debug!(
            target: "ice_paw.embedding",
            "OpenAiEmbeddingBackend: POST {} (model={}, n={})",
            url,
            self.model,
            texts.len()
        );

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AppError::Llm(format!(
                    "Embedding HTTP 请求失败 ({}): {}",
                    url, e
                ))
            })?;

        let status = response.status();
        if !status.is_success() {
            let code = status.as_u16();
            let text = response.text().await.unwrap_or_else(|e| {
                tracing::warn!(target: "ice_paw.provider", "读取 embedding error body 失败: {e}");
                String::new()
            });
            // 401/403 → ProviderNotConfigured（API Key 无效）
            if code == 401 || code == 403 {
                return Err(AppError::ProviderNotConfigured(self.model.clone()));
            }
            // 429 → LlmRateLimited（保留重试语义）
            if code == 429 {
                return Err(AppError::LlmRateLimited {
                    message: format!(
                        "Embedding API 返回 429: {}",
                        text.chars().take(500).collect::<String>()
                    ),
                    retry_after_secs: None,
                });
            }
            return Err(AppError::Llm(format!(
                "Embedding API 返回 HTTP {}: {}",
                code,
                text.chars().take(500).collect::<String>()
            )));
        }

        // 解析响应
        let parsed: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| AppError::Llm(format!("Embedding 响应解析失败: {e}")))?;

        // 按 `index` 排序，确保返回顺序与 `texts` 一致
        let mut indexed: Vec<(usize, Vec<f32>)> = parsed
            .data
            .into_iter()
            .map(|d| (d.index, d.embedding))
            .collect();
        indexed.sort_by_key(|(i, _)| *i);

        let embeddings: Vec<Vec<f32>> = indexed.into_iter().map(|(_, v)| v).collect();

        if embeddings.len() != texts.len() {
            return Err(AppError::Llm(format!(
                "Embedding 响应数量不匹配：请求 {} 条，收到 {} 条",
                texts.len(),
                embeddings.len()
            )));
        }

        Ok(embeddings)
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }
}

// =========================================================================
// `TextEmbedder` 适配（为 context 层 `InMemoryBackend` 提供桥接）
// =========================================================================
//
// `TextEmbedder` trait 定义在 `context::memory`（依赖倒置原则）。
// `OpenAiEmbeddingBackend` 同时实现 `EmbeddingBackend` 和 `TextEmbedder`
// 两个 trait：前者用于 harness 内部使用，后者让 context 层能够
// 通过 trait object 注入。
//
// 这样 context 层不需直接导入 harness 类型，保持单向依赖。

/// `OpenAiEmbeddingBackend` 作为 `TextEmbedder` 的适配实现
///
/// 转换逻辑：
/// - `embed()` 直接转发
/// - `dim()` 返回 `DEFAULT_EMBEDDING_DIM`（text-embedding-3-small 默认 1536 维）
#[async_trait]
impl crate::context::memory::TextEmbedder for OpenAiEmbeddingBackend {
    async fn embed(
        &self,
        texts: Vec<&str>,
        api_key: &str,
    ) -> AppResult<Vec<Vec<f32>>> {
        <Self as EmbeddingBackend>::embed(self, texts, api_key).await
    }

    fn dim(&self) -> usize {
        DEFAULT_EMBEDDING_DIM
    }
}

/// `NoopEmbeddingBackend` 也实现 `TextEmbedder`（供测试使用）
#[async_trait]
impl crate::context::memory::TextEmbedder for NoopEmbeddingBackend {
    async fn embed(
        &self,
        texts: Vec<&str>,
        api_key: &str,
    ) -> AppResult<Vec<Vec<f32>>> {
        <Self as EmbeddingBackend>::embed(self, texts, api_key).await
    }

    fn dim(&self) -> usize {
        DEFAULT_EMBEDDING_DIM
    }
}

// =========================================================================
// Recall 工具函数（供上层调用）
// =========================================================================

/// 语义检索：在 `candidates` 中找与 `query` 最相似的 top-K 条
///
/// ## 步骤
///
/// 1. 计算 `query` 与每条候选的 cosine 相似度
/// 2. 过滤掉相似度 < [`RECALL_SIMILARITY_THRESHOLD`] 的
/// 3. 按相似度降序排序
/// 4. 取前 [`RECALL_TOP_K`] 条，返回其 content
///
/// ## 边界情况
///
/// - `candidates` 为空 → 返回 `Vec::new()`
/// - `query` 维度与所有候选维度都不匹配 → 返回 `Vec::new()`（无相关结果）
/// - 全部候选相似度都低于阈值 → 返回 `Vec::new()`
pub fn top_k_recall(
    query: &[f32],
    candidates: &[(String, Vec<f32>)],
) -> Vec<String> {
    if query.is_empty() || candidates.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(f32, &str)> = candidates
        .iter()
        .filter_map(|(content, emb)| {
            // 维度不匹配的 embedding 直接跳过（可能是历史混合模型）
            if emb.len() != query.len() {
                warn!(
                    target: "ice_paw.embedding",
                    "跳过维度不匹配的 embedding: query={} dim, candidate={} dim",
                    query.len(),
                    emb.len()
                );
                return None;
            }
            let sim = crate::db::repo::memory_embedding::cosine_similarity(query, emb);
            if sim >= RECALL_SIMILARITY_THRESHOLD {
                Some((sim, content.as_str()))
            } else {
                None
            }
        })
        .collect();

    // 按相似度降序排列（NaN 兜底为 Equal）
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    scored
        .into_iter()
        .take(RECALL_TOP_K)
        .map(|(_, c)| c.to_string())
        .collect()
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- URL 推断 ----

    #[test]
    fn embedding_url_openai_default() {
        let url = embedding_url_for("https://api.openai.com");
        assert_eq!(url, "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn embedding_url_openai_with_trailing_slash() {
        let url = embedding_url_for("https://api.openai.com/");
        assert_eq!(url, "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn embedding_url_glm_v4() {
        let url = embedding_url_for("https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(
            url,
            "https://open.bigmodel.cn/api/paas/v4/embeddings"
        );
    }

    #[test]
    fn embedding_url_explicit_v1() {
        let url = embedding_url_for("https://api.openai.com/v1");
        assert_eq!(url, "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn embedding_url_deepseek_default() {
        let url = embedding_url_for("https://api.deepseek.com");
        assert_eq!(url, "https://api.deepseek.com/v1/embeddings");
    }

    #[test]
    fn embedding_url_version_word_not_treated_as_version() {
        // "version1" 不算 v+数字 → 追加 /v1
        let url = embedding_url_for("https://x.com/version1");
        assert_eq!(url, "https://x.com/version1/v1/embeddings");
    }

    // ---- NoopEmbeddingBackend ----

    #[tokio::test]
    async fn noop_returns_zero_vectors_with_default_dim() {
        let backend = NoopEmbeddingBackend;
        let texts = vec!["hello", "world", ""];
        let result = backend.embed(texts, "any-key").await.unwrap();
        assert_eq!(result.len(), 3);
        for v in &result {
            assert_eq!(v.len(), DEFAULT_EMBEDDING_DIM);
            assert!(v.iter().all(|&x| x == 0.0));
        }
    }

    #[tokio::test]
    async fn noop_with_empty_texts_returns_empty() {
        let backend = NoopEmbeddingBackend;
        let result = backend.embed(vec![], "any-key").await.unwrap();
        assert!(result.is_empty());
    }

    // ---- top_k_recall ----

    #[test]
    fn top_k_returns_empty_for_empty_candidates() {
        let query = vec![1.0, 0.0];
        let candidates: Vec<(String, Vec<f32>)> = vec![];
        assert!(top_k_recall(&query, &candidates).is_empty());
    }

    #[test]
    fn top_k_returns_empty_for_empty_query() {
        let query: Vec<f32> = vec![];
        let candidates = vec![("x".into(), vec![1.0, 0.0])];
        assert!(top_k_recall(&query, &candidates).is_empty());
    }

    #[test]
    fn top_k_filters_below_threshold() {
        // query = [1.0, 0.0]
        // candidates:
        //   c1 cosine=1.0  → ✓
        //   c2 cosine=0.99 → ✓
        //   c3 cosine≈0.71 → ✓ (边缘，但 ≥ 0.7)
        //   c4 cosine≈0.41 → ✗ (< 0.7)
        //   c5 cosine=0.0  → ✗
        let query = vec![1.0, 0.0];
        let candidates = vec![
            ("c1".into(), vec![1.0, 0.0]),
            ("c2".into(), vec![0.99, 0.01]),
            ("c3".into(), vec![0.5, 0.5]),
            ("c4".into(), vec![0.3, 0.7]),
            ("c5".into(), vec![0.0, 1.0]),
        ];
        let result = top_k_recall(&query, &candidates);
        assert_eq!(result.len(), 3, "c1/c2/c3 应命中，c4/c5 不命中: {result:?}");
        assert!(result.contains(&"c1".to_string()));
        assert!(result.contains(&"c2".to_string()));
        assert!(result.contains(&"c3".to_string()));
    }

    #[test]
    fn top_k_limits_to_top_5() {
        // 10 条候选：c0..c9 相似度 1.0, 0.95, 0.9, 0.85, 0.8, 0.75, 0.7, 0.65, 0.6, 0.55
        // 阈值 0.7 → c0..c6 命中（7 条），top-K=5 → 只返前 5
        let query = vec![1.0, 0.0, 0.0];
        let candidates: Vec<(String, Vec<f32>)> = (0..10)
            .map(|i| {
                // 相似度从 1.0 递减到 0.55（保证前 7 条 ≥ 0.7）
                let sim_target = 1.0 - (i as f32) * 0.05;
                // 构造 (sim, sqrt(1-sim²)) 这样的向量（保证相似度 = sim_target）
                let other = (1.0 - sim_target * sim_target).max(0.0).sqrt();
                let emb = vec![sim_target, other, 0.0];
                (format!("c{i}"), emb)
            })
            .collect();

        let result = top_k_recall(&query, &candidates);
        assert_eq!(result.len(), RECALL_TOP_K, "应限制为 ≤5，实际 {}: {result:?}", result.len());
        // top-5 应按相似度排序：c0, c1, c2, c3, c4
        assert_eq!(result, vec!["c0", "c1", "c2", "c3", "c4"]);
    }

    #[test]
    fn top_k_skips_dimension_mismatch() {
        // query 是 3 维，但 candidates 混合 2 维 / 3 维
        let query = vec![1.0, 0.0, 0.0];
        let candidates = vec![
            ("dim3".into(), vec![1.0, 0.0, 0.0]),       // ✓ cosine=1.0
            ("dim2".into(), vec![1.0, 0.0]),            // ✗ 维度不匹配
            ("dim3_other".into(), vec![0.0, 1.0, 0.0]), // ✓ cosine=0.0 (但会被过滤)
        ];
        let result = top_k_recall(&query, &candidates);
        assert_eq!(result, vec!["dim3".to_string()]);
    }

    #[test]
    fn top_k_returns_empty_when_all_below_threshold() {
        let query = vec![1.0, 0.0];
        let candidates = vec![
            ("low1".into(), vec![0.5, 0.5]),  // cosine ≈ 0.71 → 边缘，可能不在测试期望中
            ("low2".into(), vec![0.3, 0.7]),  // cosine ≈ 0.42 → ✗
            ("low3".into(), vec![0.0, 1.0]),  // cosine = 0.0 → ✗
        ];
        let result = top_k_recall(&query, &candidates);
        // 严格阈值 0.7，0.71 应该 >= 0.7 → 1 条命中
        // 我们用 0.5,0.5 验证精确阈值场景：cosine = 0.5/(sqrt(0.5)*sqrt(0.5)) = 1.0
        // 哦不，重新算：cosine([1,0], [0.5,0.5]) = (0.5+0)/(1*sqrt(0.5)) = 0.5/0.707 ≈ 0.707
        // 0.707 > 0.7 → 应命中
        assert!(
            result.iter().any(|c| c == "low1"),
            "低相似度但 >= 0.7 的应命中: {result:?}"
        );
        assert!(!result.contains(&"low2".to_string()));
        assert!(!result.contains(&"low3".to_string()));
    }

    #[test]
    fn top_k_handles_single_candidate() {
        let query = vec![1.0, 0.0];
        let candidates = vec![("only".into(), vec![1.0, 0.0])];
        assert_eq!(top_k_recall(&query, &candidates), vec!["only".to_string()]);
    }
}