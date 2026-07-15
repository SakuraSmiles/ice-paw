//! `harness::tool_registry` — 工具注册表（Tool trait + 内置工具 + 权限策略）
//!
//! **当前状态**：W1.1 建壳占位模块，文件为空。
//!
//! 后续 Sprint（W2.3 / W5.4–W5.5）将从 `llm/tool_registry.rs` 迁入：
//!
//! - `Tool` trait（含 `authorization_level()` 默认 `Always`）
//! - `ToolRegistry` 结构 + 注册/分发逻辑
//! - `AuthorizationLevel` 枚举（`Always` / `PathWhitelist` / `Confirm`）
//! - 内置工具：`ReadFileTool`（PathWhitelist 级）、`ListDirectoryTool`（Always 级）
//! - `authority` 子模块：路径白名单策略 + `AppError::AuthorizationRequired`
//!
//! 详见 Sprint 计划 W2.3 / W5.4–W5.5。