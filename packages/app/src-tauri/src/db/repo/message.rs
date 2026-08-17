//! `messages` 表的 SQL 操作
//!
//! 关键点：
//! - 列表支持 `limit` 和 `before`（created_at）翻页
//! - 默认按 `created_at ASC` 升序输出（chat 展示顺序）
//! - 同秒内多条消息的次序：用 `rowid` 做兜底排序，避免 UUID 随机化导致
//!   「用户/助手 同一对内顺序反转」的 bug（见 `list_by_conversation`）

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::db::models::{MessageRow, NewMessage};
use crate::error::{AppError, AppResult};

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 1000;

/// 历史加载的固定条数上限（send_message + session-events 派生读路径共用）。
///
/// Phase 2 起 DB 加载固定为此值，不再耦合 `max_history_messages`（后者重定义为
/// MemoryStage 的 keep_n 地板）。**单一来源**：chat_cmd 与 read_route 都引用它，
/// 保证「legacy 行加载 N 条」与「派生 tail-limit N 条」严格一致——否则两侧
/// 窗口不同会破坏「同库同消息」的读路径切换不变式。
pub const HISTORY_LOAD_LIMIT: i64 = 500;

/// 列出会话内的消息（复合游标分页）
///
/// - `before`：`(created_at, rowid)` 复合游标，表示「取此游标之前的消息」。
///   前一页最末一条消息的 `(created_at, rowid)` 即下一页要传的 `before`。
///   `Some((ts, 0))` 之类的边界值由调用方负责，函数假定传入合法游标。
/// - `limit`：上限 1000，默认 100
///
/// 排序策略：`(created_at DESC, rowid DESC)`，反转后等价于
/// `(created_at ASC, rowid ASC)`。
///
/// 关键：**不要把 `id` 当成 tie-breaker**。
/// `id` 是 TEXT 类型的随机 UUID（v4），字典序与插入顺序无关。
/// SQLite 的 `datetime('now')` 是秒级精度——`send_message` 内
/// 「先 INSERT 用户消息，再 INSERT 助手占位」两次写入通常落在同一秒，
/// 此时如果以 `id` 做兜底排序，约 50% 的概率助手会排在用户之前，
/// 再被 `rows.reverse()` 反转后变成「助手先、用户后」——
/// 表现为页面上 `用户 → AI → AI → 用户` 的顺序错乱。
///
/// 改用 `rowid`（SQLite 的物理行号，单调递增）后，助手占位一定在
/// 用户消息之后被 INSERT，`rowid` 也一定更大，排序与插入顺序一致，
/// 翻转到 ASC 后用户在前、助手在后，行为可预期。
///
/// 同时，向上翻页用 `(created_at, rowid)` 复合游标，**严格小于**两段都满足
/// 的消息才返回。这样可以确保同秒内的多页之间不重不漏：
///
/// ```sql
/// AND (created_at < ? OR (created_at = ? AND rowid < ?))
/// ```
pub async fn list_by_conversation(
    pool: &SqlitePool,
    conversation_id: &str,
    limit: Option<i64>,
    before: Option<(String, i64)>,
) -> AppResult<Vec<MessageRow>> {
    let mut lim = limit.unwrap_or(DEFAULT_LIMIT);
    if lim <= 0 {
        lim = DEFAULT_LIMIT;
    }
    if lim > MAX_LIMIT {
        lim = MAX_LIMIT;
    }

    let rows = if let Some((before_ts, before_rowid)) = before {
        sqlx::query_as::<_, MessageRow>(
            "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid, summary_id, model
               FROM messages
              WHERE conversation_id = ?
                AND (created_at < ? OR (created_at = ? AND rowid < ?))
              ORDER BY created_at DESC, rowid DESC
              LIMIT ?",
        )
        .bind(conversation_id)
        .bind(&before_ts)
        .bind(&before_ts)
        .bind(before_rowid)
        .bind(lim)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, MessageRow>(
            "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid, summary_id, model
               FROM messages
              WHERE conversation_id = ?
              ORDER BY created_at DESC, rowid DESC
              LIMIT ?",
        )
        .bind(conversation_id)
        .bind(lim)
        .fetch_all(pool)
        .await?
    };

    // 反转，按时间正序返回（chat 友好的顺序）
    let mut rows = rows;
    rows.reverse();
    Ok(rows)
}

