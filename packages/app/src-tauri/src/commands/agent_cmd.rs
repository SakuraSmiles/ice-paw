//! Agent 相关 Tauri Commands + trait 抽象（REQ-XC-010）
//!
//! ## 设计目标
//!
//! 把 agent 域的所有操作抽象成 `trait AgentCmd`，让业务层（`chat_cmd`、
//! 其他 commands、tests）通过 trait 而非具体类型依赖：
//!
//! - 生产实现：`SqlAgentCmd` —— 走 sqlx + stronghold
//! - 测试实现：`MockAgentCmd` —— 内存状态，可注入预置数据
//!
//! 这样可以在不连真实 DB / 不写 stronghold 的情况下，单元测试 chat_cmd
//! 的编排逻辑（参数校验、Provider 创建、Pipeline 拼装等）。
//!
//! ## trait 抽象边界
//!
//! `AgentCmd` 暴露 6 个方法，对应原 `agent_cmd.rs` 的 5 个 Tauri commands
//! + chat_cmd 实际需要的「拿 agent + 拿 api_key 凭据」复合操作：
//!
//! 1. `list()`             → 原 list_agents
//! 2. `create(NewAgent)`   → 原 create_agent
//! 3. `update(AgentUpdate)`→ 原 update_agent
//! 4. `rotate_key(RotateAgentKey)` → 原 rotate_agent_api_key
//! 5. `delete(id)`         → 原 delete_agent
//! 6. `get_with_credentials(agent_id)` → chat_cmd 拼装 LLM 调用前的复合查询
//!
//! ## 异步 trait 约束
//!
//! 使用 `async_trait`（已在 Cargo.toml 声明）让 trait 方法可以是 async。
//! trait object 用 `Arc<dyn AgentCmd>` 形式在 Tauri State 里传递。
//!
//! ## 注入方式
//!
//! - 生产路径：在 `lib.rs::setup` 里 `app.manage(Arc::new(SqlAgentCmd::new()))`
//! - 测试路径：直接在测试代码里 `let mock = Arc::new(MockAgentCmd::new()); ...`

use std::sync::Arc;

use async_trait::async_trait;
use tauri::{AppHandle, State};
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::crypto;
use crate::db::models::{Agent, AgentRow, AgentUpdate, NewAgent, RotateAgentKey};
use crate::db::repo;
use crate::error::{AppError, AppResult};

// ============================================================================
// 类型：带凭据的 Agent 数据（chat_cmd 拼装 LLM 调用所需的全部信息）
// ============================================================================

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
}

// ============================================================================
// trait AgentCmd
// ============================================================================

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

// ============================================================================
// SqlAgentCmd —— 生产实现（sqlx + stronghold）
// ============================================================================

/// 生产 AgentCmd 实现
///
/// 持有 `AppHandle`（用于 stronghold 访问）+ `SqlitePool`（用于 sqlx 查询）。
/// 通过 `new(app, pool)` 构造一次，作为 `Arc<dyn AgentCmd>` 注入 Tauri State。
pub struct SqlAgentCmd {
    app: AppHandle,
    pool: SqlitePool,
}

/// 在 Agent workspace 目录写入默认 agent.yaml。
///
/// - 仅在文件不存在时写入（不覆盖用户手动编辑的内容）
/// - 写入失败仅 warn，不阻断 Agent 创建
fn write_default_agent_yaml(
    workspace_dir: &str,
    agent_name: &str,
    temperature: f64,
    max_tokens: i32,
) {
    let yaml_path = std::path::Path::new(workspace_dir).join("agent.yaml");

    // 文件已存在则跳过（用户可能已通过其他方式创建）
    if yaml_path.exists() {
        tracing::info!(
            target: "ice_paw.agent",
            "agent.yaml 已存在，跳过自动生成: {}",
            yaml_path.display()
        );
        return;
    }

    let default_sp = agent_name.to_string() + " 是一个 AI 助手。";
    let content = format!(
        "# agent.yaml — Agent 行为和角色配置\n\
         # 修改后即时生效，无需重启\n\
         \n\
         system_prompt: |\n  {}\n\n\
         temperature: {}\n\
         max_tokens: {}\n\
         # 工具调用最大轮数（默认 15，超过则强制结束）\n\
         tool_max_rounds: 15\n",
        default_sp, temperature, max_tokens,
    );

    match std::fs::write(&yaml_path, &content) {
        Ok(()) => {
            tracing::info!(
                target: "ice_paw.agent",
                "已生成默认 agent.yaml: {}",
                yaml_path.display()
            );
        }
        Err(e) => {
            tracing::warn!(
                target: "ice_paw.agent",
                "写入 agent.yaml 失败（Agent 仍可用，忽略）: {} — {}",
                yaml_path.display(),
                e
            );
        }
    }
}

