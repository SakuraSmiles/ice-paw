//! session_events 事件序列端到端测试（session-event-log Phase 0）
//!
//! 用真实 typed emitters（`harness::event_log`）脚本化三类 turn，断言 Phase 0
//! 影子写入的核心不变式——Phase 1 derive-on-read 的可信地基：
//! 1. **回放序可信**：seq 从 1 起严格连续，kind 序与写入顺序一致
//! 2. **turn 归属**：同一 turn 的事件 turn_id 全一致；跨 turn seq 续接不归零
//! 3. **工具链配对**：tool_use ↔ tool_execution ↔ tool_result 按 id 串起
//! 4. **supersede**：同 message_id 多条 assistant_message 回放 last-wins
//!    （自动续写场景）
//! 5. **非成功路径完整**：message_discarded / message_error / turn_ended(abort)
//!    也成序列（终止原因此前完全不落库）
//!
//! 基建照抄 tests/memory_e2e.rs（in-memory SQLite + migrate! + 种子行）。
//! 运行：`cargo test --test session_event_log_e2e`

use std::str::FromStr;

use ice_paw_lib::db::models::SessionEventRow;
use ice_paw_lib::db::repo::session_event::list_by_session;
use ice_paw_lib::harness::event_log::{
    log_assistant_message, log_attachment_stored, log_message_discarded, log_message_error,
    log_tool_execution, log_tool_result_message, log_turn_context, log_turn_ended,
    log_user_message, AssistantMessagePayload, AttachmentPageItem, AttachmentStoredPayload,
    EventCtx, MessageDiscardedPayload, MessageErrorPayload, PayloadBlock, ToolExecutionPayload,
    ToolResultMessagePayload, TurnContextPayload, TurnEndedPayload,
};
use ice_paw_lib::infra::protocol::{ContentBlock, TokenUsage};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

// =========================================================================
// 基建（照抄 memory_e2e / repo::session_event 测试的坑位注释）
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

async fn rows(pool: &SqlitePool) -> Vec<SessionEventRow> {
    list_by_session(pool, "conv-e", None)
        .await
        .expect("list events")
}

fn kinds(rs: &[SessionEventRow]) -> Vec<&str> {
    rs.iter().map(|r| r.kind.as_str()).collect()
}

fn payload<T: serde::de::DeserializeOwned>(r: &SessionEventRow) -> T {
    serde_json::from_str(&r.payload).expect("payload 反序列化")
}

// =========================================================================
// 测试 1：带工具调用的完整 turn —— kind 序 / seq 连续 / turn 归属 / 工具链配对
// =========================================================================

