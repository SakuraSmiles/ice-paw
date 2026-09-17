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
use crate::infra::image_store;

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 1000;

/// 历史加载的固定条数上限（send_message + session-events 派生读路径共用）。
///
/// Phase 2 起 DB 加载固定为此值，不再耦合 `max_history_messages`（后者重定义为
/// MemoryStage 的 keep_n 地板）。**单一来源**：chat_cmd 与 read_route 都引用它，
/// 保证「legacy 行加载 N 条」与「派生 tail-limit N 条」严格一致——否则两侧
/// 窗口不同会破坏「同库同消息」的读路径切换不变式。
pub const HISTORY_LOAD_LIMIT: i64 = 500;

/// 回合制分页默认回合数（生产常规回合 5-40 行，8 回合约 40-320 行，
/// 与 TurnRail RAIL_WINDOW=13 同量级；短对话一页全量）。
pub const TURN_PAGE_DEFAULT_TURNS: i64 = 8;
/// 回合制分页 turns 参数钳制上限（防滥用；超界/非法一律回落默认值）。
pub const TURN_PAGE_MAX_TURNS: i64 = 50;
/// 回合制分页单页行数安全闸：触闸诚实少装回合（页界仍是回合边界），
/// 旧链式最坏 20 页 1000 行，300 单页仍小 3 倍。
const TURN_PAGE_MAX_ROWS: usize = 300;

/// 真实用户消息锚点谓词（回合制分页 SQL-A 与三个锚点查询共用——同源常量，
/// 四处共用防口径漂移；COALESCE：content_blocks 为 NULL 的旧 user 行是真实
/// 轮次，必须保留）。语义（[`TurnAnchor`] 详注）：排除 tool_result 占位行与
/// 空占位行——排除条件必须是「content 空 且 blocks 含 tool_result」的**合取**，
/// 单看子串会误伤正文粘贴该字面量的用户消息；纯图/纯附件行（content 空、
/// blocks 非空且无 tool_result）是真锚点。
const REAL_USER_ANCHOR_PREDICATE: &str = "\
role = 'user' \
AND NOT (TRIM(COALESCE(content, '')) = '' AND COALESCE(content_blocks, '') LIKE '%\"type\":\"tool_result\"%') \
AND NOT (TRIM(COALESCE(content, '')) = '' AND COALESCE(content_blocks, '') IN ('', '[]'))";

/// 回合 span 取行 SQL 前缀（列清单照抄 [`list_by_conversation`]——13 列全量，
/// 与旧读路径同构）。
const SPAN_SELECT: &str = "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid, summary_id, model, incoming_source, sender_agent_id FROM messages WHERE conversation_id = ?";

/// 读侧水合（U2-1 图片外置）：把 `content_blocks` 里的 `image_file` 指针读回
/// 内联 base64。生产唯一入口 `image_store::hydrate_json`——目录未初始化（测试 /
/// boot 前）时恒等 no-op，纯文本行的 `image_file` 快路径原样直过。逐行调用，
/// 保「repo 返回即内联形态」的不变式，覆盖所有下游 `parse_content_blocks` 消费点。
async fn hydrate_rows(rows: &mut [MessageRow]) {
    for row in rows.iter_mut() {
        row.content_blocks = image_store::hydrate_json(&row.content_blocks).await;
    }
}

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
            "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid, summary_id, model, incoming_source, sender_agent_id
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
            "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid, summary_id, model, incoming_source, sender_agent_id
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
    hydrate_rows(&mut rows).await;
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
    let sql = format!(
        "SELECT id, substr(content, 1, 120), created_at \
           FROM messages \
          WHERE conversation_id = ? AND {REAL_USER_ANCHOR_PREDICATE} \
          ORDER BY created_at ASC, rowid ASC"
    );
    let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(&sql)
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

/// 某消息之后的真实用户消息锚点（频道 v1 积压查询）。
///
/// [`TurnAnchor`] 同款排除（tool_result 占位行 + 空占位行——在途回合的工具轮
/// user 行不是「用户发言」，混进来会把抢占检查误判成用户插话）；`rowid >` 以
/// 消息 id 子查询定位（`sweep_sender_for_channel_turn` 同款模式）。按时间正序
/// 返回——频道以「链头之后出现新 user 行」判定用户插话（C8 抢占检查），以
/// 全部积压的**最后一条**为新链头。
pub async fn list_user_anchors_after(
    pool: &SqlitePool,
    conversation_id: &str,
    after_message_id: &str,
) -> AppResult<Vec<TurnAnchor>> {
    let sql = format!(
        "SELECT id, substr(content, 1, 120), created_at \
           FROM messages \
          WHERE conversation_id = ? AND {REAL_USER_ANCHOR_PREDICATE} \
            AND rowid > (SELECT rowid FROM messages WHERE id = ?) \
          ORDER BY created_at ASC, rowid ASC"
    );
    let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(&sql)
        .bind(conversation_id)
        .bind(after_message_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(message_id, preview, created_at)| TurnAnchor {
            message_id,
            preview: preview.unwrap_or_default(),
            created_at,
        })
        .collect())
}

