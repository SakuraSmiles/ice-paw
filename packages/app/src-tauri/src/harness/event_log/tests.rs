use super::*;
use crate::db::models::SessionEventRow;
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

async fn seed(pool: &SqlitePool) {
    sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES ('agent-1', 't', 'anthropic', 'claude-test', '', '', 0.7, 1024, '{}', 0, 0)",
        )
        .execute(pool)
        .await
        .expect("seed agent");
    sqlx::query(
        "INSERT INTO conversations (id, agent_id, title) VALUES ('conv-1', 'agent-1', 't')",
    )
    .execute(pool)
    .await
    .expect("seed conversation");
}

fn ctx() -> EventCtx {
    EventCtx::new("conv-1", "turn-1", "agent-1")
}

/// 读回该会话唯一一条事件的 payload 并反序列化。
async fn sole_event_payload<T: serde::de::DeserializeOwned>(pool: &SqlitePool) -> T {
    let rows = session_event::list_by_session(pool, "conv-1", None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "应恰好写入 1 条事件");
    serde_json::from_str(&rows[0].payload).expect("payload 反序列化")
}

#[tokio::test]
async fn user_message_round_trip() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;

    let blocks = vec![
        ContentBlock::Text {
            text: "看这张图".into(),
        },
        ContentBlock::Attachment {
            name: "plan.pdf".into(),
            kind: "application/pdf".into(),
            size: 282_000,
        },
    ];
    log_user_message(&pool, &ctx(), "msg-u1", "看这张图", &blocks, None).await;

    let row: SessionEventRow = session_event::list_by_session(&pool, "conv-1", None)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(row.kind, "user_message");
    assert_eq!(row.actor, "user");
    assert_eq!(row.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(row.message_id.as_deref(), Some("msg-u1"));

    let back: UserMessagePayload = serde_json::from_str(&row.payload).unwrap();
    assert_eq!(back.content, "看这张图");
    assert_eq!(back.blocks.len(), 2);
    assert!(matches!(
        back.blocks[1],
        PayloadBlock::Full(ContentBlock::Attachment { .. })
    ));
}

#[tokio::test]
async fn assistant_message_round_trip_and_supersede() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;

    let mk_blocks = |content: &str| {
        vec![ContentBlock::Text {
            text: content.into(),
        }]
    };
    // 自动续写：同 message_id 两条（supersede last-wins）
    log_assistant_message(
        &pool,
        &ctx(),
        "msg-a1",
        Some("glm-5.2"),
        "前半段",
        &mk_blocks("前半段"),
        Some(42),
        Some(3_500),
        0,
        false,
    )
    .await;
    log_assistant_message(
        &pool,
        &ctx(),
        "msg-a1",
        Some("glm-5.2"),
        "前半段后半段",
        &mk_blocks("前半段后半段"),
        Some(42),
        Some(3_500),
        1,
        true,
    )
    .await;

    let rows = session_event::list_by_session(&pool, "conv-1", None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|r| r.kind == "assistant_message" && r.actor == "agent:agent-1"));
    let last: AssistantMessagePayload = serde_json::from_str(&rows[1].payload).unwrap();
    assert_eq!(last.content, "前半段后半段");
    assert!(last.continuation);
    assert_eq!(last.round, 1);
    assert_eq!(last.duration_ms, Some(3_500));
    // 旧事件（无 duration_ms 字段）反序列化 → None，前端隐式耗时兜底的输入
    let legacy: AssistantMessagePayload = serde_json::from_str(
        r#"{"v":1,"content":"旧","blocks":[],"round":0,"continuation":false}"#,
    )
    .unwrap();
    assert_eq!(legacy.duration_ms, None);
}

