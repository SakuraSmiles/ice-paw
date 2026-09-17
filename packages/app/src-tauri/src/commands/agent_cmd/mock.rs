//! MockAgentCmd —— 测试实现（内存状态）。
//!
//! 从原 `agent_cmd.rs` God module 拆出（U3-5 ③）：与 [`super::domain::AgentCmd`]
//! 的 trait 契约同居，`#[cfg(test)]` 编译，供 `chat_cmd_tests` 等以
//! `Arc<dyn AgentCmd>` trait object 注入，验证业务编排逻辑而不连真实 DB / stronghold。

use async_trait::async_trait;
use uuid::Uuid;

use crate::db::models::{Agent, AgentRow, AgentUpdate, HookConfig, NewAgent, RotateAgentKey};
use crate::error::{AppError, AppResult};

use super::domain::{AgentCmd, AgentWithCredentials};

/// 测试用 AgentCmd 实现：内存 HashMap 存储 agent 元数据 + 凭据。
///
/// 用法：
/// ```ignore
/// let mock = Arc::new(MockAgentCmd::new());
/// mock.seed(agent_row, api_key, base_url);
/// // ... 用 mock 替换真实 SqlAgentCmd 跑业务逻辑测试
/// ```
pub struct MockAgentCmd {
    inner: std::sync::Mutex<MockAgentCmdInner>,
}

struct MockAgentCmdInner {
    /// agent_id → (AgentRow, api_key, base_url)
    agents: std::collections::HashMap<String, (AgentRow, String, Option<String>)>,
    /// 调用历史（用于断言「list 被调用了」「create 被调用了」之类）
    call_log: Vec<String>,
}

impl MockAgentCmd {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(MockAgentCmdInner {
                agents: std::collections::HashMap::new(),
                call_log: Vec::new(),
            }),
        }
    }

    /// 注入一条 agent 记录
    pub fn seed(&self, row: AgentRow, api_key: String, base_url: Option<String>) {
        let mut g = self.inner.lock().unwrap();
        g.agents.insert(row.id.clone(), (row, api_key, base_url));
    }

    /// 取调用历史（按时间顺序）
    pub fn call_log(&self) -> Vec<String> {
        let g = self.inner.lock().unwrap();
        g.call_log.clone()
    }

    fn log(&self, msg: String) {
        let mut g = self.inner.lock().unwrap();
        g.call_log.push(msg);
    }
}

impl Default for MockAgentCmd {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentCmd for MockAgentCmd {
    async fn list(&self) -> AppResult<Vec<Agent>> {
        self.log("list".into());
        let g = self.inner.lock().unwrap();
        Ok(g.agents
            .values()
            .map(|(row, _, _)| Agent::from(row.clone()))
            .collect())
    }

    async fn get(&self, agent_id: &str) -> AppResult<AgentRow> {
        self.log(format!("get({})", agent_id));
        let g = self.inner.lock().unwrap();
        g.agents
            .get(agent_id)
            .map(|(row, _, _)| row.clone())
            .ok_or_else(|| AppError::NotFound {
                resource: "agent",
                id: agent_id.to_string(),
            })
    }

    async fn get_with_credentials(&self, agent_id: &str) -> AppResult<AgentWithCredentials> {
        self.log(format!("get_with_credentials({})", agent_id));
        let g = self.inner.lock().unwrap();
        let (agent, api_key, base_url) = g
            .agents
            .get(agent_id)
            .ok_or_else(|| AppError::NotFound {
                resource: "agent",
                id: agent_id.to_string(),
            })?
            .clone();
        Ok(AgentWithCredentials {
            agent,
            api_key,
            base_url,
            hooks: HookConfig::default(),
            word_style_profile: None,
        })
    }

