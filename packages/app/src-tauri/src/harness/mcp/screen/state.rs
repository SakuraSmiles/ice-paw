//! [`ScreenState`] —— 按会话键控的「最近一次截图」坐标元数据。
//!
//! 坐标契约的运行时半边：每次成功截图把 [`CaptureMeta`] 写进本状态
//! （键 = conv_id，`ToolContext` 透传）；后续 `region` 裁剪与（操作阶段）
//! 鼠标键盘工具读最近一份做「图片像素 → 物理像素」换算 + 布局 revalidate。
//!
//! 会话数上限 64（LRU 淘汰）：坐标基准只对活跃对话有意义，旧会话重聊时
//! 第一次输入必然先重新截图（无 meta → 语义上等价过期，报「先截图」错）。
//!
//! 线程模型：`std::sync::Mutex` 包整体（map + 淘汰序一体，无双锁不一致），
//! 临界区只有 clone/insert（微秒级），持锁期间无 await——async 上下文安全。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use super::coords::CaptureMeta;

/// 会话坐标基准容量（超出按最久未使用淘汰）。
const MAX_CONVERSATIONS: usize = 64;

#[derive(Default)]
struct Inner {
    map: HashMap<String, CaptureMeta>,
    /// 近期使用序（front = 最新）。与 map 同锁一体维护。
    order: VecDeque<String>,
}

/// 进程级截图坐标状态（全局单例见 [`global`]；测试可独立 new）。
pub struct ScreenState {
    inner: Mutex<Inner>,
}

impl ScreenState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    /// 写入/刷新某会话的坐标基准（新截图成功后调用）。
    pub fn update(&self, conv_id: &str, meta: CaptureMeta) {
        let mut g = self.lock();
        // 近期使用序：命中提 front，未命中插 front（位置查找 O(n)，n ≤ 64 可忽略）
        if let Some(pos) = g.order.iter().position(|k| k == conv_id) {
            g.order.remove(pos);
        }
        g.order.push_front(conv_id.to_string());
        g.map.insert(conv_id.to_string(), meta);
        // 先 insert 再按容量淘汰（判据须包含刚插入的键；back 不可能是刚提 front 的新键）
        while g.map.len() > MAX_CONVERSATIONS {
            match g.order.pop_back() {
                Some(evict) => {
                    g.map.remove(&evict);
                }
                None => break,
            }
        }
    }

    /// 读某会话的坐标基准（None = 本会话还没截过图，或已被 LRU 淘汰）。
    pub fn get(&self, conv_id: &str) -> Option<CaptureMeta> {
        self.lock().map.get(conv_id).cloned()
    }

    /// 当前持有的会话数（诊断/测试用）。
    pub fn len(&self) -> usize {
        self.lock().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // 中毒 = 持锁线程 panic 过；临界区无 panic 源（clone/insert），中毒即上游
        // 已损坏——沿用 poisoned 语义继续用比把工具层 panic 传播出去更稳。
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for ScreenState {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL: OnceLock<Arc<ScreenState>> = OnceLock::new();

/// 进程级共享实例（capture_screen / 后续 capture_window 等工具共用同一坐标基准）。
/// 返回 `Arc` clone 供工具持有；测试用 `ScreenState::new()` 隔离。
pub fn global() -> Arc<ScreenState> {
    GLOBAL
        .get_or_init(|| Arc::new(ScreenState::new()))
        .clone()
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::mcp::screen::coords::{PhysRect, VirtualScreenLayout};

    fn meta(tag: i32) -> CaptureMeta {
        CaptureMeta {
            layout: VirtualScreenLayout {
                origin_x: 0,
                origin_y: 0,
                width: 1920,
                height: 1080,
            },
            phys_region: PhysRect {
                x: tag,
                y: 0,
                width: 100,
                height: 100,
            },
            sent_width: 50,
            sent_height: 50,
            monitor: None,
        }
    }

    #[test]
    fn update_then_get_roundtrip() {
        let s = ScreenState::new();
        assert!(s.get("c1").is_none());
        s.update("c1", meta(7));
        assert_eq!(s.get("c1"), Some(meta(7)));
        // 覆盖更新
        s.update("c1", meta(9));
        assert_eq!(s.get("c1"), Some(meta(9)));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn lru_evicts_oldest_beyond_cap() {
        let s = ScreenState::new();
        for i in 0..=MAX_CONVERSATIONS as i32 {
            s.update(&format!("c{i}"), meta(i));
        }
        assert_eq!(s.len(), MAX_CONVERSATIONS);
        // 最旧的 c0 被淘汰
        assert!(s.get("c0").is_none());
        assert!(s.get("c1").is_some());
        assert!(s.get(&format!("c{MAX_CONVERSATIONS}")).is_some());
    }

    #[test]
    fn touching_refreshes_recency() {
        let s = ScreenState::new();
        for i in 0..=MAX_CONVERSATIONS as i32 {
            s.update(&format!("c{i}"), meta(i));
        }
        // c0 已被淘汰；刷新 c1 热度后再压 63 个新会话——恰好只挤掉 c2..c64
        //（比 c1 旧的全部 63 个），c1 存活；若多压 1 个（64 个）c1 也会出局。
        s.update("c1", meta(1));
        let older_than_refreshed = MAX_CONVERSATIONS as i32 - 1; // = c2..c64 共 63 个
        for i in (MAX_CONVERSATIONS as i32 + 1)
            ..=(MAX_CONVERSATIONS as i32 + older_than_refreshed)
        {
            s.update(&format!("c{i}"), meta(i));
        }
        assert_eq!(s.len(), MAX_CONVERSATIONS);
        assert!(s.get("c1").is_some(), "刷新过的会话不应被淘汰");
        assert!(s.get("c2").is_none(), "未刷新的最老会话应被淘汰");
    }

    #[test]
    fn global_is_shared_singleton() {
        let a = global();
        let b = global();
        a.update("__probe__", meta(0));
        assert!(b.get("__probe__").is_some());
        // ScreenState 无删除接口（坐标基准只覆盖/淘汰，永不显式删），
        // 探针残留无害——其余测试一律用独立 ScreenState::new()。
    }
}
