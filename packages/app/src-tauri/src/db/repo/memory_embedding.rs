//! `memory_embeddings` 表操作（REQ-CHAT-047 语义检索）
//!
//! 提供：
//! - `insert_embedding`：插入一条 (agent_id, content, embedding) 记录
//! - `load_embeddings_for_agent`：加载某 agent 的全部 embedding 记录
//!   （用于 recall 时做 cosine 相似度计算）
//! - `delete_embeddings_for_agent`：删除某 agent 的全部记录（清空记忆）
//! - `cosine_similarity`：在两个 `&[f32]` 上做 cosine 相似度计算
//!
//! ## BLOB 编码
//!
//! `embedding` 列存的是 BLOB，编码格式：little-endian 32-bit float 数组。
//! - 每个 f32 占 4 字节
//! - 总字节数 = 维度数 × 4
//! - 大端序 / 小端序固定用小端（x86/ARM-LE 都默认小端，且 `f32::to_le_bytes` 显式转换）
//!
//! 这种格式紧凑（1536 维 = 6144 字节），无需任何 envelope / 头部。
//! 解码用 `f32::from_le_bytes(bytes.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect())`。
//!
//! ## cosine 相似度公式
//!
//! ```text
//! cosine(a, b) = dot(a, b) / (||a|| * ||b||)
//! ```
//!
//! - 任一向量为零向量时返回 0.0（避免除零 / 避免「无意义高相似度」）
//! - 两个向量维度不匹配时返回 0.0（视为不匹配而非报错 ——
//!   recall 时可能遇到「跨模型嵌入维度不同」的情况）

use sqlx::{Row, SqlitePool};

use crate::error::{AppError, AppResult};

// =========================================================================
// BLOB ↔ Vec<f32> 编解码
// =========================================================================

/// 把 `Vec<f32>` 序列化为 little-endian 字节流（每个 f32 占 4 字节）
pub fn encode_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for f in embedding {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

/// 从 BLOB 反序列化为 `Vec<f32>`
///
/// - 字节数不是 4 的倍数 → 返回 `AppError::Validation`（损坏数据）
/// - 空 BLOB → 返回空 `Vec<f32>`
pub fn decode_embedding(bytes: &[u8]) -> AppResult<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return Err(AppError::Validation(format!(
            "embedding BLOB 长度 {} 不是 4 的倍数（损坏数据）",
            bytes.len()
        )));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let arr: [u8; 4] = [chunk[0], chunk[1], chunk[2], chunk[3]];
        out.push(f32::from_le_bytes(arr));
    }
    Ok(out)
}

/// 从 BLOB 反序列化为 `Vec<f32>`，并把 `content` 一并返回（避免上层两次查询）
///
/// 返回元组 `(id, content, embedding)`，按 `created_at ASC, id ASC` 排序
/// （确保多次 recall 的顺序稳定 —— 不依赖 BLOB 物理顺序）
pub async fn load_embeddings_for_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> AppResult<Vec<(String, String, Vec<f32>)>> {
    let rows = sqlx::query(
        "SELECT id, content, embedding
           FROM memory_embeddings
          WHERE agent_id = ?
          ORDER BY created_at ASC, id ASC",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id")?;
        let content: String = row.try_get("content")?;
        let bytes: Vec<u8> = row.try_get("embedding")?;
        let embedding = decode_embedding(&bytes)?;
        out.push((id, content, embedding));
    }
    Ok(out)
}

