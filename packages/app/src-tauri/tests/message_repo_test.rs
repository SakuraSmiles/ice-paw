//! 消息仓库（`repo::message`）的回归测试
//!
//! 重点覆盖「消息顺序反转」bug：
//! 用户连续发送时，`send_message` 在 `chat_cmd` 内先后 INSERT 用户消息
//! 与助手占位——这两次写入通常落在同一秒（SQLite `datetime('now')`
//! 是秒级精度），旧的 `ORDER BY created_at DESC, id DESC` 兜底排序
//! 会以随机 UUID 做 tie-breaker，约 50% 概率把助手排到用户之前，
//! 再被 `rows.reverse()` 反转后变成「助手先、用户后」——
//! 表现为 `用户 → AI → AI → 用户`。
//!
//! 这些测试构造真实的同秒 `created_at` 场景，验证修复后顺序稳定。
//!
//! 运行：`cargo test --test message_repo_test`

use ice_paw_lib::db::models::NewMessage;
use ice_paw_lib::db::repo::{self, agent, conversation, message};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

/// 构造一个独立的 in-memory SQLite 连接池 + 跑迁移。
async fn fresh_pool() -> SqlitePool {
    // 用一个**唯一**的内存库名：`:memory:` 在 SqlitePoolOptions 下
    // 会被多连接共享，但 sqlx 默认 `pool.max_connections(1)` 时最稳。
    // 共享内存的语法是 `file::memory:?cache=shared`，简单起见用
    // `:memory:` + 1 连接——每个测试拿独立池即可。
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("valid sqlite url")
        .create_if_missing(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .min_connections(1)
        .connect_with(opts)
        .await
        .expect("connect to in-memory sqlite");

    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    pool
}

/// 写入一个 agent（messages.conversation_id 需引用 conversations.id，
/// 间接要求 agent 存在；测试方便起见先建一个）。
async fn seed_agent_and_conv(pool: &SqlitePool) -> String {
    let agent_id = "agent-001";
    agent::create(
        pool,
        &ice_paw_lib::db::models::NewAgent {
            name: "test-agent".into(),
            provider: "openai".into(),
            model: "gpt-4o-mini".into(),
            system_prompt: "".into(),
            api_key: "test-key".into(),
            base_url: None,
            temperature: 0.7,
            max_tokens: 1024,
            extra_params: None,
            sort_order: 0,
        },
        agent_id,
        agent_id, // api_key_ref 简化为 agent_id 自身
    )
    .await
    .expect("create agent");

    let conv_id = "conv-001";
    conversation::create(
        pool,
        conv_id,
        &ice_paw_lib::db::models::NewConversation {
            agent_id: agent_id.to_string(),
            title: Some("test".into()),
        },
    )
    .await
    .expect("create conversation");

    conv_id.to_string()
}

/// 强行把某条消息的 `created_at` 覆写为指定时间字符串。
///
/// 测试需要构造「多条消息共享同一秒」的场景。`repo::message::create`
/// 走 DB DEFAULT `datetime('now')`，无法注入；因此 INSERT 后用
/// `UPDATE` 把每条都压到同一个时间戳，模拟同秒写入。
async fn force_created_at(pool: &SqlitePool, id: &str, ts: &str) {
    sqlx::query("UPDATE messages SET created_at = ? WHERE id = ?")
        .bind(ts)
        .bind(id)
        .execute(pool)
        .await
        .expect("force created_at");
}

// ---------------------------------------------------------------------------
// 主回归：同秒内 user → assistant 必须按插入顺序返回
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_by_conversation_preserves_insert_order_within_same_second() {
    let pool = fresh_pool().await;
    let conv_id = seed_agent_and_conv(&pool).await;

    // 模拟一次完整的 user → assistant 交换：
    //   INSERT user_msg (T0)
    //   INSERT assistant_msg (T0)   // 与 user 同一秒（DATETIME 默认精度）
    //   UPDATE assistant_msg.content = "..."（流式结束回写，但 created_at 不变）
    //
    // 然后再来一轮：
    //   INSERT user_msg2 (T1, 1 秒后)
    //   INSERT assistant_msg2 (T1)
    let user1 = "11111111-1111-1111-1111-111111111111";
    let asst1 = "22222222-2222-2222-2222-222222222222";
    let user2 = "33333333-3333-3333-3333-333333333333";
    let asst2 = "44444444-4444-4444-4444-444444444444";

    message::create(
        &pool,
        user1,
        &NewMessage {
            conversation_id: conv_id.clone(),
            role: "user".into(),
            content: "你好".into(),
            token_count: None,
            error: None,
        },
    )
    .await
    .unwrap();
    message::create(
        &pool,
        asst1,
        &NewMessage {
            conversation_id: conv_id.clone(),
            role: "assistant".into(),
            content: String::new(),
            token_count: None,
            error: None,
        },
    )
    .await
    .unwrap();

    // 模拟流式结束后回写内容（created_at 不变）
    message::update_content(&pool, asst1, "你好世界").await.unwrap();

    message::create(
        &pool,
        user2,
        &NewMessage {
            conversation_id: conv_id.clone(),
            role: "user".into(),
            content: "今天天气如何".into(),
            token_count: None,
            error: None,
        },
    )
    .await
    .unwrap();
    message::create(
        &pool,
        asst2,
        &NewMessage {
            conversation_id: conv_id.clone(),
            role: "assistant".into(),
            content: String::new(),
            token_count: None,
            error: None,
        },
    )
    .await
    .unwrap();
    message::update_content(&pool, asst2, "晴，25°C").await.unwrap();

    // 把同对的 user+assistant 压到同一秒，模拟 SQLite 默认精度的最坏情况
    let same_second_t0 = "2026-07-13 17:11:23";
    let same_second_t1 = "2026-07-13 17:11:24";
    force_created_at(&pool, user1, same_second_t0).await;
    force_created_at(&pool, asst1, same_second_t0).await;
    force_created_at(&pool, user2, same_second_t1).await;
    force_created_at(&pool, asst2, same_second_t1).await;

    let rows = message::list_by_conversation(&pool, &conv_id, None, None)
        .await
        .unwrap();

    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![user1, asst1, user2, asst2],
        "同秒写入的 user/assistant 对必须按插入顺序返回；\
         若出现反转，bug 已回归。"
    );
}

