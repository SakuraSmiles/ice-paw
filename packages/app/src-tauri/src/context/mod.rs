//! `context` — L2 Context 层（组装 LLM 调用前的完整上下文）
//!
//! **当前状态**：W1.1 建壳占位模块，文件为空。
//!
//! 后续 Sprint（W5.1–W5.3）将逐步从 `commands/chat_context.rs` 迁入：
//!
//! - `template`       — `{{var}}` 模板变量渲染
//! - `os_context`     — OS 环境信息注入（HOME、平台等）
//! - `image`          — 图片预处理（验证大小/数量、压缩）
//! - `system_prompt`  — 四级优先 system prompt 构造（agent override > conv override > template > default）
//! - `history`        — 历史消息加载（含 summary 窗口策略）
//! - `pipeline`       — 完整组装管线 `assemble_context()`
//!
//! 详见 Sprint 计划 W5.1–W5.3。