/// refify_blocks：Image 换轻量引用（payload 无 base64），其余原样 Full；
/// 序列化 wire 形态可被 untagged 往返。
#[test]
fn refify_replaces_images_keeps_others() {
    let blocks = vec![
        ContentBlock::text("看图"),
        ContentBlock::Image {
            data: "QUJD".into(),
            media_type: "image/png".into(),
        },
        ContentBlock::ToolUse {
            id: "tu1".into(),
            name: "read_file".into(),
            input: "{}".into(),
        },
    ];
    let payload_blocks = refify_blocks("msg-u1", &blocks);

    // Image → ref（指向下标 1），其余 Full
    assert!(matches!(
        payload_blocks[0],
        PayloadBlock::Full(ContentBlock::Text { .. })
    ));
    match &payload_blocks[1] {
        PayloadBlock::ImageRef {
            marker,
            message_id,
            block_index,
        } => {
            assert_eq!(marker, "image_ref");
            assert_eq!(message_id, "msg-u1");
            assert_eq!(*block_index, 1, "索引 = 数组下标（行布局对齐）");
        }
        other => panic!("expected ImageRef, got {other:?}"),
    }
    assert!(matches!(
        payload_blocks[2],
        PayloadBlock::Full(ContentBlock::ToolUse { .. })
    ));

    // wire 序列化：无 base64 + untagged 往返还原
    let wire = serde_json::to_string(&payload_blocks).unwrap();
    assert!(!wire.contains("QUJD"), "payload 不应含图片字节: {wire}");
    assert!(wire.contains("image_ref"));
    let back: Vec<PayloadBlock> = serde_json::from_str(&wire).unwrap();
    assert_eq!(back, payload_blocks);

    // v1 内联事件（Image 原样）→ 反序列化落 Full，读侧零迁移
    let v1 =
        r#"[{"type":"text","text":"q"},{"type":"image","data":"QUJD","media_type":"image/png"}]"#;
    let legacy: Vec<PayloadBlock> = serde_json::from_str(v1).unwrap();
    assert!(matches!(
        legacy[1],
        PayloadBlock::Full(ContentBlock::Image { .. })
    ));
}

#[tokio::test]
async fn tool_execution_truncates_like_audit_row() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;

    let long = "x".repeat(9_000);
    log_tool_execution(
        &pool,
        &ctx(),
        "msg-a1",
        "tc-1",
        Some("tu-1"),
        "write_file",
        &long,
        Some(&long),
        true,
        1_234,
    )
    .await;

    let back: ToolExecutionPayload = sole_event_payload(&pool).await;
    assert_eq!(
        back.arguments.chars().count(),
        4_000 + "…[已截断]".chars().count()
    );
    assert!(back.result.as_deref().unwrap().ends_with("…[已截断]"));
    assert!(back.is_error);
    assert_eq!(back.duration_ms, 1_234);
    assert_eq!(back.tool_use_id.as_deref(), Some("tu-1"));
}

/// Y2：消息类事实 append 瞬态失败 → 立即重试自愈（事件在场、语义完整）。
///
/// 注入手段 = BEFORE INSERT 触发器 `RAISE(FAIL)`——FAIL 不回滚同语句内
/// 先行的副作用（ABORT/ROLLBACK 会把计数 UPDATE 一起回滚，做不出
/// fail-once 语义），计数表因此能跨尝试累计。
#[tokio::test]
async fn message_fact_append_retries_transient_failure() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;
    sqlx::query("CREATE TABLE evt_retry_probe (n INTEGER NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO evt_retry_probe VALUES (0)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        r#"CREATE TRIGGER evt_fail_once BEFORE INSERT ON session_events
               WHEN (SELECT n FROM evt_retry_probe) = 0
               BEGIN
                 UPDATE evt_retry_probe SET n = n + 1;
                 SELECT RAISE(FAIL, '注入瞬态失败');
               END"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    log_user_message(&pool, &ctx(), "msg-r1", "重试语义", &[], None).await;

    // 计数表只记触发器注入次数（成功插入不命中 WHEN 条件、零计数）：
    // 恰 1 次 = 首试被注入失败、立即重试时条件已不成立 → 直接成功
    let (n,): (i64,) = sqlx::query_as("SELECT n FROM evt_retry_probe")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "瞬态失败恰注入一次，重试即治愈");
    let rows = session_event::list_by_session(&pool, "conv-1", None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "瞬态失败被重试治愈，事件在场");
    assert_eq!(rows[0].kind, "user_message");
    assert_eq!(rows[0].message_id.as_deref(), Some("msg-r1"));
}

