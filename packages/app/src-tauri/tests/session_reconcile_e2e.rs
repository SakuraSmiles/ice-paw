//! session-events 对账端到端测试（session-event-log Phase 1）
//!
//! 一致脚本（**生产事件序** + 真实 repo 写行）对账应零 diff；篡改三连各自命中。
//! 这是 Phase 2（事件日志转唯一真相源）的前置闸门：对账长期全绿之前不动读路径。
//!
//! 与单元测试（harness/reconcile.rs）的差异：这里走**生产持久化路径**——
//! `repo::message::create` 建行 + `update_content`/`update_content_blocks` 回写
//! （模拟 chat_cmd 占位 → loop_engine finalize），事件走 typed emitters，
//! 事件序照 chat_cmd 实际接线（user_message → attachment_stored → turn_context，
//! **非** Phase 0 e2e 的 turn_context 先行）。
//!
//! 运行：`cargo test --test session_reconcile_e2e`

use std::str::FromStr;

use ice_paw_lib::db::models::NewMessage;
use ice_paw_lib::db::repo;
use ice_paw_lib::harness::event_log::{
    log_assistant_message, log_attachment_stored, log_tool_execution, log_tool_result_message,
    log_turn_context, log_turn_ended, log_user_message, AssistantMessagePayload,
    AttachmentPageItem, AttachmentStoredPayload, EventCtx, TurnContextPayload, TurnEndedPayload,
    UserMessagePayload,
};
use ice_paw_lib::harness::reconcile::{reconcile_session, ReconcileReport};
use ice_paw_lib::infra::protocol::ContentBlock;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

// =========================================================================
// 基建
// =========================================================================

/// 注：in-memory SQLite 每连接各一个库，pool 必须 max_connections(1)。
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

async fn seeded_pool() -> SqlitePool {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    sqlx::query(
        "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref,
             temperature, max_tokens, extra_params, sort_order, cache_prompt)
         VALUES ('agent-1', 'e2e', 'anthropic', 'glm-5.2', '', '', 0.7, 1024, '{}', 0, 0)",
    )
    .execute(&pool)
    .await
    .expect("seed agent");
    sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES ('conv-e', 'agent-1', 'e2e')")
        .execute(&pool)
        .await
        .expect("seed conversation");
    pool
}

/// 生产写路径：先 create 占位行，流式结束后回写 content + blocks。
async fn write_row(pool: &SqlitePool, id: &str, role: &str, content: &str, blocks: &[ContentBlock]) {
    repo::message::create(
        pool,
        id,
        &NewMessage {
            conversation_id: "conv-e".to_string(),
            role: role.to_string(),
            content: content.to_string(),
            token_count: None,
            error: None,
            model: None,
        },
    )
    .await
    .expect("create message row");
    let blocks_json = serde_json::to_string(blocks).expect("serialize blocks");
    repo::message::update_content_blocks(pool, id, &blocks_json)
        .await
        .expect("update blocks");
}

