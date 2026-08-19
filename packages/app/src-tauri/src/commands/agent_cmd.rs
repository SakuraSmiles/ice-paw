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
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::crypto;
use crate::db::models::{Agent, AgentRow, AgentUpdate, HookConfig, NewAgent, RotateAgentKey};
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::kb::{ensure, watcher_manager::KbWatcherManager};
use crate::harness::provider::provider_requires_key;

// ============================================================================
// 入参校验
// ============================================================================

/// `NewAgent` 入参校验（`SqlAgentCmd::create` / `MockAgentCmd::create` 共用）。
///
/// api_key 是否必填按 provider 目录判定（`provider_requires_key`）：
/// ollama / custom 等本地或免鉴权服务允许空 key——**空串仍会经
/// `crypto::store_api_key` 存一条空记录**（Stronghold 无记录时
/// `fetch_api_key` 返回 NotFound，聊天链路 `get_with_credentials` 会报错，
/// 所以必须占位）；OpenAI adapter 发空 Bearer，本地服务忽略。
fn validate_new_agent(input: &NewAgent) -> AppResult<()> {
    if input.id.trim().is_empty() {
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
    if provider_requires_key(input.provider.trim()) && input.api_key.trim().is_empty() {
        return Err(AppError::Validation("api_key 不能为空".into()));
    }
    Ok(())
}

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
    /// 对话钩子配置（来自 agent.yaml `hooks` 字段；hooks 不进 DB，纯文件）。
    /// chat_cmd 据此在各生命周期点执行 inject_prompt/call_tool/log。
    pub hooks: HookConfig,
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
#[allow(clippy::too_many_arguments)]
fn write_default_agent_yaml(
    workspace_dir: &str,
    agent_name: &str,
    provider: &str,
    model: &str,
    system_prompt: Option<&str>,
    temperature: f64,
    max_tokens: i32,
    enabled_tools: Option<&[String]>,
    base_url: Option<&str>,
) {
    let yaml_path = std::path::Path::new(workspace_dir).join("agent.yaml");

    // 文件已存在则跳过
    if yaml_path.exists() {
        tracing::info!(
            target: "ice_paw.agent",
            "agent.yaml 已存在，跳过自动生成: {}",
            yaml_path.display()
        );
        return;
    }

    let content = build_default_agent_yaml_content(
        agent_name,
        provider,
        model,
        system_prompt,
        temperature,
        max_tokens,
        enabled_tools,
        base_url,
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

/// 构造默认 agent.yaml 内容（纯函数，为单测让路）。
///
/// 模板纪律：`tool_max_rounds` / `max_total_tokens` 一律**注释掉**——显式值
/// 是 B1 语义下的硬上限（触顶即停、不自动续期），写进模板会让所有新 agent
/// 默认失去自动续期额度；留空 = 软默认 + 自动续期（长任务不误杀）。
#[allow(clippy::too_many_arguments)]
fn build_default_agent_yaml_content(
    agent_name: &str,
    provider: &str,
    model: &str,
    system_prompt: Option<&str>,
    temperature: f64,
    max_tokens: i32,
    enabled_tools: Option<&[String]>,
    base_url: Option<&str>,
) -> String {
    let default_sp = format!("{} 是一个 AI 助手。", agent_name);
    let sp = system_prompt
        .filter(|s| !s.is_empty())
        .unwrap_or(&default_sp);
    // YAML multiline: 每行缩进 2 空格
    let sp_indented = sp
        .lines()
        .map(|l| format!("  {}", l))
        .collect::<Vec<_>>()
        .join("\n");

    let mut content = format!(
        "# agent.yaml — Agent 行为和角色配置\n\
         # 修改后即时生效，无需重启\n\
         \n\
         provider: {}\n\
         model: {}\n\
         system_prompt: |\n{}\n\
         temperature: {}\n\
         max_tokens: {}\n\
         # 工具调用最大轮数（默认 50 + 自动续期 2 次；显式设置 = 硬上限，触顶即停不自动续期）\n\
         # tool_max_rounds: 50\n\
         # Token 预算上限（默认按上下文窗口自适应 3× + 自动续期 2 次；显式设置 = 硬上限，长对话会频繁中断）\n\
         # max_total_tokens: 3000000\n",
        provider, model, sp_indented, temperature, max_tokens,
    );

    if let Some(tools) = enabled_tools {
        if !tools.is_empty() {
            content.push_str("\nenabled_tools:\n");
            for t in tools {
                content.push_str(&format!("  - {}\n", t));
            }
        }
    }
    if let Some(url) = base_url {
        if !url.is_empty() {
            content.push_str(&format!("\nbase_url: {}\n", url));
        }
    }
    content
}

impl SqlAgentCmd {
    pub fn new(app: AppHandle, pool: SqlitePool) -> Self {
        Self { app, pool }
    }

    /// 取出全局 KB watcher 管理器（lib.rs boot 后存在；早期 / 测试可能缺失 → None）。
    ///
    /// agent 增删改时用它对账目录监听（模式 A 治本：运行期新建 agent 的 KB 目录
    /// 不再需要重启即可被监听）。缺失时调用方静默跳过，回退「重启后补」语义。
    fn watcher(&self) -> Option<Arc<KbWatcherManager>> {
        self.app
            .try_state::<Arc<KbWatcherManager>>()
            .map(|s| s.inner().clone())
    }

    /// 解析某 agent 的 KB 行（scope=agent, owner=agent_id），用于 watcher 增删改时
    /// 取 kb_id + 当前 directory。无则 None（agent 无约定 KB）。
    async fn agent_kb(&self, agent_id: &str) -> Option<crate::db::models::Kb> {
        repo::kb::list_by_scope(&self.pool, "agent", Some(agent_id))
            .await
            .ok()
            .and_then(|v| v.into_iter().next())
    }
}

#[async_trait]
impl AgentCmd for SqlAgentCmd {
    async fn list(&self) -> AppResult<Vec<Agent>> {
        let rows = repo::agent::list(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(Agent::from_row_with_file_config)
            .collect())
    }

    async fn get(&self, agent_id: &str) -> AppResult<AgentRow> {
        // 也读取 agent.yaml 合并到返回的 AgentRow 中
        // 注意：AgentRow 是 raw DB row，但 chat_cmd 需要的是合并后的值。
        // 这里返回原始 row，由 chat_cmd 的 get_with_credentials 做合并
        repo::agent::get_by_id(&self.pool, agent_id).await
    }

    async fn get_with_credentials(&self, agent_id: &str) -> AppResult<AgentWithCredentials> {
        let mut agent = repo::agent::get_by_id(&self.pool, agent_id).await?;
        // 尝试从 workspace_path 加载 agent.yaml 配置，合并到 AgentRow（覆盖 chat_cmd 用的字段）。
        // 同时提取 hooks（hooks 不进 DB，纯文件，不参与 apply_to_row 的字段覆盖）。
        let hooks: HookConfig = match agent.load_file_config() {
            Some(file_cfg) => {
                let h = file_cfg.hooks.clone().unwrap_or_default();
                file_cfg.apply_to_row(&mut agent);
                h
            }
            None => HookConfig::default(),
        };
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
            hooks,
        })
    }

    async fn create(&self, input: NewAgent) -> AppResult<Agent> {
        // 入参基础校验（含 per-provider 的 api_key 必填判定）
        validate_new_agent(&input)?;
        let id = input.id.trim().to_string();

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
                Ok(prefs) => prefs
                    .default_workspace_path
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
        ensure::ensure_agent_kb(
            &self.pool,
            &row.id,
            &row.name,
            row.workspace_path.as_deref(),
            default_ws.as_deref(),
        )
        .await;

        // 注册 watcher：运行期监听新建 agent 的 KB 目录（无需重启）。
        // ensure_agent_kb 已建好 KB 行 + 触发初始索引，此处查 KB 行拿 kb_id/directory
        // 登记到 watcher，后续手动往目录拖文件也能被增量索引。
        if let (Some(wm), Some(kb)) = (self.watcher(), self.agent_kb(&row.id).await) {
            wm.add_watch(kb.id, kb.directory);
        }

        // 7. 自动生成 agent.yaml（含完整配置：provider/model/tools/system_prompt/base_url）
        if let Some(ws) = row.workspace_path.as_deref() {
            write_default_agent_yaml(
                ws,
                &row.name,
                &row.provider,
                &row.model,
                Some(&row.system_prompt),
                row.temperature,
                row.max_tokens,
                row.enabled_tools
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
                    .as_deref(),
                row.base_url.as_deref(),
            );
        }

        Ok(Agent::from_row_with_file_config(row))
    }

    async fn update(&self, input: AgentUpdate) -> AppResult<Agent> {
        // 记录旧 workspace_path，用于检测变更后通知 watcher 重新绑定。
        let old_workspace = repo::agent::get_by_id(&self.pool, &input.id)
            .await
            .ok()
            .and_then(|r| r.workspace_path);
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
            input.context_window,
            input.enabled_tools,
            input.supports_vision,
            input.workspace_path.as_ref().map(|opt| opt.as_deref()),
            input.avatar.as_ref().map(|opt| opt.as_deref()),
        )
        .await?;

        // 如果更新后的 workspace_path 有值，确保目录存在
        if let Some(ref path) = row.workspace_path {
            let dir = std::path::Path::new(path);
            if !dir.exists() {
                std::fs::create_dir_all(dir)?;
            }
        }

        // workspace 变更 → watcher 重新绑定到新 knowledge 目录（best-effort）。
        // 注：KB 行的 directory 字段不可变（repo::kb::update 仅 name/enabled），
        // 此处仅让 watcher 跟到新目录以保证增量索引；KB 行 directory 停留旧值是既有局限。
        if old_workspace != row.workspace_path {
            if let (Some(wm), Some(kb)) = (self.watcher(), self.agent_kb(&input.id).await) {
                let default_ws = repo::preferences::get_all(&self.pool)
                    .await
                    .ok()
                    .and_then(|p| p.default_workspace_path);
                if let Some(root) = ensure::agent_workspace_root(
                    row.workspace_path.as_deref(),
                    default_ws.as_deref(),
                    &row.id,
                ) {
                    let new_dir = ensure::knowledge_dir(&root)
                        .to_string_lossy()
                        .replace('\\', "/");
                    wm.rebind_watch(&kb.id, Some(&kb.directory), &new_dir);
                }
            }
        }

        Ok(Agent::from_row_with_file_config(row))
    }

    async fn rotate_key(&self, input: RotateAgentKey) -> AppResult<Agent> {
        // key 是否必填按该 agent 的 provider 判定（ollama/custom 等免鉴权
        // 服务允许空 key——空串仍走 store，清掉旧值）
        let row = repo::agent::get_by_id(&self.pool, &input.agent_id).await?;
        if provider_requires_key(&row.provider) && input.api_key.trim().is_empty() {
            return Err(AppError::Validation("api_key 不能为空".into()));
        }
        crypto::store_api_key(
            &self.app,
            &input.agent_id,
            &input.api_key,
            input.base_url.as_deref(),
        )?;
        repo::agent::rotate_key_ref(
            &self.pool,
            &input.agent_id,
            &input.agent_id,
            input.base_url.as_deref(),
        )
        .await?;
        let fresh = repo::agent::get_by_id(&self.pool, &input.agent_id).await?;
        Ok(Agent::from(fresh))
    }

    async fn delete(&self, agent_id: &str) -> AppResult<()> {
        // 取消 watcher 监听（删 KB 数据前查 KB 行拿 directory；级联删除后查不到）。
        if let (Some(wm), Some(kb)) = (self.watcher(), self.agent_kb(agent_id).await) {
            wm.remove_watch(&kb.directory);
        }
        // 先清 stronghold 中的 key（容错：失败仅 warn，不阻断删除）
        if let Err(e) = crypto::delete_api_key(&self.app, agent_id) {
            tracing::warn!(target: "ice_paw.agent", "清理 agent {agent_id} API key 失败: {e}");
        }
        // 级联清理 memory 数据（容错：失败仅 warn）
        if let Err(e) =
            repo::memory_embedding::delete_embeddings_for_agent(&self.pool, agent_id).await
        {
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
            supports_vision: if input.supports_vision { 1 } else { 0 },
            description: String::new(),
            avatar: None,
            workspace_path: input.workspace_path.clone(),
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
pub async fn update_agent(
    cmd: State<'_, Arc<dyn AgentCmd>>,
    input: AgentUpdate,
) -> AppResult<Agent> {
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
            context_window: None,
            enabled_tools: None,
            supports_vision: 0,
            description: String::new(),
            avatar: None,
            workspace_path: None,
            created_at: "2024-01-01 00:00:00".to_string(),
            updated_at: "2024-01-01 00:00:00".to_string(),
        }
    }

    /// 构造合法 NewAgent 基线（各用例只改关心的字段）
    fn new_agent(provider: &str, api_key: &str) -> NewAgent {
        NewAgent {
            id: "test-agent".into(),
            name: "测试".into(),
            provider: provider.into(),
            model: "m".into(),
            system_prompt: String::new(),
            api_key: api_key.into(),
            base_url: None,
            temperature: 0.7,
            max_tokens: 16384,
            extra_params: None,
            sort_order: 0,
            cache_prompt: true,
            max_history_messages: None,
            context_window: None,
            enabled_tools: None,
            supports_vision: false,
            workspace_path: None,
            avatar: None,
        }
    }

    #[test]
    fn validate_allows_empty_key_for_ollama_and_custom() {
        // 免鉴权 provider：空 key 合法（空串仍会存 Stronghold 占位记录）
        assert!(validate_new_agent(&new_agent("ollama", "")).is_ok());
        assert!(validate_new_agent(&new_agent("custom", "")).is_ok());
    }

    #[test]
    fn validate_rejects_empty_key_for_keyed_providers() {
        for p in [
            "openai",
            "glm",
            "glm-coding",
            "deepseek",
            "anthropic",
            "minimax",
            "minimax-cn",
        ] {
            let err = validate_new_agent(&new_agent(p, "  ")).unwrap_err();
            assert!(matches!(err, AppError::Validation(_)), "{p} 空 key 应被拒");
        }
        // 未知 provider 保守按需要 key 处理
        assert!(validate_new_agent(&new_agent("totally-unknown", "")).is_err());
    }

    #[test]
    fn validate_accepts_keyed_provider_with_key() {
        assert!(validate_new_agent(&new_agent("anthropic", "sk-xxx")).is_ok());
    }

    #[test]
    fn validate_rejects_missing_id_and_name() {
        let mut a = new_agent("ollama", "");
        a.id = "  ".into();
        assert!(matches!(
            validate_new_agent(&a),
            Err(AppError::Validation(_))
        ));
        let mut b = new_agent("ollama", "");
        b.name = String::new();
        assert!(matches!(
            validate_new_agent(&b),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn validate_rejects_empty_provider_and_model() {
        let mut a = new_agent("ollama", "");
        a.provider = " ".into();
        assert!(matches!(
            validate_new_agent(&a),
            Err(AppError::Validation(_))
        ));
        let mut b = new_agent("ollama", "");
        b.model = "".into();
        assert!(matches!(
            validate_new_agent(&b),
            Err(AppError::Validation(_))
        ));
    }

    #[tokio::test]
    async fn mock_list_returns_seeded_agents() {
        let mock = MockAgentCmd::new();
        mock.seed(sample_agent_row("a1", "Agent 1"), "k1".into(), None);
        mock.seed(
            sample_agent_row("a2", "Agent 2"),
            "k2".into(),
            Some("https://api.example.com".into()),
        );

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
            context_window: None,
            enabled_tools: None,
            supports_vision: false,
            workspace_path: None,
            avatar: None,
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
            context_window: None,
            enabled_tools: None,
            supports_vision: None,
            workspace_path: None,
            avatar: None,
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

    /// 模板去雷回归：默认 agent.yaml 不得含活跃的 tool_max_rounds /
    /// max_total_tokens 行——显式值是 B1 硬上限语义（触顶即停、不自动续期），
    /// 写进模板会让所有新 agent 默认失去自动续期额度。
    #[test]
    fn default_yaml_template_comments_out_hard_caps() {
        let content = build_default_agent_yaml_content(
            "测试",
            "glm",
            "glm-5.2",
            None,
            0.7,
            4096,
            Some(&["read_file".to_string()]),
            None,
        );
        assert!(
            content.contains("# tool_max_rounds: 50"),
            "tool_max_rounds 应为注释行: {content}"
        );
        assert!(
            content.contains("# max_total_tokens: 3000000"),
            "max_total_tokens 应为注释行: {content}"
        );
        // 不得存在行首活跃（未注释）的两行
        for line in content.lines() {
            let trimmed = line.trim_start();
            assert!(
                !(trimmed.starts_with("tool_max_rounds:")
                    || trimmed.starts_with("max_total_tokens:")),
                "不得有活跃硬上限行: {line}"
            );
        }
        // 常规字段照常生成
        assert!(content.contains("provider: glm"));
        assert!(content.contains("model: glm-5.2"));
        assert!(content.contains("max_tokens: 4096"));
        assert!(content.contains("- read_file"));
    }
}