/// Y2：重试策略按 kind 分野——消息类耗尽 3 次（首试 + 立即 + 退避）后
/// 不 panic、事件缺席但调用正常返回；非消息类（message_error 等）单次
/// 不重试。持续失败经同一触发器注入（无 fail-once 条件）。
#[tokio::test]
async fn append_retry_policy_is_scoped_to_message_facts() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;
    sqlx::query("CREATE TABLE evt_retry_probe (n INTEGER NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO evt_retry_probe VALUES (0)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        r#"CREATE TRIGGER evt_fail_always BEFORE INSERT ON session_events
               BEGIN
                 UPDATE evt_retry_probe SET n = n + 1;
                 SELECT RAISE(FAIL, '注入持续失败');
               END"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    // 消息类：3 次尝试全部失败 → 正常返回（不 panic、不阻断调用方）
    log_user_message(&pool, &ctx(), "msg-r2", "耗尽语义", &[], None).await;
    // 非消息类：单次尝试
    log_message_error(&pool, &ctx(), "msg-r2", "test", "boom").await;

    let (n,): (i64,) = sqlx::query_as("SELECT n FROM evt_retry_probe")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 4, "消息类 3 次（含退避重试）+ 非消息类 1 次");
    assert_eq!(
        session_event::list_by_session(&pool, "conv-1", None)
            .await
            .unwrap()
            .len(),
        0,
        "持续失败下零事件落库"
    );
}

#[tokio::test]
async fn sweep_interrupted_turns_closes_open_and_idempotent() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;
    sqlx::query(
        "INSERT INTO conversations (id, agent_id, title) VALUES ('conv-2', 'agent-1', 't')",
    )
    .execute(&pool)
    .await
    .expect("seed conversation 2");

    // conv-1：崩溃残留——turn_context + assistant_message 已落，无 turn_ended
    // （payload 对扫尾不敏感，只看 kind/turn_id）
    session_event::append(
        &pool,
        "conv-1",
        kind::TURN_CONTEXT,
        "agent:agent-1",
        Some("turn-open"),
        None,
        "{}",
    )
    .await
    .unwrap();
    session_event::append(
        &pool,
        "conv-1",
        kind::ASSISTANT_MESSAGE,
        "agent:agent-1",
        Some("turn-open"),
        Some("msg-a1"),
        "{}",
    )
    .await
    .unwrap();
    // conv-2：正常闭合 turn（扫尾不得触碰）
    let closed_ctx = EventCtx::new("conv-2", "turn-closed", "agent-1");
    session_event::append(
        &pool,
        "conv-2",
        kind::TURN_CONTEXT,
        "agent:agent-1",
        Some("turn-closed"),
        None,
        "{}",
    )
    .await
    .unwrap();
    log_turn_ended(
        &pool,
        &closed_ctx,
        Some("msg-a2"),
        &TurnEndedPayload {
            v: 1,
            termination: "stop".into(),
            rounds: 1,
            usage: None,
            user_token_count: None,
        },
    )
    .await;

    // 扫尾：只补 conv-1 的未闭合 turn（before 传远未来 = 全量命中，等价旧行为）
    assert_eq!(
        sweep_interrupted_turns(&pool, "9999-12-31 23:59:59").await,
        1
    );
    let rows = session_event::list_by_session(&pool, "conv-1", None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3, "补记后 conv-1 应有 3 条事件");
    let last = &rows[2];
    assert_eq!(last.kind, kind::TURN_ENDED);
    assert_eq!(last.turn_id.as_deref(), Some("turn-open"));
    assert_eq!(last.actor, "agent:agent-1", "复用原 turn_context 的 actor");
    assert_eq!(last.message_id, None);
    let p: TurnEndedPayload = serde_json::from_str(&last.payload).unwrap();
    assert_eq!(p.termination, "interrupted");
    assert_eq!(p.rounds, 1, "rounds = 已落 assistant_message 事件数");
    assert!(p.usage.is_none());

    // conv-2 不受干扰（仍 2 条，无新增）
    assert_eq!(
        session_event::list_by_session(&pool, "conv-2", None)
            .await
            .unwrap()
            .len(),
        2
    );

    // 幂等：再扫零补记、零写入
    assert_eq!(
        sweep_interrupted_turns(&pool, "9999-12-31 23:59:59").await,
        0
    );
    assert_eq!(
        session_event::list_by_session(&pool, "conv-1", None)
            .await
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn sweep_respects_boot_at_boundary() {
    // sweep 后台化后的进程边界：只补记 boot_at 之前遗留的未闭合 turn，
    // 结构上排除误杀本进程刚开的新 turn（显式 created_at 造早/晚两条）
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;
    // 上一进程遗留（早于 boot_at）
    sqlx::query(
            "INSERT INTO session_events (session_id, seq, kind, actor, turn_id, payload, created_at)
             VALUES ('conv-1', 1, 'turn_context', 'agent:agent-1', 'turn-old', '{}', '2026-01-01 00:00:00')",
        )
        .execute(&pool)
        .await
        .unwrap();
    // 本进程刚开的新 turn（晚于 boot_at，进行中）
    sqlx::query(
            "INSERT INTO session_events (session_id, seq, kind, actor, turn_id, payload, created_at)
             VALUES ('conv-1', 2, 'turn_context', 'agent:agent-1', 'turn-new', '{}', '2026-06-01 00:00:00')",
        )
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        sweep_interrupted_turns(&pool, "2026-03-01 00:00:00").await,
        1
    );
    // turn-old 补记 closed；turn-new 保持 open（无 turn_ended）
    let rows = session_event::list_by_session(&pool, "conv-1", None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3, "只补记 turn-old 一条");
    assert_eq!(rows[2].turn_id.as_deref(), Some("turn-old"));
    assert_eq!(rows[2].kind, kind::TURN_ENDED);
}