/// 某消息起（含该消息）的真实用户消息锚点（频道 v1 未派发链头重消费）。
///
/// 与 [`list_user_anchors_after`] 的唯一差异 = `rowid >=`（含头）——链头登记后
/// **尚未派发任何成员回合**（等选举落定/会话忙暂缓）时，链头消息自身还没被
/// 消费过，重消费必须把它包含进来；已派发则走 `after`（严格之后）。
pub async fn list_user_anchors_from(
    pool: &SqlitePool,
    conversation_id: &str,
    from_message_id: &str,
) -> AppResult<Vec<TurnAnchor>> {
    let sql = format!(
        "SELECT id, substr(content, 1, 120), created_at \
           FROM messages \
          WHERE conversation_id = ? AND {REAL_USER_ANCHOR_PREDICATE} \
            AND rowid >= (SELECT rowid FROM messages WHERE id = ?) \
          ORDER BY created_at ASC, rowid ASC"
    );
    let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(&sql)
        .bind(conversation_id)
        .bind(from_message_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(message_id, preview, created_at)| TurnAnchor {
            message_id,
            preview: preview.unwrap_or_default(),
            created_at,
        })
        .collect())
}

/// 回合制分页结果：rows 为 rowid ASC 时间正序消息；has_more = 还有更早回合
/// （或本页被行数闸提前截断）；next_before_anchor_rowid = 下页游标（本页最旧
/// 纳入 span 的锚 rowid；has_more=false 时恒 None——防陈旧游标被误用，前端
/// 不得从 messages[0] 自造游标）。
#[derive(Debug)]
pub struct TurnPageRows {
    pub rows: Vec<MessageRow>,
    pub has_more: bool,
    pub next_before_anchor_rowid: Option<i64>,
}

/// 回合制分页（`list_messages_by_turns` 的 repo 层）：以真实 user 消息锚为
/// 回合头，返回「最近 turns 个回合」的全部消息行。每页必含可见内容、页界 =
/// 回合边界 = 前端渲染组边界（前插页头部是真 user 消息必开新组——并组键易主
/// 结构性消失）。
///
/// span 模型（全 rowid 单键——锚严格全序，无 created_at 同秒问题）：
/// 锚 a_i 的 span = [a_i, 下一更新锚)；DESC 锚窗口内逐 span 取行。
/// - span 0 上界：游标页 = `before_anchor_rowid`（排他——= 上页最旧纳入锚，
///   rows ≥ 游标属上页回合）；首载页无上界（未完成尾回合全量含，流式 live
///   一致性依赖整回合入窗）。
/// - 贪心装页（最新 span 向旧迭代）：首个纳入 span 无条件全量（巨回合独立
///   成页——锚游标模型下 mid-span 截断无法表达）；后续 span 探测取
///   `remaining+1`，超额度即停（stopped_early，断点锚留下页——下页 span 0
///   恰为该断点 span，因它的锚未被消费）。
/// - turn-0 残留：终页（窗口耗尽且未提前截断，首载页与游标页同规——长会话
///   终页几乎都是游标页）最旧纳入 span 下界撤掉（lo=0），首锚前残留行并入
///   终页——「已显示全部消息」严格为真。
/// - 零锚点分支：首载（无游标）= 全占位/空会话 → fallback 最后 300 行、
///   has_more=false；**带游标且锚窗口空（陈旧游标/防御）→ 空页——绝不能走
///   300 行 fallback（那是更新行，前插会重复合页）**。
pub async fn list_by_turn_page(
    pool: &SqlitePool,
    conversation_id: &str,
    turns: Option<i64>,
    before_anchor_rowid: Option<i64>,
) -> AppResult<TurnPageRows> {
    let mut turns_n = turns.unwrap_or(TURN_PAGE_DEFAULT_TURNS);
    if !(1..=TURN_PAGE_MAX_TURNS).contains(&turns_n) {
        turns_n = TURN_PAGE_DEFAULT_TURNS;
    }
    let probe = turns_n + 1;
    // SQL-A：锚窗口（多取 1 个探测 has_more）
    let anchors_desc: Vec<(i64,)> = if let Some(before) = before_anchor_rowid {
        let sql = format!(
            "SELECT rowid FROM messages \
             WHERE conversation_id = ? AND {REAL_USER_ANCHOR_PREDICATE} AND rowid < ? \
             ORDER BY rowid DESC LIMIT ?"
        );
        sqlx::query_as(&sql)
            .bind(conversation_id)
            .bind(before)
            .bind(probe)
            .fetch_all(pool)
            .await?
    } else {
        let sql = format!(
            "SELECT rowid FROM messages \
             WHERE conversation_id = ? AND {REAL_USER_ANCHOR_PREDICATE} \
             ORDER BY rowid DESC LIMIT ?"
        );
        sqlx::query_as(&sql)
            .bind(conversation_id)
            .bind(probe)
            .fetch_all(pool)
            .await?
    };
    if anchors_desc.is_empty() {
        // 零锚点：首载 fallback 尾部 300 行（全占位/空会话的诚实兜底）；
        // 游标页锚窗口空 = 陈旧游标/防御 → 空页（fallback 是更新行，前插会重复合页）
        if before_anchor_rowid.is_none() {
            let sql = format!("{SPAN_SELECT} ORDER BY rowid DESC LIMIT {TURN_PAGE_MAX_ROWS}");
            let mut rows: Vec<MessageRow> = sqlx::query_as(&sql)
                .bind(conversation_id)
                .fetch_all(pool)
                .await?;
            rows.reverse();
            hydrate_rows(&mut rows).await;
            return Ok(TurnPageRows {
                rows,
                has_more: false,
                next_before_anchor_rowid: None,
            });
        }
        return Ok(TurnPageRows {
            rows: Vec::new(),
            has_more: false,
            next_before_anchor_rowid: None,
        });
    }
    let has_more_window = anchors_desc.len() as i64 > turns_n;
    let n_win = if has_more_window {
        turns_n as usize
    } else {
        anchors_desc.len()
    };
    let window: Vec<i64> = anchors_desc[..n_win].iter().map(|r| r.0).collect();
    let mut spans: Vec<Vec<MessageRow>> = Vec::new();
    let mut stopped_early = false;
    let mut total: usize = 0;
    // 游标从「纳入时 window[i]」记（不能事后从 rows 派生：终页 lo=0 时 span
    // 首行是残留行不是锚；has_more=true 时任何纳入 span 的 lo 必 = 其锚
    // rowid——lo=0 仅出现在终页、而终页 has_more=false，故游标恒正确）
    let mut last_included_anchor: Option<i64> = None;
    for (i, &anchor_rowid) in window.iter().enumerate() {
        let hi = if i == 0 {
            before_anchor_rowid
        } else {
            Some(window[i - 1])
        };
        let is_last_window_span = i + 1 == n_win;
        let lo = if is_last_window_span && !has_more_window {
            0
        } else {
            anchor_rowid
        };
        let remaining = TURN_PAGE_MAX_ROWS.saturating_sub(total);
        let is_first = spans.is_empty();
        let limit = if is_first { None } else { Some(remaining + 1) };
        let rows = fetch_turn_span(pool, conversation_id, lo, hi, limit).await?;
        if !is_first && rows.len() > remaining {
            stopped_early = true;
            break;
        }
        total += rows.len();
        last_included_anchor = Some(anchor_rowid);
        spans.push(rows);
    }
    let has_more = stopped_early || has_more_window;
    let next_cursor = if has_more { last_included_anchor } else { None };
    spans.reverse();
    let mut rows: Vec<MessageRow> = spans.into_iter().flatten().collect();
    hydrate_rows(&mut rows).await;
    Ok(TurnPageRows {
        rows,
        has_more,
        next_before_anchor_rowid: next_cursor,
    })
}

