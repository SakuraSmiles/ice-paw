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
//! ## 压缩策略（截断不可避免，截断什么才是关键）
//!
//! 会话快照按被引会话自身状态选视图：
//! - **有滚动摘要**（MemoryStage 产物，锚点可解析）→「摘要 + 锚点后近窗」——
//!   滚动摘要本身就是从中段开头折叠的压缩产物，复用它零额外 LLM 成本，
//!   且比机械头尾保留更多关键决策
//! - **无摘要** → 头 2 轮（最初目标）+ 尾 8 轮（最新状态）+ 发送正文相关性
//!   补选最多 4 个中段轮（CJK bigram 打分，复用工具排序的 tokenize）
//!
//! **多引用总量护栏**：一条消息里所有会话/消息快照的潜在上限之和超过总预算时
//! 按比例收缩各自的字符上限（10 个会话引用不再 = 80K 字符灌入）。
//!
//! **诚实标注**：每处压缩都写明省略了多少、完整内容在哪——LLM 知道信息不全
//! 才会引导用户去源会话，而不是在缺口的幻觉上编造。
//!
//! 上限全 L1 默认（不进配置）：会话快照 8000 字符、消息快照 4000 字符
//! （assistant 组 ≤10 条）、agent 身份卡 ~500 字符、多引用总量 24000 字符。

use std::collections::HashSet;

use sqlx::SqlitePool;

use crate::db::models::MessageRow;
use crate::db::repo;
use crate::infra::protocol::ContentBlock;

/// 会话快照总字符上限（单引用）
const CONVERSATION_CHAR_CAP: usize = 8_000;
/// 会话快照保留的头部/尾部轮数（无摘要视图）
const CONVERSATION_HEAD_TURNS: usize = 2;
const CONVERSATION_TAIL_TURNS: usize = 8;
/// 会话快照的消息读取窗口（尾部最近 N 条；超出窗口的更早轮不进快照）
const CONVERSATION_MSG_WINDOW: i64 = 200;
/// 无摘要视图的相关性补选轮数上限（头尾之外，按发送正文打分）
const REF_RELEVANCE_EXTRA_TURNS: usize = 4;
/// 摘要视图中摘要正文占字符上限的比例（余量留给锚点后近窗）
const SUMMARY_SHARE: usize = 6; // /10，即 60%
/// 消息（assistant 组）快照总字符上限（单引用）
const MESSAGE_CHAR_CAP: usize = 4_000;
/// assistant 组快照最多并入的消息条数（前端「一次回答」组的后端对齐语义）
const GROUP_MAX_MESSAGES: usize = 10;
/// 多引用总量护栏：一条消息里所有会话/消息快照展开的总字符预算。
/// 潜在上限之和（会话数×8000 + 消息数×4000）超此值时按比例收缩各自上限；
/// agent 身份卡（~500 字符元数据）不参与——挤它只会丢委派引导，无信息收益。
const TOTAL_REF_CHAR_BUDGET: usize = 24_000;