/// 轮次锚点（聊天「轮次导航条」UX #5）：一轮 = 一条**真实**用户消息。
///
/// 两类占位行必须排除（否则轮次被膨胀/幽灵化）：
/// 1. 工具轮的 tool_result 占位行（content='' + content_blocks 含
///    tool_result）——真机：10 轮会话曾数出 49。词表与前端渲染侧
///    `isToolResultOnlyUser`（ChatMessages.vue：content 空才排除）严格对齐：
///    排除条件必须是「content 空 **且** blocks 含 tool_result」的合取，
///    不能单看 blocks 子串——用户正文里粘贴了含 `"type":"tool_result"`
///    字面量的 JSON/日志时，该子串会嵌进 blocks 的 text 块，单看子串会把
///    正常消息误排除出锚点，而前端照样渲染 → 轮号整体偏移（P11 症状①）。
/// 2. **空占位行**（content='' 且 content_blocks 空/'[]'）——loop_engine
///    阶段 F 先 create 占位再 update_content_blocks，进程死亡/崩溃残留的
///    空行会被误当锚点（真机：34 真实轮曾数出 36，导航条与实际位置错位）。
///    注意纯图/纯附件消息 content 为空但 blocks 非空，是真锚点，不得误伤。
///
/// 排除后与轨迹页 `count_turns_before`（distinct turn_id，不变式
/// turn_id == user_msg_id）同基准。
///
/// 只取 `id / 预览 / 时间` 三个轻量字段——**不加载 content_blocks 大字段**，
/// 预览在 SQL 侧 `substr` 截断（字符级），3000 轮也只有小行级成本。
/// 轮号 = 前端按下标 +1。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnAnchor {
    pub message_id: String,
    /// 用户消息正文预览（SQL substr 120 字符，前端再按显示宽收）
    pub preview: String,
    pub created_at: String,
}

pub async fn list_turn_anchors(
    pool: &SqlitePool,
    conversation_id: &str,
) -> AppResult<Vec<TurnAnchor>> {
    let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(
        // COALESCE：content_blocks 为 NULL 的旧 user 行是真实轮次，必须保留
        "SELECT id, substr(content, 1, 120), created_at \
           FROM messages \
          WHERE conversation_id = ? AND role = 'user' \
            AND NOT (TRIM(COALESCE(content, '')) = '' \
                     AND COALESCE(content_blocks, '') LIKE '%\"type\":\"tool_result\"%') \
            AND NOT (TRIM(COALESCE(content, '')) = '' \
                     AND COALESCE(content_blocks, '') IN ('', '[]')) \
          ORDER BY created_at ASC, rowid ASC",
    )
    .bind(conversation_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(message_id, preview, created_at)| TurnAnchor {
            message_id,
            // 纯图/纯附件消息 content 可能为空 → 空串，前端以「(无文本)」占位
            preview: preview.unwrap_or_default(),
            created_at,
        })
        .collect())
}

/// 统计会话内的消息总数
///
/// 返回 `i64` 以与 SQL `COUNT(*)` 对齐，且与 SQLite 的上限毫无关系。
/// 用于前端「还有 N 条历史消息」之类的展示（P2 可选，本期不调用）。
pub async fn count_by_conversation(pool: &SqlitePool, conversation_id: &str) -> AppResult<i64> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE conversation_id = ?")
        .bind(conversation_id)
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