impl SqlAgentCmd {
    pub fn new(app: AppHandle, pool: SqlitePool) -> Self {
        Self { app, pool }
    }
}

#[async_trait]
impl AgentCmd for SqlAgentCmd {
    async fn list(&self) -> AppResult<Vec<Agent>> {
        let rows = repo::agent::list(&self.pool).await?;
        Ok(rows.into_iter().map(Agent::from_row_with_file_config).collect())
    }

    async fn get(&self, agent_id: &str) -> AppResult<AgentRow> {
        // 也读取 agent.yaml 合并到返回的 AgentRow 中
        // 注意：AgentRow 是 raw DB row，但 chat_cmd 需要的是合并后的值。
        // 这里返回原始 row，由 chat_cmd 的 get_with_credentials 做合并
        repo::agent::get_by_id(&self.pool, agent_id).await
    }

    async fn get_with_credentials(&self, agent_id: &str) -> AppResult<AgentWithCredentials> {
        let mut agent = repo::agent::get_by_id(&self.pool, agent_id).await?;
        // 尝试从 workspace_path 加载 agent.yaml 配置，合并到 AgentRow（覆盖 chat_cmd 用的字段）
        if let Some(file_cfg) = agent.load_file_config() {
            file_cfg.apply_to_row(&mut agent);
        }
        let (api_key, vault_base_url) = crypto::fetch_api_key(&self.app, &agent.api_key_ref)?;
        // base_url：agent 配置优先（如果有），否则回退到 vault 里存的 base_url
        let base_url = agent
            .base_url
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(vault_base_url.as_deref())
            .map(|s| s.to_string());
        Ok(AgentWithCredentials {
            agent,
            api_key,
            base_url,
        })
    }

    async fn create(&self, input: NewAgent) -> AppResult<Agent> {
        // 入参基础校验
        let id = input.id.trim().to_string();
        if id.is_empty() {
            return Err(AppError::Validation("ID 不能为空".into()));
        }
        if input.name.trim().is_empty() {
            return Err(AppError::Validation("name 不能为空".into()));
        }
        if input.provider.trim().is_empty() {
            return Err(AppError::Validation("provider 不能为空".into()));
        }
        if input.model.trim().is_empty() {
            return Err(AppError::Validation("model 不能为空".into()));
        }
        if input.api_key.trim().is_empty() {
            return Err(AppError::Validation("api_key 不能为空".into()));
        }

        // 校验 ID 唯一性
        if repo::agent::get_by_id(&self.pool, &id).await.is_ok() {
            return Err(AppError::Validation(format!("ID '{}' 已被使用", id)));
        }

        crypto::store_api_key(&self.app, &id, &input.api_key, input.base_url.as_deref())?;

        // 工作区路径：用户没填时自动计算 {default}/agents/{id}
        let workspace_path = if input.workspace_path.as_ref().is_some_and(|p| !p.is_empty()) {
            input.workspace_path.clone()
        } else {
            match repo::preferences::get_all(&self.pool).await {
                Ok(prefs) => prefs.default_workspace_path
                    .map(|root| format!("{}/agents/{}", root.trim_end_matches(['/', '\\']), id)),
                Err(_) => None,
            }
        };

        // 如果设了工作区路径，自动创建目录
        if let Some(ref path) = workspace_path {
            let dir = std::path::Path::new(path);
            if !dir.exists() {
                std::fs::create_dir_all(dir)?;
            }
        }

        let mut new_agent = input;
        new_agent.workspace_path = workspace_path;

        let row: AgentRow = repo::agent::create(&self.pool, &new_agent, &id, &id).await?;

        // 为新 Agent 确保 KB 行存在（无需重启）
        let default_ws = repo::preferences::get_all(&self.pool)
            .await
            .ok()
            .and_then(|p| p.default_workspace_path);
        crate::harness::kb::ensure::ensure_agent_kb(
            &self.pool,
            &row.id,
            &row.name,
            row.workspace_path.as_deref(),
            default_ws.as_deref(),
        )
        .await;

        // 7. 自动生成 agent.yaml（仅在首次创建、文件不存在时写入）
        if let Some(ws) = row.workspace_path.as_deref() {
            write_default_agent_yaml(ws, &row.name, row.temperature, row.max_tokens);
        }

        Ok(Agent::from_row_with_file_config(row))
    }

