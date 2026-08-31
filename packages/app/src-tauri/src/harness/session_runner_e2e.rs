//! session_runner 全链路 e2e（清扫 S5）。
//!
//! 「发消息 → Pipeline → 落库 → 主循环（stream_loop）→ 工具执行 → 事件日志」
//! 的第一条真正端到端测试网。此前这条链焊死在 `AppHandle` 上，无法脱离 Tauri
//! 运行时驱动（tests/ 目录里只有 typed emitters 的脚本化测试）；S6 的
//! `LoopEmitter` 抽象 + S5 的 `MockProvider::ToolCallThenText` 场景让本文件用
//! 收集型 emitter + 无网络 mock 即可跑完整回合。
//!
//! 断言面（四层）：
//! 1. **消息行**：user / assistant / tool_result 行的 role、content、blocks
//! 2. **事件序**：session_events 的 kind 序 + seq 严格连续 + turn_id 全一致
//! 3. **UI 事件**：CollectEmitter 收到的瞬态事件（chat:start / chunk / tool-* / done）
//! 4. **TurnSummary**：delegate 完成信号（finish_reason / final_text / rounds）
//!
//! 场景矩阵：正常回合 / 空响应 / 限流重试中取消 / 预算触顶（显式硬上限）/
//! 流中取消（占位 discard）/ 工具轮（tool_use ↔ tool_execution ↔ tool_result 配对）/
//! 零事件旧行会话（Phase 2B legacy 读路径退役的行为锁定）。
//!
//! 基建照抄 tests/session_event_log_e2e.rs（in-memory SQLite + migrate! + 种子行）；
//! 须放 src/ 内部——`run_agent_turn` 是 pub(crate)，tests/ 目录拿不到。

use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::db::models::{ConversationRow, HookConfig};
use crate::db::repo;
use crate::error::AppResult;
use crate::harness::chat_state::CancellationToken;
use crate::harness::mcp::client::McpClient;
use crate::harness::mcp::{McpRegistry, McpServerManager};
use crate::harness::provider::mock::{MockProvider, MockScenario};
use crate::harness::r#loop::emitter::LoopEmitter;
use crate::harness::read_route::ReadRouteRegistry;
use crate::harness::session_runner::{run_agent_turn, AgentTurnInput, TurnEnv};
use crate::harness::tool_executor::ToolAuthRegistry;
use crate::infra::protocol::ContentBlock;

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

/// 种子：agent-1（extra_params 可注入 max_total_tokens 等）+ conv-e（kind 默认 chat）。
async fn seeded_pool(extra_params: &str) -> SqlitePool {
    let pool = fresh_pool().await;
    sqlx::migrate!("./src/db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    sqlx::query(
        "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref,
             temperature, max_tokens, extra_params, sort_order, cache_prompt)
         VALUES ('agent-1', 'e2e', 'anthropic', 'glm-5.2', '', '', 0.7, 1024, ?, 0, 0)",
    )
    .bind(extra_params)
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

/// 收集型 LoopEmitter（S6 测试实现）：瞬态 UI 事件全量入 Vec，on_loop_exit 计数。
#[derive(Clone, Default)]
struct CollectEmitter {
    events: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    exits: Arc<std::sync::atomic::AtomicU32>,
}

impl LoopEmitter for CollectEmitter {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.events
            .lock()
            .expect("emitter lock")
            .push((event.to_string(), payload));
    }
    fn on_loop_exit(&self) {
        self.exits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl CollectEmitter {
    /// 收到的 UI 事件名序列（断言序用）。
    fn names(&self) -> Vec<String> {
        self.events
            .lock()
            .expect("emitter lock")
            .iter()
            .map(|(n, _)| n.clone())
            .collect()
    }
}

/// 内存 echo 工具（授权级别 Always——e2e 不走弹窗）。
struct EchoTool;

#[async_trait]
impl McpClient for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "回显参数（e2e 测试工具）"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"msg": {"type": "string"}},
            "required": ["msg"]
        })
    }
    async fn execute(&self, args: &str) -> AppResult<String> {
        Ok(format!("echo: {args}"))
    }
}

/// 内存恒败工具（doom_loop e2e 用）：每次执行都失败，且错误首行是稳定
/// 家族前缀（「always_fail 恒败」）——同签名连败正是 doom_detect 的靶形态。
struct AlwaysFailTool;

