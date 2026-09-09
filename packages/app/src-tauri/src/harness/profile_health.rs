//! 模型配置（ModelProfile）健康状态监控 —— 分类单一真相源 + 记录入口。
//!
//! ## 语义：按「最后一次真实调用」的结果给 profile 定状态
//!
//! 用户需求原话：「根据最后一次调用它的情况结果来表现它的状态」——不是周期巡检，
//! 是真实调用轨迹的沉淀。五个记录点：
//! ① 视觉代读循环（`modal::ocr_image`，逐凭据成败各记各的）
//! ② KB 入库预生成 / 检索懒生成（`kb::indexer` / `mcp::kb_tool`）
//! ③④⑤ 三个手动测试命令（`test_provider_connection` profile 腿 /
//! `test_model_profile_vision` / `test_embedding_config` profile 腿——测的就是存量
//! 配置，结果即状态）
//!
//! 全 NULL = 从未调用（前端「未调用」态，**不冒充正常**）。
//!
//! ## 分类
//!
//! [`crate::error::classify_llm_error`]（`LlmErrorKind`）是既有单一真相源，本模块
//! 做面向**配置健康**的收窄映射（如 InsufficientBalance 与 GlmResourcePack 合并
//! 为 Quota——对「这条配置还能不能用」二者同义：要充值/换端点）；404 模型不存在
//! 是 LlmErrorKind 未覆盖的配置级错误，在此补扫。Sensitive/ContextTooLong 说明端点
//! 与鉴权都通但本次调用未成功——归 Unknown + 原文，不冒充 Ok。

use sqlx::SqlitePool;

use crate::db::repo;
use crate::harness::error_mapping::{classify_llm_error, LlmErrorKind};

/// 配置健康分类（DB 存 `as_str` 的 slug，前端按 slug 取文案与色档）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileHealth {
    /// 最后一次调用成功——配置可用
    Ok,
    /// 额度耗尽（余额不足 / 智谱资源包 1113）——确定性，需充值或换端点
    Quota,
    /// Key 无效 / 权限不足（401/403）——确定性，需换 Key
    Auth,
    /// 限流中（429）——瞬时，稍后自愈
    RateLimited,
    /// 端点不可达（超时 / 连接失败 / 5xx）——瞬时或地址错
    Network,
    /// 模型不存在（404 / no such model）——确定性，模型名或套餐覆盖问题
    ModelNotFound,
    /// 其他失败（原文在 last_health_detail，hover 可诊断）
    Unknown,
}

impl ProfileHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Quota => "quota",
            Self::Auth => "auth",
            Self::RateLimited => "rate_limited",
            Self::Network => "network",
            Self::ModelNotFound => "model_not_found",
            Self::Unknown => "unknown",
        }
    }
}

/// 错误文本 → 配置健康分类（`classify_llm_error` 的收窄映射 + 404 补扫）。
pub fn health_from_error(msg: &str) -> ProfileHealth {
    match classify_llm_error(msg) {
        LlmErrorKind::InsufficientBalance | LlmErrorKind::GlmResourcePack => ProfileHealth::Quota,
        LlmErrorKind::Auth | LlmErrorKind::Forbidden => ProfileHealth::Auth,
        LlmErrorKind::RateLimited => ProfileHealth::RateLimited,
        LlmErrorKind::Network => ProfileHealth::Network,
        // Sensitive/ContextTooLong：调用链路通但本次未成功 → Unknown（不冒充 Ok）；
        // Unknown 时补扫 404（LlmErrorKind 无此档，模型名错是配置级高频问题）
        LlmErrorKind::Sensitive | LlmErrorKind::ContextTooLong | LlmErrorKind::Unknown => {
            if is_model_not_found(msg) {
                ProfileHealth::ModelNotFound
            } else {
                ProfileHealth::Unknown
            }
        }
    }
}

/// 404 / 模型不存在判据（小写扫描；不收裸「不存在」防误吞无关文案）
fn is_model_not_found(msg: &str) -> bool {
    let s = msg.to_lowercase();
    s.contains("404")
        || s.contains("model not found")
        || s.contains("model_not_found")
        || s.contains("no such model")
        || s.contains("does not exist")
        || s.contains("模型不存在")
}

/// 失败原文入库前的截断上限（hover 诊断够用，避免长响应撑库）
const DETAIL_MAX_CHARS: usize = 300;

/// detail 入库截断（char 边界安全）——`record` 与命令层 `ModelProfileCmd::record_health`
/// 共用，保证两条记录通道形状一致。
pub fn clip_detail(msg: &str) -> String {
    msg.chars().take(DETAIL_MAX_CHARS).collect()
}

