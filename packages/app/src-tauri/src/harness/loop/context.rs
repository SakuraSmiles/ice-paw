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
/// 已挪到 [`LoopContext`]；B2-S1：模型档位五字段（provider/api_key/model/
/// asst_model/max_tokens）同因可变挪出为 [`RuntimeModel`]——本结构体的不可变
/// 声明现在为真）。通过 `LoopContext` 的 `Deref` 透明访问。
pub(crate) struct LoopConfig {
    // ---- 标识与会话 ----
    pub conv_id: String,
    pub asst_msg_id: String,
    /// M1.3: 用户消息 ID（用于清理阶段回写 token_count）
    pub user_msg_id: String,
    /// RAG: 当前 Agent ID（透传给 ToolContext）
    pub agent_id: String,
    /// 频道 v1：执行成员名字快照（C10 双轨 sender 事件侧——loop_engine 构造
    /// EventCtx 时 `.with_sender_name` 注入，进 assistant_message payload.sender；
    /// 1v1 回合 None = 隐含会话 agent，零标注）。
    pub sender_agent_name: Option<String>,
    /// 频道 v1：执行成员 id（C10 双轨 sender **行侧**出生打标——首占位在
    /// session_runner 落库时、多轮工具回合的后续占位在 loop_engine 落库时经
    /// `set_sender_agent` 写入 `messages.sender_agent_id`。出生即带身份 → live
    /// 视图（chat:start 触发的 loadMessages）第一时间显示头像昵称，且 sweep
    /// 的 `IS NULL` 守卫天然跳过。仅 conv.kind=='channel' 有值；1v1 回合 None
    /// = 会话 agent 隐含归属，零标注零开销）。
    pub sender_agent_id: Option<String>,
    /// RAG: 当前项目 ID
    pub project_id: Option<String>,

    // ---- 基础设施 ----
    /// 对外进度事件出口（S6：取代裸 `AppHandle`——循环不再依赖 Tauri 运行时，
    /// 集成测试用收集型实现即可跑全链路）。瞬态 UI 事件专用；可回放事实走 event_log。
    pub emitter: Arc<dyn crate::harness::r#loop::emitter::LoopEmitter>,
    /// 工具上下文注入用的真 `AppHandle`（生产 `Some`；测试 `None`——依赖它的
    /// 工具（proposal / delegate）对 `None` 已有「需要 App 上下文」降级报错）。
    /// 这是循环链仅剩的 Tauri 句柄，仅透传给 `ToolContext.app_handle`。
    pub tool_app: Option<AppHandle>,
    pub pool: SqlitePool,

    // ---- LLM 生成参数（档位无关——降级链换档不变，见 RuntimeModel） ----
    pub temperature: f64,

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

    // ---- 对话钩子 ----
    pub hooks: HookConfig,

    // ---- ③ 可观测化：上下文组成 + miss 归因 ----
    /// Pipeline 后 `build_anatomy` 的段级产物 + 跨回合基线（session_runner
    /// 组装；Default 空 = 散落构造，归因诚实降级）。经 [`LoopContext::new`]
    /// 转入 `turn_cost` 记录器。
    pub context_anatomy: crate::harness::r#loop::turn_cost::ContextAnatomyInput,
}

/// 运行时模型档位（B2-S1 拆出）：provider 适配器 + 凭据 + 模型名 + 输出上限。
///
/// 一轮对话内**可变**——降级链（B2-S3）失败换档时整体替换——故不进不可变的
/// [`LoopConfig`]，而是经 [`LoopContext::new`] 解构平铺为 LoopContext 自有字段
/// （auth_registry 遮蔽 Deref 同款先例，历史 `ctx.provider` 等访问点零改动）。
pub(crate) struct RuntimeModel {
    pub provider: Arc<dyn LlmProvider>,
    pub api_key: String,
    pub model: Option<String>,
    pub asst_model: Option<String>,
    pub max_tokens: i32,
}

/// 对话循环上下文：不可变配置 + 可变运行时件 + 可变消息缓冲。
///
/// 通过 `Deref<Target = LoopConfig>` 透明访问配置字段（`ctx.pool`、
/// `ctx.app` 等）；`auth_registry` / `auth_session` / `messages` 及五个模型档位
/// 字段（B2-S1，见 [`RuntimeModel`]）是循环中实际变异的运行时状态，直接挂在
/// 本结构体上（自有字段优先于 Deref，历史访问点 `ctx.auth_registry` /
/// `ctx.provider` 等无需改动）。构造时传入 `LoopConfig` + `RuntimeModel`。
pub(crate) struct LoopContext {
    pub config: LoopConfig,
    // ---- 运行时模型档位（B2-S1）：降级链换档时逐字段替换 ----
    pub provider: Arc<dyn LlmProvider>,
    pub api_key: String,
    pub model: Option<String>,
    pub asst_model: Option<String>,
    pub max_tokens: i32,
    /// 工具授权 oneshot 注册表（A2-3）：循环中 register/take 配对使用（运行时变异）。
    pub auth_registry: ToolAuthRegistry,
    /// 会话级已授权路径表（A2-3）：工具授权流程累积写入，循环收尾 clear（运行时变异）。
    pub auth_session: PathAuthSession,
    pub messages: Vec<ChatMessage>,
    /// 降级链状态（B2-S2）：cursor 单调前进 / 激活档位更新发生在换档时
    /// （B2-S3）。空链 = 无降级（legacy 行为，换档尝试天然 no-op）。
    pub fallback: crate::harness::r#loop::fallback::FallbackPlan,
    /// ③ 可观测化：回合级开销记录器（工具指纹逐轮 / usage 逐轮 / miss 归因 /
    /// context_breakdown 组装）。从 `config.context_anatomy` 构造；emit 单点在
    /// `stream_loop` wrapper（inner 返回后 `into_payload` → `log_context_breakdown`）。
    pub turn_cost: crate::harness::r#loop::turn_cost::TurnCostRecorder,
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
        model: RuntimeModel,
        auth_registry: ToolAuthRegistry,
        auth_session: PathAuthSession,
        messages: Vec<ChatMessage>,
        fallback: crate::harness::r#loop::fallback::FallbackPlan,
    ) -> Self {
        let mut config = config;
        let RuntimeModel {
            provider,
            api_key,
            model,
            asst_model,
            max_tokens,
        } = model;
        // anatomy 从 config 搬出（非 clone：segments/基线只此一份，记录器独占）
        // config 此后只进 Deref 不可变面——take 不破坏其「构造后不可变」约定。
        let turn_cost = crate::harness::r#loop::turn_cost::TurnCostRecorder::new(std::mem::take(
            &mut config.context_anatomy,
        ));
        Self {
            config,
            provider,
            api_key,
            model,
            asst_model,
            max_tokens,
            auth_registry,
            auth_session,
            messages,
            fallback,
            turn_cost,
        }
    }
}