#[async_trait]
impl McpClient for AlwaysFailTool {
    fn name(&self) -> &str {
        "always_fail"
    }
    fn description(&self) -> &str {
        "恒败工具（e2e 测试工具，doom_loop 场景）"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"x": {"type": "integer"}},
            "required": ["x"]
        })
    }
    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(crate::error::AppError::Validation(
            "always_fail 恒败: e2e 固定错误（模拟弱模型反复撞同一失败）".into(),
        ))
    }
}

/// 一次 e2e 回合的驱动器：env + 输入 + 完成信号。
struct TurnFixture {
    pool: SqlitePool,
    emitter: CollectEmitter,
    mock: Arc<MockProvider>,
    user_msg_id: String,
    cancel: CancellationToken,
    rx: tokio::sync::oneshot::Receiver<crate::harness::session_runner::TurnSummary>,
}

/// 跑一个完整回合（tools=false 时全局注册表为空）。
async fn run_turn(scenario: MockScenario, tools: bool, extra_params: &str) -> TurnFixture {
    let pool = seeded_pool(extra_params).await;
    run_turn_on(pool, scenario, tools).await
}

/// 在**已有库**上跑一个完整回合（调用方先注入自定义形态——如零事件旧行——
/// 再驱动回合，验证读路径对存量数据的反应）。
async fn run_turn_on(pool: SqlitePool, scenario: MockScenario, tools: bool) -> TurnFixture {
    let mut map = std::collections::HashMap::new();
    if tools {
        map.insert("echo".to_string(), Arc::new(EchoTool) as Arc<dyn McpClient>);
    }
    run_turn_with_map(pool, scenario, map).await
}

/// 带自定义工具集跑一个完整回合（doom_loop 等场景注册恒败工具用）。
async fn run_turn_with_map(
    pool: SqlitePool,
    scenario: MockScenario,
    map: std::collections::HashMap<String, Arc<dyn McpClient>>,
) -> TurnFixture {
    let emitter = CollectEmitter::default();
    let cancel = CancellationToken::new();

    let conv: ConversationRow = repo::conversation::get_by_id(&pool, "conv-e")
        .await
        .expect("seed conv");
    let agent = repo::agent::get_by_id(&pool, "agent-1")
        .await
        .expect("seed agent");

    let tools = !map.is_empty();
    let global_registry = Arc::new(McpRegistry::from_map(map));

    let mock = Arc::new(MockProvider::new("mock-model", scenario));
    let user_msg_id = uuid::Uuid::new_v4().to_string();

    let route_registry = ReadRouteRegistry::new();
    let env = TurnEnv {
        emitter: Arc::new(emitter.clone()),
        tool_app: None,
        pool: pool.clone(),
        route_registry: &route_registry,
        global_registry,
        mcp_manager: Arc::new(McpServerManager::new()),
        auth_registry: ToolAuthRegistry::new(),
    };
    let input = AgentTurnInput {
        conv,
        agent,
        hooks: HookConfig::new(),
        word_style_profile: None,
        provider: mock.clone(),
        api_key: "e2e-key".into(),
        user_msg_id: user_msg_id.clone(),
        content_text: "hello".into(),
        llm_blocks: vec![ContentBlock::text("hello")],
        persist_blocks: vec![ContentBlock::text("hello")],
        attach_db_inputs: Vec::new(),
        attach_file_inputs: Vec::new(),
        emit_user_blocks: false,
        tools_enabled: tools,
        model_override: None,
        cancel_token: cancel.clone(),
    };
    let rx = run_agent_turn(&env, input).await.expect("turn spawn 失败");
    TurnFixture {
        pool,
        emitter,
        mock,
        user_msg_id,
        cancel,
        rx,
    }
}

/// 带护栏地等待完成信号（默认 30s——重试退避等场景留足余量）。
/// 借用 fx：rx 单独 move 进 await，调用方保留 pool/emitter/mock 继续断言。
async fn finish(fx: &mut TurnFixture) -> crate::harness::session_runner::TurnSummary {
    let rx = std::mem::replace(&mut fx.rx, tokio::sync::oneshot::channel().1);
    tokio::time::timeout(Duration::from_secs(30), rx)
        .await
        .expect("完成信号超时")
        .expect("完成信号 sender 被 drop")
}

