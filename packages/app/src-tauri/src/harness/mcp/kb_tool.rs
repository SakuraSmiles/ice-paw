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

use std::path::PathBuf;

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

        let mut results: Vec<SearchHitOut> = hits
            .into_iter()
            .map(|h| SearchHitOut {
                kb_name: h.kb_name,
                file_path: h.file_path,
                title: h.title,
                summary: h.summary,
            })
            .collect();

        // 3.5 语义检索（全局 embedding 配置，独立于聊天 Agent）
        if let Some(semantic) = try_semantic_search(
            &ctx.pool,
            &parsed.query,
            &kb_ids,
            parsed.limit as usize,
        )
        .await
        {
            // 合并去重（按 file_path 去重，关键词结果优先）
            let existing_paths: std::collections::HashSet<String> =
                results.iter().map(|r| r.file_path.clone()).collect();
            for s in semantic {
                if !existing_paths.contains(&s.file_path) {
                    results.push(s);
                }
            }
            results.truncate(parsed.limit as usize);
        }

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

/// 尝试语义检索（向量）。返回 None = 不支持/失败，调用方回退纯关键词。
///
/// embedding 预生成在 indexer 入库时已做（见 [`crate::harness::kb::indexer`]），
/// 这里只对预生成漏掉/失败的 chunk 懒生成兜底。配置解析 + 批量生成收敛在
/// [`crate::harness::kb::embedding`] 模块复用。
async fn try_semantic_search(
    pool: &sqlx::SqlitePool,
    query: &str,
    kb_ids: &[String],
    limit: usize,
) -> Option<Vec<SearchHitOut>> {
    use crate::harness::kb::embedding::{ensure_chunks_embedded, resolve_embedding_config};
    use crate::harness::provider::embedding::{top_k_recall, EmbeddingBackend, OpenAiEmbeddingBackend};
    use crate::db::repo::kb::{bytes_to_embedding, load_chunks_for_vector_search};

    // 1. 配置（必须走 get_all 反序列化，见 harness::kb::embedding 模块文档 / v2 阻断①）
    let prefs = repo::preferences::get_all(pool).await.ok()?;
    let (model, url, api_key) = match resolve_embedding_config(&prefs) {
        Some(cfg) => cfg,
        None => {
            tracing::debug!(
                target: "ice_paw.kb",
                "语义检索未启用（embedding provider/model/key 未齐全），回退关键词检索"
            );
            return None;
        }
    };
    let backend = OpenAiEmbeddingBackend::new(model, url);

    // 2. 一次加载所有 chunk（ensure 回填内存，省掉原先的第二次 load）
    let mut chunks = load_chunks_for_vector_search(pool, kb_ids).await.ok()?;

    // 3. 兜底：对缺向量的 chunk 懒生成 + 回填（预生成失败/漏掉的）
    if let Err(e) = ensure_chunks_embedded(pool, &mut chunks, &backend, &api_key).await {
        tracing::warn!(target: "ice_paw.kb", "懒生成 embedding 失败，仅用已有向量检索: {e}");
    }

    // 4. query 转向量
    let query_emb = backend.embed(vec![query], &api_key).await.ok()?;
    if query_emb.is_empty() {
        return None;
    }
    let query_vec = &query_emb[0];

    // 5. 构建候选（有 embedding 的）
    let candidates: Vec<(String, Vec<f32>)> = chunks
        .iter()
        .filter_map(|c| {
            c.embedding
                .as_ref()
                .map(|bytes| (c.id.clone(), bytes_to_embedding(bytes)))
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }

    // 6. top-K 语义检索
    let top_ids = top_k_recall(query_vec, &candidates);

    // 7. 映射回 SearchHitOut（按 file_path 去重）
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for id in &top_ids {
        if results.len() >= limit {
            break;
        }
        if let Some(chunk) = chunks.iter().find(|c| &c.id == id) {
            if seen.insert(chunk.file_path.clone()) {
                let summary: String = chunk.content.chars().take(200).collect();
                results.push(SearchHitOut {
                    kb_name: "语义检索".into(),
                    file_path: chunk.file_path.clone(),
                    title: chunk.title.clone(),
                    summary,
                });
            }
        }
    }

    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}

// =========================================================================
// save_to_kb —— 聊天入库（agent 写资料到 knowledge，watcher 自动索引）
// =========================================================================

/// `save_to_kb` 工具：把资料保存为 md 写入对应级别的 knowledge 目录。
///
/// agent 在对话中判断资料价值与归属级别，调用本工具写入对应 knowledge 目录；
/// 文件监听（watcher）会自动索引，之后即可用 `search_kb` 检索。
///
/// - `scope='global'`：写入 `<default_workspace>/knowledge`（全员共享）
/// - `scope='agent'`：写入当前 agent 的 `<workspace>/knowledge`（本 agent 专有）
pub struct SaveToKbTool;

#[derive(Deserialize)]
struct SaveToKbArgs {
    title: String,
    content: String,
    /// 'agent' | 'global'（v1 不支持 project）
    scope: String,
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// 可选文件名（不含路径与后缀）；默认 `note-{timestamp}`，避免冲突。
    #[serde(default)]
    filename: Option<String>,
}

#[async_trait]
impl McpClient for SaveToKbTool {
    fn name(&self) -> &str {
        "save_to_kb"
    }

    fn description(&self) -> &str {
        "Save a piece of knowledge/material to the knowledge base as a markdown file, \
         so it can be retrieved later via search_kb. Use this when the user shares \
         information worth remembering (notes, decisions, how-tos, reference material) \
         or explicitly asks to save/remember something. Pick scope='global' for \
         widely-shared knowledge, 'agent' for this agent's dedicated notes."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Document title (used in search & display)." },
                "content": { "type": "string", "description": "Full markdown content to save." },
                "scope": { "type": "string", "enum": ["agent", "global"], "description": "Which knowledge base: 'agent' (this agent's dedicated) or 'global' (shared)." },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional tags for categorization." },
                "filename": { "type": "string", "description": "Optional filename without path/extension. Defaults to note-{timestamp}." }
            },
            "required": ["title", "content", "scope"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "save_to_kb 必须通过 execute_with_context 调用（需要 agent_id 上下文）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: SaveToKbArgs =
            serde_json::from_str(args).map_err(|e| {
                AppError::Validation(format!("save_to_kb 参数解析失败: {e}"))
            })?;
        validate_save_scope(&parsed.scope)?;

        // 推导目标 knowledge 目录
        let directory = resolve_kb_directory(&ctx.pool, &parsed.scope, &ctx.agent_id).await?;

        // 文件名（默认 note-{timestamp}，保证不冲突）
        let stem = parsed.filename.clone().unwrap_or_else(|| {
            format!("note-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"))
        });
        let filename = format!("{stem}.md");
        let file_path = directory.join(&filename);

        // 组装 md：frontmatter（serde_yaml 序列化，避免手动转义 bug）+ 正文
        let md = build_markdown(&parsed.title, &parsed.tags, &parsed.content);

        // 建目录 + 写文件
        std::fs::create_dir_all(&directory).map_err(AppError::Io)?;
        std::fs::write(&file_path, md).map_err(AppError::Io)?;

        let dir_str = directory.to_string_lossy().replace('\\', "/");
        tracing::info!(target: "ice_paw.kb", "save_to_kb 写入: {}/{}", dir_str, filename);

        Ok(serde_json::json!({
            "scope": parsed.scope,
            "directory": dir_str,
            "file_path": filename,
            "message": "已保存到知识库，文件监听将自动索引；稍后可用 search_kb 检索。"
        })
        .to_string())
    }
}

