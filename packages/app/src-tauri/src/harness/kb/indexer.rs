//! KB 目录索引 —— 扫描 → 解析 → 增量 upsert `kb_document`（RAG v1 摄入管道第 2 环）
//!
//! 增量策略（两层预筛，省 IO）：
//! 1. 递归扫描 KB `directory` 下所有可索引文件（`.md` / `.docx` / `.xlsx` / `.xls`
//!    / `.xlsb` / `.ods` / `.pdf`），`file_path` 以相对路径存储
//! 2. **mtime 预筛**：与已索引记录的 `file_mtime` 相同 → 跳过（不读内容、不解析）
//! 3. mtime 变化 → 读内容算 `content_hash` → 与已索引 hash 相同 → 跳过（touch 场景）
//! 4. hash 变化 → 解析 + `upsert_document`
//! 5. 已索引但磁盘不存在 → `delete_document`（源文件被删/移动）
//!
//! 文件读取用 `tokio::fs`、解析段（docx zip / pdf 渲染）包 `spawn_blocking`：
//! 解析是同步 CPU+IO 重活，直跑 async worker 会卡同线程的其他任务（2026-09-04
//! 质检 Q6；索引低频但单文件可达百 ms 级，watcher 触发路径与主循环共线程池）。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use sqlx::SqlitePool;

use crate::db::models::KbDocumentRow;
use crate::db::repo;
use crate::db::repo::kb::ChunkWithEmbedding;
use crate::error::{AppError, AppResult};
use crate::harness::kb::embedding::{ensure_chunks_embedded, resolve_embedding_config};
use crate::harness::provider::embedding::OpenAiEmbeddingBackend;

use super::parser::{content_hash, first_paragraph, parse_markdown, split_into_chunks, ParsedDoc};

/// 解析段 spawn_blocking 的回传形态：成功（office 或 md 兜底两路都产出
/// `(full_text, ParsedDoc)`）或失败（office 解析 Err——warn + 跳过该文件）。
/// content 已 move 进闭包，md 兜底转换只能在闭包内做，故用枚举而非裸 Result 嵌套。
enum ExtractOutcome {
    Text((String, ParsedDoc)),
    Failed(AppError),
}

/// 单次索引的统计（日志 / 返回调用方观察）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct IndexStats {
    /// 新建或更新的文档数。
    pub indexed: usize,
    /// 因 mtime/hash 未变而跳过的文档数。
    pub skipped: usize,
    /// 因源文件消失而删除的文档数。
    pub deleted: usize,
}

