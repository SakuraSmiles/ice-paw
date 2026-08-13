//! 应用统一错误类型 `AppError`
//!
//! 设计原则：
//! - 所有 `#[tauri::command]` 函数返回 `Result<T, AppError>`，由 `AppError` 自动转 `InvokeError`
//! - 数据库层（sqlx）、加密层（stronghold）、业务校验错误统一收口
//! - 实现 `Display` + `Error`（通过 `thiserror`），便于日志和前端展示

use serde::Serialize;
#[allow(unused_imports)]
use tauri::ipc::InvokeError;

/// 应用统一错误枚举
///
/// - `Database`    —— sqlx 数据访问层错误
/// - `Stronghold`  —— stronghold vault 读写错误
/// - `NotFound`    —— 资源不存在（如 agent_id、conversation_id）
/// - `Validation`  —— 入参校验失败（前端可读，业务级）
/// - `Json`        —— 内部序列化/反序列化错误
/// - `Io`          —— 文件 IO（snapshot 落盘等）
/// - `Tauri`       —— 框架错误（路径解析、状态管理等）
/// - `Internal`    —— 兜底：未分类的内部错误
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("数据库错误: {0}")]
    Database(#[from] sqlx::Error),

    #[error("数据库迁移错误: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("Stronghold 错误: {0}")]
    Stronghold(String),

    #[error("资源未找到: {resource}={id}")]
    NotFound { resource: &'static str, id: String },

    #[error("参数校验失败: {0}")]
    Validation(String),

    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("Tauri 错误: {0}")]
    Tauri(String),

    #[error("内部错误: {0}")]
    Internal(String),

    #[error("LLM 调用错误: {0}")]
    Llm(String),

    #[error("流式解析错误: {0}")]
    Stream(String),

    #[error("操作已取消")]
    Cancelled,

    #[error("授权失败: 工具 '{tool}' — {reason}")]
    AuthorizationRequired { tool: String, reason: String },
}

/// 把 stronghold 错误统一转字符串，简化上层处理
impl From<tauri_plugin_stronghold::stronghold::Error> for AppError {
    fn from(e: tauri_plugin_stronghold::stronghold::Error) -> Self {
        AppError::Stronghold(e.to_string())
    }
}

/// 把 `anyhow`-like 的 `String` 错误快捷封装
impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        AppError::Internal(s.to_string())
    }
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::Internal(s)
    }
}

/// 把 `tauri::Error`（路径解析等）也归并进来
impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        AppError::Tauri(e.to_string())
    }
}

/// 让错误可以序列化穿过 IPC 边界送达前端
/// 前端会看到 `{ kind, message }` 而不是裸字符串，便于业务层做提示
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let kind = match self {
            AppError::Database(_) => "database",
            AppError::Migrate(_) => "migrate",
            AppError::Stronghold(_) => "stronghold",
            AppError::NotFound { .. } => "not_found",
            AppError::Validation(_) => "validation",
            AppError::Json(_) => "json",
            AppError::Io(_) => "io",
            AppError::Tauri(_) => "tauri",
            AppError::Internal(_) => "internal",
            AppError::Llm(_) => "llm",
            AppError::Stream(_) => "stream",
            AppError::Cancelled => "cancelled",
            AppError::AuthorizationRequired { .. } => "authorization_required",
        };
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("kind", kind)?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

/// Tauri `command` 属性宏要求错误类型实现 `Into<InvokeError>`。
///
/// 我们依赖 `tauri::ipc::InvokeError: From<T: Serialize>` 的 blanket impl，
/// 直接通过我们手写的 `Serialize for AppError` 把错误结构化地传到前端。
///
/// 便捷别名
pub type AppResult<T> = std::result::Result<T, AppError>;

// =========================================================================
// 可重试错误分类
// =========================================================================

impl AppError {
    /// 判断错误是否值得重试。
    ///
    /// 可重试：网络层错误（连接断开、超时、LLM 侧临时错误、流解析失败、限流）。
    /// 不可重试：参数校验、资源不存在、取消、鉴权失败（401/403）、内容审核拒、上下文超长。
    ///
    /// `Llm` / `Stream` 的判定委托 [`classify_llm_error`]——单一真相源，与
    /// [`crate::harness::error_mapping::friendly_error`] 共享同一分类，避免两处
    /// 各自 substring 匹配同一错误文本而漂移（如旧实现把 400 内容审核误判为可重试）。
    pub fn is_retryable(&self) -> bool {
        match self {
            // LLM / 流错误：按语义分类决定重试（审核/鉴权/超长不重试；限流/网络/未知重试）
            AppError::Llm(msg) | AppError::Stream(msg) => classify_llm_error(msg).is_retryable(),
            AppError::Io(_) => true,
            // 其余一律不重试
            AppError::Validation(_)
            | AppError::NotFound { .. }
            | AppError::Cancelled
            | AppError::AuthorizationRequired { .. }
            | AppError::Database(_)
            | AppError::Migrate(_)
            | AppError::Stronghold(_)
            | AppError::Json(_)
            | AppError::Tauri(_)
            | AppError::Internal(_) => false,
        }
    }
}