/// 遍历 `blocks`，为每个 Reference 块在其后插入展开 Text 块（无 Reference 时原样返回）。
///
/// `query` = 发送正文（仅用户文本，不含附件提取文）——无摘要视图的相关性
/// 补选用它给中段轮打分。
pub(crate) async fn materialize_reference_blocks(
    pool: &SqlitePool,
    current_conv_id: &str,
    blocks: Vec<ContentBlock>,
    query: &str,
) -> Vec<ContentBlock> {
    // 预扫：多引用总量护栏——潜在上限之和超总预算时按比例收缩（静态预分摊，
    // 比「先展开再截断」可预期：不会把已渲染的快照拦腰砍断）
    let n_conv = refs_of_kind(&blocks, "conversation");
    let n_msg = refs_of_kind(&blocks, "message");
    let potential = n_conv * CONVERSATION_CHAR_CAP + n_msg * MESSAGE_CHAR_CAP;
    let scale = if potential > TOTAL_REF_CHAR_BUDGET {
        TOTAL_REF_CHAR_BUDGET as f64 / potential as f64
    } else {
        1.0
    };
    let conv_cap = ((CONVERSATION_CHAR_CAP as f64) * scale) as usize;
    let msg_cap = ((MESSAGE_CHAR_CAP as f64) * scale) as usize;

    let mut out = Vec::with_capacity(blocks.len() + n_conv + n_msg);
    let mut touched = false;
    for b in blocks {
        let expansion = if let ContentBlock::Reference {
            ref_kind,
            target_id,
            display,
        } = &b
        {
            touched = true;
            Some(
                expand_one(pool, current_conv_id, ref_kind, target_id, display, query, conv_cap, msg_cap)
                    .await,
            )
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

fn refs_of_kind(blocks: &[ContentBlock], kind: &str) -> usize {
    blocks
        .iter()
        .filter(|b| {
            matches!(b, ContentBlock::Reference { ref_kind, .. } if ref_kind == kind)
        })
        .count()
}

/// 单个引用的展开文本（含失效降级；永不 Err）。
#[allow(clippy::too_many_arguments)]
async fn expand_one(
    pool: &SqlitePool,
    current_conv_id: &str,
    ref_kind: &str,
    target_id: &str,
    display: &str,
    query: &str,
    conv_cap: usize,
    msg_cap: usize,
) -> String {
    let snapshot = match ref_kind {
        "conversation" => expand_conversation(pool, target_id, query, conv_cap).await,
        "agent" => expand_agent(pool, target_id).await,
        "message" => expand_message(pool, current_conv_id, target_id, msg_cap).await,
        _ => None, // 未知类型（前端版本不匹配）：按失效处理
    };
    snapshot.unwrap_or_else(|| format!("[引用已失效：{display}]"))
}

// =========================================================================
// @会话：摘要视图（有滚动摘要）或 头尾 + 相关性补选（无摘要）
// =========================================================================

async fn expand_conversation(
    pool: &SqlitePool,
    conv_id: &str,
    query: &str,
    char_cap: usize,
) -> Option<String> {
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
    let window_full = msgs.len() as i64 >= CONVERSATION_MSG_WINDOW;

    let mut out = String::with_capacity(1024);
    out.push_str(&format!(
        "<referenced_conversation id=\"{conv_id}\">\n会话「{title}」（agent：{agent_name}，共 {ge}{total_turns} 轮，最后活动 {ts}）。以下为压缩快照，非完整记录：\n",
        title = if conv.title.is_empty() { "（未命名）" } else { &conv.title },
        ge = if window_full { "≥" } else { "" },
        ts = conv.updated_at,
    ));

    // 会话名片：事件日志的结构化投影（计划终态 + 产物清单）——高价值密度
    // 优先于对话流水；消息表里没有这些信息（只活在 session_events）
    if let Some(card) = build_conversation_card(pool, conv_id).await {
        let remaining = char_cap.saturating_sub(out.chars().count());
        if card.chars().count() <= remaining {
            out.push_str(&card);
        } else {
            // 名片挤不进剩余预算（多引用分摊后）：截短保留而非整体丢弃
            out.push_str(&truncate_chars(&card, remaining.max(200)));
            out.push_str("\n（名片超预算已截短）\n");
        }
    }

    // 摘要视图：锚点可解析（Phase 2 摘要行恒带 rowid 锚；旧版行 None → 走头尾
    // 视图保守处理，不猜覆盖范围）。残余定位用 rowid——MessageRow 恒有，与
    // seq/rowid 双锚的 LLM 视图连续性无关。
    let summary_state = repo::summary::get_latest_summary_state(pool, conv_id)
        .await
        .ok()
        .flatten();
    let residual_start = summary_state
        .as_ref()
        .and_then(|s| s.covered_until_rowid)
        .map(|anchor| msgs.iter().position(|m| m.rowid > anchor).unwrap_or(msgs.len()));

    let compressed = if let (Some(state), Some(residual)) = (summary_state.as_ref(), residual_start)
    {
        render_summary_view(&mut out, state, &msgs, &turns, residual, char_cap)
    } else {
        render_headtail_view(&mut out, &msgs, &turns, query, char_cap)
    };

    // 钻取提示（仅压缩过才给——全量保留的快照无需工具，避免噪声）：
    // 把「塞多少」的决策从发送时刻移到模型使用时刻（read_attachment_page 同构）
    if compressed {
        out.push_str(&format!(
            "\n（快照已压缩；如需完整内容，调用 read_reference(target_id=\"{conv_id}\", page=1) 按页读取）\n"
        ));
    }

    out.push_str("</referenced_conversation>");
    Some(out)
}

// =========================================================================
// 会话名片：session_events 的结构化投影（计划终态 + 产物清单）
// =========================================================================

/// 名片段数上限（计划条目 / 产物各 ≤ 此值，超出标省略计数）
const CARD_MAX_ITEMS: usize = 8;
const CARD_MAX_ARTIFACTS: usize = 12;
/// 单条计划/产物的字符截断
const CARD_ITEM_CHARS: usize = 80;

/// 从事件日志派生「会话名片」：计划终态（plan_updated last-wins）+ 产物清单
/// （write/edit/move/create 工具的路径参数）。两段全空（聊天型会话）→ None，
/// 零噪声。这是 MessageRow 之外的增量信息——计划与工具调用不进消息表，
/// 只活在事件日志里。
async fn build_conversation_card(pool: &SqlitePool, conv_id: &str) -> Option<String> {
    let plan = repo::session_event::last_plan_payload(pool, conv_id)
        .await
        .ok()
        .flatten()
        .and_then(|p| serde_json::from_str::<serde_json::Value>(&p).ok());
    let plan_items = plan
        .as_ref()
        .and_then(|v| v.get("items").and_then(|i| i.as_array()))
        .map(|a| a.as_slice())
        .unwrap_or(&[]);

    let artifacts: Vec<String> = repo::session_event::list_successful_tool_calls(pool, conv_id)
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|(name, args)| artifact_path(name, args))
        .collect();
    let mut deduped: Vec<String> = Vec::new();
    for p in artifacts {
        if !deduped.contains(&p) {
            deduped.push(p);
        }
    }

    if plan_items.is_empty() && deduped.is_empty() {
        return None;
    }

    let mut card = String::from("【会话名片】\n");
    if !plan_items.is_empty() {
        let done = plan_items
            .iter()
            .filter(|it| it.get("status").and_then(|s| s.as_str()) == Some("done"))
            .count();
        card.push_str(&format!("计划（{done}/{} 完成）：\n", plan_items.len()));
        for it in plan_items.iter().take(CARD_MAX_ITEMS) {
            let status = it.get("status").and_then(|s| s.as_str()).unwrap_or("pending");
            let mark = match status {
                "done" => "✓",
                "in_progress" => "▶",
                _ => "○",
            };
            let text = it.get("text").and_then(|t| t.as_str()).unwrap_or("");
            card.push_str(&format!("  {mark} {}\n", truncate_chars(text, CARD_ITEM_CHARS)));
        }
        if plan_items.len() > CARD_MAX_ITEMS {
            card.push_str(&format!(
                "  …（另有 {} 条，见 read_reference 或源会话）\n",
                plan_items.len() - CARD_MAX_ITEMS
            ));
        }
    }
    if !deduped.is_empty() {
        card.push_str(&format!("产物（{} 个文件）：\n", deduped.len()));
        for p in deduped.iter().take(CARD_MAX_ARTIFACTS) {
            card.push_str(&format!("  - {}\n", truncate_chars(p, CARD_ITEM_CHARS)));
        }
        if deduped.len() > CARD_MAX_ARTIFACTS {
            card.push_str(&format!(
                "  …（另有 {} 个，见 read_reference 或源会话）\n",
                deduped.len() - CARD_MAX_ARTIFACTS
            ));
        }
    }
    Some(card)
}

/// 从工具调用参数提取产物路径（write/edit/create_directory → path；
/// move_file → destination）。非产物类工具 / 解析失败 → None。
fn artifact_path(tool_name: &str, arguments: &str) -> Option<String> {
    let key = match tool_name {
        "write_file" | "edit_file" | "create_directory" => "path",
        "move_file" => "destination",
        _ => return None,
    };
    let v: serde_json::Value = serde_json::from_str(arguments).ok()?;
    let p = v.get(key)?.as_str()?.trim().to_string();
    (!p.is_empty()).then_some(p)
}

/// 摘要视图：滚动摘要（60% 子预算，防 16K 自适应摘要挤掉近窗）+ 锚点后近窗
/// 尾部轮。摘要从中段开头折叠——它就是比头尾节选更好的中段压缩，复用零成本。
/// 返回值 = 是否发生压缩（早期内容被摘要替代即算，恒 true——钻取提示依据）。
fn render_summary_view(
    out: &mut String,
    state: &crate::db::repo::summary::SummaryState,
    msgs: &[MessageRow],
    turns: &[usize],
    residual_start: usize,
    char_cap: usize,
) -> bool {
    let summary_cap = char_cap / 10 * SUMMARY_SHARE;
    out.push_str("【早期内容摘要】（源会话自动折叠生成，覆盖近期内容之前的对话）\n");
    out.push_str(&truncate_chars(&state.text, summary_cap));
    if state.text.chars().count() > summary_cap {
        out.push_str("\n（摘要超长已截短）");
    }
    out.push_str("\n―――― 摘要之后的近期内容 ――――\n");

    // 近窗尾部轮选择：残余轮数 > 尾轮数时保尾部，中段省略标注
    let residual_turns: Vec<usize> = turns.iter().copied().filter(|&i| i >= residual_start).collect();
    let tail_start = residual_turns
        .len()
        .checked_sub(CONVERSATION_TAIL_TURNS)
        .filter(|&omit| omit > 0)
        .map(|omit| {
            let start = residual_turns[omit];
            out.push_str(&format!(
                "…（摘要之后省略 {omit} 轮，完整内容在源会话）\n"
            ));
            start
        })
        .unwrap_or(residual_start);

    for m in msgs.iter().skip(tail_start) {
        if m.role == "system" {
            continue; // 摘要行不进消息流（上方已显式渲染受控版本）
        }
        let line = render_message_line(m);
        if out.chars().count() + line.chars().count() > char_cap {
            out.push_str("…（已达快照长度上限，后续省略）\n");
            return true;
        }
        out.push_str(&line);
    }
    true
}

/// 头尾视图：头 2 轮（最初目标）+ 尾 8 轮（最新状态）+ 发送正文相关性补选
/// 最多 [`REF_RELEVANCE_EXTRA_TURNS`] 个中段轮（保序；query 为空时纯头尾）。
/// 返回值 = 是否发生压缩（省略段/上限截断/窗口截断）。
fn render_headtail_view(
    out: &mut String,
    msgs: &[MessageRow],
    turns: &[usize],
    query: &str,
    char_cap: usize,
) -> bool {
    // 全量轮数 ≤ 头+尾（或零轮）时整段保留（零压缩——除非撞字符上限）
    if turns.len() <= CONVERSATION_HEAD_TURNS + CONVERSATION_TAIL_TURNS || turns.is_empty() {
        for m in msgs {
            if m.role == "system" {
                continue;
            }
            let line = render_message_line(m);
            if out.chars().count() + line.chars().count() > char_cap {
                out.push_str("…（已达快照长度上限，后续省略）\n");
                return true;
            }
            out.push_str(&line);
        }
        return false;
    }

    let n = turns.len();
    let mut keep_turn = vec![true; n];
    let mid_range = CONVERSATION_HEAD_TURNS..n - CONVERSATION_TAIL_TURNS;
    for i in mid_range.clone() {
        keep_turn[i] = false;
    }

    // 相关性补选：中段轮按「发送正文 token 在轮文本中命中数」打分（去重 token，
    // CJK bigram 与工具排序同语义），取分最高的 ≤4 轮（同分靠前优先）
    let tokens: HashSet<String> = crate::harness::scoring::tokenize(query).into_iter().collect();
    if !tokens.is_empty() {
        let mut scored: Vec<(u32, usize)> = mid_range
            .clone()
            .filter_map(|ti| {
                let span = turn_span(turns, ti, msgs.len());
                let text = msgs[span]
                    .iter()
                    .map(|m| m.content.to_lowercase())
                    .collect::<Vec<_>>()
                    .join(" ");
                let score = tokens.iter().filter(|t| text.contains(t.as_str())).count() as u32;
                (score > 0).then_some((score, ti))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, ti) in scored.into_iter().take(REF_RELEVANCE_EXTRA_TURNS) {
            keep_turn[ti] = true;
        }
    }

    // 渲染：消息行按所属轮的取舍进退；跨省略段时标注该段省略的轮数（多段各自计数）
    let mut omitted_run = 0usize;
    let mut in_omission = false;
    for (i, m) in msgs.iter().enumerate() {
        let ord = turn_ord(i, turns);
        if !keep_turn[ord] {
            if turns.get(ord) == Some(&i) {
                omitted_run += 1; // 轮锚点进入省略段才计数（占位/assistant 行不膨胀计数）
            }
            in_omission = true;
            continue;
        }
        if in_omission {
            out.push_str(&format!(
                "…（省略 {omitted_run} 轮，完整内容在源会话）\n"
            ));
            omitted_run = 0;
            in_omission = false;
        }
        if m.role == "system" {
            continue;
        }
        let line = render_message_line(m);
        if out.chars().count() + line.chars().count() > char_cap {
            out.push_str("…（已达快照长度上限，后续省略）\n");
            return true;
        }
        out.push_str(&line);
    }
    true
}

/// 消息所属轮号（0 基）：`turns[k] ≤ i < turns[k+1]` → k；首锚点前的窗口
/// 前缀消息（窗口恰好切进某轮中段时）归属第 0 轮，与首锚点同进退。
fn turn_ord(i: usize, turns: &[usize]) -> usize {
    turns.iter().rposition(|&t| t <= i).unwrap_or(0)
}

/// 第 `ti` 轮的消息区间 `[turns[ti], turns[ti+1])`（末轮到窗口尾）
fn turn_span(turns: &[usize], ti: usize, len: usize) -> std::ops::Range<usize> {
    let start = turns[ti];
    let end = turns.get(ti + 1).copied().unwrap_or(len);
    start..end
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
    char_cap: usize,
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
                    if used > char_cap {
                        out.push_str("…（已达快照长度上限，完整内容在源会话）\n");
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
/// pub(crate)：read_reference 工具的全文分页与快照渲染共用同一消息行形态。
pub(crate) fn render_message_line(m: &MessageRow) -> String {
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
                avatar: None,
                emoji: None,
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
        let text = expand_conversation(&pool, "c1", "", 8_000)
            .await
            .expect("展开成功");
        assert!(text.contains("会话「设计讨论」"));
        assert!(text.contains("agent：助手"));
        assert!(text.contains("共 12 轮"));
        assert!(text.contains("压缩快照")); // 头部诚实标注
        assert!(text.contains("第1个问题")); // 头部保留
        assert!(text.contains("第2个回答"));
        assert!(!text.contains("第3个问题")); // 中段省略（第 3、4 轮）
        assert!(text.contains("第12个回答")); // 尾部保留
        assert!(text.contains("省略 2 轮")); // 省略段计数标注

        // 不存在的会话 / 空会话 → None
        assert!(expand_conversation(&pool, "nope", "", 8_000).await.is_none());
        seed_conv(&pool, "c2", "空", "a1").await;
        assert!(expand_conversation(&pool, "c2", "", 8_000).await.is_none());
    }

    #[tokio::test]
    async fn expand_conversation_summary_view() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "助手", "").await;
        seed_conv(&pool, "c1", "长会话", "a1").await;
        // 14 轮；第 6 轮后插入滚动摘要（锚 = a6 的 rowid，Phase 2 摘要行恒带锚）
        for t in 1..=14 {
            seed_msg(&pool, &format!("u{t}"), "c1", "user", &format!("第{t}个问题")).await;
            seed_msg(&pool, &format!("a{t}"), "c1", "assistant", &format!("第{t}个回答")).await;
        }
        let a6_rowid: i64 = sqlx::query_scalar("SELECT rowid FROM messages WHERE id = 'a6'")
            .fetch_one(&pool)
            .await
            .unwrap();
        repo::summary::insert_summary_message(
            &pool,
            "c1",
            "用户在做性能优化，前几轮定位到数据库慢查询",
            None,
            a6_rowid,
        )
        .await
        .unwrap();

        let text = expand_conversation(&pool, "c1", "", 8_000)
            .await
            .expect("展开成功");
        // 摘要视图：摘要显式渲染 + 锚点后近窗（残余 8 轮 ≤ 尾轮数 → 全保留）
        assert!(text.contains("【早期内容摘要】"));
        assert!(text.contains("用户在做性能优化"));
        assert!(text.contains("摘要之后的近期内容"));
        assert!(text.contains("第7个问题")); // 残余首轮
        assert!(text.contains("第14个回答")); // 残余末轮
        // 被摘要覆盖的早期轮不再以消息行出现（内容在摘要里，不双份渲染）
        assert!(!text.contains("第1个问题"));
        assert!(!text.contains("第3个问题"));
        // system 摘要行不进消息流（受控版本已显式渲染）
        assert!(!text.contains("[system]"));
    }

    #[tokio::test]
    async fn expand_conversation_relevance_fill() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "助手", "").await;
        seed_conv(&pool, "c1", "排障会话", "a1").await;
        // 16 轮，第 5 轮内容与发送正文同主题；query 为空时它本会被头尾省略
        for t in 1..=16 {
            let q = if t == 5 { "数据库连接池怎么配" } else { &format!("第{t}个问题") };
            seed_msg(&pool, &format!("u{t}"), "c1", "user", q).await;
            seed_msg(&pool, &format!("a{t}"), "c1", "assistant", &format!("第{t}个回答")).await;
        }

        // query 命中 → 第 5 轮被补选进快照（CJK bigram 打分，复用工具排序分词）
        let text = expand_conversation(&pool, "c1", "帮我看看数据库连接池", 8_000)
            .await
            .expect("展开成功");
        assert!(text.contains("数据库连接池怎么配"));
        assert!(text.contains("第1个问题")); // 头部仍在
        assert!(text.contains("第16个回答")); // 尾部仍在
        assert!(text.contains("省略")); // 头尾之外仍有省略段标注
        assert!(!text.contains("第4个问题")); // 未命中且无摘要 → 仍省略

        // 空 query → 纯头尾（既有行为不变）
        let plain = expand_conversation(&pool, "c1", "", 8_000).await.unwrap();
        assert!(!plain.contains("数据库连接池怎么配"));
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
        let text = expand_message(&pool, "c1", "u1", 4_000).await.expect("展开");
        assert!(text.contains("[user]: 帮我看看"));
        assert!(!text.contains("第一段"));

        // assistant 组：从 s1 起连续到 role 变化（s1+s2，不含 u2）
        let text = expand_message(&pool, "c1", "s1", 4_000).await.expect("展开");
        assert!(text.contains("[assistant]: 第一段"));
        assert!(text.contains("[assistant]: 第二段"));
        assert!(!text.contains("再来"));

        // 跨会话引用 → None（后端兜底，前端入口本就只给当前会话）
        seed_conv(&pool, "c9", "别的", "a1").await;
        assert!(expand_message(&pool, "c9", "u1", 4_000).await.is_none());
        // 不存在 → None
        assert!(expand_message(&pool, "c1", "nope", 4_000).await.is_none());
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
        let out = materialize_reference_blocks(&pool, "c1", blocks, "看看这个").await;
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
        let out = materialize_reference_blocks(&pool, "c1", blocks, "正文").await;
        assert_eq!(out.len(), 1);
    }

    #[tokio::test]
    async fn conversation_card_from_events() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "助手", "").await;
        seed_conv(&pool, "c1", "工程会话", "a1").await;
        // 3 轮短会话（≤ 头+尾 → 全量保留，零压缩 → 不该有钻取提示）
        for t in 1..=3 {
            seed_msg(&pool, &format!("u{t}"), "c1", "user", &format!("第{t}个问题")).await;
            seed_msg(&pool, &format!("a{t}"), "c1", "assistant", &format!("第{t}个回答")).await;
        }
        // 事件侧：计划终态（全量快照语义）+ 产物工具调用（含失败与非产物工具）
        let ev = |kind: &str, payload: String| {
            let kind = kind.to_string();
            let pool = pool.clone();
            async move {
                repo::session_event::append(&pool, "c1", &kind, "agent:a1", Some("t1"), None, &payload)
                    .await
                    .unwrap();
            }
        };
        ev(
            "plan_updated",
            r#"{"items":[{"text":"搭脚手架","status":"done"},{"text":"写核心模块","status":"in_progress"},{"text":"补测试","status":"pending"}]}"#.into(),
        )
        .await;
        ev("tool_execution", r#"{"tool_call_id":"c1","tool_name":"write_file","arguments":"{\"path\":\"src/main.rs\",\"content\":\"x\"}","is_error":false,"duration_ms":10}"#.into()).await;
        ev("tool_execution", r#"{"tool_call_id":"c2","tool_name":"edit_file","arguments":"{\"path\":\"src/lib.rs\"}","is_error":false,"duration_ms":10}"#.into()).await;
        ev("tool_execution", r#"{"tool_call_id":"c3","tool_name":"move_file","arguments":"{\"source\":\"a.md\",\"destination\":\"docs/a.md\"}","is_error":false,"duration_ms":10}"#.into()).await;
        ev("tool_execution", r#"{"tool_call_id":"c4","tool_name":"write_file","arguments":"{\"path\":\"bad.rs\"}","is_error":true,"duration_ms":10}"#.into()).await;
        ev("tool_execution", r#"{"tool_call_id":"c5","tool_name":"run_command","arguments":"{\"cmd\":\"ls\"}","is_error":false,"duration_ms":10}"#.into()).await;

        let text = expand_conversation(&pool, "c1", "", 8_000)
            .await
            .expect("展开成功");
        // 名片：计划终态 + 产物清单（事件投影，消息表里没有的信息）
        assert!(text.contains("【会话名片】"));
        assert!(text.contains("计划（1/3 完成）"));
        assert!(text.contains("✓ 搭脚手架"));
        assert!(text.contains("▶ 写核心模块"));
        assert!(text.contains("○ 补测试"));
        assert!(text.contains("产物（3 个文件）"));
        assert!(text.contains("src/main.rs"));
        assert!(text.contains("src/lib.rs"));
        assert!(text.contains("docs/a.md")); // move 取 destination
        assert!(!text.contains("bad.rs")); // 失败调用不算产物
        // 短会话零压缩 → 无钻取提示（避免噪声）
        assert!(!text.contains("read_reference"));

        // 纯聊天会话（无计划无产物）→ 零名片
        seed_conv(&pool, "c2", "闲聊", "a1").await;
        seed_msg(&pool, "c2-u1", "c2", "user", "在吗").await;
        seed_msg(&pool, "c2-a1", "c2", "assistant", "在的").await;
        let plain = expand_conversation(&pool, "c2", "", 8_000).await.unwrap();
        assert!(!plain.contains("【会话名片】"));
    }

    #[tokio::test]
    async fn compressed_snapshot_carries_drill_hint() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "助手", "").await;
        seed_conv(&pool, "c1", "长会话", "a1").await;
        // 12 轮（> 头2+尾8）→ 有省略段 → 尾部应带钻取提示
        for t in 1..=12 {
            seed_msg(&pool, &format!("u{t}"), "c1", "user", &format!("第{t}个问题")).await;
            seed_msg(&pool, &format!("a{t}"), "c1", "assistant", &format!("第{t}个回答")).await;
        }
        let text = expand_conversation(&pool, "c1", "", 8_000).await.unwrap();
        assert!(text.contains("省略 2 轮"));
        assert!(text.contains("read_reference(target_id=\"c1\""));
    }

    #[tokio::test]
    async fn materialize_total_budget_scales_caps() {
        let pool = test_pool().await;
        seed_agent(&pool, "a1", "助手", "").await;
        seed_conv(&pool, "c1", "超长会话", "a1").await;
        // 40 轮 × 每条 600 字符 ≈ 48K 字符 >> 单引用上限——单引用也会触顶
        let long = "长".repeat(600);
        for t in 1..=40 {
            seed_msg(&pool, &format!("u{t}"), "c1", "user", &format!("第{t}{long}")).await;
            seed_msg(&pool, &format!("a{t}"), "c1", "assistant", &format!("答{t}{long}")).await;
        }

        // 单引用：8000 上限内
        let single = materialize_reference_blocks(
            &pool,
            "c1",
            vec![ContentBlock::reference("conversation", "c1", "超长#0001")],
            "",
        )
        .await;
        let snap1 = single[1].as_text().unwrap();
        assert!(snap1.chars().count() <= 8_200);
        assert!(snap1.contains("已达快照长度上限"));

        // 4 个会话引用：潜在 32K > 总预算 24K → 各自收缩到 6000
        let refs: Vec<ContentBlock> = (0..4)
            .map(|i| ContentBlock::reference("conversation", "c1", format!("超长#000{i}")))
            .collect();
        let multi = materialize_reference_blocks(&pool, "c1", refs, "").await;
        for (i, b) in multi.iter().enumerate() {
            if let Some(text) = b.as_text() {
                assert!(
                    text.chars().count() <= 6_200,
                    "第 {i} 个快照应 ≤ ~6000：{}",
                    text.chars().count()
                );
            }
        }
    }
}
