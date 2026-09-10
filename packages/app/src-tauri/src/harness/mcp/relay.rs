//! `send_message_to_session` / `list_conversations`——MA-3 跨会话通讯的 agent
//! 入口（投递 + 寻址发现）。
//!
//! 会话 A 的 agent 向会话 B **异步**投递消息（与 [`super::delegate`] 的同步
//! 阻塞形态互补不替代）：工具立即返回（`held`/`queued`/`delivered`），目标
//! 会话排队、空闲时消费——由 [`crate::harness::inbox`] 引擎承载（收件三态/
//! 护栏/expect_reply 回投全在彼处，本文件只是薄壳：目标解析 + 调用 + 结果
//! JSON 化）。
//!
//! `list_conversations` 是**寻址发现**（2026-09-10 拍板）：agent 查全部会话的
//! 概述元信息（id/标题/所属 agent/所属项目/更新时间），解决「投递必须点对点
//! 但 agent 查不到地址、用户只能拿 @会话 兼职报地址」的语义混乱——@ 的语义
//! 恒为「向内引用」（快照注入），不兼职寻址。**可见性 ≠ 写权限**：列表全量
//! 可见（知道无害），投递层拦同项目边界（跨项目/散落会话看得到但投不了，
//! 报错指路）。
//!
//! ## 注册边界（与 delegate 同款）
//!
//! 组装期按 `conv.kind == "chat"` 注册（session_runner）——delegation 子会话
//! 拿不到本工具族，防子会话侧信道绕过委派深度护栏；PLATFORM_TOOLS 白名单
//! 补入（enabled_tools 收窄不断跨会话通讯能力）。
//!
//! ## 授权
//!
//! `AuthorizationLevel::Always`（通用授权层不弹）——授权决策点是**收件三态**
//! （目标会话的 inbox_policy），不是弹卡。

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
         HARD BOUNDARY: the target must belong to the SAME project as the current conversation \
         (scattered conversations without a project cannot send or receive). Use \
         list_conversations to find addressable ids/titles. `target` accepts the conversation id \
         or its exact title (ambiguous titles are rejected; use the id). Returns status: \
         'delivered' (consumption started - default policy auto-consumes), 'queued' (target busy, \
         auto-consume when free), or 'held' (target policy requires user approval). Set \
         expect_reply=true to have the target's answer sent back to this conversation \
         automatically; with it on, the target agent is instructed to answer directly in its \
         turn (do NOT call send_message_to_session to reply - the system delivers the final \
         answer back, calling it yourself would duplicate the reply)."
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
                    "description": "If true, the target agent's final answer is delivered back to this conversation automatically; the target is then told to answer directly (no need for it to call send_message_to_session). Default false."
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
            false,
        )
        .await?;

        // --- 结果 JSON：状态三态 + 目标名 + 队列位（源 agent 据此告知用户） ---
        let note = match outcome.status {
            "held" => Some("消息已进入对方收件箱，等待该会话用户批准后才会被消费（对方收件政策设为需批准）"),
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

/// 投递方身份快照：本会话标题 + 本 agent 名 + 所属项目（来源标注 / 事件
/// 归因 / 项目边界校验用）。
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
        project_id: conv.project_id,
    })
}

// =========================================================================
// list_conversations —— 寻址发现（概述元信息；可见性 ≠ 写权限）
// =========================================================================

/// 会话概述列表上限（按更新时间倒序截断——发现工具够用即止，全量灌入
/// 只会稀释注意力；超限附 truncated 提示诚实边界）。
const OVERVIEW_CAP: usize = 100;

pub struct ListConversationsTool;

/// 概述条目（纯函数产物，可测）：只含寻址所需的元信息——**零内容**。
fn overview_entry(
    conv: &ConversationRow,
    agent_name: Option<&str>,
    project_name: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": conv.id,
        "title": conv.title,
        "agent_name": agent_name.unwrap_or("（agent 已删除）"),
        // 散落会话显式 null——agent 一眼看出「不能投」（投递层会再拦一道）
        "project_id": conv.project_id,
        "project_name": project_name,
        "updated_at": conv.updated_at,
    })
}