#[tokio::test]
async fn full_tool_turn_sequence_is_replayable() {
    let pool = seeded_pool().await;
    let ev = EventCtx::new("conv-e", "turn-1", "agent-1");

    // --- 脚本化真实写入顺序（对应 send_message → loop_engine → cleanup）---
    log_turn_context(
        &pool,
        &ev,
        &TurnContextPayload {
            v: 1,
            provider: "anthropic".into(),
            effective_model: "glm-5.2".into(),
            model_override: None,
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
    log_user_message(
        &pool,
        &ev,
        "msg-u1",
        "读一下 README",
        &[ContentBlock::text("读一下 README")],
    )
    .await;
    log_attachment_stored(
        &pool,
        &ev,
        "msg-u1",
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
    // round 0：assistant 带 tool_use
    log_assistant_message(
        &pool,
        &ev,
        "msg-a1",
        Some("glm-5.2"),
        "我来看一下",
        &[
            ContentBlock::text("我来看一下"),
            ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "read_file".into(),
                input: "{\"path\":\"README.md\"}".into(),
            },
        ],
        Some(12),
        None,
        0,
        false,
    )
    .await;
    log_tool_execution(
        &pool,
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
    log_tool_result_message(
        &pool,
        &ev,
        "msg-tr1",
        &[ContentBlock::ToolResult {
            tool_use_id: "tu_1".into(),
            content: "# IcePaw\n本地优先...".into(),
            is_error: Some(false),
        }],
    )
    .await;
    // round 1：最终回答
    log_assistant_message(
        &pool,
        &ev,
        "msg-a2",
        Some("glm-5.2"),
        "README 说这是本地优先的 LLM 工作站。",
        &[ContentBlock::text("README 说这是本地优先的 LLM 工作站。")],
        Some(20),
        None,
        1,
        false,
    )
    .await;
    log_turn_ended(
        &pool,
        &ev,
        Some("msg-a2"),
        &TurnEndedPayload {
            v: 1,
            termination: "stop".into(),
            rounds: 2,
            usage: Some(TokenUsage {
                prompt_tokens: 3_000,
                completion_tokens: 32,
                cached_tokens: 0,
            }),
            user_token_count: Some(3_000),
        },
    )
    .await;

    // --- 断言 ---
    let rs = rows(&pool).await;
    assert_eq!(rs.len(), 8, "应恰好 8 条事件");
    assert_eq!(
        kinds(&rs),
        vec![
            "turn_context",
            "user_message",
            "attachment_stored",
            "assistant_message",
            "tool_execution",
            "tool_result_message",
            "assistant_message",
            "turn_ended",
        ],
        "kind 序应与真实写入顺序一致（回放序）"
    );
    // seq 从 1 起严格连续（append-only 无 UPDATE ⇒ 回放无空洞）
    let seqs: Vec<i64> = rs.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, (1..=8).collect::<Vec<_>>(), "seq 应 1..=8 连续");
    assert!(
        rs.iter().all(|r| r.turn_id.as_deref() == Some("turn-1")),
        "同一 turn 的事件 turn_id 应全一致"
    );

    // actor：用户侧事件 = user，agent 侧事件 = agent:<id>（多 agent 图的地基）
    assert_eq!(rs[1].actor, "user", "user_message actor=user");
    assert_eq!(rs[2].actor, "user", "attachment_stored actor=user");
    for (i, r) in rs.iter().enumerate() {
        if i != 1 && i != 2 {
            assert_eq!(
                r.actor, "agent:agent-1",
                "事件 {i} actor 应为 agent:agent-1"
            );
        }
    }

    // 工具链配对：tool_use(id) ↔ tool_execution(tool_use_id) ↔ tool_result(tool_use_id)
    let asst1: AssistantMessagePayload = payload(&rs[3]);
    let has_use = asst1.blocks.iter().any(|b| {
        matches!(
            b,
            PayloadBlock::Full(ContentBlock::ToolUse { id, .. }) if id == "tu_1"
        )
    });
    assert!(has_use, "assistant_message 应含 ToolUse id=tu_1");
    let exec: ToolExecutionPayload = payload(&rs[4]);
    assert_eq!(exec.tool_use_id.as_deref(), Some("tu_1"));
    assert_eq!(exec.tool_call_id, "tc-1");
    assert_eq!(exec.arguments, "{\"path\":\"README.md\"}");
    let trmsg: ToolResultMessagePayload = payload(&rs[5]);
    match &trmsg.blocks[0] {
        PayloadBlock::Full(ContentBlock::ToolResult { tool_use_id, .. }) => {
            assert_eq!(tool_use_id, "tu_1", "tool_result 应按 id 配对 tool_use");
        }
        other => panic!("tool_result_message 首块应为 ToolResult，got {other:?}"),
    }
    // tool_result_message 的 message_id 是独立 user 行（非 assistant 行）
    assert_eq!(rs[5].message_id.as_deref(), Some("msg-tr1"));
    assert_ne!(rs[5].message_id, rs[4].message_id);

    // turn_ended 指向最终 assistant
    let ended: TurnEndedPayload = payload(&rs[7]);
    assert_eq!(rs[7].message_id.as_deref(), Some("msg-a2"));
    assert_eq!(ended.termination, "stop");
    assert_eq!(ended.rounds, 2);
    assert_eq!(ended.usage.unwrap().prompt_tokens, 3_000);
}

// =========================================================================
// 测试 2：自动续写 supersede + 跨 turn seq 续接
// =========================================================================

#[tokio::test]
async fn supersede_last_wins_and_seq_continues_across_turns() {
    let pool = seeded_pool().await;

    // turn-1：截断 → 自动续写（同 msg-a1 两条 assistant_message，Phase 0 supersede）
    let ev1 = EventCtx::new("conv-e", "turn-1", "agent-1");
    log_turn_context(
        &pool,
        &ev1,
        &TurnContextPayload {
            v: 1,
            provider: "anthropic".into(),
            effective_model: "glm-5.2".into(),
            model_override: None,
            tools_enabled: false,
            tool_names: vec![],
            temperature: None,
            max_tokens: None,
            tool_max_rounds: None,
            budget_max_tokens: None,
            context_window: None,
        },
    )
    .await;
    log_user_message(
        &pool,
        &ev1,
        "msg-u1",
        "写一篇长文",
        &[ContentBlock::text("写一篇长文")],
    )
    .await;
    for (round, text) in [(0u32, "前半段"), (1u32, "前半段后半段")] {
        log_assistant_message(
            &pool,
            &ev1,
            "msg-a1",
            Some("glm-5.2"),
            text,
            &[ContentBlock::text(text)],
            Some(10),
            None,
            round,
            round > 0,
        )
        .await;
    }
    log_turn_ended(
        &pool,
        &ev1,
        Some("msg-a1"),
        &TurnEndedPayload {
            v: 1,
            termination: "length".into(),
            rounds: 2,
            usage: None,
            user_token_count: Some(100),
        },
    )
    .await;

    // turn-2：同一 session 的下一个 turn
    let ev2 = EventCtx::new("conv-e", "turn-2", "agent-1");
    log_turn_context(
        &pool,
        &ev2,
        &TurnContextPayload {
            v: 1,
            provider: "anthropic".into(),
            effective_model: "glm-5.2".into(),
            model_override: None,
            tools_enabled: false,
            tool_names: vec![],
            temperature: None,
            max_tokens: None,
            tool_max_rounds: None,
            budget_max_tokens: None,
            context_window: None,
        },
    )
    .await;
    log_user_message(&pool, &ev2, "msg-u2", "继续", &[ContentBlock::text("继续")]).await;
    log_assistant_message(
        &pool,
        &ev2,
        "msg-a2",
        Some("glm-5.2"),
        "好的",
        &[ContentBlock::text("好的")],
        Some(2),
        None,
        0,
        false,
    )
    .await;
    log_turn_ended(
        &pool,
        &ev2,
        Some("msg-a2"),
        &TurnEndedPayload {
            v: 1,
            termination: "stop".into(),
            rounds: 1,
            usage: None,
            user_token_count: Some(50),
        },
    )
    .await;

    // --- 断言 ---
    let rs = rows(&pool).await;
    assert_eq!(rs.len(), 9);
    // 跨 turn seq 续接（1..=9 连续，不归零）——回放时按 (turn_id, seq) 分组即得 turn 序
    let seqs: Vec<i64> = rs.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, (1..=9).collect::<Vec<_>>(), "跨 turn seq 应续接连续");
    assert!(
        rs[..5]
            .iter()
            .all(|r| r.turn_id.as_deref() == Some("turn-1")),
        "前 5 条属 turn-1"
    );
    assert!(
        rs[5..]
            .iter()
            .all(|r| r.turn_id.as_deref() == Some("turn-2")),
        "后 4 条属 turn-2"
    );
    // turn 边界干净：turn-1 末条是 turn_ended，turn-2 首条是 turn_context
    assert_eq!(rs[4].kind, "turn_ended");
    assert_eq!(rs[5].kind, "turn_context");

    // supersede 回放：同 message_id 的 assistant_message 按 last-wins 折叠
    let mut last_content: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for r in &rs {
        if r.kind == "assistant_message" {
            let p: AssistantMessagePayload = payload(r);
            last_content.insert(r.message_id.clone().unwrap(), p.content);
        }
    }
    assert_eq!(
        last_content.get("msg-a1").map(String::as_str),
        Some("前半段后半段"),
        "supersede 回放应取最后一条（全文覆写）"
    );
    assert_eq!(last_content.get("msg-a2").map(String::as_str), Some("好的"));
}

// =========================================================================
// 测试 3：非成功路径 —— message_discarded / message_error / turn_ended(abort)
// =========================================================================

#[tokio::test]
async fn abort_and_discard_paths_form_complete_sequence() {
    let pool = seeded_pool().await;

    // 场景 A：终止守卫删占位（cancel 时本轮纯 tool_use/thinking-only 无文本）
    let ev1 = EventCtx::new("conv-e", "turn-1", "agent-1");
    log_turn_context(
        &pool,
        &ev1,
        &TurnContextPayload {
            v: 1,
            provider: "anthropic".into(),
            effective_model: "glm-5.2".into(),
            model_override: None,
            tools_enabled: true,
            tool_names: vec!["run_command".into()],
            temperature: None,
            max_tokens: None,
            tool_max_rounds: None,
            budget_max_tokens: None,
            context_window: None,
        },
    )
    .await;
    log_user_message(
        &pool,
        &ev1,
        "msg-u1",
        "跑一下构建",
        &[ContentBlock::text("跑一下构建")],
    )
    .await;
    log_message_discarded(&pool, &ev1, "msg-a1", "termination_guard_no_text").await;
    log_turn_ended(
        &pool,
        &ev1,
        Some("msg-a1"),
        &TurnEndedPayload {
            v: 1,
            termination: "abort".into(),
            rounds: 1,
            usage: None,
            user_token_count: None,
        },
    )
    .await;

    // 场景 B：不可重试错误（emit_round_error → message_error + abort）
    let ev2 = EventCtx::new("conv-e", "turn-2", "agent-1");
    log_turn_context(
        &pool,
        &ev2,
        &TurnContextPayload {
            v: 1,
            provider: "anthropic".into(),
            effective_model: "glm-5.2".into(),
            model_override: None,
            tools_enabled: false,
            tool_names: vec![],
            temperature: None,
            max_tokens: None,
            tool_max_rounds: None,
            budget_max_tokens: None,
            context_window: None,
        },
    )
    .await;
    log_user_message(&pool, &ev2, "msg-u2", "你好", &[ContentBlock::text("你好")]).await;
    log_message_error(&pool, &ev2, "msg-a2", "Network", "connection refused").await;
    log_turn_ended(
        &pool,
        &ev2,
        Some("msg-a2"),
        &TurnEndedPayload {
            v: 1,
            termination: "abort".into(),
            rounds: 0,
            usage: None,
            user_token_count: None,
        },
    )
    .await;

    // --- 断言：两类非成功路径事件完整可回放 ---
    let rs = rows(&pool).await;
    assert_eq!(
        kinds(&rs),
        vec![
            "turn_context",
            "user_message",
            "message_discarded",
            "turn_ended",
            "turn_context",
            "user_message",
            "message_error",
            "turn_ended",
        ],
        "非成功路径也应成完整序列"
    );
    let disc: MessageDiscardedPayload = payload(&rs[2]);
    assert_eq!(disc.reason, "termination_guard_no_text");
    let err: MessageErrorPayload = payload(&rs[6]);
    assert_eq!(err.kind, "Network");
    assert_eq!(err.error, "connection refused");
    // 两个 turn 的终止语义都记为 abort（此前完全不落库的新增价值点）
    for idx in [3usize, 7] {
        let ended: TurnEndedPayload = payload(&rs[idx]);
        assert_eq!(ended.termination, "abort");
    }
}
