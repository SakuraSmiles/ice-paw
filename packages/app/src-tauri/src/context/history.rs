//! 历史消息加载
//!
//! 从 `commands/chat_context.rs` 迁入（W5.3）。
//!
//! 提供历史消息窗口配置和从历史行转换到 `Vec<ChatMessage>` 的逻辑。
//!
//! A3-2 变更：窗口大小可由 Agent 的 `max_history_messages` 字段覆盖；
//! 该字段为 `None` 时回退到本模块的 [`DEFAULT_HISTORY_WINDOW`]。

use std::collections::{HashMap, HashSet};

use crate::db::models::MessageRow;
use crate::db::repo::summary::SUMMARY_PREFIX;
use crate::infra::protocol::{ChatMessage, ContentBlock};

/// 系统默认历史窗口大小（最近 N 条消息）
///
/// 当 Agent 未配置 `max_history_messages` 时使用该默认值。
/// 集中定义以便全栈统一：后端 [`load_history`] + Pipeline + 前端 placeholder
/// 都引用此值（W6.4 Sprint #6.4 沿用历史行为，避免破坏既有 UI 体验）。
pub const DEFAULT_HISTORY_WINDOW: usize = 20;

/// 从 Agent 配置解析得到有效窗口大小
///
/// - `None`（Agent 未配置） → 系统默认 [`DEFAULT_HISTORY_WINDOW`]
/// - 非法值（<= 0 或过大）→ 退回到系统默认
///
/// `max_history_messages` 在 Rust 侧是 `Option<i32>`，
/// 但历史窗口作为「最近 N 条」必须 >= 1，所以兜底到 1；过大值
/// 由调用方在 DB 加载阶段限制（见 `repo::message::MAX_LIMIT`）。
pub fn resolve_window(agent_max: Option<i32>) -> usize {
    match agent_max {
        Some(n) if n > 0 => n as usize,
        // n <= 0 视为非法，回退默认
        _ => DEFAULT_HISTORY_WINDOW,
    }
}

/// 将历史消息行转换为 `Vec<ChatMessage>`（仅文本）
///
/// - 跳过非 user/assistant/system 角色（如 `tool`）
/// - 仅取 `MessageRow.content`（文本），多模态历史 TODO
/// - 如果提供 `window`，仅保留**最近** `window` 条（按切片顺序尾部）
///
/// 设计：
/// - **窗口化在 Stage 内**完成（而非 DB 加载侧）：调用者可以一次加载
///   足够多的历史，未来 A3-4 摘要阶段可读取完整历史，窗口只在最终
///   注入 LLM 时应用。这是 A3-2 设计原则「窗口大小按 Agent 配置」
///   的正确层级。
/// - 当调用方传 `None`（典型场景：未走 PipelineRunner 的内部调用）
///   时保持向后兼容，不过滤。
#[cfg(test)]
pub(crate) fn load_history(history: &[MessageRow]) -> Vec<ChatMessage> {
    load_history_with_window(history, None)
}

/// 带窗口的版本：A3-2 引入，供 [`crate::context::stages::HistoryStage`] 使用。
///
/// `window = Some(n)` → 仅保留最后 n 条
/// `window = None`    → 不过滤（向后兼容）
///
/// P2-2 (G1)：当 [`MessageRow::content_blocks`] 非空时，从该 JSON 还原完整
/// 多模态消息（含 `ContentBlock::Image`），否则回退到纯文本 [`MessageRow::content`]，
/// 以兼容旧消息（`content_blocks = "[]"`）。
pub(crate) fn load_history_with_window(
    history: &[MessageRow],
    window: Option<usize>,
) -> Vec<ChatMessage> {
    // 窗口裁剪：仅在有窗口且需要时执行
    let slice: &[MessageRow] = match window {
        Some(n) if n < history.len() => &history[history.len() - n..],
        _ => history,
    };

    let mut messages = Vec::with_capacity(slice.len());
    for msg in slice {
        let role = match msg.role.as_str() {
            // 双注入修复（Phase 2）：摘要行（role=system + SUMMARY_PREFIX 前缀）不进
            // history——它由 MemoryStage 经 ctx.summary 唯一注入。这里跳过避免重复。
            "system" if msg.content.starts_with(SUMMARY_PREFIX) => continue,
            "user" | "assistant" | "system" => msg.role.clone(),
            _ => continue,
        };
        // Phase 2：记录源 rowid，供 MemoryStage 按值定位摘要覆盖切断点。
        let source_rowid = Some(msg.rowid);

        // P2-2 G1: 优先从 content_blocks 还原多模态消息。
        // 空数组 / 无效 JSON / 解析失败 → 回退到纯文本（兼容旧消息）。
        // 安全网：把历史 ToolUse 的 name 合规化为 `^[a-zA-Z0-9_-]+$`——旧版持久化了
        // `{中文server名}.{工具名}`（违反 OpenAI/Anthropic function-name 正则，deepseek
        // 等会 400）。仅作用于发给 LLM 的请求 copy；UI 消息加载走独立路径，不受影响。
        let blocks: Vec<ContentBlock> = parse_content_blocks(&msg.content_blocks)
            .into_iter()
            .map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => ContentBlock::ToolUse {
                    id,
                    name: sanitize_tool_name(&name),
                    input,
                },
                other => other,
            })
            .collect();
        if blocks.is_empty() {
            messages.push(ChatMessage {
                role,
                content: vec![ContentBlock::text(msg.content.clone())],
                source_rowid,
            });
            continue;
        }

        // 规范化：assistant 消息不应包含 ToolResult（Anthropic 协议要求 ToolResult
        // 位于 user 消息）。历史持久化时可能把同一轮的 tool_use + tool_result 合并
        // 进了 assistant 消息，这里在加载层拆开，避免发给 LLM 时触发
        // "tool result's tool id not found"（MiniMax 兼容端点 400）。
        // 拆分出的两条子消息共享同一 source_rowid（源自同一 MessageRow）。
        if role == "assistant" {
            let (asst_blocks, result_blocks): (Vec<ContentBlock>, Vec<ContentBlock>) = blocks
                .into_iter()
                .partition(|b| !matches!(b, ContentBlock::ToolResult { .. }));
            if !asst_blocks.is_empty() {
                messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: asst_blocks,
                    source_rowid,
                });
            }
            if !result_blocks.is_empty() {
                messages.push(ChatMessage {
                    role: "user".into(),
                    content: result_blocks,
                    source_rowid,
                });
            }
        } else {
            messages.push(ChatMessage {
                role,
                content: blocks,
                source_rowid,
            });
        }
    }
    sanitize_history(messages)
}

