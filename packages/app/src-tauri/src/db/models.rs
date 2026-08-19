//! 数据库行结构（`FromRow`）与对外传输结构（`Serialize/Deserialize`）
//!
//! 关键点：
//! - `Agent` 的 `api_key_ref` 和 `base_url` 用 `#[serde(skip_serializing)]` 屏蔽，
//!   前端 `list_agents` 永远不会拿到敏感字段
//! - 列表返回类型 `Agent` 与数据库 `AgentRow` 解耦，可在未来加过滤字段不影响持久化
//! - 时间字段统一用 `DateTime<Utc>`，序列化时走 RFC3339

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// 数据库行版本：包含全部字段（包括敏感引用）
#[derive(Debug, Clone, FromRow)]
pub struct AgentRow {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    pub system_prompt: String,
    pub api_key_ref: String,
    pub base_url: Option<String>,
    pub temperature: f64,
    pub max_tokens: i32,
    pub extra_params: String,
    pub sort_order: i32,
    /// P2-3: 是否启用 Anthropic prompt caching（OpenAI 自动缓存无需此字段）
    pub cache_prompt: i32,
    /// Phase 2: MemoryStage 的 **keep_n 地板**（verbatim 保留窗，NULL = 系统默认 20）。
    /// 最后 keep_n 条消息永远原样发送、永不被摘要压缩。语义自 Phase 2 起从
    /// 「加载/发送上限」重定义为此（加载改用固定 `MEMORY_LOAD_LIMIT`，
    /// 发送上限由 token 窗口 + 摘要两级控制）。
    pub max_history_messages: Option<i32>,
    /// Phase 0: 模型上下文窗口（token 数，NULL = 运行时按 provider+model 查
    /// 已知默认表，查不到回退 128K）。显式设置可覆盖默认（自定义/本地模型）。
    pub context_window: Option<i32>,
    /// Task 4: 工具白名单（NULL = 全部启用）。
    /// JSON 数组格式：`["read_file", "list_directory"]`
    pub enabled_tools: Option<String>,
    /// 是否支持图片输入（0 = 不支持, 1 = 支持）
    pub supports_vision: i32,
    /// M2-1: Agent 描述
    pub description: String,
    /// M2-1: Agent 头像（URL 或 base64）
    pub avatar: Option<String>,
    /// Phase 3: 工作区目录路径（存放 agent.yaml 的本地目录）
    pub workspace_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// =========================================================================
// 对话钩子（hooks）— 生命周期回调，内置动作（inject_prompt / call_tool / log）
// 由 agent.yaml 配置，见 AgentFileConfig.hooks
// =========================================================================

/// 钩子触发时机（对话生命周期点）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookPoint {
    /// 对话开始（send_message 入口，上下文拼装后）
    ConversationStart,
    /// 每轮 LLM 请求前（stream_chat 前）
    BeforeLlm,
    /// 每次工具执行后（dispatch 后）
    AfterTool,
    /// 对话结束（finalize）
    ConversationEnd,
}

/// 钩子动作（内置，选 + 配参数；非脚本，安全可控）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HookAction {
    /// 注入 prompt 片段（追加到 system/上下文）
    InjectPrompt { content: String },
    /// 自动调用某工具（args 为 JSON 字符串）
    CallTool { tool: String, args: String },
    /// 记日志（tracing，target=ice_paw.hooks）
    Log { message: String },
}

/// 钩子配置：HookPoint → 动作列表
pub type HookConfig = HashMap<HookPoint, Vec<HookAction>>;

/// agent.yaml 文件配置（行为层，覆盖 DB 中的对应字段）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentFileConfig {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<i32>,
    #[serde(default)]
    pub cache_prompt: Option<bool>,
    #[serde(default)]
    pub supports_vision: Option<bool>,
    #[serde(default)]
    pub max_history_messages: Option<i32>,
    /// Phase 0: 模型上下文窗口（None = 运行时按 provider+model 查默认表）
    #[serde(default)]
    pub context_window: Option<i32>,
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    #[serde(default)]
    pub extra_params: Option<serde_json::Value>,
    /// 工具调用最大轮数（None = 使用系统默认 50）
    #[serde(default)]
    pub tool_max_rounds: Option<u32>,
    /// Token 预算上限（None = 按上下文窗口自适应 3×，由 chat_cmd 兜底）
    #[serde(default)]
    pub max_total_tokens: Option<usize>,
    /// 对话钩子（生命周期回调，由 agent.yaml 配置；见 [`HookConfig`]）
    #[serde(default)]
    pub hooks: Option<HookConfig>,
}

