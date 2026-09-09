//! 读路径基准（手动跑的 ignored 测试，不进 CI 常规面）。
//!
//! 合成千轮规模会话（事件 + 行双写），分段计时读路径核心环节，量化
//! 「每次发送 O(会话规模)」成本（P1）：事件全量读 / derive 纯回放 /
//! load_history_from_events / reconcile_session。尾部窗口派生（第 3 批）
//! 是否值得做，看这里的数字。
//!
//! payload 尺寸对齐生产有界上限：tool result/arguments 4KB 截断、assistant
//! 正文 ~2KB、用户消息数百字节——事件字节量与千轮真机会话同量级。
//!
//! 跑法（仓库根）：
//! ```bash
//! SODIUM_LIB_DIR=<见 CLAUDE.md> cargo test --manifest-path packages/app/src-tauri/Cargo.toml \
//!   --lib read_path_bench -- --ignored --nocapture
//! ```
//! 规模可调：环境变量 `BENCH_TURNS`（默认 1000 轮 ≈ 1.3 万事件 / 8 千行）。
//! ⚠️ in-memory SQLite（IO≈memcpy），绝对值偏乐观——看相对量级与增长斜率，
//! 不看单点毫秒。

use std::time::Instant;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

use crate::db::models::NewMessage;
use crate::db::repo;
use crate::harness::derive::derive_history;
use crate::harness::event_log::{
    log_assistant_message, log_tool_execution, log_tool_result_message, log_turn_context,
    log_turn_ended, log_user_message, EventCtx, TurnContextPayload, TurnEndedPayload,
};
use crate::infra::protocol::ContentBlock;

/// 默认合成轮数（≈13 事件/轮：user+ctx+3×(assistant+tool_exec+result)+终答+ended）。
const DEFAULT_TURNS: usize = 1000;
/// 每轮工具往返数。
const TOOL_ROUNDS: usize = 3;

async fn fresh_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .unwrap()
        .create_if_missing(true)
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap()
}

async fn seeded_pool() -> SqlitePool {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref,
             temperature, max_tokens, extra_params, sort_order, cache_prompt)
         VALUES ('a1', 'bench', 'anthropic', 'glm-5.2', '', '', 0.7, 1024, '{}', 0, 0)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES ('c1', 'a1', 'bench')")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

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
            conversation_id: "c1".into(),
            role: role.into(),
            content: content.into(),
            token_count: None,
            error: None,
            model: None,
        },
    )
    .await
    .unwrap();
    let blocks_json = serde_json::to_string(blocks).unwrap();
    repo::message::update_content_blocks(pool, id, &blocks_json)
        .await
        .unwrap();
}

/// 生产尺寸 filler：中英混排，长度按需（truncate 收敛到 char 边界防 panic）。
fn filler(prefix: &str, bytes: usize) -> String {
    let mut s = format!("{prefix}：");
    while s.len() < bytes {
        s.push_str("读路径基准载荷数据 data-payload-0123456789 ");
    }
    let mut n = bytes.max(prefix.len() + 3); // +3 跨过「：」，必为 char 边界
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    s.truncate(n);
    s
}

/// 一轮生产形态脚本：user → ctx → 3×(assistant+tool_use / tool_exec / tool_result)
/// → 终答 assistant → turn_ended。每 20 轮给终答做一次 supersede（自动续写，
/// 事件 last-wins + 行同步覆写——与生产 finalize 同形状）。
async fn synth_turn(pool: &SqlitePool, ctx: &EventCtx, turn: usize) {
    let turn_id = format!("turn-{turn}");

    // 用户消息（数百字节）
    let u_content = filler(&format!("第 {turn} 轮任务"), 400);
    let u_blocks = vec![ContentBlock::text(u_content.clone())];
    write_row(pool, &turn_id, "user", &u_content, &u_blocks).await;
    log_user_message(pool, ctx, &turn_id, &u_content, &u_blocks).await;

    log_turn_context(
        pool,
        ctx,
        &TurnContextPayload {
            v: 1,
            provider: "anthropic".into(),
            effective_model: "glm-5.2".into(),
            model_override: None,
            tools_enabled: true,
            tool_names: vec!["read_file".into(), "edit_file".into(), "run_command".into()],
            temperature: Some(0.7),
            max_tokens: Some(16384),
            tool_max_rounds: Some(12),
            budget_max_tokens: None,
            context_window: None,
        },
    )
    .await;

    for r in 0..TOOL_ROUNDS {
        // assistant（工具调用轮，正文 ~1.5KB + ToolUse）
        let a_id = format!("t{turn}-a{r}");
        let a_content = filler(&format!("第 {turn} 轮第 {r} 次工具分析"), 1500);
        let a_blocks = vec![
            ContentBlock::text(a_content.clone()),
            ContentBlock::ToolUse {
                id: format!("tu-{turn}-{r}"),
                name: "read_file".into(),
                input: serde_json::json!({"path": format!("src/mod_{turn}_{r}.rs")}).to_string(),
            },
        ];
        write_row(pool, &a_id, "assistant", &a_content, &a_blocks).await;
        log_assistant_message(
            pool,
            ctx,
            &a_id,
            Some("glm-5.2"),
            &a_content,
            &a_blocks,
            Some(600),
            Some(3_000),
            r as u32,
            false,
        )
        .await;

        // 工具审计（arguments 小 / result 5KB → 走 4KB 截断，对齐生产上限）
        let result = filler("工具输出", 5_000);
        log_tool_execution(
            pool,
            ctx,
            &a_id,
            &format!("tc-{turn}-{r}"),
            Some(&format!("tu-{turn}-{r}")),
            "read_file",
            "{\"path\":\"README.md\"}",
            Some(&result),
            false,
            120,
        )
        .await;

        // 工具结果消息（行 content 恒空 + 4KB blocks）
        let tr_id = format!("t{turn}-r{r}");
        let tr_blocks = vec![ContentBlock::ToolResult {
            tool_use_id: format!("tu-{turn}-{r}"),
            content: filler("文件内容", 4_000),
            is_error: Some(false),
        }];
        write_row(pool, &tr_id, "user", "", &tr_blocks).await;
        log_tool_result_message(pool, ctx, &tr_id, &tr_blocks).await;
    }

    // 终答 assistant（~2KB）
    let af_id = format!("t{turn}-af");
    let af_content = filler(&format!("第 {turn} 轮结论"), 2_000);
    let af_blocks = vec![ContentBlock::text(af_content.clone())];
    write_row(pool, &af_id, "assistant", &af_content, &af_blocks).await;
    log_assistant_message(
        pool,
        ctx,
        &af_id,
        Some("glm-5.2"),
        &af_content,
        &af_blocks,
        Some(900),
        Some(8_000),
        TOOL_ROUNDS as u32,
        false,
    )
    .await;

    // 每 20 轮一次 supersede（续写全文覆写）：事件 + 行双侧同步，保持对账 green
    if turn.is_multiple_of(20) {
        let sup_content = filler(&format!("第 {turn} 轮结论（续写）"), 2_400);
        let sup_blocks = vec![ContentBlock::text(sup_content.clone())];
        repo::message::update_content(pool, &af_id, &sup_content)
            .await
            .unwrap();
        let json = serde_json::to_string(&sup_blocks).unwrap();
        repo::message::update_content_blocks(pool, &af_id, &json)
            .await
            .unwrap();
        log_assistant_message(
            pool,
            ctx,
            &af_id,
            Some("glm-5.2"),
            &sup_content,
            &sup_blocks,
            Some(1_100),
            Some(9_000),
            TOOL_ROUNDS as u32,
            true,
        )
        .await;
    }

    log_turn_ended(
        pool,
        ctx,
        Some(&af_id),
        &TurnEndedPayload {
            v: 1,
            termination: "stop".into(),
            rounds: (TOOL_ROUNDS + 1) as u32,
            usage: None,
            user_token_count: Some(3_000),
        },
    )
    .await;
}

