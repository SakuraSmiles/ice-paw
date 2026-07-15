//! L2 Tool Executor — 工具执行编排（W3.3）
//!
//! 职责：对一批已完成的工具调用执行，收集 tool_use + tool_result 的 ContentBlock，
//! 并 emit `chat:tool-result` 事件。
//!
//! 抽取范围：`commands/chat_loop.rs` 的工具执行循环块。

use tauri::{AppHandle, Emitter};

use crate::infra::protocol::{ChatToolResultPayload, ContentBlock};
use crate::harness::tool_registry::ToolRegistry;

/// 执行一批工具调用，返回 (tool_use_blocks, tool_result_blocks)。
pub async fn execute_tool_round(
    app: &AppHandle,
    registry: &ToolRegistry,
    completed_calls: &[(String, String, String)],
    conv_id: &str,
    asst_msg_id: &str,
) -> crate::error::AppResult<(Vec<ContentBlock>, Vec<ContentBlock>)> {
    let mut tool_use_blocks: Vec<ContentBlock> = Vec::new();
    let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

    for (tc_id, tc_name, tc_args) in completed_calls {
        let result = registry.dispatch(tc_name, tc_args).await;

        match result {
            Ok(content) => {
                let _ = app.emit(
                    "chat:tool-result",
                    ChatToolResultPayload {
                        conversation_id: conv_id.to_string(),
                        message_id: asst_msg_id.to_string(),
                        tool_use_id: tc_id.clone(),
                        content: content.clone(),
                        is_error: false,
                    },
                );
                tool_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: tc_id.clone(),
                    content,
                    is_error: Some(false),
                });
            }
            Err(e) => {
                let err_content = e.to_string();
                let _ = app.emit(
                    "chat:tool-result",
                    ChatToolResultPayload {
                        conversation_id: conv_id.to_string(),
                        message_id: asst_msg_id.to_string(),
                        tool_use_id: tc_id.clone(),
                        content: err_content.clone(),
                        is_error: true,
                    },
                );
                tool_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: tc_id.clone(),
                    content: err_content,
                    is_error: Some(true),
                });
            }
        }

        tool_use_blocks.push(ContentBlock::ToolUse {
            id: tc_id.clone(),
            name: tc_name.clone(),
            input: tc_args.clone(),
        });
    }

    Ok((tool_use_blocks, tool_result_blocks))
}
