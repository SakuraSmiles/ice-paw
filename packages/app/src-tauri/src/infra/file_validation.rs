//! 文件附件校验 — office/pdf 附件的扩展名白名单、大小上限、base64 合法性。
//!
//! 与 [`image_validation`] 对称：在 `send_message` 入口处（materialize 前）调用，
//! 先于任何 DB 写入或 LLM 调用。错误信息直接回前端 toast（`AppError::Validation`）。

use base64::Engine as _;

use crate::error::{AppError, AppResult};
use crate::infra::protocol::AttachedFile;

/// 支持的附件扩展名（与 [`crate::harness::doc`] 支持格式对齐）。
///
/// 注意：不含 `.doc`（老格式，无成熟 Rust 提取 crate）与 `.xlsb`/`.ods`
/// （附件场景罕见，聚焦主流）；`read_file`/KB 走 doc::try_extract 仍覆盖更广。
pub const SUPPORTED_FILE_EXTS: &[&str] = &["docx", "xlsx", "xls", "pdf"];

/// 单个附件最大字节数（base64 解码后的原始字节大小）。
///
/// office 文档（含 ZIP 容器 + 媒体）通常比纯文本大，提到 20MB；
/// 超大文件应让 agent 用 `read_file` 分页读取，而非整份灌进对话。
pub const MAX_FILE_SIZE: usize = 20 * 1024 * 1024; // 20 MiB

/// 单条消息最多附件数。
pub const MAX_FILE_COUNT: usize = 10;

/// 校验扩展名（已小写、去点）是否受支持。
pub fn is_supported_file_ext(ext: &str) -> bool {
    SUPPORTED_FILE_EXTS.contains(&ext)
}

/// 从文件名取扩展名（小写、去点）。无扩展名返回空串。
fn ext_of(name: &str) -> String {
    match name.rsplit('.').next() {
        Some(e) if name.contains('.') => e.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// 校验附件列表（数量 / 扩展名 / base64 合法性 / 尺寸）。
///
/// 在 `send_message` 入口、`materialize_file_blocks` 之前调用。
pub fn validate_files(files: &[AttachedFile]) -> AppResult<()> {
    if files.len() > MAX_FILE_COUNT {
        return Err(AppError::Validation(format!(
            "单条消息最多 {} 个附件，当前 {} 个",
            MAX_FILE_COUNT,
            files.len()
        )));
    }

    for (idx, f) in files.iter().enumerate() {
        // 1. 扩展名白名单
        let ext = ext_of(&f.name);
        if !is_supported_file_ext(&ext) {
            return Err(AppError::Validation(format!(
                "第 {} 个附件格式不支持：{}（允许：{}）",
                idx + 1,
                f.name,
                SUPPORTED_FILE_EXTS.join(" / ")
            )));
        }

        // 2. base64 解码（materialize 还要再解一次，但这里先验合法性 + 尺寸，
        //    避免无效/超大附件走到相对昂贵的 office 提取）
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&f.data)
            .map_err(|e| {
                AppError::Validation(format!("第 {} 个附件 base64 解码失败：{}", idx + 1, e))
            })?;

        // 3. 尺寸
        if decoded.len() > MAX_FILE_SIZE {
            let mb = decoded.len() as f64 / 1024.0 / 1024.0;
            return Err(AppError::Validation(format!(
                "第 {} 个附件过大：{:.2} MB（最大 {} MB）",
                idx + 1,
                mb,
                MAX_FILE_SIZE / 1024 / 1024
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn af(name: &str, data: &str) -> AttachedFile {
        AttachedFile {
            name: name.into(),
            data: data.into(),
        }
    }

    #[test]
    fn ext_of_basic() {
        assert_eq!(ext_of("report.docx"), "docx");
        assert_eq!(ext_of("DATA.XLSX"), "xlsx");
        assert_eq!(ext_of("noext"), "");
        assert_eq!(ext_of(""), "");
        assert_eq!(ext_of("a.b.pdf"), "pdf");
    }

    #[test]
    fn validate_rejects_unsupported_ext() {
        let files = vec![af("notes.txt", "YWJj")];
        let err = validate_files(&files).unwrap_err();
        assert!(err.to_string().contains("不支持"));
    }

    #[test]
    fn validate_rejects_bad_base64() {
        let files = vec![af("ok.docx", "!!!not base64!!!")];
        assert!(validate_files(&files).is_err());
    }

    #[test]
    fn validate_accepts_supported_small() {
        // "abc" 的 base64
        let files = vec![af("ok.pdf", "YWJj")];
        assert!(validate_files(&files).is_ok());
    }

    #[test]
    fn validate_rejects_too_many() {
        let files: Vec<_> = (0..(MAX_FILE_COUNT + 1))
            .map(|i| af(&format!("f{i}.pdf"), "YWJj"))
            .collect();
        let err = validate_files(&files).unwrap_err();
        assert!(err.to_string().contains("最多"));
    }
}