/// 分段计时：事件全量读 / derive 纯回放 / 派生历史加载 / 全量对账。
/// 断言对账 green（合成数据必须零 diff，否则基准无效）。
#[tokio::test]
#[ignore = "基准测试（合成数据 + 计时），手工跑：--ignored --nocapture"]
async fn bench_read_path_large_session() {
    let turns: usize = std::env::var("BENCH_TURNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TURNS);
    let pool = seeded_pool().await;

    let t0 = Instant::now();
    for turn in 1..=turns {
        let ctx = EventCtx::new("c1", &format!("turn-{turn}"), "a1");
        synth_turn(&pool, &ctx, turn).await;
    }
    let build_ms = t0.elapsed().as_millis();

    // ① 事件全量读（load 的主要 IO）
    let t1 = Instant::now();
    let events = repo::session_event::list_by_session(&pool, "c1", None)
        .await
        .unwrap();
    let read_ms = t1.elapsed().as_millis();

    // ② derive 纯回放（supersede 查找成本在此）
    let t2 = Instant::now();
    let derived = derive_history(&events);
    let derive_ms = t2.elapsed().as_millis();

    // ③ 派生历史加载端到端（发送路径每轮跑一次的函数）
    let t3 = Instant::now();
    let rows = crate::harness::read_route::load_history_from_events(&pool, "c1")
        .await
        .unwrap();
    let load_ms = t3.elapsed().as_millis();

    // ④ 全量对账（当前热路径每次发送也跑；后台化后移出延迟路径）
    let t4 = Instant::now();
    let report = crate::harness::reconcile::reconcile_session(&pool, "c1")
        .await
        .unwrap();
    let reconcile_ms = t4.elapsed().as_millis();

    let payload_bytes: usize = events.iter().map(|e| e.payload.len()).sum();
    println!("\n===== 读路径基准（{turns} 轮）=====");
    println!(
        "合成数据      {build_ms}ms   事件 {} 条 / {} KB   行（messages）{} 条",
        events.len(),
        payload_bytes / 1024,
        turns * (2 + TOOL_ROUNDS * 2),
    );
    println!("① 事件全量读  {read_ms}ms");
    println!(
        "② derive 回放 {derive_ms}ms   派生消息 {} 条 / issues {} 条",
        derived.messages.len(),
        derived.issues.len()
    );
    println!(
        "③ 历史加载    {load_ms}ms   窗口保留 {} 条（tail-limit {}）",
        rows.len(),
        repo::message::HISTORY_LOAD_LIMIT
    );
    println!(
        "④ 全量对账    {reconcile_ms}ms   diffs {} / skipped {:?}",
        report.diffs.len(),
        report.skipped.iter().map(|s| s.reason).collect::<Vec<_>>()
    );
    println!(
        "（每轮发送现付 ③ ≈ {load_ms}ms + 两个标量查询；④ 已后台化（resolve_for_turn），每 turn 静默期跑一次；in-memory 偏乐观，看量级）"
    );

    // 合成数据有效性：对账必须 green（否则量的是坏数据不是读路径）
    assert!(
        report.diffs.is_empty(),
        "合成会话对账不应有 diff：{:?}",
        report.diffs.first()
    );
    assert!(
        derived.issues.is_empty(),
        "回放不应有 issue：{:?}",
        derived.issues.first()
    );
    assert_eq!(
        rows.len(),
        (turns * (2 + TOOL_ROUNDS * 2)).min(repo::message::HISTORY_LOAD_LIMIT as usize)
    );
}