/// 按 rowid 正序全量读取会话消息（session-events 对账专用，Phase 1）。
///
/// 与 [`list_by_conversation`] 的差异：
/// - **排序只用 rowid**（物理插入序）——对账要比对「事件 seq 序 vs 行写入序」，
///   `created_at` 是秒级时间戳，时钟回拨（NTP step）会让复合序与 rowid 反转。
/// - **无 limit 钳制**——`list_by_conversation` 的 `MAX_LIMIT=1000` 会静默截断
///   长会话，对账必须全量（截断 = 假差异源）。
pub async fn list_all_by_rowid(
    pool: &SqlitePool,
    conversation_id: &str,
) -> AppResult<Vec<MessageRow>> {
    let rows = sqlx::query_as::<_, MessageRow>(
        "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid, summary_id, model
           FROM messages
          WHERE conversation_id = ?
          ORDER BY rowid ASC",
    )
    .bind(conversation_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 当前会话的最大 rowid（无消息时返回 0）。
///
/// **轻量指纹探测**（session-events Phase 2A 读路径路由用）：单聚合查询、不取行体，
/// 与 [`crate::db::repo::session_event::max_seq`] 组成会话「数据指纹」——指纹未变即
/// 缓存的路由决策仍有效，免去每轮都跑全量对账。
pub async fn max_rowid(pool: &SqlitePool, conversation_id: &str) -> AppResult<i64> {
    let (max,): (i64,) =
        sqlx::query_as("SELECT COALESCE(MAX(rowid), 0) FROM messages WHERE conversation_id = ?")
            .bind(conversation_id)
            .fetch_one(pool)
            .await?;
    Ok(max)
}

/// id → rowid 映射（session-events Phase 2A 派生读路径用）。
///
/// 派生消息（来自事件回放）只有 message_id，无物理行号；而 MemoryStage 的滚动摘要
/// 靠 `source_rowid` 按值定位覆盖切断点（`covered_until_rowid`）。本映射把派生消息
/// **锚回真实物理 rowid**，使摘要连续性在读路径切换（legacy → derive）后依然成立——
/// 切换前用真 rowid 记的 `covered_until_rowid`，切换后在派生消息里照样查得到同值。
///
/// 路由判为 Derive 的会话（对账零 diff）里每个 evented message_id 必有对应行，
/// 映射完备；缺项只可能出现在有 diff 的会话（已被路由到 Legacy，不会走到派生路径）。
pub async fn id_rowid_map(
    pool: &SqlitePool,
    conversation_id: &str,
) -> AppResult<HashMap<String, i64>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT id, rowid FROM messages WHERE conversation_id = ?")
            .bind(conversation_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

/// 按 id 取行 `content_blocks` 原文（Image 引用水合的行侧原语，S1 阶段 3）。
///
/// `messages.id` 全局唯一（TEXT PRIMARY KEY）——事件 payload 的 image_ref 只带
/// message_id + block_index，水合即「查行 → parse → 取下标」。缺行 → None
/// （调用方按 [`crate::harness::derive::IMAGE_UNRECOVERABLE_MARKER`] 降级）。
pub async fn get_content_blocks_by_id(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT content_blocks FROM messages WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(content_blocks,)| content_blocks))
}

/// 写入新消息
pub async fn create(pool: &SqlitePool, id: &str, new_msg: &NewMessage) -> AppResult<MessageRow> {
    // 基本校验
    if !matches!(
        new_msg.role.as_str(),
        "system" | "user" | "assistant" | "tool"
    ) {
        return Err(AppError::Validation(format!(
            "非法 role: {}, 必须是 system/user/assistant/tool",
            new_msg.role
        )));
    }

    sqlx::query(
        "INSERT INTO messages
            (id, conversation_id, role, content, content_blocks, token_count, error, model)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&new_msg.conversation_id)
    .bind(&new_msg.role)
    .bind(&new_msg.content)
    .bind("[]") // content_blocks 默认空数组
    .bind(new_msg.token_count)
    .bind(new_msg.error.as_deref())
    .bind(new_msg.model.as_deref())
    .execute(pool)
    .await?;

    // 更新父会话的 updated_at 触发器虽然有，但 INSERT 不触发；这里手动 update 一下
    sqlx::query("UPDATE conversations SET updated_at = datetime('now') WHERE id = ?")
        .bind(&new_msg.conversation_id)
        .execute(pool)
        .await?;

    get_by_id(pool, id).await
}

async fn get_by_id(pool: &SqlitePool, id: &str) -> AppResult<MessageRow> {
    sqlx::query_as::<_, MessageRow>(
        "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid, summary_id, model
           FROM messages WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "message",
        id: id.to_string(),
    })
}

/// 更新消息内容（流式生成结束后回写完整文本）
pub async fn update_content(pool: &SqlitePool, id: &str, content: &str) -> AppResult<()> {
    let affected = sqlx::query("UPDATE messages SET content = ? WHERE id = ?")
        .bind(content)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "message",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 更新消息的 content_blocks 字段（P2-1 工具调用场景）
pub async fn update_content_blocks(
    pool: &SqlitePool,
    id: &str,
    content_blocks: &str,
) -> AppResult<()> {
    let affected = sqlx::query("UPDATE messages SET content_blocks = ? WHERE id = ?")
        .bind(content_blocks)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "message",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 更新消息错误字段（流式生成失败时记录）
pub async fn update_error(pool: &SqlitePool, id: &str, error: &str) -> AppResult<()> {
    let affected = sqlx::query("UPDATE messages SET error = ? WHERE id = ?")
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "message",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// M1.3: 更新消息的 token_count 字段（流式结束后回填）
///
/// - `id`          消息 ID
/// - `token_count` token 数（应为非负 i32；调用方负责下限保护）
///
/// # 错误
/// - 消息 ID 不存在 → `AppError::NotFound`
///
/// # 注意
/// - 0 视为合法值（被存储）
/// - 负数应被业务侧拦截；这里仅做最基础的 SQL 执行
pub async fn update_token_count(pool: &SqlitePool, id: &str, token_count: i32) -> AppResult<()> {
    let affected = sqlx::query("UPDATE messages SET token_count = ? WHERE id = ?")
        .bind(token_count)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "message",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 按 id 删除消息。
///
/// 用于 cancel 时清理无内容的空占位行（避免刷新后残留空气泡）。
/// 注意：`tool_calls.message_id` 外键引用 `messages.id`——空占位无 tool_calls 记录，
/// 删除安全；若未来对有 tool_calls 的消息调用，需先清理 tool_calls 或依赖外键级联。
pub async fn delete(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let affected = sqlx::query("DELETE FROM messages WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            resource: "message",
            id: id.to_string(),
        });
    }
    Ok(())
}

/// 取单条消息所属的 `conversation_id`。
///
/// 用于 `read_attachment_page` 工具的越权守卫：消息不存在返回 `None`，
/// 存在则返回其会话 ID，由调用方比对当前会话（`ctx.conv_id`）。
pub async fn conversation_id(pool: &SqlitePool, message_id: &str) -> AppResult<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT conversation_id FROM messages WHERE id = ?")
            .bind(message_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(c,)| c))
}

