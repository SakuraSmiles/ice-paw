//! SqlAgentCmd —— 生产实现（sqlx + stronghold）。
//!
//! 从原 `agent_cmd.rs` God module 拆出（U3-5 ③）：持有生产路径所需的全部 IO
//! 依赖（sqlx / stronghold / KB watcher / ModelProfile 引用解析 / 频道级联删除
//! 守卫）。抽象契约见 [`super::domain::AgentCmd`]，本文件只实现它。

use std::sync::Arc;

use async_trait::async_trait;
use tauri::{AppHandle, Manager};

use sqlx::SqlitePool;

use crate::crypto;
use crate::db::models::{Agent, AgentRow, AgentUpdate, HookConfig, NewAgent, RotateAgentKey};
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::kb::{ensure, watcher_manager::KbWatcherManager};
use crate::harness::provider::provider_requires_key;

use super::default_yaml::write_default_agent_yaml;
use super::domain::{AgentCmd, AgentWithCredentials};
use super::validation::{
    default_url_on_provider_switch, manual_materializable, resolve_base_url_arg,
    validate_new_agent, validate_update_model_fields_conflict,
};

use crate::commands::agent_yaml::sync_agent_yaml_mirror_file;
use crate::commands::model_profile_cmd::{resolve_profile_credentials, ResolveProfileError};
use crate::harness::profile_materialize::materialize_from_manual;

/// 生产 AgentCmd 实现
///
/// 持有 `AppHandle`（用于 stronghold 访问）+ `SqlitePool`（用于 sqlx 查询）。
/// 通过 `new(app, pool)` 构造一次，作为 `Arc<dyn AgentCmd>` 注入 Tauri State。
pub struct SqlAgentCmd {
    app: AppHandle,
    pool: SqlitePool,
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
            match resolve_profile_credentials(&self.app, &self.pool, pid).await {
                Ok(cred) => {
                    // 快照回写失败不阻塞对话（快照列仅显示用，下轮解析会再写），
                    // 但不再静默吞（U0-11）：warn 留排查线索
                    if let Err(e) = repo::agent::update_model_snapshot(
                        &self.pool,
                        &agent.id,
                        &cred.profile.provider,
                        &cred.profile.model,
                        cred.base_url.as_deref(),
                    )
                    .await
                    {
                        tracing::warn!(
                            target: "ice_paw.agent",
                            "agent {agent_id} 模型快照回写失败（仅显示用，不阻塞）: {e}"
                        );
                    }
                    agent.provider = cred.profile.provider.clone();
                    agent.model = cred.profile.model.clone();
                    (cred.api_key, cred.base_url)
                }
                Err(ResolveProfileError::NotFound { .. }) => {
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
                let cred = resolve_profile_credentials(&self.app, &self.pool, &pid)
                    .await
                    .map_err(|e| AppError::Validation(format!("引用的模型配置不可用（{pid}）：{e}")))?;
                profile_snapshot = Some((cred.profile.provider, cred.profile.model, cred.base_url));
                resolved_profile = Some(pid);
            }
            None if manual_materializable(&input) => {
                let m = materialize_from_manual(
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
                let cred = resolve_profile_credentials(
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
                let cred = resolve_profile_credentials(&self.app, &self.pool, pid)
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
                // tool_scopes 是旋钮：唯一写入通道 set_agent_tool_scopes，不进 AgentUpdate
                tool_scopes: None,
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
            // 快照回写失败不阻塞更新主流程（快照列仅显示用，下轮解析会再写），
            // 但不再静默吞（U0-11）：warn 留排查线索
            if let Err(e) = repo::agent::update_model_snapshot(
                &self.pool,
                &input.id,
                &provider,
                &model,
                base_url.as_deref(),
            )
            .await
            {
                tracing::warn!(
                    target: "ice_paw.agent",
                    "agent {} 模型快照回写失败（仅显示用，不阻塞）: {e}",
                    input.id
                );
            }
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
            if let Err(e) = sync_agent_yaml_mirror_file(
                ws,
                &row.provider,
                &row.model,
                row.base_url.as_deref(),
            )
            .await
            {
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
        // 频道 v1 删除守卫：conversations.agent_id 是 ON DELETE CASCADE——agent
        // 是活跃频道（kind='channel' 且未归档）的投影时，删除会连带删掉整个
        // 频道对话流。先迁移投影给「joined_at 最早的剩余成员」（选举平票同款
        // 基准序）+ role 置 coordinator（统筹者真相源），换帅事实记
        // channel_coordinator(failed-over) 事件；无剩余成员 → 拒删三段式。
        // 归档频道不在守卫面：项目已删无候选可迁，随 CASCADE 消失（边缘路径，
        // 边界披露见 CLAUDE.md 频道节）。
        let channels =
            repo::conversation::channels_coordinated_by(&self.pool, agent_id).await?;
        for ch in channels {
            let Some(pid) = ch.project_id.clone() else {
                continue; // 散落频道不该存在（ensure 只建挂项目的），防御性跳过
            };
            let remaining: Vec<repo::project::ProjectMemberProfile> =
                repo::project::list_member_profiles(&self.pool, &pid)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|m| m.agent_id != agent_id)
                    .collect();
            let Some(successor) = remaining.first() else {
                let proj_name = repo::project::get_by_id(&self.pool, &pid)
                    .await
                    .map(|p| p.name)
                    .unwrap_or_else(|_| pid.clone());
                return Err(AppError::Validation(format!(
                    "无法删除：该 agent 是频道「{}」的最后成员，删除会连带删除整个频道对话。\
                     请先为项目「{}」添加其他成员（频道将自动迁移给最早加入的成员），\
                     或先删除频道所在项目",
                    ch.title, proj_name
                )));
            };
            repo::project::set_member_role(&self.pool, &pid, &successor.agent_id, "coordinator")
                .await?;
            repo::conversation::set_conversation_agent(&self.pool, &ch.id, &successor.agent_id)
                .await?;
            crate::harness::event_log::log_channel_coordinator(
                &self.pool,
                &crate::harness::event_log::EventCtx::new(&ch.id, "", &successor.agent_id),
                &crate::harness::event_log::ChannelCoordinatorPayload {
                    v: 1,
                    action: "failed-over".into(),
                    agent_id: Some(successor.agent_id.clone()),
                    reason: Some(
                        "统筹者 agent 被删除，自动迁移给最早加入的剩余成员".to_string(),
                    ),
                },
            )
            .await;
            tracing::info!(
                target: "ice_paw.agent",
                "频道「{}」统筹者迁移（agent 删除守卫）: {agent_id} → {}",
                ch.title,
                successor.agent_id
            );
        }
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
