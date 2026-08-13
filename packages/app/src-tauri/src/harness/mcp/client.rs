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
use crate::infra::cancel::CancellationToken;
use crate::infra::protocol::ToolDef;

use super::types::AuthorizationLevel;

/// 必须始终注入的平台元工具（不受 agent `enabled_tools` 白名单限制）。
///
/// 这些是 agent 安全/自省的基础设施：`propose_config_change` 是变更 agent 配置的
/// **唯一合法通道**（绕过它直接改文件会被 `reject_sensitive` 拦截），`read_agent_config`
/// 是读取自身配置的正规通道。若被白名单过滤掉，agent 会退而用文件工具直接改 agent.yaml，
/// 击穿配置提案审批系统。详见 `McpRegistry::register_meta_tools`。
const PLATFORM_META_TOOLS: &[&str] = &["propose_config_change", "read_agent_config"];

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
    /// 对话取消令牌（propose_config_change 等需在「停止生成」时提前返回的工具用；
    /// 由 execute_tool_round 的 enriched_ctx 注入；None = 无取消监听，回退纯超时）
    pub cancel: Option<CancellationToken>,
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

    /// 执行并可能产出图片（Phase B 视觉工具入口）。
    ///
    /// **默认实现 = 包 [`execute_with_context`] 为纯文本** [`ToolOutput`]——所有旧工具零改动。
    /// 仅视觉工具（`view_attachment_image`）override 此方法，在 [`ToolOutput::image_png`]
    /// 里返回渲染出的 PNG 字节。[`McpRegistry::dispatch`] 统一调此方法，故 `tool_executor`
    /// 拿到的永远是 `ToolOutput`，可按需附 `Image` 块。
    async fn execute_with_output(
        &self,
        args: &str,
        ctx: &ToolContext,
    ) -> AppResult<ToolOutput> {
        let text = self.execute_with_context(args, ctx).await?;
        Ok(ToolOutput::text(text))
    }
}

/// 工具执行结果（Phase B 视觉路径扩展）。
///
/// 绝大多数工具只回传文本——用 [`ToolOutput::text`] 即可（吃 trait 默认实现，零改动）。
/// **视觉工具**（`view_attachment_image`）需要把渲染出的图片一并发给模型：填 `image_png`
/// （原始 PNG 字节，**非 base64**），`tool_executor` 会把它编码成 base64 `Image` 块、
/// 与 `ToolResult` 同消息注入（Anthropic / GLM 自动识别）。
#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
    /// 回传给 LLM 的文本结果（JSON 或纯文本，即原 `execute` 的返回值）。
    pub text: String,
    /// 可选 PNG 图片（原始字节）。`None` = 纯文本工具（绝大多数）。
    pub image_png: Option<Vec<u8>>,
}

impl ToolOutput {
    /// 纯文本结果（绝大多数工具的返回）。
    pub fn text<T: Into<String>>(t: T) -> Self {
        Self {
            text: t.into(),
            image_png: None,
        }
    }

