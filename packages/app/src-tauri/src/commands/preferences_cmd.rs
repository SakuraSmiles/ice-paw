//! 用户偏好设置相关 Tauri Commands
//!
//! - `get_preferences`     读取全部偏好
//! - `set_preference`      更新单个偏好（key-value）
//! - `test_vision_config`  视觉读取条目健康检查（设置页逐条「测试」按钮）

use sqlx::SqlitePool;
use tauri::State;

use crate::db::models::{UserPreferences, VisionConfigEntry};
use crate::db::repo;
use crate::error::AppResult;

/// 读取全部用户偏好设置
#[tauri::command]
pub async fn get_preferences(pool: State<'_, SqlitePool>) -> AppResult<UserPreferences> {
    repo::preferences::get_all(pool.inner()).await
}

/// 更新单个偏好项
///
/// `value` 接收字符串，前端传 JSON.stringify 后的字符串。
#[tauri::command]
pub async fn set_preference(
    pool: State<'_, SqlitePool>,
    key: String,
    value: String,
) -> AppResult<()> {
    repo::preferences::set(pool.inner(), &key, &value).await
}

/// 1×1 PNG（测试探针图）——视觉链路里最小合法图片字节。
///
/// 健康检查只验证「端点可达 + key 有效 + 模型收图不报错」，不验证识别质量；
/// 用常量字节而非运行时生成，保证每次测试发的内容完全一致（可对比、可重放）。
const TINY_PNG_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// `test_vision_config` 的返回——前端展示延迟与模型回声（证明端到端通）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct VisionTestResult {
    /// 端到端耗时（毫秒）
    pub latency_ms: u64,
    /// 模型对探针图的回复节选（前 120 字符）——「真能读图」的可信证据
    pub sample: String,
}

/// 视觉读取条目健康检查：用**传入的条目**（非已存配置，测的是未保存的新值）
/// 代读一张 1×1 探针图，验证 provider/model/key/base_url 有效。失败返回 Err
///（describe 的错误文案自带三段式上下文）。
///
/// 与 `test_embedding_config` 对称；端点推导与正式代读同源（`entry_to_credential`），
/// 测试通过 = 正式链路同参数可用（同一鉴权域、同一 JSON 形状、同一思考策略）。
#[tauri::command]
pub async fn test_vision_config(
    provider: String,
    model: String,
    api_key: String,
    base_url: Option<String>,
) -> AppResult<VisionTestResult> {
    use crate::harness::vision;

    let entry = VisionConfigEntry {
        provider,
        model,
        api_key,
        base_url,
    };
    let cred = vision::entry_to_credential(&entry, 1).ok_or_else(|| {
        crate::error::AppError::Validation(format!(
            "视觉配置条目无效：provider 未知或 provider/model/key 为空（{} / {}）",
            entry.provider,
            entry.model
        ))
    })?;

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(TINY_PNG_B64)
        .map_err(|e| crate::error::AppError::Internal(format!("探针图 base64 损坏: {e}")))?;

    let started = std::time::Instant::now();
    let sample = vision::describe_image(
        &cred.provider,
        &cred.model,
        &cred.base_url,
        &cred.api_key,
        "image/png",
        &bytes,
    )
    .await?;
    Ok(VisionTestResult {
        latency_ms: started.elapsed().as_millis() as u64,
        sample: sample.chars().take(120).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 探针图常量必须是合法 PNG（开头 8 字节 PNG magic）——常量手抄防 typo 的护栏。
    #[test]
    fn tiny_png_probe_is_valid_png() {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(TINY_PNG_B64)
            .expect("探针图 base64 应可解码");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "开头 8 字节须为 PNG magic");
        // 尾部 IEND chunk 存在（完整性粗验）
        assert_eq!(&bytes[bytes.len() - 8..], b"\x49END\xae\x42\x60\x82");
    }
}
