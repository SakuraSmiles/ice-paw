//! 工具注册表（Tool Registry）
//!
//! P2-1c: 定义 `Tool` trait 和内置工具，支持注册/查询/分发。
//! W5.4: 新增 `AuthorizationLevel` 枚举，支持分级权限控制。
//!
//! 设计要点：
//! - `Tool` trait 定义统一的工具接口（name, description, parameters, execute）
//! - `ToolRegistry` 持有所有已注册的工具，按名称查询和分发
//! - 内置工具：`read_file`、`list_directory`（安全只读工具）见 `builtin` 子模块
//! - 所有工具在 Rust 侧执行（安全 + 性能）
//! - 工具执行返回 JSON 字符串结果
//!
//! **W2.3**：从 `llm/tool_registry.rs`（451 行）迁入并拆为：
//!   - `mod.rs` — Tool trait + ToolRegistry struct + 注册/分发逻辑
//!   - `builtin.rs` — ReadFileTool + ListDirectoryTool + 工具单测
//!
//! W5.4: 新增 `AuthorizationLevel` 枚举 + `Tool::authorization_level()` 默认方法。

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};
use crate::infra::protocol::ToolDef;

pub mod authority;
pub mod builtin;

// =========================================================================
// AuthorizationLevel
// =========================================================================

/// 工具授权级别
///
/// - `Always`：无需授权，直接执行（如 `list_directory`）
/// - `PathWhitelist`：路径白名单校验（如 `read_file`）
/// - `Confirm`：需用户确认（未来扩展）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationLevel {
    /// 无需授权
    Always,
    /// 路径白名单校验
    PathWhitelist,
    /// 需用户确认（预留）
    Confirm,
}

impl Default for AuthorizationLevel {
    fn default() -> Self {
        Self::Always
    }
}

// =========================================================================
// Tool Trait
// =========================================================================

/// 工具接口 trait
///
/// 每个工具实现此 trait，提供名称、描述、参数 schema 和执行逻辑。
/// 工具在 Rust 侧执行（安全沙箱），返回 JSON 字符串结果。
#[async_trait]
pub trait Tool: Send + Sync {
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
// Tool Registry
// =========================================================================

/// 工具注册表
///
/// 持有所有已注册的工具（`Arc<dyn Tool>`），支持按名称查询和分发执行。
/// 使用 `RwLock<HashMap>` 实现线程安全的注册/查询。
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
}

impl ToolRegistry {
    /// 创建空的注册表
    pub fn new() -> Self {
        ToolRegistry {
            tools: RwLock::new(HashMap::new()),
        }
    }

    /// 创建注册表并注册内置工具
    pub fn with_builtin() -> Self {
        let registry = Self::new();
        registry.register_builtin();
        registry
    }

    /// 注册一个工具
    pub async fn register(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        let mut tools = self.tools.write().await;
        tools.insert(name, tool);
    }

    /// 同步注册（用于初始化时）
    pub fn register_sync(&self, tool: Arc<dyn Tool>) {
        // 使用 try_write 在非 async 上下文中注册
        // 前提：在初始化阶段调用，此时没有竞争
        if let Ok(mut tools) = self.tools.try_write() {
            let name = tool.name().to_string();
            tools.insert(name, tool);
        }
    }

    /// 注册内置工具
    pub fn register_builtin(&self) {
        self.register_sync(Arc::new(builtin::ReadFileTool));
        self.register_sync(Arc::new(builtin::ListDirectoryTool));
    }

    /// 按名称查询工具
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.get(name).cloned()
    }

    /// 列出所有已注册工具的定义（发给 LLM）
    pub async fn list_tool_defs(&self) -> Vec<ToolDef> {
        let tools = self.tools.read().await;
        tools
            .values()
            .map(|t| ToolDef {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters(),
            })
            .collect()
    }

    /// 执行工具
    ///
    /// - `name`：工具名称
    /// - `args`：参数 JSON 字符串
    /// - 返回：结果 JSON 字符串
    pub async fn dispatch(&self, name: &str, args: &str) -> AppResult<String> {
        let tool = self
            .get(name)
            .await
            .ok_or_else(|| AppError::NotFound {
                resource: "tool",
                id: name.to_string(),
            })?;

        tool.execute(args).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::with_builtin()
    }
}

// =========================================================================
// 单元测试（注册表本身）
// =========================================================================
//
// 内置工具（ReadFileTool / ListDirectoryTool）的单测见 `builtin.rs`。

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn registry_builtin_tools() {
        let registry = ToolRegistry::with_builtin();

        let defs = registry.list_tool_defs().await;
        assert!(defs.iter().any(|d| d.name == "read_file"));
        assert!(defs.iter().any(|d| d.name == "list_directory"));
    }

    #[tokio::test]
    async fn registry_get_existing() {
        let registry = ToolRegistry::with_builtin();
        let tool = registry.get("read_file").await;
        assert!(tool.is_some());
        assert_eq!(tool.unwrap().name(), "read_file");
    }

    #[tokio::test]
    async fn registry_get_nonexistent() {
        let registry = ToolRegistry::with_builtin();
        let tool = registry.get("nonexistent").await;
        assert!(tool.is_none());
    }

    #[tokio::test]
    async fn registry_dispatch_nonexistent() {
        let registry = ToolRegistry::with_builtin();
        let result = registry.dispatch("nonexistent", "{}").await;
        assert!(result.is_err());
    }
}