    /// 带图片的结果（视觉工具）。文本通常是一段说明 / JSON 摘要。
    pub fn with_image<T: Into<String>>(text: T, png: Vec<u8>) -> Self {
        Self {
            text: text.into(),
            image_png: Some(png),
        }
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
    /// 同步快照克隆（使用 `try_read`）。
    ///
    /// **注意**：若当前有写锁占用（register/unregister），
    /// 会返回空注册表并打 warn 日志。热路径（如 `send_message`）
    /// 应优先使用 `snapshot().await` 获取可靠快照。
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
    /// 异步获取可靠的工具快照（阻塞读，不受写锁竞争影响）。
    ///
    /// 热路径（如 `send_message`）应使用此方法替代 `clone()`，
    /// 确保 LLM 看到完整的工具列表。
    pub async fn snapshot(&self) -> HashMap<String, Arc<dyn McpClient>> {
        let guard = self.clients.read().await;
        (*guard).clone()
    }

    /// 从快照 HashMap 构造注册表（供 snapshot() 调用方使用）
    pub fn from_map(map: HashMap<String, Arc<dyn McpClient>>) -> Self {
        McpRegistry {
            clients: RwLock::new(map),
        }
    }

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
            ("directory_tree", Arc::new(super::internal::DirectoryTreeTool)),
            ("get_file_info", Arc::new(super::internal::GetFileInfoTool)),
            ("read_multiple_files", Arc::new(super::internal::ReadMultipleFilesTool)),
            ("search_kb", Arc::new(super::kb_tool::SearchKbTool)),
            ("save_to_kb", Arc::new(super::kb_tool::SaveToKbTool)),
            ("read_kb_document", Arc::new(super::kb_tool::ReadKbDocumentTool)),
            ("read_attachment_page", Arc::new(super::read_attachment_tool::ReadAttachmentPageTool)),
            ("view_attachment_image", Arc::new(super::attachment_image_tool::ViewAttachmentImageTool)),
            ("write_file", Arc::new(super::file_tools::WriteFileTool)),
            ("edit_file", Arc::new(super::file_tools::EditFileTool)),
            ("delete_file", Arc::new(super::file_tools::DeleteFileTool)),
            ("move_file", Arc::new(super::file_tools::MoveFileTool)),
            ("create_directory", Arc::new(super::file_tools::CreateDirectoryTool)),
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
        self.register_sync(Arc::new(super::internal::DirectoryTreeTool));
        self.register_sync(Arc::new(super::internal::GetFileInfoTool));
        self.register_sync(Arc::new(super::internal::ReadMultipleFilesTool));
        self.register_sync(Arc::new(super::kb_tool::SearchKbTool));
        self.register_sync(Arc::new(super::kb_tool::SaveToKbTool));
        self.register_sync(Arc::new(super::kb_tool::ReadKbDocumentTool));
        // 聊天附件分页按页读取（Phase A：大附件按块存表，首页内联，余页按需取）
        self.register_sync(Arc::new(super::read_attachment_tool::ReadAttachmentPageTool));
        // 聊天附件视觉读取（Phase B：扫描件/图片型 PDF 文本提取为空时，渲染成图喂视觉模型）
        self.register_sync(Arc::new(super::attachment_image_tool::ViewAttachmentImageTool));
        // agentic 工具集（文件读写编辑 / shell / grep / git / web）
        self.register_sync(Arc::new(super::file_tools::WriteFileTool));
        self.register_sync(Arc::new(super::file_tools::EditFileTool));
        self.register_sync(Arc::new(super::file_tools::DeleteFileTool));
        self.register_sync(Arc::new(super::file_tools::MoveFileTool));
        self.register_sync(Arc::new(super::file_tools::CreateDirectoryTool));
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

    /// 列出工具定义；工具数超过 `sort_threshold` 时按 query 相关性排序，否则保持注册原序。
    ///
    /// **所有工具始终全量返回**——排序只影响顺序（相关工具靠前），不裁剪、不降级、
    /// 不追加 `[deprioritized]` 标记。旧实现会给靠后工具打降级标记，导致 agent 误判
    /// 新增/远程工具不可用（如 GLM 系列）；全量无标记设计专为修正该行为。
    ///
    /// `sort_threshold`：工具数 > 此值才打分排序；`None` 永不排序（原序全发）。
    /// 推荐传 [`crate::harness::scoring::DEFAULT_TOOL_SORT_THRESHOLD`]。
    pub async fn list_tool_defs_with_query(
        &self,
        query: &str,
        sort_threshold: Option<usize>,
        call_history: &[String],
    ) -> Vec<ToolDef> {
        let defs = self.list_tool_defs().await;

        let need_sort = match sort_threshold {
            Some(th) => defs.len() > th,
            None => false,
        };
        if !need_sort {
            return defs;
        }

        let scores = crate::harness::scoring::score_tools(query, &defs, call_history);

        let mut order: Vec<usize> = (0..defs.len()).collect();
        order.sort_by(|&a, &b| {
            let sa = scores.get(&defs[a].name).copied().unwrap_or(0);
            let sb = scores.get(&defs[b].name).copied().unwrap_or(0);
            sb.cmp(&sa).then(a.cmp(&b))
        });

        order.into_iter().map(|i| defs[i].clone()).collect()
    }

    /// 执行工具（带上下文）
    ///
    /// 查找工具后转调 `execute_with_output`。旧工具用默认实现（包 `execute_with_context`
    /// 为纯文本 [`ToolOutput`]）；`view_attachment_image` 等视觉工具 override `execute_with_output`
    /// 回传 PNG。`search_kb` 等 override `execute_with_context` 的工具同样经默认包装拿到完整 ctx。
    pub async fn dispatch(
        &self,
        name: &str,
        args: &str,
        ctx: &ToolContext,
    ) -> AppResult<ToolOutput> {
        let client = self
            .get(name)
            .await
            .ok_or_else(|| AppError::NotFound {
                resource: "tool",
                id: name.to_string(),
            })?;
        client.execute_with_output(args, ctx).await
    }

    /// 与 [`dispatch`](Self::dispatch) 同，但用 `catch_unwind` 兜住工具执行期间的
    /// panic，转成 [`AppError::Internal`] 返回——不让单个工具的 panic 拖垮整个进程。
    ///
    /// 前提：release profile 必须是 `panic = "unwind"`（见 `Cargo.toml`），否则
    /// `catch_unwind` 捕获不到 panic（进程直接 abort）。`dispatch` 参数均为 `&` /
    /// `&self`，catch 后直接转 `Err` 不复用可能污染的状态，`AssertUnwindSafe` 安全。
    pub async fn dispatch_catch_panic(
        &self,
        name: &str,
        args: &str,
        ctx: &ToolContext,
    ) -> AppResult<ToolOutput> {
        use std::panic::AssertUnwindSafe;
        use futures::future::FutureExt;

        match AssertUnwindSafe(self.dispatch(name, args, ctx))
            .catch_unwind()
            .await
        {
            Ok(r) => r,
            Err(panic) => {
                let msg = panic
                    .downcast_ref::<String>()
                    .map(|s| s.clone())
                    .or_else(|| panic.downcast_ref::<&'static str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "<未知 panic>".to_string());
                tracing::error!(
                    target: "ice_paw.tool_panic",
                    tool = name,
                    "工具执行 panic 被兜底（对话可继续）: {msg}"
                );
                Err(AppError::Internal(format!(
                    "工具 `{name}` 执行时发生内部错误（已兜底，对话可继续）"
                )))
            }
        }
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

    /// 从 source 强制注入所有平台元工具（[`PLATFORM_META_TOOLS`]），忽略 agent 白名单。
    ///
    /// 当前组装默认全开（`from_map(snapshot)` 已含元工具），此方法暂未在组装路径调用；
    /// 保留供将来 agent 工具白名单 UI 启用时恢复——届时白名单分支需调它补元工具，否则
    /// agent 会退而用文件工具直接改 agent.yaml 绕过审批。
    pub async fn register_meta_tools(&self, source: &McpRegistry) {
        for &name in PLATFORM_META_TOOLS {
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

    /// P0-A：白名单模式下，平台元工具必须被强制注入（即使白名单未列出）。
    #[tokio::test]
    async fn register_meta_tools_forces_injection_ignoring_whitelist() {
        // 模拟 chat_cmd 白名单分支：只注册业务工具，再强制注入元工具
        let source = McpRegistry::with_builtin();
        let reg = McpRegistry::new();
        reg.register_names_from(&source, &["read_file".to_string()])
            .await;
        reg.register_meta_tools(&source).await;

        let names = reg.tool_names().await;
        // 白名单业务工具在
        assert!(names.iter().any(|n| n == "read_file"));
        // 元工具被强制注入（即使白名单没列）
        assert!(names.iter().any(|n| n == "propose_config_change"));
        assert!(names.iter().any(|n| n == "read_agent_config"));
    }

    /// source 不含元工具时 register_meta_tools 静默跳过，不 panic。
    #[tokio::test]
    async fn register_meta_tools_skips_missing_in_source() {
        let source = McpRegistry::new();
        let reg = McpRegistry::new();
        reg.register_meta_tools(&source).await;
        assert!(reg.tool_names().await.is_empty());
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
            cancel: None,
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
            cancel: None,
        };
        // StubClient 未 override execute_with_context → 走默认实现（包成 ToolOutput::text）→ 返回 "stub"
        registry.register(make_stub("legacy", "legacy tool")).await;
        let result = registry.dispatch("legacy", "{}", &ctx).await;
        assert!(result.is_ok());
        let out = result.unwrap();
        assert_eq!(out.text, "stub");
        assert!(out.image_png.is_none(), "纯文本工具（默认实现）不应回传图片");
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
    // list_tool_defs_with_query 单测——纯排序语义（全量发送，不裁剪不标记）
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
    async fn sort_threshold_above_no_reorder() {
        // 工具数(2) ≤ 阈值(5) → 不排序。验「未触发排序 → 与 baseline 一致」；
        // baseline 取自 list_tool_defs()（HashMap 序，非插入序），故此处断言的是
        // 「无 reorder」而非字面注册顺序。
        let registry = make_registry_with_n_tools(2).await;
        let baseline = registry.list_tool_defs().await;
        let out = registry.list_tool_defs_with_query("", Some(5), &[]).await;
        let names: Vec<_> = out.iter().map(|d| d.name.clone()).collect();
        let base_names: Vec<_> = baseline.iter().map(|d| d.name.clone()).collect();
        assert_eq!(out.len(), 2);
        assert_eq!(names, base_names, "未达排序阈值应与 baseline 一致（无 reorder）");
    }

    #[tokio::test]
    async fn sort_threshold_keeps_all_tools() {
        // 工具数(4) > 阈值(2) → 触发排序；核心保证是「不丢工具」（全量返回）。
        // reorder 的正确性由 sort_ranks_query_match_first 覆盖（这里 query="" 全 0 分，
        // 稳定排序保持原序，观察不到 reorder）。
        let registry = make_registry_with_n_tools(4).await;
        let out = registry.list_tool_defs_with_query("", Some(2), &[]).await;
        assert_eq!(out.len(), 4, "排序不得丢工具");
    }

    #[tokio::test]
    async fn sort_never_adds_deprioritized_marker() {
        // 关键回归：无论是否排序，description 都不得被追加降级标记
        let registry = make_registry_with_n_tools(5).await;
        let out = registry.list_tool_defs_with_query("", Some(2), &[]).await;
        assert!(
            out.iter().all(|d| !d.description.contains("deprioritized")),
            "新设计永不标记工具，实际: {:?}",
            out.iter().map(|d| &d.description).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn sort_ranks_query_match_first() {
        // query 命中某工具描述 → 该工具应排到最前（且仍全量、无标记）
        let registry = make_registry_with_n_tools(4).await;
        let out = registry
            .list_tool_defs_with_query("tool 3", Some(2), &[])
            .await;
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].name, "tool_3", "query 命中的工具应排首位");
    }

    #[tokio::test]
    async fn sort_none_threshold_no_reorder() {
        // None → 永不排序。验「与 baseline 一致（无 reorder）」；baseline 为 HashMap 序。
        let registry = make_registry_with_n_tools(3).await;
        let baseline = registry.list_tool_defs().await;
        let out = registry
            .list_tool_defs_with_query("anything", None, &[])
            .await;
        let names: Vec<_> = out.iter().map(|d| d.name.clone()).collect();
        let base_names: Vec<_> = baseline.iter().map(|d| d.name.clone()).collect();
        assert_eq!(out.len(), baseline.len());
        assert_eq!(names, base_names, "None 阈值应与 baseline 一致（无 reorder）");
    }

    #[tokio::test]
    async fn sort_threshold_zero_always_sorts_but_keeps_all() {
        // Some(0) → 工具数总 > 0 → 总触发排序，但仍全量
        let registry = make_registry_with_n_tools(2).await;
        let out = registry.list_tool_defs_with_query("", Some(0), &[]).await;
        assert_eq!(out.len(), 2, "即便总触发排序也不得丢工具");
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