// ===========================================================================
// LLM / 视觉代读错误语义分类（单一真相源）
// ---------------------------------------------------------------------------
// `AppError::is_retryable` 与 `harness::error_mapping::friendly_error` 都消费
// 本分类，避免两处各自 substring 匹配同一个错误文本而漂移。新增 provider 措辞
// 只需在 [`classify_llm_error`] 加关键词，重试策略与友好文案自动跟进。
// ===========================================================================

/// LLM / 视觉代读错误的语义分类。
///
/// 由 [`classify_llm_error`] 从错误文本推断；覆盖主流 LLM provider
/// （Anthropic / OpenAI / GLM / DeepSeek / MiniMax）与内部视觉代读路径
/// （`harness::vision::describe_image`）的所有已知措辞。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmErrorKind {
    /// 内容审核拒绝（图片/文本违规、敏感内容）。确定性错误，重试无意义。
    Sensitive,
    /// 限流（HTTP 429 too many requests）。瞬时错误，重试可恢复。
    RateLimited,
    /// 余额 / 配额不足（GLM code:1113「余额不足或无可用资源包,请充值」、OpenAI
    /// `insufficient_quota` 等）。确定性错误（需充值 / 换包），区别于瞬时限流——
    /// GLM 把这类账户问题**也以 HTTP 429 返回**，[`classify_llm_error`] 须先于 429 命中。
    InsufficientBalance,
    /// 鉴权失败（401，密钥无效/过期）。重试无意义。
    Auth,
    /// 权限不足（403）。重试无意义。
    Forbidden,
    /// 上下文超长（token 超窗口）。重试无意义。
    ContextTooLong,
    /// 网络/服务端瞬时错误（超时、连接断开、5xx）。重试可恢复。
    Network,
    /// 未识别。保守按可重试处理（兼容旧实现除 401/403 外皆重试的行为）；
    /// 友好提示回落原文，便于诊断。
    Unknown,
}

impl LlmErrorKind {
    /// 用户可读中文提示。[`LlmErrorKind::Unknown`] 返回空串，由
    /// [`crate::harness::error_mapping::friendly_error`] 回落原文。
    pub fn friendly_text(self) -> &'static str {
        match self {
            Self::Sensitive => "图片内容未通过安全审核，请更换图片后重试",
            Self::RateLimited => "请求过于频繁，请稍后再试",
            Self::InsufficientBalance => "API 余额或配额不足，请充值或更换套餐后重试",
            Self::Auth => "API 密钥无效或已过期，请在设置中检查",
            Self::Forbidden => "API 权限不足，请检查配置",
            Self::ContextTooLong => "消息过长，请缩短内容或清除部分历史消息",
            Self::Network => "网络或服务暂时不可用，请检查网络后重试",
            Self::Unknown => "",
        }
    }

    /// 是否值得重试。确定性错误（审核/鉴权/超长）不重试；瞬时错误（限流/网络）重试；
    /// `Unknown` 保守重试以兼容旧行为。
    pub fn is_retryable(self) -> bool {
        match self {
            Self::RateLimited | Self::Network | Self::Unknown => true,
            Self::Sensitive
            | Self::Auth
            | Self::Forbidden
            | Self::ContextTooLong
            | Self::InsufficientBalance => false,
        }
    }

    /// 多个视觉凭据全失败时，选**最具行动价值**的错误上报（[`crate::harness::modal`]
    /// 代读循环 + [`crate::harness::mcp::attachment_image_tool`] Arch B fallback 共用）。
    ///
    /// **规则**：`Sensitive` 是关于**输入**（图片本身违规）的判定，确定性高于任何
    /// **凭据级**错误（限流 / 余额 / 鉴权 / 网络）——只要任一凭据给出 Sensitive，
    /// 它就是这张图读不出的真正原因，应优先上报（让用户「换图」而非误以为「稍后重试」）。
    /// 其余情况保留**首个**（首选凭据最相关），不被后续瞬态错误覆盖。
    ///
    /// 触发面：实测一张敏感图，首选凭据（MiniMax）正确返回 Sensitive、次选（GLM）因
    /// 余额不足返回 429；旧实现 `last_kind = Some(kind)` 只留最后一个 → 把 Sensitive
    /// 丢成 RateLimited「请求过于频繁」，掩盖真正原因。
    pub fn prefer(prev: Option<LlmErrorKind>, new: LlmErrorKind) -> Option<LlmErrorKind> {
        match (prev, new) {
            (None, n) => Some(n),
            // Sensitive（输入判定）优先：已有或新来都保留 Sensitive
            (Some(LlmErrorKind::Sensitive), _) | (_, LlmErrorKind::Sensitive) => {
                Some(LlmErrorKind::Sensitive)
            }
            // 其余保留首个（首选凭据最相关）
            (Some(p), _) => Some(p),
        }
    }
}

