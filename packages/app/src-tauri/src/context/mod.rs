//! `context` — L2 Context 层（组装 LLM 调用前的完整上下文）
//!
//! W5.1–W5.3：从 `commands/chat_context.rs` 迁入全部子模块。
//!
//! - `template`       — `{{var}}` 模板变量渲染
//! - `os_context`     — OS 环境信息注入
//! - `system_prompt`  — system prompt 构造逻辑
//! - `history`        — 历史消息加载
//! - `pipeline`       — 完整组装管线 trait + Runner + Context（trait 定义）
//! - `stages`         — 5 个具体 PipelineStage 实现（M2.2 拆分自 pipeline.rs）
//! - `pipeline_tests` — Pipeline 单元测试（M2.2 拆分自 pipeline.rs）
//! - `memory`         — MemoryStage（B04，独立维护）
//! - `token`          — 上下文预算 / token 估算

pub(crate) mod history;
pub(crate) mod memory;
pub(crate) mod os_context;
pub(crate) mod pipeline;
pub(crate) mod stages;
pub(crate) mod system_prompt;
pub(crate) mod template;
pub(crate) mod token;

#[cfg(test)]
mod pipeline_tests;