//! session-events derive-on-read 回放器（事件日志 Phase 1）
//!
//! 从 append-only `session_events` 回放出「行级原始形态」的消息流，与 legacy
//! messages 表提取（reconcile.rs 的 A 侧，语义同 `context::history` 的输入前
//! 形态）同构对账。**纯函数无 IO**——Phase 2 切唯一真相源时直接复用为读路径内核。
//!
//! 回放规则（与 Phase 0 写入语义一一对应）：
//! - `user_message` → user 消息（blocks 空 → 回退 `[Text(content)]`，同 legacy 提取器）
//! - `tool_result_message` → user 消息（blocks 镜像；行 content 恒空）
//! - `assistant_message` → assistant 消息；**supersede**：同 message_id 多条
//!   （自动续写全文覆写）内容取最后一条、位置取首现
//! - `message_error` / `message_discarded`：错误行空内容 / 占位已删，均不进回放
//! - `turn_context` / `turn_ended` / `modal_adapted` / `hook_injected` /
//!   `attachment_stored` / `summary_*`：非消息行事实，不进回放
//! - 未知 kind（未来词表扩展）与 payload 解析失败：跳过并记入 `issues`
//!   （对账侧作为差异上报，不静默吞）

use crate::db::models::SessionEventRow;
use crate::harness::event_log::{
    AssistantMessagePayload, ToolResultMessagePayload, UserMessagePayload,
};
use crate::infra::protocol::ContentBlock;

/// 回放出的一条消息（与 legacy 行提取结果同构，reconcile 按 message_id 对齐）。
#[derive(Debug, Clone, PartialEq)]
pub struct DerivedMessage {
    pub message_id: String,
    /// "user" | "assistant"（回放不产 system 行；摘要行由 legacy 侧跳过）
    pub role: String,
    /// 与行 `content` 列直比（user=pre-materialize 文本 / tool_result 恒空 /
    /// assistant=全文）。不与 blocks 交叉推导。
    pub content: String,
    pub blocks: Vec<ContentBlock>,
    /// 所属 turn（= user_msg_id）；reconcile 按 turn 分组截断用
    pub turn_id: Option<String>,
    /// 首现事件 seq（消息在回放流中的位置锚点，ORDER 对齐用）
    pub first_seq: i64,
    /// 最后一次 supersede 的事件 seq（内容版本）
    pub last_seq: i64,
}

/// 回放中跳过的事件（payload 解析失败 / 无 message_id / 未知 kind）。
/// 正常数据为零；非零即事件侧异常，对账必上报。
#[derive(Debug, Clone, PartialEq)]
pub struct DeriveIssue {
    pub seq: i64,
    pub kind: String,
    pub reason: String,
}

/// 回放结果：消息流 + 跳过清单。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeriveResult {
    pub messages: Vec<DerivedMessage>,
    pub issues: Vec<DeriveIssue>,
}

/// 回放入口。输入须按 `seq` 升序（`repo::session_event::list_by_session` 即此序）。
pub fn derive_history(events: &[SessionEventRow]) -> DeriveResult {
    let mut result = DeriveResult::default();
    for ev in events {
        let seq = ev.seq;
        match ev.kind.as_str() {
            "user_message" => match parse::<UserMessagePayload>(&ev.payload) {
                Ok(p) => {
                    let mid = require_message_id(ev, &mut result);
                    push_message(
                        &mut result,
                        DerivedMessage {
                            message_id: mid,
                            role: "user".into(),
                            content: p.content,
                            blocks: p.blocks,
                            turn_id: ev.turn_id.clone(),
                            first_seq: seq,
                            last_seq: seq,
                        },
                    );
                }
                Err(e) => result.issues.push(DeriveIssue {
                    seq,
                    kind: ev.kind.clone(),
                    reason: format!("payload 解析失败: {e}"),
                }),
            },
            "tool_result_message" => match parse::<ToolResultMessagePayload>(&ev.payload) {
                Ok(p) => {
                    let mid = require_message_id(ev, &mut result);
                    push_message(
                        &mut result,
                        DerivedMessage {
                            message_id: mid,
                            role: "user".into(),
                            // 行 content 恒空（loop_engine 阶段 F 只写 blocks）
                            content: String::new(),
                            blocks: p.blocks,
                            turn_id: ev.turn_id.clone(),
                            first_seq: seq,
                            last_seq: seq,
                        },
                    );
                }
                Err(e) => result.issues.push(DeriveIssue {
                    seq,
                    kind: ev.kind.clone(),
                    reason: format!("payload 解析失败: {e}"),
                }),
            },
            "assistant_message" => {
                match parse::<AssistantMessagePayload>(&ev.payload) {
                    Ok(p) => {
                        let mid = require_message_id(ev, &mut result);
                        // supersede：同 message_id 已存在 → 原位覆写内容（位置取首现）
                        if let Some(existing) =
                            result.messages.iter_mut().find(|m| m.message_id == mid)
                        {
                            existing.content = p.content;
                            existing.blocks = p.blocks;
                            existing.last_seq = seq;
                        } else {
                            push_message(
                                &mut result,
                                DerivedMessage {
                                    message_id: mid,
                                    role: "assistant".into(),
                                    content: p.content,
                                    blocks: p.blocks,
                                    turn_id: ev.turn_id.clone(),
                                    first_seq: seq,
                                    last_seq: seq,
                                },
                            );
                        }
                    }
                    Err(e) => result.issues.push(DeriveIssue {
                        seq,
                        kind: ev.kind.clone(),
                        reason: format!("payload 解析失败: {e}"),
                    }),
                }
            }
            // 错误行（空内容双方不可见）/ 已删占位 → 不进回放。
            // 其余 kind 均非消息行事实。plan_updated（计划快照）同为非消息行——
            // 不容忍会记 DeriveIssue → reconcile 出 DERIVE_ISSUE → read_route 永久回退 Legacy。
            "message_error" | "message_discarded" | "turn_context" | "turn_ended"
            | "modal_adapted" | "hook_injected" | "attachment_stored" | "summary_created"
            | "summary_updated" | "tool_execution" | "plan_updated" => {}
            other => result.issues.push(DeriveIssue {
                seq,
                kind: other.to_string(),
                reason: "未知 kind（词表扩展后需更新回放器）".into(),
            }),
        }
    }
    result
}