impl AgentRow {
    /// 从 extra_params JSON 中读取工具调用最大轮数（None = 使用系统默认）
    pub fn tool_max_rounds(&self) -> Option<u32> {
        let params: serde_json::Value = serde_json::from_str(&self.extra_params).ok()?;
        params
            .get("tool_max_rounds")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
    }

    /// 从 extra_params JSON 中读取 Token 预算上限（None = 使用系统默认）
    pub fn max_total_tokens(&self) -> Option<usize> {
        let params: serde_json::Value = serde_json::from_str(&self.extra_params).ok()?;
        params
            .get("max_total_tokens")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
    }

    /// 若 workspace_path 存在，尝试读取 `<workspace_path>/agent.yaml` 并解析为文件配置。
    /// 文件不存在或解析失败静默返回 None（向后兼容）。
    pub fn load_file_config(&self) -> Option<AgentFileConfig> {
        let dir = self.workspace_path.as_ref()?;
        let yaml_path = std::path::Path::new(dir).join("agent.yaml");
        let content = std::fs::read_to_string(yaml_path).ok()?;
        serde_yaml::from_str(&content).ok()
    }
}

impl AgentFileConfig {
    /// 把文件配置合并到 Agent 中（文件配置优先覆盖 DB 值）
    pub fn apply_to(&self, agent: &mut Agent) {
        if let Some(v) = &self.description {
            agent.description = v.clone();
        }
        if let Some(v) = &self.system_prompt {
            agent.system_prompt = v.clone();
        }
        if let Some(v) = self.temperature {
            agent.temperature = v;
        }
        if let Some(v) = self.max_tokens {
            agent.max_tokens = v;
        }
        if let Some(v) = self.cache_prompt {
            agent.cache_prompt = v;
        }
        if let Some(v) = self.supports_vision {
            agent.supports_vision = v;
        }
        if let Some(v) = self.max_history_messages {
            agent.max_history_messages = Some(v);
        }
        if let Some(v) = self.context_window {
            agent.context_window = Some(v);
        }
        if let Some(v) = &self.enabled_tools {
            agent.enabled_tools = Some(v.clone());
        }
        if let Some(v) = &self.extra_params {
            agent.extra_params = v.clone();
        }
        if let Some(v) = self.tool_max_rounds {
            if let Some(obj) = agent.extra_params.as_object_mut() {
                obj.insert("tool_max_rounds".into(), serde_json::json!(v));
            }
        }
        if let Some(v) = self.max_total_tokens {
            if let Some(obj) = agent.extra_params.as_object_mut() {
                obj.insert("max_total_tokens".into(), serde_json::json!(v));
            }
        }
    }

    /// 把文件配置合并到 AgentRow 中（供 chat_cmd 内部使用）
    pub fn apply_to_row(&self, row: &mut AgentRow) {
        if let Some(v) = &self.description {
            row.description = v.clone();
        }
        if let Some(v) = &self.system_prompt {
            row.system_prompt = v.clone();
        }
        if let Some(v) = self.temperature {
            row.temperature = v;
        }
        if let Some(v) = self.max_tokens {
            row.max_tokens = v;
        }
        if let Some(v) = self.cache_prompt {
            row.cache_prompt = if v { 1 } else { 0 };
        }
        if let Some(v) = self.supports_vision {
            row.supports_vision = if v { 1 } else { 0 };
        }
        if let Some(v) = self.max_history_messages {
            row.max_history_messages = Some(v);
        }
        if let Some(v) = self.context_window {
            row.context_window = Some(v);
        }
        if let Some(v) = &self.enabled_tools {
            row.enabled_tools = Some(serde_json::to_string(v).unwrap_or_default());
        }
        if let Some(v) = &self.extra_params {
            row.extra_params = serde_json::to_string(v).unwrap_or_default();
        }
        if let Some(v) = self.tool_max_rounds {
            let mut params: serde_json::Value = serde_json::from_str(&row.extra_params)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            if let Some(obj) = params.as_object_mut() {
                obj.insert("tool_max_rounds".into(), serde_json::json!(v));
                row.extra_params = serde_json::to_string(&params).unwrap_or_default();
            }
        }
        if let Some(v) = self.max_total_tokens {
            let mut params: serde_json::Value = serde_json::from_str(&row.extra_params)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            if let Some(obj) = params.as_object_mut() {
                obj.insert("max_total_tokens".into(), serde_json::json!(v));
                row.extra_params = serde_json::to_string(&params).unwrap_or_default();
            }
        }
    }
}

