//! @ 引用展开 — Reference 块 → 快照 Text 块（与 attachments.rs 同模式的物化层）。
//!
//! 前端把用户在输入框 @ 的对象（会话 / agent / 消息）作为
//! [`ContentBlock::Reference`] 块传入；本模块在 send_message 入口
//! （附件物化之后、persist_blocks 落库快照 clone 之前）为每个 Reference
//! 块紧随其后插入一个展开 Text 块：
//!
//! - Reference 块本身保留落库 → 前端渲染引用卡片（同 Attachment 的 UI 卡模式）
//! - 展开 Text 块是**发送时刻的快照** → 落库 / session_events / derive / 回放
//!   全走现有通道，**append-only 内核零特例**；引用目标后续变化（会话继续、
//!   被删）不影响已发送引用的保真回放
//!
//! 失效降级（确定性兜底）：目标查不到 / 跨会话引消息 → 展开
//! `[引用已失效：display]`，绝不阻塞整条消息。
//!
//! 上限全 L1 默认（不进配置）：会话快照 8000 字符（头 2 轮 + 尾 8 轮）、
//! 消息快照 4000 字符（assistant 组 ≤10 条）、agent 身份卡 ~500 字符。

use sqlx::SqlitePool;

use crate::db::repo;
use crate::infra::protocol::ContentBlock;

/// 会话快照总字符上限
const CONVERSATION_CHAR_CAP: usize = 8_000;
/// 会话快照保留的头部/尾部轮数
const CONVERSATION_HEAD_TURNS: usize = 2;
const CONVERSATION_TAIL_TURNS: usize = 8;
/// 会话快照的消息读取窗口（尾部最近 N 条；超出窗口的更早轮不进快照）
const CONVERSATION_MSG_WINDOW: i64 = 200;
/// 消息（assistant 组）快照总字符上限
const MESSAGE_CHAR_CAP: usize = 4_000;
/// assistant 组快照最多并入的消息条数（前端「一次回答」组的后端对齐语义）
const GROUP_MAX_MESSAGES: usize = 10;

/// 遍历 `blocks`，为每个 Reference 块在其后插入展开 Text 块（无 Reference 时原样返回）。
pub(crate) async fn materialize_reference_blocks(
    pool: &SqlitePool,
    current_conv_id: &str,
    blocks: Vec<ContentBlock>,
) -> Vec<ContentBlock> {
    let mut out = Vec::with_capacity(blocks.len());
    let mut touched = false;
    for b in blocks {
        let expansion = if let ContentBlock::Reference {
            ref_kind,
            target_id,
            display,
        } = &b
        {
            touched = true;
            Some(expand_one(pool, current_conv_id, ref_kind, target_id, display).await)
        } else {
            None
        };
        out.push(b);
        if let Some(text) = expansion {
            out.push(ContentBlock::Text { text });
        }
    }
    if !touched {
        // 无引用：返回原 Vec（保住调用方 clone 语义之外的零开销路径）
        return out;
    }
    out
}

/// 单个引用的展开文本（含失效降级；永不 Err）。
async fn expand_one(
    pool: &SqlitePool,
    current_conv_id: &str,
    ref_kind: &str,
    target_id: &str,
    display: &str,
) -> String {
    let snapshot = match ref_kind {
        "conversation" => expand_conversation(pool, target_id).await,
        "agent" => expand_agent(pool, target_id).await,
        "message" => expand_message(pool, current_conv_id, target_id).await,
        _ => None, // 未知类型（前端版本不匹配）：按失效处理
    };
    snapshot.unwrap_or_else(|| format!("[引用已失效：{display}]"))
}

// =========================================================================
// @会话：头部 2 轮 + 尾部 8 轮节选
// =========================================================================

