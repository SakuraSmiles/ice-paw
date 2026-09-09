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
#[cfg(test)]
use uuid::Uuid;

use sqlx::SqlitePool;

use crate::crypto;
use crate::db::models::{Agent, AgentRow, AgentUpdate, HookConfig, NewAgent, RotateAgentKey};
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::kb::{ensure, watcher_manager::KbWatcherManager};
use crate::harness::provider::{
    provider_default_url, provider_requires_base_url, provider_requires_key,
};

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
///
/// ModelProfile Phase 2：引用模式（`model_profile_id` 非空）跳过 provider/
/// model/api_key 必填——模型身份来自 profile，api_key 空串占位（agent 自身
/// 槽位不再被引用路径读取）；降级链非空时要求主档存在。
fn validate_new_agent(input: &NewAgent) -> AppResult<()> {
    if input.id.trim().is_empty() {
        return Err(AppError::Validation("ID 不能为空".into()));
    }
    if input.name.trim().is_empty() {
        return Err(AppError::Validation("name 不能为空".into()));
    }
    if input
        .model_profile_id
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty())
    {
        validate_fallback_has_primary(
            true,
            input
                .fallback_profile_ids
                .as_ref()
                .is_some_and(|v| !v.is_empty()),
        )?;
        return Ok(());
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

/// 降级链依赖主档：非空链必须搭配主档引用（链的起点是主 profile，legacy
/// agent 无主档链无处生效）。纯函数，create/update 共用（两处入参形态不同，
/// 链是否非空由调用方算好传入）。
fn validate_fallback_has_primary(has_primary: bool, chain_nonempty: bool) -> AppResult<()> {
    if chain_nonempty && !has_primary {
        return Err(AppError::Validation(
            "降级链需要先选择主模型（引用的模型配置）".into(),
        ));
    }
    Ok(())
}

/// 手动新建字段能否物化为 ModelProfile 实体（Phase 3，2026-09-08 拍板：新建
/// 表单保留手写配置、保存时自动转实体）。UI 路径经 `validate_new_agent` 后
/// 恒真；旁路（提案 CreateAgent 等）凭据不齐时为 false → legacy 行照常可用
/// （不硬造 keyless 实体），编辑时选实体即转正。判定与 boot 存量抽离
/// （agent_profile_migration）的跳过条件同构。
fn manual_materializable(input: &NewAgent) -> bool {
    let provider = input.provider.trim();
    if provider.is_empty() || input.model.trim().is_empty() {
        return false;
    }
    if provider_requires_key(provider) && input.api_key.trim().is_empty() {
        return false;
    }
    let explicit_url = input
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    !provider_requires_base_url(provider) || explicit_url.is_some()
}

/// AgentUpdate 冲突校验（纯函数，便于测试回归）。
///
/// 设引用（Some(Some)）与 provider/model/base_url 同批 → 拒：快照列族由
/// profile 解析产生，同批手填会被下一轮解析覆盖，形成「看似改了实则没改」
/// 的假象。解除引用（Some(None)）与手填同批**合法**——切回手动模式一并完成
/// （AgentForm 手动模式提交正是这个形状）。降级链检查走
/// [`validate_fallback_has_primary`]（目标态 = input 显式值优先、否则旧行值）。
///
/// 引用态行（旧行 model_profile_id 非空）未显式解除引用（字段缺席 = 引用
/// 保持）+ 手填快照族 → 同理拒：行仍挂在引用上，快照列入库后下轮解析即覆盖
/// 回实体值，用户批准的换模型静默失效，agent.yaml 镜像还会把永不生效的值写
/// 进文件（审查实案：提案卡批准 update_agent 直传快照族、不带
/// model_profile_id——绕过前端表单的引用态锁定）。
fn validate_update_model_fields_conflict(
    input: &AgentUpdate,
    old_has_primary: bool,
) -> AppResult<()> {
    if matches!(input.model_profile_id, Some(Some(_)))
        && (input.provider.is_some() || input.model.is_some() || input.base_url.is_some())
    {
        return Err(AppError::Validation(
            "设模型引用时不能同时修改厂商/模型/端点——请先保存引用，再单独调整".into(),
        ));
    }
    // 引用态行 + 引用字段缺席 + 手填快照族 → 拒。Some(None)（显式解除回
    // legacy）+ 手填 = 合法（切回手动模式一并完成），不进本分支。
    if old_has_primary
        && input.model_profile_id.is_none()
        && (input.provider.is_some() || input.model.is_some() || input.base_url.is_some())
    {
        return Err(AppError::Validation(
            "该 Agent 挂着模型配置引用，厂商/模型/端点由所引用的模型配置决定，本次修改不会生效——这些列会在下轮对话被实体值覆盖。请到「设置 → 模型」修改对应模型配置，或先解除引用改回手动模式后再修改".into(),
        ));
    }
    // 目标态主档：显式传了以传值为准（Some(None)=解除 → 无主档），否则沿用旧行
    let target_has_primary = match &input.model_profile_id {
        Some(opt) => opt.as_deref().is_some_and(|v| !v.trim().is_empty()),
        None => old_has_primary,
    };
    let chain_nonempty = matches!(
        &input.fallback_profile_ids,
        Some(Some(v)) if !v.is_empty()
    );
    validate_fallback_has_primary(target_has_primary, chain_nonempty)?;
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
    /// Word 文档样式偏好（agent.yaml `word_style_profile` 自由文字块；不进 DB，
    /// 纯文件）——D12 双轨承载。非空时注入 system prompt「Word 文档样式偏好」
    /// 小节；与 hooks 同款旁路：不参与 apply_to_row 的字段覆盖。
    pub word_style_profile: Option<String>,
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

/// B：换厂商时未显式提供 base_url → 新厂商注册表默认地址（端点跟随厂商）。
/// 纯函数便于测试；custom/未知 provider 无默认（空串）→ None（保持不改）。
fn default_url_on_provider_switch(
    provider_changed: bool,
    new_provider: Option<&str>,
) -> Option<String> {
    if !provider_changed {
        return None;
    }
    new_provider
        .map(provider_default_url)
        .filter(|url| !url.is_empty())
}

/// base_url 双层 Option 解析（纯函数，便于测试回归）。
///
/// 显式 `Some(opt)` → 照设/照清（Some(None)=清空端点是合法显式意图）；
/// absent → 仅「换厂商且有注册表默认」时跟随（端点跟随厂商）；否则 `None`
/// = 保持不改。此前 absent 一律映射成 `Some(switch_url)`，未换厂商时变成
/// `Some(None)` = **静默清空端点**——任何不带 base_url 的 update（含提案卡）
/// 都会抹掉测试连接选定的端点；custom 换入无默认也被清空（与 B 修注释
/// 「保持不改」矛盾）。与 MockAgentCmd 的 if-let-Some 语义对齐。
fn resolve_base_url_arg<'a>(
    explicit: Option<Option<&'a str>>,
    switch_url: Option<&'a str>,
) -> Option<Option<&'a str>> {
    match explicit {
        Some(opt) => Some(opt),
        None => switch_url.map(Some),
    }
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

    /// 组装前端 Agent DTO：合并 agent.yaml + `has_api_key` 真相实查。
    /// models.rs 的旧值 `!api_key_ref.is_empty()` 恒真（api_key_ref 建行时就被填成
    /// agent id），「未配置 Key」徽标形同虚设。真相 = 免 key 厂商（ollama/custom）
    /// 恒 true；要求 key 的厂商查 stronghold 记录存在（空占位记录只会出现在免 key
    /// 厂商，create/rotate 的必填校验保证）。
    ///
    /// Phase 3（2026-09-08）：**引用形态查 profile 槽位**——新建表单保存即物化
    /// 后 agent 槽位不再写 Key（只落 profile 槽位），旧判据会让每个新 agent 误报
    /// 「未配置 Key」。悬空引用（profile 已删）诚实显示未配置（Key 无处可取）。
    /// 每行一次小查询（agent 数量级小，接受 N+1）。
    async fn agent_dto(&self, row: AgentRow) -> Agent {
        let has_api_key = if let Some(pid) = row
            .model_profile_id
            .as_deref()
            .filter(|p| !p.trim().is_empty())
        {
            repo::model_profile::get_by_id(&self.pool, pid)
                .await
                .map(|p| {
                    !provider_requires_key(&p.provider)
                        || crypto::has_api_key(&self.app, &p.api_key_ref).unwrap_or(false)
                })
                .unwrap_or(false)
        } else {
            !provider_requires_key(&row.provider)
                || crypto::has_api_key(&self.app, &row.api_key_ref).unwrap_or(false)
        };
        let mut agent = Agent::from_row_with_file_config(row);
        agent.has_api_key = has_api_key;
        agent
    }

    /// legacy 路径凭据（agent 行 api_key_ref 槽位 + 行 base_url/vault 兜底）。
    /// 引用模式的悬空降级与无引用的 agent 共用。
    async fn legacy_credentials(&self, agent: &AgentRow) -> AppResult<(String, Option<String>)> {
        let (api_key, vault_base_url) = crypto::fetch_api_key(&self.app, &agent.api_key_ref)?;
        // base_url：agent 配置优先（如果有），否则回退到 vault 里存的 base_url
        let base_url = agent
            .base_url
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(vault_base_url.as_deref())
            .map(|s| s.to_string());
        Ok((api_key, base_url))
    }
}

