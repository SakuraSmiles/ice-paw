//! KB 目录监听管理器 —— 运行时可增删的 watch 注册（模式 A+B 治本）。
//!
//! 原 `watcher::start` 只在 App 启动时一次性注册全部 KB 目录，运行期新建 /
//! 改 workspace 的 agent 的 KB 目录不被监听 → 文件落盘但 `kb_document` 表为空
//! → UI 列表空（即便 `save_to_kb` 内联索引兜住了工具写入路径，用户手动往目录
//! 拖文件仍要重启）。本管理器把 debouncer 收进 `Arc<Mutex<…>>`，暴露
//! `add_watch`/`remove_watch`/`rebind_watch`，供 `agent_cmd` 在 create/update/delete
//! 时调用，实现运行期对账。
//!
//! 生命周期：manager 由 app state 保活（`Arc<KbWatcherManager>`）；debouncer 随
//! manager 存活（其底层 watcher + 事件线程因此常驻）；消费线程只持 mpsc `rx`，
//! debouncer drop 时 `tx` 随之销毁，`rx` 迭代结束 → 线程自然退出。
//!
//! 并发模型：`debouncer.watch/unwatch` 取 `&mut self`，用 `std::sync::Mutex`
//! 串行化（notify 的 watch 是同步非阻塞注册，锁不跨 await，安全）。
//! `dir_map` 同样 `std::sync::Mutex`：manager 写（add/remove）、消费线程读（路由）。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use sqlx::SqlitePool;

use crate::error::{AppError, AppResult};

use super::indexer::index_directory;

/// 文件事件的 debounce 窗口：编辑器单次保存常产生多次写事件，合并为一个索引批次。
const DEBOUNCE_WINDOW: Duration = Duration::from_secs(2);

/// notify 底层 watcher + cache（new_debouncer 的返回类型）。
///
/// 用库导出的 `RecommendedCache`（平台条件别名：Linux=NoCache，Win/Mac=FileIdMap），
/// 与 `new_debouncer` 返回类型严格对齐。**切勿硬编码 `FileIdMap`**——否则 Linux CI
/// 撞 E0308（本地 Windows 测不到该分支，因 Win 上 RecommendedCache 恰好=FileIdMap）。
type KbDebouncer = Debouncer<notify::RecommendedWatcher, RecommendedCache>;

/// KB watcher 管理器：运行时可增删的目录监听 + 增量索引。
///
/// 由 `lib.rs` setup 构造一次，`Arc<KbWatcherManager>` 注入 Tauri State；
/// `agent_cmd` 经 `app.try_state` 取出，在 agent 增删改时调 add/remove/rebind。
pub struct KbWatcherManager {
    debouncer: Arc<std::sync::Mutex<KbDebouncer>>,
    dir_map: Arc<std::sync::Mutex<HashMap<PathBuf, String>>>,
    pool: SqlitePool,
}

impl KbWatcherManager {
    /// 创建管理器：建 debouncer + dir_map + 后台消费线程。
    ///
    /// 必须在 tokio runtime 内调用（消费线程用 `Handle::current()` 把 async 的
    /// `index_directory` 投回 runtime）。失败仅因 `new_debouncer` 出错。
    pub fn new(pool: SqlitePool) -> AppResult<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<DebounceEventResult>();
        let debouncer = match new_debouncer(DEBOUNCE_WINDOW, None, tx) {
            Ok(d) => d,
            Err(e) => {
                return Err(AppError::Internal(format!(
                    "创建 KB watcher 失败: {e}"
                )))
            }
        };
        let dir_map = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let debouncer = Arc::new(std::sync::Mutex::new(debouncer));
        let handle = tokio::runtime::Handle::current();

        let dir_map_for_consumer = Arc::clone(&dir_map);
        let pool_for_consumer = pool.clone();
        tokio::task::spawn_blocking(move || {
            run_consumer(rx, pool_for_consumer, dir_map_for_consumer, handle);
        });