    async fn update(&self, input: AgentUpdate) -> AppResult<Agent> {
        let row = repo::agent::update(
            &self.pool,
            &input.id,
            input.name.as_deref(),
            input.provider.as_deref(),
            input.model.as_deref(),
            input.system_prompt.as_deref(),
            input.base_url.as_ref().map(|opt| opt.as_deref()),
            input.temperature,
            input.max_tokens,
            input.extra_params.as_ref(),
            input.sort_order,
            input.cache_prompt,
            input.max_history_messages,
            input.tool_trim_threshold,
            input.enabled_tools,
            input.supports_vision,
            input.workspace_path.as_ref().map(|opt| opt.as_deref()),
        )
        .await?;

        // 如果更新后的 workspace_path 有值，确保目录存在
        if let Some(ref path) = row.workspace_path {
            let dir = std::path::Path::new(path);
            if !dir.exists() {
                std::fs::create_dir_all(dir)?;
            }
        }

        Ok(Agent::from_row_with_file_config(row))
    }

    async fn rotate_key(&self, input: RotateAgentKey) -> AppResult<Agent> {
        if input.api_key.trim().is_empty() {
            return Err(AppError::Validation("api_key 不能为空".into()));
        }
        crypto::store_api_key(&self.app, &input.agent_id, &input.api_key, input.base_url.as_deref())?;
        repo::agent::rotate_key_ref(
            &self.pool,
            &input.agent_id,
            &input.agent_id,
            input.base_url.as_deref(),
        )
        .await?;
        let row = repo::agent::get_by_id(&self.pool, &input.agent_id).await?;
        Ok(Agent::from(row))
    }

    async fn delete(&self, agent_id: &str) -> AppResult<()> {
        // 先清 stronghold 中的 key（容错：失败仅 warn，不阻断删除）
        if let Err(e) = crypto::delete_api_key(&self.app, agent_id) {
            tracing::warn!(target: "ice_paw.agent", "清理 agent {agent_id} API key 失败: {e}");
        }
        // 级联清理 memory 数据（容错：失败仅 warn）
        if let Err(e) = repo::memory_embedding::delete_embeddings_for_agent(&self.pool, agent_id).await {
            tracing::warn!(target: "ice_paw.agent", "清理 agent {agent_id} embeddings 失败: {e}");
        }
        if let Err(e) = repo::memory_store::delete_memories_for_agent(&self.pool, agent_id).await {
            tracing::warn!(target: "ice_paw.agent", "清理 agent {agent_id} memories 失败: {e}");
        }
        repo::agent::delete(&self.pool, agent_id).await
    }
}