/// 取单个回合 span 的行（[lo, hi) rowid 半开区间，ASC）。
/// LIMIT 由调用方内联（探测 = remaining+1；首纳入 span 无界——巨回合独立成页）。
async fn fetch_turn_span(
    pool: &SqlitePool,
    conversation_id: &str,
    lo: i64,
    hi: Option<i64>,
    limit: Option<usize>,
) -> AppResult<Vec<MessageRow>> {
    let sql = match (hi, limit) {
        (Some(_), Some(n)) => {
            format!("{SPAN_SELECT} AND rowid >= ? AND rowid < ? ORDER BY rowid ASC LIMIT {n}")
        }
        (Some(_), None) => format!("{SPAN_SELECT} AND rowid >= ? AND rowid < ? ORDER BY rowid ASC"),
        (None, Some(n)) => format!("{SPAN_SELECT} AND rowid >= ? ORDER BY rowid ASC LIMIT {n}"),
        (None, None) => format!("{SPAN_SELECT} AND rowid >= ? ORDER BY rowid ASC"),
    };
    let mut q = sqlx::query_as::<_, MessageRow>(&sql)
        .bind(conversation_id)
        .bind(lo);
    if let Some(hi) = hi {
        q = q.bind(hi);
    }
    Ok(q.fetch_all(pool).await?)
}

/// 会话内**最后一条真实 assistant 消息** id（频道 v1 无 runtime 时的重消费边界）。
///
/// 「真实」= content 非空（产出了可见终文）：回合启动即建的空 assistant 占位
/// 行（崩溃/中断残留）与只含 tool_use/thinking 的中间行都不能当边界——否则
/// 「已占位未回答」的用户消息会被永久排除出重消费面。错误回合若残留这类行
/// → 该消息被重试消费（C5 streak 护栏有界，方向是「多干活」非丢消息）。
/// 无任何真实 assistant 行 → None（全新频道：全部 user 锚点都是积压）。
pub async fn last_assistant_message_id(
    pool: &SqlitePool,
    conversation_id: &str,
) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM messages \
          WHERE conversation_id = ? AND role = 'assistant' \
            AND TRIM(COALESCE(content, '')) != '' \
          ORDER BY rowid DESC LIMIT 1",
    )
    .bind(conversation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id,)| id))
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
    let mut rows = sqlx::query_as::<_, MessageRow>(
        "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid, summary_id, model, incoming_source, sender_agent_id
           FROM messages
          WHERE conversation_id = ?
          ORDER BY rowid ASC",
    )
    .bind(conversation_id)
    .fetch_all(pool)
    .await?;
    hydrate_rows(&mut rows).await;
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
pub async fn get_content_blocks_by_id(pool: &SqlitePool, id: &str) -> AppResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT content_blocks FROM messages WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    // 读侧水合：Image 引用 resolve（derive.rs::hydrate_image_refs）从这里取字节——
    // 必须返回内联 base64 形态，否则 resolve 拿到的 Image.data 是空串（offload 后）。
    Ok(match row {
        Some((content_blocks,)) => Some(image_store::hydrate_json(&content_blocks).await),
        None => None,
    })
}