/// 把工具名规整为 OpenAI/Anthropic function-name 合规形式 `^[a-zA-Z0-9_-]+$`。
///
/// 历史 ToolUse 的 name 可能是旧版持久化的 `{中文server名}.{工具名}`
/// （中文 + 点号均违规 → deepseek 等 OpenAI 兼容端点 400）。这里就地剥离
/// 非合规字符；它是**只读上下文**（不参与 dispatch，sanitize_history 按
/// `tool_use_id` 配对），故改名不影响结构。已是合规名（含新版 `t{idx}_...`）原样返回。
fn sanitize_tool_name(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    s.extend(
        name.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-'),
    );
    if s.is_empty() {
        s.push_str("tool");
    }
    s
}

/// 净化历史消息，确保 Anthropic 协议合规（通用，适用于所有兼容端点）。
///
/// 防御历史数据的五类违规（多由旧版持久化遗留、工具超时/出错或窗口裁剪边界引入），
/// 确保发给任意 Anthropic 兼容 LLM 的历史都协议合规——严格校验的端点（如 MiniMax）
/// 遇到违规会直接 400（"tool call result does not follow tool call" / "tool id not found"），
/// 宽松端点也会行为异常：
/// 1. **重复 tool_use id** → 仅保留首个（旧版可能把同一 tool_use 写多份）
/// 2. **孤儿 tool_result**（tool_use_id 不在窗口内）→ 丢弃（裁剪裁掉 tool_use 留 tool_result）
/// 3. **孤儿 tool_use**（窗口内无配对 tool_result）→ 丢弃（工具超时/出错未补结果；
///    严格的 MiniMax 端点对「有 tool_call 无 tool_result」会 400）
/// 4. **空白消息**（空 assistant 占位 / 错误遗留，序列化为 `content:""` 破坏协议）→ 丢弃
/// 5. **连续同角色消息** → 合并 content（协议要求 user/assistant 交替）
///
/// 纯函数 + 无副作用，便于单元测试。
pub(crate) fn sanitize_history(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    if messages.is_empty() {
        return messages;
    }

    // 窗口内存在配对 tool_result 的 tool_use id 集合（孤儿 tool_use 判定基准）
    let ids_with_result: HashSet<String> = messages
        .iter()
        .flat_map(|m| {
            m.content.iter().filter_map(|b| match b {
                ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
                _ => None,
            })
        })
        .collect();
    // 窗口内出现过的所有 tool_use id（孤儿 tool_result 判定基准）
    let valid_ids: HashSet<String> = messages
        .iter()
        .flat_map(|m| {
            m.content.iter().filter_map(|b| match b {
                ContentBlock::ToolUse { id, .. } => Some(id.clone()),
                _ => None,
            })
        })
        .collect();

    // 过滤每条消息：tool_use 去重 + 去孤儿；tool_result 去孤儿 + 去重；text 去空白。
    // 过滤后 content 为空的消息（如仅含孤儿 tool_use 的 assistant、或纯空白的占位）整体丢弃。
    let mut tool_use_seen: HashSet<String> = HashSet::new();
    let mut tool_result_seen: HashSet<String> = HashSet::new();
    let mut filtered: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    for msg in messages {
        let role = msg.role;
        // Phase 2：sanitize 重建消息时保留 source_rowid（合并连续同角色时保留首条的）。
        let source_rowid = msg.source_rowid;
        let mut content: Vec<ContentBlock> = Vec::new();
        for block in msg.content {
            let keep = match &block {
                // 必须有配对 tool_result，且未重复出现
                ContentBlock::ToolUse { id, .. } => {
                    ids_with_result.contains(id) && tool_use_seen.insert(id.clone())
                }
                // tool_use_id 必须在窗口内，且未重复出现
                ContentBlock::ToolResult { tool_use_id, .. } => {
                    valid_ids.contains(tool_use_id) && tool_result_seen.insert(tool_use_id.clone())
                }
                // 丢弃空白文本（空 assistant 占位会序列化成 content:""，破坏协议）
                ContentBlock::Text { text } => !text.trim().is_empty(),
                _ => true,
            };
            if keep {
                content.push(block);
            }
        }
        // assistant 消息必须含 Text 或 ToolUse——OpenAI 协议要求 assistant 的 content
        // 或 tool_calls 至少其一非空。孤儿 tool_use 被上文剔除后，assistant 可能仅剩
        // Thinking 块（thinking 在 OpenAI 序列化层被丢弃 → content=null 且无 tool_calls），
        // 这种消息发给 deepseek 等严格端点会 400「content or tool_calls must be set」，
        // 且一旦写入历史，每轮都带上 → 会话永久卡死。这里整体丢弃；丢弃后产生的连续
        // 同角色由下一步合并处理，不会破坏 user/assistant 交替。
        let has_text = content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { .. }));
        let has_tool_use = content
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
        let drop_msg = role == "assistant" && !has_text && !has_tool_use;
        if !content.is_empty() && !drop_msg {
            filtered.push(ChatMessage {
                role,
                content,
                source_rowid,
            });
        }
    }

    // 合并连续同角色（协议要求交替；裁剪边界或丢弃空消息后可能产生连续 user/assistant）。
    // 合并后，任何存活的 tool_result 其 tool_use 必然位于紧邻的前一条 assistant（因 tool_use
    // 与 tool_result 本就是相邻的两轮，丢空白/孤儿不会在二者之间插入异类消息）。
    let mut merged: Vec<ChatMessage> = Vec::with_capacity(filtered.len());
    for msg in filtered {
        if let Some(last) = merged.last_mut() {
            if last.role == msg.role {
                last.content.extend(msg.content);
                continue;
            }
        }
        merged.push(msg);
    }

    merged
}

// =========================================================================
// 失败工具调用折叠（v1，纯函数）
// =========================================================================

/// 连续相同失败调用的最小折叠阈值（含）。
///
/// 一个"连续重复失败" run 的长度 ≥ 此值时才折叠成 1 条摘要。
/// 设为 2：单次失败永远保留完整诊断，出现重复（卡死循环征兆）才压缩。
const TOOL_FAILURE_FOLD_THRESHOLD: usize = 2;