/// 从错误文本推断 [`LlmErrorKind`]（分类单一真相源）。
///
/// 大小写不敏感扫描关键词。**顺序敏感**：`Sensitive` 最先匹配——GLM 的敏感拒可能
/// 伴随 HTTP 500（"image is sensitive"），须先于 `Network` 的 5xx 命中，否则会被误判
/// 为可重试的网络错误。
pub fn classify_llm_error(msg: &str) -> LlmErrorKind {
    let s = msg.to_lowercase();

    // 内容审核 / 敏感。覆盖：GLM(sensitive/new_sensitive)、OpenAI(content_filter/
    // content_policy_violation/content filter)、Anthropic(content policy/safety)、
    // 通用(moderation/inappropriate/nsfw)、中文(审核/违规/敏感)。
    if s.contains("sensitive")
        || s.contains("content_filter")
        || s.contains("content filter")
        || s.contains("content_policy")
        || s.contains("content policy")
        || s.contains("policy_violation")
        || s.contains("moderation")
        || s.contains("safety")
        || s.contains("inappropriate")
        || s.contains("nsfw")
        || s.contains("审核")
        || s.contains("违规")
        || s.contains("敏感")
    {
        return LlmErrorKind::Sensitive;
    }
    // 余额 / 配额不足。**须先于 429**——GLM 把 code:1113「余额不足或无可用资源包,请充值」
    // 也以 HTTP 429 返回，但它是确定性账户问题（需充值 / 换包），非瞬时限流；先命中此处
    // 才不会被下面的 429 → RateLimited 误判成「请求过于频繁」（误导用户等待重试）。
    if s.contains("余额不足")
        || s.contains("无可用资源包")
        || s.contains("充值")
        || s.contains("insufficient balance")
        || s.contains("insufficient_quota")
        || s.contains("insufficient quota")
        || s.contains("out of quota")
    {
        return LlmErrorKind::InsufficientBalance;
    }
    // 限流（429）
    if s.contains("429")
        || s.contains("rate limit")
        || s.contains("rate_limit")
        || s.contains("too many requests")
        || s.contains("过于频繁")
        || s.contains("限流")
    {
        return LlmErrorKind::RateLimited;
    }
    // 鉴权失败（401）
    if s.contains("401")
        || s.contains("unauthorized")
        || s.contains("invalid api key")
        || s.contains("api key expired")
        || s.contains("密钥无效")
        || s.contains("密钥已过期")
    {
        return LlmErrorKind::Auth;
    }
    // 权限不足（403）
    if s.contains("403")
        || s.contains("forbidden")
        || s.contains("permission_denied")
        || s.contains("permission denied")
        || s.contains("权限不足")
    {
        return LlmErrorKind::Forbidden;
    }
    // 上下文超长
    if s.contains("context_length")
        || s.contains("context length")
        || s.contains("context_window")
        || s.contains("maximum context")
        || s.contains("too many tokens")
        || s.contains("token limit")
        || s.contains("tokens") && (s.contains("exceed") || s.contains("limit"))
        || s.contains("消息过长")
        || s.contains("超出上下文")
    {
        return LlmErrorKind::ContextTooLong;
    }
    // 网络 / 服务端瞬时（超时、连接、5xx、vision 请求失败/响应读取失败）
    if s.contains("timeout")
        || s.contains("超时")
        || s.contains("connection")
        || s.contains("连接")
        || s.contains("network")
        || s.contains("dns")
        || s.contains("请求失败")
        || s.contains("响应读取失败")
        || s.contains("502")
        || s.contains("503")
        || s.contains("504")
        || s.contains("bad gateway")
        || s.contains("service unavailable")
        || s.contains("server error")
    {
        return LlmErrorKind::Network;
    }

    LlmErrorKind::Unknown
}