/// 会话消息行（rowid 升序）。
async fn message_rows(pool: &SqlitePool) -> Vec<crate::db::models::MessageRow> {
    repo::message::list_by_conversation(pool, "conv-e", None, None)
        .await
        .expect("list messages")
}

/// 会话事件行（seq 升序）。
async fn event_rows(pool: &SqlitePool) -> Vec<crate::db::models::SessionEventRow> {
    repo::session_event::list_by_session(pool, "conv-e", None)
        .await
        .expect("list events")
}

fn kinds(rs: &[crate::db::models::SessionEventRow]) -> Vec<&str> {
    rs.iter().map(|r| r.kind.as_str()).collect()
}

/// 事件序不变式：seq 从 1 严格连续 + turn_id 全一致（== user_msg_id）。
fn assert_event_invariants(rs: &[crate::db::models::SessionEventRow], user_msg_id: &str) {
    for (i, r) in rs.iter().enumerate() {
        assert_eq!(
            r.seq,
            (i + 1) as i64,
            "seq 应从 1 严格连续: {:?}",
            kinds(rs)
        );
        assert_eq!(
            r.turn_id.as_deref(),
            Some(user_msg_id),
            "turn_id 应全为 user_msg_id（seq={} kind={}）",
            r.seq,
            r.kind
        );
    }
}

fn blocks_of(row: &crate::db::models::MessageRow) -> Vec<ContentBlock> {
    serde_json::from_str(&row.content_blocks).expect("content_blocks 反序列化")
}

// =========================================================================
// 场景 1：正常回合 —— 消息行 + 事件序 + TurnSummary
// =========================================================================

#[tokio::test]
async fn normal_round_persists_and_signals() {
    let mut fx = run_turn(MockScenario::NormalReply, false, "{}").await;
    let summary = finish(&mut fx).await;

    // TurnSummary（delegate 完成信号的数据面）
    assert_eq!(summary.finish_reason, "stop");
    assert_eq!(summary.final_text, "Hello from MockProvider");
    assert_eq!(summary.rounds, 1, "单轮文本回合 rounds=1");
    let usage = summary.usage.expect("NormalReply 附带 usage");
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 4);

    // 消息行：user(hello) + assistant(全文)
    let rows = message_rows(&fx.pool).await;
    assert_eq!(rows.len(), 2, "应恰有 user + assistant 两行");
    assert_eq!(rows[0].role, "user");
    assert_eq!(rows[0].content, "hello");
    assert_eq!(rows[1].role, "assistant");
    assert_eq!(rows[1].content, "Hello from MockProvider");

    // 事件序：user_message → turn_context → assistant_message → turn_ended
    let events = event_rows(&fx.pool).await;
    assert_eq!(
        kinds(&events),
        vec![
            "user_message",
            "turn_context",
            "assistant_message",
            "turn_ended"
        ],
        "正常回合的事件 kind 序"
    );
    assert_event_invariants(&events, &fx.user_msg_id);
    let ended: crate::harness::event_log::TurnEndedPayload =
        serde_json::from_str(&events[3].payload).expect("turn_ended payload");
    assert_eq!(ended.termination, "stop");

    // UI 事件：start → chunk → done(stop)；无工具/重试事件
    let names = fx.emitter.names();
    assert!(names.contains(&"chat:start".to_string()));
    assert!(names.contains(&"chat:chunk".to_string()));
    assert!(
        names.contains(&"chat:done".to_string()),
        "应含终态 chat:done: {names:?}"
    );
    // W2.4：stream_loop 返回后还有一条终态 round-state（行为快照——最后一条是它）
    assert_eq!(names.last().map(String::as_str), Some("chat:round-state"));
    assert!(!names.iter().any(|n| n.starts_with("chat:tool-")));
    assert!(!names.contains(&"chat:retrying".to_string()));
    // on_loop_exit 恰两次 = S6 双保险设计快照：finalize 内 cleanup() 一次 +
    // spawn 任务的 RAII Drop 守卫一次（生产侧对应 ChatState 注销，幂等）
    assert_eq!(
        fx.emitter.exits.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "正常收尾 on_loop_exit 应为 finalize + RAII 守卫共两次"
    );
}

// =========================================================================
// 场景 2：空响应 —— LLM 沉默时回合仍完整收尾（行为快照）
// =========================================================================