/// save_to_kb 仅允许 agent / global（v1 不支持 project）。
fn validate_save_scope(scope: &str) -> AppResult<()> {
    match scope {
        "agent" | "global" => Ok(()),
        _ => Err(AppError::Validation(format!(
            "scope 必须是 agent / global，得到 '{scope}'"
        ))),
    }
}

/// 推导某 scope 的 knowledge 目录：
/// global = `<default_workspace>/knowledge`；agent = 当前 agent 的 `<workspace>/knowledge`。
async fn resolve_kb_directory(
    pool: &sqlx::SqlitePool,
    scope: &str,
    agent_id: &str,
) -> AppResult<PathBuf> {
    use crate::harness::kb::ensure::{agent_workspace_root, knowledge_dir};
    let prefs = repo::preferences::get_all(pool).await?;
    match scope {
        "global" => prefs
            .default_workspace_path
            .as_deref()
            .map(knowledge_dir)
            .ok_or_else(|| AppError::Internal("未配置 default_workspace_path".into())),
        "agent" => {
            let agent = repo::agent::get_by_id(pool, agent_id).await?;
            let root = agent_workspace_root(
                agent.workspace_path.as_deref(),
                prefs.default_workspace_path.as_deref(),
                agent_id,
            )
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "agent {agent_id} 无 workspace_path 且无 default_workspace_path"
                ))
            })?;
            Ok(knowledge_dir(&root))
        }
        _ => Err(AppError::Validation(format!(
            "scope 必须是 agent / global，得到 '{scope}'"
        ))),
    }
}

