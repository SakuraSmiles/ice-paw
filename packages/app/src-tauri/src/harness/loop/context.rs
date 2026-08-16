//! `stream_loop` 的输入配置封装 — `LoopConfig`（不可变配置）+ `LoopContext`（配置 + 可变运行时件 + 可变消息缓冲）。
//!
//! 从 `harness::loop_engine` 原样迁入（拆分大文件，纯搬运，无逻辑变更）。
//! 通过 `loop_engine` 的 re-export 保持调用方 import 路径不变。

use std::sync::Arc;

use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::db::models::HookConfig;
use crate::harness::authority::{PathAuthSession, PathWhitelistConfig};
use crate::harness::budget::LoopBudget;
use crate::harness::chat_state::CancellationToken;
use crate::harness::mcp::McpRegistry;
use crate::harness::tool_executor::ToolAuthRegistry;
use crate::infra::protocol::{ChatMessage, LlmProvider};

/// `stream_loop` 的输入配置封装。
///
/// 13 个原本独立的参数（app / pool / provider / api_key / messages /
/// temperature / max_tokens / cancel / conv_id / asst_msg_id /
/// tool_registry / tools_enabled / budget）整合到一个结构体中：
/// - 消除 `clippy::too_many_arguments`
/// - 让 `stream_loop` 的 signature 保持 `fn(&mut LoopContext, &mut RoundState)`
/// - 为后续扩展（如加上 tools 缓存、agent 配置、continue-from 等）提供容器
///
/// `RoundState`（observable）刻意未收入此结构体，因为它是循环过程中
/// 累积写入的**输出**遥测状态，而不是配置输入。
///
/// 对话循环的不可变配置（从 LoopContext 拆分，消除 24 参数构造器）。
///
/// 创建后不被循环修改（S4：auth_registry / auth_session 两个**运行时可变**件
/// 已挪到 [`LoopContext`]，本结构体的不可变声明现在为真）。通过 `LoopContext`
/// 的 `Deref` 透明访问。
pub(crate) struct LoopConfig {
    // ---- 标识与会话 ----
    pub conv_id: String,
    pub asst_msg_id: String,
    /// M1.3: 用户消息 ID（用于清理阶段回写 token_count）
    pub user_msg_id: String,
    /// RAG: 当前 Agent ID（透传给 ToolContext）
    pub agent_id: String,
    /// RAG: 当前项目 ID
    pub project_id: Option<String>,

    // ---- 基础设施 ----
    pub app: AppHandle,
    pub pool: SqlitePool,

    // ---- LLM Provider ----
    pub provider: Arc<dyn LlmProvider>,
    pub api_key: String,
    pub temperature: f64,
    pub max_tokens: i32,

    // ---- 工具 ----
    pub tool_registry: McpRegistry,
    pub tools_enabled: bool,
    pub whitelist: PathWhitelistConfig,

    // ---- 循环控制 ----
    pub cancel: CancellationToken,
    pub budget: LoopBudget,

    // ---- M1.2: 工具裁剪 ----
    pub query: Option<String>,
    pub call_history: Vec<String>,

    // ---- P0-3: 会话级 model override ----
    pub model: Option<String>,
    pub asst_model: Option<String>,

    // ---- 对话钩子 ----
    pub hooks: HookConfig,
}

/// 对话循环上下文：不可变配置 + 可变运行时件 + 可变消息缓冲。
///
/// 通过 `Deref<Target = LoopConfig>` 透明访问配置字段（`ctx.pool`、
/// `ctx.app` 等）；`auth_registry` / `auth_session` / `messages` 是循环中
/// 实际变异的运行时状态，直接挂在本结构体上（自有字段优先于 Deref，历史
/// 访问点 `ctx.auth_registry` 等无需改动）。构造时传入 `LoopConfig`。
pub(crate) struct LoopContext {
    pub config: LoopConfig,
    /// 工具授权 oneshot 注册表（A2-3）：循环中 register/take 配对使用（运行时变异）。
    pub auth_registry: ToolAuthRegistry,
    /// 会话级已授权路径表（A2-3）：工具授权流程累积写入，循环收尾 clear（运行时变异）。
    pub auth_session: PathAuthSession,
    pub messages: Vec<ChatMessage>,
}

impl std::ops::Deref for LoopContext {
    type Target = LoopConfig;
    fn deref(&self) -> &LoopConfig {
        &self.config
    }
}

impl LoopContext {
    pub(crate) fn new(
        config: LoopConfig,
        auth_registry: ToolAuthRegistry,
        auth_session: PathAuthSession,
        messages: Vec<ChatMessage>,
    ) -> Self {
        Self {
            config,
            auth_registry,
            auth_session,
            messages,
        }
    }
}