/// 折叠摘要里 args / 最近错误字段的最大字符数（超出截断加省略号）。
const FOLD_SUMMARY_FIELD_MAX: usize = 300;

/// 工具调用入参的"可比较键"。
///
/// - JSON 入参按**值**比较（对象 key 顺序无关，依赖 `serde_json::Value` 的
///   `PartialEq` 顺序无关特性）；模型用不同 key 顺序产出相同参数也能判等。
/// - 非 JSON（解析失败）回退到 trim 后的原串比较。
#[derive(Clone, Debug, PartialEq)]
enum InputKey {
    Json(serde_json::Value),
    Raw(String),
}

impl InputKey {
    fn new(input: &str) -> Self {
        match serde_json::from_str::<serde_json::Value>(input) {
            Ok(v) => InputKey::Json(v),
            Err(_) => InputKey::Raw(input.trim().to_string()),
        }
    }
}

#[derive(Clone)]
struct CallMeta {
    name: String,
    key: InputKey,
    /// 原始入参串（摘要展示用，避免显示规范化后的形式）。
    raw_input: String,
}

/// 把"连续 N 次（≥ [`TOOL_FAILURE_FOLD_THRESHOLD`]）同工具同参数的**失败**调用"
/// 压缩成 1 条摘要，避免卡死循环的失败记录占满历史窗口、诱发模型反复道歉。
///
/// # 性质
/// - **只读 / 非破坏 / 幂等**：仅变换入参消息列表，不触碰 DB 或 UI。
/// - **协议安全**：保留 run 第一对 `tool_use`+`tool_result`（仅替换 result 文本，
///   `is_error` 保持 `true`），删除 run 其余成员的 use 与 result block，再做
///   drop-empty + 合并同角色——折叠后每个存活 `tool_use` 仍有配对 `tool_result`，
///   角色仍交替。
/// - 应在 `sanitize_history` **之后**调用（依赖其"每个 tool_use 恰有一个配对
///   tool_result"不变量）。
///
/// # "连续重复失败"的判定
/// 按工具调用在消息流中的出现顺序，一个 run 满足：每个都是 `is_error == true`、
/// 相同 `name`、相同入参（[`InputKey`] 比较）。中间穿插的 assistant 文本/思考
/// 不打断 run——只看工具调用的相邻性。
///
/// # 不折叠的情况
/// 单次失败、不同工具/不同参数的失败、成功调用，以及它们之间的组合均原样保留。
pub(crate) fn fold_repeated_tool_failures(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    if messages.len() < 2 {
        return messages;
    }

    // 1. 扫描收集每个 tool_use_id 的元信息、配对 result 是否失败及其内容，
    //    以及 tool_use 的出现顺序（用于判定"连续"）。依赖 sanitize 不变量。
    let mut use_meta: HashMap<String, CallMeta> = HashMap::new();
    let mut result_is_error: HashMap<String, bool> = HashMap::new();
    let mut result_content: HashMap<String, String> = HashMap::new();
    let mut ordered_ids: Vec<String> = Vec::new();

    for msg in &messages {
        for block in &msg.content {
            match block {
                ContentBlock::ToolUse { id, name, input } => {
                    if !use_meta.contains_key(id) {
                        use_meta.insert(
                            id.clone(),
                            CallMeta {
                                name: name.clone(),
                                key: InputKey::new(input),
                                raw_input: input.clone(),
                            },
                        );
                        ordered_ids.push(id.clone());
                    }
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    result_is_error
                        .entry(tool_use_id.clone())
                        .or_insert(is_error.unwrap_or(false));
                    result_content
                        .entry(tool_use_id.clone())
                        .or_insert_with(|| content.clone());
                }
                _ => {}
            }
        }
    }

    // 2. 在有序 id 上划分"连续相同签名的失败" run，标记保留首对 / 删除其余。
    let mut drop_ids: HashSet<String> = HashSet::new(); // run 中待删成员的 tool_use_id
    let mut replace_result: HashMap<String, String> = HashMap::new(); // 保留首对的 id → 摘要文本

    let mut i = 0;
    while i < ordered_ids.len() {
        let cur = match use_meta.get(&ordered_ids[i]) {
            Some(m) => m,
            None => {
                i += 1;
                continue;
            }
        };
        let cur_err = result_is_error
            .get(&ordered_ids[i])
            .copied()
            .unwrap_or(false);
        if !cur_err {
            i += 1;
            continue;
        }
        // 向后扩展同签名同失败的连续 run → 区间 [i, j)
        let mut j = i + 1;
        while j < ordered_ids.len() {
            let is_err = result_is_error
                .get(&ordered_ids[j])
                .copied()
                .unwrap_or(false);
            let m = match use_meta.get(&ordered_ids[j]) {
                Some(m) => m,
                None => break,
            };
            if is_err && m.name == cur.name && m.key == cur.key {
                j += 1;
            } else {
                break;
            }
        }
        let run_len = j - i;
        if run_len >= TOOL_FAILURE_FOLD_THRESHOLD {
            // run 最后一个成员的错误即"最近一次错误"
            let last_id = &ordered_ids[j - 1];
            let last_err = result_content.get(last_id).cloned().unwrap_or_default();
            let summary = make_fold_summary(run_len, &cur.name, &cur.raw_input, &last_err);
            replace_result.insert(ordered_ids[i].clone(), summary);
            for id in &ordered_ids[(i + 1)..j] {
                drop_ids.insert(id.clone());
            }
        }
        i = j;
    }

    if drop_ids.is_empty() && replace_result.is_empty() {
        return messages; // 无可折叠，原样返回（省一次重建）
    }

    // 3. 按 id 重写消息：删 drop 成员的 use/result，替换保留首对的 result 文本。
    let mut rewritten: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    for msg in messages {
        let role = msg.role;
        let mut new_blocks: Vec<ContentBlock> = Vec::with_capacity(msg.content.len());
        for block in msg.content {
            match &block {
                ContentBlock::ToolUse { id, .. } => {
                    if !drop_ids.contains(id) {
                        new_blocks.push(block);
                    }
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    is_error,
                    ..
                } => {
                    if let Some(new_text) = replace_result.get(tool_use_id) {
                        // 保留首对：替换 content，保持原 is_error（仍为 true）
                        new_blocks.push(ContentBlock::ToolResult {
                            tool_use_id: tool_use_id.clone(),
                            content: new_text.clone(),
                            is_error: *is_error,
                        });
                    } else if !drop_ids.contains(tool_use_id) {
                        new_blocks.push(block);
                    }
                    // 其余：属于被删成员的 result → 丢弃
                }
                _ => new_blocks.push(block),
            }
        }
        if !new_blocks.is_empty() {
            rewritten.push(ChatMessage {
                role,
                content: new_blocks,
                ..Default::default()
            });
        }
    }

    // 4. 防御性清理：drop 空消息（步骤 3 已无，双保险）+ 合并连续同角色。
    drop_empty_and_merge(rewritten)
}