/// 组装 markdown：frontmatter（title + 可选 tags）+ 正文。
/// frontmatter 用 serde_yaml 序列化，保证特殊字符（冒号/引号/井号）不破坏解析。
fn build_markdown(title: &str, tags: &Option<Vec<String>>, content: &str) -> String {
    #[derive(Serialize)]
    struct FrontMatterOut<'a> {
        title: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        tags: &'a Option<Vec<String>>,
    }
    let fm = FrontMatterOut { title, tags };
    let fm_yaml = serde_yaml::to_string(&fm).unwrap_or_default();
    format!("---\n{fm_yaml}---\n\n{content}")
}

// =========================================================================
// read_kb_document —— 读知识库文档全文（RAG 检索后读细节，免授权）
// =========================================================================

/// `read_kb_document` 工具：读取知识库文档的完整内容。
///
/// `search_kb` 只返回 title + summary（索引摘要），agent 看完摘要判断需要细节时，
/// 用本工具传 `file_path`（即 search_kb 返回的相对路径）读全文。工具内部知道 KB
/// 目录、自动定位、**免路径授权**（KB 是系统管理的信任内容，不该走通用文件授权）。
///
/// 定位顺序：当前 agent 的 KB → global KB（与 search_kb 的检索范围一致）。
pub struct ReadKbDocumentTool;

#[derive(Deserialize)]
struct ReadKbDocArgs {
    /// search_kb 返回的相对路径（相对 kb.directory）
    file_path: String,
}

#[async_trait]
impl McpClient for ReadKbDocumentTool {
    fn name(&self) -> &str {
        "read_kb_document"
    }

