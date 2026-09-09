//! `send_message_to_session` 工具——MA-3 跨会话通讯的 agent 投递入口。
//!
//! 会话 A 的 agent 向会话 B **异步**投递消息（与 [`super::delegate`] 的同步
//! 阻塞形态互补不替代）：工具立即返回（`held`/`queued`/`delivered`），目标
//! 会话排队、空闲时消费——由 [`crate::harness::inbox`] 引擎承载（收件三态/
//! 护栏/expect_reply 回投全在彼处，本文件只是薄壳：目标解析 + 调用 + 结果
//! JSON 化）。
//!
//! ## 注册边界（与 delegate 同款）
//!
//! 组装期按 `conv.kind == "chat"` 注册（session_runner）——delegation 子会话
//! 拿不到本工具，防子会话侧信道绕过委派深度护栏；PLATFORM_TOOLS 白名单
//! 补入（enabled_tools 收窄不断跨会话通讯能力）。
//!
//! ## 授权
//!
//! `AuthorizationLevel::Always`（通用授权层不弹）——授权决策点是**收件三态**
//! （目标会话的 inbox_policy：hold 默认扣住待用户批准），不是弹卡。

use async_trait::async_trait;
use serde::Deserialize;

use crate::db::models::ConversationRow;
use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::harness::inbox::{self, SourceInfo};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

pub struct SendToSessionTool;

#[derive(Deserialize)]
struct RelayArgs {
    /// 目标会话 id 或标题（标题须唯一匹配，重名不猜——用 id 重试）
    target: String,
    content: String,
    /// true = 消费回合结束后自动把目标 agent 的回复回投本会话（链一次止）
    #[serde(default)]
    expect_reply: bool,
}

/// 目标会话解析（纯函数，可测）：id 精确匹配优先，退标题唯一匹配
/// （delegate::resolve_target 同哲学——重名歧义返回 None 走「找不到」错误，
/// 逼调用方用 id）。标题匹配只认 kind='chat' 会话（delegation 子会话的
/// 任务标题是内部产物，不该被跨会话投递命中）。
fn resolve_session_target<'a>(
    conv: Option<&'a ConversationRow>,
    chats: &'a [ConversationRow],
    target: &str,
) -> Option<&'a ConversationRow> {
    if let Some(c) = conv {
        return Some(c);
    }
    let mut by_title = chats.iter().filter(|c| c.title == target);
    let first = by_title.next()?;
    if by_title.next().is_some() {
        return None; // 重名歧义：不猜
    }
    Some(first)
}

#[async_trait]
impl McpClient for SendToSessionTool {
    fn name(&self) -> &str {
        "send_message_to_session"
    }