#[tokio::test]
async fn empty_response_turn_ends_with_stop() {
    let mut fx = run_turn(MockScenario::EmptyResponse, false, "{}").await;
    let summary = finish(&mut fx).await;

    assert_eq!(summary.finish_reason, "stop");
    assert_eq!(summary.final_text, "");

    let rows = message_rows(&fx.pool).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].role, "assistant");
    assert_eq!(
        rows[1].content, "",
        "空响应的 assistant 行为空（当前行为快照）"
    );

    let events = event_rows(&fx.pool).await;
    assert_eq!(
        kinds(&events),
        vec![
            "user_message",
            "turn_context",
            "assistant_message",
            "turn_ended"
        ],
        "空响应也走完整事件序（assistant_message 空内容）"
    );
    assert_event_invariants(&events, &fx.user_msg_id);
}

// =========================================================================
// 场景 3：限流重试中取消 —— retrying 事件 + abort 收尾
// =========================================================================

#[tokio::test]
async fn rate_limited_retry_cancelled_mid_backoff() {
    let mut fx = run_turn(MockScenario::RateLimited, false, "{}").await;

    // 300ms 后取消（正处第 1 次退避 sleep(1s) 中）
    let token = fx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        token.cancel();
    });
    let summary = finish(&mut fx).await;

    assert_eq!(summary.finish_reason, "abort");
    assert_eq!(summary.rounds, 0, "未成功产出任何轮");

    // 事件序：无 assistant_message（零产出），直接 turn_ended(abort)
    let events = event_rows(&fx.pool).await;
    assert_eq!(
        kinds(&events),
        vec!["user_message", "turn_context", "turn_ended"],
        "重试中取消的事件 kind 序"
    );
    assert_event_invariants(&events, &fx.user_msg_id);
    let ended: crate::harness::event_log::TurnEndedPayload =
        serde_json::from_str(&events[2].payload).expect("turn_ended payload");
    assert_eq!(ended.termination, "abort");

    // UI：chat:retrying 已发出（至少一次），终态 chat:done(abort)
    let names = fx.emitter.names();
    assert!(
        names.contains(&"chat:retrying".to_string()),
        "退避期间应 emit chat:retrying: {names:?}"
    );
    assert!(
        names.contains(&"chat:done".to_string()),
        "应含终态 chat:done: {names:?}"
    );
}

// =========================================================================
// 场景 4：预算触顶（显式硬上限）—— S8-4 后语义：先收尾轮再终止
// =========================================================================

#[tokio::test]
async fn explicit_budget_cap_terminates_with_guidance() {
    // NormalReply 的 usage(prompt=10+completion=4=14) > 显式上限 1；
    // 显式 max_total_tokens → 续期额度 0。
    // S8-4（2026-08-21）：触顶不再硬停——先给 +4096 收尾额度并注入收尾指令，
    // Mock 的后续回复为正常文本 → 收尾轮自然完成（finish_reason=stop，
    // 模型输出收尾总结）。这正是 S8-4 的设计行为：给 agent 一次收尾发言权。
    let mut fx = run_turn(
        MockScenario::NormalReply,
        false,
        r#"{"max_total_tokens":1}"#,
    )
    .await;
    let summary = finish(&mut fx).await;

    assert_eq!(
        summary.finish_reason, "stop",
        "S8-4：触顶先走收尾轮（+4096 额度注入收尾指令），Mock 正常回文本 → 自然 stop"
    );

    // 守卫语义快照：本轮有真实文本（NormalReply）→ fallback 被忽略、
    // 保留模型文本（fallback 只救「无 Text 的纯 tool_use/thinking-only 轮」）
    assert_eq!(
        summary.final_text, "Hello from MockProvider",
        "有真实文本时 budget fallback 不抢占: {}",
        summary.final_text
    );

    let rows = message_rows(&fx.pool).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].role, "assistant");
    assert_eq!(rows[1].content, "Hello from MockProvider");

    let events = event_rows(&fx.pool).await;
    assert_eq!(
        kinds(&events),
        vec![
            "user_message",
            "turn_context",
            "assistant_message",
            "turn_ended"
        ],
        "预算触顶的事件 kind 序"
    );
    assert_event_invariants(&events, &fx.user_msg_id);
    let ended: crate::harness::event_log::TurnEndedPayload =
        serde_json::from_str(&events[3].payload).expect("turn_ended payload");
    // S8-4：触顶走收尾轮后自然完成 → termination=stop（收尾轮有真实文本输出）
    assert_eq!(ended.termination, "stop");

    // UI：终态 chat:budget + chat:done(budget_exceeded)
    let names = fx.emitter.names();
    assert!(names.contains(&"chat:budget".to_string()));
    assert!(
        names.contains(&"chat:done".to_string()),
        "应含终态 chat:done: {names:?}"
    );
}

