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
use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};
use crate::infra::protocol::ToolDef;

use super::types::AuthorizationLevel;

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

    /// 同步注册（用于初始化时，无竞争场景）
    pub fn register_sync(&self, client: Arc<dyn McpClient>) {
        if let Ok(mut clients) = self.clients.try_write() {
            let name = client.name().to_string();
            clients.insert(name, client);
        }
    }

    /// 注册内置工具
    pub fn register_builtin(&self) {
        self.register_sync(Arc::new(super::internal::ReadFileTool));
        self.register_sync(Arc::new(super::internal::ListDirectoryTool));
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

    /// 执行工具
    pub async fn dispatch(&self, name: &str, args: &str) -> AppResult<String> {
        let client = self
            .get(name)
            .await
            .ok_or_else(|| AppError::NotFound {
                resource: "tool",
                id: name.to_string(),
            })?;
        client.execute(args).await
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
        let result = registry.dispatch("nonexistent", "{}").await;
        assert!(result.is_err());
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
}
