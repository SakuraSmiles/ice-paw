//! 数据库行结构（`FromRow`）与对外传输结构（`Serialize/Deserialize`）
//!
//! 关键点：
//! - `Agent` 的 `api_key_ref` 和 `base_url` 用 `#[serde(skip_serializing)]` 屏蔽，
//!   前端 `list_agents` 永远不会拿到敏感字段
//! - 列表返回类型 `Agent` 与数据库 `AgentRow` 解耦，可在未来加过滤字段不影响持久化
//! - 时间字段统一用 `DateTime<Utc>`，序列化时走 RFC3339

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
    /// 不同 Agent 可拥有不同上下文长度（8K vs 200K）。
    pub max_history_messages: Option<i32>,
    /// M1.2 A2-4: 工具裁剪阈值（NULL = 使用系统默认 5）。
    /// 当注册工具数 >= 此值时启用软裁剪（deprioritized 标记）。
    pub tool_trim_threshold: Option<i32>,
    /// Task 4: 工具白名单（NULL = 全部启用）。
    /// JSON 数组格式：`["read_file", "list_directory"]`
    pub enabled_tools: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 前端可见的 `Agent`（不含敏感引用）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
    /// 前端在「高级设置」中可显式覆盖；不传则保持 NULL。
    #[serde(default)]
    pub max_history_messages: Option<i32>,
    /// M1.2 A2-4: 工具裁剪阈值（None 表示使用系统默认值 5）。
    #[serde(default)]
    pub tool_trim_threshold: Option<i32>,
    /// Task 4: 工具白名单（None = 全部启用，Some(空 vec) = 全部禁用）。
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    pub created_at: String,
    pub updated_at: String,
    /// 是否已配置 API Key（前端业务提示用），对应 stronghold 中是否存在
    pub has_api_key: bool,
}

impl From<AgentRow> for Agent {
    fn from(row: AgentRow) -> Self {
        let extra = serde_json::from_str(&row.extra_params)
            .unwrap_or(serde_json::Value::Object(Default::default()));
        // api_key_ref 非空即视为已配置；强一致需依赖 stronghold，但最少提示给前端
        let has_api_key = !row.api_key_ref.is_empty();
        Agent {
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
            // P2-3: i32 0/1 → bool（DB 默认 1，零值兜底为 false）
            cache_prompt: row.cache_prompt != 0,
            // A3-2: 历史消息窗口（NULL 由 Option 直接透传）
            max_history_messages: row.max_history_messages,
            // M1.2 A2-4: 工具裁剪阈值
            tool_trim_threshold: row.tool_trim_threshold,
            // Task 4: 工具白名单（JSON 数组字符串 → Vec<String>）
            enabled_tools: row.enabled_tools
                .as_deref()
                .map(|s| serde_json::from_str::<Vec<String>>(s).unwrap_or_default()),
            created_at: row.created_at,
            updated_at: row.updated_at,
            has_api_key,
        }
    }
}

/// 创建 agent 入参（前端 → Rust）
#[derive(Debug, Deserialize)]
pub struct NewAgent {
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

#[derive(Debug, Clone, FromRow)]
pub struct ConversationRow {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub pinned: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Conversation {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
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
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct NewConversation {
    pub agent_id: String,
    #[serde(default)]
    pub title: Option<String>,
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
// Template
// =========================================================================
//
// 用户自定义「Prompt 模板」：带变量占位符的 system prompt + user prompt 前缀。
// 详情见 icepaw-p0-p2-plan.md §2.4 P2-4。

/// 模板变量定义（来自 `variables` JSON 数组中的单条）
///
/// - `name`        变量名（占位符，与 `{{name}}` 替换目标一致）
/// - `label`       前端展示标签（中文/友好名）
/// - `type`        控件类型：`text`（单行文本）/ `textarea`（多行）/ `select`（下拉）
/// - `default`     默认值（字符串）
/// - `options`     仅 `select` 类型使用：候选值列表
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateVariable {
    pub name: String,
    pub label: String,
    /// `text` | `textarea` | `select`
    #[serde(rename = "type")]
    pub var_type: String,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

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

/// 前端可见的 `Template`（自动展开 variables / tools JSON）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub user_prompt_prefix: String,
    pub variables: Vec<TemplateVariable>,
    pub tools: Vec<String>,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

impl From<TemplateRow> for Template {
    fn from(row: TemplateRow) -> Self {
        let variables: Vec<TemplateVariable> =
            serde_json::from_str(&row.variables).unwrap_or_default();
        let tools: Vec<String> = serde_json::from_str(&row.tools).unwrap_or_default();
        Template {
            id: row.id,
            name: row.name,
            description: row.description,
            system_prompt: row.system_prompt,
            user_prompt_prefix: row.user_prompt_prefix,
            variables,
            tools,
            sort_order: row.sort_order,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// 创建模板入参（前端 → Rust）
#[derive(Debug, Deserialize)]
pub struct NewTemplate {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub user_prompt_prefix: String,
    #[serde(default)]
    pub variables: Option<Vec<TemplateVariable>>,
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub sort_order: i32,
}

/// 更新模板入参（partial update）
#[derive(Debug, Deserialize)]
pub struct TemplateUpdate {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    /// 字符串字段统一使用单层 Option：要清空就传空串
    pub system_prompt: Option<String>,
    pub user_prompt_prefix: Option<String>,
    /// 数组字段：传 Some(vec![]) 表示清空
    pub variables: Option<Vec<TemplateVariable>>,
    pub tools: Option<Vec<String>>,
    pub sort_order: Option<i32>,
}
