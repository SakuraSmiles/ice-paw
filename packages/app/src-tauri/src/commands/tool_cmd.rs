//! REQ-AGENT-004/024: 工具列表动态加载与 danger_level 展示。
//!
//! 提供 `list_tool_defs` 命令，返回所有已注册工具的定义。
//! 与 `ToolRegistry::list_tool_defs_with_danger_level()` 对齐，
//! 返回 `ToolDefWithDanger`（含 `danger_level` 字段）。

use crate::error::AppResult;
use crate::harness::tool_registry::{ToolRegistry, ToolDefWithDanger};

#[tauri::command]
pub async fn list_tool_defs() -> AppResult<Vec<ToolDefWithDanger>> {
    let registry = ToolRegistry::with_builtin();
    Ok(registry.list_tool_defs_with_danger_level().await)
}
