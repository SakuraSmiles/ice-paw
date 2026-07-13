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
}

fn default_temperature() -> f64 { 0.7 }
fn default_max_tokens() -> i32 { 4096 }

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
    pub token_count: Option<i32>,
    pub error: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub token_count: Option<i32>,
    pub error: Option<String>,
    pub created_at: String,
}

impl From<MessageRow> for Message {
    fn from(row: MessageRow) -> Self {
        Message {
            id: row.id,
            conversation_id: row.conversation_id,
            role: row.role,
            content: row.content,
            token_count: row.token_count,
            error: row.error,
            created_at: row.created_at,
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
