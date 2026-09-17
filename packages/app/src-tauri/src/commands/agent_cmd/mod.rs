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
//! ## 模块布局（U3-5 ③ 拆分）
//!
//! 原 `agent_cmd.rs` 是 1860 行的 God module（trait+SQL+DTO+yaml 镜像+频道级联
//! 同居），拆成六个关注面：
//!
//! - [`domain`]：`AgentCmd` trait + `AgentWithCredentials` DTO（抽象契约，业务层只依赖它）
//! - [`validation`]：入参校验 + 换厂商端点跟随纯函数（无 IO，单测友好）
//! - [`default_yaml`]：出生 agent.yaml 内容构造 + 落盘
//! - [`sql`]：生产实现 `SqlAgentCmd`（sqlx + stronghold + 频道级联删除守卫）
//! - [`mock`]：测试实现 `MockAgentCmd`（内存状态，`#[cfg(test)]`）
//! - `tests`：单测（`#[cfg(test)]`）
//!
//! 本文件只保留子模块声明 + 对外 re-export + 5 个 Tauri command 包装。
//!
//! ## 注入方式
//!
//! - 生产路径：在 `lib.rs::setup` 里 `app.manage(Arc::new(SqlAgentCmd::new()))`
//! - 测试路径：直接在测试代码里 `let mock = Arc::new(MockAgentCmd::new()); ...`

mod default_yaml;
mod domain;
mod sql;
mod validation;
#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub use self::domain::{AgentCmd, AgentWithCredentials};
pub use self::sql::SqlAgentCmd;
#[cfg(test)]
pub use self::mock::MockAgentCmd;

use std::sync::Arc;

use tauri::State;

use crate::db::models::{Agent, AgentUpdate, NewAgent, RotateAgentKey};
use crate::error::AppResult;

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