#[tokio::test]
async fn turn_context_and_turn_ended_round_trip() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;

    log_turn_context(
        &pool,
        &ctx(),
        &TurnContextPayload {
            v: 1,
            provider: "anthropic".into(),
            effective_model: "glm-5.2".into(),
            model_override: Some("glm-5-turbo".into()),
            tools_enabled: true,
            tool_names: vec!["read_file".into(), "run_command".into()],
            temperature: Some(0.7),
            max_tokens: Some(16_384),
            tool_max_rounds: Some(12),
            budget_max_tokens: Some(200_000),
            context_window: Some(1_000_000),
        },
    )
    .await;

    let back: TurnContextPayload = sole_event_payload(&pool).await;
    assert_eq!(back.effective_model, "glm-5.2");
    assert_eq!(back.tool_names.len(), 2);
    assert_eq!(back.v, 1);

    // 换 turn 重新 seed 事件表不可行（append-only），直接在新会话验证 turn_ended
    sqlx::query(
        "INSERT INTO conversations (id, agent_id, title) VALUES ('conv-2', 'agent-1', 't')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let ctx2 = EventCtx::new("conv-2", "turn-2", "agent-1");
    log_turn_ended(
        &pool,
        &ctx2,
        Some("msg-a1"),
        &TurnEndedPayload {
            v: 1,
            termination: "budget_exceeded".into(),
            rounds: 3,
            usage: Some(TokenUsage {
                prompt_tokens: 10_000,
                completion_tokens: 2_000,
                cached_tokens: 512,
            }),
            user_token_count: Some(120),
        },
    )
    .await;

    let rows = session_event::list_by_session(&pool, "conv-2", None)
        .await
        .unwrap();
    let back: TurnEndedPayload = serde_json::from_str(&rows[0].payload).unwrap();
    assert_eq!(back.termination, "budget_exceeded");
    assert_eq!(back.usage.unwrap().completion_tokens, 2_000);
    assert_eq!(back.user_token_count, Some(120));
}