/// 生成折叠后的摘要 result 文本。
fn make_fold_summary(n: usize, name: &str, input: &str, last_err: &str) -> String {
    format!(
        "[已折叠] 此前连续 {n} 次调用工具 `{name}` 均失败（参数：{args}）。最近一次错误：{err}",
        n = n,
        name = name,
        args = truncate_chars(input, FOLD_SUMMARY_FIELD_MAX),
        err = truncate_chars(last_err, FOLD_SUMMARY_FIELD_MAX),
    )
}

/// 按 Unicode char 数截断，超出加省略号。
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max).collect();
    t.push('…');
    t
}

/// 折叠后防御性清理：丢弃空消息 + 合并连续同角色消息（协议要求 user/assistant 交替）。
fn drop_empty_and_merge(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut merged: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    for msg in messages {
        if msg.content.is_empty() {
            continue;
        }
        if let Some(last) = merged.last_mut() {
            if last.role == msg.role {
                last.content.extend(msg.content);
                continue;
            }
        }
        merged.push(msg);
    }
    merged
}

/// 解析 `content_blocks` JSON 字符串为 `Vec<ContentBlock>`。
///
/// - 空字符串 / `"[]"` / 无效 JSON → 返回空 `Vec`（调用方会回退到纯文本）
/// - 仅含 `Text` 块 → 返回该 `Vec`（含若干 `Text` 块）
/// - 含 `Image` 等多模态块 → 返回完整还原的 blocks
///
/// **注意**：assistant 消息的 `ToolUse` / `ToolResult` 块也需要通过此路径还原，
/// 否则多轮工具调用对话的历史上下文会丢失工具调用记录。
///
/// pub(crate)：session-events 对账器（harness/reconcile.rs）用同一解析器
/// 提取 legacy 行原始形态，保证两侧 blocks 解析语义一致。
pub(crate) fn parse_content_blocks(json: &str) -> Vec<ContentBlock> {
    if json.is_empty() || json == "[]" {
        return Vec::new();
    }
    serde_json::from_str::<Vec<ContentBlock>>(json).unwrap_or_default()
}

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row(idx: usize, role: &str, content: &str) -> MessageRow {
        MessageRow {
            id: format!("msg-{idx}"),
            conversation_id: "conv".into(),
            role: role.into(),
            content: content.into(),
            content_blocks: "[]".into(),
            token_count: None,
            error: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            rowid: idx as i64,
            summary_id: None,
            model: None,
        }
    }

    #[test]
    fn resolve_window_uses_default_when_agent_none() {
        assert_eq!(resolve_window(None), DEFAULT_HISTORY_WINDOW);
    }

    #[test]
    fn resolve_window_uses_agent_value_when_positive() {
        assert_eq!(resolve_window(Some(60)), 60);
        assert_eq!(resolve_window(Some(1)), 1);
    }

    #[test]
    fn resolve_window_falls_back_to_default_on_non_positive() {
        // 0 / 负数视为未配置 → 回退默认
        assert_eq!(resolve_window(Some(0)), DEFAULT_HISTORY_WINDOW);
        assert_eq!(resolve_window(Some(-5)), DEFAULT_HISTORY_WINDOW);
    }

    #[test]
    fn sanitize_tool_name_strips_non_compliant() {
        // 旧版「中文server名.工具名」→ 剥离中文与点号
        assert_eq!(
            sanitize_tool_name("浏览器自动化.browser_click"),
            "browser_click"
        );
        assert_eq!(
            sanitize_tool_name("深度推理.sequentialthinking"),
            "sequentialthinking"
        );
    }

    #[test]
    fn sanitize_tool_name_keeps_compliant_unchanged() {
        // 新版合规名（含 t{idx}_ 前缀）原样返回
        assert_eq!(sanitize_tool_name("t0_browser_click"), "t0_browser_click");
        assert_eq!(sanitize_tool_name("read_file"), "read_file");
        assert_eq!(sanitize_tool_name("tool-with-dash"), "tool-with-dash");
    }

    #[test]
    fn sanitize_tool_name_falls_back_when_empty() {
        // 纯中文 / 全违规字符 → 兜底 "tool"
        assert_eq!(sanitize_tool_name("浏览器自动化"), "tool");
        assert_eq!(sanitize_tool_name("。。"), "tool");
        assert_eq!(sanitize_tool_name(""), "tool");
    }

    #[test]
    fn load_history_skips_tool_role() {
        let rows = vec![
            make_row(1, "user", "hello"),
            make_row(2, "assistant", "hi"),
            make_row(3, "tool", "result"),
            make_row(4, "system", "sys"),
        ];
        let msgs = load_history(&rows);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[2].role, "system");
    }

    #[test]
    fn load_history_empty() {
        let msgs = load_history(&[]);
        assert!(msgs.is_empty());
    }

    #[test]
    fn load_history_with_window_keeps_last_n() {
        let rows: Vec<MessageRow> = (0..10)
            .map(|i| {
                make_row(
                    i,
                    if i % 2 == 0 { "user" } else { "assistant" },
                    &format!("msg-{i}"),
                )
            })
            .collect();
        let msgs = load_history_with_window(&rows, Some(3));
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].content_text(), "msg-7");
        assert_eq!(msgs[1].content_text(), "msg-8");
        assert_eq!(msgs[2].content_text(), "msg-9");
    }

    #[test]
    fn load_history_with_window_none_keeps_all() {
        // window=None 走向后兼容路径，不过滤
        let rows: Vec<MessageRow> = (0..5)
            .map(|i| {
                make_row(
                    i,
                    if i % 2 == 0 { "user" } else { "assistant" },
                    &format!("msg-{i}"),
                )
            })
            .collect();
        let msgs = load_history_with_window(&rows, None);
        assert_eq!(msgs.len(), 5);
    }

    #[test]
    fn load_history_with_window_larger_than_input_keeps_all() {
        // window >= input 长度 → 全部保留
        let rows: Vec<MessageRow> = (0..3)
            .map(|i| {
                make_row(
                    i,
                    if i % 2 == 0 { "user" } else { "assistant" },
                    &format!("msg-{i}"),
                )
            })
            .collect();
        let msgs = load_history_with_window(&rows, Some(100));
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn load_history_with_window_still_skips_tool_role() {
        // 窗口裁剪 + role 过滤 是两个独立动作，都要生效
        let rows = vec![
            make_row(1, "user", "u1"),
            make_row(2, "assistant", "a1"),
            make_row(3, "tool", "skip"),
            make_row(4, "user", "u2"),
            make_row(5, "assistant", "a2"),
        ];
        let msgs = load_history_with_window(&rows, Some(3));
        // 最后 3 条: tool(user? no—tool), user(u2), assistant(a2)
        // tool 被过滤后剩 user(u2) + assistant(a2)
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content_text(), "u2");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content_text(), "a2");
    }

    #[test]
    fn load_history_filters_summary_rows() {
        // 双注入修复（Phase 2）：摘要行（role=system + SUMMARY_PREFIX 前缀）必须被
        // 过滤出 history_messages——否则 FinalAssembleStage 又经 ctx.summary 再注入一次，
        // LLM 会收到两份摘要。注意普通 system 行（无前缀）不受影响。
        let summary_content = format!("{SUMMARY_PREFIX}\n这是滚动摘要正文");
        let rows = vec![
            make_row(1, "user", "u1"),
            make_row(2, "system", &summary_content), // 摘要行 → 过滤
            make_row(3, "assistant", "a1"),
            make_row(4, "system", "普通系统提示"), // 普通 system → 不过滤（保留）
            make_row(5, "user", "u2"),
        ];
        let msgs = load_history_with_window(&rows, None);
        // 摘要行被剔除；其余 4 条保留
        assert_eq!(msgs.len(), 4, "摘要行应被过滤: {:?}", msgs);
        assert!(
            !msgs
                .iter()
                .any(|m| m.content_text().contains(SUMMARY_PREFIX)),
            "history 不应含摘要行"
        );
        assert!(
            msgs.iter().any(|m| m.content_text() == "普通系统提示"),
            "普通 system 行应保留"
        );
    }

    // ===== sanitize_history：协议净化（去重 / 孤儿 / 合并）=====

    fn text_blk(t: &str) -> ContentBlock {
        ContentBlock::Text { text: t.into() }
    }
    fn tu(id: &str) -> ContentBlock {
        ContentBlock::ToolUse {
            id: id.into(),
            name: "x".into(),
            input: "{}".into(),
        }
    }
    fn tr(id: &str) -> ContentBlock {
        ContentBlock::ToolResult {
            tool_use_id: id.into(),
            content: "r".into(),
            is_error: Some(false),
        }
    }
    fn cm(role: &str, blocks: Vec<ContentBlock>) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: blocks,
            ..Default::default()
        }
    }

    #[test]
    fn sanitize_dedupes_duplicate_tool_use() {
        // 同一消息内 2 个相同 id 的 tool_use → 仅留 1 个
        let out = sanitize_history(vec![
            cm("user", vec![text_blk("q")]),
            cm("assistant", vec![tu("A"), tu("A"), text_blk("...")]),
            cm("user", vec![tr("A")]),
        ]);
        let n = out[1]
            .content
            .iter()
            .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
            .count();
        assert_eq!(n, 1, "重复 tool_use id 应去重到 1");
    }

    #[test]
    fn sanitize_drops_orphan_tool_result() {
        // tool_result 引用的 id 窗口内无 tool_use → 丢弃（裁剪边界场景）
        let out = sanitize_history(vec![
            cm("user", vec![text_blk("q")]),
            cm("assistant", vec![text_blk("a")]),
            cm("user", vec![tr("orphan"), text_blk("more")]),
        ]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2].content.len(), 1, "孤儿 tool_result 丢弃后只剩 text");
        assert!(matches!(out[2].content[0], ContentBlock::Text { .. }));
    }

    #[test]
    fn sanitize_merges_consecutive_same_role() {
        let out = sanitize_history(vec![
            cm("assistant", vec![text_blk("a")]),
            cm("user", vec![text_blk("u1")]),
            cm("user", vec![text_blk("u2")]),
        ]);
        let users: Vec<_> = out.iter().filter(|m| m.role == "user").collect();
        assert_eq!(users.len(), 1, "连续 user 合并为 1 条");
        assert_eq!(users[0].content.len(), 2, "合并后含 2 个 block");
    }

    #[test]
    fn sanitize_keeps_valid_pair_intact() {
        // 正常 tool_use + tool_result 配对 → 不应被误删
        let out = sanitize_history(vec![
            cm("user", vec![text_blk("q")]),
            cm("assistant", vec![tu("A"), text_blk("...")]),
            cm("user", vec![tr("A")]),
            cm("assistant", vec![text_blk("done")]),
        ]);
        let has_use = out.iter().any(|m| {
            m.content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { id, .. } if id == "A"))
        });
        let has_result = out.iter().any(|m| {
            m.content.iter().any(
                |b| matches!(b, ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "A"),
            )
        });
        assert!(
            has_use && has_result,
            "正常配对的 tool_use/tool_result 应保留"
        );
    }

    #[test]
    fn sanitize_drops_orphan_tool_use() {
        // tool_use 在窗口内但无配对 tool_result（工具超时/出错未补结果）→ 丢弃该 tool_use。
        // 若 assistant 仅含这个孤儿 tool_use，整条消息丢弃，避免发给 MiniMax 触发 2013。
        let out = sanitize_history(vec![
            cm("user", vec![text_blk("q")]),
            cm("assistant", vec![tu("orphan")]),
            cm("user", vec![text_blk("next")]),
            cm("assistant", vec![text_blk("ok")]),
        ]);
        let has_orphan = out.iter().any(|m| {
            m.content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { id, .. } if id == "orphan"))
        });
        assert!(!has_orphan, "孤儿 tool_use（无配对 result）应被丢弃");
        // 其余正常消息保留
        assert!(out.iter().any(|m| m.content_text() == "ok"));
    }

    #[test]
    fn sanitize_drops_empty_assistant_placeholder() {
        // 空 assistant 占位（错误遗留，content 为空白）→ 丢弃；不破坏其余消息的交替。
        let out = sanitize_history(vec![
            cm("user", vec![text_blk("q")]),
            cm("assistant", vec![text_blk("")]), // 空占位
            cm("user", vec![text_blk("again")]),
            cm("assistant", vec![text_blk("resp")]),
        ]);
        // 空占位被丢弃 + 两个连续 user 合并 → [user(merged), assistant(resp)]，共 2 条
        assert_eq!(out.len(), 2, "空 assistant 丢弃 + 两 user 合并后应为 2 条");
        assert_eq!(out[0].role, "user");
        // 合并后的 user 应含两段文本（q + again）
        assert_eq!(out[0].content.len(), 2);
        assert_eq!(out[1].role, "assistant");
        assert_eq!(out[1].content_text(), "resp");
    }

    #[test]
    fn sanitize_orphan_use_with_text_keeps_text() {
        // assistant 同时含文本和孤儿 tool_use → 丢弃 tool_use，保留文本（不误删整条）
        let out = sanitize_history(vec![
            cm("user", vec![text_blk("q")]),
            cm("assistant", vec![text_blk("partial"), tu("orphan")]),
            cm("user", vec![text_blk("next")]),
        ]);
        let asst = out
            .iter()
            .find(|m| m.role == "assistant")
            .expect("assistant 应保留");
        assert_eq!(asst.content.len(), 1, "仅剩文本块");
        assert!(matches!(asst.content[0], ContentBlock::Text { .. }));
    }

    #[test]
    fn sanitize_drops_thinking_only_assistant() {
        // 孤儿 tool_use 被剔除后，assistant 仅剩 thinking → 无法序列化为合法 OpenAI
        // assistant（content=null 且无 tool_calls，deepseek 等严格端点 400）。必须整体丢弃。
        // 真实场景：reasoning 模型产出 [thinking + tool_use]，但 tool_use 的 result 丢失
        // （工具中断/出错未补结果），下一条是纯文本 user → tool_use 成孤儿被剔除。
        let out = sanitize_history(vec![
            cm("user", vec![text_blk("q")]),
            cm(
                "assistant",
                vec![
                    ContentBlock::Thinking {
                        thinking: "let me try...".into(),
                        signature: None,
                    },
                    tu("orphan"),
                ],
            ),
            cm("user", vec![text_blk("情况如何了？")]),
            cm("assistant", vec![text_blk("ok")]),
        ]);
        // thinking-only assistant 被丢弃；两个 user 合并 → [user(merged), assistant(ok)]
        assert_eq!(out.len(), 2, "thinking-only assistant 应被丢弃");
        assert_eq!(out[0].role, "user");
        assert_eq!(out[0].content.len(), 2, "两个 user 文本块应合并");
        assert_eq!(out[1].role, "assistant");
        assert_eq!(out[1].content_text(), "ok");
        // 不应残留任何 thinking-only assistant
        assert!(
            !out.iter().any(|m| m.role == "assistant"
                && !m.content.is_empty()
                && m.content
                    .iter()
                    .all(|b| matches!(b, ContentBlock::Thinking { .. }))),
            "不应残留 thinking-only assistant"
        );
    }

    #[test]
    fn sanitize_keeps_assistant_with_thinking_and_text() {
        // assistant 同时含 thinking 和 text → 有合法 content，不应被丢弃（thinking 保留，
        // 由序列化层决定是否发送；此处只确保不被 sanitize 误删）。
        let out = sanitize_history(vec![
            cm("user", vec![text_blk("q")]),
            cm(
                "assistant",
                vec![
                    ContentBlock::Thinking {
                        thinking: "h".into(),
                        signature: None,
                    },
                    text_blk("answer"),
                ],
            ),
            cm("user", vec![text_blk("thx")]),
        ]);
        let a = out
            .iter()
            .find(|m| m.role == "assistant")
            .expect("assistant 应保留");
        assert_eq!(a.content.len(), 2, "thinking + text 都应保留");
        assert!(a
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { .. })));
    }

    #[test]
    fn sanitize_keeps_assistant_with_thinking_and_tool_use() {
        // assistant 含 thinking + 有效 tool_use（有配对 result）→ 有合法 tool_calls，保留。
        let out = sanitize_history(vec![
            cm("user", vec![text_blk("q")]),
            cm(
                "assistant",
                vec![
                    ContentBlock::Thinking {
                        thinking: "h".into(),
                        signature: None,
                    },
                    tu("A"),
                ],
            ),
            cm("user", vec![tr("A")]),
            cm("assistant", vec![text_blk("done")]),
        ]);
        let a = out
            .iter()
            .find(|m| {
                m.role == "assistant"
                    && m.content
                        .iter()
                        .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
            })
            .expect("含 tool_use 的 assistant 应保留");
        assert!(
            a.content
                .iter()
                .any(|b| matches!(b, ContentBlock::Thinking { .. })),
            "thinking 应保留"
        );
    }

    // ===== P2-2 G1：历史消息图片重注入 =====

    fn make_row_with_blocks(
        idx: usize,
        role: &str,
        content: &str,
        content_blocks: &str,
    ) -> MessageRow {
        MessageRow {
            id: format!("msg-{idx}"),
            conversation_id: "conv".into(),
            role: role.into(),
            content: content.into(),
            content_blocks: content_blocks.into(),
            token_count: None,
            error: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            rowid: idx as i64,
            summary_id: None,
            model: None,
        }
    }

    #[test]
    fn load_history_restores_image_from_content_blocks() {
        // 含图片的多模态消息: content_blocks 是权威源，应优先使用
        let blocks_json = r#"[{"type":"text","text":"看图"},{"type":"image","data":"AAAA","media_type":"image/png"}]"#;
        let row = make_row_with_blocks(1, "user", "看图", blocks_json);
        let msgs = load_history(&[row]);
        assert_eq!(msgs.len(), 1);
        // content_blocks 还原出 2 个块：Text + Image
        assert_eq!(msgs[0].content.len(), 2);
        assert!(!msgs[0].content[0].is_image(), "第一个块应是 Text");
        assert!(msgs[0].content[1].is_image(), "第二个块应是 Image");
    }

    #[test]
    fn load_history_fallback_to_text_when_blocks_empty() {
        // 纯文本消息（content_blocks = "[]"）走原有路径，行为不变
        let row = make_row_with_blocks(1, "user", "hello", "[]");
        let msgs = load_history(&[row]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content_text(), "hello");
        // 纯文本回退后仅一个 Text 块
        assert_eq!(msgs[0].content.len(), 1);
        assert!(!msgs[0].content[0].is_image());
    }

    #[test]
    fn load_history_invalid_blocks_json_falls_back_gracefully() {
        // 无效 JSON 不应崩溃，应静默回退到纯文本
        let row = make_row_with_blocks(1, "user", "hello", "INVALID JSON {");
        let msgs = load_history(&[row]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content_text(), "hello");
        // 回退后仅一个 Text 块
        assert_eq!(msgs[0].content.len(), 1);
        assert!(!msgs[0].content[0].is_image());
    }

    // ===== fold_repeated_tool_failures：失败工具调用折叠 =====

    fn tu_full(id: &str, name: &str, input: &str) -> ContentBlock {
        ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input: input.into(),
        }
    }
    fn tr_err(id: &str, content: &str) -> ContentBlock {
        ContentBlock::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error: Some(true),
        }
    }
    fn tr_ok(id: &str, content: &str) -> ContentBlock {
        ContentBlock::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error: Some(false),
        }
    }

    fn count_tool_uses(msgs: &[ChatMessage]) -> usize {
        msgs.iter()
            .flat_map(|m| m.content.iter())
            .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
            .count()
    }
    fn count_tool_results(msgs: &[ChatMessage]) -> usize {
        msgs.iter()
            .flat_map(|m| m.content.iter())
            .filter(|b| matches!(b, ContentBlock::ToolResult { .. }))
            .count()
    }
    fn result_text_for<'a>(msgs: &'a [ChatMessage], tool_use_id: &str) -> Option<&'a str> {
        for m in msgs {
            for b in &m.content {
                if let ContentBlock::ToolResult {
                    tool_use_id: id,
                    content,
                    ..
                } = b
                {
                    if id == tool_use_id {
                        return Some(content);
                    }
                }
            }
        }
        None
    }
    fn result_is_error_for(msgs: &[ChatMessage], tool_use_id: &str) -> Option<bool> {
        for m in msgs {
            for b in &m.content {
                if let ContentBlock::ToolResult {
                    tool_use_id: id,
                    is_error,
                    ..
                } = b
                {
                    if id == tool_use_id {
                        return *is_error;
                    }
                }
            }
        }
        None
    }

    #[test]
    fn fold_no_tool_calls_unchanged() {
        let msgs = vec![
            cm("user", vec![text_blk("hi")]),
            cm("assistant", vec![text_blk("yo")]),
        ];
        let out = fold_repeated_tool_failures(msgs);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn fold_single_failure_not_folded() {
        // 单次失败：run=1 < 阈值 2，原样保留
        let msgs = vec![
            cm("user", vec![text_blk("do x")]),
            cm("assistant", vec![tu_full("A", "run_command", "{}")]),
            cm("user", vec![tr_err("A", "boom")]),
            cm("assistant", vec![text_blk("sorry")]),
        ];
        let out = fold_repeated_tool_failures(msgs);
        assert_eq!(count_tool_uses(&out), 1);
        assert_eq!(count_tool_results(&out), 1);
        assert_eq!(result_text_for(&out, "A"), Some("boom")); // 未替换
    }

    #[test]
    fn fold_two_identical_failures_collapsed() {
        let msgs = vec![
            cm("user", vec![text_blk("do x")]),
            cm("assistant", vec![tu_full("A1", "run_command", "{}")]),
            cm("user", vec![tr_err("A1", "timeout")]),
            cm("assistant", vec![tu_full("A2", "run_command", "{}")]),
            cm("user", vec![tr_err("A2", "timeout")]),
            cm("assistant", vec![text_blk("done")]),
        ];
        let out = fold_repeated_tool_failures(msgs);
        assert_eq!(count_tool_uses(&out), 1, "两连失败应折叠为 1 个 tool_use");
        assert_eq!(count_tool_results(&out), 1);
        // 保留首个 A1，其 result 被替换为摘要
        let txt = result_text_for(&out, "A1").expect("A1 应保留");
        assert!(txt.contains("已折叠"), "应含折叠标记: {txt}");
        assert!(txt.contains("2 次"), "应说明折叠次数: {txt}");
        assert!(txt.contains("timeout"), "应含最近一次错误: {txt}");
        // is_error 仍为 true
        assert_eq!(result_is_error_for(&out, "A1"), Some(true));
    }

    #[test]
    fn fold_three_identical_failures_count() {
        let msgs = vec![
            cm("user", vec![text_blk("do x")]),
            cm("assistant", vec![tu_full("A1", "run_command", "{\"a\":1}")]),
            cm("user", vec![tr_err("A1", "e")]),
            cm("assistant", vec![tu_full("A2", "run_command", "{\"a\":1}")]),
            cm("user", vec![tr_err("A2", "e")]),
            cm("assistant", vec![tu_full("A3", "run_command", "{\"a\":1}")]),
            cm("user", vec![tr_err("A3", "e")]),
            cm("assistant", vec![text_blk("ok")]),
        ];
        let out = fold_repeated_tool_failures(msgs);
        assert_eq!(count_tool_uses(&out), 1);
        let txt = result_text_for(&out, "A1").unwrap();
        assert!(txt.contains("3 次"), "应说明 3 次: {txt}");
    }

    #[test]
    fn fold_different_inputs_not_folded() {
        // 同工具不同参数的失败 → 不同签名，不折叠
        let msgs = vec![
            cm("user", vec![text_blk("do")]),
            cm("assistant", vec![tu_full("A", "run_command", "{\"a\":1}")]),
            cm("user", vec![tr_err("A", "e1")]),
            cm("assistant", vec![tu_full("B", "run_command", "{\"a\":2}")]),
            cm("user", vec![tr_err("B", "e2")]),
        ];
        let out = fold_repeated_tool_failures(msgs);
        assert_eq!(count_tool_uses(&out), 2, "不同参数不应折叠");
    }

    #[test]
    fn fold_json_key_order_insensitive() {
        // 同值不同 key 顺序 → 视为相同签名
        let msgs = vec![
            cm("user", vec![text_blk("do")]),
            cm(
                "assistant",
                vec![tu_full("A", "run_command", "{\"a\":1,\"b\":2}")],
            ),
            cm("user", vec![tr_err("A", "e")]),
            cm(
                "assistant",
                vec![tu_full("B", "run_command", "{\"b\":2,\"a\":1}")],
            ),
            cm("user", vec![tr_err("B", "e")]),
        ];
        let out = fold_repeated_tool_failures(msgs);
        assert_eq!(count_tool_uses(&out), 1, "key 顺序不同但值相同应折叠");
    }

    #[test]
    fn fold_success_breaks_run() {
        // 失败、成功交替：成功调用打断失败 run，且成功调用不折叠
        let msgs = vec![
            cm("user", vec![text_blk("do")]),
            cm("assistant", vec![tu_full("A1", "run_command", "{}")]),
            cm("user", vec![tr_err("A1", "e")]),
            cm("assistant", vec![tu_full("A2", "run_command", "{}")]),
            cm("user", vec![tr_ok("A2", "ok")]),
        ];
        let out = fold_repeated_tool_failures(msgs);
        assert_eq!(count_tool_uses(&out), 2, "成功调用打断 run，两调用均保留");
    }

    #[test]
    fn fold_failures_then_success_collapses_only_failures() {
        // 连续 2 失败后成功：折叠前 2 失败为 1，成功调用保留
        let msgs = vec![
            cm("user", vec![text_blk("do")]),
            cm("assistant", vec![tu_full("A1", "run_command", "{}")]),
            cm("user", vec![tr_err("A1", "e1")]),
            cm("assistant", vec![tu_full("A2", "run_command", "{}")]),
            cm("user", vec![tr_err("A2", "e2")]),
            cm("assistant", vec![tu_full("A3", "run_command", "{}")]),
            cm("user", vec![tr_ok("A3", "finally")]),
            cm("assistant", vec![text_blk("done")]),
        ];
        let out = fold_repeated_tool_failures(msgs);
        // 折叠后：1 个失败摘要 + 1 个成功 = 2 个 tool_use
        assert_eq!(count_tool_uses(&out), 2);
        let folded = result_text_for(&out, "A1").unwrap();
        assert!(folded.contains("已折叠") && folded.contains("2 次"));
        assert_eq!(result_text_for(&out, "A3"), Some("finally"));
    }

    #[test]
    fn fold_preserves_text_blocks() {
        let msgs = vec![
            cm("user", vec![text_blk("do")]),
            cm(
                "assistant",
                vec![tu_full("A1", "run_command", "{}"), text_blk("thinking...")],
            ),
            cm("user", vec![tr_err("A1", "e")]),
            cm(
                "assistant",
                vec![tu_full("A2", "run_command", "{}"), text_blk("retry...")],
            ),
            cm("user", vec![tr_err("A2", "e")]),
        ];
        let out = fold_repeated_tool_failures(msgs);
        // 文本 block 应保留（折叠只动 tool block）
        let joined: String = out
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|b| {
                if let ContentBlock::Text { text } = b {
                    Some(text.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(joined.contains("thinking..."));
        assert!(joined.contains("retry..."));
    }

    #[test]
    fn fold_differing_errors_shows_last() {
        // run 里各次错误不同 → 摘要显示"最近一次"（最后一个成员的错误）
        let msgs = vec![
            cm("user", vec![text_blk("do")]),
            cm("assistant", vec![tu_full("A1", "run_command", "{}")]),
            cm("user", vec![tr_err("A1", "first-error")]),
            cm("assistant", vec![tu_full("A2", "run_command", "{}")]),
            cm("user", vec![tr_err("A2", "second-error")]),
            cm("assistant", vec![tu_full("A3", "run_command", "{}")]),
            cm("user", vec![tr_err("A3", "third-error")]),
        ];
        let out = fold_repeated_tool_failures(msgs);
        let txt = result_text_for(&out, "A1").unwrap();
        assert!(txt.contains("third-error"), "应展示最近一次错误: {txt}");
        assert!(!txt.contains("first-error"), "不应含最早错误: {txt}");
    }

    #[test]
    fn fold_idempotent() {
        let msgs = vec![
            cm("user", vec![text_blk("do")]),
            cm("assistant", vec![tu_full("A1", "run_command", "{}")]),
            cm("user", vec![tr_err("A1", "e")]),
            cm("assistant", vec![tu_full("A2", "run_command", "{}")]),
            cm("user", vec![tr_err("A2", "e")]),
        ];
        let once = fold_repeated_tool_failures(msgs.clone());
        let twice = fold_repeated_tool_failures(once.clone());
        assert_eq!(once.len(), twice.len());
        assert_eq!(count_tool_uses(&once), count_tool_uses(&twice));
    }

    #[test]
    fn fold_protocol_alternation_preserved() {
        // 折叠后角色必须严格交替（Anthropic 契约），且 tool_use 与 tool_result 配对
        let msgs = vec![
            cm("user", vec![text_blk("do")]),
            cm("assistant", vec![tu_full("A1", "run_command", "{}")]),
            cm("user", vec![tr_err("A1", "e")]),
            cm("assistant", vec![tu_full("A2", "run_command", "{}")]),
            cm("user", vec![tr_err("A2", "e")]),
            cm("assistant", vec![tu_full("A3", "run_command", "{}")]),
            cm("user", vec![tr_err("A3", "e")]),
            cm("assistant", vec![text_blk("end")]),
        ];
        let out = fold_repeated_tool_failures(msgs);
        assert!(!out.is_empty());
        for w in out.windows(2) {
            assert_ne!(
                w[0].role, w[1].role,
                "折叠后出现连续同角色: {} {}",
                w[0].role, w[1].role
            );
        }
        // 存活的 tool_use 必须都有配对 tool_result（无孤儿）
        assert_eq!(count_tool_uses(&out), count_tool_results(&out));
    }
}