fn parse<T: serde::de::DeserializeOwned>(payload: &str) -> serde_json::Result<T> {
    serde_json::from_str(payload)
}

/// 消息类事件必须有 message_id；缺失属事件侧异常，记 issue 并以空串占位
/// （对账时必命中 CONTENT_MISMATCH/MISSING，不会被静默吞掉）。
fn require_message_id(ev: &SessionEventRow, result: &mut DeriveResult) -> String {
    match &ev.message_id {
        Some(id) => id.clone(),
        None => {
            result.issues.push(DeriveIssue {
                seq: ev.seq,
                kind: ev.kind.clone(),
                reason: "消息类事件缺 message_id 列".into(),
            });
            String::new()
        }
    }
}

fn push_message(result: &mut DeriveResult, mut msg: DerivedMessage) {
    // 与 legacy 提取器同构的空回退：blocks 空 → [Text(content)]（空内容则为 [Text("")]，
    // 与行「content=''+blocks='[]'」的提取结果一致，差异不会被回退规则抹掉）。
    if msg.blocks.is_empty() {
        msg.blocks = vec![ContentBlock::Text {
            text: msg.content.clone(),
        }];
    }
    result.messages.push(msg);
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn row(seq: i64, kind: &str, message_id: Option<&str>, payload: String) -> SessionEventRow {
        SessionEventRow {
            id: seq,
            session_id: "conv".into(),
            seq,
            kind: kind.into(),
            actor: "user".into(),
            turn_id: Some("turn-1".into()),
            message_id: message_id.map(|s| s.to_string()),
            payload,
            created_at: "2026-08-14T00:00:00Z".into(),
        }
    }

    fn text_block(t: &str) -> ContentBlock {
        ContentBlock::Text { text: t.into() }
    }

    #[test]
    fn derive_plain_turn_sequence() {
        let events = vec![
            row(
                1,
                "user_message",
                Some("m-u1"),
                r#"{"v":1,"content":"读文件","blocks":[{"type":"text","text":"读文件"}]}"#.into(),
            ),
            row(
                2,
                "turn_context",
                None,
                r#"{"v":1,"tools_enabled":true}"#.into(),
            ),
            row(
                3,
                "assistant_message",
                Some("m-a1"),
                r#"{"v":1,"content":"","blocks":[{"type":"tool_use","id":"tu_1","name":"read_file","input":"{}"}],"round":0,"continuation":false}"#.into(),
            ),
            row(
                4,
                "tool_execution",
                Some("m-a1"),
                r#"{"v":1,"tool_call_id":"tc","tool_name":"read_file","arguments":"{}","is_error":false,"duration_ms":5}"#.into(),
            ),
            row(
                5,
                "tool_result_message",
                Some("m-r1"),
                r#"{"v":1,"blocks":[{"type":"tool_result","tool_use_id":"tu_1","content":"文件内容","is_error":false}]}"#.into(),
            ),
            row(
                6,
                "assistant_message",
                Some("m-a2"),
                r#"{"v":1,"content":"读到了","blocks":[{"type":"text","text":"读到了"}],"round":1,"continuation":false}"#.into(),
            ),
            row(7, "turn_ended", Some("m-a2"), r#"{"v":1,"termination":"stop","rounds":3}"#.into()),
        ];
        let out = derive_history(&events);
        assert!(out.issues.is_empty(), "issues: {:?}", out.issues);
        assert_eq!(out.messages.len(), 4);
        assert_eq!(out.messages[0].role, "user");
        assert_eq!(out.messages[0].message_id, "m-u1");
        assert_eq!(out.messages[0].blocks, vec![text_block("读文件")]);
        assert_eq!(out.messages[1].role, "assistant");
        assert_eq!(
            out.messages[1].blocks[0].clone(),
            ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "read_file".into(),
                input: "{}".into(),
            }
        );
        // tool_result → user，content 恒空
        assert_eq!(out.messages[2].role, "user");
        assert_eq!(out.messages[2].content, "");
        assert_eq!(out.messages[3].content, "读到了");
    }

    #[test]
    fn derive_supersede_last_wins_position_first() {
        let events = vec![
            row(1, "user_message", Some("m-u1"), r#"{"v":1,"content":"写长文","blocks":[]}"#.into()),
            row(
                2,
                "assistant_message",
                Some("m-a1"),
                r#"{"v":1,"content":"前半段","blocks":[{"type":"text","text":"前半段"}],"round":0,"continuation":true}"#.into(),
            ),
            row(
                3,
                "assistant_message",
                Some("m-a1"),
                r#"{"v":1,"content":"前半段后半段","blocks":[{"type":"text","text":"前半段后半段"}],"round":1,"continuation":true}"#.into(),
            ),
            row(4, "turn_ended", Some("m-a1"), r#"{"v":1,"termination":"stop","rounds":2}"#.into()),
        ];
        let out = derive_history(&events);
        assert_eq!(out.messages.len(), 2, "supersede 不新增消息");
        let a = &out.messages[1];
        assert_eq!(a.content, "前半段后半段");
        assert_eq!(a.blocks, vec![text_block("前半段后半段")]);
        assert_eq!(a.first_seq, 2, "位置取首现");
        assert_eq!(a.last_seq, 3, "内容取最后");
    }

    #[test]
    fn derive_empty_blocks_falls_back_to_text() {
        let events = vec![row(
            1,
            "user_message",
            Some("m-u1"),
            r#"{"v":1,"content":"纯文本","blocks":[]}"#.into(),
        )];
        let out = derive_history(&events);
        assert_eq!(out.messages[0].blocks, vec![text_block("纯文本")]);
    }

    #[test]
    fn derive_error_and_discarded_skipped() {
        let events = vec![
            row(
                1,
                "user_message",
                Some("m-u1"),
                r#"{"v":1,"content":"q","blocks":[]}"#.into(),
            ),
            row(
                2,
                "message_error",
                Some("m-a1"),
                r#"{"v":1,"kind":"Network","error":"refused"}"#.into(),
            ),
            row(
                3,
                "message_discarded",
                Some("m-a2"),
                r#"{"v":1,"reason":"cancel_top_placeholder"}"#.into(),
            ),
            row(
                4,
                "turn_ended",
                None,
                r#"{"v":1,"termination":"abort","rounds":0}"#.into(),
            ),
        ];
        let out = derive_history(&events);
        assert_eq!(out.messages.len(), 1, "error/discarded 不进回放");
        assert!(out.issues.is_empty());
    }

    #[test]
    fn derive_unknown_kind_and_bad_payload_reported() {
        let events = vec![
            row(
                1,
                "user_message",
                Some("m-u1"),
                r#"{"v":1,"content":"q","blocks":[]}"#.into(),
            ),
            row(2, "task_started", None, r#"{"v":1}"#.into()),
            row(3, "assistant_message", Some("m-a1"), r#"NOT JSON"#.into()),
            row(
                4,
                "user_message",
                None,
                r#"{"v":1,"content":"无id","blocks":[]}"#.into(),
            ),
        ];
        let out = derive_history(&events);
        // 未知 kind + 坏 payload + 缺 message_id → 三条 issue
        assert_eq!(out.issues.len(), 3, "issues: {:?}", out.issues);
        assert!(out.issues.iter().any(|i| i.kind == "task_started"));
        assert!(out.issues.iter().any(|i| i.reason.contains("解析失败")));
        assert!(out.issues.iter().any(|i| i.reason.contains("message_id")));
    }

    #[test]
    fn derive_modal_and_hook_events_ignored() {
        let events = vec![
            row(
                1,
                "modal_adapted",
                None,
                r#"{"v":1,"stage":"user_image","mode":"ocr_substitute","items":[]}"#.into(),
            ),
            row(
                2,
                "hook_injected",
                None,
                r#"{"v":1,"point":"before_llm","prompt":"x"}"#.into(),
            ),
            row(
                3,
                "attachment_stored",
                Some("m-u1"),
                r#"{"kind":"bytes","v":1,"items":[]}"#.into(),
            ),
        ];
        let out = derive_history(&events);
        assert!(out.messages.is_empty());
        assert!(out.issues.is_empty());
    }

    /// plan_updated（计划快照）是非消息行事实：须静默跳过。不忍耐会记 DeriveIssue
    /// → reconcile 判 DERIVE_ISSUE → 用过计划的会话永久回退 Legacy 读路径。
    #[test]
    fn derive_plan_updated_ignored() {
        let events = vec![
            row(1, "user_message", Some("m-u1"), r#"{"v":1,"content":"q","blocks":[]}"#.into()),
            row(
                2,
                "plan_updated",
                None,
                r#"{"v":1,"items":[{"text":"评审","status":"in_progress","task_conversation_id":"c1"}]}"#.into(),
            ),
        ];
        let out = derive_history(&events);
        assert_eq!(out.messages.len(), 1, "只有 user 行");
        assert!(out.issues.is_empty(), "issues: {:?}", out.issues);
    }
}
