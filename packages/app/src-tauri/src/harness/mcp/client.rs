//! McpClient trait + McpRegistry — 统一工具接口与注册表
//!
//! Phase 1: 替代旧的 `Tool` trait + `ToolRegistry`。
//!
//! 设计要点：
//! - `McpClient` trait 定义统一的工具接口（name, description, parameters, execute）
//! - `McpRegistry` 持有所有已注册的客户端，按名称查询/分发
//! - 内置工具（read_file / list_directory）见 `internal` 子模块
//! - 对外暴露的 `list_tool_defs` 系列方法与旧 `ToolRegistry` 兼容

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use sqlx::SqlitePool;
use tauri::AppHandle;
use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};
use crate::harness::proposal_registry::ProposalRegistry;
use crate::infra::protocol::ToolDef;

use super::types::AuthorizationLevel;

// =========================================================================
// ToolContext — 工具执行上下文（RAG: agent_id/project_id 透传）
// =========================================================================

/// 工具执行上下文。
///
/// 大多数内置工具（read_file / list_directory / 外部 MCP Server 代理）
/// 不需要上下文 —— 它们只用 LLM 传入的 `args`。但某些工具（如 RAG 的
/// `search_kb`）必须知道「当前是哪个 agent / 项目 / 对话」才能查询对应的
/// 知识库。`ToolContext` 就是把这部分运行时信息透传给工具的载体。
///
/// 透传链路：`chat_cmd` 从 `ConversationRow` 取 agent_id/project_id →
/// `LoopContext` 携带 → `execute_tool_round` 构造 `ToolContext` →
/// `McpRegistry::dispatch(ctx)` → `McpClient::execute_with_context(ctx)`。
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// 当前对话 ID
    pub conv_id: String,
    /// 当前 Agent ID（决定查询哪级「agent 专业」知识库）
    pub agent_id: String,
    /// 当前项目 ID（v1 暂不启用 project KB，字段预留；None = 默认项目）
    pub project_id: Option<String>,
    /// 当前 Agent 的 workspace 绝对路径（run_command / git 作 current_dir 用）；None = 未设
    pub workspace: Option<String>,
    /// 数据库连接池（search_kb 查 kb/kb_document 表用）
    pub pool: SqlitePool,
    /// 当前 Agent 的 API Key（search_kb 调 embedding API 用；None = 不支持语义检索）
    pub api_key: Option<String>,
    /// AppHandle（propose_config_change 等需要 emit 事件的工具用；None = 不可用）
    pub app_handle: Option<AppHandle>,
    /// 提案注册表（propose_config_change 等配置工具用；None = 不可用）
    pub proposal_registry: Option<ProposalRegistry>,
}

// =========================================================================
// McpClient Trait
// =========================================================================

/// MCP 客户端统一接口
///
/// 每个工具/服务实现此 trait，提供名称、描述、参数 schema 和执行逻辑。
/// 内部工具在 Rust 侧执行，外部工具通过 stdio JSON-RPC 通信。
#[async_trait]
pub trait McpClient: Send + Sync {
    /// 工具名称（唯一标识，与 LLM 请求中的 tool name 一致）
    fn name(&self) -> &str;

    /// 工具描述（发给 LLM，帮助其判断何时调用）
    fn description(&self) -> &str;

    /// 参数 JSON Schema（发给 LLM，描述参数结构）
    fn parameters(&self) -> serde_json::Value;

    /// 工具授权级别（默认 `Always`，子类可覆盖）
    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    /// 执行工具
    ///
    /// - `args`：参数 JSON 字符串（由 LLM 产出）
    /// - 返回：结果 JSON 字符串（回传给 LLM）
    async fn execute(&self, args: &str) -> AppResult<String>;

    /// 带上下文执行工具（默认实现 = 忽略 ctx，直接转调 `execute`）。
    ///
    /// **旧工具零改动**：read_file / list_directory / 外部 MCP Server 代理
    /// 不需要运行时上下文，使用此默认实现即可。
    ///
    /// 需要上下文的工具（如 `search_kb`）override 此方法，从 `ctx` 取
    /// agent_id/project_id/pool 决定查询范围。
    async fn execute_with_context(
        &self,
        args: &str,
        _ctx: &ToolContext,
    ) -> AppResult<String> {
        self.execute(args).await
    }
}