/// 记录一次调用结果（warn-only：状态是旁路数据，记录失败绝不影响主流程）。
///
/// `pool`/`profile_id` 任一 None 直接返回——调用方（纯函数测试 / 旧格式凭据链）
/// 无需重复判空。成功调用 detail=None；失败调用 detail=分类前原文（截断）。
pub async fn record(
    pool: Option<&SqlitePool>,
    profile_id: Option<&str>,
    health: ProfileHealth,
    detail: Option<&str>,
) {
    let (Some(pool), Some(pid)) = (pool, profile_id) else {
        return;
    };
    let clipped = detail.map(clip_detail);
    if let Err(e) =
        repo::model_profile::record_health(pool, pid, health.as_str(), clipped.as_deref()).await
    {
        tracing::warn!(target: "ice_paw.profile_health", "记录模型配置 {pid} 健康状态失败: {e}");
    }
}

/// 记录一次成功调用（detail 恒 None 的薄包装）
pub async fn record_ok(pool: Option<&SqlitePool>, profile_id: Option<&str>) {
    record(pool, profile_id, ProfileHealth::Ok, None).await;
}

/// 记录一次失败调用（原文分类 + 原文入 detail）
pub async fn record_error(pool: Option<&SqlitePool>, profile_id: Option<&str>, msg: &str) {
    record(pool, profile_id, health_from_error(msg), Some(msg)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_llm_kinds_to_config_health() {
        // 余额族合并为 Quota（对配置可用性同义）
        assert_eq!(health_from_error("余额不足，请充值"), ProfileHealth::Quota);
        assert_eq!(
            health_from_error("code:1113 无可用资源包"),
            ProfileHealth::Quota
        );
        assert_eq!(
            health_from_error("insufficient_quota"),
            ProfileHealth::Quota
        );
        // 鉴权族 → Auth
        assert_eq!(
            health_from_error("HTTP 401: invalid api key"),
            ProfileHealth::Auth
        );
        assert_eq!(
            health_from_error("403 permission denied"),
            ProfileHealth::Auth
        );
        // 限流 / 网络
        assert_eq!(
            health_from_error("HTTP 429: too many requests"),
            ProfileHealth::RateLimited
        );
        assert_eq!(
            health_from_error("vision 请求失败 (glm): timeout"),
            ProfileHealth::Network
        );
        assert_eq!(health_from_error("502 bad gateway"), ProfileHealth::Network);
    }

    #[test]
    fn scans_404_model_not_found_beyond_llm_kind() {
        // LlmErrorKind 无 404 档（会落 Unknown）——配置级补扫
        assert_eq!(
            health_from_error("Embedding API 返回 HTTP 404: no such model"),
            ProfileHealth::ModelNotFound
        );
        assert_eq!(
            health_from_error("vision glm 返回 404 Not Found: Model Not Found"),
            ProfileHealth::ModelNotFound
        );
        assert_eq!(
            health_from_error("模型不存在或无权限"),
            ProfileHealth::ModelNotFound
        );
        // 裸「不存在」不收（防误吞无关文案）
        assert_eq!(
            health_from_error("引用的模型配置不存在"),
            ProfileHealth::Unknown
        );
    }

    #[test]
    fn sensitive_and_context_are_unknown_not_ok() {
        // 端点与鉴权都通但本次调用未成功——诚实归 Unknown，不冒充 Ok
        assert_eq!(
            health_from_error("图片内容未通过安全审核"),
            ProfileHealth::Unknown
        );
        assert_eq!(
            health_from_error("context_length_exceeded"),
            ProfileHealth::Unknown
        );
        assert_eq!(health_from_error("随便什么别的错"), ProfileHealth::Unknown);
    }

    #[test]
    fn slugs_stable_for_frontend() {
        // 前端 HEALTH_META 按 slug 匹配——slug 是序列化契约，改名=前端断档
        assert_eq!(ProfileHealth::Ok.as_str(), "ok");
        assert_eq!(ProfileHealth::Quota.as_str(), "quota");
        assert_eq!(ProfileHealth::Auth.as_str(), "auth");
        assert_eq!(ProfileHealth::RateLimited.as_str(), "rate_limited");
        assert_eq!(ProfileHealth::Network.as_str(), "network");
        assert_eq!(ProfileHealth::ModelNotFound.as_str(), "model_not_found");
        assert_eq!(ProfileHealth::Unknown.as_str(), "unknown");
    }

    #[tokio::test]
    async fn record_none_args_are_noop() {
        // pool=None / profile_id=None 直接返回（纯函数路径与旧格式链零记录）
        record(None, Some("mp1"), ProfileHealth::Ok, None).await;
        record(None, None, ProfileHealth::Ok, None).await;
        record_ok(None, None).await;
        record_error(None, None, "x").await;
    }
}