async fn expand_conversation(pool: &SqlitePool, conv_id: &str) -> Option<String> {
    let conv = repo::conversation::get_by_id(pool, conv_id).await.ok()?;
    let agent_name = repo::agent::get_by_id(pool, &conv.agent_id)
        .await
        .ok()
        .map(|a| a.name)
        .unwrap_or_else(|| conv.agent_id.clone());

    // 尾部窗口（ASC）：超长会话的更早轮不在快照内（引用语义 = 看最近结论）
    let msgs =
        repo::message::list_by_conversation(pool, conv_id, Some(CONVERSATION_MSG_WINDOW), None)
            .await
            .ok()?;
    if msgs.is_empty() {
        return None; // 会话存在但无消息：按失效处理（没有可引内容）
    }

    // 轮 = 真实 user 消息（占位词表与 list_turn_anchors 对齐：content 空且
    // blocks 含 tool_result / blocks 空的行不是锚）
    let turns: Vec<usize> = msgs
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "user" && !is_placeholder_user(m))
        .map(|(i, _)| i)
        .collect();
    let total_turns = turns.len();

    // 头尾轮区间（窗口内）：全量 ≤ 头+尾 轮时整段保留
    let keep: Vec<usize> = if total_turns <= CONVERSATION_HEAD_TURNS + CONVERSATION_TAIL_TURNS {
        (0..msgs.len()).collect()
    } else {
        let head_end = turns[CONVERSATION_HEAD_TURNS]; // 第 3 轮起点 = 头部保留终点
        let tail_start = turns[total_turns - CONVERSATION_TAIL_TURNS];
        let mut v: Vec<usize> = (0..head_end).collect();
        v.extend(tail_start..msgs.len());
        v
    };
    let omitted = total_turns.saturating_sub(CONVERSATION_HEAD_TURNS + CONVERSATION_TAIL_TURNS);

    let mut out = String::with_capacity(1024);
    out.push_str(&format!(
        "<referenced_conversation id=\"{conv_id}\">\n会话「{title}」（agent：{agent_name}，共 {total_turns} 轮）。以下为节选快照：\n",
        title = if conv.title.is_empty() { "（未命名）" } else { &conv.title },
    ));
    let mut used = out.len();
    let mut prev_kept = true;
    for (i, m) in msgs.iter().enumerate() {
        if !keep.contains(&i) {
            if prev_kept {
                out.push_str(&format!(
                    "…（中间省略 {omitted} 轮，如需更早内容请让用户在源会话中查阅）\n"
                ));
                prev_kept = false;
            }
            continue;
        }
        let line = render_message_line(m);
        used += line.len();
        if used > CONVERSATION_CHAR_CAP {
            out.push_str("…（已达快照长度上限，后续内容省略）\n");
            break;
        }
        out.push_str(&line);
    }
    out.push_str("</referenced_conversation>");
    Some(out)
}

// =========================================================================
// @agent：身份卡（轻语义——注入身份引导委派，不含 system_prompt 人设全文）
// =========================================================================

async fn expand_agent(pool: &SqlitePool, agent_id: &str) -> Option<String> {
    let a = repo::agent::get_by_id(pool, agent_id).await.ok()?;
    let desc = if a.description.is_empty() {
        "（无职责描述）".to_string()
    } else {
        truncate_chars(&a.description, 400)
    };
    Some(format!(
        "<referenced_agent id=\"{agent_id}\">\n{desc}（provider: {provider}，model: {model}）。用户在消息中提及此 agent：若适合，可通过 delegate_to_agent 把相关子任务委派给它。\n</referenced_agent>",
        provider = a.provider,
        model = a.model,
    ))
}

// =========================================================================
// @消息：user 单条 / assistant 连续组（= 前端「一次回答」组语义）
// =========================================================================

async fn expand_message(
    pool: &SqlitePool,
    current_conv_id: &str,
    message_id: &str,
) -> Option<String> {
    let m = repo::message::find_by_id(pool, message_id).await.ok()??;
    // 安全：消息引用限当前会话（前端入口本就只给当前会话，此处后端兜底）
    if m.conversation_id != current_conv_id {
        return None;
    }

    let mut out = String::with_capacity(512);
    out.push_str(&format!(
        "<referenced_message id=\"{message_id}\">\n"
    ));

    if m.role == "assistant" {
        // 组语义：该消息起向后连续 assistant 直到 role 变化（窗口 ≤200 条内；
        // 引用点更早时降级为单条）。前端 assistant 组 footer 引用组首 id，
        // 此规则天然展开「一次完整回答」。
        let msgs = repo::message::list_by_conversation(
            pool,
            current_conv_id,
            Some(CONVERSATION_MSG_WINDOW),
            None,
        )
        .await
        .ok()?;
        let start = msgs.iter().position(|x| x.id == message_id);
        let mut used = 0usize;
        match start {
            Some(i) => {
                for m in msgs.iter().skip(i).take(GROUP_MAX_MESSAGES) {
                    if m.role != "assistant" {
                        break;
                    }
                    let line = render_message_line(m);
                    used += line.len();
                    if used > MESSAGE_CHAR_CAP {
                        out.push_str("…（已达快照长度上限）\n");
                        break;
                    }
                    out.push_str(&line);
                }
            }
            None => out.push_str(&render_message_line(&m)), // 窗口外更早消息：单条
        }
    } else {
        out.push_str(&render_message_line(&m));
    }

    out.push_str("</referenced_message>");
    Some(out)
}