// =========================================================================
// McpRegistry
// =========================================================================

/// 工具注册表
///
/// 持有所有已注册的 `McpClient`（`Arc<dyn McpClient>`），
/// 支持按名称查询和分发执行。
///
/// Clone 实现语义：与旧 `ToolRegistry` 一致——通过读锁快照复制内部 HashMap。
pub struct McpRegistry {
    clients: RwLock<HashMap<String, Arc<dyn McpClient>>>,
}

impl Clone for McpRegistry {
    fn clone(&self) -> Self {
        let snapshot: HashMap<String, Arc<dyn McpClient>> = match self.clients.try_read() {
            Ok(guard) => (*guard).clone(),
            Err(_) => {
                tracing::warn!(
                    target: "ice_paw.mcp",
                    "McpRegistry::clone() 读锁获取失败（写锁占用中），返回空快照；\
                     下游工具列表将缺失，请检查并发写锁占用情况"
                );
                HashMap::new()
            }
        };
        McpRegistry {
            clients: RwLock::new(snapshot),
        }
    }
}

impl McpRegistry {
    /// 创建空的注册表
    pub fn new() -> Self {
        McpRegistry {
            clients: RwLock::new(HashMap::new()),
        }
    }

    /// 创建注册表并注册内置工具
    pub fn with_builtin() -> Self {
        let registry = Self::new();
        registry.register_builtin();
        registry
    }

    /// 仅注册指定名称的内置工具。
    /// 未识别的工具名静默跳过（容错）。
    pub fn with_filter(names: &[String]) -> Self {
        let registry = Self::new();
        let all_builtins: Vec<(&str, Arc<dyn McpClient>)> = vec![
            ("read_file", Arc::new(super::internal::ReadFileTool)),
            ("list_directory", Arc::new(super::internal::ListDirectoryTool)),
            ("search_kb", Arc::new(super::kb_tool::SearchKbTool)),
            ("save_to_kb", Arc::new(super::kb_tool::SaveToKbTool)),
            ("read_kb_document", Arc::new(super::kb_tool::ReadKbDocumentTool)),
            ("write_file", Arc::new(super::file_tools::WriteFileTool)),
            ("edit_file", Arc::new(super::file_tools::EditFileTool)),
            ("delete_file", Arc::new(super::file_tools::DeleteFileTool)),
            ("run_command", Arc::new(super::shell::RunCommandTool)),
            ("search_files", Arc::new(super::search::SearchFilesTool)),
            ("git", Arc::new(super::git::GitTool)),
            ("web_fetch", Arc::new(super::web::WebFetchTool)),
            ("read_agent_config", Arc::new(super::agent_config::ReadAgentConfigTool)),
            ("propose_config_change", Arc::new(super::proposal_tool::ProposeConfigChangeTool)),
            ("delegate_to_agent", Arc::new(super::delegate::DelegateTool)),
        ];
        for name in names {
            if let Some((_, client)) = all_builtins.iter().find(|(n, _)| n == name) {
                registry.register_sync(client.clone());
            }
        }
        registry
    }

    /// 注册一个工具客户端
    pub async fn register(&self, client: Arc<dyn McpClient>) {
        let name = client.name().to_string();
        let mut clients = self.clients.write().await;
        clients.insert(name, client);
    }

    /// 按工具名批量反注册（server 停止/删除/改配时调用，避免死工具残留 → 调用卡 30s 超时）。
    /// 与 `register` 对称：register 用 `client.name()` 为 key，这里按名移除。
    pub async fn unregister(&self, names: &[String]) {
        let mut clients = self.clients.write().await;
        for name in names {
            clients.remove(name);
        }
    }