/// 前端可见的 `Agent`（不含敏感引用）
#[derive(Debug, Clone, Serialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    pub system_prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub temperature: f64,
    pub max_tokens: i32,
    pub extra_params: serde_json::Value,
    pub sort_order: i32,
    /// P2-3: 是否启用 prompt caching（Anthropic 显式注入 cache_control 断点）
    #[serde(default)]
    pub cache_prompt: bool,
    /// A3-2: 历史消息窗口上限（None 表示使用系统默认值）。
    #[serde(default)]
    pub max_history_messages: Option<i32>,
    /// Phase 0: 模型上下文窗口（None = 运行时按 provider+model 查已知默认表）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i32>,
    /// Task 4: 工具白名单（None = 全部启用，Some(空 vec) = 全部禁用）。
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    /// 是否支持图片输入
    #[serde(default)]
    pub supports_vision: bool,
    /// M2-1: Agent 描述
    #[serde(default)]
    pub description: String,
    /// M2-1: Agent 头像
    #[serde(default)]
    pub avatar: Option<String>,
    /// Phase 3: 工作区目录路径
    #[serde(default)]
    pub workspace_path: Option<String>,
    /// Phase 3: 是否从 agent.yaml 读取了部分配置
    #[serde(default)]
    pub config_from_file: bool,
    pub created_at: String,
    pub updated_at: String,
    /// 是否已配置 API Key（前端业务提示用）
    pub has_api_key: bool,
}

impl Agent {
    /// 从 AgentRow 转换，并尝试读取 workspace_path 下的 agent.yaml 合并配置。
    /// 文件不存在或解析失败时静默回退到 DB 原始值。
    pub fn from_row_with_file_config(row: AgentRow) -> Self {
        let mut agent = Agent::from(row);
        if let Some(file_cfg) = agent.load_file_config() {
            file_cfg.apply_to(&mut agent);
            agent.config_from_file = true;
        }
        agent
    }

    /// 尝试从 workspace_path 读取 agent.yaml。
    /// 注意：此方法在 Agent 上调用时 workspace_path 已通过 AgentRow 赋值。
    fn load_file_config(&self) -> Option<AgentFileConfig> {
        let dir = self.workspace_path.as_ref()?;
        let yaml_path = std::path::Path::new(dir).join("agent.yaml");
        let content = std::fs::read_to_string(yaml_path).ok()?;
        serde_yaml::from_str(&content).ok()
    }
}

/// 从 AgentRow 转换到 Agent（不含文件配置合并）
impl From<AgentRow> for Agent {
    fn from(row: AgentRow) -> Self {
        let extra = serde_json::from_str(&row.extra_params)
            .unwrap_or(serde_json::Value::Object(Default::default()));
        let has_api_key = !row.api_key_ref.is_empty();
        Agent {
            workspace_path: row.workspace_path.clone(),
            config_from_file: false,
            id: row.id,
            name: row.name,
            provider: row.provider,
            model: row.model,
            system_prompt: row.system_prompt,
            base_url: row.base_url,
            temperature: row.temperature,
            max_tokens: row.max_tokens,
            extra_params: extra,
            sort_order: row.sort_order,
            cache_prompt: row.cache_prompt != 0,
            max_history_messages: row.max_history_messages,
            context_window: row.context_window,
            enabled_tools: row
                .enabled_tools
                .as_deref()
                .map(|s| serde_json::from_str::<Vec<String>>(s).unwrap_or_default()),
            supports_vision: row.supports_vision != 0,
            created_at: row.created_at,
            updated_at: row.updated_at,
            has_api_key,
            description: row.description,
            avatar: row.avatar,
        }
    }
}