/// 对一个 KB 目录做全量增量索引。
///
/// - `kb_id`：目标 KB 的 id（写入 `kb_document.kb_id`）
/// - `directory`：KB 根目录绝对路径；`file_path` 以相对它的路径存储（正斜杠分隔）
///
/// 幂等：可安全重复调用（watcher 触发 / 启动全量扫描都走这里）。
/// 单个文件读失败/解析失败仅记 warn 并跳过，不中断整体索引。
pub async fn index_directory(
    pool: &SqlitePool,
    kb_id: &str,
    directory: &Path,
) -> AppResult<IndexStats> {
    let mut stats = IndexStats::default();

    // 1. 扫描磁盘：相对路径 → 绝对路径（.md / office / pdf）
    let disk_files = scan_indexable_files(directory);

    // 2. 已索引记录：file_path → KbDocumentRow
    let existing: HashMap<String, KbDocumentRow> = repo::kb::list_documents(pool, kb_id)
        .await?
        .into_iter()
        .map(|d| (d.file_path.clone(), d))
        .collect();

    let mut seen: HashSet<String> = HashSet::new();

    // embedding 预生成配置：循环外读一次 preferences + 构造一次 backend。
    // 未配置 → None（跳过预生成，search_kb 时懒生成兜底）。
    let backend_and_key = repo::preferences::get_all(pool)
        .await
        .ok()
        .and_then(|p| resolve_embedding_config(&p))
        .and_then(|(m, u, k)| OpenAiEmbeddingBackend::new(m, u).ok().map(|be| (be, k)));

    for (rel_path, abs_path) in &disk_files {
        seen.insert(rel_path.clone());

        let mtime = file_mtime(abs_path);
        let prev = existing.get(rel_path);

        // 2a. mtime 预筛：与上次索引相同且已有 hash → 文件未变，跳过
        if let Some(prev) = prev {
            if prev.file_mtime.as_deref() == mtime.as_deref() && prev.content_hash.is_some() {
                stats.skipped += 1;
                continue;
            }
        }

        // 3. 读内容（tokio::fs：索引跑在 async 语境，watcher/重建复用同函数）
        let content = match tokio::fs::read(abs_path).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    target: "ice_paw.kb",
                    "索引跳过：读取失败 kb={} path={} err={}",
                    kb_id, rel_path, e
                );
                continue;
            }
        };

        // 3a. hash 预筛：mtime 变了但内容没变（touch）→ 跳过
        //     （代价：下次仍会因 mtime 不同再读一次；touch 场景罕见，可接受）
        let hash = content_hash(&content);
        if let Some(prev) = prev {
            if prev.content_hash.as_deref() == Some(hash.as_str()) {
                stats.skipped += 1;
                continue;
            }
        }

        // 4. 解析：office/pdf 走 doc::try_extract（docx zip / pdf 渲染是 CPU+IO 同步
        //    重活），markdown/文本走 parse_markdown。整段 spawn_blocking 离开 async
        //    worker（Q6）；md 兜底分支一并入内——content 已 move，lossy 转换留在外
        //    面会拿不到字节。office 解析失败 → warn + 跳过（不退回乱码）。
        let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let ext_owned = ext.to_string();
        let outcome = match tokio::task::spawn_blocking(move || {
            match crate::harness::doc::try_extract(&content, &ext_owned) {
                Ok(Some(d)) => {
                    let p = ParsedDoc {
                        title: d.title.unwrap_or_default(),
                        summary: first_paragraph(&d.text),
                        tags: "[]".into(),
                    };
                    ExtractOutcome::Text((d.text, p))
                }
                Ok(None) => {
                    // md/文本兜底：无 office/pdf 识别结果 → 按 markdown 解析
                    let s = String::from_utf8_lossy(&content).into_owned();
                    let p = parse_markdown(&s);
                    ExtractOutcome::Text((s, p))
                }
                Err(e) => ExtractOutcome::Failed(e),
            }
        })
        .await
        {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(
                    target: "ice_paw.kb",
                    "索引跳过：文档解析任务失败 kb={} path={} err={}",
                    kb_id, rel_path, e
                );
                continue;
            }
        };
        let (full_text, parsed) = match outcome {
            ExtractOutcome::Text(tp) => tp,
            ExtractOutcome::Failed(e) => {
                tracing::warn!(
                    target: "ice_paw.kb",
                    "索引跳过：office 文档解析失败 kb={} path={} err={}",
                    kb_id, rel_path, e
                );
                continue;
            }
        };
        let title = if parsed.title.is_empty() {
            title_from_filename(rel_path)
        } else {
            parsed.title
        };

        let doc_id = repo::kb::upsert_document(
            pool,
            kb_id,
            rel_path,
            &title,
            &parsed.summary,
            &parsed.tags,
            Some(&hash),
            mtime.as_deref(),
        )
        .await?;

        // RAG v2: 切分 chunk + 增量存储（office 的 full_text 已在解析阶段确定）
        let chunks = split_into_chunks(&full_text);
        match repo::kb::upsert_chunks_incremental(pool, &doc_id, &chunks).await {
            Ok(need) => {
                // 入库同步预生成 embedding（未配置则跳过；失败 warn，search 时懒生成兜底）
                if let Some((backend, key)) = &backend_and_key {
                    if !need.is_empty() {
                        let mut to_embed: Vec<ChunkWithEmbedding> = need
                            .iter()
                            .map(|(id, content)| ChunkWithEmbedding {
                                id: id.clone(),
                                doc_id: doc_id.clone(),
                                kb_id: kb_id.to_string(),
                                title: title.clone(),
                                file_path: rel_path.to_string(),
                                summary: parsed.summary.clone(),
                                content: content.clone(),
                                embedding: None,
                            })
                            .collect();
                        match ensure_chunks_embedded(pool, &mut to_embed, backend, key).await {
                            Ok(n) => tracing::info!(
                                target: "ice_paw.kb",
                                "为 {n} 个 chunk 预生成了 embedding doc={}", doc_id
                            ),
                            Err(e) => tracing::warn!(
                                target: "ice_paw.kb",
                                "预生成 embedding 失败 doc={} err={}（搜索时将懒生成兜底）", doc_id, e
                            ),
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(target: "ice_paw.kb", "chunk 存储失败 doc={} err={}", doc_id, e)
            }
        }

        stats.indexed += 1;
    }

    // 5. 删除孤儿：已索引但本次扫描未见到（源文件被删/移走）
    for rel_path in existing.keys() {
        if !seen.contains(rel_path) {
            if let Err(e) = repo::kb::delete_document(pool, kb_id, rel_path).await {
                tracing::warn!(
                    target: "ice_paw.kb",
                    "删除孤儿文档失败 kb={} path={} err={}",
                    kb_id, rel_path, e
                );
            } else {
                stats.deleted += 1;
            }
        }
    }

    tracing::info!(
        target: "ice_paw.kb",
        "KB 索引完成 kb={} indexed={} skipped={} deleted={}",
        kb_id, stats.indexed, stats.skipped, stats.deleted
    );
    Ok(stats)
}

// =========================================================================
// 内部辅助
// =========================================================================

/// 递归扫描 `directory` 下所有**可索引**文件（`.md` / `.docx` / `.xlsx` / `.xls` /
/// `.xlsb` / `.ods` / `.pdf`），返回 `(相对路径, 绝对路径)` 列表。
///
/// 相对路径统一用正斜杠（跨平台一致，便于 DB 存储与检索展示）。
/// 跳过隐藏目录（如 `.git` / `.icepaw`）。
fn scan_indexable_files(directory: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    walk_dir(directory, directory, &mut out);
    out
}

/// 判断扩展名（不含点）是否可索引。与 [`doc::try_extract`] 支持的格式一致。
fn is_indexable_ext(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    matches!(
        e.as_str(),
        "md" | "docx" | "xlsx" | "xls" | "xlsb" | "ods" | "pdf"
    )
}

fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            // 跳过隐藏目录（`.git` 等）
            let is_hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with('.'))
                .unwrap_or(false);
            if is_hidden {
                continue;
            }
            walk_dir(root, &path, out);
        } else if ft.is_file() {
            let is_indexable = path
                .extension()
                .and_then(|e| e.to_str())
                .map(is_indexable_ext)
                .unwrap_or(false);
            if is_indexable {
                if let Ok(rel) = path.strip_prefix(root) {
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    out.push((rel_str, path));
                }
            }
        }
    }
}