        Ok(Self {
            debouncer,
            dir_map,
            pool,
        })
    }

    /// 注册监听一个 KB 目录 + 初始全量索引（幂等）。
    ///
    /// 运行期新建 KB（agent 创建）或手动纳入目录时调用。三步：
    /// 1. 底层 `debouncer.watch`（递归）；2. dir_map 登记路径→kb_id（事件路由用）；
    /// 3. 后台 `index_directory`（与磁盘同步，幂等）。
    ///
    /// 任何一步失败仅 warn，不向上抛（KB 监听是附加能力，失败回退「重启后补」语义）。
    pub fn add_watch(&self, kb_id: String, directory: String) {
        let dir = PathBuf::from(&directory);
        if let Ok(mut deb) = self.debouncer.lock() {
            if let Err(e) = deb.watch(&dir, RecursiveMode::Recursive) {
                tracing::warn!(
                    target: "ice_paw.kb",
                    "watch 注册失败 kb={} dir={} err={}",
                    kb_id,
                    directory,
                    e
                );
            }
        }
        if let Ok(mut m) = self.dir_map.lock() {
            m.insert(dir.clone(), kb_id.clone());
        }
        tracing::info!(target: "ice_paw.kb", "watch KB {} -> {}", kb_id, directory);

        // 初始全量索引（后台，与磁盘同步）。幂等：已索引文件按 content_hash 跳过。
        let pool = self.pool.clone();
        let kb_id = kb_id.clone();
        let dir = dir.clone();
        tokio::spawn(async move {
            if let Err(e) = index_directory(&pool, &kb_id, &dir).await {
                tracing::warn!(
                    target: "ice_paw.kb",
                    "add_watch 初始索引失败 kb={} err={}",
                    kb_id,
                    e
                );
            }
        });
    }

    /// 取消监听一个目录（agent 删除 / workspace 迁走）。
    pub fn remove_watch(&self, directory: &str) {
        let dir = PathBuf::from(directory);
        if let Ok(mut deb) = self.debouncer.lock() {
            if let Err(e) = deb.unwatch(&dir) {
                tracing::warn!(
                    target: "ice_paw.kb",
                    "unwatch 失败 dir={} err={}",
                    directory,
                    e
                );
            }
        }
        if let Ok(mut m) = self.dir_map.lock() {
            m.remove(&dir);
        }
        tracing::info!(target: "ice_paw.kb", "unwatch dir={}", directory);
    }

    /// workspace 变更：解绑旧目录 + 绑定新目录（agent 更新 workspace_path）。
    pub fn rebind_watch(&self, kb_id: &str, old_dir: Option<&str>, new_dir: &str) {
        if let Some(old) = old_dir.filter(|s| !s.is_empty() && *s != new_dir) {
            self.remove_watch(old);
        }
        self.add_watch(kb_id.to_string(), new_dir.to_string());
    }
}

// ============================================================================
// 事件消费线程（从原 watcher.rs 迁入；dir_map 改为 Mutex 共享）
// ============================================================================

/// 事件消费循环：debounce 事件 → 路由出涉及的 KB → handle.spawn 增量索引。
///
/// 在 `spawn_blocking` 线程里运行；`handle.spawn` 把 async 的 `index_directory`
/// 投递回 tokio runtime。debouncer 由 manager 保活，不在此持有。
fn run_consumer(
    rx: std::sync::mpsc::Receiver<DebounceEventResult>,
    pool: SqlitePool,
    dir_map: Arc<std::sync::Mutex<HashMap<PathBuf, String>>>,
    handle: tokio::runtime::Handle,
) {
    for result in rx {
        match result {
            Ok(events) => {
                // 收集本次事件涉及的 KB（去重），每个 KB 只触发一次增量索引
                let mut dirty: HashSet<String> = HashSet::new();
                {
                    let Ok(m) = dir_map.lock() else { continue };
                    for ev in &events {
                        for path in &ev.paths {
                            if let Some(kb_id) = route_path(&m, path) {
                                dirty.insert(kb_id.clone());
                            }
                        }
                    }
                }
                for kb_id in dirty {
                    // 反查该 KB 的 directory
                    let Some(dir) = ({
                        let Ok(m) = dir_map.lock() else { continue };
                        m.iter()
                            .find_map(|(d, id)| (id == &kb_id).then_some(d.clone()))
                    }) else {
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

// ============================================================================
// 单元测试（route_path 纯逻辑，沿用原 watcher.rs）
// ============================================================================

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
