//! 会话级全自动开关注册表（migration 58，2026-09-29 用户拍板「会话级别，
//! 含此会话发起的委派任务」）。
//!
//! 内存 HashSet（boot 从 DB 种子 + `set_conversation_auto_approve` 命令增量
//! 维护 + delegate 创建子会话时继承插入）——工具授权热路径每 Confirm 级调用
//! 查一次（读锁 HashSet，纳秒级，不查 DB）。
//!
//! 生效点：`tool_executor` 的 Confirm 臂——命中会话直接放行（等效用户逐次点
//! 「允许」，授权记忆照走）。**例外（刻意）**：
//! - 屏幕家族不沾光：`request_screen_session` 是屏幕共享通道的唯一入口（Confirm
//!   是它的存在意义），Off 提议制与通道治理是独立授权体系（见 screen/channel.rs
//!   2026-09-24 拍板）——全自动不越界到「看我的屏幕」。
//! - 「禁用的 MCP server 永不自动复活」不受影响（那是 server 治理状态非工具
//!   授权；自动复活=复活禁用，越权）。

use std::collections::HashSet;

use sqlx::SqlitePool;
use tokio::sync::RwLock;

#[derive(Default)]
pub struct AutoApproveRegistry {
    inner: RwLock<HashSet<String>>,
}

/// 进程级单例（screen_channel::global() 先例——工具授权热路径直查，免穿
/// loop context 五层参数；lib.rs boot 种子 + 命令层开关时同步维护）。
pub fn global() -> &'static AutoApproveRegistry {
    static G: std::sync::OnceLock<AutoApproveRegistry> = std::sync::OnceLock::new();
    G.get_or_init(AutoApproveRegistry::default)
}

impl AutoApproveRegistry {

    /// boot 种子：扫全部开启的会话。
    pub async fn seed_from_db(&self, pool: &SqlitePool) {
        match crate::db::repo::conversation::list_auto_approve_ids(pool).await {
            Ok(ids) => {
                let n = ids.len();
                *self.inner.write().await = ids.into_iter().collect();
                if n > 0 {
                    tracing::info!(target: "ice_paw.auto_approve", "会话级全自动开关：{n} 个会话开启");
                }
            }
            Err(e) => {
                tracing::warn!(target: "ice_paw.auto_approve", "boot 扫描全自动会话失败: {e}");
            }
        }
    }

    /// 开关（命令层调用——写 DB 由命令层负责，这里只同步内存）。
    pub async fn set(&self, conv_id: &str, on: bool) {
        let mut g = self.inner.write().await;
        if on {
            g.insert(conv_id.to_string());
        } else {
            g.remove(conv_id);
        }
    }

    /// 委派继承：子会话创建时插入（父会话开启 → 子会话同权）。
    pub async fn inherit(&self, parent: &str, child: &str) {
        let mut g = self.inner.write().await;
        if g.contains(parent) {
            g.insert(child.to_string());
        }
    }

    /// 热路径查询（Confirm 臂）。
    pub async fn is_on(&self, conv_id: &str) -> bool {
        self.inner.read().await.contains(conv_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_toggle_and_inherit() {
        let r = AutoApproveRegistry::default();
        r.set("c1", true).await;
        assert!(r.is_on("c1").await);
        // 委派继承
        r.inherit("c1", "child1").await;
        assert!(r.is_on("child1").await);
        // 父关 → 新子不继承；已继承的子独立（开关语义=出生继承非实时联动）
        r.set("c1", false).await;
        r.inherit("c1", "child2").await;
        assert!(!r.is_on("c2").await);
        assert!(!r.is_on("child2").await);
        assert!(r.is_on("child1").await);
        // 关闭摘除
        r.set("child1", false).await;
        assert!(!r.is_on("child1").await);
    }
}
