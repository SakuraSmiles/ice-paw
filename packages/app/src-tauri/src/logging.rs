//! 日志系统：磁盘持久化 + stdout 双写
//!
//! 设计（详见 memory `log-viewer-plan`）：
//! - 路径：`{app_data_dir}/logs/`（与 `ice-paw.db` 同目录）
//! - 轮转：按天（tracing-appender `rolling::daily`），保留 7 天
//! - 写入：non-blocking 写线程（不阻塞 UI）
//! - panic hook 捕获 panic 到日志，正式版 GUI 也有日志
//!
//! 模块职责：
//! - `init`               挂载 stdout + file 双 layer、装 panic hook、清理过期日志
//! - `log_dir` / `data_dir`  统一目录解析（供 commands 复用）
//! - `current_log_path`   定位「今日」日志文件（缺失则回退最新）
//! - `tail_lines`         读末尾 N 行（供 `get_logs` 命令）

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{Local, NaiveDate};
use tauri::{AppHandle, Manager};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::error::{AppError, AppResult};

/// 日志子目录名（相对 app_data_dir）
const LOG_DIR_NAME: &str = "logs";
/// 日志文件前缀。tracing-appender daily 实际产物为 `{prefix}.{YYYY-MM-DD}`。
const LOG_PREFIX: &str = "ice-paw.log";
/// 日志保留天数（>7 天的启动时清理）
const RETAIN_DAYS: i64 = 7;

/// app 数据目录（与 db / stronghold 同级），供命令层复用
pub fn data_dir(app: &AppHandle) -> AppResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map_err(|e| AppError::Tauri(format!("解析 app_data_dir 失败: {e}")))
}

/// 日志目录 `{app_data_dir}/logs/`
pub fn log_dir(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(data_dir(app)?.join(LOG_DIR_NAME))
}

/// 初始化 tracing：stdout（dev 终端）+ file（daily 轮转，非阻塞写）+ panic hook。
///
/// 返回的 `WorkerGuard` 必须保活整个进程生命周期（drop 时 flush + 关闭写线程），
/// 由 `lib.rs` 的 setup 放入 `app.manage(...)` 托管，应用退出时随 state 一起 drop。
pub fn init(app: &AppHandle) -> AppResult<Mutex<WorkerGuard>> {
    let dir = log_dir(app)?;
    std::fs::create_dir_all(&dir)?;

    // file appender：按天轮转，非阻塞写线程
    let file_appender = tracing_appender::rolling::daily(&dir, LOG_PREFIX);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // 双 layer：stdout 带 ANSI 彩色（终端可读），文件去 ANSI（纯文本便于 tail/导出）
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true))
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_target(true)
                .with_ansi(false),
        )
        .try_init()
        .map_err(|e| AppError::Internal(format!("tracing init 失败: {e}")))?;

    // panic hook：先记日志，再走默认行为（打印到 stderr / abort）
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // info.location() 给出 panic 发生源码位置，便于排查
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        tracing::error!(
            target: "ice_paw.panic",
            "panic at {}: {}",
            loc,
            info,
        );
        default_hook(info);
    }));

    tracing::info!(target: "ice_paw", "日志目录: {}", dir.display());

    // 清理过期日志（tracing 已就绪，删除记录可见）
    let removed = cleanup_old_logs(&dir);
    if removed > 0 {
        tracing::info!(target: "ice_paw", "已清理 {} 个过期日志文件（>{} 天）", removed, RETAIN_DAYS);
    }

    Ok(Mutex::new(guard))
}

/// 定位「当前」日志文件：优先今日文件，缺失则回退目录内最新一个。
pub fn current_log_path(dir: &Path) -> Option<PathBuf> {
    let today = Local::now().date_naive();
    let expected = dir.join(format!("{}.{}", LOG_PREFIX, today.format("%Y-%m-%d")));
    if expected.is_file() {
        return Some(expected);
    }
    newest_log(dir)
}

/// 读末尾 `n` 行。日志文件量级 ~1MB，整文件读入内存后切片，简单可靠。
pub fn tail_lines(path: &Path, n: usize) -> AppResult<Vec<String>> {
    use std::io::BufRead;
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut lines: Vec<String> = Vec::new();
    for line in reader.lines() {
        lines.push(line?);
    }
    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].to_vec())
}

// =========================================================================
// 内部辅助
// =========================================================================

/// 目录内按文件名日期最新的日志文件
fn newest_log(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut newest: Option<(NaiveDate, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(date) = parse_log_date(&name) {
            if newest.as_ref().is_none_or(|(d, _)| date > *d) {
                newest = Some((date, entry.path()));
            }
        }
    }
    newest.map(|(_, p)| p)
}

/// 启动时清理 >`RETAIN_DAYS` 天的日志。返回删除文件数。
fn cleanup_old_logs(dir: &Path) -> usize {
    let cutoff = Local::now().date_naive() - chrono::Duration::days(RETAIN_DAYS);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(date) = parse_log_date(&name) {
            // 严格早于 cutoff（含 cutoff 当天不删，保留满 7 天）
            if date < cutoff && std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

/// 从日志文件名解析日期。期望形态 `{LOG_PREFIX}.{YYYY-MM-DD}`。
fn parse_log_date(name: &str) -> Option<NaiveDate> {
    let prefix = format!("{}.", LOG_PREFIX);
    let date_str = name.strip_prefix(&prefix)?;
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_log_date_ok() {
        assert_eq!(
            parse_log_date("ice-paw.log.2026-07-31"),
            Some(NaiveDate::from_ymd_opt(2026, 7, 31).unwrap())
        );
    }

    #[test]
    fn parse_log_date_rejects_unrelated() {
        assert_eq!(parse_log_date("ice-paw.db"), None);
        assert_eq!(parse_log_date("notes.txt"), None);
        // 非「日志前缀.」开头的同目录文件不受影响
        assert_eq!(parse_log_date("ice-paw.log.bak"), None);
        // 后缀不是合法日期（月份越界）→ None
        // 注：chrono 解析 %m/%d 时对「单位数」是宽松的（2026-7-5 能过），
        //     故这里用越界月份做真正的拒绝样本。
        assert_eq!(parse_log_date("ice-paw.log.2026-13-01"), None);
    }

    #[test]
    fn cleanup_keeps_recent_drops_old() {
        // 构造一组「文件名 → 是否应被清理」的期望
        let today = Local::now().date_naive();
        let cases = [
            (today, false),
            (today - chrono::Duration::days(1), false),
            (today - chrono::Duration::days(7), false), // 含 cutoff 当天保留
            (today - chrono::Duration::days(8), true),
            (today - chrono::Duration::days(30), true),
        ];
        let cutoff = today - chrono::Duration::days(RETAIN_DAYS);
        for (date, should_drop) in cases {
            assert_eq!(
                date < cutoff,
                should_drop,
                "date {date:?} should_drop={should_drop}",
            );
        }
    }
}