/// 创建 agent 入参（前端 → Rust）
#[derive(Debug, Deserialize)]
pub struct NewAgent {
    /// 用户自定义 ID（唯一且不可修改）
    pub id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub system_prompt: String,
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: i32,
    #[serde(default)]
    pub extra_params: Option<serde_json::Value>,
    #[serde(default)]
    pub sort_order: i32,
    /// P2-3: 是否启用 prompt caching（默认 true）
    #[serde(default = "default_cache_prompt")]
    pub cache_prompt: bool,
    /// A3-2: 历史消息窗口上限（None = 使用系统默认）。
    /// 旧调用方无需关心（`#[serde(default)]` 兜底为 None）。
    #[serde(default)]
    pub max_history_messages: Option<i32>,
    /// Phase 0: 模型上下文窗口（None = 运行时按 provider+model 查已知默认表）。
    #[serde(default)]
    pub context_window: Option<i32>,
    /// Task 4: 工具白名单（None = 全部启用，Some(vec) = 仅启用列出的工具）。
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    /// 是否支持图片输入（默认 false）
    #[serde(default)]
    pub supports_vision: bool,
    /// Phase 3: 工作区目录路径（存放 agent.yaml）
    #[serde(default)]
    pub workspace_path: Option<String>,
    /// M2-1: 头像图片（base64 dataURL，前端 canvas 压缩）
    #[serde(default)]
    pub avatar: Option<String>,
}

fn default_temperature() -> f64 {
    0.7
}
/// 新建 agent 的输出 token 上限默认值（页面不暴露此字段，进阶配置走 agent.yaml）。
///
/// 旧值为 4096，会截断稍长的回答（如两份数千字文档的对比，输出常达 6–12K token），
/// 被 provider 以 finish_reason=length / max_tokens 在半句处截断。16384 覆盖绝大多数
/// 长回答；运行时由 chat_cmd 的 effective_max_tokens（模型策展表 .max，只抬不降）再
/// 抬到模型能力值（已知模型 32K）。需突破上限的进阶用户在 agent.yaml 配置 max_tokens。
/// 历史 agent 的 4096 由 migration 41 一并抬升。
fn default_max_tokens() -> i32 {
    16384
}
fn default_cache_prompt() -> bool {
    true
}

/// 更新 agent 入参（partial update）
///
/// 与 `NewAgent` 保持一致，使用 snake_case 字段名。
/// 之前使用 `#[serde(rename_all = "camelCase")]`，会导致前端发来的
/// snake_case 字段（system_prompt、base_url、max_tokens、extra_params、
/// sort_order）反序列化时全部命中 None，造成静默更新失败。
#[derive(Debug, Deserialize)]
pub struct AgentUpdate {
    pub id: String,
    pub name: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    /// 双层 Option：外层 Some 表示调用方传了该字段，内层 None 表示清空
    pub base_url: Option<Option<String>>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i32>,
    pub extra_params: Option<serde_json::Value>,
    pub sort_order: Option<i32>,
    /// P2-3: 是否启用 prompt caching（None 表示不改）
    pub cache_prompt: Option<bool>,
    /// A3-2: 历史消息窗口上限。
    /// 双层 Option：外层 Some 表示调用方传了该字段，内层 None 表示清空（恢复为系统默认）。
    pub max_history_messages: Option<Option<i32>>,
    /// Phase 0: 模型上下文窗口。双层 Option：
    /// - None = 不更新
    /// - Some(None) = 清空（恢复为运行时查默认表）
    /// - Some(Some(n)) = 显式设为 n
    #[serde(default)]
    pub context_window: Option<Option<i32>>,
    /// Task 4: 工具白名单。双层 Option：外层 Some = 调用方传了，内层 None = 清空（全部启用）。
    #[serde(default)]
    pub enabled_tools: Option<Option<Vec<String>>>,
    /// 是否支持图片输入（None = 不改）
    pub supports_vision: Option<bool>,
    /// Phase 3: 工作区路径。双层 Option：
    /// - None = 不更新
    /// - Some(None) = 清空
    /// - Some(Some(path)) = 设为 path
    #[serde(default)]
    pub workspace_path: Option<Option<String>>,
    /// M2-1: 头像图片（base64 dataURL）。双层 Option：
    /// - None = 不更新（JSON 字段缺席）
    /// - Some(None) = 清空（JSON null，须 deserialize_double_option 区分缺席）
    /// - Some(Some(v)) = 设定
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub avatar: Option<Option<String>>,
}