#[cfg(test)]
mod tests {
    use super::{classify_llm_error, AppError, LlmErrorKind};

    #[test]
    fn retryable_llm_generic() {
        assert!(AppError::Llm("HTTP 502: bad gateway".into()).is_retryable());
        assert!(AppError::Llm("HTTP 503: service unavailable".into()).is_retryable());
        assert!(AppError::Llm("连接超时".into()).is_retryable());
    }

    #[test]
    fn not_retryable_llm_auth() {
        assert!(!AppError::Llm("HTTP 401: invalid api key".into()).is_retryable());
        assert!(!AppError::Llm("HTTP 403: forbidden".into()).is_retryable());
    }

    #[test]
    fn retryable_stream_error() {
        assert!(AppError::Stream("连接断开".into()).is_retryable());
    }

    #[test]
    fn retryable_io_error() {
        assert!(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset",
        ))
        .is_retryable());
    }

    #[test]
    fn not_retryable_others() {
        assert!(!AppError::Validation("bad input".into()).is_retryable());
        assert!(!AppError::Cancelled.is_retryable());
        assert!(!AppError::Internal("oops".into()).is_retryable());
    }

    // ---- classify_llm_error：各 provider 敏感拒措辞 ----

    #[test]
    fn classify_sensitive_all_providers() {
        // GLM（实测措辞）
        assert_eq!(
            classify_llm_error("HTTP 500: input new_sensitive, image is sensitive"),
            LlmErrorKind::Sensitive
        );
        // OpenAI
        assert_eq!(
            classify_llm_error("HTTP 400: content_filter triggered"),
            LlmErrorKind::Sensitive
        );
        assert_eq!(
            classify_llm_error("HTTP 400: content_policy_violation"),
            LlmErrorKind::Sensitive
        );
        assert_eq!(
            classify_llm_error("API returned 400: content filter"),
            LlmErrorKind::Sensitive
        );
        // Anthropic
        assert_eq!(
            classify_llm_error("HTTP 400: invalid_request_error: content policy violation"),
            LlmErrorKind::Sensitive
        );
        assert_eq!(
            classify_llm_error("HTTP 400: safety violation"),
            LlmErrorKind::Sensitive
        );
        // 中文
        assert_eq!(
            classify_llm_error("图片内容违规，未通过审核"),
            LlmErrorKind::Sensitive
        );
    }

    #[test]
    fn classify_rate_auth_forbidden() {
        assert_eq!(classify_llm_error("HTTP 429: rate_limit_exceeded"), LlmErrorKind::RateLimited);
        assert_eq!(classify_llm_error("Too Many Requests: rate limit reached"), LlmErrorKind::RateLimited);
        assert_eq!(classify_llm_error("HTTP 401: invalid api key"), LlmErrorKind::Auth);
        assert_eq!(classify_llm_error("HTTP 403: forbidden"), LlmErrorKind::Forbidden);
        assert_eq!(classify_llm_error("permission_denied: no access"), LlmErrorKind::Forbidden);
    }

    #[test]
    fn classify_context_too_long() {
        assert_eq!(classify_llm_error("context_length_exceeded: max 8192 tokens"), LlmErrorKind::ContextTooLong);
        assert_eq!(classify_llm_error("Too many tokens in prompt"), LlmErrorKind::ContextTooLong);
    }

    #[test]
    fn classify_network_variants() {
        assert_eq!(classify_llm_error("HTTP 502: bad gateway"), LlmErrorKind::Network);
        assert_eq!(classify_llm_error("HTTP 503: service unavailable"), LlmErrorKind::Network);
        assert_eq!(classify_llm_error("vision 请求失败 (glm): connection timeout"), LlmErrorKind::Network);
        assert_eq!(classify_llm_error("vision 响应读取失败: timeout"), LlmErrorKind::Network);
    }

    #[test]
    fn classify_unknown_passthrough() {
        assert_eq!(classify_llm_error("一些无法识别的内部错误 xyz123"), LlmErrorKind::Unknown);
        assert_eq!(classify_llm_error(""), LlmErrorKind::Unknown);
    }

    /// 关键顺序不变式：GLM 敏感拒常伴随 500，必须先命中 Sensitive（不可重试），
    /// 而非被 Network（500 → 可重试）吞掉——否则同一张敏感图被无意义重试。
    #[test]
    fn sensitive_takes_precedence_over_network_500() {
        let kind = classify_llm_error("HTTP 500 Internal Server Error: image is sensitive");
        assert_eq!(kind, LlmErrorKind::Sensitive);
        assert!(!kind.is_retryable(), "敏感拒不可重试");
    }

    #[test]
    fn kind_is_retryable_mapping() {
        assert!(!LlmErrorKind::Sensitive.is_retryable());
        assert!(!LlmErrorKind::Auth.is_retryable());
        assert!(!LlmErrorKind::Forbidden.is_retryable());
        assert!(!LlmErrorKind::ContextTooLong.is_retryable());
        assert!(LlmErrorKind::RateLimited.is_retryable());
        assert!(LlmErrorKind::Network.is_retryable());
        assert!(LlmErrorKind::Unknown.is_retryable());
        assert!(
            !LlmErrorKind::InsufficientBalance.is_retryable(),
            "余额不足不可重试（需充值，瞬态重试无意义）"
        );
    }

    #[test]
    fn friendly_text_non_empty_except_unknown() {
        for kind in [
            LlmErrorKind::Sensitive,
            LlmErrorKind::RateLimited,
            LlmErrorKind::Auth,
            LlmErrorKind::Forbidden,
            LlmErrorKind::ContextTooLong,
            LlmErrorKind::Network,
            LlmErrorKind::InsufficientBalance,
        ] {
            assert!(!kind.friendly_text().is_empty(), "{kind:?} 应有友好文案");
        }
        assert!(LlmErrorKind::Unknown.friendly_text().is_empty());
    }

    // ---- 余额 / 配额不足：须先于 429 命中（GLM 把余额不足也以 429 返回）----

    #[test]
    fn classify_insufficient_balance_beats_429() {
        // 实测 GLM 措辞：HTTP 429 + code:1113 + 余额不足
        assert_eq!(
            classify_llm_error("HTTP 429 Too Many Requests: {\"error\":{\"code\":\"1113\",\
                                \"message\":\"余额不足或无可用资源包,请充值。\"}}"),
            LlmErrorKind::InsufficientBalance
        );
        // OpenAI 措辞
        assert_eq!(
            classify_llm_error("HTTP 429: insufficient_quota"),
            LlmErrorKind::InsufficientBalance
        );
        assert_eq!(
            classify_llm_error("You exceeded your current quota, out of quota"),
            LlmErrorKind::InsufficientBalance
        );
        // 友好文案是充值提示，不是「请求过于频繁」
        assert!(
            LlmErrorKind::InsufficientBalance.friendly_text().contains("余额"),
            "余额不足应有充值文案"
        );
    }

    #[test]
    fn classify_pure_rate_limit_still_rate_limited() {
        // 纯限流（无余额关键词）仍归 RateLimited，不被余额分支吞掉
        assert_eq!(
            classify_llm_error("HTTP 429: rate_limit_exceeded, retry after 30s"),
            LlmErrorKind::RateLimited
        );
        assert_eq!(
            classify_llm_error("HTTP 429 Too Many Requests"),
            LlmErrorKind::RateLimited
        );
    }

    // ---- prefer()：Sensitive（输入判定）优先于凭据级瞬态错误 ----

    #[test]
    fn prefer_sensitive_wins_over_transient() {
        // 实测场景：MiniMax 先返回 Sensitive，GLM 后返回余额不足 → 应保留 Sensitive
        let mut best = None;
        best = LlmErrorKind::prefer(best, LlmErrorKind::Sensitive);
        best = LlmErrorKind::prefer(best, LlmErrorKind::InsufficientBalance);
        assert_eq!(best, Some(LlmErrorKind::Sensitive));
        // 反序：瞬态先、Sensitive 后 → 也应升为 Sensitive
        let mut best = None;
        best = LlmErrorKind::prefer(best, LlmErrorKind::Network);
        best = LlmErrorKind::prefer(best, LlmErrorKind::Sensitive);
        assert_eq!(best, Some(LlmErrorKind::Sensitive));
    }

    #[test]
    fn prefer_keeps_first_among_transient() {
        // 无 Sensitive 时保留首个（首选凭据最相关），不被后续瞬态错误覆盖
        let mut best = None;
        best = LlmErrorKind::prefer(best, LlmErrorKind::Network);
        best = LlmErrorKind::prefer(best, LlmErrorKind::RateLimited);
        assert_eq!(best, Some(LlmErrorKind::Network));
    }

    #[test]
    fn prefer_none_passthrough() {
        assert_eq!(LlmErrorKind::prefer(None, LlmErrorKind::Auth), Some(LlmErrorKind::Auth));
    }
}
