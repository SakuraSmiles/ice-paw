//! KB 目录监听 —— notify + debounce 后台监听变更 → 触发增量索引
//!（RAG v1 摄入管道第 3 环）
//!
//! 职责：启动时对每个 `enabled` KB 的 `directory` 注册 watch，并先做一次全量
//! 索引；之后文件保存/增删自动触发该 KB 的 `index_directory`（幂等增量）。
//!
//! 设计要点：
//! - `notify-debouncer-full` 合并短时间内的密集文件事件（2s 窗口），避免
//!   编辑器「保存时多次写」造成的重复索引
//! - **debouncer 必须保活**：它持有底层 watcher + 后台线程，drop 即停止监听。
//!   这里把 debouncer move 进消费线程（`spawn_blocking`），使其生命周期 = 消费线程
//! - 消费线程是同步的（mpsc 阻塞接收），通过捕获的 `tokio::runtime::Handle`
//!   用 `handle.spawn` 调起 async 的 `index_directory`
//!
//! 启动集成：本模块只提供 `start()`；在 `lib.rs` setup 阶段调用（第 5 步）。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use sqlx::SqlitePool;

use crate::db::models::Kb;
use crate::db::repo;
use crate::error::{AppError, AppResult};

use super::indexer::index_directory;

/// 文件事件的 debounce 窗口：编辑器单次保存常产生多次写事件，合并为一个索引批次。
const DEBOUNCE_WINDOW: Duration = Duration::from_secs(2);

/// 启动 KB 监听 + 首次全量索引。
///
/// 1. 加载所有 `enabled` KB，对其 `directory` 注册 watch
/// 2. 对每个 KB 触发一次 `index_directory`（与磁盘同步）
/// 3. 后台消费线程持续监听变更 → 路由到对应 KB 做增量索引
///
/// 无启用的 KB 时静默返回（不启动 watcher）。单个 KB watch 失败仅 warn，
/// 不影响其它 KB 与整体启动。
pub async fn start(pool: SqlitePool) -> AppResult<()> {
    let kbs = repo::kb::list_all(&pool).await?;
    let enabled: Vec<Kb> = kbs.into_iter().filter(|k| k.enabled).collect();
    if enabled.is_empty() {
        tracing::info!(target: "ice_paw.kb", "无启用的知识库，watcher 不启动");
        return Ok(());
    }

    // directory → kb_id 映射（事件路由用）
    let dir_map: HashMap<PathBuf, String> = enabled
        .iter()
        .map(|k| (PathBuf::from(&k.directory), k.id.clone()))
        .collect();

    let (tx, rx) = std::sync::mpsc::channel::<DebounceEventResult>();
    let mut debouncer = match new_debouncer(DEBOUNCE_WINDOW, None, tx) {
        Ok(d) => d,
        Err(e) => {
            return Err(AppError::Internal(format!(
                "创建 KB watcher 失败: {e}"
            )))
        }
    };

    for kb in &enabled {
        let dir = Path::new(&kb.directory);
        match debouncer.watch(dir, RecursiveMode::Recursive) {
            Ok(()) => tracing::info!(
                target: "ice_paw.kb",
                "watch KB {} -> {}",
                kb.id,
                kb.directory
            ),
            Err(e) => tracing::warn!(
                target: "ice_paw.kb",
                "watch 失败 KB={} dir={} err={}",
                kb.id,
                kb.directory,
                e
            ),
        }
    }

    let handle = tokio::runtime::Handle::current();

    // 首次全量索引（与磁盘同步）
    for kb in &enabled {
        let pool = pool.clone();
        let kb_id = kb.id.clone();
        let dir = PathBuf::from(&kb.directory);
        let h = handle.clone();
        h.spawn(async move {
            if let Err(e) = index_directory(&pool, &kb_id, &dir).await {
                tracing::warn!(
                    target: "ice_paw.kb",
                    "启动全量索引失败 KB={} err={}",
                    kb_id,
                    e
                );
            }
        });
    }

    // 后台消费事件：debouncer move 进闭包保活（其生命周期 = 消费线程）
    let dir_map = Arc::new(dir_map);
    tokio::task::spawn_blocking(move || {
        // 下划线前缀变量 = 有意绑定但不读，确保 debouncer 活到循环结束
        let _keep_alive = debouncer;
        run_consumer(rx, pool, dir_map, handle);
    });

    Ok(())
}

/// 事件消费循环：debounce 事件 → 路由出涉及的 KB → handle.spawn 增量索引。
///
/// 在 `spawn_blocking` 线程里运行；`handle.spawn` 把 async 的 `index_directory`
/// 投递回 tokio runtime。
fn run_consumer(
    rx: std::sync::mpsc::Receiver<DebounceEventResult>,
    pool: SqlitePool,
    dir_map: Arc<HashMap<PathBuf, String>>,
    handle: tokio::runtime::Handle,
) {
    for result in rx {
        match result {
            Ok(events) => {
                // 收集本次事件涉及的 KB（去重），每个 KB 只触发一次增量索引
                let mut dirty: HashSet<String> = HashSet::new();
                for ev in &events {
                    for path in &ev.paths {
                        if let Some(kb_id) = route_path(&dir_map, path) {
                            dirty.insert(kb_id.clone());
                        }
                    }
                }
                for kb_id in dirty {
                    // 反查该 KB 的 directory
                    let Some(dir) = dir_map
                        .iter()
                        .find_map(|(d, id)| (id == &kb_id).then_some(d.clone()))
                    else {
                        continue;
                    };
                    let pool = pool.clone();
                    let handle = handle.clone();
                    handle.spawn(async move {
                        tracing::debug!(
                            target: "ice_paw.kb",
                            "文件变更触发增量索引 KB={}",
                            kb_id
                        );
                        if let Err(e) = index_directory(&pool, &kb_id, &dir).await {
                            tracing::warn!(
                                target: "ice_paw.kb",
                                "增量索引失败 KB={} err={}",
                                kb_id,
                                e
                            );
                        }
                    });
                }
            }
            Err(errs) => {
                tracing::warn!(target: "ice_paw.kb", "KB watcher 事件错误: {:?}", errs);
            }
        }
    }
    tracing::info!(target: "ice_paw.kb", "KB watcher 消费线程退出");
}

/// 路径 → 所属 KB 的 id（最长前缀匹配）。
///
/// 嵌套目录场景下，事件路径可能同时「starts_with」多个 KB 根，取 component 数
/// 最多的（最具体）那个。
fn route_path<'a>(dir_map: &'a HashMap<PathBuf, String>, path: &Path) -> Option<&'a String> {
    dir_map
        .iter()
        .filter(|(dir, _)| path.starts_with(dir))
        .max_by_key(|(dir, _)| dir.components().count())
        .map(|(_, id)| id)
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_path_matches_most_specific_prefix() {
        let mut m = HashMap::new();
        m.insert(PathBuf::from("a"), "kb-a".to_string());
        m.insert(PathBuf::from("a/b"), "kb-ab".to_string());
        // 嵌套路径命中更具体的 KB
        assert_eq!(
            route_path(&m, Path::new("a/b/note.md")),
            Some(&"kb-ab".to_string())
        );
        // 只匹配上层
        assert_eq!(
            route_path(&m, Path::new("a/other.md")),
            Some(&"kb-a".to_string())
        );
    }

    #[test]
    fn route_path_returns_none_when_outside() {
        let mut m = HashMap::new();
        m.insert(PathBuf::from("a"), "kb-a".to_string());
        assert_eq!(route_path(&m, Path::new("c/x.md")), None);
    }
}
