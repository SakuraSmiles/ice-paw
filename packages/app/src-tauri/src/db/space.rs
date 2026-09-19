//! DB 空间回收（W3）——boot 后台按 freelist 阈值一次性 VACUUM。
//!
//! 背景：U2-1 图片外置把 base64 移出 content_blocks 后，UPDATE 释放的页只进
//! freelist（auto_vacuum=0），物理文件从不收缩——生产实证 free 534.7MB /
//! used 168.6MB（freelist_count=136875 / page_count=180031 页 × 4096B）。
//! VACUUM 重写整库代价与 used 成正比，只在 free 占比 ≥25% 且 ≥64MB 时才值得
//! 跑，否则不值得磁盘折腾。VACUUM 幂等——失败下次 boot 重试无副作用。

use std::time::Instant;

use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::error::AppResult;

/// free 占比阈值：free 页占比低于该比例（空间主体是活数据）不值得 VACUUM。
const VACUUM_MIN_FREE_RATIO: f64 = 0.25;
/// free 绝对量阈值：占比达标但可回收量仅几十 MB，不值得整库重写的磁盘折腾。
const VACUUM_MIN_FREE_BYTES: u64 = 64 * 1024 * 1024;
/// VACUUM 期间锁等待超时。连接池 acquire_timeout 5s 且未设 busy_timeout；
/// VACUUM 持写锁重写整库，需更长等待避免 SQLITE_BUSY 假失败。
const VACUUM_BUSY_TIMEOUT_MS: u32 = 15_000;
/// 初始延迟：避开 boot 迁移/自愈扫尾（2b 链：崩溃扫尾→backfill→图片外置）写峰。
const BOOT_SWEEP_DELAY: std::time::Duration = std::time::Duration::from_secs(30);

/// 页空间统计（PRAGMA 三读）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpaceStats {
    pub freelist_pages: u64,
    pub total_pages: u64,
    pub page_size: u64,
}

impl SpaceStats {
    /// 可回收字节数（free 页 × 页大小）。
    pub fn free_bytes(&self) -> u64 {
        self.freelist_pages * self.page_size
    }

    /// 当前文件体积字节数（总页数 × 页大小）。
    pub fn total_bytes(&self) -> u64 {
        self.total_pages * self.page_size
    }
}

/// 双阈值判定：free 字节与 free 占比同时达标才值得 VACUUM（≥ 含等号）。
pub fn should_vacuum(stats: &SpaceStats) -> bool {
    if stats.total_pages == 0 || stats.page_size == 0 {
        return false;
    }
    stats.free_bytes() >= VACUUM_MIN_FREE_BYTES
        && stats.free_bytes() as f64 / stats.total_bytes() as f64 >= VACUUM_MIN_FREE_RATIO
}

/// 读页统计（PRAGMA 三读：freelist_count / page_count / page_size）。
pub async fn read_space_stats(pool: &SqlitePool) -> AppResult<SpaceStats> {
    let freelist_pages =
        sqlx::query_scalar::<_, i64>("PRAGMA freelist_count").fetch_one(pool).await? as u64;
    let total_pages =
        sqlx::query_scalar::<_, i64>("PRAGMA page_count").fetch_one(pool).await? as u64;
    let page_size =
        sqlx::query_scalar::<_, i64>("PRAGMA page_size").fetch_one(pool).await? as u64;
    Ok(SpaceStats {
        freelist_pages,
        total_pages,
        page_size,
    })
}