// ---------------------------------------------------------------------------
// 交叉轮：N 轮 user/assistant 全部同秒
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_by_conversation_handles_many_pairs_all_same_second() {
    let pool = fresh_pool().await;
    let conv_id = seed_agent_and_conv(&pool).await;

    // 连续 10 轮，每轮 user+assistant 共享一个时间戳（极坏情况）
    let mut expected_ids: Vec<String> = Vec::new();
    for i in 0..10 {
        let u = format!("user-pair-{i:02}-u");
        let a = format!("user-pair-{i:02}-a");
        message::create(
            &pool,
            &u,
            &NewMessage {
                conversation_id: conv_id.clone(),
                role: "user".into(),
                content: format!("q{i}"),
                token_count: None,
                error: None,
            },
        )
        .await
        .unwrap();
        message::create(
            &pool,
            &a,
            &NewMessage {
                conversation_id: conv_id.clone(),
                role: "assistant".into(),
                content: format!("a{i}"),
                token_count: None,
                error: None,
            },
        )
        .await
        .unwrap();
        force_created_at(&pool, &u, "2026-07-13 17:11:23").await;
        force_created_at(&pool, &a, "2026-07-13 17:11:23").await;
        expected_ids.push(u);
        expected_ids.push(a);
    }

    let rows = message::list_by_conversation(&pool, &conv_id, None, None)
        .await
        .unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let expected_refs: Vec<&str> = expected_ids.iter().map(String::as_str).collect();

    assert_eq!(ids, expected_refs, "10 对 user/assistant 全部同秒时，每对内 user 必须先于 assistant");
}

// ---------------------------------------------------------------------------
// 翻页 `before`：同秒内次序仍要稳
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_by_conversation_with_before_preserves_order_within_second() {
    let pool = fresh_pool().await;
    let conv_id = seed_agent_and_conv(&pool).await;

    // 三对，跨三秒
    let ids = vec![
        ("m-uu-1", "user", "2026-07-13 17:11:23"),
        ("m-aa-1", "assistant", "2026-07-13 17:11:23"),
        ("m-uu-2", "user", "2026-07-13 17:11:24"),
        ("m-aa-2", "assistant", "2026-07-13 17:11:24"),
        ("m-uu-3", "user", "2026-07-13 17:11:25"),
        ("m-aa-3", "assistant", "2026-07-13 17:11:25"),
    ];
    for (id, role, _ts) in &ids {
        message::create(
            &pool,
            id,
            &NewMessage {
                conversation_id: conv_id.clone(),
                role: (*role).into(),
                content: format!("{id}-content"),
                token_count: None,
                error: None,
            },
        )
        .await
        .unwrap();
    }
    for (id, _role, ts) in &ids {
        force_created_at(&pool, id, ts).await;
    }

    // 翻页：取 "2026-07-13 17:11:25" 之前的 → 应是 [1, 2, 3, 4]
    let rows = message::list_by_conversation(&pool, &conv_id, None, Some("2026-07-13 17:11:25"))
        .await
        .unwrap();
    let got: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        got,
        vec!["m-uu-1", "m-aa-1", "m-uu-2", "m-aa-2"],
        "翻页 before 也必须保持同秒内的插入顺序"
    );

    // 取 "2026-07-13 17:11:24" 之前的 → 应是 [1, 2]
    let rows = message::list_by_conversation(&pool, &conv_id, None, Some("2026-07-13 17:11:24"))
        .await
        .unwrap();
    let got: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(got, vec!["m-uu-1", "m-aa-1"]);
}

// ---------------------------------------------------------------------------
// 公共函数存在性：避免命名漂移导致测试假阳性
// ---------------------------------------------------------------------------

#[test]
fn test_helper_imports_compile() {
    // 这个测试本身没断言；目的是让 `repo`、`agent`、`conversation`
    // 三个模块被显式引用，CI 上若有人改了可见性至少能在这里炸出来。
    let _ = (
        repo::message::list_by_conversation as *const () as usize,
        agent::create as *const () as usize,
        conversation::create as *const () as usize,
    );
}