    fn description(&self) -> &str {
        "Read the full content of a knowledge base document. Pass the file_path returned \
         by search_kb. Use this after search_kb to get the complete content of a matched \
         document — search_kb only returns title + summary, this gives the full text."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Relative file path returned by search_kb (e.g. 'basics/ownership.md')."
                }
            },
            "required": ["file_path"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "read_kb_document 必须通过 execute_with_context 调用（需要 agent_id 上下文）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: ReadKbDocArgs =
            serde_json::from_str(args).map_err(|e| {
                AppError::Validation(format!("read_kb_document 参数解析失败: {e}"))
            })?;

        // 定位范围与 search_kb 一致：agent 专有 → global
        let agent_kbs = repo::kb::list_by_scope(&ctx.pool, "agent", Some(&ctx.agent_id)).await?;
        let global_kbs = repo::kb::list_by_scope(&ctx.pool, "global", None).await?;
        let scopes: Vec<_> = agent_kbs.into_iter().chain(global_kbs).collect();

        // 在这些 KB 中找含该 file_path 的文档（agent 优先）
        for kb in &scopes {
            if let Some(doc) =
                repo::kb::get_document_by_path(&ctx.pool, &kb.id, &parsed.file_path).await?
            {
                let abs = PathBuf::from(&kb.directory).join(&parsed.file_path);
                let content = match std::fs::read_to_string(&abs) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(
                            target: "ice_paw.kb",
                            "读取 KB 文档失败: {}/{} err={}",
                            kb.directory, parsed.file_path, e
                        );
                        return Err(AppError::Io(e));
                    }
                };
                tracing::info!(
                    target: "ice_paw.kb",
                    "read_kb_document: {}/{} ({} 字符)",
                    kb.directory, parsed.file_path, content.chars().count()
                );
                return Ok(serde_json::json!({
                    "file_path": parsed.file_path,
                    "title": doc.title,
                    "content": content,
                })
                .to_string());
            }
        }

        // 未命中（file_path 不在任何可用 KB 中）
        Ok(serde_json::json!({
            "file_path": parsed.file_path,
            "found": false,
            "message": "未在当前 agent 知识库或全局知识库中找到该文档，请确认 file_path 来自 search_kb 的返回。"
        })
        .to_string())
    }
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
            workspace: None,
            pool: pool.clone(),
            api_key: None,
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
            workspace: None,
            pool: pool.clone(),
            api_key: None,
        };
        let result = tool
            .execute_with_context(r#"{"query":"anything"}"#, &ctx)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["count"], 0);
        assert!(v["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn save_to_kb_schema_and_metadata() {
        let tool = SaveToKbTool;
        assert_eq!(tool.name(), "save_to_kb");
        assert_eq!(tool.authorization_level(), AuthorizationLevel::Always);
        let p = tool.parameters();
        assert_eq!(p["required"][0], "title");
        assert_eq!(p["properties"]["scope"]["enum"][0], "agent");
    }

    #[test]
    fn save_to_kb_rejects_invalid_scope() {
        assert!(validate_save_scope("project").is_err());
        assert!(validate_save_scope("agent").is_ok());
        assert!(validate_save_scope("global").is_ok());
    }

    /// 闭环：build_markdown 产出的 md 能被 parse_markdown 正确解析回 title/tags。
    /// 保证「写入的能被检索」这条 RAG 链路自洽（含冒号/井号等特殊字符）。
    #[test]
    fn save_to_kb_build_markdown_roundtrips_with_parser() {
        use crate::harness::kb::parser::parse_markdown;
        let md = build_markdown(
            "标题: 含冒号 与 #井号",
            &Some(vec!["rust".into(), "笔记".into()]),
            "这是正文内容，应该被解析为首段。",
        );
        let parsed = parse_markdown(&md);
        assert_eq!(parsed.title, "标题: 含冒号 与 #井号");
        assert_eq!(parsed.tags, r#"["rust","笔记"]"#);
        assert!(parsed.summary.contains("正文内容"));
    }

    /// read_kb_document：真实临时文件 → 返回全文（验证 RAG 全文读取闭环）
    #[tokio::test]
    async fn read_kb_document_returns_full_content() {
        let pool = fresh_pool().await;
        let dir = std::env::temp_dir().join("icepaw_test_read_kb_doc");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.md"), "# 测试标题\n\n这是完整正文。").unwrap();
        repo::kb::create(
            &pool,
            &NewKb {
                id: "kb-rd".into(),
                name: "test".into(),
                scope: "global".into(),
                owner_id: None,
                directory: dir.to_string_lossy().to_string(),
                enabled: true,
            },
        )
        .await
        .unwrap();
        repo::kb::upsert_document(&pool, "kb-rd", "note.md", "测试标题", "摘要", "[]", Some("h"), None)
            .await
            .unwrap();

        let tool = ReadKbDocumentTool;
        let ctx = ToolContext {
            conv_id: "c".into(),
            agent_id: "a".into(),
            project_id: None,
            workspace: None,
            pool: pool.clone(),
            api_key: None,
        };
        let result = tool.execute_with_context(r#"{"file_path":"note.md"}"#, &ctx).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["title"], "测试标题");
        assert!(v["content"].as_str().unwrap().contains("完整正文"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// read_kb_document：file_path 不在任何 KB → 返回未命中提示（不报错）
    #[tokio::test]
    async fn read_kb_document_not_found_returns_hint() {
        let pool = fresh_pool().await;
        let tool = ReadKbDocumentTool;
        let ctx = ToolContext {
            conv_id: "c".into(),
            agent_id: "a".into(),
            project_id: None,
            workspace: None,
            pool: pool.clone(),
            api_key: None,
        };
        let result = tool.execute_with_context(r#"{"file_path":"nope.md"}"#, &ctx).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["found"], false);
    }

    /// 回归：前端 bridge.preferences.set 用 JSON.stringify 存储，DB 实际值带引号。
    /// 必须走 get_all 反序列化才能拿到干净的 "glm"；裸 query_scalar 读到 "\"glm\""
    /// 会让 try_semantic_search 静默回退关键词（v2 失效根因，2026-08-05 修复）。
    #[tokio::test]
    async fn embedding_config_reads_json_stringified_storage() {
        let pool = fresh_pool().await;
        // 模拟前端存储（bridge.preferences.set → JSON.stringify → DB 带引号）
        repo::preferences::set(&pool, "embedding_provider", "\"glm\"").await.unwrap();
        repo::preferences::set(&pool, "embedding_model", "\"embedding-3\"").await.unwrap();
        repo::preferences::set(&pool, "embedding_api_key", "\"sk-test-xxx\"").await.unwrap();

        // get_all 必须去 JSON 引号
        let prefs = repo::preferences::get_all(&pool).await.unwrap();
        assert_eq!(prefs.embedding_provider.as_deref(), Some("glm"));
        assert_eq!(prefs.embedding_model.as_deref(), Some("embedding-3"));
        assert_eq!(prefs.embedding_api_key.as_deref(), Some("sk-test-xxx"));

        // 且 resolve_embedding_config 能解析出 glm 端点
        let (model, url, key) = crate::harness::kb::embedding::resolve_embedding_config(&prefs).expect("glm 配置应被解析");
        assert_eq!(model, "embedding-3");
        assert_eq!(url, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(key, "sk-test-xxx");
    }

    /// 端到端（需真实智谱 key）：验证修复后 search_kb 完整语义链路能跑通——
    /// 配置读取（JSON 存储）→ embed API → 懒生成 embedding → 持久化 → cosine 检索。
    ///
    /// 用法（bash）：
    ///   export ICEPAW_EMBEDDING_API_KEY=你的智谱key
    ///   SODIUM_LIB_DIR=... SODIUM_STATIC=true \
    ///     cargo test --manifest-path packages/app/src-tauri/Cargo.toml --lib -- \
    ///     --ignored e2e_search_kb_semantic_with_real_glm_api
    #[ignore]
    #[tokio::test]
    async fn e2e_search_kb_semantic_with_real_glm_api() {
        use crate::db::repo::kb::{
            load_chunks_for_vector_search, upsert_chunks_incremental, upsert_document,
        };

        let key = std::env::var("ICEPAW_EMBEDDING_API_KEY")
            .expect("需设置 ICEPAW_EMBEDDING_API_KEY（智谱 key）后用 --ignored 跑");
        let pool = fresh_pool().await;

        // 模拟前端 JSON.stringify 存储（与真实 app 完全一致）
        repo::preferences::set(&pool, "embedding_provider", "\"glm\"").await.unwrap();
        repo::preferences::set(&pool, "embedding_model", "\"embedding-3\"").await.unwrap();
        repo::preferences::set(&pool, "embedding_api_key", &format!("\"{key}\"")).await.unwrap();

        // seed global KB + 文档 + 3 个 chunk（embedding=NULL）
        seed_kb(&pool, "kb-g", "global", None).await;
        let doc_id = upsert_document(
            &pool, "kb-g", "rust-web.md", "Rust Web 开发",
            "用 axum 搭建 HTTP 服务的入门笔记", "[]", Some("h"), None,
        )
        .await
        .unwrap();
        upsert_chunks_incremental(&pool, &doc_id, &[
            "Rust 的异步运行时 tokio 提供了高效的并发能力。".into(),
            "axum 是基于 tower 的轻量 web 框架，支持路由与中间件。".into(),
            "序列化通常用 serde，性能与生态都很好。".into(),
        ])
        .await
        .unwrap();

        // 调用前：chunk embedding 应全为 NULL（刚入库）
        let before = load_chunks_for_vector_search(&pool, &["kb-g".to_string()]).await.unwrap();
        assert!(before.iter().all(|c| c.embedding.is_none()), "入库后 embedding 应为 NULL");

        // 触发 search_kb：语义检索路径会懒生成 embedding
        let tool = SearchKbTool;
        let ctx = ToolContext {
            conv_id: "c1".into(),
            agent_id: "any".into(),
            project_id: None,
            workspace: None,
            pool: pool.clone(),
            api_key: None,
        };
        let result = tool
            .execute_with_context(r#"{"query":"如何搭建网络服务","limit":3}"#, &ctx)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        eprintln!("search_kb 返回: {v}");

        // 关键断言：语义路径执行后，embedding 应被填充。
        // 修复前 try_semantic_search 读配置失败 → return None → 不生成 → 这里会是 0。
        let after = load_chunks_for_vector_search(&pool, &["kb-g".to_string()]).await.unwrap();
        let filled = after.iter().filter(|c| c.embedding.is_some()).count();
        assert_eq!(
            filled, 3,
            "3 个 chunk 的 embedding 都应被懒生成填充；实际 {filled}（若为 0 说明配置仍读不出或 API 不通）"
        );
    }
}