/// boot 后台空间回收扫尾：延迟后读统计 → 达阈值一次性 VACUUM。
/// 全失败路径仅 warn——VACUUM 幂等，下次启动重试无害。
pub async fn boot_vacuum_sweep(pool: SqlitePool) {
    tokio::time::sleep(BOOT_SWEEP_DELAY).await;
    let stats = match read_space_stats(&pool).await {
        Ok(s) => s,
        Err(e) => {
            warn!(
                target: "ice_paw.db",
                "空间回收：读页统计失败（本轮跳过）: {e}"
            );
            return;
        }
    };
    if !should_vacuum(&stats) {
        tracing::debug!(
            target: "ice_paw.db",
            "空间回收：未达阈值跳过（free {:.1}MB / total {:.1}MB）",
            stats.free_bytes() as f64 / 1048576.0,
            stats.total_bytes() as f64 / 1048576.0
        );
        return;
    }
    let mut conn = match pool.acquire().await {
        Ok(c) => c,
        Err(e) => {
            warn!(
                target: "ice_paw.db",
                "空间回收：取连接失败（下次启动重试）: {e}"
            );
            return;
        }
    };
    if let Err(e) = sqlx::query(&format!("PRAGMA busy_timeout={VACUUM_BUSY_TIMEOUT_MS}"))
        .execute(&mut *conn)
        .await
    {
        warn!(
            target: "ice_paw.db",
            "空间回收：busy_timeout 设置失败（本轮跳过）: {e}"
        );
        return;
    }
    let started = Instant::now();
    match sqlx::query("VACUUM").execute(&mut *conn).await {
        Ok(_) => {
            // 防御性重设：VACUUM 理论上保留 journal_mode，重设 WAL 幂等；
            // 失败仅 warn——下个新连接的连接池选项自动校正。
            if let Err(e) = sqlx::query("PRAGMA journal_mode=WAL")
                .execute(&mut *conn)
                .await
            {
                warn!(
                    target: "ice_paw.db",
                    "空间回收：VACUUM 后重设 WAL 失败（新连接自愈）: {e}"
                );
            }
            info!(
                target: "ice_paw.db",
                "空间回收：VACUUM 完成，回收 {:.1}MB，耗时 {:.1}s",
                stats.free_bytes() as f64 / 1048576.0,
                started.elapsed().as_secs_f64()
            );
        }
        Err(e) => {
            warn!(
                target: "ice_paw.db",
                "空间回收：VACUUM 失败（下次启动幂等重试）: {e}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vacuums_when_over_both_thresholds() {
        // 生产实证形态：free 136875/180031 页 × 4096B ≈ 534.7MB，占比 0.76
        let stats = SpaceStats {
            freelist_pages: 136_875,
            total_pages: 180_031,
            page_size: 4096,
        };
        assert!(should_vacuum(&stats));
    }

    #[test]
    fn skips_when_ratio_enough_but_bytes_too_small() {
        // 占比 0.9 但 free 仅 36.9MB < 64MB——小回收量不值得整库重写
        let stats = SpaceStats {
            freelist_pages: 9_000,
            total_pages: 10_000,
            page_size: 4096,
        };
        assert!(!should_vacuum(&stats));
    }

    #[test]
    fn skips_when_bytes_enough_but_ratio_too_low() {
        // 131MB ≥ 64MB 但占比 0.16 < 0.25——库主体是活数据非虚胀
        let stats = SpaceStats {
            freelist_pages: 32_000,
            total_pages: 200_000,
            page_size: 4096,
        };
        assert!(!should_vacuum(&stats));
    }

    #[test]
    fn skips_on_zero_pages_or_page_size() {
        assert!(!should_vacuum(&SpaceStats {
            freelist_pages: 0,
            total_pages: 0,
            page_size: 4096,
        }));
        assert!(!should_vacuum(&SpaceStats {
            freelist_pages: 100,
            total_pages: 0,
            page_size: 4096,
        }));
        assert!(!should_vacuum(&SpaceStats {
            freelist_pages: 100,
            total_pages: 200,
            page_size: 0,
        }));
    }

    #[test]
    fn boundary_ratio_at_threshold_passes() {
        // 恰 64MiB free 且恰 0.25 占比——≥ 含等号
        let stats = SpaceStats {
            freelist_pages: 16_384,
            total_pages: 65_536,
            page_size: 4096,
        };
        assert_eq!(stats.free_bytes(), 64 * 1024 * 1024);
        assert_eq!(stats.total_bytes(), 256 * 1024 * 1024);
        assert!(should_vacuum(&stats));
    }
}