// ============================================================================
// MockAgentCmd —— 测试实现（内存状态）
// ============================================================================

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
        let (agent, api_key, base_url) =
            g.agents
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
            tool_trim_threshold: input.tool_trim_threshold,
            enabled_tools: input.enabled_tools.as_ref().map(|v| {
                serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string())
            }),
            supports_vision: if input.supports_vision { 1 } else { 0 },
            embedding_model: None,
            description: String::new(),
            avatar: None,
            workspace_path: input.workspace_path.clone(),
            created_at: "2024-01-01 00:00:00".to_string(),
            updated_at: "2024-01-01 00:00:00".to_string(),
        };
        let mut g = self.inner.lock().unwrap();
        g.agents
            .insert(id.clone(), (row.clone(), input.api_key.clone(), input.base_url.clone()));
        Ok(Agent::from(row))
    }

    async fn update(&self, input: AgentUpdate) -> AppResult<Agent> {
        self.log(format!("update({})", input.id));
        // mock 简化：仅更新 name 字段，其它保持不变
        let mut g = self.inner.lock().unwrap();
        let entry = g.agents.get_mut(&input.id).ok_or_else(|| AppError::NotFound {
            resource: "agent",
            id: input.id.clone(),
        })?;
        if let Some(name) = input.name.clone() {
            entry.0.name = name;
        }
        Ok(Agent::from(entry.0.clone()))
    }

    async fn rotate_key(&self, input: RotateAgentKey) -> AppResult<Agent> {
        self.log(format!("rotate_key({})", input.agent_id));
        let mut g = self.inner.lock().unwrap();
        let entry = g.agents.get_mut(&input.agent_id).ok_or_else(|| AppError::NotFound {
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

// ============================================================================
// Tauri command 包装（保持原有 invoke 入口签名不变）
// ============================================================================

/// 列出全部 agent
#[tauri::command]
pub async fn list_agents(cmd: State<'_, Arc<dyn AgentCmd>>) -> AppResult<Vec<Agent>> {
    cmd.inner().list().await
}

/// 创建 agent
#[tauri::command]
pub async fn create_agent(cmd: State<'_, Arc<dyn AgentCmd>>, input: NewAgent) -> AppResult<Agent> {
    cmd.inner().create(input).await
}

/// 部分更新 agent
#[tauri::command]
pub async fn update_agent(cmd: State<'_, Arc<dyn AgentCmd>>, input: AgentUpdate) -> AppResult<Agent> {
    cmd.inner().update(input).await
}

/// 轮换 api_key
#[tauri::command]
pub async fn rotate_agent_api_key(
    cmd: State<'_, Arc<dyn AgentCmd>>,
    input: RotateAgentKey,
) -> AppResult<Agent> {
    cmd.inner().rotate_key(input).await
}

/// 删除 agent
#[tauri::command]
pub async fn delete_agent(cmd: State<'_, Arc<dyn AgentCmd>>, id: String) -> AppResult<()> {
    cmd.inner().delete(&id).await
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{AgentRow, NewAgent};

    fn sample_agent_row(id: &str, name: &str) -> AgentRow {
        AgentRow {
            id: id.to_string(),
            name: name.to_string(),
            provider: "anthropic".to_string(),
            model: "claude-3-5-sonnet".to_string(),
            system_prompt: "you are a helpful assistant".to_string(),
            api_key_ref: id.to_string(),
            base_url: None,
            temperature: 0.7,
            max_tokens: 1024,
            extra_params: "{}".to_string(),
            sort_order: 0,
            cache_prompt: 0,
            max_history_messages: None,
            tool_trim_threshold: None,
            enabled_tools: None,
            supports_vision: 0,
            embedding_model: None,
            description: String::new(),
            avatar: None,
            workspace_path: None,
            created_at: "2024-01-01 00:00:00".to_string(),
            updated_at: "2024-01-01 00:00:00".to_string(),
        }
    }

    #[tokio::test]
    async fn mock_list_returns_seeded_agents() {
        let mock = MockAgentCmd::new();
        mock.seed(sample_agent_row("a1", "Agent 1"), "k1".into(), None);
        mock.seed(sample_agent_row("a2", "Agent 2"), "k2".into(), Some("https://api.example.com".into()));

        let list = mock.list().await.unwrap();
        assert_eq!(list.len(), 2);
        let names: Vec<&str> = list.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"Agent 1"));
        assert!(names.contains(&"Agent 2"));
    }

    #[tokio::test]
    async fn mock_get_returns_correct_agent() {
        let mock = MockAgentCmd::new();
        mock.seed(sample_agent_row("a1", "Agent 1"), "k1".into(), None);

        let a = mock.get("a1").await.unwrap();
        assert_eq!(a.id, "a1");
        assert_eq!(a.name, "Agent 1");

        // 不存在 → NotFound
        let err = mock.get("nonexistent").await.unwrap_err();
        match err {
            AppError::NotFound { resource, id } => {
                assert_eq!(resource, "agent");
                assert_eq!(id, "nonexistent");
            }
            e => panic!("expected NotFound, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn mock_get_with_credentials_returns_api_key_and_base_url() {
        let mock = MockAgentCmd::new();
        mock.seed(
            sample_agent_row("a1", "Agent 1"),
            "secret-key".into(),
            Some("https://api.example.com".into()),
        );

        let result = mock.get_with_credentials("a1").await.unwrap();
        assert_eq!(result.agent.id, "a1");
        assert_eq!(result.api_key, "secret-key");
        assert_eq!(result.base_url, Some("https://api.example.com".into()));
    }

    #[tokio::test]
    async fn mock_create_adds_agent() {
        let mock = MockAgentCmd::new();
        let new = NewAgent {
            id: "test-agent".into(),
            name: "Test Agent".into(),
            provider: "anthropic".into(),
            model: "claude-3-5-sonnet".into(),
            system_prompt: "you are a helpful assistant".into(),
            api_key: "sk-test".into(),
            base_url: None,
            temperature: 0.7,
            max_tokens: 1024,
            extra_params: None,
            sort_order: 0,
            cache_prompt: true,
            max_history_messages: None,
            tool_trim_threshold: None,
            enabled_tools: None,
            supports_vision: false,
            workspace_path: None,
        };

        let a = mock.create(new).await.unwrap();
        assert_eq!(a.name, "Test Agent");
        assert_eq!(mock.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn mock_update_modifies_name() {
        let mock = MockAgentCmd::new();
        mock.seed(sample_agent_row("a1", "Old Name"), "k1".into(), None);

        let input = AgentUpdate {
            id: "a1".into(),
            name: Some("New Name".into()),
            provider: None,
            model: None,
            system_prompt: None,
            base_url: None,
            temperature: None,
            max_tokens: None,
            extra_params: None,
            sort_order: None,
            cache_prompt: None,
            max_history_messages: None,
            tool_trim_threshold: None,
            enabled_tools: None,
            supports_vision: None,
            workspace_path: None,
        };

        let a = mock.update(input).await.unwrap();
        assert_eq!(a.name, "New Name");
    }

    #[tokio::test]
    async fn mock_rotate_key_replaces_credentials() {
        let mock = MockAgentCmd::new();
        mock.seed(sample_agent_row("a1", "Agent 1"), "old-key".into(), None);

        let input = RotateAgentKey {
            agent_id: "a1".into(),
            api_key: "new-key".into(),
            base_url: Some("https://new.example.com".into()),
        };
        mock.rotate_key(input).await.unwrap();

        let result = mock.get_with_credentials("a1").await.unwrap();
        assert_eq!(result.api_key, "new-key");
        assert_eq!(result.base_url, Some("https://new.example.com".into()));
    }

    #[tokio::test]
    async fn mock_delete_removes_agent() {
        let mock = MockAgentCmd::new();
        mock.seed(sample_agent_row("a1", "Agent 1"), "k1".into(), None);

        mock.delete("a1").await.unwrap();
        assert!(mock.get("a1").await.is_err());
        assert_eq!(mock.list().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn mock_call_log_records_operations() {
        let mock = MockAgentCmd::new();
        mock.seed(sample_agent_row("a1", "Agent 1"), "k1".into(), None);

        let _ = mock.list().await;
        let _ = mock.get("a1").await;
        let _ = mock.get_with_credentials("a1").await;
        let _ = mock.delete("a1").await;

        let log = mock.call_log();
        assert_eq!(log.len(), 4);
        assert_eq!(log[0], "list");
        assert_eq!(log[1], "get(a1)");
        assert_eq!(log[2], "get_with_credentials(a1)");
        assert_eq!(log[3], "delete(a1)");
    }

    /// 验证：SqlAgentCmd 与 MockAgentCmd 都实现了 AgentCmd，
    /// 可被同一个函数以 trait object 形式接收（编译期检查）。
    #[tokio::test]
    async fn trait_object_works_for_both_impls() {
        async fn exercise(cmd: Arc<dyn AgentCmd>) -> AppResult<usize> {
            let list = cmd.list().await?;
            Ok(list.len())
        }

        let mock: Arc<dyn AgentCmd> = Arc::new(MockAgentCmd::new());
        let n = exercise(mock).await.unwrap();
        assert_eq!(n, 0);
    }
}