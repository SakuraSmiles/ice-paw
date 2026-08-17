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
//!
//! Image 引用（S1 阶段 3）：blocks 保持 payload 原始形态（可能含 `ImageRef`）。
//! 消费前必须经 [`hydrate_image_refs`] 还原（字节只在 messages 行）或
//! [`DerivedMessage::to_content_blocks`] 降级——ref 形态不得直接进 LLM / 对账平面。

use crate::db::models::SessionEventRow;
use crate::harness::event_log::{
    AssistantMessagePayload, PayloadBlock, ToolResultMessagePayload, UserMessagePayload,
};
use crate::infra::protocol::ContentBlock;

/// Image 引用水合未命中（行已删 / content_blocks 变形）时的降级占位文本。
/// 诚实标注字节不可恢复，不让 ref 静默消失。
pub const IMAGE_UNRECOVERABLE_MARKER: &str = "[图片内容已不可恢复]";

/// 回放出的一条消息（与 legacy 行提取结果同构，reconcile 按 message_id 对齐）。
#[derive(Debug, Clone, PartialEq)]
pub struct DerivedMessage {
    pub message_id: String,
    /// "user" | "assistant"（回放不产 system 行；摘要行由 legacy 侧跳过）
    pub role: String,
    /// 与行 `content` 列直比（user=pre-materialize 文本 / tool_result 恒空 /
    /// assistant=全文）。不与 blocks 交叉推导。
    pub content: String,
    /// payload 原始形态（含 ImageRef 时**未水合**）。进对账 / LLM 视图前必须
    /// 先 [`hydrate_image_refs`]，或经 [`Self::to_content_blocks`] 降级提取。
    pub blocks: Vec<PayloadBlock>,
    /// 所属 turn（= user_msg_id）；reconcile 按 turn 分组截断用
    pub turn_id: Option<String>,
    /// 首现事件 seq（消息在回放流中的位置锚点，ORDER 对齐用）
    pub first_seq: i64,
    /// 最后一次 supersede 的事件 seq（内容版本）
    pub last_seq: i64,
}

impl DerivedMessage {
    /// 残留 ref 降级提取（防泄漏最后闸）：Full 原样、ImageRef → Text marker。
    ///
    /// 正常流程不应走到这里（读侧先 `hydrate_image_refs`）；仅供「跳过水合
    /// 直接消费 blocks」的防御性路径（如 serialize 兜底），保证 ref 形态
    /// **不可能**以非 Text 形态进入 LLM / 对账平面。
    pub fn to_content_blocks(&self) -> Vec<ContentBlock> {
        self.blocks
            .iter()
            .map(|b| match b {
                PayloadBlock::Full(c) => c.clone(),
                PayloadBlock::ImageRef { .. } => ContentBlock::Text {
                    text: IMAGE_UNRECOVERABLE_MARKER.to_string(),
                },
            })
            .collect()
    }
}

