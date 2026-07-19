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
pub mod scoring;

// =========================================================================
// AuthorizationLevel
// =========================================================================

/// 工具授权级别
///
/// - `Always`：无需授权，直接执行（如 `list_directory`）
/// - `PathWhitelist`：路径白名单校验（如 `read_file`）
/// - `Confirm`：需用户确认（未来扩展）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum AuthorizationLevel {
    /// 无需授权
    #[default]
    Always,
    /// 路径白名单校验
    PathWhitelist,
    /// 需用户确认（预留）
    Confirm,
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
///
/// Clone 实现：`tokio::sync::RwLock` 内部使用 `UnsafeCell` + 原子操作，
/// 不实现 `Clone`。但 `Arc<dyn Tool>` 是 `Clone` 的，且 `HashMap::new()`
/// 可直接 clone（HashMap 实现了 Clone）。这里用手动 impl Clone：拷贝
/// `Arc` 引用 + 新建空 HashMap（让读锁懒加载），提供给 PipelineStage
/// 等需要拥有独立引用所有权的场景。
///
/// 实际上「真」的实现需要拿到内部 HashMap 的快照；但 ToolRegistry 的
/// clone 用法（ToolTrimStage）只需要「能调 list_tool_defs / get /
/// dispatch」即可，这些方法都拿读锁从 HashMap 里读，**不要求** clone
/// 后立刻看到另一个实例注册的「后续」工具——因为 PipelineStage 在
/// pipeline 启动时就固定了 registry 集合。所以这里采用轻量 clone：
/// 直接 `Arc::clone` 内部工具的 Arc，构造一个新 HashMap。
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        // 由于 `tokio::sync::RwLock` 本身不实现 Clone，这里采取
        // 「获取读锁快照 + 重新构造」的策略。
        // 读锁在同步上下文中通过 `try_read` 获取（不阻塞）。若当前被
        // 写锁占用则返回空快照——这种情况仅在另一个线程持有写锁时发生，
        // 而 ToolRegistry 的 clone 多在初始化 / Pipeline 阶段调用，
        // 此时通常不会有写锁占用；但一旦发生竞争，clone 会拿到空集合，
        // 下游工具列表将完全缺失，必须留日志以便排查。
        let snapshot: HashMap<String, Arc<dyn Tool>> = match self.tools.try_read() {
            Ok(guard) => (*guard).clone(),
            Err(_) => {
                tracing::warn!(
                    target: "ice_paw.tool_registry",
                    "ToolRegistry::clone() 读锁获取失败（写锁占用中），返回空快照；\
                     下游工具列表将缺失，请检查并发写锁占用情况"
                );
                HashMap::new()
            }
        };
        ToolRegistry {
            tools: RwLock::new(snapshot),
        }
    }
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

    /// 列出工具定义，并按 query 相关性打分 + 软裁剪（M1.2）
    ///
    /// # 行为
    /// - `trim_threshold == None` → 不裁剪，返回全量（等价于 `list_tool_defs`）
    /// - `defs.len() <= trim_threshold` → 不裁剪，返回全量
    /// - 否则：按 score_tools 打分 → 按分数降序 → top-k 保留原样 → 其余追加
    ///   ` [deprioritized]` 软标记
    ///
    /// # 参数
    /// - `query`              当前用户消息纯文本（用于子串匹配打分）
    /// - `trim_threshold`     工具数超过此值才触发裁剪；None 表示不裁剪
    /// - `trim_top_k`         保留原样的工具数
    /// - `call_history`       最近调用过的工具名列表（顺序不限）
    pub async fn list_tool_defs_with_query(
        &self,
        query: &str,
        trim_threshold: Option<usize>,
        trim_top_k: usize,
        call_history: &[String],
    ) -> Vec<ToolDef> {
        let defs = self.list_tool_defs().await;

        // 1) 阈值未设置 / 未触发 → 直接返回全量
        let need_trim = match trim_threshold {
            Some(th) => defs.len() > th,
            None => false,
        };
        if !need_trim {
            return defs;
        }

        // 2) 打分
        let scores = scoring::score_tools(query, &defs, call_history);

        // 3) 按分数降序稳定排序：分数相同则保持原顺序（用 enumerate index 作 tie-breaker）
        let mut order: Vec<usize> = (0..defs.len()).collect();
        order.sort_by(|&a, &b| {
            let sa = scores.get(&defs[a].name).copied().unwrap_or(0);
            let sb = scores.get(&defs[b].name).copied().unwrap_or(0);
            sb.cmp(&sa).then(a.cmp(&b))
        });

        // 4) 按新顺序组装
        let mut ordered: Vec<ToolDef> = order.into_iter().map(|i| defs[i].clone()).collect();

        // 5) 软裁剪：top-k 不动，其余追加 [deprioritized]
        scoring::apply_trim_markers(&mut ordered, trim_top_k);
        ordered
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

    // =========================================================================
    // M1.2 list_tool_defs_with_query 单测
    // =========================================================================

    /// 构造一个含 N 个工具的注册表：name = "tool_{i}"，description = "desc_{i}"
    async fn make_registry_with_n_tools(n: usize) -> (ToolRegistry, Vec<ToolDef>) {
        let registry = ToolRegistry::new();
        let defs: Vec<ToolDef> = (0..n)
            .map(|i| ToolDef {
                name: format!("tool_{i}"),
                description: format!("desc for tool {i}"),
                parameters: serde_json::json!({}),
            })
            .collect();

        // 用一个 stub Tool 实现直接塞进 registry 比较麻烦——
        // 直接用 builtin.rs 的两个 + 自定义若干：这里用更轻量的方式：
        // 我们只关心 list_tool_defs() 的输出形状，不在乎 Tool trait。
        // 但 list_tool_defs() 内部是从 tools HashMap 里读 Tool 的 name/desc。
        // 为了能注册自定义 defs，需要实现 Tool trait。简化方案：复用 builtin
        // 工具，再加几个「一次性」 stub。
        use async_trait::async_trait;
        struct StubTool {
            name: String,
            description: String,
        }
        #[async_trait]
        impl crate::harness::tool_registry::Tool for StubTool {
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

        for d in &defs {
            registry
                .register(Arc::new(StubTool {
                    name: d.name.clone(),
                    description: d.description.clone(),
                }))
                .await;
        }
        // 验证长度
        let actual = registry.list_tool_defs().await;
        assert_eq!(actual.len(), n);
        (registry, defs)
    }

    #[tokio::test]
    async fn trim_threshold_above_does_nothing() {
        // 2 个工具，threshold=5 → 不裁剪，2 个都返回
        let (registry, _defs) = make_registry_with_n_tools(2).await;
        let out = registry
            .list_tool_defs_with_query("", Some(5), 1, &[])
            .await;
        assert_eq!(out.len(), 2);
        // 没有任何标记
        assert!(out.iter().all(|d| !d.description.ends_with(" [deprioritized]")));
    }

    #[tokio::test]
    async fn trim_threshold_triggers_soft_trim() {
        // 4 个工具，threshold=2, top_k=1 → 1 个保留原样，3 个被标记
        let (registry, _defs) = make_registry_with_n_tools(4).await;
        let out = registry
            .list_tool_defs_with_query("", Some(2), 1, &[])
            .await;
        assert_eq!(out.len(), 4);
        // 1 个不带标记
        let unmarked: Vec<&ToolDef> = out
            .iter()
            .filter(|d| !d.description.ends_with(" [deprioritized]"))
            .collect();
        assert_eq!(unmarked.len(), 1);
        // 3 个带标记
        let marked: usize = out
            .iter()
            .filter(|d| d.description.ends_with(" [deprioritized]"))
            .count();
        assert_eq!(marked, 3);
    }

    #[tokio::test]
    async fn trim_preserves_top_k_count() {
        // 5 个工具，threshold=2, top_k=3 → 3 个保留原样，2 个被标记
        let (registry, _defs) = make_registry_with_n_tools(5).await;
        let out = registry
            .list_tool_defs_with_query("", Some(2), 3, &[])
            .await;
        let unmarked = out
            .iter()
            .filter(|d| !d.description.ends_with(" [deprioritized]"))
            .count();
        assert_eq!(unmarked, 3);
        let marked = out
            .iter()
            .filter(|d| d.description.ends_with(" [deprioritized]"))
            .count();
        assert_eq!(marked, 2);
    }

    #[tokio::test]
    async fn trim_with_empty_history_falls_back_to_score() {
        // 空历史 + 与 query 匹配的 token → 应优先排在 top_k 里
        let (registry, _defs) = make_registry_with_n_tools(4).await;
        // query="tool 3" → tool_3 拿到 name 精确 +3；其他仅 description 命中
        let out = registry
            .list_tool_defs_with_query("tool 3", Some(2), 1, &[])
            .await;
        // tool_3 应排第 1（保留原样）
        assert_eq!(out[0].name, "tool_3");
        assert!(!out[0].description.ends_with(" [deprioritized]"));
        // 其余 3 个带标记
        for d in &out[1..] {
            assert!(d.description.ends_with(" [deprioritized]"));
        }
    }

    #[tokio::test]
    async fn trim_under_cfg_equals_trivially() {
        // threshold=None → 不裁剪，等价 list_tool_defs
        let (registry, _) = make_registry_with_n_tools(3).await;
        let out = registry
            .list_tool_defs_with_query("anything", None, 1, &[])
            .await;
        let baseline = registry.list_tool_defs().await;
        assert_eq!(out.len(), baseline.len());
        // 没有任何标记
        assert!(out.iter().all(|d| !d.description.ends_with(" [deprioritized]")));
    }

    #[tokio::test]
    async fn trim_threshold_zero_triggers_always() {
        // 边界：threshold=0 → 任何 defs.len() > 0 都触发裁剪
        let (registry, _) = make_registry_with_n_tools(2).await;
        let out = registry
            .list_tool_defs_with_query("", Some(0), 1, &[])
            .await;
        assert_eq!(out.len(), 2);
        let unmarked = out
            .iter()
            .filter(|d| !d.description.ends_with(" [deprioritized]"))
            .count();
        assert_eq!(unmarked, 1);
    }
}