#[tokio::test]
async fn plan_updated_round_trip() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;

    // 全量覆写语义：两次调用两条事件，回放 last-wins（最后一条 = 当前计划）
    log_plan_updated(
        &pool,
        &ctx(),
        &PlanUpdatedPayload {
            v: 1,
            items: vec![
                PlanItem {
                    text: "调研渲染方案".into(),
                    status: "done".into(),
                    task_conversation_id: None,
                },
                PlanItem {
                    text: "设计评审".into(),
                    status: "in_progress".into(),
                    task_conversation_id: Some("conv-child-1".into()),
                },
            ],
        },
    )
    .await;
    log_plan_updated(
        &pool,
        &ctx(),
        &PlanUpdatedPayload {
            v: 1,
            items: vec![
                PlanItem {
                    text: "设计评审".into(),
                    status: "done".into(),
                    task_conversation_id: Some("conv-child-1".into()),
                },
                PlanItem {
                    text: "终稿交付".into(),
                    status: "pending".into(),
                    task_conversation_id: None,
                },
            ],
        },
    )
    .await;

    let rows = session_event::list_by_session(&pool, "conv-1", None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| {
        r.kind == "plan_updated" && r.actor == "agent:agent-1" && r.message_id.is_none()
    }));
    let last: PlanUpdatedPayload = serde_json::from_str(&rows[1].payload).unwrap();
    assert_eq!(last.items.len(), 2);
    assert_eq!(last.items[0].status, "done");
    assert_eq!(
        last.items[0].task_conversation_id.as_deref(),
        Some("conv-child-1")
    );
    assert_eq!(last.items[1].task_conversation_id, None);
}

#[tokio::test]
async fn context_breakdown_round_trip() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;

    log_context_breakdown(
        &pool,
        &ctx(),
        &ContextBreakdownPayload {
            v: 1,
            segments: vec![
                ContextBreakdownSegment {
                    label: "system_persona".into(),
                    est: 120,
                    count: None,
                },
                ContextBreakdownSegment {
                    label: "tool_defs".into(),
                    est: 3_400,
                    count: Some(18),
                },
            ],
            est_total: 3_520,
            actual_prompt_tokens: Some(4_096),
            fingerprint: ContextBreakdownFingerprint {
                tools: "aaaabbbbcccc".into(),
                system_stable: "ddddddddeeee".into(),
                os_stable: "ffff00001111".into(),
            },
            rounds: vec![
                ContextBreakdownRound {
                    prompt: 4_096,
                    cached: 0,
                    tools_hash: "aaaabbbbcccc".into(),
                    injected: false,
                    model_switched: false,
                },
                ContextBreakdownRound {
                    prompt: 6_100,
                    cached: 4_096,
                    tools_hash: "aaaabbbbcccd".into(),
                    injected: true,
                    model_switched: true,
                },
            ],
        },
    )
    .await;

    let back: ContextBreakdownPayload = sole_event_payload(&pool).await;
    assert_eq!(back.v, 1);
    assert_eq!(back.segments.len(), 2);
    assert_eq!(back.segments[1].count, Some(18));
    assert_eq!(back.est_total, 3_520);
    assert_eq!(back.actual_prompt_tokens, Some(4_096));
    assert_eq!(back.fingerprint.tools, "aaaabbbbcccc");
    assert_eq!(back.rounds.len(), 2);
    assert_eq!(back.rounds[1].cached, 4_096);
    assert!(back.rounds[1].model_switched);

    // 旧 / 局部 JSON：缺 v、actual_prompt_tokens、count 仍可解析（可选字段
    // serde default —— 前端旧轮次与降级路径不炸）
    let minimal: ContextBreakdownPayload = serde_json::from_str(
            r#"{"segments":[{"label":"history","est":900}],"est_total":900,
                "fingerprint":{"tools":"a","system_stable":"b","os_stable":"c"},
                "rounds":[{"prompt":900,"cached":900,"tools_hash":"a","injected":false,"model_switched":false}]}"#,
        )
        .unwrap();
    assert_eq!(minimal.v, 1, "缺 v 默认 1");
    assert_eq!(minimal.actual_prompt_tokens, None);
    assert_eq!(minimal.segments[0].count, None);
}