// =========================================================================
// 渲染辅助
// =========================================================================

/// 占位 user 行（词表对齐 list_turn_anchors：content 空且 blocks 空/含 tool_result）
fn is_placeholder_user(m: &crate::db::models::MessageRow) -> bool {
    if !m.content.trim().is_empty() {
        return false;
    }
    let blocks = m.content_blocks.trim();
    blocks.is_empty() || blocks == "[]" || blocks.contains("\"type\":\"tool_result\"")
}

/// 单条消息 → `[role]: 正文` 一行。正文来自 blocks 全序列渲染
/// （Text 原文、图片/附件降级占位——**绝不塞 base64**、工具/引用块简短占位），
/// blocks 无可渲染内容时兜底 content。
fn render_message_line(m: &crate::db::models::MessageRow) -> String {
    let parsed = crate::context::history::parse_content_blocks(&m.content_blocks);
    let mut body = String::new();
    for b in &parsed {
        match b {
            ContentBlock::Text { text } => {
                if !text.trim().is_empty() {
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    body.push_str(text);
                }
            }
            // 用户 mid-turn 补充需求：被引消息可能带图片/文档——降级占位符，
            // 不内联 base64（快照是文本，视觉内容由源消息承载）
            ContentBlock::Image { .. } => body.push_str(" [图片]"),
            ContentBlock::Attachment { name, kind, .. } => {
                body.push_str(&format!(" [附件：{name}（{kind}）]"))
            }
            ContentBlock::ToolUse { name, .. } => {
                body.push_str(&format!(" [调用工具 {name}]"))
            }
            ContentBlock::ToolResult { content, .. } => {
                body.push_str(&format!(" [工具结果：{}]", truncate_chars(content, 200)))
            }
            ContentBlock::Thinking { .. } => {} // 内部推理不进引用快照
            ContentBlock::Reference { display, .. } => {
                body.push_str(&format!(" [引用：{display}]"))
            }
        }
    }
    if body.trim().is_empty() {
        body = m.content.clone();
    }
    format!("[{}]: {}\n", m.role, body.trim_end())
}

/// 按字符数截断（中文安全：char 边界，不用 String::truncate）
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

