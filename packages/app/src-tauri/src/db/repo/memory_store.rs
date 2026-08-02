//! `memory_store` 表操作(REQ-CHAT-048 记忆加密存储)
//!
//! 提供:
//! - `insert_encrypted_memory`:加密并写入一条记忆
//! - `load_memories`:加载某 agent 的全部记忆(解密后返回)
//! - `load_latest_memory`:加载某 agent 指定类型的最新一条记忆(解密后返回)
//! - `delete_memories_for_agent`:删除某 agent 的全部记忆
//! - `count_memories_for_agent`:统计某 agent 的记忆条数
//!
//! ## BLOB 编码
//!
//! `content_encrypted` 列存的是加密 BLOB,格式:
//!
//! ```text
//! [nonce: 24 bytes][ciphertext: N bytes][poly1305 tag: 16 bytes]
//! ```
//!
//! 由 [`crate::crypto::encrypt_blob`] 加密;
//! 解密调 [`crate::crypto::decrypt_blob`]。
//!
//! ## 加密方案
//!
//! - **算法**:XChaCha20-Poly1305(与 Stronghold vault 内部一致)
//! - **Key 派生**:blake2b256(`DEFAULT_PASSPHRASE` || `MEMORY_KEY_DOMAIN`)
//! - **Nonce**:每次 encrypt 调 `OsRng` 生成 24 字节随机值,前置到密文
//!
//! 详细说明见 [`crate::crypto`] 顶部注释。

use sqlx::{Row, SqlitePool};

use crate::crypto::{decrypt_blob, encrypt_blob};
use crate::error::{AppError, AppResult};

// =========================================================================
// 写入路径
// =========================================================================

/// 加密 `content` 并写入 `memory_store` 表
///
/// - `pool`         SQLite 连接池
/// - `agent_id`     所属 agent ID(必填)
/// - `content`      明文内容(任意字节序列,典型 UTF-8 文本)
/// - `content_type` 记忆类型(默认 'summary');未来扩展 'fact' / 'note' 等
///
/// @returns 新插入行的 ID(uuid)
pub async fn insert_encrypted_memory(
    pool: &SqlitePool,
    agent_id: &str,
    content: &[u8],
    content_type: &str,
) -> AppResult<String> {
    if agent_id.is_empty() {
        return Err(AppError::Validation("agent_id 不能为空".to_string()));
    }
    if content_type.is_empty() {
        return Err(AppError::Validation("content_type 不能为空".to_string()));
    }

    let encrypted = encrypt_blob(content)?;

    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO memory_store (id, agent_id, content_encrypted, content_type)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(agent_id)
    .bind(&encrypted)
    .bind(content_type)
    .execute(pool)
    .await?;

    Ok(id)
}

// =========================================================================
// 读取路径
// =========================================================================

/// 加载某 agent 的全部记忆(解密后返回)
///
/// 返回元组 `(id, content_type, decrypted_content, created_at)`,按
/// `created_at ASC, id ASC` 排序(多次查询顺序稳定)。
///
/// ## 失败模式
///
/// - 解密失败(数据被篡改 / nonce 错位 / 非本 key 加密)→ 返回 `AppError::Validation`
///   该错误冒泡到调用方:单条记录损坏应被视为「数据完整性问题」而非 silent skip
pub async fn load_memories(
    pool: &SqlitePool,
    agent_id: &str,
) -> AppResult<Vec<(String, String, Vec<u8>, String)>> {
    let rows = sqlx::query(
        "SELECT id, content_type, content_encrypted, created_at
           FROM memory_store
          WHERE agent_id = ?
          ORDER BY created_at ASC, id ASC",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id")?;
        let content_type: String = row.try_get("content_type")?;
        let encrypted: Vec<u8> = row.try_get("content_encrypted")?;
        let created_at: String = row.try_get("created_at")?;
        let decrypted = decrypt_blob(&encrypted)?;
        out.push((id, content_type, decrypted, created_at));
    }
    Ok(out)
}

