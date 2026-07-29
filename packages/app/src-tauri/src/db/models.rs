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
    /// A3-2: 历史消息窗口上限（NULL = 使用系统默认）。
    pub max_history_messages: Option<i32>,
    /// M1.2 A2-4: 工具裁剪阈值（NULL = 使用系统默认 5）。
    pub tool_trim_threshold: Option<i32>,
    /// Task 4: 工具白名单（NULL = 全部启用）。
    /// JSON 数组格式：`["read_file", "list_directory"]`
    pub enabled_tools: Option<String>,
    /// 是否支持图片输入（0 = 不支持, 1 = 支持）
    pub supports_vision: i32,
    /// M2-1: Embedding 模型名称（用于语义检索 recall）
    pub embedding_model: Option<String>,
    /// M2-1: Agent 描述
    pub description: String,
    /// M2-1: Agent 头像（URL 或 base64）
    pub avatar: Option<String>,
    /// Phase 3: 工作区目录路径（存放 agent.yaml 的本地目录）
    pub workspace_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

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
    #[serde(default)]
    pub tool_trim_threshold: Option<i32>,
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    #[serde(default)]
    pub extra_params: Option<serde_json::Value>,
    #[serde(default)]
    pub embedding_model: Option<String>,
    /// 工具调用最大轮数（None = 使用系统默认 10）
    #[serde(default)]
    pub tool_max_rounds: Option<u32>,
}

impl AgentRow {
    /// 从 extra_params JSON 中读取工具调用最大轮数（None = 使用系统默认）
    pub fn tool_max_rounds(&self) -> Option<u32> {
        let params: serde_json::Value = serde_json::from_str(&self.extra_params).ok()?;
        params.get("tool_max_rounds").and_then(|v| v.as_u64()).map(|v| v as u32)
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
        if let Some(v) = &self.description { agent.description = v.clone(); }
        if let Some(v) = &self.system_prompt { agent.system_prompt = v.clone(); }
        if let Some(v) = self.temperature { agent.temperature = v; }
        if let Some(v) = self.max_tokens { agent.max_tokens = v; }
        if let Some(v) = self.cache_prompt { agent.cache_prompt = v; }
        if let Some(v) = self.supports_vision { agent.supports_vision = v; }
        if let Some(v) = self.max_history_messages { agent.max_history_messages = Some(v); }
        if let Some(v) = self.tool_trim_threshold { agent.tool_trim_threshold = Some(v); }
        if let Some(v) = &self.enabled_tools { agent.enabled_tools = Some(v.clone()); }
        if let Some(v) = &self.extra_params { agent.extra_params = v.clone(); }
        if let Some(v) = &self.embedding_model { agent.embedding_model = Some(v.clone()); }
        if let Some(v) = self.tool_max_rounds {
            if let Some(obj) = agent.extra_params.as_object_mut() {
                obj.insert("tool_max_rounds".into(), serde_json::json!(v));
            }
        }
    }

    /// 把文件配置合并到 AgentRow 中（供 chat_cmd 内部使用）
    pub fn apply_to_row(&self, row: &mut AgentRow) {
        if let Some(v) = &self.description { row.description = v.clone(); }
        if let Some(v) = &self.system_prompt { row.system_prompt = v.clone(); }
        if let Some(v) = self.temperature { row.temperature = v; }
        if let Some(v) = self.max_tokens { row.max_tokens = v; }
        if let Some(v) = self.cache_prompt { row.cache_prompt = if v { 1 } else { 0 }; }
        if let Some(v) = self.supports_vision { row.supports_vision = if v { 1 } else { 0 }; }
        if let Some(v) = self.max_history_messages { row.max_history_messages = Some(v); }
        if let Some(v) = self.tool_trim_threshold { row.tool_trim_threshold = Some(v); }
        if let Some(v) = &self.enabled_tools {
            row.enabled_tools = Some(serde_json::to_string(v).unwrap_or_default());
        }
        if let Some(v) = &self.extra_params {
            row.extra_params = serde_json::to_string(v).unwrap_or_default();
        }
        if let Some(v) = &self.embedding_model { row.embedding_model = Some(v.clone()); }
        if let Some(v) = self.tool_max_rounds {
            // 合并到 extra_params JSON 中
            let mut params: serde_json::Value = serde_json::from_str(&row.extra_params)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            if let Some(obj) = params.as_object_mut() {
                obj.insert("tool_max_rounds".into(), serde_json::json!(v));
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
    /// M1.2 A2-4: 工具裁剪阈值（None 表示使用系统默认值 5）。
    #[serde(default)]
    pub tool_trim_threshold: Option<i32>,
    /// Task 4: 工具白名单（None = 全部启用，Some(空 vec) = 全部禁用）。
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    /// 是否支持图片输入
    #[serde(default)]
    pub supports_vision: bool,
    /// M2-1: Embedding 模型名称
    #[serde(default)]
    pub embedding_model: Option<String>,
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
            tool_trim_threshold: row.tool_trim_threshold,
            enabled_tools: row.enabled_tools
                .as_deref()
                .map(|s| serde_json::from_str::<Vec<String>>(s).unwrap_or_default()),
            supports_vision: row.supports_vision != 0,
            created_at: row.created_at,
            updated_at: row.updated_at,
            has_api_key,
            embedding_model: row.embedding_model,
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
    /// M1.2 A2-4: 工具裁剪阈值（None = 使用系统默认 5）。
    #[serde(default)]
    pub tool_trim_threshold: Option<i32>,
    /// Task 4: 工具白名单（None = 全部启用，Some(vec) = 仅启用列出的工具）。
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    /// 是否支持图片输入（默认 false）
    #[serde(default)]
    pub supports_vision: bool,
    /// Phase 3: 工作区目录路径（存放 agent.yaml）
    #[serde(default)]
    pub workspace_path: Option<String>,
}

fn default_temperature() -> f64 { 0.7 }
fn default_max_tokens() -> i32 { 4096 }
fn default_cache_prompt() -> bool { true }

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
    /// M1.2 A2-4: 工具裁剪阈值。
    /// 双层 Option：外层 Some 表示调用方传了该字段，内层 None 表示清空（恢复为系统默认）。
    pub tool_trim_threshold: Option<Option<i32>>,
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
            tools_override: row.tools_override
                .as_deref()
                .map(|s| serde_json::from_str::<HashMap<String, bool>>(s).unwrap_or_default()),
            project_id: row.project_id,
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

#[allow(dead_code)]
pub type UtcDateTime = DateTime<Utc>;

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
}