// =========================================================================
// 场景 5：流中取消 —— 占位行 discard + abort 收尾
// =========================================================================

#[tokio::test]
async fn cancel_mid_stream_discards_placeholder() {
    let mut fx = run_turn(MockScenario::Timeout, false, "{}").await;

    // 150ms 后取消（Timeout 场景挂起中，cancel 后 yield Done{abort}）
    let token = fx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        token.cancel();
    });
    let summary = finish(&mut fx).await;

    assert_eq!(summary.finish_reason, "abort");

    // 事件序：无产出的空占位 → message_discarded（对账无「零事件行」）
    let events = event_rows(&fx.pool).await;
    assert_eq!(
        kinds(&events),
        vec![
            "user_message",
            "turn_context",
            "message_discarded",
            "turn_ended"
        ],
        "流中取消的事件 kind 序（占位 discard）"
    );
    assert_event_invariants(&events, &fx.user_msg_id);

    // assistant 占位行已删，只剩 user 行
    let rows = message_rows(&fx.pool).await;
    assert_eq!(rows.len(), 1, "空占位应被删除");
    assert_eq!(rows[0].role, "user");
}

// =========================================================================
// 场景 6：工具轮 —— tool_use ↔ tool_execution ↔ tool_result 配对 + 二轮收尾
// =========================================================================