    async fn create(&self, input: NewAgent) -> AppResult<Agent> {
        self.log(format!("create({})", input.name));
        let id = if input.id.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            input.id.clone()
        };
        let row = AgentRow {
            id: id.clone(),
            name: input.name.clone(),
            provider: input.provider.clone(),
            model: input.model.clone(),
            system_prompt: input.system_prompt.clone(),
            api_key_ref: id.clone(),
            base_url: input.base_url.clone(),
            temperature: input.temperature,
            max_tokens: input.max_tokens,
            extra_params: input
                .extra_params
                .clone()
                .unwrap_or_else(|| serde_json::json!({}))
                .to_string(),
            sort_order: input.sort_order,
            cache_prompt: if input.cache_prompt { 1 } else { 0 },
            max_history_messages: input.max_history_messages,
            context_window: input.context_window,
            enabled_tools: input
                .enabled_tools
                .as_ref()
                .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string())),
            tool_scopes: None,
            supports_vision: if input.supports_vision { 1 } else { 0 },
            description: String::new(),
            avatar: None,
            workspace_path: input.workspace_path.clone(),
            model_profile_id: input.model_profile_id.clone(),
            fallback_profile_ids: input
                .fallback_profile_ids
                .as_ref()
                .map(|v| serde_json::to_string(v).unwrap_or_default()),
            created_at: "2024-01-01 00:00:00".to_string(),
            updated_at: "2024-01-01 00:00:00".to_string(),
        };
        let mut g = self.inner.lock().unwrap();
        g.agents.insert(
            id.clone(),
            (row.clone(), input.api_key.clone(), input.base_url.clone()),
        );
        Ok(Agent::from(row))
    }

    async fn update(&self, input: AgentUpdate) -> AppResult<Agent> {
        self.log(format!("update({})", input.id));
        let mut g = self.inner.lock().unwrap();
        let entry = g
            .agents
            .get_mut(&input.id)
            .ok_or_else(|| AppError::NotFound {
                resource: "agent",
                id: input.id.clone(),
            })?;
        // 逐字段应用更新，None 表示不修改
        if let Some(v) = input.name {
            entry.0.name = v;
        }
        if let Some(v) = input.provider {
            entry.0.provider = v;
        }
        if let Some(v) = input.model {
            entry.0.model = v;
        }
        if let Some(v) = input.system_prompt {
            entry.0.system_prompt = v;
        }
        if let Some(v) = input.base_url {
            entry.2 = v;
        }
        if let Some(v) = input.temperature {
            entry.0.temperature = v;
        }
        if let Some(v) = input.max_tokens {
            entry.0.max_tokens = v;
        }
        if let Some(v) = input.extra_params {
            entry.0.extra_params = serde_json::to_string(&v).unwrap_or_default();
        }
        if let Some(v) = input.sort_order {
            entry.0.sort_order = v;
        }
        if let Some(v) = input.cache_prompt {
            entry.0.cache_prompt = v as i32;
        }
        if let Some(v) = input.max_history_messages {
            entry.0.max_history_messages = v;
        }
        if let Some(v) = input.context_window {
            entry.0.context_window = v;
        }
        if let Some(v) = input.enabled_tools {
            entry.0.enabled_tools =
                v.map(|tools| serde_json::to_string(&tools).unwrap_or_default());
        }
        if let Some(v) = input.supports_vision {
            entry.0.supports_vision = v as i32;
        }
        if let Some(v) = input.workspace_path {
            entry.0.workspace_path = v;
        }
        if let Some(v) = input.avatar {
            entry.0.avatar = v;
        }
        // ModelProfile Phase 2：双层 Option 与 Sql 版语义对齐
        // （Some(None)=解除/清链、Some(Some)=设定；None=不动）
        if let Some(v) = input.model_profile_id {
            entry.0.model_profile_id = v;
        }
        if let Some(v) = input.fallback_profile_ids {
            entry.0.fallback_profile_ids =
                v.map(|ids| serde_json::to_string(&ids).unwrap_or_default());
        }
        entry.0.updated_at = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        Ok(Agent::from(entry.0.clone()))
    }

    async fn rotate_key(&self, input: RotateAgentKey) -> AppResult<Agent> {
        self.log(format!("rotate_key({})", input.agent_id));
        let mut g = self.inner.lock().unwrap();
        let entry = g
            .agents
            .get_mut(&input.agent_id)
            .ok_or_else(|| AppError::NotFound {
                resource: "agent",
                id: input.agent_id.clone(),
            })?;
        entry.1 = input.api_key.clone();
        if let Some(bu) = input.base_url.clone() {
            entry.2 = Some(bu);
        }
        Ok(Agent::from(entry.0.clone()))
    }

    async fn delete(&self, agent_id: &str) -> AppResult<()> {
        self.log(format!("delete({})", agent_id));
        let mut g = self.inner.lock().unwrap();
        g.agents.remove(agent_id);
        Ok(())
    }
}