/// 1x1 PNG（最小合法图，验证 Image 块 rowid 行↔事件 双侧往返一致）。
const TINY_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// 一致脚本：一个完整 turn（用户带图+附件 → 工具调用 → 工具结果 → 终答），
/// 行与事件按生产路径/生产序写入，两侧严格镜像。
async fn script_consistent_turn(pool: &SqlitePool) {
    // 生产序：turn_id == user_msg_id（EventCtx 语义）
    let ev = EventCtx::new("conv-e", "turn-1", "agent-1");

    let user_blocks = vec![
        ContentBlock::text("看这张图，再读一下 README"),
        ContentBlock::Image {
            data: TINY_PNG.to_string(),
            media_type: "image/png".to_string(),
        },
    ];
    // 行：chat_cmd 先 create（content=pre-materialize 文本）再回写 blocks
    write_row(pool, "turn-1", "user", "看这张图，再读一下 README", &user_blocks).await;
    log_user_message(
        pool,
        &ev,
        "turn-1",
        &UserMessagePayload {
            v: 1,
            content: "看这张图，再读一下 README".into(),
            blocks: user_blocks.clone(),
        },
    )
    .await;
    log_attachment_stored(
        pool,
        &ev,
        "turn-1",
        &AttachmentStoredPayload::Pages {
            v: 1,
            items: vec![AttachmentPageItem {
                idx: 0,
                name: "spec.pdf".into(),
                kind: "application/pdf".into(),
                label: "spec.pdf (第1/3页)".into(),
                token_est: 812,
            }],
        },
    )
    .await;
    log_turn_context(
        pool,
        &ev,
        &TurnContextPayload {
            v: 1,
            provider: "anthropic".into(),
            effective_model: "glm-5.2".into(),
            model_override: None,
            tools_enabled: true,
            tool_names: vec!["read_file".into()],
            temperature: Some(0.7),
            max_tokens: Some(16_384),
            tool_max_rounds: Some(12),
            budget_max_tokens: None,
            context_window: None,
        },
    )
    .await;

    // round 0：assistant 文本 + tool_use
    let a1_blocks = vec![
        ContentBlock::text("我来看一下"),
        ContentBlock::ToolUse {
            id: "tu_1".into(),
            name: "read_file".into(),
            input: "{\"path\":\"README.md\"}".into(),
        },
    ];
    write_row(pool, "msg-a1", "assistant", "我来看一下", &a1_blocks).await;
    log_assistant_message(
        pool,
        &ev,
        "msg-a1",
        &AssistantMessagePayload {
            v: 1,
            model: Some("glm-5.2".into()),
            content: "我来看一下".into(),
            blocks: a1_blocks.clone(),
            token_count: Some(12),
            round: 0,
            continuation: false,
        },
    )
    .await;
    log_tool_execution(
        pool, &ev, "msg-a1", "tc-1", Some("tu_1"), "read_file",
        "{\"path\":\"README.md\"}", Some("# IcePaw\n本地优先..."), false, 34,
    )
    .await;

    // 工具结果行：content 恒空、只写 blocks（loop_engine 阶段 F 语义）
    let tr_blocks = vec![ContentBlock::ToolResult {
        tool_use_id: "tu_1".into(),
        content: "# IcePaw\n本地优先...".into(),
        is_error: Some(false),
    }];
    write_row(pool, "msg-tr1", "user", "", &tr_blocks).await;
    log_tool_result_message(pool, &ev, "msg-tr1", &tr_blocks).await;

    // round 1：终答
    let a2_blocks = vec![ContentBlock::text("README 说这是本地优先的 LLM 工作站。")];
    write_row(pool, "msg-a2", "assistant", "README 说这是本地优先的 LLM 工作站。", &a2_blocks).await;
    log_assistant_message(
        pool,
        &ev,
        "msg-a2",
        &AssistantMessagePayload {
            v: 1,
            model: Some("glm-5.2".into()),
            content: "README 说这是本地优先的 LLM 工作站。".into(),
            blocks: a2_blocks.clone(),
            token_count: Some(20),
            round: 1,
            continuation: false,
        },
    )
    .await;
    log_turn_ended(
        pool,
        &ev,
        Some("msg-a2"),
        &TurnEndedPayload {
            v: 1,
            termination: "stop".into(),
            rounds: 2,
            usage: None,
            user_token_count: Some(3_000),
        },
    )
    .await;
}

fn categories(report: &ReconcileReport) -> Vec<&'static str> {
    report.diffs.iter().map(|d| d.category).collect()
}

// =========================================================================
// 测试 1：一致脚本 → 零 diff（完成判据的地基）
// =========================================================================

#[tokio::test]
async fn consistent_script_reconciles_with_zero_diffs() {
    let pool = seeded_pool().await;
    script_consistent_turn(&pool).await;

    let report = reconcile_session(&pool, "conv-e").await.expect("reconcile");
    assert!(
        report.diffs.is_empty(),
        "两侧镜像写入的 turn 对账必须零 diff：\n{:#?}",
        report.diffs
    );
    assert_eq!(report.events_total, 8);
    assert_eq!(report.turns_total, 1);
    assert_eq!(report.turns_compared, 1);
    assert_eq!(report.legacy_rows_compared, 4, "user/a1/tr1/a2");
    assert_eq!(report.derived_messages_compared, 4);
    // Image 块在两侧（行 content_blocks ↔ 事件 payload）往返一致，无容忍项兜底
    assert!(
        report.skipped.is_empty(),
        "一致数据不应有 skipped：{:#?}",
        report.skipped
    );
}

