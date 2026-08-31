//! 软件级共享模板目录（word-capability-roadmap D17）：安装包内置模板资产
//! boot 时落盘到 `<app_data_dir>/templates/`，全 agent 共享、用户可直接在
//! 资源管理器里改样式或塞自己的模板。
//!
//! - **幂等且不覆盖**：文件已存在（含用户改过的）一律不动；删除则下次启动重建。
//!   同款语义见 [kb::ensure] 的 help 种子。
//! - **解析链**（[mcp::docx_tool] 的 `resolve_template`）：相对模板名依次查
//!   ① workspace `templates/`（更具体者优先）② 本共享目录 ③ 内置档位名兜底；
//!   同名文件可覆盖内置档位。
//!
//! 纯文件操作无 AppHandle 依赖（目录由调用方传入，lib.rs 用 [crate::logging::data_dir]
//! 推导），测试注入临时目录即可。

use std::path::Path;

use crate::error::{AppError, AppResult};

/// 随安装包分发的共享模板种子表：文件名 → 资产字节。
///
/// formal-report：正式报告模板（标题 1-4 级/正文/表格/列表样式、密级页眉 +
/// 编号占位行、页码页脚，A4 单节空壳）。源自真机 Word 产物的手术净化品，
/// 业务词表终验 0 命中（D7 净化豁免经用户拍板 2026-08-31）。
pub const SHARED_TEMPLATE_SEEDS: &[(&str, &[u8])] =
    &[("formal-report.docx", include_bytes!("assets/formal-report.docx"))];

/// 把种子模板落盘到共享目录（幂等）：目录缺失创建；文件缺失写入、存在不动。
/// 返回本次实际写入的份数（0 = 全部已存在，boot 日志静默）。
pub fn ensure_shared_templates(dir: &Path) -> AppResult<usize> {
    if dir.exists() {
        if !dir.is_dir() {
            return Err(AppError::Io(std::io::Error::other(format!(
                "共享模板路径被文件占用: {}",
                dir.display()
            ))));
        }
    } else {
        std::fs::create_dir_all(dir).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "创建共享模板目录失败 {}: {e}",
                dir.display()
            )))
        })?;
    }
    let mut written = 0usize;
    for (name, bytes) in SHARED_TEMPLATE_SEEDS {
        let path = dir.join(name);
        if path.exists() {
            continue;
        }
        std::fs::write(&path, bytes).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "写入共享模板失败 {}: {e}",
                path.display()
            )))
        })?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("icepaw_shared_tpl_{tag}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn seeds_are_docx_named_and_zip_shaped() {
        assert!(!SHARED_TEMPLATE_SEEDS.is_empty());
        for (name, bytes) in SHARED_TEMPLATE_SEEDS {
            assert!(name.to_ascii_lowercase().ends_with(".docx"), "{name}");
            // zip 本地文件头魔数（PK\x03\x04）——资产损坏在此拦截，不必等引擎
            assert_eq!(&bytes[..4], b"PK\x03\x04", "{name} 非 zip 形态");
        }
    }

    /// 资产健康度闸：每份种子必须能过生成引擎全链（清空→锚→写块→自检）。
    /// 模板资产被误改（如手工编辑损坏）时 CI 在此红，而非真机 Word 打不开。
    #[test]
    fn seeds_digest_through_generation_engine() {
        for (name, bytes) in SHARED_TEMPLATE_SEEDS {
            let blocks = vec![super::super::WriteBlock::Heading {
                level: 1,
                text: "样式自检".into(),
            }];
            match super::super::generate_from_template(bytes, &blocks) {
                Ok(doc) => assert_eq!(doc.paragraphs, 1, "{name}"),
                Err(e) => panic!("{name} 未过生成引擎: {e}"),
            }
        }
    }

    #[test]
    fn ensure_writes_missing_then_idempotent() {
        let dir = temp_dir("ensure");
        assert_eq!(ensure_shared_templates(&dir).unwrap(), SHARED_TEMPLATE_SEEDS.len());
        assert_eq!(ensure_shared_templates(&dir).unwrap(), 0, "二次运行应零写入");
        for (name, bytes) in SHARED_TEMPLATE_SEEDS {
            assert_eq!(&std::fs::read(dir.join(name)).unwrap()[..], *bytes);
        }
    }

    #[test]
    fn ensure_never_overwrites_user_edits_but_recreates_deleted() {
        let dir = temp_dir("preserve");
        ensure_shared_templates(&dir).unwrap();
        let first = dir.join(SHARED_TEMPLATE_SEEDS[0].0);
        std::fs::write(&first, b"user-modified").unwrap();
        assert_eq!(ensure_shared_templates(&dir).unwrap(), 0);
        assert_eq!(
            std::fs::read(&first).unwrap(),
            b"user-modified",
            "用户改动不得被覆盖"
        );
        std::fs::remove_file(&first).unwrap();
        ensure_shared_templates(&dir).unwrap();
        assert_eq!(
            std::fs::read(&first).unwrap(),
            SHARED_TEMPLATE_SEEDS[0].1,
            "删除后应重建"
        );
    }
}