/// 加载某 agent 指定 `content_type` 的最新一条记忆(解密后)
///
/// 用于 M1.5 摘要复用场景:取该 agent 最新一条 `content_type='summary'` 的记录。
///
/// - 有记录 → `Some(decrypted_content_bytes)`
/// - 无记录 → `None`
///
/// ## 排序稳定性
///
/// 按 `created_at DESC, id DESC` 排序,取 LIMIT 1。`created_at` 使用
/// SQLite `datetime('now')` 精度(秒),同秒内插入的多条记录用 `id DESC`
/// 兜底稳定排序。
pub async fn load_latest_memory(
    pool: &SqlitePool,
    agent_id: &str,
    content_type: &str,
) -> AppResult<Option<Vec<u8>>> {
    let row: Option<(Vec<u8>,)> = sqlx::query_as(
        "SELECT content_encrypted
           FROM memory_store
          WHERE agent_id = ?
            AND content_type = ?
          ORDER BY created_at DESC, id DESC
          LIMIT 1",
    )
    .bind(agent_id)
    .bind(content_type)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((encrypted,)) => {
            let decrypted = decrypt_blob(&encrypted)?;
            Ok(Some(decrypted))
        }
        None => Ok(None),
    }
}

/// 加载某 agent 的全部「摘要」类型记忆(解密为字符串)
///
/// `load_latest_memory` 的字符串便捷版:假设 `content` 是合法 UTF-8,
/// 用 `String::from_utf8` 转换(失败 → `AppError::Validation`)。
///
/// 典型用途:`summary.rs::get_latest_summary` 复用旧摘要时。
pub async fn load_latest_summary_string(
    pool: &SqlitePool,
    agent_id: &str,
) -> AppResult<Option<String>> {
    match load_latest_memory(pool, agent_id, "summary").await? {
        Some(bytes) => {
            let s = String::from_utf8(bytes).map_err(|e| {
                AppError::Validation(
                    format!("memory_store.content 不是合法 UTF-8(agent_id={agent_id}): {e}"),
                )
            })?;
            Ok(Some(s))
        }
        None => Ok(None),
    }
}

// =========================================================================
// 删除 / 统计
// =========================================================================

/// 删除某 agent 的全部记忆(清空该 agent 的长期记忆)
///
/// @returns 实际删除的行数
pub async fn delete_memories_for_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> AppResult<u64> {
    let affected = sqlx::query("DELETE FROM memory_store WHERE agent_id = ?")
        .bind(agent_id)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(affected)
}

