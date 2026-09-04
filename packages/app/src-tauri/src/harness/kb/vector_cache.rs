//! KB 向量缓存（2026-09-04 质检 Q7）：search_kb 语义检索的 per-KB 已解码向量缓存。
//!
//! ## 为什么
//!
//! 语义检索原先每次调用 `load_chunks_for_vector_search` 全量加载范围内所有
//! chunk 的 `content`（大文本列）+ `embedding` BLOB（1536 维 ≈ 6 KB/chunk），
//! 再逐 chunk `bytes_to_embedding` 解码——全在 async worker 线程上。千 chunk 级
//! KB 单次搜索即物化数 MB 字符串 + 向量，agent 连续多轮 search_kb 时重复支付。
//!
//! ## 失效策略：签名失效（而非写点失效）
//!
//! 每个 KB 的签名 = `(COUNT(*), COUNT(embedding), MAX(rowid), SUM(LENGTH(content)))`，
//! 一条 GROUP BY SQL 取全部（[`crate::db::repo::kb::chunk_signatures`]）：
//! - 增删 chunk → COUNT(\*) 变；向量补齐/清空 → COUNT(embedding) 变；
//! - 同数量行替换（增量 upsert 的 DELETE+INSERT，COUNT 可能凑巧不变）→
//!   MAX(rowid) 兜底——但 SQLite DELETE 后**回收 rowid**（删最高行再插同数量，
//!   MAX 不变；单测实测踩中），且变更 chunk 的向量被 indexer 预生成立即回填后
//!   COUNT(embedding) 也复原 → 唯有内容长度和（第 4 维）能区分「换过内容的
//!   同构表」。
//!
//! 写点失效（update/upsert/clear 处各自 evict）不可取：写点分散（repo 层无 kb_id
//! 语境的 update_chunk_embedding、indexer 的 upsert、rebuild 的全清），漏一处即
//! 脏缓存；签名失效只用读侧一条 SQL，漏不掉任何行级变化。
//!
//! ## 顺序不变式（守住）
//!
//! **签名必须先于数据读**（T0 ≤ T1）：签名查询（T0）→ load 全量 chunk（T1）→
//! store（以 T0 签名入账）。若 T0..T1 间数据变化，缓存记的是旧签名 → 下次搜索
//! 的新签名对不上 → 失效重载（过度失效方向，永不脏）。反过来（先 load 后取签名）
//! 会缓存新签名 + 旧数据 = 脏缓存。
//!
//! ## 完整性约定（懒生成自愈语义）
//!
//! 只缓存**向量齐全**的 KB：仍有 NULL embedding 的 KB（懒生成网络失败等）不入
//! 缓存 → 下次搜索恒 miss → 冷路径重跑 `ensure_chunks_embedded` 重试懒生成
//! （既有自愈语义不因缓存丢失）。但该 KB **已嵌入的 chunk 仍参与本次召回**
//! （检索面与缓存面分离，见 [`decode_cold`]）——懒生成部分失败只降缓存效率，
//! 不降召回质量。空 KB 存零 chunk 空条目（零签，与 chunk_signatures 对
//! 无 chunk KB 不返回条目对称），暖路径命中空集 → 检索语义与「范围内无 chunk」
//! 一致。
//!
//! ## 内存画像
//!
//! CachedChunk 不存 content（大文本列不缓存），每 chunk ≈ 6 KB 向量 + 元数据
//! 字符串；千 chunk 级 KB ≈ 6 MB。KB 删除（remove）/ 全量重建（clear）时清理。

use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};

use crate::db::repo::kb::{bytes_to_embedding, ChunkWithEmbedding};

/// KB 签名：`(COUNT(*), COUNT(embedding), MAX(rowid), SUM(LENGTH(content)))`
/// （各维抓什么见模块头；与 [`crate::db::repo::kb::chunk_signatures`] 同型）。
pub type KbSig = (i64, i64, i64, i64);

