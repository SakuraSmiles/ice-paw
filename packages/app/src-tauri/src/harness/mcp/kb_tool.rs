//! `search_kb` 工具 —— RAG 知识库检索（agentic，v1）
//!
//! 实现 `McpClient`，**override `execute_with_context`**：从 `ctx` 取 `agent_id`，
//! 合并查「agent 专有 + global」两级 KB，对 `kb_document` 做关键词匹配，
//! 返回命中文档列表（title/summary/file_path），供 agent 用 `read_file` 读全文。
//!
//! - `authorization_level = Always`：检索索引本身不需授权；读全文走 `read_file`
//!   现有的路径授权链路
//! - v1 不查 project KB（字段预留）
//! - scope 隔离：只查「当前 agent 的 KB + global」，**绝不**查其他 agent 的 KB

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::db::repo;
use crate::error::{AppError, AppResult};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

/// `search_kb` 工具：在当前 agent 的知识库（agent 专有 + 全局）里按关键词检索文档。
pub struct SearchKbTool;

#[derive(Deserialize)]
struct SearchKbArgs {
    query: String,
    #[serde(default = "default_limit")]
    limit: i64,
}

/// 默认返回条数。关键词检索的典型 top-N，过多会让 LLM 上下文膨胀。
fn default_limit() -> i64 {
    5
}

#[async_trait]
impl McpClient for SearchKbTool {
    fn name(&self) -> &str {
        "search_kb"
    }

    fn description(&self) -> &str {
        "Search the user's knowledge base for relevant documents by keyword. \
         Covers the current agent's dedicated knowledge base plus the global one. \
         Use this proactively when the user asks about their own notes, documents, \
         or domain-specific materials, or when such knowledge may be needed to answer. \
         Returns matching documents with title, summary, and file_path — then call \
         read_file on file_path to read the full content."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query — keywords or topic to look up in the knowledge base."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max number of results to return (default: 5).",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        // search_kb 必须带 agent_id 上下文，走 execute_with_context；
        // dispatch 已统一调 execute_with_context，这里只是 trait 兜底。
        Err(AppError::Internal(
            "search_kb 必须通过 execute_with_context 调用（需要 agent_id 上下文）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: SearchKbArgs = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("search_kb 参数解析失败: {e}"))
        })?;

        // 1. 确定检索范围：agent 专有 + global（v1 不查 project）
        let mut kb_ids: Vec<String> = Vec::new();
        let agent_kbs = repo::kb::list_by_scope(&ctx.pool, "agent", Some(&ctx.agent_id)).await?;
        let global_kbs = repo::kb::list_by_scope(&ctx.pool, "global", None).await?;
        kb_ids.extend(agent_kbs.into_iter().map(|k| k.id));
        kb_ids.extend(global_kbs.into_iter().map(|k| k.id));

        // 2. 无可用 KB 时直接返回空（避免下游 search 用空 kb_ids 报错/无意义查询）
        if kb_ids.is_empty() {
            return Ok(serde_json::json!({
                "query": parsed.query,
                "count": 0,
                "results": [],
                "note": "当前无可用的知识库（agent 专有 / global 均为空）"
            })
            .to_string());
        }

        // 3. 关键词检索（repo 已做 title 权重排序 + limit）
        let hits = repo::kb::search(&ctx.pool, &parsed.query, &kb_ids, parsed.limit).await?;

        // 4. 组装返回 JSON
        let results: Vec<SearchHitOut> = hits
            .into_iter()
            .map(|h| SearchHitOut {
                kb_name: h.kb_name,
                file_path: h.file_path,
                title: h.title,
                summary: h.summary,
            })
            .collect();

        let count = results.len();
        Ok(serde_json::json!({
            "query": parsed.query,
            "count": count,
            "results": results,
        })
        .to_string())
    }
}

/// 返回给 LLM 的单条命中（精简字段，全文让 agent 用 read_file 取）。
#[derive(Serialize)]
struct SearchHitOut {
    kb_name: String,
    file_path: String,
    title: String,
    summary: String,
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::NewKb;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;
    use std::str::FromStr;

    /// 建内存库 + 跑全部迁移（含 27_kb.sql）。
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

    async fn seed_kb(pool: &SqlitePool, id: &str, scope: &str, owner: Option<&str>) {
        repo::kb::create(
            pool,
            &NewKb {
                id: id.into(),
                name: id.into(),
                scope: scope.into(),
                owner_id: owner.map(String::from),
                directory: format!("/tmp/{id}"),
                enabled: true,
            },
        )
        .await
        .unwrap();
    }

    #[test]
    fn search_kb_schema_and_metadata() {
        let tool = SearchKbTool;
        assert_eq!(tool.name(), "search_kb");
        assert_eq!(tool.authorization_level(), AuthorizationLevel::Always);
        let p = tool.parameters();
        assert_eq!(p["type"], "object");
        assert_eq!(p["required"][0], "query");
        assert!(p["properties"]["query"].is_object());
    }

    /// 核心：scope 隔离 —— agent 只能命中「自己的 KB + global」，
    /// 绝不能查到其他 agent 的 KB（RAG 安全边界）。
    #[tokio::test]
    async fn search_kb_isolates_agent_scope() {
        let pool = fresh_pool().await;
        seed_kb(&pool, "kb-global", "global", None).await;
        seed_kb(&pool, "kb-a", "agent", Some("agent-a")).await;
        seed_kb(&pool, "kb-b", "agent", Some("agent-b")).await;

        repo::kb::upsert_document(
            &pool, "kb-global", "global.md", "Global Rust 笔记", "rust 语言基础", "[]",
            Some("h1"), None,
        )
        .await
        .unwrap();
        repo::kb::upsert_document(
            &pool, "kb-a", "a.md", "AgentA Rust 私货", "rust 进阶", "[]",
            Some("h2"), None,
        )
        .await
        .unwrap();
        repo::kb::upsert_document(
            &pool, "kb-b", "b.md", "AgentB 机密", "rust 机密内容", "[]",
            Some("h3"), None,
        )
        .await
        .unwrap();

        let tool = SearchKbTool;
        let ctx = ToolContext {
            conv_id: "c1".into(),
            agent_id: "agent-a".into(),
            project_id: None,
            pool: pool.clone(),
        };

        let result = tool
            .execute_with_context(r#"{"query":"rust"}"#, &ctx)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["count"], 2, "应命中 global + agent-a 共 2 条");

        let paths: Vec<&str> = v["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["file_path"].as_str().unwrap())
            .collect();
        assert!(paths.contains(&"global.md"));
        assert!(paths.contains(&"a.md"));
        assert!(
            !paths.contains(&"b.md"),
            "绝对不应命中其他 agent 的 KB（scope 隔离）"
        );
    }

    /// 无可用 KB（agent 无专有 KB + 无 global）→ 返回空结果而非报错。
    #[tokio::test]
    async fn search_kb_empty_when_no_kb() {
        let pool = fresh_pool().await;
        let tool = SearchKbTool;
        let ctx = ToolContext {
            conv_id: "c1".into(),
            agent_id: "lonely-agent".into(),
            project_id: None,
            pool: pool.clone(),
        };
        let result = tool
            .execute_with_context(r#"{"query":"anything"}"#, &ctx)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["count"], 0);
        assert!(v["results"].as_array().unwrap().is_empty());
    }
}