    /// 同步注册（用于初始化时，无竞争场景）
    pub fn register_sync(&self, client: Arc<dyn McpClient>) {
        if let Ok(mut clients) = self.clients.try_write() {
            let name = client.name().to_string();
            clients.insert(name, client);
        }
    }

    /// 注册内置工具
    pub fn register_builtin(&self) {
        // 只读 / 知识库
        self.register_sync(Arc::new(super::internal::ReadFileTool));
        self.register_sync(Arc::new(super::internal::ListDirectoryTool));
        self.register_sync(Arc::new(super::kb_tool::SearchKbTool));
        self.register_sync(Arc::new(super::kb_tool::SaveToKbTool));
        self.register_sync(Arc::new(super::kb_tool::ReadKbDocumentTool));
        // agentic 工具集（文件读写编辑 / shell / grep / git / web）
        self.register_sync(Arc::new(super::file_tools::WriteFileTool));
        self.register_sync(Arc::new(super::file_tools::EditFileTool));
        self.register_sync(Arc::new(super::file_tools::DeleteFileTool));
        self.register_sync(Arc::new(super::shell::RunCommandTool));
        self.register_sync(Arc::new(super::search::SearchFilesTool));
        self.register_sync(Arc::new(super::git::GitTool));
        self.register_sync(Arc::new(super::web::WebFetchTool));
        self.register_sync(Arc::new(super::agent_config::ReadAgentConfigTool));
        self.register_sync(Arc::new(super::proposal_tool::ProposeConfigChangeTool));
        // 注：delegate_to_agent 不在 register_builtin 中，仅在 with_filter 动态注册
    }

    /// 按名称查询工具客户端
    pub async fn get(&self, name: &str) -> Option<Arc<dyn McpClient>> {
        let clients = self.clients.read().await;
        clients.get(name).cloned()
    }

    /// 列出所有已注册客户端的工具定义（发给 LLM）
    pub async fn list_tool_defs(&self) -> Vec<ToolDef> {
        let clients = self.clients.read().await;
        clients
            .values()
            .map(|c| ToolDef {
                name: c.name().to_string(),
                description: c.description().to_string(),
                parameters: c.parameters(),
            })
            .collect()
    }

    /// 列出工具定义，并按 query 相关性打分 + 软裁剪
    ///
    /// 行为与旧 `ToolRegistry::list_tool_defs_with_query` 完全一致。
    pub async fn list_tool_defs_with_query(
        &self,
        query: &str,
        trim_threshold: Option<usize>,
        trim_top_k: usize,
        call_history: &[String],
    ) -> Vec<ToolDef> {
        let defs = self.list_tool_defs().await;

        let need_trim = match trim_threshold {
            Some(th) => defs.len() > th,
            None => false,
        };
        if !need_trim {
            return defs;
        }

        let scores = crate::harness::scoring::score_tools(query, &defs, call_history);

        let mut order: Vec<usize> = (0..defs.len()).collect();
        order.sort_by(|&a, &b| {
            let sa = scores.get(&defs[a].name).copied().unwrap_or(0);
            let sb = scores.get(&defs[b].name).copied().unwrap_or(0);
            sb.cmp(&sa).then(a.cmp(&b))
        });

        let mut ordered: Vec<ToolDef> = order.into_iter().map(|i| defs[i].clone()).collect();
        crate::harness::scoring::apply_trim_markers(&mut ordered, trim_top_k);
        ordered
    }

    /// 执行工具（带上下文）
    ///
    /// 查找工具后转调 `execute_with_context`。旧工具用默认实现（忽略 ctx），
    /// `search_kb` 等 override `execute_with_context` 的工具会拿到完整 ctx。
    pub async fn dispatch(
        &self,
        name: &str,
        args: &str,
        ctx: &ToolContext,
    ) -> AppResult<String> {
        let client = self
            .get(name)
            .await
            .ok_or_else(|| AppError::NotFound {
                resource: "tool",
                id: name.to_string(),
            })?;
        client.execute_with_context(args, ctx).await
    }

    /// 返回所有已注册的工具名列表
    pub async fn tool_names(&self) -> Vec<String> {
        let clients = self.clients.read().await;
        clients.keys().cloned().collect()
    }

