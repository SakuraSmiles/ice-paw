//! 原生能力发现工具四件（2026-09-29 用户实测反馈「原生功能可调用工具不足」）：
//! `list_projects` / `get_project_details` / `search_conversations` /
//! `get_task_ledger`——全部**只读 Always 级**（发现 ≠ 写权限，relay 的
//! `list_conversations` 同哲学）。给 agent 补齐「项目维度 / 会话内容检索 /
//! 任务台账」三个失明面：
//!
//! - 「我们有哪些项目」「XX 项目进展」→ list_projects / get_project_details
//!   （概览 + 成员 + 委派任务状态聚合）
//! - 「我们之前讨论过 X 吗」→ search_conversations（**全文**检索 messages
//!   内容——list_conversations 只有元信息零内容；LIKE 匹配 + 截断摘要 +
//!   上限 20 防爆）
//! - 「项目任务现在什么状态」→ get_task_ledger（MA-2 台账的 agent 侧出口）
//!
//! 注册边界：全局注册（register_builtin），不限会话 kind——发现类工具对
//! 委派子会话也安全（只读）。tool_scopes 归 config 组（平台发现族）。

use async_trait::async_trait;

use crate::db::repo;
use crate::error::{AppError, AppResult};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

/// 任务状态三档（与 project_cmd termination_bucket 同源口径）：
/// stop/end_turn = done；无终态 = running；其余（abort/doom/budget…）= failed。
fn task_status_of(t: &repo::project_ledger::ProjectTaskRow) -> &'static str {
    match &t.ended_payload {
        None => "running",
        Some(p) => {
            let ended: Option<crate::harness::event_log::TurnEndedPayload> =
                serde_json::from_str(p).ok();
            match ended.as_ref().map(|e| e.termination.as_str()) {
                Some("stop") | Some("end_turn") => "done",
                _ => "failed",
            }
        }
    }
}

// =========================================================================
// list_projects
// =========================================================================

pub struct ListProjectsTool;

#[async_trait]
impl McpClient for ListProjectsTool {
    fn name(&self) -> &str {
        "list_projects"
    }

    fn description(&self) -> &str {
        "List all projects with id, name, description, member agent ids, archived state and \\
         workspace path. Use it when the user asks 'what projects do we have', or before \\
         tools that need a project id (channels, task ledger, member addressing)."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(format!(
            "{} 必须通过 execute_with_context 调用",
            self.name()
        )))
    }

    async fn execute_with_context(&self, _args: &str, ctx: &ToolContext) -> AppResult<String> {
        let projects = repo::project::list(&ctx.pool).await?;
        let agents = repo::agent::list(&ctx.pool).await.unwrap_or_default();
        let name_of = |id: &str| {
            agents
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "（已删除）".into())
        };
        // 成员表按项目查（ProjectRow 不含 agents——那是前端投影拼装的）
        let mut items = Vec::with_capacity(projects.len());
        for p in &projects {
            let members = repo::project::list_agents(&ctx.pool, &p.id)
                .await
                .unwrap_or_default();
            items.push(serde_json::json!({
                "id": p.id,
                "name": p.name,
                "description": p.description,
                "archived": p.archived,
                "workspace_path": p.workspace_path,
                "members": members.iter().map(|a| serde_json::json!({
                    "agent_id": a.agent_id,
                    "agent_name": name_of(&a.agent_id),
                    "role": a.role,
                })).collect::<Vec<_>>(),
            }));
        }
        Ok(serde_json::json!({ "projects": items }).to_string())
    }
}

// =========================================================================
// get_project_details
// =========================================================================

pub struct GetProjectDetailsTool;

#[derive(serde::Deserialize)]
struct ProjectRefArgs {
    /// 项目 id 或名称唯一匹配（重名不猜——用 id 重试）
    project: String,
}

async fn resolve_project(
    pool: &sqlx::SqlitePool,
    target: &str,
) -> AppResult<crate::db::models::ProjectRow> {
    let list = repo::project::list(pool).await?;
    if let Some(p) = list.iter().find(|p| p.id == target) {
        return Ok(p.clone());
    }
    let mut by_name = list.iter().filter(|p| p.name == target);
    match (by_name.next(), by_name.next()) {
        (Some(p), None) => Ok(p.clone()),
        _ => Err(AppError::Validation(format!(
            "找不到项目 '{}' 或名称不唯一——请用 list_projects 查 id 后重试",
            target
        ))),
    }
}

