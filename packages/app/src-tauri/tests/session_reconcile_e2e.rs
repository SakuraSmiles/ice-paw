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
    log_turn_context, log_turn_ended, log_user_message, AttachmentPageItem,
    AttachmentStoredPayload, EventCtx, TurnContextPayload, TurnEndedPayload,
};
use ice_paw_lib::harness::read_route::{ReadRoute, ReadRouteRegistry};
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
    sqlx::query(
        "INSERT INTO conversations (id, agent_id, title) VALUES ('conv-e', 'agent-1', 'e2e')",
    )
    .execute(&pool)
    .await
    .expect("seed conversation");
    pool
}

/// 生产写路径：先 create 占位行，流式结束后回写 content + blocks。
async fn write_row(
    pool: &SqlitePool,
    id: &str,
    role: &str,
    content: &str,
    blocks: &[ContentBlock],
) {
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
    write_row(
        pool,
        "turn-1",
        "user",
        "看这张图，再读一下 README",
        &user_blocks,
    )
    .await;
    log_user_message(
        pool,
        &ev,
        "turn-1",
        "看这张图，再读一下 README",
        &user_blocks,
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
        Some("glm-5.2"),
        "我来看一下",
        &a1_blocks,
        Some(12),
        None,
        0,
        false,
    )
    .await;
    log_tool_execution(
        pool,
        &ev,
        "msg-a1",
        "tc-1",
        Some("tu_1"),
        "read_file",
        "{\"path\":\"README.md\"}",
        Some("# IcePaw\n本地优先..."),
        false,
        34,
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
    write_row(
        pool,
        "msg-a2",
        "assistant",
        "README 说这是本地优先的 LLM 工作站。",
        &a2_blocks,
    )
    .await;
    log_assistant_message(
        pool,
        &ev,
        "msg-a2",
        Some("glm-5.2"),
        "README 说这是本地优先的 LLM 工作站。",
        &a2_blocks,
        Some(20),
        None,
        1,
        false,
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
    sqlx::query(
        "DELETE FROM session_events WHERE kind='assistant_message' AND message_id='msg-a2'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let report = reconcile_session(&pool, "conv-e").await.expect("reconcile");
    assert_eq!(
        categories(&report),
        vec!["MISSING_IN_DERIVED"],
        "{:#?}",
        report.diffs
    );
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
    assert_eq!(
        categories(&report),
        vec!["CONTENT_MISMATCH"],
        "{:#?}",
        report.diffs
    );
    assert!(
        report.diffs[0].detail.contains("blocks"),
        "{}",
        report.diffs[0].detail
    );
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
    assert_eq!(
        categories(&report),
        vec!["MISSING_IN_LEGACY"],
        "{:#?}",
        report.diffs
    );
    assert_eq!(report.diffs[0].message_id.as_deref(), Some("msg-a2"));
}

// =========================================================================
// 读路径路由（session-event-log Phase 2A）：reconcile-green ⇒ Derive；混合纪元 ⇒ Legacy
//
// 路由细节（指纹缓存 / force_legacy / 篡改回退）由 harness::read_route 的 in-crate
// 集成测试覆盖；这里验证两条 e2e 关键不变式——丰富脚本（含 Image+tool_use）零 diff
// 应路由 Derive；事件纪元前的旧行存在时（派生看不到它们）应路由 Legacy。
// =========================================================================

/// reconcile 零 diff 的丰富脚本（图 + 工具调用 + 附件）应路由到 Derive。
#[tokio::test]
async fn read_route_derive_for_reconcile_green_rich_script() {
    let pool = seeded_pool().await;
    script_consistent_turn(&pool).await;

    let reg = ReadRouteRegistry::new();
    let d = reg.resolve(&pool, "conv-e").await.expect("resolve");
    assert_eq!(
        d.route,
        ReadRoute::Derive,
        "reconcile 零 diff 的丰富脚本应判绿（监控语义）: {:#?}",
        d
    );
    assert_eq!(d.reason, "green");
    assert_eq!(d.diffs, 0);
}

/// 事件纪元前还有旧行（混合纪元）→ 派生看不到旧行 → 必须路由 Legacy。
#[tokio::test]
async fn read_route_legacy_for_mixed_epoch() {
    let pool = seeded_pool().await;
    // 纪元前的旧行（无事件，模拟 Phase 0 升级前已存在的历史）
    write_row(
        &pool,
        "legacy-1",
        "user",
        "旧消息",
        &[ContentBlock::text("旧消息")],
    )
    .await;
    write_row(
        &pool,
        "legacy-2",
        "assistant",
        "旧回复",
        &[ContentBlock::text("旧回复")],
    )
    .await;
    // 之后是事件纪元的完整 turn
    script_consistent_turn(&pool).await;

    let reg = ReadRouteRegistry::new();
    let d = reg.resolve(&pool, "conv-e").await.expect("resolve");
    assert_eq!(
        d.route,
        ReadRoute::Legacy,
        "混合纪元（旧行+新事件）应判非绿（派生看不到旧行）: {:#?}",
        d
    );
    assert_eq!(d.reason, "mixed_epoch");
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
    let conv_id =
        std::env::var("ICEPAW_RECONCILE_CONV").expect("设 ICEPAW_RECONCILE_CONV=<会话 id>");

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
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("serialize report")
    );
}

// =========================================================================
// 真机快照 backfill 验证（非 CI；S1 Phase 2B 阶段 0 门槛）
//
// 对【真机库的拷贝】执行完整 boot 序列（migrate → backfill → 全会话对账路由），
// 验证 backfill 把零事件旧会话转为纯事件纪元且构造性零 diff——legacy 读路径
// 退役（S1 本体）的前置闸门。
//
// ⚠️ backfill 会写库——**只许指向拷贝**（sqlite backup API 先做一致性快照），
// 绝不指向生产库。
//
//     ICEPAW_BACKFILL_DB="C:\Users\<u>\AppData\Local\Temp\s1-verify\ice-paw-copy.db" \
//     cargo test --test session_reconcile_e2e backfill_real_db_snapshot -- --ignored --nocapture
// =========================================================================

#[tokio::test]
#[ignore = "需真实库快照路径，显式触发；只指向拷贝"]
async fn backfill_real_db_snapshot() {
    let db_path =
        std::env::var("ICEPAW_BACKFILL_DB").expect("设 ICEPAW_BACKFILL_DB=<真机库拷贝路径>");

    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{db_path}"))
        .expect("valid sqlite url")
        .create_if_missing(false)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect snapshot db");

    // boot 序列 1：migrate（幂等——真实库已应用至最新则 no-op）
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");

    // boot 序列 2：backfill（写拷贝）
    let report = ice_paw_lib::harness::backfill::backfill_legacy_sessions(&pool).await;
    println!(
        "backfill 报告: backfilled={} events_written={} failed={} epoch_rows={}",
        report.backfilled, report.events_written, report.failed, report.epoch_rows
    );
    assert_eq!(report.failed, 0, "backfill 不应有失败会话");

    // 全会话路由 + 对账：backfill 后所有会话应 Derive + 零 diff（打印细节后断言）
    let convs: Vec<(String,)> = sqlx::query_as("SELECT id FROM conversations ORDER BY created_at")
        .fetch_all(&pool)
        .await
        .expect("list conversations");

    let reg = ReadRouteRegistry::new();
    let mut derive_n = 0usize;
    let mut legacy_n = 0usize;
    for (cid,) in &convs {
        let d = reg.resolve(&pool, cid).await.expect("resolve");
        let rep = reconcile_session(&pool, cid).await.expect("reconcile");
        if d.route == ReadRoute::Derive && rep.diffs.is_empty() {
            derive_n += 1;
        } else {
            legacy_n += 1;
            println!(
                "非绿: {cid} route={:?} reason={} events={} diffs={} skipped={:?}",
                d.route,
                d.reason,
                d.events_total,
                rep.diffs.len(),
                rep.skipped
            );
        }
    }
    println!(
        "总结: {} 会话 = {} Derive+零diff / {} 非绿",
        convs.len(),
        derive_n,
        legacy_n
    );
    assert_eq!(legacy_n, 0, "backfill 后仍存在非绿会话（细节见上方打印）");
    assert!(
        derive_n + legacy_n >= 9,
        "9 个 pre-Phase-0 旧会话应全部覆盖（含 17 个已事件会话共 26）"
    );
    let _ = report.payload_bytes; // 报告字段留存（boot 日志同款计量）
}