/// 无 chunk 的 KB 在签名 map 中的等价签名（chunk_signatures 不返回空 KB 条目，
/// 调用方以本值补齐，使「空 KB 的缓存条目」与「DB 侧空」可用同一比较）。
pub const EMPTY_KB_SIG: KbSig = (0, 0, 0, 0);

/// 缓存的单个 chunk：检索所需元数据 + 已解码向量（不含 content 大列）。
#[derive(Debug, Clone, PartialEq)]
pub struct CachedChunk {
    pub kb_id: String,
    pub id: String,
    pub file_path: String,
    pub title: String,
    pub summary: String,
    pub vec: Vec<f32>,
}

#[derive(Debug)]
struct CacheEntry {
    sig: KbSig,
    chunks: Vec<CachedChunk>,
}

/// per-KB 向量缓存（进程级单例，见 [`KbVectorCache::global`]）。
///
/// 锁选型：临界区是纯内存瞬态操作（查表/插入/回调内余弦），无并发 await，
/// `std::sync::RwLock` 足够（read_route 同款）。暖路径余弦在读锁内回调执行
/// （guard 非 Send 不能跨 await，闭包内联是唯一形态；百 chunk 级亚毫秒）。
#[derive(Default)]
pub struct KbVectorCache {
    inner: RwLock<HashMap<String, CacheEntry>>,
}

impl KbVectorCache {
    /// 进程级单例（ScreenState::global 同款 OnceLock 模式）。
    pub fn global() -> &'static Self {
        static GLOBAL: OnceLock<KbVectorCache> = OnceLock::new();
        GLOBAL.get_or_init(KbVectorCache::default)
    }

    /// 暖路径：范围内**所有** KB 都已缓存且签名与 DB 一致 → 在读锁内以扁平
    /// chunk 列表回调 `f`，返回 `Some(f(...))`；任一 KB 未缓存/签名失配 →
    /// `None`（调用方走冷路径全量重载）。
    ///
    /// all-or-nothing（不做 per-KB 部分命中）：冷路径的 load 本就是一条
    /// IN 查询全量加载，部分命中省不掉那次查询，只会增加混合态复杂度。
    ///
    /// `sigs` 为**新鲜**DB 签名（调用方刚查的 chunk_signatures 结果）；无 chunk
    /// 的 KB 以 [`EMPTY_KB_SIG`] 语义比较。
    pub fn with_matches<R>(
        &self,
        kb_ids: &[String],
        sigs: &HashMap<String, KbSig>,
        f: impl FnOnce(&[&CachedChunk]) -> R,
    ) -> Option<R> {
        if kb_ids.is_empty() {
            return Some(f(&[]));
        }
        let guard = self.inner.read().ok()?;
        let mut flat: Vec<&CachedChunk> = Vec::new();
        for id in kb_ids {
            let db_sig = sigs.get(id).copied().unwrap_or(EMPTY_KB_SIG);
            let entry = guard.get(id)?;
            if entry.sig != db_sig {
                return None;
            }
            flat.extend(entry.chunks.iter());
        }
        Some(f(&flat))
    }

    /// 冷路径收尾：把刚解码的完整 KB 条目入账（以 `sigs` 的当前签名；顺序
    /// 不变式见模块头——sigs 必须取自 load **之前**的查询）。
    ///
    /// kb_ids 中既无 DB 条目也无 entries 输入的 KB（空 KB）写零 chunk 空条目，
    /// 使下次搜索暖命中空集而非恒 miss。
    pub fn store(
        &self,
        kb_ids: &[String],
        sigs: &HashMap<String, KbSig>,
        entries: HashMap<String, Vec<CachedChunk>>,
    ) {
        let Ok(mut guard) = self.inner.write() else {
            return;
        };
        for id in kb_ids {
            let sig = sigs.get(id).copied().unwrap_or(EMPTY_KB_SIG);
            let chunks = entries.get(id).cloned().unwrap_or_default();
            guard.insert(id.clone(), CacheEntry { sig, chunks });
        }
    }

    /// 单 KB 失效（KB 删除时的内存卫生；不清也无泄漏——条目是小集合，签名
    /// 失效兜底正确性）。
    pub fn remove(&self, kb_id: &str) {
        if let Ok(mut guard) = self.inner.write() {
            guard.remove(kb_id);
        }
    }

    /// 全部失效（切换 embedding 模型全量重建时；签名失效本已覆盖，此为卫生清理）。
    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.write() {
            guard.clear();
        }
    }

    /// 诊断：当前缓存的 KB 数与 chunk 总数（日志/测试用）。
    pub fn stats(&self) -> (usize, usize) {
        let Ok(guard) = self.inner.read() else {
            return (0, 0);
        };
        (guard.len(), guard.values().map(|e| e.chunks.len()).sum())
    }
}