/// 文件 mtime → epoch 秒字符串（存 `kb_document.file_mtime`）。
fn file_mtime(path: &Path) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let secs = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(secs.to_string())
}

/// 无显式标题时，用文件名（去掉可索引扩展名）作为 title 兜底。
///
/// 兼容所有可索引格式（.md / .docx / .xlsx / .pdf …）——取 `file_stem` 去掉最后一段扩展名。
fn title_from_filename(rel_path: &str) -> String {
    let stem = rel_path.rsplit(['/', '\\']).next().unwrap_or(rel_path);
    Path::new(stem)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| stem.to_string())
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_from_filename_strips_md() {
        assert_eq!(title_from_filename("notes/a.md"), "a");
    }

    #[test]
    fn title_from_filename_strips_uppercase_md() {
        assert_eq!(title_from_filename("README.MD"), "README");
    }

    #[test]
    fn title_from_filename_no_extension() {
        assert_eq!(title_from_filename("README"), "README");
    }

    #[test]
    fn title_from_filename_nested_path() {
        assert_eq!(title_from_filename("docs/sub/note.md"), "note");
    }

    #[test]
    fn title_from_filename_strips_office_ext() {
        assert_eq!(title_from_filename("财务/报表.xlsx"), "报表");
        assert_eq!(title_from_filename("report.docx"), "report");
        assert_eq!(title_from_filename("manual.pdf"), "manual");
    }

    #[test]
    fn is_indexable_ext_matches_supported_formats() {
        // 可索引
        for ext in ["md", "docx", "xlsx", "xls", "xlsb", "ods", "pdf"] {
            assert!(is_indexable_ext(ext), "{ext} 应可索引");
        }
        // 大小写不敏感
        assert!(is_indexable_ext("DOCX"));
        assert!(is_indexable_ext("XLSX"));
        // 非可索引
        for ext in ["txt", "json", "html", "doc", "ppt", ""] {
            assert!(!is_indexable_ext(ext), "{ext} 不应可索引");
        }
    }

    #[test]
    fn index_stats_default() {
        let s = IndexStats::default();
        assert_eq!(s.indexed, 0);
        assert_eq!(s.skipped, 0);
        assert_eq!(s.deleted, 0);
    }
}