#[tokio::test]
async fn tool_round_pairs_use_execution_result() {
    let mut fx = run_turn(
        MockScenario::ToolCallThenText {
            tool_name: "echo".into(),
            arguments: r#"{"msg":"hi"}"#.into(),
        },
        true,
        "{}",
    )
    .await;
    let summary = finish(&mut fx).await;

    // 二轮收尾：工具轮 + 文本轮
    assert_eq!(summary.finish_reason, "stop");
    assert_eq!(summary.final_text, "Tool finished. Final answer from mock.");
    assert_eq!(summary.rounds, 2, "工具轮 + 收尾轮 = 2");
    assert_eq!(fx.mock.call_count(), 2, "provider 应被调用两次");

    // 消息行：user / assistant(tool_use) / user(tool_result) / assistant(终答)
    let rows = message_rows(&fx.pool).await;
    assert_eq!(
        rows.len(),
        4,
        "四行消息: {:#?}",
        rows.iter().map(|r| &r.role).collect::<Vec<_>>()
    );
    assert_eq!(rows[0].role, "user");
    assert_eq!(rows[1].role, "assistant");
    let asst1 = blocks_of(&rows[1]);
    match asst1
        .iter()
        .find(|b| matches!(b, ContentBlock::ToolUse { .. }))
    {
        Some(ContentBlock::ToolUse { id, name, input }) => {
            assert_eq!(id, "mock_tool_call_1");
            assert_eq!(name, "echo");
            assert_eq!(input, r#"{"msg":"hi"}"#);
        }
        other => panic!("assistant 首轮 blocks 应含 ToolUse: {other:?}"),
    }
    assert_eq!(
        rows[2].role, "user",
        "tool_result 须在 user 消息里（Anthropic 协议）"
    );
    match blocks_of(&rows[2]).first() {
        Some(ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        }) => {
            assert_eq!(tool_use_id, "mock_tool_call_1");
            assert_eq!(content, r#"echo: {"msg":"hi"}"#);
            assert_eq!(*is_error, Some(false));
        }
        other => panic!("tool_result 行 blocks 应为 ToolResult: {other:?}"),
    }
    assert_eq!(rows[3].role, "assistant");
    assert_eq!(rows[3].content, "Tool finished. Final answer from mock.");

    // 事件序：两轮 assistant_message 中间夹 tool_execution + tool_result_message
    let events = event_rows(&fx.pool).await;
    assert_eq!(
        kinds(&events),
        vec![
            "user_message",
            "turn_context",
            "assistant_message",
            "tool_execution",
            "tool_result_message",
            "assistant_message",
            "turn_ended",
        ],
        "工具轮的事件 kind 序"
    );
    assert_event_invariants(&events, &fx.user_msg_id);
    // 配对：tool_execution.tool_use_id 指向 mock 工具调用 id
    let exec: crate::harness::event_log::ToolExecutionPayload =
        serde_json::from_str(&events[3].payload).expect("tool_execution payload");
    assert_eq!(exec.tool_use_id.as_deref(), Some("mock_tool_call_1"));
    assert_eq!(exec.tool_name, "echo");
    assert_eq!(exec.result.as_deref(), Some(r#"echo: {"msg":"hi"}"#));

    // UI：工具流事件 + 二轮占位 + 终态
    let names = fx.emitter.names();
    for expected in [
        "chat:tool-call-start",
        "chat:tool-call-delta",
        "chat:tool-call-end",
        "chat:tool-result",
        "chat:assistant-start",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "UI 事件应含 {expected}: {names:?}"
        );
    }
    assert!(
        names.contains(&"chat:done".to_string()),
        "应含终态 chat:done: {names:?}"
    );
}

// =========================================================================
// 场景 6b：doom_loop 恒败连击 —— D15 八波⑤ nudge 逐次注入（3/4/5）+ 终止
//
// 形态：模型无视失败连续调同一工具（ToolCallRepeat）× 恒败工具。
// 预期：streak 3/4/5 各注入一次 nudge（3 次，其中 4/5 带升级段——`>=` 修复前
// 只有 streak 3 的一次）；streak 6 触发 TERMINATE_AT → finish_reason=doom_loop，
// 第 7 次调用永不发生。
// =========================================================================

#[tokio::test]
async fn doom_loop_constant_failure_nudges_each_time_then_terminates() {
    let pool = seeded_pool("{}").await;
    let mut map = std::collections::HashMap::new();
    map.insert(
        "always_fail".to_string(),
        Arc::new(AlwaysFailTool) as Arc<dyn McpClient>,
    );
    let mut fx = run_turn_with_map(
        pool,
        MockScenario::ToolCallRepeat {
            tool_name: "always_fail".into(),
            // {round} 占位：每轮参数不同（stuck 轮指纹随之变化，抢不走
            // doom_loop 的戏——这正是 doom_detect 相对 stuck_detect 的靶形态）
            arguments: r#"{"x":{round}}"#.into(),
            times: 7,
        },
        map,
    )
    .await;
    let summary = finish(&mut fx).await;

    assert_eq!(summary.finish_reason, "doom_loop", "连败 6 次须终止回合");
    assert_eq!(
        fx.mock.call_count(),
        6,
        "TERMINATE_AT=6：第 7 次调用在终止后才不会发生"
    );

    let events = event_rows(&fx.pool).await;
    // 恒败执行审计：6 条全部 is_error
    let fail_execs: Vec<&crate::db::models::SessionEventRow> = events
        .iter()
        .filter(|r| r.kind == "tool_execution")
        .collect();
    assert_eq!(fail_execs.len(), 6, "6 轮恒败各留一条 tool_execution 审计");
    for e in &fail_execs {
        let p: serde_json::Value = serde_json::from_str(&e.payload).expect("payload");
        assert_eq!(p["is_error"], true, "恒败工具的审计行全部 is_error");
        assert_eq!(p["tool_name"], "always_fail");
    }

    // nudge 注入审计：hook_injected(point=doom_loop_nudge) 恰 3 次（streak 3/4/5）
    let nudges: Vec<&crate::db::models::SessionEventRow> = events
        .iter()
        .filter(|r| r.kind == "hook_injected")
        .collect();
    assert_eq!(
        nudges.len(),
        3,
        "`>=` 升级（D15 前 `==` 只注入 1 次）：连败 3/4/5 各一次"
    );
    for (i, e) in nudges.iter().enumerate() {
        let p: serde_json::Value = serde_json::from_str(&e.payload).expect("payload");
        assert_eq!(p["point"], "doom_loop_nudge", "注入点不变（skip 事件零迁移）");
        let streak = 3 + i;
        let prompt = p["prompt"].as_str().expect("prompt 文本");
        assert!(
            prompt.contains(&format!("已连续 {streak} 次以同类方式失败")),
            "第 {i} 条 nudge 须报实际连败数 {streak}: {prompt}"
        );
        let escalated = streak > 3;
        assert_eq!(
            prompt.contains("[升级指令]"),
            escalated,
            "streak={streak} {}升级段",
            if escalated { "须带" } else { "不带" }
        );
    }

    // 落库的 tool_result 行：nudge 文案进了模型可见内容（下轮历史）——
    // [System] 出现 3 次、其中带升级段的 2 次（streak 6 走终止分支无 nudge）
    let rows = message_rows(&fx.pool).await;
    let mut system_nudges = 0;
    let mut escalated = 0;
    for row in &rows {
        for block in blocks_of(row) {
            if let ContentBlock::ToolResult { content, .. } = block {
                if content.contains("[System]") {
                    system_nudges += 1;
                    if content.contains("[升级指令]") {
                        escalated += 1;
                    }
                }
            }
        }
    }
    assert_eq!(
        system_nudges, 3,
        "三轮 nudge 追加进 tool_result 内容（模型下一轮可见）"
    );
    assert_eq!(escalated, 2, "streak 4/5 两轮带升级段");

    assert_event_invariants(&events, &fx.user_msg_id);
    let names = fx.emitter.names();
    assert!(
        names.contains(&"chat:done".to_string()),
        "应含终态 chat:done: {names:?}"
    );
}

// =========================================================================
// 场景 7：零事件旧行会话 —— Phase 2B legacy 读路径退役的行为锁定。
//
// 形态：会话有 pre-Phase-0 旧行（无事件），boot backfill 未覆盖的残留
// （如 backfill 后 SQLite 异常降级）。退役后这是**降级但诚实**的行为：
// 派生历史为空 → LLM 看不到旧行 → 回合照常完成；旧行本身不动
// （前端 list_messages 照常显示）；下次 boot 会被 backfill 捕获自愈。
// =========================================================================

/// 预置零事件旧行（不经事件日志的裸行——pre-Phase-0 形态）。
async fn seed_legacy_rows_without_events(pool: &SqlitePool) {
    for (id, role, text) in [("old-u", "user", "旧问题"), ("old-a", "assistant", "旧回答")] {
        repo::message::create(
            pool,
            id,
            &crate::db::models::NewMessage {
                conversation_id: "conv-e".into(),
                role: role.into(),
                content: text.into(),
                token_count: None,
                error: None,
                model: None,
            },
        )
        .await
        .expect("seed legacy row");
        let blocks = serde_json::to_string(&[ContentBlock::text(text)]).unwrap();
        repo::message::update_content_blocks(pool, id, &blocks)
            .await
            .expect("seed legacy blocks");
    }
}

#[tokio::test]
async fn legacy_rows_without_events_yield_empty_history_but_turn_completes() {
    let pool = seeded_pool("{}").await;
    seed_legacy_rows_without_events(&pool).await;
    let mut fx = run_turn_on(pool, MockScenario::NormalReply, false).await;
    let summary = finish(&mut fx).await;

    // 回合照常完成（不 panic、不 Err——降级是「缺历史」不是「崩」）
    assert_eq!(summary.finish_reason, "stop");
    assert_eq!(summary.final_text, "Hello from MockProvider");

    // 最硬证据：LLM 实际收到的 messages 里**没有**旧行（派生历史为空），
    // 只有本轮输入（system 前缀 + user("hello")）。
    let received = fx.mock.received_messages();
    assert_eq!(received.len(), 1, "NormalReply 单次调用");
    let texts: Vec<String> = received[0].iter().map(|m| m.content_text()).collect();
    assert!(
        !texts.iter().any(|t| t.contains("旧问题") || t.contains("旧回答")),
        "零事件旧行不应进入 LLM 历史: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("hello")),
        "本轮输入应在: {texts:?}"
    );

    // 旧行原样保留（前端 list_messages 路径不受读路径退役影响）
    let rows = message_rows(&fx.pool).await;
    assert_eq!(rows.len(), 4, "旧行×2 + 本轮 user/assistant: {rows:#?}");
    assert_eq!(rows[0].id, "old-u");
    assert_eq!(rows[1].id, "old-a");

    // 本轮事件从 seq=1 正常写入（该会话首个事件纪元，append-only 语义不受旧行影响）
    let events = event_rows(&fx.pool).await;
    assert_eq!(
        kinds(&events),
        vec![
            "user_message",
            "turn_context",
            "assistant_message",
            "turn_ended"
        ]
    );
    assert_event_invariants(&events, &fx.user_msg_id);
}