    fn description(&self) -> &str {
        "Send an async message to another conversation's agent. The message is queued in the \
         target's inbox and consumed when that conversation is free (the target agent runs a full \
         turn seeing it). Unlike delegate_to_agent this returns immediately - fire and forget. \
         `target` accepts the conversation id or its exact title (ambiguous titles are rejected; \
         use the id). Returns status: 'held' (waiting for the target user's approval - default), \
         'queued' (auto-consume when the target is free), or 'delivered' (consumption started). \
         Set expect_reply=true to have the target's answer sent back to this conversation \
         automatically."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "Target conversation id or exact title."
                },
                "content": {
                    "type": "string",
                    "description": "Message body (max 8000 chars). Self-contained: the target agent sees only this plus a source header."
                },
                "expect_reply": {
                    "type": "boolean",
                    "description": "If true, the target agent's final answer is delivered back to this conversation automatically. Default false."
                }
            },
            "required": ["target", "content"]
        })
    }

    /// 恒 Always：授权决策点是目标会话的收件三态（hold 扣住 / accept 自动 /
    /// refuse 拒收），不与通用 Confirm 通道纠缠（delegate 同款先例）。
    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "send_message_to_session 必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: RelayArgs = serde_json::from_str(args).map_err(|e| {
            AppError::Validation(format!("send_message_to_session 参数解析失败: {e}"))
        })?;

        let app = ctx.app_handle.clone().ok_or_else(|| {
            AppError::Internal("send_message_to_session 需要 App 上下文（app_handle）".into())
        })?;

        // --- 目标解析（id 精确 → 标题唯一，只认 chat 会话） ---
        let by_id = match repo::conversation::get_by_id(&ctx.pool, &parsed.target).await {
            Ok(c) if c.kind == "chat" => Some(c),
            Ok(_) => {
                return Err(AppError::Validation(format!(
                    "目标会话 '{}' 不是普通会话——委派任务会话不是跨会话通讯单位，请改投其父会话",
                    parsed.target
                )))
            }
            Err(_) => None,
        };
        let chats = repo::conversation::list_all(&ctx.pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.kind == "chat")
            .collect::<Vec<_>>();
        let target = resolve_session_target(by_id.as_ref(), &chats, &parsed.target).ok_or_else(|| {
            AppError::Validation(format!(
                "找不到会话 '{}'——请确认标题完全一致（重名标题须用会话 id），\
                 或请用户提供目标会话 id",
                parsed.target
            ))
        })?;

        // --- 投递方身份快照（来源标注 + 事件归因用；读取失败诚实报错） ---
        let source = source_info(&ctx.pool, ctx).await?;

        let outcome = inbox::deliver(
            &app,
            &ctx.pool,
            &source,
            &target.id,
            &parsed.content,
            parsed.expect_reply,
        )
        .await?;

        // --- 结果 JSON：状态三态 + 目标名 + 队列位（源 agent 据此告知用户） ---
        let note = match outcome.status {
            "held" => Some("消息已进入对方收件箱，等待该会话用户批准后才会被消费（对方默认收件政策为扣留）"),
            "queued" => Some("对方会话正在生成中，消息已排队，将在其空闲时自动消费"),
            _ => None, // delivered：消费已开始，无需多言
        };
        let mut result = serde_json::json!({
            "status": outcome.status,
            "target_title": outcome.target_title,
            "queue_position": outcome.queue_position,
        });
        if let Some(n) = note {
            result["note"] = serde_json::Value::String(n.into());
        }
        Ok(result.to_string())
    }
}

/// 投递方身份快照：本会话标题 + 本 agent 名（消费侧来源标注与事件归因用）。
async fn source_info(pool: &sqlx::SqlitePool, ctx: &ToolContext) -> AppResult<SourceInfo> {
    let conv = repo::conversation::get_by_id(pool, &ctx.conv_id)
        .await
        .map_err(|e| AppError::Internal(format!("读取本会话（{}）失败: {e}", ctx.conv_id)))?;
    let agent = repo::agent::get_by_id(pool, &ctx.agent_id)
        .await
        .map_err(|e| {
            AppError::Internal(format!("读取本 agent（{}）档案失败: {e}", ctx.agent_id))
        })?;
    Ok(SourceInfo {
        conv_id: conv.id,
        conv_title: conv.title,
        agent_id: agent.id,
        agent_name: agent.name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(id: &str, title: &str, kind: &str) -> ConversationRow {
        ConversationRow {
            id: id.into(),
            agent_id: "agent-1".into(),
            title: title.into(),
            pinned: 0,
            created_at: String::new(),
            updated_at: String::new(),
            tools_override: None,
            project_id: None,
            kind: kind.into(),
            initiator_type: None,
            initiator_agent_id: None,
            parent_conversation_id: None,
            inbox_policy: "hold".into(),
        }
    }

    #[test]
    fn resolve_prefers_id_over_title() {
        let by_id = conv("c-9", "同名", "chat");
        let chats = vec![conv("c-1", "同名", "chat")];
        let t = resolve_session_target(Some(&by_id), &chats, "同名").unwrap();
        assert_eq!(t.id, "c-9", "id 命中优先，不进标题匹配");
    }

    #[test]
    fn resolve_falls_back_to_unique_title() {
        let chats = vec![conv("c-1", "主控", "chat"), conv("c-2", "材质", "chat")];
        let t = resolve_session_target(None, &chats, "材质").unwrap();
        assert_eq!(t.id, "c-2");
    }

    #[test]
    fn resolve_ambiguous_title_rejects() {
        let chats = vec![conv("c-1", "写作", "chat"), conv("c-2", "写作", "chat")];
        assert!(
            resolve_session_target(None, &chats, "写作").is_none(),
            "重名不猜，逼调用方用 id"
        );
    }

    #[test]
    fn resolve_unknown_returns_none() {
        let chats = vec![conv("c-1", "主控", "chat")];
        assert!(resolve_session_target(None, &chats, "不存在").is_none());
    }
}