// =========================================================================
// 冷路径解码
// =========================================================================

/// 冷路径解码产物：一次遍历同时产出**检索面**与**缓存面**。
#[derive(Debug, Default)]
pub struct ColdDecode {
    /// 检索面：所有有向量的 chunk——含不完整 KB 的已嵌入部分（与缓存引入前的
    /// 行为一致：懒生成部分失败时不丢该 KB 已嵌入 chunk 的召回质量）。
    pub flat: Vec<CachedChunk>,
    /// 存在向量缺失 chunk 的 KB 名单（NULL 懒生成失败 / BLOB 损坏）——这些 KB
    /// 不入缓存（下次搜索 miss → 重试懒生成，自愈语义保持）。
    pub incomplete_kbs: HashSet<String>,
}

/// 解码冷路径 load 出的 chunk 列表（同步重活，调用方应包在 spawn_blocking 里）。
///
/// 损坏 BLOB（长度非 4 的倍数，仅损坏数据会出现）按缺失处理——as_chunks 会
/// 静默截断造出短向量，进 recall 才按维度不匹配告警，不如标记不完整不入账。
pub fn decode_cold(chunks: &[ChunkWithEmbedding]) -> ColdDecode {
    let mut out = ColdDecode {
        flat: Vec::with_capacity(chunks.len()),
        incomplete_kbs: HashSet::new(),
    };
    for c in chunks {
        match c.embedding.as_ref() {
            Some(bytes) if bytes.len() % 4 == 0 => out.flat.push(CachedChunk {
                kb_id: c.kb_id.clone(),
                id: c.id.clone(),
                file_path: c.file_path.clone(),
                title: c.title.clone(),
                summary: c.summary.clone(),
                vec: bytes_to_embedding(bytes),
            }),
            _ => {
                out.incomplete_kbs.insert(c.kb_id.clone());
            }
        }
    }
    out
}