#[tokio::test]
async fn attachment_modal_hook_summary_round_trip() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;

    log_attachment_stored(
        &pool,
        &ctx(),
        "msg-u1",
        &AttachmentStoredPayload::Bytes {
            v: 1,
            items: vec![AttachmentBytesItem {
                idx: 0,
                name: "plan.pdf".into(),
                ext: "pdf".into(),
                bytes_len: 282_000,
            }],
        },
    )
    .await;
    let back: AttachmentStoredPayload = sole_event_payload(&pool).await;
    match back {
        AttachmentStoredPayload::Bytes { items, .. } => {
            assert_eq!(items[0].bytes_len, 282_000);
            // 元信息 only：payload 里不得出现 BLOB/base64
            assert!(items[0].name == "plan.pdf");
        }
        other => panic!("应反序列化为 Bytes 变体，got {other:?}"),
    }

    // modal_adapted（含 OCR 全文）
    sqlx::query("DELETE FROM session_events")
        .execute(&pool)
        .await
        .unwrap();
    log_modal_adapted(
        &pool,
        &ctx(),
        &ModalAdaptedPayload {
            v: 1,
            stage: "user_image".into(),
            mode: "ocr_substitute".into(),
            items: vec![ModalAdaptedItem {
                index: 0,
                outcome: "substituted".into(),
                ocr_text: Some("一张户型图：三室两厅".into()),
            }],
        },
    )
    .await;
    let back: ModalAdaptedPayload = sole_event_payload(&pool).await;
    assert_eq!(
        back.items[0].ocr_text.as_deref(),
        Some("一张户型图：三室两厅")
    );

    // hook_injected
    sqlx::query("DELETE FROM session_events")
        .execute(&pool)
        .await
        .unwrap();
    log_hook_injected(
        &pool,
        &ctx(),
        &HookInjectedPayload {
            v: 1,
            point: "before_llm".into(),
            prompt: "注意编码规范".into(),
        },
    )
    .await;
    let back: HookInjectedPayload = sole_event_payload(&pool).await;
    assert_eq!(back.point, "before_llm");

    // summary
    sqlx::query("DELETE FROM session_events")
        .execute(&pool)
        .await
        .unwrap();
    log_summary_updated(
        &pool,
        &ctx(),
        &SummaryPayload {
            v: 1,
            summary_message_id: "msg-s1".into(),
            content: "[Previous conversation summary] ...".into(),
            covered_until_rowid: 77,
            covered_until_seq: Some(77),
        },
    )
    .await;
    let back: SummaryPayload = sole_event_payload(&pool).await;
    assert_eq!(back.covered_until_rowid, 77);
}

/// MA-3 来件族 round-trip：投递（actor=源 agent、turn 独立、message_id 关联）
/// + 终态（actor=user）。
#[tokio::test]
async fn cross_session_message_round_trip() {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    seed(&pool).await;

    // 投递：ctx 按投递方契约构造（turn = cross:{message_id}，agent = 源 agent）
    let relay_ctx = EventCtx::new("conv-1", "cross:xm-1", "agent-src");
    log_cross_session_message(
        &pool,
        &relay_ctx,
        &CrossSessionMessagePayload {
            v: 1,
            message_id: "xm-1".into(),
            source_conversation_id: "conv-src".into(),
            source_conversation_title: "源会话".into(),
            source_agent_id: "agent-src".into(),
            source_agent_name: "源 agent".into(),
            content: "材质参数定稿了吗".into(),
            expect_reply: true,
            is_reply: false,
            delivered_at_unix: 1_750_000_000,
        },
    )
    .await;
    let rows = session_event::list_by_session(&pool, "conv-1", None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "cross_session_message");
    assert_eq!(
        rows[0].actor, "agent:agent-src",
        "中继不带用户权威：源 agent 归因"
    );
    assert_eq!(rows[0].turn_id.as_deref(), Some("cross:xm-1"));
    assert_eq!(rows[0].message_id.as_deref(), Some("xm-1"));
    let back: CrossSessionMessagePayload = serde_json::from_str(&rows[0].payload).unwrap();
    assert_eq!(back.source_conversation_title, "源会话");
    assert!(back.expect_reply);

    // 终态：actor 恒 user，message_id 关联
    log_cross_session_message_settled(
        &pool,
        &relay_ctx,
        &CrossSessionMessageSettledPayload {
            v: 1,
            message_id: "xm-1".into(),
            action: "refused".into(),
            by: "user-refused".into(),
        },
    )
    .await;
    let rows = session_event::list_by_session(&pool, "conv-1", None)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].kind, "cross_session_message_settled");
    assert_eq!(rows[1].actor, "user");
    assert_eq!(rows[1].message_id.as_deref(), Some("xm-1"));
    let back: CrossSessionMessageSettledPayload = serde_json::from_str(&rows[1].payload).unwrap();
    assert_eq!(back.action, "refused");
}