#[async_trait]
impl McpClient for GetProjectDetailsTool {
    fn name(&self) -> &str {
        "get_project_details"
    }

    fn description(&self) -> &str {
        "Get one project's full details: description, members (with roles), workspace, and \\
         its delegated-task ledger (running / done / failed counts + recent task titles with \\
         last status). Use it for 'how is project X going' style questions. `project` accepts \\
         the id or its unique name."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": "Project id or unique name." }
            },
            "required": ["project"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(format!(
            "{} 必须通过 execute_with_context 调用",
            self.name()
        )))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: ProjectRefArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("get_project_details 参数解析失败: {e}")))?;
        let p = resolve_project(&ctx.pool, &parsed.project).await?;
        let members = repo::project::list_agents(&ctx.pool, &p.id)
            .await
            .unwrap_or_default();
        let agents = repo::agent::list(&ctx.pool).await.unwrap_or_default();
        let name_of = |id: &str| {
            agents
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "（已删除）".into())
        };
        let tasks = repo::project_ledger::list_project_tasks(&ctx.pool, &p.id).await?;
        let (running, done, failed) = tasks.iter().fold((0u32, 0u32, 0u32), |(r, d, f), t| {
            match task_status_of(t) {
                "running" => (r + 1, d, f),
                "done" => (r, d + 1, f),
                "failed" => (r, d, f + 1),
                _ => (r + 1, d, f), // 无终态 = 进行中
            }
        });
        let recent: Vec<serde_json::Value> = tasks
            .iter()
            .rev()
            .take(10)
            .map(|t| {
                serde_json::json!({
                    "title": t.title,
                    "agent": name_of(&t.agent_id),
                    "status": task_status_of(t),
                    "updated_at": t.updated_at,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "id": p.id,
            "name": p.name,
            "description": p.description,
            "archived": p.archived,
            "workspace_path": p.workspace_path,
            "members": members.iter().map(|a| serde_json::json!({
                "agent_id": a.agent_id,
                "agent_name": name_of(&a.agent_id),
                "role": a.role,
            })).collect::<Vec<_>>(),
            "task_summary": { "running": running, "done": done, "failed": failed, "total": tasks.len() },
            "recent_tasks": recent,
        })
        .to_string())
    }
}

// =========================================================================
// search_conversations（全文检索——messages 内容，不只标题）
// =========================================================================

pub struct SearchConversationsTool;

#[derive(serde::Deserialize)]
struct SearchArgs {
    query: String,
    /// 结果上限（默认 20，硬顶 50——防一次检索爆上下文）
    #[serde(default)]
    limit: Option<i64>,
}

#[async_trait]
impl McpClient for SearchConversationsTool {
    fn name(&self) -> &str {
        "search_conversations"
    }