/// 轮换 API Key 入参
#[derive(Debug, Deserialize)]
pub struct RotateAgentKey {
    pub agent_id: String,
    pub api_key: String,
    pub base_url: Option<String>,
}

// =========================================================================
// Conversation
// =========================================================================

/// Task 3b: `tools_override` 列存 JSON 字符串（`{"read_file": true, ...}`）。
/// `NULL` 表示继承 Agent 配置，不覆盖。
#[derive(Debug, Clone, FromRow)]
pub struct ConversationRow {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub pinned: i32,
    pub created_at: String,
    pub updated_at: String,
    /// Task 3b: 对话级工具覆盖（JSON 字符串，NULL = 继承 Agent 配置）
    pub tools_override: Option<String>,
    /// Phase 2: 所属项目 ID（NULL = 默认项目）
    pub project_id: Option<String>,
    /// MA-1: 会话类型 'chat' | 'delegation' | 'channel'（migration 45，存量行默认 'chat'）
    pub kind: String,
    /// MA-1: 发起者 'user' | 'agent'（NULL ≡ 'user'，旧数据语义）
    pub initiator_type: Option<String>,
    /// MA-1: delegation 子会话的发起 agent（无 FK——agent 可删，会话须活得比 agent 久）
    pub initiator_agent_id: Option<String>,
    /// MA-1: 委派图边——发起委派的父会话（ON DELETE SET NULL，父删边不删子）
    pub parent_conversation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Conversation {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    /// Task 3b: 对话级工具覆盖（None = 继承 Agent 配置）
    #[serde(default)]
    pub tools_override: Option<HashMap<String, bool>>,
    /// Phase 2: 所属项目 ID（None = 默认项目）
    #[serde(default)]
    pub project_id: Option<String>,
    /// MA-1: 会话类型 'chat' | 'delegation' | 'channel'（serde default 兼容旧缓存负载）
    #[serde(default = "default_conversation_kind")]
    pub kind: String,
    /// MA-1: 发起者（'user' | 'agent'；None ≡ 'user'）
    #[serde(default)]
    pub initiator_agent_id: Option<String>,
    /// MA-1: 委派父会话（None = 非委派会话）
    #[serde(default)]
    pub parent_conversation_id: Option<String>,
}

/// `kind` 的 serde 默认值（旧负载无此字段时视为普通聊天会话）
fn default_conversation_kind() -> String {
    "chat".to_string()
}

impl From<ConversationRow> for Conversation {
    fn from(row: ConversationRow) -> Self {
        Conversation {
            id: row.id,
            agent_id: row.agent_id,
            title: row.title,
            pinned: row.pinned != 0,
            created_at: row.created_at,
            updated_at: row.updated_at,
            tools_override: row
                .tools_override
                .as_deref()
                .map(|s| serde_json::from_str::<HashMap<String, bool>>(s).unwrap_or_default()),
            project_id: row.project_id,
            kind: if row.kind.is_empty() {
                default_conversation_kind()
            } else {
                row.kind
            },
            initiator_agent_id: row.initiator_agent_id,
            parent_conversation_id: row.parent_conversation_id,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct NewConversation {
    pub agent_id: String,
    #[serde(default)]
    pub title: Option<String>,
    /// Phase 2: 所属项目 ID（None = 默认项目）
    #[serde(default)]
    pub project_id: Option<String>,
    /// MA-1: 会话类型（None = 'chat'；'delegation' = agent 委派子会话）
    #[serde(default)]
    pub kind: Option<String>,
    /// MA-1: 发起委派的 agent（None = 用户发起；Some 即 initiator_type='agent'）
    #[serde(default)]
    pub initiator_agent_id: Option<String>,
    /// MA-1: 委派父会话 ID（None = 非委派会话）
    #[serde(default)]
    pub parent_conversation_id: Option<String>,
}

// =========================================================================
// Message
// =========================================================================

#[derive(Debug, Clone, FromRow)]
pub struct MessageRow {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    /// P2-1: JSON 数组字符串（ContentBlock[]），默认 '[]' 兼容旧消息
    pub content_blocks: String,
    pub token_count: Option<i32>,
    pub error: Option<String>,
    pub created_at: String,
    /// SQLite 物理行号（分页游标用）。
    ///
    /// `messages` 表使用 `id TEXT PRIMARY KEY` 但未声明 `WITHOUT ROWID`，
    /// SQLite 默认会给该表分配一个单调递增的隐式 `rowid` 列。SQLx
    /// 可在 SELECT 列表里显式取出。`rowid` 是 `INTEGER PRIMARY KEY` 的
    /// 别名，本身就是 `i64`，分页时用作「同秒消息 tie-breaker」非常稳定。
    pub rowid: i64,
    /// M1.5: 该消息被某条摘要覆盖时，指向摘要消息（role="system",
    /// content 以 `[Previous conversation summary]` 开头）。
    /// 摘要消息本身此列为 NULL（自指无意义）。
    /// 旧消息（迁移前已存在）此列也为 NULL —— 历史行为完全兼容。
    pub summary_id: Option<String>,
    /// 消息实际使用的模型名（仅 assistant 消息有值；历史消息可能为 NULL）。
    pub model: Option<String>,
    /// 消息首现事件 seq（Phase 2B 阶段 2，**非表列**）：仅 derive 读路径
    /// （`read_route::to_message_row`）填充；DB SELECT 读出的行恒 `None`。
    /// `#[sqlx(default)]` 兼容三个现有 SELECT（缺列取 Default）。
    #[sqlx(default)]
    pub source_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    /// P2-1: JSON 数组字符串（ContentBlock[]），空数组表示旧消息
    pub content_blocks: String,
    pub token_count: Option<i32>,
    pub error: Option<String>,
    pub created_at: String,
    /// 分页游标（与 `MessageRow.rowid` 对齐）。
    /// 前端不展示此字段，仅在向上翻页时回传 `(created_at, rowid)` 复合游标。
    pub rowid: i64,
    /// M1.5: 摘要外键（参见 `MessageRow.summary_id`）。
    /// 前端一般用不到，保留以备审计 / UI 展示「已压缩 N 条」之用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_id: Option<String>,
    /// 实际使用的模型名（仅 assistant 消息有值）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl From<MessageRow> for Message {
    fn from(row: MessageRow) -> Self {
        Message {
            id: row.id,
            conversation_id: row.conversation_id,
            role: row.role,
            content: row.content,
            content_blocks: row.content_blocks,
            token_count: row.token_count,
            error: row.error,
            created_at: row.created_at,
            rowid: row.rowid,
            summary_id: row.summary_id,
            model: row.model,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct NewMessage {
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub token_count: Option<i32>,
    #[serde(default)]
    pub error: Option<String>,
    /// 消息实际使用的模型名（仅 assistant 消息需要传）
    #[serde(default)]
    pub model: Option<String>,
}

// 用 `DateTime<Utc>` 仅是给上层时间工具备查；当前 SQL 用 TEXT 存 ISO8601，因此保留 String 字段

// =========================================================================
// Tool Call（P2-1 工具调用审计日志）
// =========================================================================

/// 数据库行版本：工具调用记录
#[derive(Debug, Clone, FromRow)]
pub struct ToolCallRow {
    pub id: String,
    pub message_id: String,
    pub tool_name: String,
    pub arguments: String,
    pub result: Option<String>,
    pub is_error: i32,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub created_at: String,
}

/// 前端可见的工具调用记录
#[derive(Debug, Clone, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub message_id: String,
    pub tool_name: String,
    /// JSON 字符串（参数）
    pub arguments: String,
    /// JSON 字符串（结果），未完成为 None
    pub result: Option<String>,
    pub is_error: bool,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub created_at: String,
}

impl From<ToolCallRow> for ToolCall {
    fn from(row: ToolCallRow) -> Self {
        ToolCall {
            id: row.id,
            message_id: row.message_id,
            tool_name: row.tool_name,
            arguments: row.arguments,
            result: row.result,
            is_error: row.is_error != 0,
            started_at: row.started_at,
            finished_at: row.finished_at,
            created_at: row.created_at,
        }
    }
}

/// 创建工具调用记录入参
#[derive(Debug, Deserialize)]
pub struct NewToolCall {
    pub id: String,
    pub message_id: String,
    pub tool_name: String,
    pub arguments: String,
}

// =========================================================================
// Session Event（session-event-log Phase 0：append-only 事件日志）
// =========================================================================

/// 数据库行版本：会话事件
///
/// 表见 `migrations/44_session_events.sql`。append-only 不变式：永不
/// UPDATE/DELETE（唯一删除路径是会话 CASCADE）。`payload` 是 JSON 字符串，
/// 按 `kind` 反序列化为 `harness::event_log` 里的强类型 struct。
#[derive(Debug, Clone, FromRow)]
pub struct SessionEventRow {
    pub id: i64,
    pub session_id: String,
    pub seq: i64,
    pub kind: String,
    pub actor: String,
    pub turn_id: Option<String>,
    pub message_id: Option<String>,
    pub payload: String,
    pub created_at: String,
}

/// 传输对象：会话事件（`payload` 已 parse）。
///
/// [`SessionEventRow`] 的序列化友好版本——`payload` 由 JSON 字符串解析为
/// [`serde_json::Value`]（与 `export_session_trajectory` 的 parse 兜底一致：非法
/// JSON 降级为字符串值）。供 `list_session_events` 命令返回前端「轨迹回放」视图
/// 直接消费，免逐行 `JSON.parse`。
#[derive(Debug, Clone, Serialize)]
pub struct SessionEvent {
    pub id: i64,
    pub session_id: String,
    pub seq: i64,
    pub kind: String,
    pub actor: String,
    pub turn_id: Option<String>,
    pub message_id: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: String,
}

impl From<SessionEventRow> for SessionEvent {
    fn from(r: SessionEventRow) -> Self {
        let payload: serde_json::Value = serde_json::from_str(&r.payload)
            .unwrap_or(serde_json::Value::String(r.payload.clone()));
        Self {
            id: r.id,
            session_id: r.session_id,
            seq: r.seq,
            kind: r.kind,
            actor: r.actor,
            turn_id: r.turn_id,
            message_id: r.message_id,
            payload,
            created_at: r.created_at,
        }
    }
}

// =========================================================================
// TemplateRow（仅 pipeline context 使用）
// =========================================================================

/// 数据库行版本：包含全部字段
#[derive(Debug, Clone, FromRow)]
pub struct TemplateRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub user_prompt_prefix: String,
    pub variables: String,
    pub tools: String,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

// =========================================================================
// Knowledge Base（知识库 RAG v1，agentic 检索）
// =========================================================================

/// 数据库行版本：知识库
#[derive(Debug, Clone, FromRow)]
pub struct KbRow {
    pub id: String,
    pub name: String,
    /// 归属层级：'agent' | 'project' | 'global'
    pub scope: String,
    /// agent_id / project_id / NULL(global)
    pub owner_id: Option<String>,
    /// 监听的知识库目录绝对路径
    pub directory: String,
    pub enabled: i32,
    pub created_at: String,
    pub updated_at: String,
}

/// 前端可见的知识库
#[derive(Debug, Clone, Serialize)]
pub struct Kb {
    pub id: String,
    pub name: String,
    pub scope: String,
    pub owner_id: Option<String>,
    pub directory: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<KbRow> for Kb {
    fn from(row: KbRow) -> Self {
        Kb {
            enabled: row.enabled != 0,
            id: row.id,
            name: row.name,
            scope: row.scope,
            owner_id: row.owner_id,
            directory: row.directory,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct NewKb {
    pub id: String,
    pub name: String,
    /// 'agent' | 'project' | 'global'
    pub scope: String,
    #[serde(default)]
    pub owner_id: Option<String>,
    pub directory: String,
    #[serde(default = "default_kb_enabled")]
    pub enabled: bool,
}

fn default_kb_enabled() -> bool {
    true
}

/// 创建知识库的入参（前端 invoke，不含 id —— 由命令层生成）。
///
/// `scope` 限定 `'agent' | 'project' | 'global'`；`owner_id` 对应 agent_id /
/// project_id，global 时为 None。
#[derive(Debug, Clone, Deserialize)]
pub struct CreateKbInput {
    pub name: String,
    /// 'agent' | 'project' | 'global'
    pub scope: String,
    #[serde(default)]
    pub owner_id: Option<String>,
    /// 监听的知识库目录绝对路径
    pub directory: String,
    #[serde(default = "default_kb_enabled")]
    pub enabled: bool,
}

/// 更新知识库的入参（仅 name / enabled 可改；directory 改动需删后重建，
/// 避免 watcher 监听目录与 DB 不一致）。
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateKb {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// 数据库行版本：知识库文档（索引）。前端也可读（文档列表）
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct KbDocumentRow {
    pub id: String,
    pub kb_id: String,
    /// 相对 kb.directory 的路径
    pub file_path: String,
    pub title: String,
    pub summary: String,
    /// JSON 数组（frontmatter tags）
    pub tags: String,
    pub content_hash: Option<String>,
    pub file_mtime: Option<String>,
    pub indexed_at: String,
}

/// 检索命中项（search_kb 返回给 agent）
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct KbSearchHit {
    pub kb_id: String,
    pub kb_name: String,
    pub file_path: String,
    pub title: String,
    pub summary: String,
}

// =========================================================================
// UserPreferences（全局配置）
// =========================================================================

/// 用户偏好设置（前端 ↔ 后端传输结构）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserPreferences {
    pub timezone: Option<String>,
    pub default_agent_id: Option<String>,
    pub default_template_id: Option<String>,
    pub on_startup: Option<String>,
    pub language: Option<String>,
    pub theme: Option<String>,
    pub code_theme: Option<String>,
    pub font_size: Option<i32>,
    /// Phase 3: 默认工作区根路径
    pub default_workspace_path: Option<String>,
    /// Embedding 全局配置（独立于聊天 Agent）
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_api_key: Option<String>,
    pub embedding_base_url: Option<String>,
    /// Vision 全局配置（Phase B）：当前 Agent 不支持视觉（supports_vision=0）时，
    /// 附件图片识别 fallback 到此配置。仿 embedding。Agent 自带 supports_vision=1 时优先用 Agent 自己的模型。
    pub vision_provider: Option<String>,
    pub vision_model: Option<String>,
    pub vision_api_key: Option<String>,
    pub vision_base_url: Option<String>,
}

// =========================================================================
// Project（项目管理）— DB schema 已由 migration 13/14/21 建好
// =========================================================================

/// projects 表行结构
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_project_icon")]
    pub icon: String,
    pub sort_order: i32,
    #[serde(default)]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub theme_color: Option<String>,
    /// 项目头像图片（base64 dataURL；NULL 走前端名字渐变兜底）
    #[serde(default)]
    pub avatar: Option<String>,
    /// 是否已归档（软删除）：0 = 活跃，1 = 已归档
    #[serde(default)]
    pub archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_project_icon() -> String {
    "folder".into()
}

/// project_agents 关联表行结构
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ProjectAgentRow {
    pub project_id: String,
    pub agent_id: String,
    #[serde(default = "default_agent_role")]
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

fn default_agent_role() -> String {
    "member".into()
}

/// 对外传输：Project 基础字段 + 成员列表
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    #[serde(flatten)]
    pub row: ProjectRow,
    #[serde(default)]
    pub agents: Vec<ProjectAgentRow>,
}

/// 创建项目入参
#[derive(Debug, Clone, Deserialize)]
pub struct NewProject {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub theme_color: Option<String>,
    /// 项目头像图片（base64 dataURL，前端压缩后）
    #[serde(default)]
    pub avatar: Option<String>,
    /// 初始成员 agent_id 列表（role 默认 member）
    #[serde(default)]
    pub agent_ids: Vec<String>,
}

/// 自定义反序列化：把 JSON `null` 映射为 `Some(None)`（=清空），字段缺失由
/// `#[serde(default)]` 兜底为 `None`（=不改）。这样双层 Option 就能正确表达三种状态。
fn deserialize_double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    let inner = Option::<T>::deserialize(deserializer)?;
    Ok(Some(inner))
}

/// 更新项目入参（partial update）：普通字段 `None` = 不改；双层 Option 字段
/// `None`=不改 / `Some(None)`=清空为 null / `Some(Some(v))`=设定。
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateProject {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    /// None=不改 / Some(None)=清空 / Some(Some(v))=设定
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub workspace_path: Option<Option<String>>,
    /// None=不改 / Some(None)=清空 / Some(Some(v))=设定
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub theme_color: Option<Option<String>>,
    /// 项目头像图片。None=不改 / Some(None)=清空 / Some(Some(v))=设定
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub avatar: Option<Option<String>>,
}