/// 把解码产物分组为缓存条目（消耗 flat）：不完整 KB 整体丢弃（其 chunk 已完成
/// 本次召回使命），返回 `(条目, 被跳过的 KB 名单)`——名单供调用方日志披露。
pub fn group_complete(decoded: ColdDecode) -> (HashMap<String, Vec<CachedChunk>>, Vec<String>) {
    let ColdDecode {
        flat,
        incomplete_kbs,
    } = decoded;
    let mut entries: HashMap<String, Vec<CachedChunk>> = HashMap::new();
    for cc in flat {
        entries.entry(cc.kb_id.clone()).or_default().push(cc);
    }
    let mut skipped: Vec<String> = incomplete_kbs.iter().cloned().collect();
    skipped.sort();
    entries.retain(|kb, _| !incomplete_kbs.contains(kb));
    (entries, skipped)
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &str, kb: &str, file: &str, vec: &[f32]) -> ChunkWithEmbedding {
        ChunkWithEmbedding {
            id: id.into(),
            doc_id: format!("doc-{id}"),
            kb_id: kb.into(),
            title: format!("t-{id}"),
            file_path: file.into(),
            summary: "s".into(),
            content: "c".into(),
            embedding: Some(
                vec.iter()
                    .flat_map(|f| f.to_le_bytes())
                    .collect::<Vec<u8>>(),
            ),
        }
    }

    fn cached(id: &str, file: &str) -> CachedChunk {
        CachedChunk {
            kb_id: "k1".into(),
            id: id.into(),
            file_path: file.into(),
            title: format!("t-{id}"),
            summary: "s".into(),
            vec: vec![1.0, 0.0],
        }
    }

    #[test]
    fn warm_hit_after_store() {
        let cache = KbVectorCache::default();
        let kb = "k1".to_string();
        let mut entries = HashMap::new();
        entries.insert(kb.clone(), vec![cached("c1", "a.md"), cached("c2", "b.md")]);
        let mut sigs = HashMap::new();
        sigs.insert(kb.clone(), (2, 2, 7, 12));
        cache.store(std::slice::from_ref(&kb), &sigs, entries);

        let out = cache.with_matches(std::slice::from_ref(&kb), &sigs, |flat| {
            assert_eq!(flat.len(), 2);
            flat.iter().map(|c| c.id.clone()).collect::<Vec<_>>()
        });
        assert_eq!(
            out,
            Some(vec!["c1".to_string(), "c2".to_string()]),
            "同签名应暖命中"
        );
    }

    #[test]
    fn sig_change_is_miss() {
        let cache = KbVectorCache::default();
        let kb = "k1".to_string();
        let mut entries = HashMap::new();
        entries.insert(kb.clone(), vec![cached("c1", "a.md")]);
        let mut sigs = HashMap::new();
        sigs.insert(kb.clone(), (1, 1, 3, 5));
        cache.store(std::slice::from_ref(&kb), &sigs, entries);

        // 任一标量变 → miss（四维各 bump 一档）
        for bumped in [(2, 1, 3, 5), (1, 2, 3, 5), (1, 1, 4, 5), (1, 1, 3, 6)] {
            let mut s2 = HashMap::new();
            s2.insert(kb.clone(), bumped);
            assert!(
                cache.with_matches(std::slice::from_ref(&kb), &s2, |_| ()).is_none(),
                "{bumped:?} 应 miss"
            );
        }
        // 签名回旧值 → 命中（比较纯按值）
        assert!(cache.with_matches(std::slice::from_ref(&kb), &sigs, |_| ()).is_some());
    }

    #[test]
    fn any_missing_kb_is_miss() {
        let cache = KbVectorCache::default();
        let k1 = "k1".to_string();
        let k2 = "k2".to_string();
        let mut entries = HashMap::new();
        entries.insert(k1.clone(), vec![cached("c1", "a.md")]);
        let sigs = HashMap::from([(k1.clone(), (1, 1, 1, 1))]);
        cache.store(std::slice::from_ref(&k1), &sigs, entries);

        // k2 从未缓存 → all-or-nothing miss（即使 k1 签名仍匹配）
        let sigs2 = HashMap::from([(k1.clone(), (1, 1, 1, 1)), (k2.clone(), (1, 1, 1, 1))]);
        assert!(
            cache
                .with_matches(&[k1.clone(), k2.clone()], &sigs2, |_| ())
                .is_none()
        );
    }

    #[test]
    fn empty_kb_stores_zero_entry_and_hits() {
        let cache = KbVectorCache::default();
        let k1 = "k1".to_string();
        let k2 = "k2".to_string();
        let mut entries = HashMap::new();
        entries.insert(k1.clone(), vec![cached("c1", "a.md")]);
        let sigs = HashMap::from([(k1.clone(), (1, 1, 1, 1))]);
        // store 范围含空 KB k2 → 写零条目
        cache.store(&[k1.clone(), k2.clone()], &sigs, entries);

        // k2 无 DB 签名条目 → EMPTY_KB_SIG 语义 → 暖命中空集
        let out = cache.with_matches(&[k1.clone(), k2.clone()], &sigs, |flat| flat.len());
        assert_eq!(out, Some(1), "k1 的 1 chunk + k2 空集，flat 应为 1");

        // k2 后来进 chunk → DB 签名出现 → miss（失效正确）
        let sigs2 = HashMap::from([(k1.clone(), (1, 1, 1, 1)), (k2.clone(), (1, 0, 9, 4))]);
        assert!(
            cache
                .with_matches(&[k1.clone(), k2.clone()], &sigs2, |_| ())
                .is_none()
        );
    }

    #[test]
    fn decode_cold_dual_face_partial_kb() {
        let c1 = chunk("c1", "k1", "a.md", &[1.0, 0.5]);
        let mut c2 = chunk("c2", "k1", "a.md", &[0.7, 0.3]);
        c2.embedding = None; // k1 懒生成失败形态（部分嵌入）
        let c3 = chunk("c3", "k2", "b.md", &[0.0, 1.0]);

        let decoded = decode_cold(&[c1, c2, c3]);
        // 检索面：k1 已嵌入的 c1 + k2 的 c3（c2 缺向量不入，但 c1 不受连坐）
        assert_eq!(decoded.flat.len(), 2, "检索面含部分嵌入 KB 的已嵌入 chunk");
        assert_eq!(decoded.flat[0].vec, vec![1.0, 0.5], "BLOB → f32 解码");
        // 不完整名单：仅 k1
        assert_eq!(decoded.incomplete_kbs, HashSet::from(["k1".into()]));

        // 缓存面：k1 整体丢弃、k2 入账
        let (entries, skipped) = group_complete(decoded);
        assert!(!entries.contains_key("k1"), "不完整 KB 不入缓存（下次重试懒生成）");
        assert_eq!(entries["k2"].len(), 1);
        assert_eq!(entries["k2"][0].file_path, "b.md");
        assert_eq!(skipped, vec!["k1".to_string()], "跳过名单供日志披露");
    }

    #[test]
    fn decode_cold_corrupt_blob_marks_incomplete() {
        let mut c1 = chunk("c1", "k1", "a.md", &[1.0]);
        c1.embedding = Some(vec![1, 2, 3]); // 长度非 4 倍数（损坏 BLOB）
        let decoded = decode_cold(&[c1]);
        assert!(decoded.flat.is_empty());
        assert!(decoded.incomplete_kbs.contains("k1"));
    }

    #[test]
    fn decode_cold_complete_kb_groups_cleanly() {
        let chunks = vec![
            chunk("c1", "k1", "a.md", &[1.0]),
            chunk("c2", "k1", "a.md", &[1.0]),
        ];
        let decoded = decode_cold(&chunks);
        assert!(decoded.incomplete_kbs.is_empty());
        let (entries, skipped) = group_complete(decoded);
        assert!(skipped.is_empty());
        assert_eq!(entries["k1"].len(), 2);
    }

    #[test]
    fn remove_and_clear_hygiene() {
        let cache = KbVectorCache::default();
        let k1 = "k1".to_string();
        let mut entries = HashMap::new();
        entries.insert(k1.clone(), vec![cached("c1", "a.md")]);
        let sigs = HashMap::from([(k1.clone(), (1, 1, 1, 1))]);
        cache.store(std::slice::from_ref(&k1), &sigs, entries);
        assert_eq!(cache.stats(), (1, 1));

        cache.remove(&k1);
        assert_eq!(cache.stats(), (0, 0));

        let mut entries = HashMap::new();
        entries.insert(k1.clone(), vec![cached("c1", "a.md")]);
        cache.store(std::slice::from_ref(&k1), &sigs, entries);
        cache.clear();
        assert_eq!(cache.stats(), (0, 0));
    }

    #[test]
    fn global_singleton_shape() {
        // 两条引用同柄（OnceLock 语义冒烟）
        assert_eq!(
            KbVectorCache::global() as *const _,
            KbVectorCache::global() as *const _
        );
    }
}