    fn description(&self) -> &str {
        "Full-text search across conversation message CONTENT (not just titles) for a \\
         keyword or phrase. Use it for 'have we discussed X before', 'find that conversation \\
         where we decided Y'. Search scope: conversations in the CURRENT project (including \\
         the project channel) plus your own standalone conversations — other projects' and \\
         other agents' standalone conversations are NOT searchable. Returns matching \\
         conversation id/title/agent + a truncated excerpt around the match. Read-only; use \\
         send_message_to_session or @-references to act on what you find."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keyword or phrase to search for in message content." },
                "limit": { "type": "integer", "description": "Max results (default 20, cap 50)." }
            },
            "required": ["query"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(format!(
            "{} 必须通过 execute_with_context 调用",
            self.name()
        )))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: SearchArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("search_conversations 参数解析失败: {e}")))?;
        let q = parsed.query.trim();
        if q.is_empty() {
            return Err(AppError::Validation("检索词不能为空".into()));
        }
        let limit = parsed.limit.unwrap_or(20).clamp(1, 50);
        // LIKE 转义（%/_ 用户字面量）——共享 db::repo::escape_like（2026-10-08 起单一真相源）
        let pattern = format!("%{}", crate::db::repo::escape_like(q));
        // 2026-10-08 P1 边界收窄（用户拍板）：检索范围 = 同项目会话（含频道）+
        // 本 agent 自己的散落会话——与 MA-3 项目边界/委派沙箱的信息边界对齐，
        // 不再全库可检索。project_id 绑 None 时第一支恒不中（SQL NULL 比较），
        // 散落会话发起的检索自然只剩自己的。
        let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT c.id, COALESCE(c.title, ''), c.agent_id, m.content, m.created_at \
             FROM messages m JOIN conversations c ON c.id = m.conversation_id \
             WHERE m.content LIKE ? ESCAPE '\\' AND c.kind != 'delegation' \
               AND (c.project_id = ? OR (c.project_id IS NULL AND c.agent_id = ?)) \
             ORDER BY m.created_at DESC LIMIT ?",
        )
        .bind(&pattern)
        .bind(&ctx.project_id)
        .bind(&ctx.agent_id)
        .bind(limit)
        .fetch_all(&ctx.pool)
        .await
        .map_err(|e| AppError::Internal(format!("会话内容检索失败: {e}")))?;

        let agents = repo::agent::list(&ctx.pool).await.unwrap_or_default();
        let name_of = |id: &str| {
            agents
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "（已删除）".into())
        };
        let items: Vec<serde_json::Value> = rows
            .iter()
            .map(|(cid, title, agent_id, content, at)| {
                // 摘录：命中点前后各 60 字符（char 边界对齐——CJK 命中点跨
                // 字节边界时裸切片 panic，dispatch_catch_panic 兜底为工具错误
                // 但 doom 签名被污染；2026-10-08 顺手件）
                let excerpt = match content.find(q) {
                    Some(i) => {
                        let mut start = i.saturating_sub(60);
                        while start > 0 && !content.is_char_boundary(start) {
                            start -= 1;
                        }
                        let mut end = (i + q.len() + 60).min(content.len());
                        while end < content.len() && !content.is_char_boundary(end) {
                            end += 1;
                        }
                        let mut s = String::new();
                        if start > 0 {
                            s.push('…');
                        }
                        s.push_str(&content[start..end]);
                        if end < content.len() {
                            s.push('…');
                        }
                        s.chars().take(160).collect::<String>()
                    }
                    None => content.chars().take(120).collect::<String>(),
                };
                serde_json::json!({
                    "conversation_id": cid,
                    "title": if title.is_empty() { "（无标题）" } else { title },
                    "agent_name": name_of(agent_id),
                    "excerpt": excerpt,
                    "at": at,
                })
            })
            .collect();
        Ok(serde_json::json!({ "matches": items, "query": q }).to_string())
    }
}

// =========================================================================
// get_task_ledger
// =========================================================================

pub struct GetTaskLedgerTool;

#[async_trait]
impl McpClient for GetTaskLedgerTool {
    fn name(&self) -> &str {
        "get_task_ledger"
    }