/// M1.2: 列出会话内最近的工具调用名（从 `tool_calls` 审计表 JOIN `messages` 查询）
///
/// # 用途
/// - 为 `loop_engine` 在每轮调用 `list_tool_defs_with_query` 时的
///   「调用历史权重」提供输入（M1.4 之前由 `ToolTrimStage` 消费，现已下沉
///   到 loop_engine 直接打分）
/// - 按 `tool_calls.created_at DESC` 取最近 `limit` 条，返回顺序不限（按出现次数累计）
///
/// # 行为
/// - 返回 Vec<String>：仅含 `tool_calls.tool_name`，**不去重**（让打分函数按出现次数加权）
/// - `limit <= 0` → 使用默认值 10
pub async fn list_recent_tool_names(
    pool: &SqlitePool,
    conversation_id: &str,
    limit: i32,
) -> AppResult<Vec<String>> {
    let lim = if limit <= 0 { 10 } else { limit };

    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT tc.tool_name
           FROM tool_calls tc
           JOIN messages m ON m.id = tc.message_id
          WHERE m.conversation_id = ?
          ORDER BY tc.created_at DESC
          LIMIT ?",
    )
    .bind(conversation_id)
    .bind(lim)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|(n,)| n).collect())
}

// =========================================================================
// 单元测试（M1.3）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::NewMessage;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("valid sqlite url")
            .create_if_missing(true)
            .foreign_keys(true);
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite")
    }

    async fn seed_message(pool: &SqlitePool, id: &str, conv_id: &str) {
        // 需要 agent 作为 conversation 外键依赖
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind("agent-1")
        .bind("test-agent")
        .bind("anthropic")
        .bind("claude-test")
        .bind("")
        .bind("")
        .bind(0.7)
        .bind(1024)
        .bind("{}")
        .bind(0)
        .bind(0)
        .execute(pool)
        .await
        .expect("seed agent");
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES (?, ?, ?)")
            .bind(conv_id)
            .bind("agent-1")
            .bind("test conv")
            .execute(pool)
            .await
            .expect("seed conversation");
        create(
            pool,
            id,
            &NewMessage {
                conversation_id: conv_id.to_string(),
                role: "user".to_string(),
                content: "hello".to_string(),
                token_count: None,
                error: None,
                model: None,
            },
        )
        .await
        .expect("seed message");
    }

    #[tokio::test]
    async fn update_token_count_writes_value() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed_message(&pool, "msg-1", "conv-1").await;

        update_token_count(&pool, "msg-1", 42).await.unwrap();
        let row = get_by_id(&pool, "msg-1").await.unwrap();
        assert_eq!(row.token_count, Some(42));
    }

    #[tokio::test]
    async fn update_token_count_unknown_id_returns_err() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();

        let result = update_token_count(&pool, "nonexistent", 10).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::NotFound { resource, id } => {
                assert_eq!(resource, "message");
                assert_eq!(id, "nonexistent");
            }
            e => panic!("expected NotFound, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn update_token_count_zero_is_stored() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed_message(&pool, "msg-1", "conv-1").await;

        update_token_count(&pool, "msg-1", 0).await.unwrap();
        let row = get_by_id(&pool, "msg-1").await.unwrap();
        // 0 作为合法值被存储（业务层会在调用前保护下限）
        assert_eq!(row.token_count, Some(0));
    }

    #[tokio::test]
    async fn delete_removes_message_and_reports_unknown_id() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed_message(&pool, "msg-del", "conv-del").await;

        // 已存在 → 删除成功
        delete(&pool, "msg-del").await.unwrap();
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE id = ?")
            .bind("msg-del")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "删除后消息应不存在");

        // 不存在 → NotFound
        match delete(&pool, "msg-del").await {
            Err(AppError::NotFound { resource, id }) => {
                assert_eq!(resource, "message");
                assert_eq!(id, "msg-del");
            }
            e => panic!("expected NotFound, got {e:?}"),
        }
    }
}