// =========================================================================
// 测试 2：篡改三连——每类 diff 各自命中（差异即 bug 清单的探测器验证）
// =========================================================================

/// 篡改①：抹掉终答事件（行保留）→ MISSING_IN_DERIVED
#[tokio::test]
async fn tamper_delete_event_fires_missing_in_derived() {
    let pool = seeded_pool().await;
    script_consistent_turn(&pool).await;
    sqlx::query("DELETE FROM session_events WHERE kind='assistant_message' AND message_id='msg-a2'")
        .execute(&pool)
        .await
        .unwrap();

    let report = reconcile_session(&pool, "conv-e").await.expect("reconcile");
    assert_eq!(categories(&report), vec!["MISSING_IN_DERIVED"], "{:#?}", report.diffs);
    assert_eq!(report.diffs[0].message_id.as_deref(), Some("msg-a2"));
}

/// 篡改②：改行 blocks（事件保留）→ CONTENT_MISMATCH
#[tokio::test]
async fn tamper_row_blocks_fires_content_mismatch() {
    let pool = seeded_pool().await;
    script_consistent_turn(&pool).await;
    sqlx::query(
        "UPDATE messages SET content_blocks = '[{\"type\":\"text\",\"text\":\"被篡改\"}]'
          WHERE id = 'msg-a2'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let report = reconcile_session(&pool, "conv-e").await.expect("reconcile");
    assert_eq!(categories(&report), vec!["CONTENT_MISMATCH"], "{:#?}", report.diffs);
    assert!(report.diffs[0].detail.contains("blocks"), "{}", report.diffs[0].detail);
}

/// 篡改③：删行（事件保留）→ MISSING_IN_LEGACY
#[tokio::test]
async fn tamper_delete_row_fires_missing_in_legacy() {
    let pool = seeded_pool().await;
    script_consistent_turn(&pool).await;
    sqlx::query("DELETE FROM messages WHERE id = 'msg-a2'")
        .execute(&pool)
        .await
        .unwrap();

    let report = reconcile_session(&pool, "conv-e").await.expect("reconcile");
    assert_eq!(categories(&report), vec!["MISSING_IN_LEGACY"], "{:#?}", report.diffs);
    assert_eq!(report.diffs[0].message_id.as_deref(), Some("msg-a2"));
}

// =========================================================================
// 真机对账手验入口（非 CI；Phase 2 长期对账监控的雏形）
//
// 对真实 dev/生产库跑完整对账，打印报告（--nocapture）。
// reconcile 只执行 SELECT，不写库；app 运行中也可安全执行。
//
//     ICEPAW_RECONCILE_DB="C:\Users\<u>\AppData\Roaming\com.icepaw.app\ice-paw.db" \
//     ICEPAW_RECONCILE_CONV=<conversation-id> \
//     cargo test --test session_reconcile_e2e reconcile_real_db -- --ignored --nocapture
// =========================================================================

#[tokio::test]
#[ignore = "需真实库路径 + 会话 id，显式触发"]
async fn reconcile_real_db() {
    let db_path = std::env::var("ICEPAW_RECONCILE_DB").expect("设 ICEPAW_RECONCILE_DB=<db 路径>");
    let conv_id = std::env::var("ICEPAW_RECONCILE_CONV").expect("设 ICEPAW_RECONCILE_CONV=<会话 id>");

    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{db_path}"))
        .expect("valid sqlite url")
        .read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect real db（app 运行中占用 WAL 时可退出 app 后重试）");

    let report = reconcile_session(&pool, &conv_id).await.expect("reconcile");
    // pretty-print 全量报告：diffs 非空 = bug 嫌疑清单；skipped 逐条核对 reason
    println!("{}", serde_json::to_string_pretty(&report).expect("serialize report"));
}
