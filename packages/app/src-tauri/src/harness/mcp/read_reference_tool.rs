//! `read_reference` 工具 —— @引用会话的完整内容按页钻取（P2）。
//!
//! 配合 [`crate::harness::references`] 的压缩快照：会话引用默认只注入压缩视图
//! （摘要/头尾节选），被压缩时快照尾部附本工具提示。agent 需要被省略的细节时
//! 按页读取——「塞多少」的决策从发送时刻移到模型使用时刻（与
//! `read_attachment_page` 的大附件分页同构）。
//!
//! - **越权守卫**：`target_id`（会话 id）必须在当前会话的消息 blocks 里被
//!   Reference 块引用过（instr 精确匹配 `"id"` JSON 值）。不校验则 agent 可
//!   任意翻用户全部会话。
//! - **分页**：全文按消息行贪心装页（~10K 字符/页，永不拆单行），返回
//!   `total_pages` / `has_next`，页号 1-based。
//! - `authorization_level = Always`：读用户自己 @ 进来的会话内容，无需授权
//!   （与 read_attachment_page 同理——内容来源是用户自己的数据）。
//! - system 摘要行跳过（压缩元数据非对话内容，与快照渲染同词表）。

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::db::repo;
use crate::error::{AppError, AppResult};

use super::client::{McpClient, ToolContext};
use super::types::AuthorizationLevel;

/// 单页字符预算（贪心装页：页满即切，永不拆单条消息行）
const PAGE_CHAR_BUDGET: usize = 10_000;
/// 读取窗口（尾部最近 N 条消息；更早内容超出窗口时诚实标注）
const READ_WINDOW: i64 = 500;

pub struct ReadReferenceTool;

#[derive(Deserialize)]
struct ReadReferenceArgs {
    /// 目标会话 ID（来自压缩快照尾部的 read_reference(target_id="...") 提示）
    target_id: String,
    /// 1-based 页号
    #[serde(default = "default_page")]
    page: i64,
}

fn default_page() -> i64 {
    1
}

#[async_trait]
impl McpClient for ReadReferenceTool {
    fn name(&self) -> &str {
        "read_reference"
    }

    fn description(&self) -> &str {
        "Read the full content of a conversation that the user @-referenced in a message. \
         The inline reference snapshot is compressed (summary / head-tail excerpt); \
         use this tool to drill into the omitted turns when you need the details. \
         The compressed snapshot's footer tells you the target_id. \
         page is 1-based; the response includes total_pages and has_next."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "target_id": {
                    "type": "string",
                    "description": "The conversation id given in the snapshot footer's read_reference(target_id=\"...\") note."
                },
                "page": {
                    "type": "integer",
                    "description": "1-based page number (within 1..=total_pages).",
                    "default": 1
                }
            },
            "required": ["target_id"]
        })
    }

    fn authorization_level(&self) -> AuthorizationLevel {
        AuthorizationLevel::Always
    }

    async fn execute(&self, _args: &str) -> AppResult<String> {
        // 需要 conv_id 上下文做越权守卫，走 execute_with_context。
        Err(AppError::Internal(
            "read_reference 必须通过 execute_with_context 调用（需要 conv_id 上下文）".into(),
        ))
    }

    async fn execute_with_context(&self, args: &str, ctx: &ToolContext) -> AppResult<String> {
        let parsed: ReadReferenceArgs = serde_json::from_str(args)
            .map_err(|e| AppError::Validation(format!("read_reference 参数解析失败: {e}")))?;
        if parsed.page < 1 {
            return Err(AppError::Validation(format!(
                "page 必须 ≥ 1（1-based），收到 {}",
                parsed.page
            )));
        }

        // 越权守卫：目标会话必须被当前会话的某条消息引用过（Reference 块的
        // target_id 落在 content_blocks JSON 里；instr 匹配带引号的完整 id，
        // 误命中概率仅剩 UUID 子串巧合，且本工具读的本来就是用户自己的数据）
        let needle = format!("\"{}\"", parsed.target_id);
        let referenced: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM messages
               WHERE conversation_id = ?1 AND instr(content_blocks, ?2) > 0)",
        )
        .bind(&ctx.conv_id)
        .bind(&needle)
        .fetch_one(&ctx.pool)
        .await
        .unwrap_or(false);
        if !referenced {
            return Err(AppError::Validation(
                "该会话未被当前对话引用过（请让用户先在消息里 @ 它）".into(),
            ));
        }

        let conv = repo::conversation::get_by_id(&ctx.pool, &parsed.target_id)
            .await
            .map_err(|_| AppError::Validation("目标会话不存在（可能已被删除）".into()))?;
        let agent_name = repo::agent::get_by_id(&ctx.pool, &conv.agent_id)
            .await
            .ok()
            .map(|a| a.name)
            .unwrap_or_else(|| conv.agent_id.clone());

        let msgs = repo::message::list_by_conversation(
            &ctx.pool,
            &parsed.target_id,
            Some(READ_WINDOW),
            None,
        )
        .await?;
        if msgs.is_empty() {
            return Err(AppError::Validation("目标会话没有消息".into()));
        }
        let window_truncated = msgs.len() as i64 >= READ_WINDOW;

        // 全文按消息行贪心装页（system 摘要行跳过——压缩元数据非对话内容）
        let mut pages: Vec<String> = Vec::new();
        let mut cur = String::new();
        for m in &msgs {
            if m.role == "system" {
                continue;
            }
            let line = crate::harness::references::render_message_line(m);
            if !cur.is_empty() && cur.chars().count() + line.chars().count() > PAGE_CHAR_BUDGET {
                pages.push(std::mem::take(&mut cur));
            }
            cur.push_str(&line);
        }
        if !cur.is_empty() {
            pages.push(cur);
        }

        let total = pages.len() as i64;
        let idx = (parsed.page - 1) as usize;
        let page = pages.get(idx).ok_or_else(|| {
            AppError::Validation(format!("第 {} 页不存在（共 {total} 页）", parsed.page))
        })?;

        Ok(json!({
            "target_id": parsed.target_id,
            "title": if conv.title.is_empty() { "（未命名）" } else { &conv.title },
            "agent": agent_name,
            "page": parsed.page,
            "total_pages": total,
            "window_note": if window_truncated {
                serde_json::Value::String(format!(
                    "仅含最近 {READ_WINDOW} 条消息，更早内容不在读取窗口内"
                ))
            } else {
                serde_json::Value::Null
            },
            "content": page,
            "has_next": parsed.page < total,
        })
        .to_string())
    }
}

