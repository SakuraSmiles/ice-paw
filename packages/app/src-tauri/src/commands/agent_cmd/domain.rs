//! Agent 域命令 trait 抽象 + 带凭据 DTO（REQ-XC-010）。
//!
//! 从原 `agent_cmd.rs` God module 拆出（U3-5 ③）：trait 是 `chat_cmd` /
//! `provider_cmd` / `agent_yaml` / `channel` / `inbox` / `delegate` 共同依赖的
//! 抽象边界，把它与 SQL 生产实现、yaml 出生镜像、校验纯函数解耦——业务层
//! `use` 抽象时不必把整座 SQL 山一起拉进来。

use async_trait::async_trait;

use crate::db::models::{Agent, AgentRow, AgentUpdate, HookConfig, NewAgent, RotateAgentKey};
use crate::error::AppResult;

/// chat_cmd 在 `send_message` 时需要的全部 agent 信息。
///
/// 之所以做成独立结构体而非简单返回 `AgentRow + (api_key, base_url)`：
/// - 减少 trait 方法签名数量（chat_cmd 只要调一次）
/// - Mock 实现可以一次性预置所有数据，无需分别 mock agent + stronghold
#[derive(Debug, Clone)]
pub struct AgentWithCredentials {
    /// Agent 元数据行
    pub agent: AgentRow,
    /// 解密后的 api_key（明文，传给 provider）
    pub api_key: String,
    /// base_url（vault 优先；agent 配置 fallback）
    pub base_url: Option<String>,
    /// 对话钩子配置（来自 agent.yaml `hooks` 字段；hooks 不进 DB，纯文件）。
    /// chat_cmd 据此在各生命周期点执行 inject_prompt/call_tool/log。
    pub hooks: HookConfig,
    /// Word 文档样式偏好（agent.yaml `word_style_profile` 自由文字块；不进 DB，
    /// 纯文件）——D12 双轨承载。非空时注入 system prompt「Word 文档样式偏好」
    /// 小节；与 hooks 同款旁路：不参与 apply_to_row 的字段覆盖。
    pub word_style_profile: Option<String>,
}

/// Agent 域命令 trait
///
/// 所有方法 async，由调用方在 `tokio::spawn` 或 `tauri::command` async fn
/// 中直接 `.await`。trait object：`Arc<dyn AgentCmd>`。
#[async_trait]
pub trait AgentCmd: Send + Sync {
    /// 列出全部 agent（不含敏感字段）
    async fn list(&self) -> AppResult<Vec<Agent>>;

    /// 取单个 agent 元数据（不含 api_key）
    async fn get(&self, agent_id: &str) -> AppResult<AgentRow>;

    /// 取 agent + 解密后的 api_key + base_url（chat_cmd 拼装 LLM 调用专用）
    ///
    /// 默认实现：先 `get` 再 `crypto::fetch_api_key`。Mock 可直接 override
    /// 返回预置数据，避免依赖真实 stronghold。
    async fn get_with_credentials(&self, agent_id: &str) -> AppResult<AgentWithCredentials>;

    /// 创建 agent（含 api_key 写入 stronghold）
    async fn create(&self, input: NewAgent) -> AppResult<Agent>;

    /// 部分更新 agent
    async fn update(&self, input: AgentUpdate) -> AppResult<Agent>;

    /// 单独轮换 api_key
    async fn rotate_key(&self, input: RotateAgentKey) -> AppResult<Agent>;

    /// 删除 agent（级联清理 conversations + messages）
    async fn delete(&self, agent_id: &str) -> AppResult<()>;
}