/// 组装概述 JSON（纯函数）：kind='chat' 过滤 + 按更新时间倒序 + 截断。
fn conversation_overview_json(
    chats: &[ConversationRow],
    agent_names: &std::collections::HashMap<String, String>,
    project_names: &std::collections::HashMap<String, String>,
) -> serde_json::Value {
    let mut sorted: Vec<&ConversationRow> = chats.iter().filter(|c| c.kind == "chat").collect();
    sorted.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let truncated = sorted.len() > OVERVIEW_CAP;
    let items: Vec<serde_json::Value> = sorted
        .into_iter()
        .take(OVERVIEW_CAP)
        .map(|c| {
            overview_entry(
                c,
                agent_names.get(&c.agent_id).map(String::as_str),
                c.project_id
                    .as_ref()
                    .and_then(|pid| project_names.get(pid).map(String::as_str)),
            )
        })
        .collect();
    let mut v = serde_json::json!({ "conversations": items });
    if truncated {
        v["note"] = serde_json::Value::String(format!(
            "仅显示最近更新的 {OVERVIEW_CAP} 个会话（共 {} 个）",
            chats.iter().filter(|c| c.kind == "chat").count()
        ));
    }
    v
}

#[async_trait]
impl McpClient for ListConversationsTool {
    fn name(&self) -> &str {
        "list_conversations"
    }

    fn description(&self) -> &str {
        "List all conversations with addressing metadata only (id, title, owning agent, project, \
         last-updated time) - NO content. Use this to find the target for \
         send_message_to_session. Cross-session delivery only works between conversations in the \
         SAME project (scattered conversations without a project cannot send or receive) - the \
         list is visible in full, but delivery enforces the project boundary."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    /// 恒 Always：只读概述元信息（无内容），与 send 工具同款授权语义——
    /// 权限闸在投递层（项目边界 + 收件政策），不在发现层。
    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        Err(AppError::Internal(
            "list_conversations 必须通过 execute_with_context 调用".into(),
        ))
    }

    async fn execute_with_context(&self, _args: &str, ctx: &ToolContext) -> AppResult<String> {
        let chats = repo::conversation::list_all(&ctx.pool)
            .await
            .map_err(|e| AppError::Internal(format!("读取会话列表失败: {e}")))?;
        let agent_names: std::collections::HashMap<String, String> =
            repo::agent::list(&ctx.pool)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|a| (a.id, a.name))
                .collect();
        let project_names: std::collections::HashMap<String, String> =
            repo::project::list(&ctx.pool)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|p| (p.id, p.name))
                .collect();
        Ok(conversation_overview_json(&chats, &agent_names, &project_names).to_string())
    }
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
            inbox_policy: "accept".into(),
        }
    }

    fn conv_at(id: &str, title: &str, project: Option<&str>, updated: &str) -> ConversationRow {
        ConversationRow {
            updated_at: updated.into(),
            project_id: project.map(str::to_string),
            ..conv(id, title, "chat")
        }
    }

    fn names(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn overview_filters_delegation_and_sorts_by_recency() {
        let chats = vec![
            conv_at("c-1", "旧", None, "2026-01-01 00:00:00"),
            conv("d-1", "任务", "delegation"),
            conv_at("c-2", "新", None, "2026-09-01 00:00:00"),
        ];
        let v = conversation_overview_json(&chats, &names(&[("agent-1", "甲")]), &names(&[]));
        let items = v["conversations"].as_array().unwrap();
        assert_eq!(items.len(), 2, "delegation 子会话不入列表");
        assert_eq!(items[0]["id"], "c-2", "按更新时间倒序");
        assert_eq!(items[0]["agent_name"], "甲");
        assert!(items[0]["project_id"].is_null(), "散落会话 project 显式 null");
        assert!(v.get("note").is_none(), "未截断无 note");
    }

    #[test]
    fn overview_carries_project_and_truncates_with_note() {
        let mut chats: Vec<ConversationRow> = (0..OVERVIEW_CAP + 5)
            .map(|i| conv_at(&format!("c-{i}"), &format!("会话{i}"), Some("p-1"), &format!("2026-09-{i:02} 00:00:00")))
            .collect();
        chats.push(conv("d-9", "子任务", "delegation"));
        let v = conversation_overview_json(
            &chats,
            &names(&[("agent-1", "甲")]),
            &names(&[("p-1", "主线项目")]),
        );
        let items = v["conversations"].as_array().unwrap();
        assert_eq!(items.len(), OVERVIEW_CAP, "截断到上限");
        assert_eq!(items[0]["project_name"], "主线项目");
        assert_eq!(items[0]["project_id"], "p-1");
        let note = v["note"].as_str().unwrap();
        assert!(note.contains("仅显示"), "截断诚实披露: {note}");
        assert!(note.contains(&format!("{}", OVERVIEW_CAP + 5)), "总数计入 chat 会话（delegation 不计）: {note}");
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