    fn description(&self) -> &str {
        "Get a project's delegated-task ledger: every delegation child conversation with title, \\
         assigned agent, running/done/failed state (derived from last turn_ended) and update \\
         time. Use it for 'what's the status of tasks in project X'. Read-only."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": "Project id or unique name." }
            },
            "required": ["project"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(format!(
            "{} 必须通过 execute_with_context 调用",
            self.name()
        )))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: ProjectRefArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("get_task_ledger 参数解析失败: {e}")))?;
        let p = resolve_project(&ctx.pool, &parsed.project).await?;
        let tasks = repo::project_ledger::list_project_tasks(&ctx.pool, &p.id).await?;
        let agents = repo::agent::list(&ctx.pool).await.unwrap_or_default();
        let name_of = |id: &str| {
            agents
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "（已删除）".into())
        };
        let items: Vec<serde_json::Value> = tasks
            .iter()
            .map(|t| {
                serde_json::json!({
                    "conversation_id": t.id,
                    "title": t.title,
                    "agent": name_of(&t.agent_id),
                    "initiated_by": t.initiator_agent_id.as_deref().map(name_of).unwrap_or_else(|| "用户".into()),
                    "status": task_status_of(t),
                    "updated_at": t.updated_at,
                })
            })
            .collect();
        Ok(serde_json::json!({ "project": p.name, "tasks": items }).to_string())
    }
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::mcp::client::ToolContext;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        pool
    }

    async fn seed_agent(pool: &SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, api_key_ref) VALUES (?, ?, 'mock', 'm', ?)",
        )
        .bind(id)
        .bind(id)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_project(pool: &SqlitePool, id: &str) {
        sqlx::query("INSERT INTO projects (id, name) VALUES (?, ?)")
            .bind(id)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_conv(pool: &SqlitePool, id: &str, agent: &str, project: Option<&str>, kind: &str) {
        sqlx::query(
            "INSERT INTO conversations (id, title, agent_id, project_id, kind, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, '2026-01-01 00:00:00', '2026-01-01 00:00:00')",
        )
        .bind(id)
        .bind(id)
        .bind(agent)
        .bind(project)
        .bind(kind)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_msg(pool: &SqlitePool, id: &str, conv: &str, needle: &str) {
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, role, content, created_at) \
             VALUES (?, ?, 'user', ?, '2026-01-01 00:00:00')",
        )
        .bind(id)
        .bind(conv)
        .bind(needle)
        .execute(pool)
        .await
        .unwrap();
    }

    fn ctx(pool: &SqlitePool, agent: &str, project: Option<&str>) -> ToolContext {
        ToolContext {
            conv_id: "conv-self".into(),
            agent_id: agent.into(),
            project_id: project.map(str::to_string),
            workspace: None,
            pool: pool.clone(),
            api_key: None,
            app_handle: None,
            proposal_registry: None,
            turn_id: None,
            cancel: None,
            tool_use_id: None,
        }
    }

    async fn hit_ids(tool: &SearchConversationsTool, pool: &SqlitePool, agent: &str, project: Option<&str>) -> Vec<String> {
        let out = tool
            .execute_with_context(r#"{"query":"needle-xy"}"#, &ctx(pool, agent, project))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        v["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["conversation_id"].as_str().unwrap().to_string())
            .collect()
    }

    /// 2026-10-08 P1 边界收窄回归锁：检索范围 = 同项目会话（含他人 agent 的
    /// 项目会话）+ 本 agent 自己的散落会话；跨项目 / 他人散落 / 委派子会话不中。
    #[tokio::test]
    async fn search_scopes_to_project_and_own_standalone() {
        let pool = fresh_pool().await;
        seed_agent(&pool, "agent-1").await;
        seed_agent(&pool, "agent-2").await;
        seed_project(&pool, "projA").await;
        seed_project(&pool, "projB").await;
        seed_conv(&pool, "c-proj-a", "agent-1", Some("projA"), "chat").await;
        // 同项目、他人 agent 的会话——项目内共享上下文，应命中
        seed_conv(&pool, "c-proj-a2", "agent-2", Some("projA"), "chat").await;
        // 跨项目、自己的会话——不中
        seed_conv(&pool, "c-proj-b", "agent-1", Some("projB"), "chat").await;
        // 散落、自己的会话——应命中
        seed_conv(&pool, "c-loose-1", "agent-1", None, "chat").await;
        // 散落、他人的会话——不中
        seed_conv(&pool, "c-loose-2", "agent-2", None, "chat").await;
        // 委派子会话（即便同项目）——kind 过滤，不中
        seed_conv(&pool, "c-dele", "agent-2", Some("projA"), "delegation").await;
        for (i, c) in [
            "c-proj-a",
            "c-proj-a2",
            "c-proj-b",
            "c-loose-1",
            "c-loose-2",
            "c-dele",
        ]
        .into_iter()
        .enumerate()
        {
            seed_msg(&pool, &format!("m{i}"), c, "needle-xy").await;
        }

        let tool = SearchConversationsTool;
        // 项目语境：同项目两条 + 自己散落一条
        let mut ids = hit_ids(&tool, &pool, "agent-1", Some("projA")).await;
        ids.sort();
        assert_eq!(ids, vec!["c-loose-1", "c-proj-a", "c-proj-a2"]);

        // 散落语境（project_id=None）：只剩自己的散落会话
        let ids = hit_ids(&tool, &pool, "agent-1", None).await;
        assert_eq!(ids, vec!["c-loose-1"]);
    }

    /// LIKE 字面量回归（与 kb.rs B2 同族）：检索词含 %/_ 不当通配符
    #[tokio::test]
    async fn search_escapes_like_wildcards() {
        let pool = fresh_pool().await;
        seed_agent(&pool, "agent-1").await;
        seed_project(&pool, "projA").await;
        seed_conv(&pool, "c-1", "agent-1", Some("projA"), "chat").await;
        seed_msg(&pool, "m-1", "c-1", "记录 file_name_v2 与 100% 进度").await;
        seed_msg(&pool, "m-2", "c-1", "无关内容 fileXnameYvZ2").await;

        let tool = SearchConversationsTool;
        let out = tool
            .execute_with_context(r#"{"query":"file_name_v2"}"#, &ctx(&pool, "agent-1", Some("projA")))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        // `file_name_v2` 的下划线不被当通配符——精确命中，m-2 的 fileXnameYvZ2 不中
        assert_eq!(v["matches"].as_array().unwrap().len(), 1);
    }
}
