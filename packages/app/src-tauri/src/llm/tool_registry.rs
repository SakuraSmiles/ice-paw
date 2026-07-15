//! 工具注册表（Tool Registry）
//!
//! P2-1c: 定义 `Tool` trait 和内置工具，支持注册/查询/分发。
//!
//! 设计要点：
//! - `Tool` trait 定义统一的工具接口（name, description, parameters, execute）
//! - `ToolRegistry` 持有所有已注册的工具，按名称查询和分发
//! - 内置工具：`read_file`、`list_directory`（安全只读工具）
//! - 所有工具在 Rust 侧执行（安全 + 性能）
//! - 工具执行返回 JSON 字符串结果

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};
use crate::llm::ToolDef;

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

    /// 执行工具
    ///
    /// - `args`：参数 JSON 字符串（由 LLM 产出）
    /// - 返回：结果 JSON 字符串（回传给 LLM）
    async fn execute(&self, args: &str) -> AppResult<String>;
}

// =========================================================================
// 内置工具
// =========================================================================

/// `read_file` 工具：读取本地文件内容
pub struct ReadFileTool;

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
    #[serde(default = "default_max_read_bytes")]
    max_bytes: usize,
}

fn default_max_read_bytes() -> usize {
    1024 * 1024 // 1MB 默认上限
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a local file. Returns the file content as text."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file to read."
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Maximum bytes to read (default: 1048576 = 1MB).",
                    "default": 1048576
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: ReadFileArgs =
            serde_json::from_str(args).map_err(|e| AppError::Validation(format!(
                "read_file 参数解析失败: {e}"
            )))?;

        let path = Path::new(&parsed.path);

        // 安全检查：拒绝读取特殊文件
        let canonical = path.canonicalize().map_err(|e| {
            AppError::Validation(format!("文件路径无效: {e}"))
        })?;

        // 拒绝读取 /proc /sys /dev 等
        let path_str = canonical.to_string_lossy();
        if path_str.starts_with("/proc/") || path_str.starts_with("/sys/") || path_str.starts_with("/dev/") {
            return Err(AppError::Validation(
                "出于安全原因，不允许读取系统虚拟文件系统".into(),
            ));
        }

        let metadata = tokio::fs::metadata(&canonical).await.map_err(|e| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("文件不存在或不可访问: {e}"),
            ))
        })?;

        if metadata.len() as usize > parsed.max_bytes {
            return Err(AppError::Validation(format!(
                "文件过大: {} bytes > {} bytes 上限",
                metadata.len(),
                parsed.max_bytes
            )));
        }

        let content = tokio::fs::read_to_string(&canonical)
            .await
            .map_err(AppError::Io)?;

        #[derive(Serialize)]
        struct ReadFileResult {
            path: String,
            size: u64,
            content: String,
        }

        let result = ReadFileResult {
            path: parsed.path,
            size: metadata.len(),
            content,
        };

        Ok(serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()))
    }
}

/// `list_directory` 工具：列出目录内容
pub struct ListDirectoryTool;

#[derive(Deserialize)]
struct ListDirectoryArgs {
    path: String,
}

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List the contents of a local directory. Returns a list of files and subdirectories."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the directory to list."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: &str) -> AppResult<String> {
        let parsed: ListDirectoryArgs =
            serde_json::from_str(args).map_err(|e| AppError::Validation(format!(
                "list_directory 参数解析失败: {e}"
            )))?;

        let path = Path::new(&parsed.path);

        if !path.exists() {
            return Err(AppError::Validation(format!(
                "目录不存在: {}",
                parsed.path
            )));
        }

        if !path.is_dir() {
            return Err(AppError::Validation(format!(
                "路径不是目录: {}",
                parsed.path
            )));
        }

        let mut entries = Vec::new();

        let mut reader = tokio::fs::read_dir(path).await.map_err(|e| {
            AppError::Io(e)
        })?;

        #[derive(Serialize)]
        struct DirEntry {
            name: String,
            is_dir: bool,
            size: Option<u64>,
        }

        while let Some(entry) = reader.next_entry().await.map_err(AppError::Io)? {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            let size = if is_dir {
                None
            } else {
                entry.metadata().await.ok().map(|m| m.len())
            };

            entries.push(DirEntry { name, is_dir, size });
        }

        // 按名称排序（目录优先）
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string()))
    }
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
        self.register_sync(Arc::new(ReadFileTool));
        self.register_sync(Arc::new(ListDirectoryTool));
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
// 单元测试
// =========================================================================

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

    #[tokio::test]
    async fn read_file_tool_valid() {
        let tool = ReadFileTool;
        // 读取自身 Cargo.toml（确定存在）
        let args = serde_json::json!({
            "path": "Cargo.toml",
            "max_bytes": 10240
        })
        .to_string();

        let result = tool.execute(&args).await;
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("ice-paw"));
    }

    #[tokio::test]
    async fn read_file_tool_nonexistent() {
        let tool = ReadFileTool;
        let args = r#"{"path": "/nonexistent/file/that/should/not/exist"}"#;

        let result = tool.execute(args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_file_tool_rejects_proc() {
        let tool = ReadFileTool;
        let args = r#"{"path": "/proc/self/status"}"#;

        let result = tool.execute(args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("安全") || msg.contains("不存在") || msg.contains("无效"),
            "should reject proc: {msg}"
        );
    }

    #[tokio::test]
    async fn list_directory_tool_valid() {
        let tool = ListDirectoryTool;
        let args = r#"{"path": "."}"#;

        let result = tool.execute(args).await;
        assert!(result.is_ok());
        let content = result.unwrap();
        // 应该是一个 JSON 数组
        assert!(content.starts_with("["));
    }

    #[tokio::test]
    async fn list_directory_tool_on_file() {
        let tool = ListDirectoryTool;
        let args = r#"{"path": "Cargo.toml"}"#;

        let result = tool.execute(args).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_directory_tool_nonexistent() {
        let tool = ListDirectoryTool;
        let args = r#"{"path": "/nonexistent/directory"}"#;

        let result = tool.execute(args).await;
        assert!(result.is_err());
    }

    #[test]
    fn tool_def_generation() {
        let tool = ReadFileTool;
        let params = tool.parameters();
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["path"].is_object());
        assert_eq!(params["required"][0], "path");
    }
}