/// 按消息 id 取整行，不存在返回 None（@ 引用展开用：需要
/// role/content/blocks/conversation_id；与私有 `get_by_id` 的 NotFound 语义区分）。
pub async fn find_by_id(pool: &SqlitePool, id: &str) -> AppResult<Option<MessageRow>> {
    let mut row = sqlx::query_as::<_, MessageRow>(
        "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid, summary_id, model, incoming_source, sender_agent_id
           FROM messages WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    if let Some(r) = row.as_mut() {
        r.content_blocks = image_store::hydrate_json(&r.content_blocks).await;
    }
    Ok(row)
}

/// 回填 MA-3 来件来源元数据 JSON（`update_content_blocks` 同款二段写模式：
/// create 先落行、本 UPDATE 补列——NewMessage 不扩字段，全量构造点零改动）。
pub async fn set_incoming_source(pool: &SqlitePool, id: &str, json: &str) -> AppResult<()> {
    sqlx::query("UPDATE messages SET incoming_source = ? WHERE id = ?")
        .bind(json)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 频道发言者批量回填（C10 双轨列侧，`set_incoming_source` 同款二段写）。
///
/// 频道消费回合的 assistant 行由 run_agent_turn 正常创建（不带 sender——
/// NewMessage 无此字段），引擎在回合完成后按「链头用户消息 rowid 起、
/// `sender_agent_id IS NULL` 的 assistant 行」范围 sweep 打上执行成员 id。
/// `IS NULL` 守卫保证幂等且不覆盖既有归属；行级渲染优先读本列（名字经
/// 事件侧 SenderMeta 快照或查表得到）。返回受影响行数（0 = 无可标行，正常）。
pub async fn sweep_sender_for_channel_turn(
    pool: &SqlitePool,
    conversation_id: &str,
    since_message_id: &str,
    agent_id: &str,
) -> AppResult<u64> {
    let affected = sqlx::query(
        "UPDATE messages SET sender_agent_id = ? \
         WHERE conversation_id = ? AND role = 'assistant' AND sender_agent_id IS NULL \
           AND rowid >= (SELECT rowid FROM messages WHERE id = ?)",
    )
    .bind(agent_id)
    .bind(conversation_id)
    .bind(since_message_id)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected)
}

/// 回填频道发言者（`set_incoming_source` 同款二段写；NewMessage 不扩字段防全量
/// 构造点改动）。出生打标专用：频道成员回合占位 / 选举投票行在**创建时**即落
/// 真实 sender——live 视图（外部回合 chat:start 触发的 loadMessages）第一时间
/// 带身份，且 sweep 的 `IS NULL` 守卫天然跳过（不再误归属，生产实案 2026-09-11：
/// 4 条投票行全被 sweep 盖成统筹者）。
pub async fn set_sender_agent(pool: &SqlitePool, id: &str, agent_id: &str) -> AppResult<()> {
    sqlx::query("UPDATE messages SET sender_agent_id = ? WHERE id = ?")
        .bind(agent_id)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
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
    let mut row = sqlx::query_as::<_, MessageRow>(
        "SELECT id, conversation_id, role, content, content_blocks, token_count, error, created_at, rowid, summary_id, model, incoming_source, sender_agent_id
           FROM messages WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound {
        resource: "message",
        id: id.to_string(),
    })?;
    row.content_blocks = image_store::hydrate_json(&row.content_blocks).await;
    Ok(row)
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
///
/// **写侧唯一入口（U2-1 图片外置）**：落库前把内联 base64 图片外置为文件，
/// DB 只留 `image_file` 指针。`offload_json` 目录未初始化 / 无图片块时恒等直过，
/// 测试与 boot 早于目录初始化时零行为变化。
pub async fn update_content_blocks(
    pool: &SqlitePool,
    id: &str,
    content_blocks: &str,
) -> AppResult<()> {
    let content_blocks = image_store::offload_json(content_blocks).await;
    let affected = sqlx::query("UPDATE messages SET content_blocks = ? WHERE id = ?")
        .bind(&content_blocks)
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

/// 图片外置存量迁移报告（U2-1）。
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct OffloadReport {
    /// 扫描到的含 `"type":"image"` 块的消息行数。
    pub scanned: u64,
    /// 本次实际外置了至少一张图的行数。
    pub offloaded: u64,
    /// 已外置 / 无图 / 坏 base64 等无变化的行数。
    pub unchanged: u64,
    /// 外置或行更新失败、保留内联的行数（下次 boot 重试）。
    pub failed: u64,
}

/// 图片外置存量迁移（U2-1 boot 扫尾）：把 `messages.content_blocks` 里内联
/// base64 图片外置为文件，逐行原子、幂等、崩溃安全。
///
/// - **幂等**：已外置（`image_file` 在场）的行 `offload_json` 原样直过。
/// - **崩溃安全**：文件写经 `write_dedup`（临时文件 + 原子 rename）去重；行
///   UPDATE 单条原子——崩在哪一行，重跑只从该行起，已外置行零成本跳过。
/// - **软失败**：坏 base64 / 写文件失败的行保留内联（下次 boot 重试），绝不
///   因外置失败丢字节。
///
/// 失败不阻塞启动（调用方 warn 后照常）；返回统计供日志披露。
pub async fn offload_all_images(pool: &SqlitePool) -> OffloadReport {
    let mut report = OffloadReport::default();
    // 扫描谓词与 offload_json 快路径同源（`"image"` 子串）；已外置行仍含
    // `"type":"image"`，一并扫进来幂等跳过，无需区分「内联 vs 已外置」。
    let rows: Vec<(String, String)> = match sqlx::query_as::<_, (String, String)>(
        "SELECT id, content_blocks FROM messages WHERE content_blocks LIKE '%\"image\"%'",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(target: "ice_paw.image_store", "扫描含图消息失败（跳过本次外置）: {e}");
            return report;
        }
    };
    report.scanned = rows.len() as u64;
    for (id, content_blocks) in rows {
        let offloaded = image_store::offload_json(&content_blocks).await;
        if offloaded == content_blocks {
            report.unchanged += 1;
            continue;
        }
        match sqlx::query("UPDATE messages SET content_blocks = ? WHERE id = ?")
            .bind(&offloaded)
            .bind(&id)
            .execute(pool)
            .await
        {
            Ok(_) => report.offloaded += 1,
            Err(e) => {
                tracing::warn!(target: "ice_paw.image_store", id, "外置行更新失败（保留内联）: {e}");
                report.failed += 1;
            }
        }
    }
    report
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

    /// 频道积压查询族（`list_user_anchors_from` 含头 + 占位行排除）。
    /// 生产实案 2026-09-10：旧回退边界 = 最后一条 user 锚点 = 刚物化的消息
    /// 自己 → backlog 恒空 → 频道路由整体静默 no-op——含头查询是修复的地基。
    #[tokio::test]
    async fn channel_anchors_from_includes_head_and_filters_placeholders() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed_message(&pool, "u1", "conv-backlog").await; // content = "hello"

        let new_msg = |role: &str, content: &str| NewMessage {
            conversation_id: "conv-backlog".to_string(),
            role: role.to_string(),
            content: content.to_string(),
            token_count: None,
            error: None,
            model: None,
        };
        create(&pool, "u2", &new_msg("user", "第二条")).await.unwrap();
        // tool_result 占位 user 行（在途工具轮，非用户发言）→ 排除
        create(&pool, "u-tool", &new_msg("user", "")).await.unwrap();
        update_content_blocks(&pool, "u-tool", r#"[{"type":"tool_result","tool_use_id":"t1"}]"#)
            .await
            .unwrap();
        // 空占位 user 行（崩溃残留）→ 排除
        create(&pool, "u-empty", &new_msg("user", "")).await.unwrap();

        let ids = |v: Vec<TurnAnchor>| v.into_iter().map(|a| a.message_id).collect::<Vec<_>>();

        // 含头查询（head_dispatched=false 的重消费路径）：从 u2 起 = [u2]（含自己）
        let from_u2 = list_user_anchors_from(&pool, "conv-backlog", "u2").await.unwrap();
        assert_eq!(ids(from_u2), vec!["u2"]);
        // 从 u1 起 = [u1, u2]（占位行两侧都不混入）
        let from_u1 = list_user_anchors_from(&pool, "conv-backlog", "u1").await.unwrap();
        assert_eq!(ids(from_u1), vec!["u1", "u2"]);
        // 对照：严格之后（已派发路径）= [u2]
        let after_u1 = list_user_anchors_after(&pool, "conv-backlog", "u1").await.unwrap();
        assert_eq!(ids(after_u1), vec!["u2"]);
    }

    /// 频道无 runtime 时的重消费边界（`last_assistant_message_id`）：
    /// 空 content 的 assistant 行（占位/纯 tool_use/纯 thinking）不产终文，
    /// 不能当边界——否则其触发消息被永久排除出重消费面。
    #[tokio::test]
    async fn last_assistant_message_id_skips_non_text_rows() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed_message(&pool, "u1", "conv-boundary").await;

        let new_msg = |content: &str| NewMessage {
            conversation_id: "conv-boundary".to_string(),
            role: "assistant".to_string(),
            content: content.to_string(),
            token_count: None,
            error: None,
            model: None,
        };
        // 占位行（content='' blocks='[]'）与纯 tool_use 行（content=''）都跳过
        create(&pool, "a-ph", &new_msg("")).await.unwrap();
        create(&pool, "a-tool", &new_msg("")).await.unwrap();
        update_content_blocks(&pool, "a-tool", r#"[{"type":"tool_use","id":"t1","name":"x"}]"#)
            .await
            .unwrap();
        create(&pool, "a-real", &new_msg("回答正文")).await.unwrap();
        create(&pool, "a-ph2", &new_msg("  ")).await.unwrap(); // 纯空白也跳过

        let last = last_assistant_message_id(&pool, "conv-boundary")
            .await
            .unwrap();
        assert_eq!(last.as_deref(), Some("a-real"));

        // 全部为空行 → None（全新频道：全部 user 锚点都是积压）
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES ('conv-fresh', 'agent-1', 'fresh')")
            .execute(&pool)
            .await
            .unwrap();
        create(&pool, "a-fresh-ph", &NewMessage { conversation_id: "conv-fresh".to_string(), role: "assistant".to_string(), content: String::new(), token_count: None, error: None, model: None })
            .await
            .unwrap();
        let none = last_assistant_message_id(&pool, "conv-fresh").await.unwrap();
        assert_eq!(none, None);
    }

    /// 出生打标 × sweep 互锁（生产实案 2026-09-11 回归锁）：出生带 sender 的行
    /// （成员回合占位 / 选举投票行）被 sweep 的 `IS NULL` 守卫跳过——不会在
    /// 回合结束时被盖成本链执行者；未打标的行照旧由 sweep 兜底。
    #[tokio::test]
    async fn birth_stamped_sender_survives_sweep() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations")
            .run(&pool)
            .await
            .unwrap();
        seed_message(&pool, "head-u1", "conv-sweep").await; // 链头 user 行（含 agent-1）
        // 第二个成员（投票者）：sender_agent_id 有 FK → agents(id)，须真实行
        sqlx::query(
            "INSERT INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES ('voter-x', '投票者', 'zhipu', 'glm-5.3', '', '', 0.7, 1024, '{}', 1, 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let new_msg = |content: &str| NewMessage {
            conversation_id: "conv-sweep".to_string(),
            role: "assistant".to_string(),
            content: content.to_string(),
            token_count: None,
            error: None,
            model: None,
        };
        // 投票行：出生即打标真实投票者 voter-x
        create(&pool, "vote-1", &new_msg("弃权")).await.unwrap();
        set_sender_agent(&pool, "vote-1", "voter-x").await.unwrap();
        // 未打标行（旧路径 / 打标失败的兜底面）
        create(&pool, "a-1", &new_msg("回答正文")).await.unwrap();

        let n = sweep_sender_for_channel_turn(&pool, "conv-sweep", "head-u1", "agent-1")
            .await
            .unwrap();
        assert_eq!(n, 1, "只 sweep 到未打标的一行");

        async fn sender_of(pool: &SqlitePool, id: &str) -> String {
            get_by_id(pool, id)
                .await
                .unwrap()
                .sender_agent_id
                .unwrap_or_default()
        }
        assert_eq!(
            sender_of(&pool, "vote-1").await,
            "voter-x",
            "出生打标不被 sweep 覆盖"
        );
        assert_eq!(
            sender_of(&pool, "a-1").await,
            "agent-1",
            "未打标行由 sweep 兜底"
        );
    }

    // ---- 回合制分页（list_by_turn_page）----

    async fn seed_turn_conv(pool: &SqlitePool, conv_id: &str) {
        sqlx::query(
            "INSERT OR IGNORE INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens, extra_params, sort_order, cache_prompt)
             VALUES ('agent-1', 'test-agent', 'anthropic', 'claude-test', '', '', 0.7, 1024, '{}', 0, 0)",
        )
        .execute(pool)
        .await
        .expect("seed agent");
        sqlx::query("INSERT INTO conversations (id, agent_id, title) VALUES (?, 'agent-1', 'turn page conv')")
            .bind(conv_id)
            .execute(pool)
            .await
            .expect("seed conversation");
    }

    fn turn_msg(conv: &str, role: &str, content: &str) -> NewMessage {
        NewMessage {
            conversation_id: conv.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            token_count: None,
            error: None,
            model: None,
        }
    }

    /// 建一个回合：真 user 锚 + N 条 assistant 行，返回锚 rowid。
    async fn seed_turn(pool: &SqlitePool, conv: &str, anchor_id: &str, assistant_rows: usize) -> i64 {
        create(pool, anchor_id, &turn_msg(conv, "user", &format!("问题 {anchor_id}")))
            .await
            .unwrap();
        for i in 0..assistant_rows {
            create(
                pool,
                &format!("{anchor_id}-a{i}"),
                &turn_msg(conv, "assistant", &format!("答 {anchor_id} #{i}")),
            )
            .await
            .unwrap();
        }
        get_by_id(pool, anchor_id).await.unwrap().rowid
    }

    /// 首载页含最近 turns 个回合全量 + has_more 游标（t2 起整回合，t1 留下页）。
    #[tokio::test]
    async fn turn_page_first_load_includes_tail_turns_with_cursor() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_turn_conv(&pool, "conv-tp1").await;
        seed_turn(&pool, "conv-tp1", "t1", 1).await; // 2 行
        let t2 = seed_turn(&pool, "conv-tp1", "t2", 2).await; // 3 行
        seed_turn(&pool, "conv-tp1", "t3", 3).await; // 4 行

        let page = list_by_turn_page(&pool, "conv-tp1", Some(2), None).await.unwrap();
        // 尾回合（未完成尾同规）全量含：t2 + t3 两回合 = 7 行
        assert_eq!(page.rows.len(), 7);
        assert_eq!(page.rows[0].id, "t2");
        assert!(page.rows.iter().all(|r| r.id != "t1"), "t1 回合留下页");
        assert!(page.has_more);
        assert_eq!(page.next_before_anchor_rowid, Some(t2));
    }

    /// turn-0 残留：终页最旧纳入 span 下界撤掉，首锚前的残留行并入终页
    /// （游标页同规——长会话终页几乎都是游标页）。
    #[tokio::test]
    async fn turn_page_terminal_residue_merges_before_first_anchor() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_turn_conv(&pool, "conv-tp2").await;
        // 首锚前的 assistant 残留行（崩溃恢复/占位残留形态）
        create(&pool, "r0", &turn_msg("conv-tp2", "assistant", "残留行")).await.unwrap();
        seed_turn(&pool, "conv-tp2", "t1", 0).await;
        let t2 = seed_turn(&pool, "conv-tp2", "t2", 1).await;
        seed_turn(&pool, "conv-tp2", "t3", 1).await;

        let p1 = list_by_turn_page(&pool, "conv-tp2", Some(2), None).await.unwrap();
        assert!(p1.has_more);
        assert_eq!(p1.next_before_anchor_rowid, Some(t2));

        let p2 = list_by_turn_page(&pool, "conv-tp2", Some(2), p1.next_before_anchor_rowid)
            .await
            .unwrap();
        let ids: Vec<&str> = p2.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["r0", "t1"], "残留行并入终页");
        assert!(!p2.has_more);
        assert_eq!(p2.next_before_anchor_rowid, None);
    }

    /// 首载即终页（短会话单 span）也撤下界——残留 + 全部回合一页全量。
    #[tokio::test]
    async fn turn_page_first_load_terminal_single_span_full() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_turn_conv(&pool, "conv-tp2b").await;
        create(&pool, "r0", &turn_msg("conv-tp2b", "assistant", "残留行")).await.unwrap();
        seed_turn(&pool, "conv-tp2b", "t1", 1).await;

        let page = list_by_turn_page(&pool, "conv-tp2b", None, None).await.unwrap();
        let ids: Vec<&str> = page.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["r0", "t1", "t1-a0"]);
        assert!(!page.has_more);
        assert_eq!(page.next_before_anchor_rowid, None);
    }

    /// 锚谓词同源锁：占位/空占位不作锚、纯图行是真锚（误伤 guard），且分页
    /// 锚集合与 list_turn_anchors 输出一致；连翻页拼回全部 5 行不重不漏。
    #[tokio::test]
    async fn turn_page_anchor_predicate_matches_list_turn_anchors() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_turn_conv(&pool, "conv-tp3").await;
        create(&pool, "u1", &turn_msg("conv-tp3", "user", "第一条")).await.unwrap();
        // tool_result 占位 user 行 → 非锚
        create(&pool, "u-tool", &turn_msg("conv-tp3", "user", "")).await.unwrap();
        update_content_blocks(&pool, "u-tool", r#"[{"type":"tool_result","tool_use_id":"t1"}]"#)
            .await
            .unwrap();
        // 空占位 user 行 → 非锚
        create(&pool, "u-empty", &turn_msg("conv-tp3", "user", "")).await.unwrap();
        // 纯图 user 行（content 空、blocks 非空且无 tool_result）→ 真锚（误伤 guard）
        create(&pool, "u-img", &turn_msg("conv-tp3", "user", "")).await.unwrap();
        update_content_blocks(
            &pool,
            "u-img",
            r#"[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}}]"#,
        )
        .await
        .unwrap();
        let u_img = get_by_id(&pool, "u-img").await.unwrap().rowid;
        create(&pool, "u2", &turn_msg("conv-tp3", "user", "第二条")).await.unwrap();
        let u2 = get_by_id(&pool, "u2").await.unwrap().rowid;

        let mut collected: Vec<String> = Vec::new();
        let mut cursor: Option<i64> = None;
        for _ in 0..5 {
            let page = list_by_turn_page(&pool, "conv-tp3", Some(1), cursor).await.unwrap();
            collected.extend(page.rows.iter().map(|r| r.id.clone()));
            if !page.has_more {
                assert_eq!(page.next_before_anchor_rowid, None);
                break;
            }
            cursor = page.next_before_anchor_rowid;
        }
        // 5 行恰一次：占位行随所属回合入页（u-tool/u-empty 落 u1 的 span 尾）
        let mut sorted = collected.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 5, "全部行恰一次：{collected:?}");
        assert_eq!(collected.first().map(String::as_str), Some("u2"), "首页 = 最新锚");
        assert_eq!(collected.last().map(String::as_str), Some("u-empty"), "终页含最旧行");

        // 同源锁：分页锚集合 == list_turn_anchors
        let anchors = list_turn_anchors(&pool, "conv-tp3").await.unwrap();
        let anchor_ids: Vec<&str> = anchors.iter().map(|a| a.message_id.as_str()).collect();
        assert_eq!(anchor_ids, vec!["u1", "u-img", "u2"]);
        // 游标链恰为锚子集（u-img 中间页）
        assert_eq!(cursor, Some(u_img));
        let _ = u2;
    }

    /// 行数闸诚实截断：3 回合 × 175 行、turns=2 → 每页恰一回合，连翻三页
    /// 拼回 525 行不重不漏；页界仍是回合边界。
    #[tokio::test]
    async fn turn_page_row_cap_truncates_honestly_and_reassembles() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_turn_conv(&pool, "conv-tp4").await;
        seed_turn(&pool, "conv-tp4", "t1", 174).await; // 175 行/回合
        let t2 = seed_turn(&pool, "conv-tp4", "t2", 174).await;
        let t3 = seed_turn(&pool, "conv-tp4", "t3", 174).await;

        let mut all: Vec<String> = Vec::new();
        let mut cursor: Option<i64> = None;
        let mut pages = 0;
        loop {
            let page = list_by_turn_page(&pool, "conv-tp4", Some(2), cursor).await.unwrap();
            pages += 1;
            // 行数闸（300）下每页只装得下 1 个 175 行回合
            assert_eq!(page.rows.len(), 175, "第 {pages} 页 = 恰一回合");
            assert!(page.rows[0].id.starts_with('t'), "页头是回合锚（页界=回合边界）");
            all.extend(page.rows.iter().map(|r| r.id.clone()));
            if !page.has_more {
                assert_eq!(page.next_before_anchor_rowid, None);
                break;
            }
            cursor = page.next_before_anchor_rowid;
            assert!(pages < 5, "死循环防御");
        }
        assert_eq!(pages, 3);
        let mut sorted = all.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 525, "525 行拼回不重不漏");
        let _ = (t2, t3);
    }

    /// 巨回合独立成页：单回合 350 行 > 300 闸——首个纳入 span 无条件全量
    /// （锚游标模型下 mid-span 截断无法表达）。
    #[tokio::test]
    async fn turn_page_giant_turn_gets_own_page() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_turn_conv(&pool, "conv-tp5").await;
        seed_turn(&pool, "conv-tp5", "giant", 349).await; // 350 行
        let small = seed_turn(&pool, "conv-tp5", "small", 1).await; // 2 行

        let p1 = list_by_turn_page(&pool, "conv-tp5", None, None).await.unwrap();
        assert_eq!(p1.rows.len(), 2, "首页只装小回合（巨回合探测超闸留下页）");
        assert!(p1.has_more);
        assert_eq!(p1.next_before_anchor_rowid, Some(small));

        let p2 = list_by_turn_page(&pool, "conv-tp5", None, p1.next_before_anchor_rowid)
            .await
            .unwrap();
        assert_eq!(p2.rows.len(), 350, "巨回合独立成页（首纳入 span 全量，超闸例外）");
        assert!(!p2.has_more);
        assert_eq!(p2.next_before_anchor_rowid, None);
    }

    /// 零锚点会话（全占位/空）首载 fallback 尾部 300 行。
    #[tokio::test]
    async fn turn_page_zero_anchor_conversation_falls_back_to_tail_rows() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_turn_conv(&pool, "conv-tp6").await;
        create(&pool, "r1", &turn_msg("conv-tp6", "assistant", "回复")).await.unwrap();
        create(&pool, "u-tool", &turn_msg("conv-tp6", "user", "")).await.unwrap();
        update_content_blocks(&pool, "u-tool", r#"[{"type":"tool_result","tool_use_id":"t1"}]"#)
            .await
            .unwrap();
        create(&pool, "u-empty", &turn_msg("conv-tp6", "user", "")).await.unwrap();

        let page = list_by_turn_page(&pool, "conv-tp6", None, None).await.unwrap();
        let ids: Vec<&str> = page.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["r1", "u-tool", "u-empty"], "fallback 尾部行 ASC");
        assert!(!page.has_more);
        assert_eq!(page.next_before_anchor_rowid, None);
    }

    /// 陈旧游标（锚窗口空）→ 空页而非 fallback（fallback 是更新行，前插会
    /// 重复合页）。
    #[tokio::test]
    async fn turn_page_stale_cursor_returns_empty_page_not_fallback() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_turn_conv(&pool, "conv-tp7").await;
        seed_turn(&pool, "conv-tp7", "u1", 1).await; // 会话有真实回合

        // 空池首条消息 rowid=1；锚 rowid < 1 不存在 → 窗口空
        let page = list_by_turn_page(&pool, "conv-tp7", Some(2), Some(1)).await.unwrap();
        assert!(page.rows.is_empty(), "空页，不是 fallback 的 2 行");
        assert!(!page.has_more);
        assert_eq!(page.next_before_anchor_rowid, None);
    }

    /// turns 钳制：≤0 或超 50 → 默认 8；合法边界原样生效。
    #[tokio::test]
    async fn turn_page_turns_clamping() {
        let pool = fresh_pool().await;
        sqlx::migrate!("./src/db/migrations").run(&pool).await.unwrap();
        seed_turn_conv(&pool, "conv-tp8").await;
        for i in 1..=5 {
            seed_turn(&pool, "conv-tp8", &format!("t{i}"), 0).await;
        }

        // 非法值 → 默认 8 → 5 回合一页全量
        for bad in [0, -3] {
            let page = list_by_turn_page(&pool, "conv-tp8", Some(bad), None).await.unwrap();
            assert_eq!(page.rows.len(), 5, "turns={bad} 回落默认 8");
            assert!(!page.has_more);
        }
        // 上限原样合法
        let page = list_by_turn_page(&pool, "conv-tp8", Some(50), None).await.unwrap();
        assert_eq!(page.rows.len(), 5);
        assert!(!page.has_more);
        // 正常分页
        let t4 = get_by_id(&pool, "t4").await.unwrap().rowid;
        let page = list_by_turn_page(&pool, "conv-tp8", Some(2), None).await.unwrap();
        let ids: Vec<&str> = page.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["t4", "t5"]);
        assert!(page.has_more);
        assert_eq!(page.next_before_anchor_rowid, Some(t4));
    }
}