// =========================================================================
// 单元测试（in-memory SQLite）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{NewAgent, NewConversation, NewMessage};
    use crate::db::repo;
    use sqlx::SqlitePool;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn test_pool() -> SqlitePool {
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

    async fn seed(pool: &SqlitePool) {
        repo::agent::create(
            pool,
            &NewAgent {
                id: "a1".into(),
                name: "助手".into(),
                provider: "openai".into(),
                model: "m".into(),
                system_prompt: String::new(),
                api_key: String::new(),
                base_url: None,
                temperature: 0.7,
                max_tokens: 1024,
                extra_params: None,
                sort_order: 0,
                cache_prompt: true,
                supports_vision: false,
                max_history_messages: None,
                context_window: None,
                enabled_tools: None,
                workspace_path: None,
                avatar: None,
                emoji: None,
            },
            "a1",
            "slot",
        )
        .await
        .unwrap();
    }

    async fn seed_conv(pool: &SqlitePool, id: &str) {
        repo::conversation::create(
            pool,
            id,
            &NewConversation {
                agent_id: "a1".into(),
                title: Some(format!("会话{id}")),
                project_id: None,
                kind: None,
                initiator_agent_id: None,
                parent_conversation_id: None,
            },
        )
        .await
        .unwrap();
    }

    async fn seed_msg(pool: &SqlitePool, id: &str, conv: &str, role: &str, content: &str) {
        repo::message::create(
            pool,
            id,
            &NewMessage {
                conversation_id: conv.into(),
                role: role.into(),
                content: content.into(),
                token_count: None,
                error: None,
                model: None,
            },
        )
        .await
        .unwrap();
    }

    fn ctx(pool: &SqlitePool, conv_id: &str) -> ToolContext {
        ToolContext {
            conv_id: conv_id.into(),
            agent_id: "a1".into(),
            project_id: None,
            workspace: None,
            pool: pool.clone(),
            api_key: None,
            app_handle: None,
            proposal_registry: None,
            turn_id: None,
            cancel: None,
        }
    }

    #[tokio::test]
    async fn reads_referenced_conversation_single_page() {
        let pool = test_pool().await;
        seed(&pool).await;
        seed_conv(&pool, "cur").await;
        seed_conv(&pool, "c2").await;
        // 当前会话一条消息引用了 c2（Reference 块落库形态）
        seed_msg(&pool, "u1", "cur", "user", "看看").await;
        sqlx::query("UPDATE messages SET content_blocks = ? WHERE id = 'u1'")
            .bind(r#"[{"type":"text","text":"看看"},{"type":"reference","ref_kind":"conversation","target_id":"c2","display":"会话c2#1234"}]"#)
            .execute(&pool)
            .await
            .unwrap();
        // 目标会话内容（含一条 system 摘要行——应被跳过）
        seed_msg(&pool, "m1", "c2", "user", "旧问题一").await;
        seed_msg(&pool, "m2", "c2", "assistant", "旧回答一").await;
        seed_msg(&pool, "m3", "c2", "system", "[Previous conversation summary]\nxx").await;

        let out = ReadReferenceTool
            .execute_with_context(r#"{"target_id":"c2","page":1}"#, &ctx(&pool, "cur"))
            .await
            .expect("读取成功");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["total_pages"], 1);
        assert_eq!(v["has_next"], false);
        assert_eq!(v["title"], "会话c2");
        let content = v["content"].as_str().unwrap();
        assert!(content.contains("[user]: 旧问题一"));
        assert!(content.contains("[assistant]: 旧回答一"));
        assert!(!content.contains("summary")); // system 摘要行不进全文
        assert!(v["window_note"].is_null());
    }

    #[tokio::test]
    async fn guard_rejects_unreferenced_target() {
        let pool = test_pool().await;
        seed(&pool).await;
        seed_conv(&pool, "cur").await;
        seed_conv(&pool, "secret").await;
        seed_msg(&pool, "u1", "cur", "user", "无关消息").await;
        seed_msg(&pool, "m1", "secret", "user", "机密").await;

        // 当前会话从未引用 secret → 拒绝
        let err = ReadReferenceTool
            .execute_with_context(r#"{"target_id":"secret"}"#, &ctx(&pool, "cur"))
            .await
            .expect_err("未引用应被拒");
        assert!(err.to_string().contains("未被当前对话引用"));

        // 引用后放行
        sqlx::query("UPDATE messages SET content_blocks = ? WHERE id = 'u1'")
            .bind(r#"[{"type":"reference","ref_kind":"conversation","target_id":"secret","display":"x#1"}]"#)
            .execute(&pool)
            .await
            .unwrap();
        let ok = ReadReferenceTool
            .execute_with_context(r#"{"target_id":"secret"}"#, &ctx(&pool, "cur"))
            .await;
        assert!(ok.is_ok());

        // 无 context 直调 → Internal 错（走 dispatch 的 execute_with_context 路径）
        assert!(ReadReferenceTool.execute("{}").await.is_err());
    }

    #[tokio::test]
    async fn big_conversation_pages_and_navigation() {
        let pool = test_pool().await;
        seed(&pool).await;
        seed_conv(&pool, "cur").await;
        seed_conv(&pool, "big").await;
        seed_msg(&pool, "cur-u1", "cur", "user", "引用它").await;
        sqlx::query("UPDATE messages SET content_blocks = ? WHERE id = 'cur-u1'")
            .bind(r#"[{"type":"reference","ref_kind":"conversation","target_id":"big","display":"b#1"}]"#)
            .execute(&pool)
            .await
            .unwrap();
        // 30 轮 × 每条 ~700 字符 ≈ 42K 字符 → 多页
        let long = "内".repeat(700);
        for t in 1..=30 {
            seed_msg(&pool, &format!("u{t}"), "big", "user", &format!("第{t}轮{long}")).await;
            seed_msg(&pool, &format!("a{t}"), "big", "assistant", &format!("答{t}{long}")).await;
        }

        let p1: serde_json::Value = serde_json::from_str(
            &ReadReferenceTool
                .execute_with_context(r#"{"target_id":"big","page":1}"#, &ctx(&pool, "cur"))
                .await
                .unwrap(),
        )
        .unwrap();
        let total = p1["total_pages"].as_i64().unwrap();
        assert!(total >= 3, "42K 字符应分 ≥3 页，实际 {total}");
        assert_eq!(p1["has_next"], true);
        assert!(p1["content"].as_str().unwrap().contains("第1轮"));
        assert!(!p1["content"].as_str().unwrap().contains("第30轮")); // 首页不含末轮

        // 末页：has_next=false 且含末轮
        let pn: serde_json::Value = serde_json::from_str(
            &ReadReferenceTool
                .execute_with_context(
                    &format!(r#"{{"target_id":"big","page":{total}}}"#),
                    &ctx(&pool, "cur"),
                )
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(pn["has_next"], false);
        assert!(pn["content"].as_str().unwrap().contains("第30轮"));

        // 越界页号 → 明确报错
        let err = ReadReferenceTool
            .execute_with_context(
                &format!(r#"{{"target_id":"big","page":{}}}"#, total + 1),
                &ctx(&pool, "cur"),
            )
            .await;
        assert!(err.is_err());
    }
}
