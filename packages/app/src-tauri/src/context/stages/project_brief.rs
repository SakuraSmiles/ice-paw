//! ProjectBriefStage — 项目上下文动态注入（2026-09-29 项目维度存在感批）。
//!
//! 治「同一个项目下的不同会话/不同 agent 感觉和不是一个项目没什么差别」：
//! 每个挂项目的会话，agent 上下文自动携带——
//! - **成员名册**：谁在项目里、各自 agent 名字（@ 引用 / 委派寻址的可用性地基，
//!   与频道简报的 roster 同构）
//! - **近况摘要**：项目内最近 N 个会话的最后一条摘要（跨会话智慧——agent A
//!   在会话 1 学到的东西，agent B 在会话 2 打开就能看到）
//! - **已有 project.md** 沿用 SystemPromptStage 的注入（本 Stage 不重复）
//!
//! 注入位置：`PipelineContext.project_brief`（SystemPromptStage 在拼 system
//! prompt 时读取拼接——保持单点拼装、不搞双段）。
//!
//! 散落会话（project_id=None）零注入零开销——Stage 直接 return。

use async_trait::async_trait;
use sqlx::SqlitePool;

use crate::db::repo;
use crate::error::AppResult;

use super::PipelineStage;

pub(crate) struct ProjectBriefStage;

#[async_trait]
impl PipelineStage for ProjectBriefStage {
    fn name(&self) -> &'static str {
        "project_brief"
    }

    async fn execute(&self, ctx: &mut super::PipelineContext) -> AppResult<()> {
        // 散落会话 / 无项目上下文目录 → 零注入（普通 1v1 零开销）
        let Some(dir) = ctx.project_context_dir.as_ref() else {
            return Ok(());
        };
        // 目录路径含 project_id（{ws}/projects/{pid}/）——从路径提取
        let Some(pid) = dir.rsplit('/').next().filter(|s| !s.is_empty()) else {
            return Ok(());
        };

        let pool = &ctx.pool;
        let mut brief = String::new();

        // ===== ① 成员名册 =====
        if let Ok(members) = repo::project::list_member_profiles(pool, pid).await {
            if !members.is_empty() {
                let names: Vec<String> = members
                    .iter()
                    .map(|m| {
                        format!(
                            "- {}（{}）",
                            m.name,
                            if m.role == "coordinator" {
                                "统筹"
                            } else {
                                "成员"
                            }
                        )
                    })
                    .collect();
                brief.push_str(&format!(
                    "\n\n## 项目成员\n{}\n（跨会话通讯可投递同项目会话；委派可指定项目成员）",
                    names.join("\n")
                ));
            }
        }

        // ===== ② 近况摘要（项目内最近 3 个会话的最后一条 turn 摘要） =====
        if let Some(briefs) = recent_session_briefs(pool, pid, 3).await {
            if !briefs.is_empty() {
                brief.push_str("\n\n## 项目近况（最近会话摘要）");
                for (title, summary) in briefs {
                    brief.push_str(&format!("\n### {title}\n{summary}"));
                }
            }
        }

        if !brief.is_empty() {
            ctx.project_brief = Some(brief);
        }
        Ok(())
    }
}

/// 项目内最近 N 个会话的最新摘要（跨会话智慧——摘要存 messages 表
/// role='system' 行，content 以 `[Previous conversation summary]` 开头，
/// 取项目内最新有摘要的会话各一条）。
async fn recent_session_briefs(
    pool: &SqlitePool,
    project_id: &str,
    n: i64,
) -> Option<Vec<(String, String)>> {
    const SUMMARY_PREFIX: &str = "[Previous conversation summary]";
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT c.title, m.content \
         FROM conversations c \
         JOIN messages m ON m.conversation_id = c.id \
             AND m.role = 'system' \
             AND instr(m.content, ?) = 1 \
             AND m.created_at = (SELECT MAX(m2.created_at) FROM messages m2 \
                                   WHERE m2.conversation_id = c.id AND m2.role = 'system' \
                                     AND instr(m2.content, ?) = 1) \
         WHERE c.project_id = ? AND c.kind = 'chat' \
         ORDER BY c.updated_at DESC LIMIT ?",
    )
    .bind(SUMMARY_PREFIX)
    .bind(SUMMARY_PREFIX)
    .bind(project_id)
    .bind(n)
    .fetch_all(pool)
    .await
    .ok()?;

    Some(
        rows.into_iter()
            .filter_map(|(title, content)| {
                if title.is_empty() || content.is_empty() {
                    return None;
                }
                // 剥前缀行 + 截前 300 字
                let text = content
                    .strip_prefix(SUMMARY_PREFIX)
                    .unwrap_or(&content)
                    .trim();
                if text.is_empty() {
                    return None;
                }
                let short: String = text.chars().take(300).collect();
                Some((title, short))
            })
            .collect(),
    )
}