/// 插入一条 embedding 记录
///
/// - `id`         调用方生成（uuid）
/// - `agent_id`   所属 agent
/// - `content`    原文（检索命中时返回）
/// - `embedding`  已通过 `embed()` 获取的向量
pub async fn insert_embedding(
    pool: &SqlitePool,
    id: &str,
    agent_id: &str,
    content: &str,
    embedding: &[f32],
) -> AppResult<()> {
    let bytes = encode_embedding(embedding);
    sqlx::query(
        "INSERT INTO memory_embeddings (id, agent_id, content, embedding)
         VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(agent_id)
    .bind(content)
    .bind(&bytes)
    .execute(pool)
    .await?;
    Ok(())
}

/// 删除某 agent 的全部 embedding 记录（清空记忆）
///
/// @returns 实际删除的行数（用于日志 / 调试）
pub async fn delete_embeddings_for_agent(pool: &SqlitePool, agent_id: &str) -> AppResult<u64> {
    let affected = sqlx::query("DELETE FROM memory_embeddings WHERE agent_id = ?")
        .bind(agent_id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected)
}

/// 统计某 agent 的 embedding 记录数
#[cfg(test)]
pub async fn count_embeddings_for_agent(pool: &SqlitePool, agent_id: &str) -> AppResult<i64> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM memory_embeddings WHERE agent_id = ?")
            .bind(agent_id)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

// =========================================================================
// Cosine 相似度
// =========================================================================

/// 计算两个等长 `&[f32]` 的 cosine 相似度
///
/// 返回值范围 `[-1.0, 1.0]`：
/// - `1.0`  完全相同方向
/// - `0.0`  正交
/// - `-1.0` 完全相反
///
/// ## 边界情况
///
/// - 任一为空 → 返回 `0.0`（视为「无意义」）
/// - 维度不匹配 → 返回 `0.0`（视为「不匹配」而非报错 —— recall 时跨模型可能发生）
/// - 任一向量为零向量 → 返回 `0.0`（避免除零，避免「无意义高相似度」）
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    if a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let xf = *x as f64;
        let yf = *y as f64;
        dot += xf * yf;
        norm_a += xf * xf;
        norm_b += yf * yf;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        return 0.0;
    }
    (dot / denom) as f32
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- BLOB 编解码 ----

    #[test]
    fn encode_decode_roundtrip() {
        let v: Vec<f32> = vec![0.0, 1.0, -1.0, 0.5, 1.5e-3, 1.234];
        let bytes = encode_embedding(&v);
        // 字节数 = 维度 × 4
        assert_eq!(bytes.len(), v.len() * 4);
        let decoded = decode_embedding(&bytes).unwrap();
        // f32 精度下 roundtrip 应 bit-identical
        assert_eq!(decoded.len(), v.len());
        for (a, b) in v.iter().zip(decoded.iter()) {
            assert_eq!(a.to_bits(), b.to_bits(), "roundtrip 失真: {a} != {b}");
        }
    }

    #[test]
    fn encode_decode_empty() {
        let bytes = encode_embedding(&[]);
        assert!(bytes.is_empty());
        let decoded = decode_embedding(&bytes).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn decode_rejects_misaligned_length() {
        // 5 字节 → 不是 4 的倍数 → 应返回 Validation 错误
        let bytes = vec![0u8; 5];
        let err = decode_embedding(&bytes).unwrap_err();
        match err {
            AppError::Validation(msg) => {
                assert!(msg.contains("embedding"), "msg: {msg}");
                assert!(msg.contains("不是 4 的倍数"), "msg: {msg}");
            }
            other => panic!("应返回 Validation，实际: {other:?}"),
        }
    }

    #[test]
    fn encode_produces_little_endian() {
        // f32::from_bits(0x3F800000) = 1.0，LE 字节序列 = [0x00, 0x00, 0x80, 0x3F]
        let bytes = encode_embedding(&[1.0]);
        assert_eq!(bytes, vec![0x00, 0x00, 0x80, 0x3F]);
    }

    // ---- cosine_similarity ----

    #[test]
    fn cosine_identical_vectors_returns_one() {
        let v = vec![1.0, 2.0, 3.0, 4.0];
        let sim = cosine_similarity(&v, &v);
        assert!(
            (sim - 1.0).abs() < 1e-6,
            "相同向量 cosine 应为 1.0，实际 {sim}"
        );
    }

    #[test]
    fn cosine_orthogonal_vectors_returns_zero() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6, "正交向量 cosine 应为 0.0，实际 {sim}");
    }

    #[test]
    fn cosine_opposite_vectors_returns_minus_one() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim + 1.0).abs() < 1e-6,
            "相反方向 cosine 应为 -1.0，实际 {sim}"
        );
    }

    #[test]
    fn cosine_scale_invariant() {
        // 向量缩放后 cosine 不变
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![2.0, 4.0, 6.0]; // = 2 * a
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim - 1.0).abs() < 1e-6,
            "缩放向量 cosine 应为 1.0，实际 {sim}"
        );
    }

    #[test]
    fn cosine_mismatched_dimensions_returns_zero() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0, "维度不匹配应返回 0.0 而不是错误，实际 {sim}");
    }

    #[test]
    fn cosine_empty_vectors_returns_zero() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn cosine_zero_vector_returns_zero() {
        // 任一向量为零向量 → 应返回 0.0（避免除零 / 无意义高相似度）
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0);
    }

    #[test]
    fn cosine_negative_values() {
        // 含负值的相似度计算（OpenAI embedding 会出现负值）
        let a = vec![1.0, -2.0, 3.0];
        let b = vec![-1.0, 2.0, -3.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6, "a = -b 应为 -1.0，实际 {sim}");
    }

    #[test]
    fn cosine_high_dimensional_consistency() {
        // 模拟真实 embedding 维度（1536 维）
        let a: Vec<f32> = (0..1536).map(|i| (i as f32).sin()).collect();
        let mut b: Vec<f32> = (0..1536).map(|i| (i as f32).cos()).collect();
        let sim_ab = cosine_similarity(&a, &b);

        // 改 b 一个微小的元素，similarity 应有变化但不剧烈
        b[100] += 0.01;
        let sim_ab2 = cosine_similarity(&a, &b);
        assert!(
            (sim_ab - sim_ab2).abs() < 1.0,
            "高维向量微调应产生有限变化: {sim_ab} vs {sim_ab2}"
        );
        // 验证计算未 panic / 未产生 NaN
        assert!(!sim_ab.is_nan());
        assert!(!sim_ab2.is_nan());
        // 两个值都在 [-1, 1]
        assert!((-1.0..=1.0).contains(&sim_ab));
        assert!((-1.0..=1.0).contains(&sim_ab2));

        // 与自身比 → 1.0
        let sim_self = cosine_similarity(&a, &a);
        assert!((sim_self - 1.0).abs() < 1e-6);
    }

    // ---- DB 集成测试 ----

    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite")
    }

    #[tokio::test]
    async fn insert_and_load_embeddings() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();

        let emb1 = vec![1.0, 0.0, 0.0];
        let emb2 = vec![0.0, 1.0, 0.0];
        insert_embedding(&pool, "m1", "agent-a", "doc1", &emb1)
            .await
            .unwrap();
        insert_embedding(&pool, "m2", "agent-a", "doc2", &emb2)
            .await
            .unwrap();

        let rows = load_embeddings_for_agent(&pool, "agent-a").await.unwrap();
        assert_eq!(rows.len(), 2);
        // 按 created_at ASC, id ASC 排序 → m1 在前
        assert_eq!(rows[0].0, "m1");
        assert_eq!(rows[0].1, "doc1");
        assert_eq!(rows[0].2, emb1);
        assert_eq!(rows[1].0, "m2");
        assert_eq!(rows[1].2, emb2);
    }

    #[tokio::test]
    async fn load_filters_by_agent_id() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();

        let emb = vec![1.0, 0.0];
        insert_embedding(&pool, "m1", "agent-a", "doc-a", &emb)
            .await
            .unwrap();
        insert_embedding(&pool, "m2", "agent-b", "doc-b", &emb)
            .await
            .unwrap();

        let rows_a = load_embeddings_for_agent(&pool, "agent-a").await.unwrap();
        let rows_b = load_embeddings_for_agent(&pool, "agent-b").await.unwrap();
        let rows_c = load_embeddings_for_agent(&pool, "agent-c").await.unwrap();

        assert_eq!(rows_a.len(), 1);
        assert_eq!(rows_a[0].0, "m1");
        assert_eq!(rows_b.len(), 1);
        assert_eq!(rows_b[0].0, "m2");
        assert!(rows_c.is_empty());
    }

    #[tokio::test]
    async fn delete_embeddings_for_agent_clears_all() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();

        let emb = vec![1.0, 0.0];
        insert_embedding(&pool, "m1", "agent-a", "d1", &emb)
            .await
            .unwrap();
        insert_embedding(&pool, "m2", "agent-a", "d2", &emb)
            .await
            .unwrap();
        insert_embedding(&pool, "m3", "agent-b", "d3", &emb)
            .await
            .unwrap();

        let affected = delete_embeddings_for_agent(&pool, "agent-a").await.unwrap();
        assert_eq!(affected, 2);

        let rows_a = load_embeddings_for_agent(&pool, "agent-a").await.unwrap();
        assert!(rows_a.is_empty());
        // agent-b 不受影响
        let rows_b = load_embeddings_for_agent(&pool, "agent-b").await.unwrap();
        assert_eq!(rows_b.len(), 1);
    }

    #[tokio::test]
    async fn count_embeddings_for_agent_returns_correct_count() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();

        assert_eq!(
            count_embeddings_for_agent(&pool, "agent-x").await.unwrap(),
            0
        );

        let emb = vec![1.0, 0.0];
        insert_embedding(&pool, "m1", "agent-x", "d1", &emb)
            .await
            .unwrap();
        insert_embedding(&pool, "m2", "agent-x", "d2", &emb)
            .await
            .unwrap();

        assert_eq!(
            count_embeddings_for_agent(&pool, "agent-x").await.unwrap(),
            2
        );
    }

    /// REQ-CHAT-047：端到端 cosine 检索 —— 插入 N 条 → 计算相似度 →
    /// 验证 top-5 + 阈值过滤的行为
    #[tokio::test]
    async fn cosine_top5_threshold_filter() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();

        // query: [1.0, 0.0, 0.0]
        // docs:
        //   doc1 = [1.0, 0.0, 0.0] → cosine 1.0   (✓ 强相关)
        //   doc2 = [0.9, 0.1, 0.0] → cosine ~0.99 (✓ 强相关)
        //   doc3 = [0.5, 0.5, 0.0] → cosine ~0.71 (✓ 阈值边缘)
        //   doc4 = [0.0, 1.0, 0.0] → cosine 0.0   (✗ 不相关)
        //   doc5 = [0.0, 0.0, 1.0] → cosine 0.0   (✗ 不相关)
        //   doc6 = [1.0, 0.0, 0.0] → cosine 1.0   (✓ 强相关)
        let docs: Vec<(&str, Vec<f32>)> = vec![
            ("d1", vec![1.0, 0.0, 0.0]),
            ("d2", vec![0.9, 0.1, 0.0]),
            ("d3", vec![0.5, 0.5, 0.0]),
            ("d4", vec![0.0, 1.0, 0.0]),
            ("d5", vec![0.0, 0.0, 1.0]),
            ("d6", vec![1.0, 0.0, 0.0]),
        ];
        for (i, (id, v)) in docs.iter().enumerate() {
            insert_embedding(&pool, id, "agent-q", &format!("doc-{i}"), v)
                .await
                .unwrap();
        }

        let query = vec![1.0, 0.0, 0.0];
        let rows = load_embeddings_for_agent(&pool, "agent-q").await.unwrap();

        // 阈值过滤 + 按相似度排序
        let threshold = 0.7_f32;
        let mut scored: Vec<(f32, &str)> = rows
            .iter()
            .map(|(_, content, emb)| (cosine_similarity(&query, emb), content.as_str()))
            .filter(|(s, _)| *s >= threshold)
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // 取 top-5
        let top5: Vec<&str> = scored.iter().take(5).map(|(_, c)| *c).collect();

        // 验证：d1、d2、d3、d6 命中（d4、d5 不命中）
        assert_eq!(top5.len(), 4);
        assert!(top5.contains(&"doc-0"));
        assert!(top5.contains(&"doc-1"));
        assert!(top5.contains(&"doc-2"));
        assert!(top5.contains(&"doc-5"));
    }
}
