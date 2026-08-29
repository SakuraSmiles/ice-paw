//! 主循环的对外事件出口（S6 / A3：loop_engine 去 `AppHandle` 硬依赖）。
//!
//! 主循环链（loop_engine / session_runner / cleanup / stream_consumer /
//! tool_executor）此前直接持有 `tauri::AppHandle` 发进度事件，导致**不启动
//! Tauri 运行时就无法运行循环**——集成测试（S5 全链路 e2e）无从下手。
//!
//! 现在循环只依赖本 trait：
//! - 生产实现 [`TauriEmitter`]：包一个 `AppHandle`，`emit` 即 webview 事件；
//!   `on_loop_exit` 注销 ChatState 的 cancel token（原 RAII 守卫职责）。
//! - 测试实现：收集事件到 `Mutex<Vec>` 断言序列（见 loop_e2e_tests）。
//!
//! 与 `session_events` 事件日志的分工：本出口是**瞬态 UI 进度**（chat:chunk /
//! chat:round-state / 授权弹窗），失败仅 warn 不落库；跨会话可回放的事实走
//! `harness::event_log`（append-only，inline await）——两条通道勿混。
//!
//! **事件节奏约定（不变式）**：token 级进度事件（`chat:chunk` / `chat:thinking` /
//! `chat:tool-call-delta`）**禁止**在流式消费循环里逐条 emit——必须经
//! `stream_consumer::DeltaAggregator` 按 40ms 窗口聚合后发出。Windows/WebView2 上
//! 后端 emit 的 JS 注入走主线程，逐 delta emit 会打满主线程（同步命令 / IPC 分发
//! 全体排队）并触发前端全列表重渲染，低配机器放大为生成中全局卡顿。新增 token
//! 级事件时：内容为可拼接字符串 ⇒ 进 DeltaAggregator；否则先评估发射频率再定。

use std::sync::Arc;

use serde::Serialize;

/// 主循环对外事件出口。实现须 `Send + Sync`（循环跑在 tokio::spawn 任务里）。
pub(crate) trait LoopEmitter: Send + Sync {
    /// 发一条进度事件（`chat:chunk` / `chat:round-state` / `chat:tool-auth-request` …）。
    /// 实现自行处理失败（生产 warn 一次，不向上传播——与原 `let _ = app.emit` 语义一致）。
    fn emit(&self, event: &str, payload: serde_json::Value);

    /// 流式循环任务退出时的善后（无论正常 / panic / 被取消）。
    /// 生产实现：从 `ChatState` 注销本会话 cancel token（原 RAII Drop 守卫）。
    fn on_loop_exit(&self) {}
}

/// 便捷包装：任意 `Serialize` 负载 → [`LoopEmitter::emit`]。
///
/// 序列化失败仅 warn 并发 `Value::Null`（payload 定义错误不该打断流式循环——
/// 与 emit 失败同等对待）。
pub(crate) fn emit_ser<E: Serialize + ?Sized>(emitter: &dyn LoopEmitter, event: &str, payload: &E) {
    match serde_json::to_value(payload) {
        Ok(v) => emitter.emit(event, v),
        Err(e) => {
            tracing::warn!(target: "ice_paw.loop", event, "进度事件序列化失败: {e}");
            emitter.emit(event, serde_json::Value::Null);
        }
    }
}

/// 生产实现：转发到 Tauri webview + 退出时注销 ChatState。
pub(crate) struct TauriEmitter {
    pub app: tauri::AppHandle,
    /// 退出善次要注销的会话（ChatState key）。
    pub conv_id: String,
}

impl LoopEmitter for TauriEmitter {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        use tauri::Emitter as _;
        if let Err(e) = self.app.emit(event, payload) {
            tracing::warn!(target: "ice_paw.loop", event, "进度事件发送失败（忽略）: {e}");
        }
    }

    fn on_loop_exit(&self) {
        use tauri::Manager as _;
        let chat_state = self.app.state::<crate::harness::chat_state::ChatState>();
        chat_state.unregister(&self.conv_id);
        // 屏幕通道写令牌归还（§4.3 持有粒度=回合）：归属检查 + 队列摘除 +
        // 队头授予全在 `release_write` 内；通道 Off 时是空操作。
        crate::harness::mcp::screen::channel::global().release_write(&self.conv_id);
    }
}

/// 构造生产 emitter 的便捷函数。
pub(crate) fn tauri_emitter(
    app: tauri::AppHandle,
    conv_id: impl Into<String>,
) -> Arc<dyn LoopEmitter> {
    Arc::new(TauriEmitter {
        app,
        conv_id: conv_id.into(),
    })
}
