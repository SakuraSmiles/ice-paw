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
    /// 可重试：网络层错误（连接断开、超时、LLM 侧临时错误、流解析失败）。
    /// 不可重试：参数校验、资源不存在、取消、鉴权失败（401/403）。
    ///
    /// 注意：HTTP 502/503/504 的判断在 `stream_loop` 中根据 HTTP 状态码单独处理，
    /// 不经过 `AppError::is_retryable()`（因为 `AppError::Llm` 包含了 HTTP 状态码文本，
    /// 无法精确区分 401 和 502）。
    pub fn is_retryable(&self) -> bool {
        match self {
            // 网络层 / LLM 侧临时错误 → 可重试
            AppError::Llm(msg) | AppError::Stream(msg) => {
                // 排除鉴权错误（401/403），这些重试无意义
                let s = msg.to_lowercase();
                !s.contains("401") && !s.contains("403")
            }
            AppError::Io(_) => true,
            // 其余一律不重试
            AppError::Validation(_)
            | AppError::NotFound { .. }
            | AppError::Cancelled
            | AppError::Database(_)
            | AppError::Migrate(_)
            | AppError::Stronghold(_)
            | AppError::Json(_)
            | AppError::Tauri(_)
            | AppError::Internal(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

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
}