/// Image 引用水合：把 `ImageRef` 就地还原为所指行 `content_blocks` 下标的完整块。
///
/// 纯同步（derive 保持无 IO）：`resolve(message_id, block_index)` 由调用方闭包
/// 提供（查 messages 行 → parse → 取下标）。命中（且确为 Image 块）→ `Full(img)`；
/// 未命中 / 下标越界 / 非 Image → 降级 `Text(IMAGE_UNRECOVERABLE_MARKER)`。
///
/// 返回未命中数（正常数据为 0；非零即行侧数据债，调用方记 warn/对账上报）。
pub fn hydrate_image_refs(
    messages: &mut [DerivedMessage],
    resolve: &impl Fn(&str, usize) -> Option<ContentBlock>,
) -> usize {
    let mut missed = 0usize;
    for msg in messages.iter_mut() {
        for b in msg.blocks.iter_mut() {
            if let PayloadBlock::ImageRef {
                message_id,
                block_index,
                ..
            } = b
            {
                let hydrated = match resolve(message_id, *block_index) {
                    Some(c @ ContentBlock::Image { .. }) => PayloadBlock::Full(c),
                    _ => {
                        missed += 1;
                        PayloadBlock::Full(ContentBlock::Text {
                            text: IMAGE_UNRECOVERABLE_MARKER.to_string(),
                        })
                    }
                };
                *b = hydrated;
            }
        }
    }
    missed
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
        msg.blocks = vec![PayloadBlock::Full(ContentBlock::Text {
            text: msg.content.clone(),
        })];
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
        assert_eq!(
            out.messages[0].to_content_blocks(),
            vec![text_block("读文件")]
        );
        assert_eq!(out.messages[1].role, "assistant");
        assert_eq!(
            out.messages[1].to_content_blocks()[0].clone(),
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
        assert_eq!(a.to_content_blocks(), vec![text_block("前半段后半段")]);
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
        assert_eq!(
            out.messages[0].to_content_blocks(),
            vec![text_block("纯文本")]
        );
    }

    /// Image 引用水合：ref（事件 payload）→ 所指行 content_blocks 下标的完整块。
    /// v1 内联事件（Full(Image)）原样保留，不进 resolver。
    #[test]
    fn hydrate_resolves_refs_and_keeps_inline_images() {
        let img = || ContentBlock::Image {
            data: "QUJD".into(),
            media_type: "image/png".into(),
        };
        let events = vec![
            // v2 形态：text + ref(idx1)
            row(
                1,
                "user_message",
                Some("m-u1"),
                r#"{"v":2,"content":"看图","blocks":[{"type":"text","text":"看图"},{"type":"image_ref","message_id":"m-u1","block_index":1}]}"#.into(),
            ),
            // v1 形态：内联 Image 原样
            row(
                2,
                "user_message",
                Some("m-u2"),
                r#"{"v":1,"content":"旧图","blocks":[{"type":"image","data":"WFla","media_type":"image/png"}]}"#.into(),
            ),
        ];
        let mut out = derive_history(&events);

        // resolver 只会被 m-u1 的 ref 调用（v1 内联零查询）。
        // hydrate 取 &impl Fn（非 FnMut）——记录调用用 RefCell 内部可变。
        let queried = std::cell::RefCell::new(Vec::new());
        let missed = hydrate_image_refs(&mut out.messages, &|mid, idx| {
            queried.borrow_mut().push((mid.to_string(), idx));
            assert_eq!(mid, "m-u1");
            Some(img())
        });
        assert_eq!(missed, 0);
        assert_eq!(queried.into_inner(), vec![("m-u1".to_string(), 1)]);

        assert_eq!(
            out.messages[0].to_content_blocks(),
            vec![text_block("看图"), img()],
            "ref 应水合为完整 Image 块"
        );
        // v1 内联 Image 原样保留（data 未被 resolver 的返回值覆盖）
        assert_eq!(
            out.messages[1].to_content_blocks(),
            vec![ContentBlock::Image {
                data: "WFla".into(),
                media_type: "image/png".into(),
            }]
        );
    }

    /// 水合未命中（行已删 / 下标越界 / 行内该下标非 Image）→ 降级 marker；
    /// `to_content_blocks` 对残留 ref 同款降级（防泄漏最后闸）。
    #[test]
    fn hydrate_miss_degrades_to_marker_and_to_content_blocks_gates() {
        let events = vec![
            row(
                1,
                "user_message",
                Some("m-u1"),
                r#"{"v":2,"content":"q","blocks":[{"type":"image_ref","message_id":"m-gone","block_index":0}]}"#.into(),
            ),
            row(
                2,
                "user_message",
                Some("m-u2"),
                r#"{"v":2,"content":"q","blocks":[{"type":"image_ref","message_id":"m-bad","block_index":3}]}"#.into(),
            ),
        ];
        let mut out = derive_history(&events);

        // resolver：m-gone 无行（None）；m-bad 行存在但下标 3 是 Text 非 Image
        let missed = hydrate_image_refs(&mut out.messages, &|mid, _| {
            match mid {
                "m-gone" => None,
                "m-bad" => Some(text_block("不是图片")),
                _ => unreachable!(),
            }
        });
        assert_eq!(missed, 2, "两种未命中形态都应计数");
        assert_eq!(
            out.messages[0].to_content_blocks(),
            vec![text_block(IMAGE_UNRECOVERABLE_MARKER)]
        );
        // 非 Image 命中也降级（ref 语义只能指向 Image）
        assert_eq!(
            out.messages[1].to_content_blocks(),
            vec![text_block(IMAGE_UNRECOVERABLE_MARKER)]
        );

        // to_content_blocks 对**未水合**的残留 ref 同样降级（跳过水合的防御路径）
        let out2 = derive_history(&events);
        assert_eq!(
            out2.messages[0].to_content_blocks(),
            vec![text_block(IMAGE_UNRECOVERABLE_MARKER)],
            "残留 ref 不得以非 Text 形态流出"
        );
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