/// 统计某 agent 的记忆条数
#[cfg(test)]
pub async fn count_memories_for_agent(
    pool: &SqlitePool,
    agent_id: &str,
) -> AppResult<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memory_store WHERE agent_id = ?",
    )
    .bind(agent_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
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

    /// 种子 agent(外键依赖 -- memory_store.agent_id 不强制外键,但插入
    /// summary_message 时 conversation 表需要 agent)
    async fn seed_agent(pool: &SqlitePool, agent_id: &str) {
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(agent_id)
        .bind(agent_id)
        .bind("anthropic")
        .bind("claude-test")
        .bind("")
        .bind("")
        .bind(0.7)
        .bind(1024)
        .bind("{}")
        .bind(0)
        .bind(0)
        .execute(pool)
        .await
        .expect("seed agent");
    }

    // ---- insert_encrypted_memory / load_memories ----

    #[tokio::test]
    async fn insert_and_load_roundtrip() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-a").await;

        let content = "用户想修改 foo 函数".as_bytes();
        let id = insert_encrypted_memory(&pool, "agent-a", content, "summary")
            .await
            .unwrap();

        // 直接从 DB 取密文,验证不是明文
        let (raw_bytes,): (Vec<u8>,) = sqlx::query_as(
            "SELECT content_encrypted FROM memory_store WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            !raw_bytes.windows(content.len()).any(|w| w == content),
            "密文中不应出现明文"
        );

        // 通过 load_memories 读取,应解密成功
        let rows = load_memories(&pool, "agent-a").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, id);
        assert_eq!(rows[0].1, "summary");
        assert_eq!(rows[0].2, content);
        assert!(!rows[0].3.is_empty(), "created_at 应自动填充");
    }

    #[tokio::test]
    async fn insert_empty_content_succeeds() {
        // 空明文加密 → 仍然产生 nonce (24) + tag (16) = 40 字节密文
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-b").await;

        let id = insert_encrypted_memory(&pool, "agent-b", b"", "summary")
            .await
            .unwrap();
        let rows = load_memories(&pool, "agent-b").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, id);
        assert!(rows[0].2.is_empty(), "空明文 round-trip 仍是空");
    }

    #[tokio::test]
    async fn insert_large_content_succeeds() {
        // 1 MB 明文 → 加密 → 存储 → 解密
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-c").await;

        let content: Vec<u8> = (0..1_000_000).map(|i| (i % 256) as u8).collect();
        let _id = insert_encrypted_memory(&pool, "agent-c", &content, "summary")
            .await
            .unwrap();

        let rows = load_memories(&pool, "agent-c").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2.len(), content.len());
        assert_eq!(rows[0].2, content, "大数据 round-trip 应 bit-identical");
    }

    #[tokio::test]
    async fn load_memories_filters_by_agent() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-d1").await;
        seed_agent(&pool, "agent-d2").await;

        insert_encrypted_memory(&pool, "agent-d1", b"d1-content-1", "summary")
            .await
            .unwrap();
        // 跨秒插入,让 created_at 至少相差 1 秒(SQLite `datetime('now')` 精度)
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        insert_encrypted_memory(&pool, "agent-d1", b"d1-content-2", "summary")
            .await
            .unwrap();
        insert_encrypted_memory(&pool, "agent-d2", b"d2-content", "note")
            .await
            .unwrap();

        let d1 = load_memories(&pool, "agent-d1").await.unwrap();
        assert_eq!(d1.len(), 2);
        // 按 created_at ASC 排序:先插入的 d1-content-1 在前
        assert_eq!(
            d1[0].2, b"d1-content-1",
            "先插入的应在前面(按 created_at ASC)"
        );
        assert_eq!(d1[1].2, b"d1-content-2");

        let d2 = load_memories(&pool, "agent-d2").await.unwrap();
        assert_eq!(d2.len(), 1);
        assert_eq!(d2[0].2, b"d2-content");
        assert_eq!(d2[0].1, "note");

        let d3 = load_memories(&pool, "agent-none").await.unwrap();
        assert!(d3.is_empty());
    }

    // ---- load_latest_memory ----

    #[tokio::test]
    async fn load_latest_returns_most_recent() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-e").await;

        insert_encrypted_memory(&pool, "agent-e", b"old-summary", "summary")
            .await
            .unwrap();
        // SQLite `datetime('now')` 精度为秒,两条同秒插入;
        // 我们的 ORDER BY created_at DESC, id DESC 让 id 大的(后插入的)排前面
        // -- 但我们用 sleep 让 created_at 跨越至少 1 秒,避免靠 id 排序兜底
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        insert_encrypted_memory(&pool, "agent-e", b"new-summary", "summary")
            .await
            .unwrap();
        insert_encrypted_memory(&pool, "agent-e", b"a-note", "note")
            .await
            .unwrap();

        let latest_summary = load_latest_memory(&pool, "agent-e", "summary")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            latest_summary, b"new-summary",
            "应返回 content_type='summary' 的最新一条"
        );

        let latest_note = load_latest_memory(&pool, "agent-e", "note")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest_note, b"a-note");

        // 不存在的 content_type
        let none = load_latest_memory(&pool, "agent-e", "nonexistent")
            .await
            .unwrap();
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn load_latest_returns_none_when_empty() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-f").await;

        let r = load_latest_memory(&pool, "agent-f", "summary")
            .await
            .unwrap();
        assert!(r.is_none());
    }

    // ---- load_latest_summary_string ----

    #[tokio::test]
    async fn load_latest_summary_string_decodes_utf8() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-g").await;

        insert_encrypted_memory(
            &pool,
            "agent-g",
            "用户修改了 foo.rs 的第 42 行".as_bytes(),
            "summary",
        )
        .await
        .unwrap();

        let s = load_latest_summary_string(&pool, "agent-g").await.unwrap();
        assert_eq!(s, Some("用户修改了 foo.rs 的第 42 行".to_string()));
    }

    #[tokio::test]
    async fn load_latest_summary_string_rejects_non_utf8() {
        // 加密二进制内容(非 UTF-8)后用 *_string 读取 → 应报 Validation
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-h").await;

        // 0xFF 0xFE 是非法 UTF-8 起始字节
        insert_encrypted_memory(&pool, "agent-h", &[0xFF, 0xFE, 0xFD], "summary")
            .await
            .unwrap();

        let err = load_latest_summary_string(&pool, "agent-h")
            .await
            .unwrap_err();
        match err {
            AppError::Validation(message) => {
                assert!(message.contains("UTF-8"), "msg 应提到 UTF-8: {message}");
            }
            other => panic!("应返回 Validation,实际: {other:?}"),
        }
    }

    // ---- delete / count ----

    #[tokio::test]
    async fn delete_memories_clears_all_for_agent() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-i1").await;
        seed_agent(&pool, "agent-i2").await;

        insert_encrypted_memory(&pool, "agent-i1", b"i1-1", "summary")
            .await
            .unwrap();
        insert_encrypted_memory(&pool, "agent-i1", b"i1-2", "summary")
            .await
            .unwrap();
        insert_encrypted_memory(&pool, "agent-i2", b"i2-1", "note")
            .await
            .unwrap();

        let affected = delete_memories_for_agent(&pool, "agent-i1")
            .await
            .unwrap();
        assert_eq!(affected, 2);

        assert!(load_memories(&pool, "agent-i1").await.unwrap().is_empty());
        // agent-i2 不受影响
        assert_eq!(load_memories(&pool, "agent-i2").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn count_memories_returns_correct_count() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-j").await;

        assert_eq!(
            count_memories_for_agent(&pool, "agent-j").await.unwrap(),
            0
        );
        insert_encrypted_memory(&pool, "agent-j", b"c1", "summary")
            .await
            .unwrap();
        insert_encrypted_memory(&pool, "agent-j", b"c2", "note")
            .await
            .unwrap();
        insert_encrypted_memory(&pool, "agent-j", b"c3", "fact")
            .await
            .unwrap();
        assert_eq!(
            count_memories_for_agent(&pool, "agent-j").await.unwrap(),
            3
        );
    }

    // ---- 输入校验 ----

    #[tokio::test]
    async fn insert_rejects_empty_agent_id() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();

        let err = insert_encrypted_memory(&pool, "", b"x", "summary")
            .await
            .unwrap_err();
        match err {
            AppError::Validation(message) => {
                assert!(message.contains("agent_id"), "msg: {message}");
            }
            other => panic!("应返回 Validation，实际: {other:?}"),
        }
    }

    #[tokio::test] 
    async fn insert_rejects_empty_content_type() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-k").await;

        let err = insert_encrypted_memory(&pool, "agent-k", b"x", "")
            .await
            .unwrap_err();
        match err {
            AppError::Validation(message) => {
                assert!(message.contains("content_type"), "msg: {message}");
            }
            other => panic!("应返回 Validation,实际: {other:?}"),
        }
    }

    // ---- 篡改检测 ----

    /// REQ-CHAT-048 安全性:密文被篡改时 decrypt 应拒绝(tag 校验失败)
    #[tokio::test]
    async fn tampered_ciphertext_is_rejected() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-l").await;

        let id = insert_encrypted_memory(&pool, "agent-l", b"secret-text", "summary")
            .await
            .unwrap();

        // 直接改密文 BLOB 中的一个字节(nonce 之后)
        let (mut bytes,): (Vec<u8>,) = sqlx::query_as(
            "SELECT content_encrypted FROM memory_store WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(bytes.len() > 25, "密文长度应至少 24+1 字节");
        bytes[25] ^= 0x01; // 翻转 ciphertext 区域第 1 字节
        sqlx::query("UPDATE memory_store SET content_encrypted = ? WHERE id = ?")
            .bind(&bytes)
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();

        // 解密应失败(tag 校验失败 → Validation 错误)
        let err = load_latest_memory(&pool, "agent-l", "summary")
            .await
            .unwrap_err();
        match err {
            AppError::Validation(message) => {
                assert!(message.contains("认证失败"), "msg 应提及认证失败: {message}");
            }
            other => panic!("篡改密文应返回 Validation,实际: {other:?}"),
        }
    }

    /// REQ-CHAT-048 安全性:nonce 区域被翻转也应被拒绝
    #[tokio::test]
    async fn tampered_nonce_is_rejected() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_agent(&pool, "agent-m").await;

        let id = insert_encrypted_memory(&pool, "agent-m", b"hi", "summary")
            .await
            .unwrap();

        let (mut bytes,): (Vec<u8>,) = sqlx::query_as(
            "SELECT content_encrypted FROM memory_store WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
        bytes[0] ^= 0x80; // 翻转 nonce 第 0 字节
        sqlx::query("UPDATE memory_store SET content_encrypted = ? WHERE id = ?")
            .bind(&bytes)
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();

        let err = load_latest_memory(&pool, "agent-m", "summary")
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::Validation(_)),
            "nonce 篡改应返回 Validation,实际: {err:?}"
        );
    }

    /// 加密是确定性的吗?不 -- 每次 encrypt 都用随机 nonce,因此同样明文
    /// 加密两次应得到不同密文(这是 AEAD 安全性的基本要求)。
    #[tokio::test]
    async fn encrypt_uses_random_nonce_each_time() {
        let content = b"same input";
        let ct1 = crate::crypto::encrypt_blob(content).unwrap();
        let ct2 = crate::crypto::encrypt_blob(content).unwrap();

        // 前 24 字节(nonce)应不同
        assert_ne!(
            &ct1[..24],
            &ct2[..24],
            "nonce 应随机:两次加密应产生不同 nonce"
        );
        // 之后字节也可能不同(虽然 XChaCha20 相同 key+nonce+pt 输出相同 ct+tag,
        // 但 nonce 不同 → ct 与 tag 都不同)
        assert_ne!(ct1, ct2, "不同 nonce 应产生不同密文");

        // 但两个密文都能解密出原文
        assert_eq!(crate::crypto::decrypt_blob(&ct1).unwrap(), content);
        assert_eq!(crate::crypto::decrypt_blob(&ct2).unwrap(), content);
    }

    /// REQ-CHAT-048:解密密文长度不足时返回 Validation(不是 panic)
    #[tokio::test]
    async fn decrypt_rejects_short_input() {
        // 24+16-1 = 39 字节,少于最小长度
        let short = vec![0u8; 39];
        let err = crate::crypto::decrypt_blob(&short).unwrap_err();
        match err {
            AppError::Validation(message) => {
                assert!(message.contains("长度"), "msg: {message}");
            }
            other => panic!("应返回 Validation,实际: {other:?}"),
        }

        // 刚好 40 字节(24 nonce + 16 tag + 0 ct)-- nonce OK 但 tag 校验必失败
        let just_min = vec![0u8; 40];
        let err2 = crate::crypto::decrypt_blob(&just_min).unwrap_err();
        assert!(
            matches!(err2, AppError::Validation(_)),
            "最小长度但 tag 不匹配应返回 Validation"
        );
    }
}