// =========================================================================
// 单元测试（纯函数部分：render_message_line / truncate_chars / 占位词表）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str, blocks: &str) -> crate::db::models::MessageRow {
        crate::db::models::MessageRow {
            id: "m1".into(),
            conversation_id: "c1".into(),
            role: role.into(),
            content: content.into(),
            content_blocks: blocks.into(),
            token_count: None,
            error: None,
            created_at: "2026-08-17 10:00:00".into(),
            rowid: 1,
            summary_id: None,
            model: None,
            source_seq: None,
        }
    }

    #[test]
    fn render_message_line_text_and_content_fallback() {
        let m = msg("user", "你好", "[]");
        assert_eq!(render_message_line(&m), "[user]: 你好\n");
    }

    #[test]
    fn render_message_line_image_and_attachment_become_placeholders() {
        // 用户补充需求：被引消息带图片/文档 → 占位符，绝不内联 base64
        let m = msg(
            "user",
            "",
            r#"[{"type":"text","text":"看一下这两份"},{"type":"image","data":"iVBORw0KG...","media_type":"image/png"},{"type":"attachment","name":"report.docx","kind":"docx","size":12345}]"#,
        );
        let line = render_message_line(&m);
        assert!(line.contains("看一下这两份"));
        assert!(line.contains("[图片]"));
        assert!(line.contains("[附件：report.docx（docx）]"));
        assert!(!line.contains("iVBORw0KG")); // base64 绝不进快照
    }

    #[test]
    fn render_message_line_tool_and_reference_blocks() {
        let m = msg(
            "assistant",
            "",
            r#"[{"type":"tool_use","id":"t1","name":"run_command","input":"{}"},{"type":"tool_result","tool_use_id":"t1","content":"ok"},{"type":"reference","ref_kind":"conversation","target_id":"c9","display":"设计#1234"},{"type":"thinking","thinking":"内部"}]"#,
        );
        let line = render_message_line(&m);
        assert!(line.contains("[调用工具 run_command]"));
        assert!(line.contains("[工具结果：ok]"));
        assert!(line.contains("[引用：设计#1234]"));
        assert!(!line.contains("内部")); // thinking 不进快照
    }

    #[test]
    fn placeholder_user_detection() {
        assert!(is_placeholder_user(&msg("user", "", "[]")));
        assert!(is_placeholder_user(&msg(
            "user",
            "",
            r#"[{"type":"tool_result","tool_use_id":"t1","content":"x"}]"#
        )));
        // 用户正文里粘贴含字面量的 JSON 不是占位（词表对齐 list_turn_anchors）
        assert!(!is_placeholder_user(&msg(
            "user",
            "粘贴了日志",
            r#"[{"type":"text","text":"\"type\":\"tool_result\""}]"#
        )));
        assert!(!is_placeholder_user(&msg("user", "正常", "[]")));
    }

    #[test]
    fn truncate_chars_cjk_safe() {
        assert_eq!(truncate_chars("你好世界", 2), "你好…");
        assert_eq!(truncate_chars("ab", 5), "ab");
    }

    // ---- DB 层 e2e（in-memory SQLite，地基同 context/memory.rs 测试）----

    async fn test_pool() -> SqlitePool {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        use std::str::FromStr;
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        pool
    }

    async fn seed_agent(pool: &SqlitePool, id: &str, name: &str, description: &str) {
        repo::agent::create(
            pool,
            &crate::db::models::NewAgent {
                id: id.into(),
                name: name.into(),
                provider: "openai".into(),
                model: "test-model".into(),
                system_prompt: String::new(),
                api_key: String::new(),
                base_url: None,
                temperature: 0.7,
                max_tokens: 4096,
                extra_params: None,
                sort_order: 0,
                cache_prompt: true,
                supports_vision: false,
                max_history_messages: None,
                context_window: None,
                enabled_tools: None,
                workspace_path: None,
            },
            id,
            "ref-slot",
        )
        .await
        .expect("seed agent");
        // NewAgent 不含 description（M2-1 经 UPDATE 维护）；身份卡展开要读它
        sqlx::query("UPDATE agents SET description = ? WHERE id = ?")
            .bind(description)
            .bind(id)
            .execute(pool)
            .await
            .expect("seed agent description");
    }

    async fn seed_conv(pool: &SqlitePool, id: &str, title: &str, agent_id: &str) {
        repo::conversation::create(
            pool,
            id,
            &crate::db::models::NewConversation {
                agent_id: agent_id.into(),
                title: Some(title.into()),
                project_id: None,
                kind: None,
                initiator_agent_id: None,
                parent_conversation_id: None,
            },
        )
        .await
        .expect("seed conv");
    }

    async fn seed_msg(pool: &SqlitePool, id: &str, conv: &str, role: &str, content: &str) {
        repo::message::create(
            pool,
            id,
            &crate::db::models::NewMessage {
                conversation_id: conv.into(),
                role: role.into(),
                content: content.into(),
                token_count: None,
                error: None,
                model: None,
            },
        )
        .await
        .expect("seed msg");
    }

    #[tokio::test]
    async fn expand_agent_renders_identity_card() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "审查员", "负责代码审查与安全把关").await;
        let text = expand_agent(&pool, "a1").await.expect("展开成功");
        assert!(text.contains("负责代码审查与安全把关"));
        assert!(text.contains("provider: openai"));
        assert!(text.contains("delegate_to_agent")); // 轻语义引导
        assert!(!text.contains("system_prompt")); // 不含人设全文

        // 失效 agent → None（调用方降级占位）
        assert!(expand_agent(&pool, "nope").await.is_none());
    }

    #[tokio::test]
    async fn expand_conversation_head_tail_turns() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "助手", "").await;
        seed_conv(&pool, "c1", "设计讨论", "a1").await;
        // 12 轮（> 头2+尾8=10）→ 中段省略
        for t in 1..=12 {
            seed_msg(&pool, &format!("u{t}"), "c1", "user", &format!("第{t}个问题")).await;
            seed_msg(&pool, &format!("a{t}"), "c1", "assistant", &format!("第{t}个回答")).await;
        }
        let text = expand_conversation(&pool, "c1").await.expect("展开成功");
        assert!(text.contains("会话「设计讨论」"));
        assert!(text.contains("agent：助手"));
        assert!(text.contains("共 12 轮"));
        assert!(text.contains("第1个问题")); // 头部保留
        assert!(text.contains("第2个回答"));
        assert!(!text.contains("第3个问题")); // 中段省略（第 3、4 轮）
        assert!(text.contains("第12个回答")); // 尾部保留
        assert!(text.contains("中间省略"));

        // 不存在的会话 / 空会话 → None
        assert!(expand_conversation(&pool, "nope").await.is_none());
        seed_conv(&pool, "c2", "空", "a1").await;
        assert!(expand_conversation(&pool, "c2").await.is_none());
    }

    #[tokio::test]
    async fn expand_message_user_single_and_assistant_group() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "助手", "").await;
        seed_conv(&pool, "c1", "会话", "a1").await;
        seed_msg(&pool, "u1", "c1", "user", "帮我看看").await;
        seed_msg(&pool, "s1", "c1", "assistant", "第一段").await;
        seed_msg(&pool, "s2", "c1", "assistant", "第二段").await;
        seed_msg(&pool, "u2", "c1", "user", "再来").await;

        // user 单条
        let text = expand_message(&pool, "c1", "u1").await.expect("展开");
        assert!(text.contains("[user]: 帮我看看"));
        assert!(!text.contains("第一段"));

        // assistant 组：从 s1 起连续到 role 变化（s1+s2，不含 u2）
        let text = expand_message(&pool, "c1", "s1").await.expect("展开");
        assert!(text.contains("[assistant]: 第一段"));
        assert!(text.contains("[assistant]: 第二段"));
        assert!(!text.contains("再来"));

        // 跨会话引用 → None（后端兜底，前端入口本就只给当前会话）
        seed_conv(&pool, "c9", "别的", "a1").await;
        assert!(expand_message(&pool, "c9", "u1").await.is_none());
        // 不存在 → None
        assert!(expand_message(&pool, "c1", "nope").await.is_none());
    }

    #[tokio::test]
    async fn materialize_inserts_snapshot_after_each_reference() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "助手", "").await;
        seed_conv(&pool, "c1", "会话", "a1").await;
        seed_msg(&pool, "u1", "c1", "user", "旧问题").await;

        let blocks = vec![
            ContentBlock::text("看看这个"),
            ContentBlock::reference("message", "u1", "消息#1234"),
            ContentBlock::reference("conversation", "nope", "幽灵#0000"), // 失效
        ];
        let out = materialize_reference_blocks(&pool, "c1", blocks).await;
        // text + ref + 展开 + ref + 降级占位 = 5 块，顺序：每个 ref 后紧跟其展开
        assert_eq!(out.len(), 5);
        assert!(matches!(&out[1], ContentBlock::Reference { .. }));
        let snap = out[2].as_text().expect("展开为 Text");
        assert!(snap.contains("<referenced_message"));
        assert!(snap.contains("旧问题"));
        let dead = out[4].as_text().expect("降级为 Text");
        assert!(dead.contains("[引用已失效：幽灵#0000]"));
    }

    #[tokio::test]
    async fn materialize_no_references_returns_blocks_unchanged() {
        let pool = test_pool().await;
        let blocks = vec![ContentBlock::text("普通消息")];
        let out = materialize_reference_blocks(&pool, "c1", blocks).await;
        assert_eq!(out.len(), 1);
    }
}