#[async_trait]
impl AgentCmd for SqlAgentCmd {
    async fn list(&self) -> AppResult<Vec<Agent>> {
        let rows = repo::agent::list(&self.pool).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(self.agent_dto(row).await);
        }
        Ok(out)
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
        // 同时提取 hooks 与 word_style_profile（两者不进 DB，纯文件，不参与
        // apply_to_row 的字段覆盖）。
        let (hooks, word_style_profile): (HookConfig, Option<String>) =
            match agent.load_file_config() {
                Some(file_cfg) => {
                    let h = file_cfg.hooks.clone().unwrap_or_default();
                    let w = file_cfg
                        .word_style_profile
                        .clone()
                        .filter(|s| !s.trim().is_empty());
                    file_cfg.apply_to_row(&mut agent);
                    (h, w)
                }
                None => (HookConfig::default(), None),
            };
        // ModelProfile Phase 2：引用模式解析——profile 是模型身份唯一权威
        // （provider/model/base_url/api_key 四值），快照列仅显示用。值变才回写
        // 快照（trg_agents_upd 无 WHEN，同值零写入不刷 updated_at）。悬空引用
        // （profile 已删 = NotFound）降级 legacy 快照列继续可用 + warn 披露；
        // 行在但凭据损坏（Corrupted：槽位缺失/JSON 坏）是数据故障——静默换旧
        // Key 照跑会掩盖问题 + 归因记错主档，诚实上抛让发送失败可见。
        let (api_key, base_url) = if let Some(pid) = agent
            .model_profile_id
            .as_deref()
            .filter(|p| !p.trim().is_empty())
        {
            match super::model_profile_cmd::resolve_profile_credentials(&self.app, &self.pool, pid)
                .await
            {
                Ok(cred) => {
                    let _ = repo::agent::update_model_snapshot(
                        &self.pool,
                        &agent.id,
                        &cred.profile.provider,
                        &cred.profile.model,
                        cred.base_url.as_deref(),
                    )
                    .await;
                    agent.provider = cred.profile.provider.clone();
                    agent.model = cred.profile.model.clone();
                    (cred.api_key, cred.base_url)
                }
                Err(super::model_profile_cmd::ResolveProfileError::NotFound { .. }) => {
                    tracing::warn!(
                        target: "ice_paw.agent",
                        "agent {agent_id} 引用的模型配置 {pid} 已删除，降级用行内快照继续"
                    );
                    self.legacy_credentials(&agent).await?
                }
                Err(e) => {
                    tracing::error!(
                        target: "ice_paw.agent",
                        "agent {agent_id} 引用的模型配置 {pid} 凭据损坏，拒绝降级: {e}"
                    );
                    return Err(e.into());
                }
            }
        } else {
            self.legacy_credentials(&agent).await?
        };
        Ok(AgentWithCredentials {
            agent,
            api_key,
            base_url,
            hooks,
            word_style_profile,
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

        // 模型身份三路（Phase 3，2026-09-08 拍板——数据从出生就是「实体+引用」）：
        // a) 引用直传（Phase 2 既有）：解析 profile 写快照列——provider/model/
        //    base_url 一次写对；profile 不存在在此拦下（槽位/行都还没写，无残局可清）；
        // b) 手动字段自动物化（新建表单）：保存即转 ModelProfile 实体 + 引用——
        //    匹配键与 boot 存量抽离同源（profile_match），同配置（厂商+模型+端点+
        //    Key）复用既有实体不重复建；Key 只落 profile 槽位（agent 槽位不写：
        //    重复密文 + rotate 后陈旧副本两头害，`agent_dto` 的 has_api_key 真相
        //    也跟着 profile 走）；
        // c) 凭据不齐的旁路（如提案 CreateAgent 缺 Key / custom 缺端点）：不硬造
        //    keyless 实体，legacy 行照常可用，编辑时选实体即转正。
        let mut resolved_profile: Option<String> = None;
        let mut profile_snapshot: Option<(String, String, Option<String>)> = None;
        match input
            .model_profile_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(pid) => {
                let pid = pid.to_string();
                let cred = super::model_profile_cmd::resolve_profile_credentials(
                    &self.app, &self.pool, &pid,
                )
                .await
                .map_err(|e| AppError::Validation(format!("引用的模型配置不可用（{pid}）：{e}")))?;
                profile_snapshot = Some((cred.profile.provider, cred.profile.model, cred.base_url));
                resolved_profile = Some(pid);
            }
            None if manual_materializable(&input) => {
                let m = crate::harness::profile_materialize::materialize_from_manual(
                    &self.pool,
                    &input.provider,
                    &input.model,
                    &input.api_key,
                    input.base_url.as_deref(),
                    |slot| crypto::fetch_api_key(&self.app, slot).ok(),
                    |slot, key, url| crypto::store_api_key(&self.app, slot, key, url),
                    |slot| crypto::delete_api_key(&self.app, slot),
                )
                .await?;
                // 解析回读拿规范快照值（刚建的行，同时校验落库成功）
                let cred = super::model_profile_cmd::resolve_profile_credentials(
                    &self.app,
                    &self.pool,
                    &m.profile_id,
                )
                .await?;
                profile_snapshot = Some((cred.profile.provider, cred.profile.model, cred.base_url));
                resolved_profile = Some(m.profile_id.clone());
                tracing::info!(
                    target: "ice_paw.agent",
                    profile = %m.profile_id,
                    created = m.created,
                    "新建 agent 的手动模型配置已物化为模型配置实体（同配置复用既有实体）"
                );
            }
            None => {}
        }

        // Key 槽位：legacy 路径照旧落 agent 槽位（含空串占位）；引用/物化路径
        // Key 只在 profile 槽位（见上）
        if resolved_profile.is_none() {
            crypto::store_api_key(&self.app, &id, &input.api_key, input.base_url.as_deref())?;
        }

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
        if let Some(pid) = resolved_profile {
            new_agent.model_profile_id = Some(pid);
        }
        if let Some((provider, model, base_url)) = profile_snapshot {
            new_agent.provider = provider;
            new_agent.model = model;
            new_agent.base_url = base_url;
        }

        let row: AgentRow = repo::agent::create(&self.pool, &new_agent, &id, &id).await?;

        // 为新 Agent 确保 KB 行存在（无需重启）
        let default_ws = repo::preferences::get_all(&self.pool)
            .await
            .ok()
            .and_then(|p| p.default_workspace_path);
        ensure::ensure_agent_kb(
            Some(&self.app),
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

        Ok(self.agent_dto(row).await)
    }

    async fn update(&self, input: AgentUpdate) -> AppResult<Agent> {
        // 记录旧行，用于检测 workspace / provider 变更（watcher 重绑定 + 端点跟随）。
        let old_row = repo::agent::get_by_id(&self.pool, &input.id).await.ok();
        let old_has_primary = old_row
            .as_ref()
            .and_then(|r| r.model_profile_id.as_deref())
            .is_some_and(|v| !v.trim().is_empty());
        validate_update_model_fields_conflict(&input, old_has_primary)?;
        let old_workspace = old_row.as_ref().and_then(|r| r.workspace_path.clone());
        // B（2026-08-26 生产反馈根治）：换厂商而未显式提供 base_url → 重置为新
        // 厂商注册表默认地址。否则 DB 残留旧厂商 URL，新 provider 客户端会打到
        // 旧厂商端点 + 旧 vault key，报出来的是**对端厂商**的错误（如旧 key 余额
        // 用完），完全误导排障。端点跟随厂商；custom/未知无默认则保持不改（前端
        // 本就要求 custom 显式填 URL）。
        let provider_changed = input
            .provider
            .as_deref()
            .zip(old_row.as_ref().map(|r| r.provider.as_str()))
            .is_some_and(|(new, old)| new != old);
        let switch_url =
            default_url_on_provider_switch(provider_changed, input.provider.as_deref());
        let base_url_arg = resolve_base_url_arg(
            input.base_url.as_ref().map(|o| o.as_deref()),
            switch_url.as_deref(),
        );
        // ModelProfile Phase 2：设引用时解析 profile——存在性在此拦 + 快照即时
        // 写对（前端显示与 yaml 镜像都拿到解析后的值）。冲突校验已保证此时
        // provider/model/base_url 不在同批，快照覆盖不会吞用户输入。
        let profile_snapshot = match input.model_profile_id.as_ref() {
            Some(Some(pid)) if !pid.trim().is_empty() => {
                let cred = super::model_profile_cmd::resolve_profile_credentials(
                    &self.app, &self.pool, pid,
                )
                .await
                .map_err(|e| AppError::Validation(format!("引用的模型配置不可用（{pid}）：{e}")))?;
                Some((cred.profile.provider, cred.profile.model, cred.base_url))
            }
            _ => None,
        };
        let row = repo::agent::update(
            &self.pool,
            &input.id,
            &repo::agent::AgentRepoUpdate {
                name: input.name.clone(),
                provider: input.provider.clone(),
                model: input.model.clone(),
                system_prompt: input.system_prompt.clone(),
                base_url: base_url_arg.map(|o| o.map(String::from)),
                temperature: input.temperature,
                max_tokens: input.max_tokens,
                extra_params: input.extra_params.clone(),
                sort_order: input.sort_order,
                cache_prompt: input.cache_prompt,
                max_history_messages: input.max_history_messages,
                context_window: input.context_window,
                enabled_tools: input.enabled_tools.clone(),
                supports_vision: input.supports_vision,
                workspace_path: input.workspace_path.clone(),
                avatar: input.avatar.clone(),
                model_profile_id: input.model_profile_id.clone(),
                fallback_profile_ids: input.fallback_profile_ids.clone(),
            },
        )
        .await?;
        // 引用快照回写：repo update 落引用列后紧跟写快照三列（值变才写），再取
        // 新行让下游（yaml 镜像 / DTO 返回）都拿到解析后的模型身份
        let row = if let Some((provider, model, base_url)) = profile_snapshot {
            let _ = repo::agent::update_model_snapshot(
                &self.pool,
                &input.id,
                &provider,
                &model,
                base_url.as_deref(),
            )
            .await;
            repo::agent::get_by_id(&self.pool, &input.id).await?
        } else {
            row
        };
        if provider_changed && switch_url.is_some() && input.base_url.is_none() {
            tracing::info!(
                target: "ice_paw.agent",
                "agent {} 换厂商（{} → {}）且未显式提供地址：base_url 已重置为注册表默认 {}",
                row.id,
                old_row.as_ref().map(|r| r.provider.as_str()).unwrap_or("?"),
                row.provider,
                row.base_url.as_deref().unwrap_or("")
            );
        }

        // A（2026-08-26 生产反馈根治）：同步 agent.yaml 的 provider/model/base_url
        // 镜像行——它们是创建时写入的信息性镜像（运行时不读），不更新会与 UI 分裂
        // 误导排障。文件不存在不创建；失败 best-effort warn（DB 已更新，不回滚）。
        if let Some(ws) = row.workspace_path.as_deref() {
            if let Err(e) = super::agent_yaml::sync_agent_yaml_mirror_file(
                ws,
                &row.provider,
                &row.model,
                row.base_url.as_deref(),
            ) {
                tracing::warn!(
                    target: "ice_paw.agent",
                    "agent.yaml 镜像同步失败（DB 已更新，文件保持原样）: {e}"
                );
            }
        }

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

        Ok(self.agent_dto(row).await)
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
#[cfg(test)]
pub struct MockAgentCmd {
    inner: std::sync::Mutex<MockAgentCmdInner>,
}

#[cfg(test)]
struct MockAgentCmdInner {
    /// agent_id → (AgentRow, api_key, base_url)
    agents: std::collections::HashMap<String, (AgentRow, String, Option<String>)>,
    /// 调用历史（用于断言「list 被调用了」「create 被调用了」之类）
    call_log: Vec<String>,
}

#[cfg(test)]
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

#[cfg(test)]
impl Default for MockAgentCmd {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
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
            model_profile_id: None,
            fallback_profile_ids: None,
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
            model_profile_id: None,
            fallback_profile_ids: None,
        }
    }

    #[test]
    fn switch_base_url_follows_provider() {
        // 未换厂商：不注入默认地址（保持「不改」语义）
        assert_eq!(default_url_on_provider_switch(false, Some("glm")), None);
        // 换厂商：注册表默认地址（端点跟随厂商，防 DB 残留旧厂商 URL）
        assert_eq!(
            default_url_on_provider_switch(true, Some("glm")).as_deref(),
            Some("https://open.bigmodel.cn/api/paas/v4")
        );
        assert_eq!(
            default_url_on_provider_switch(true, Some("deepseek")).as_deref(),
            Some("https://api.deepseek.com")
        );
        // custom / 未知 provider 无默认（空串）→ None（保持不改）
        assert_eq!(default_url_on_provider_switch(true, Some("custom")), None);
        assert_eq!(default_url_on_provider_switch(true, Some("nope")), None);
        assert_eq!(default_url_on_provider_switch(true, None), None);
    }

    #[test]
    fn base_url_absent_keeps_endpoint() {
        // absent + switch_url=None（未换厂商，或换入 custom 无默认）→ None（保持
        // 不改；回归：曾一律映射成 Some(None) 静默清空端点）
        assert_eq!(resolve_base_url_arg(None, None), None);
        // absent + 换厂商有默认 → Some(Some(默认))
        assert_eq!(
            resolve_base_url_arg(None, Some("https://api.deepseek.com")),
            Some(Some("https://api.deepseek.com"))
        );
        // 显式设值 → 照设
        assert_eq!(
            resolve_base_url_arg(Some(Some("https://x.example")), None),
            Some(Some("https://x.example"))
        );
        // 显式清空 → 照清（合法显式意图）
        assert_eq!(resolve_base_url_arg(Some(None), None), Some(None));
    }

    #[test]
    fn agent_update_base_url_double_option_serde() {
        // 双层 Option 的 JSON 三形态（2026-09-07 补齐：base_url 曾缺
        // deserialize_double_option，null 与缺席同为「不改」，显式清空不可达——
        // resolve_base_url_arg 的 Some(None)=清空语义靠它才可达）：
        // 字段缺席 → None（不改）
        let absent: AgentUpdate = serde_json::from_str(r#"{"id":"a1"}"#).unwrap();
        assert_eq!(absent.base_url, None);
        // JSON null → Some(None)（清空）
        let nulled: AgentUpdate = serde_json::from_str(r#"{"id":"a1","base_url":null}"#).unwrap();
        assert_eq!(nulled.base_url, Some(None));
        // 值 → Some(Some(v))（设定）
        let valued: AgentUpdate =
            serde_json::from_str(r#"{"id":"a1","base_url":"https://api.deepseek.com"}"#).unwrap();
        assert_eq!(
            valued.base_url,
            Some(Some("https://api.deepseek.com".to_string()))
        );
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

    /// 物化闸：UI 完整输入可物化；旁路形态（需 Key 无 Key / custom 缺端点 /
    /// 字段不齐）保持 legacy 不硬造 keyless 实体
    #[test]
    fn manual_materializable_gate() {
        // UI 路径：完整手动字段（keyed 厂商带 Key）
        assert!(manual_materializable(&new_agent("glm", "sk-1")));
        // 需 Key 厂商无 Key（提案旁路）→ 不物化
        assert!(!manual_materializable(&new_agent("glm", "  ")));
        // custom 缺端点 → 不物化；补端点 → 可
        let mut c = new_agent("custom", "");
        c.base_url = None;
        assert!(!manual_materializable(&c));
        c.base_url = Some("http://localhost:11434/v1".into());
        assert!(manual_materializable(&c));
        // ollama 免 Key 且有注册表默认端点 → 可
        assert!(manual_materializable(&new_agent("ollama", "")));
        // 字段不齐 → 不物化
        let mut p = new_agent("glm", "sk-1");
        p.provider = " ".into();
        assert!(!manual_materializable(&p));
        let mut m = new_agent("glm", "sk-1");
        m.model = "".into();
        assert!(!manual_materializable(&m));
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
            model_profile_id: None,
            fallback_profile_ids: None,
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
            model_profile_id: None,
            fallback_profile_ids: None,
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

    /// ModelProfile Phase 2：AgentUpdate.model_profile_id / fallback_profile_ids
    /// 双层 Option JSON 三形态（base_url 惯例）。
    #[test]
    fn agent_update_model_profile_serde_three_forms() {
        // 字段缺席 → None（不改）
        let absent: AgentUpdate = serde_json::from_str(r#"{"id":"a1"}"#).unwrap();
        assert_eq!(absent.model_profile_id, None);
        assert_eq!(absent.fallback_profile_ids, None);

        // JSON null → Some(None)（解除引用 / 清链）
        let nulled: AgentUpdate = serde_json::from_str(
            r#"{"id":"a1","model_profile_id":null,"fallback_profile_ids":null}"#,
        )
        .unwrap();
        assert_eq!(nulled.model_profile_id, Some(None));
        assert_eq!(nulled.fallback_profile_ids, Some(None));

        // 值 → Some(Some(v))（设引用 / 设链）
        let valued: AgentUpdate = serde_json::from_str(
            r#"{"id":"a1","model_profile_id":"mp-1","fallback_profile_ids":["mp-2"]}"#,
        )
        .unwrap();
        assert_eq!(valued.model_profile_id, Some(Some("mp-1".into())));
        assert_eq!(
            valued.fallback_profile_ids,
            Some(Some(vec!["mp-2".to_string()]))
        );
    }

    /// 设引用与快照列族手填同批拒；解除引用与手填同批合法；链依赖主档。
    #[test]
    fn update_conflicts_profile_and_manual_fields_rejected() {
        let base = || AgentUpdate {
            id: "a1".into(),
            name: None,
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
            model_profile_id: None,
            fallback_profile_ids: None,
        };

        // 设引用 + provider 同批 → 拒
        let mut u = base();
        u.model_profile_id = Some(Some("mp-1".into()));
        u.provider = Some("glm".into());
        assert!(validate_update_model_fields_conflict(&u, false).is_err());

        // 设引用 + base_url 同批 → 拒（快照列族完整性）
        let mut u = base();
        u.model_profile_id = Some(Some("mp-1".into()));
        u.base_url = Some(Some("https://x.example".into()));
        assert!(validate_update_model_fields_conflict(&u, false).is_err());

        // 解除引用 + provider 同批 → 过（切回手动模式一并完成）
        let mut u = base();
        u.model_profile_id = Some(None);
        u.provider = Some("glm".into());
        u.model = Some("glm-5.3".into());
        assert!(validate_update_model_fields_conflict(&u, true).is_ok());

        // 引用态行 + 引用字段缺席（不改）+ 手填 model → 拒（提案卡批准
        // update_agent 直传快照族的形状——下轮解析覆盖回实体值，改了白改）
        let mut u = base();
        u.model = Some("glm-5.3".into());
        assert!(validate_update_model_fields_conflict(&u, true).is_err());

        // 同上但显式解除引用（Some(None)）→ 过（解除并改模型一次完成）
        let mut u = base();
        u.model_profile_id = Some(None);
        u.model = Some("glm-5.3".into());
        assert!(validate_update_model_fields_conflict(&u, true).is_ok());

        // legacy 行（无主档）手填 → 过（现状不变，非引用态无覆盖问题）
        let mut u = base();
        u.model = Some("glm-5.3".into());
        assert!(validate_update_model_fields_conflict(&u, false).is_ok());

        // legacy 行（无主档）设非空链 → 拒
        let mut u = base();
        u.fallback_profile_ids = Some(Some(vec!["mp-2".into()]));
        assert!(validate_update_model_fields_conflict(&u, false).is_err());

        // 同批设主档 + 链 → 过
        let mut u = base();
        u.model_profile_id = Some(Some("mp-1".into()));
        u.fallback_profile_ids = Some(Some(vec!["mp-2".into()]));
        assert!(validate_update_model_fields_conflict(&u, false).is_ok());

        // 已有主档的行只设链 → 过（沿用旧行主档）
        let mut u = base();
        u.fallback_profile_ids = Some(Some(vec!["mp-2".into()]));
        assert!(validate_update_model_fields_conflict(&u, true).is_ok());

        // 解除引用 + 留非空链 → 拒（目标态无主档）
        let mut u = base();
        u.model_profile_id = Some(None);
        u.fallback_profile_ids = Some(Some(vec!["mp-2".into()]));
        assert!(validate_update_model_fields_conflict(&u, true).is_err());

        // 清链 + 解除引用 → 过
        let mut u = base();
        u.model_profile_id = Some(None);
        u.fallback_profile_ids = Some(None);
        assert!(validate_update_model_fields_conflict(&u, true).is_ok());
    }

    /// 引用模式新建：跳过 provider/model/api_key 必填；链依赖主档。
    #[test]
    fn validate_new_agent_profile_mode_skips_manual_required() {
        let mut a = new_agent("", "");
        a.provider = String::new();
        a.model = String::new();
        a.api_key = String::new();
        // legacy 路径：空 provider 应拒
        assert!(validate_new_agent(&a).is_err());

        // 引用模式：同一入参全空合法（模型身份来自 profile）
        a.model_profile_id = Some("mp-1".into());
        assert!(validate_new_agent(&a).is_ok());

        // 引用模式但链无主档 → 拒
        let mut b = new_agent("", "");
        b.model_profile_id = None;
        b.fallback_profile_ids = Some(vec!["mp-2".into()]);
        assert!(validate_new_agent(&b).is_err());
    }

    /// Mock 双层 Option 语义：create 落引用列 / update 设与清。
    #[tokio::test]
    async fn mock_create_update_model_profile_fields() {
        let mock = MockAgentCmd::new();
        let mut new = new_agent("anthropic", "sk-xxx");
        new.model_profile_id = Some("mp-1".into());
        new.fallback_profile_ids = Some(vec!["mp-2".to_string(), "mp-3".to_string()]);
        let a = mock.create(new).await.unwrap();
        assert_eq!(a.model_profile_id.as_deref(), Some("mp-1"));
        assert_eq!(
            a.fallback_profile_ids,
            Some(vec!["mp-2".to_string(), "mp-3".to_string()])
        );

        // update：解除引用 + 清链
        let u = AgentUpdate {
            id: "test-agent".into(),
            model_profile_id: Some(None),
            fallback_profile_ids: Some(None),
            ..serde_json::from_str::<AgentUpdate>(r#"{"id":"test-agent"}"#).unwrap()
        };
        let a = mock.update(u).await.unwrap();
        assert_eq!(a.model_profile_id, None);
        assert_eq!(a.fallback_profile_ids, None);
    }
}