    /// 从另一个 registry 克隆指定名称的工具到自身
    pub async fn register_names_from(&self, source: &McpRegistry, names: &[String]) {
        for name in names {
            if let Some(client) = source.get(name).await {
                self.register(client).await;
            }
        }
    }
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self::with_builtin()
    }
}

// =========================================================================
// 单测
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    /// 用于测试的 stub McpClient
    struct StubClient {
        name: String,
        description: String,
    }

    #[async_trait]
    impl McpClient for StubClient {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            &self.description
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(&self, _args: &str) -> AppResult<String> {
            Ok("stub".into())
        }
    }

    fn make_stub(name: &str, desc: &str) -> Arc<dyn McpClient> {
        Arc::new(StubClient {
            name: name.into(),
            description: desc.into(),
        })
    }

    #[tokio::test]
    async fn registry_builtin_tools() {
        let registry = McpRegistry::with_builtin();
        let defs = registry.list_tool_defs().await;
        assert!(defs.iter().any(|d| d.name == "read_file"));
        assert!(defs.iter().any(|d| d.name == "list_directory"));
    }

    #[tokio::test]
    async fn registry_get_existing() {
        let registry = McpRegistry::with_builtin();
        let client = registry.get("read_file").await;
        assert!(client.is_some());
        assert_eq!(client.unwrap().name(), "read_file");
    }

    #[tokio::test]
    async fn registry_get_nonexistent() {
        let registry = McpRegistry::with_builtin();
        let client = registry.get("nonexistent").await;
        assert!(client.is_none());
    }

    #[tokio::test]
    async fn registry_dispatch_nonexistent() {
        let registry = McpRegistry::with_builtin();
        let ctx = ToolContext {
            conv_id: "c1".into(),
            agent_id: "a1".into(),
            project_id: None,
            workspace: None,
            pool: sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap(),
            api_key: None,
            app_handle: None,
            proposal_registry: None,
        };
        let result = registry.dispatch("nonexistent", "{}", &ctx).await;
        assert!(result.is_err());
    }

    /// 验证 RAG 透传的核心保证：未 override `execute_with_context` 的旧工具
    /// 走默认实现（= 转调 execute），`dispatch(ctx)` 仍能正常执行它们。
    #[tokio::test]
    async fn dispatch_with_context_runs_legacy_tool() {
        let registry = McpRegistry::with_builtin();
        let ctx = ToolContext {
            conv_id: "c1".into(),
            agent_id: "a1".into(),
            project_id: None,
            workspace: None,
            pool: sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap(),
            api_key: None,
            app_handle: None,
            proposal_registry: None,
        };
        // StubClient 未 override execute_with_context → 走默认实现 → 返回 "stub"
        registry.register(make_stub("legacy", "legacy tool")).await;
        let result = registry.dispatch("legacy", "{}", &ctx).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "stub");
    }

    #[tokio::test]
    async fn registry_register_custom() {
        let registry = McpRegistry::new();
        registry.register(make_stub("my_tool", "My custom tool")).await;
        let defs = registry.list_tool_defs().await;
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "my_tool");
    }

    // =====================================================================
    // list_tool_defs_with_query 单测（与旧 ToolRegistry 测试等价）
    // =====================================================================

    async fn make_registry_with_n_tools(n: usize) -> McpRegistry {
        let registry = McpRegistry::new();
        for i in 0..n {
            registry
                .register(make_stub(&format!("tool_{i}"), &format!("desc for tool {i}")))
                .await;
        }
        let actual = registry.list_tool_defs().await;
        assert_eq!(actual.len(), n);
        registry
    }

    #[tokio::test]
    async fn trim_threshold_above_does_nothing() {
        let registry = make_registry_with_n_tools(2).await;
        let out = registry
            .list_tool_defs_with_query("", Some(5), 1, &[])
            .await;
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|d| !d.description.ends_with(" [deprioritized]")));
    }

    #[tokio::test]
    async fn trim_threshold_triggers_soft_trim() {
        let registry = make_registry_with_n_tools(4).await;
        let out = registry
            .list_tool_defs_with_query("", Some(2), 1, &[])
            .await;
        assert_eq!(out.len(), 4);
        let unmarked = out.iter().filter(|d| !d.description.ends_with(" [deprioritized]")).count();
        assert_eq!(unmarked, 1);
        let marked = out.iter().filter(|d| d.description.ends_with(" [deprioritized]")).count();
        assert_eq!(marked, 3);
    }

    #[tokio::test]
    async fn trim_preserves_top_k_count() {
        let registry = make_registry_with_n_tools(5).await;
        let out = registry
            .list_tool_defs_with_query("", Some(2), 3, &[])
            .await;
        let unmarked = out.iter().filter(|d| !d.description.ends_with(" [deprioritized]")).count();
        assert_eq!(unmarked, 3);
        let marked = out.iter().filter(|d| d.description.ends_with(" [deprioritized]")).count();
        assert_eq!(marked, 2);
    }

    #[tokio::test]
    async fn trim_with_empty_history_falls_back_to_score() {
        let registry = make_registry_with_n_tools(4).await;
        let out = registry
            .list_tool_defs_with_query("tool 3", Some(2), 1, &[])
            .await;
        assert_eq!(out[0].name, "tool_3");
        assert!(!out[0].description.ends_with(" [deprioritized]"));
        for d in &out[1..] {
            assert!(d.description.ends_with(" [deprioritized]"));
        }
    }

    #[tokio::test]
    async fn trim_under_cfg_equals_trivially() {
        let registry = make_registry_with_n_tools(3).await;
        let out = registry
            .list_tool_defs_with_query("anything", None, 1, &[])
            .await;
        let baseline = registry.list_tool_defs().await;
        assert_eq!(out.len(), baseline.len());
        assert!(out.iter().all(|d| !d.description.ends_with(" [deprioritized]")));
    }

    #[tokio::test]
    async fn trim_threshold_zero_triggers_always() {
        let registry = make_registry_with_n_tools(2).await;
        let out = registry
            .list_tool_defs_with_query("", Some(0), 1, &[])
            .await;
        assert_eq!(out.len(), 2);
        let unmarked = out.iter().filter(|d| !d.description.ends_with(" [deprioritized]")).count();
        assert_eq!(unmarked, 1);
    }

    #[tokio::test]
    async fn registry_clone_shares_state() {
        let reg1 = McpRegistry::with_builtin();
        let reg2 = reg1.clone();
        let defs1 = reg1.list_tool_defs().await;
        let defs2 = reg2.list_tool_defs().await;
        assert_eq!(defs1.len(), defs2.len());
        assert!(defs1.iter().any(|d| d.name == "read_file"));
        assert!(defs2.iter().any(|d| d.name == "read_file"));
    }

    #[tokio::test]
    async fn registry_unregister_removes_only_named() {
        // stop/delete server 时按工具名反注册，必须只移除命中的、其余保留
        let registry = McpRegistry::new();
        registry.register(make_stub("tool_a", "")).await;
        registry.register(make_stub("tool_b", "")).await;
        assert_eq!(registry.list_tool_defs().await.len(), 2);
        assert!(registry.get("tool_a").await.is_some());

        // 反注册单个
        registry.unregister(&["tool_a".to_string()]).await;
        assert!(registry.get("tool_a").await.is_none(), "tool_a 应已反注册");
        assert!(registry.get("tool_b").await.is_some(), "tool_b 应保留");
        assert_eq!(registry.list_tool_defs().await.len(), 1);

        // 反注册不存在的名 / 重复反注册 → 安全无副作用
        registry.unregister(&["tool_a".to_string(), "nope".to_string()]).await;
        assert_eq!(registry.list_tool_defs().await.len(), 1);

        // 批量反注册
        registry.register(make_stub("tool_c", "")).await;
        registry.unregister(&["tool_b".to_string(), "tool_c".to_string()]).await;
        assert_eq!(registry.list_tool_defs().await.len(), 0);
    }
}
