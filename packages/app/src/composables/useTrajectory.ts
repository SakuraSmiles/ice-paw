// useTrajectory — 会话事件流 → 轨迹表格行模型（dsh inspection-ledger 风格）
//
// 数据层两条线：
// 1. buildRows(events, opts) 纯函数：seq 正序扁平事件 → 表格行（turn 分割头 +
//    单行事件 + 折叠摘要行）。核心取舍：**不嵌套聚合**（旧 groupTurns 的
//    rounds 结构退役）——dsh 的启示是「紧凑事件表保留因果结构，深度进检查器」，
//    扁平 seq 序天然就是因果序。
// 2. useTrajectory() composable：尾部优先分页拉取（大会话只载最新 PAGE 条，
//    「加载更早」用最小 seq 作游标向前翻），暴露响应式状态。
//
// 关键语义（已核实生产 emitter，见 harness/event_log.rs + loop_engine/tool_executor）：
// - assistant_message 同 (turn_id, message_id) 多条 = supersede（自动续写）→ 只保留最后一条
// - tool_execution 事件自含 arguments/result/duration_ms/is_error，单源无需跨查
// - tool_result_message 是 DB 结果行侧的镜像（tool_execution 已含结果）→ 不生成行
// - turn_context / turn_ended 折进 turn 分割头（配置与终止摘要常驻可见，全量进检查器）
// - attachment_stored / modal_adapted / hook_injected = 辅助事件（低频审计信息）→ 默认隐藏，开关显示
import { ref } from "vue";
import { bridge } from "../api/bridge";
import type {
  AssistantMessagePayload,
  ContentBlock,
  MessageDiscardedPayload,
  MessageErrorPayload,
  ModalAdaptedPayload,
  HookInjectedPayload,
  SessionEvent,
  SummaryPayload,
  ToolExecutionPayload,
  TurnContextPayload,
  TurnEndedPayload,
  UserMessagePayload,
} from "../types";

// =========================================================================
// 行模型
// =========================================================================

/** 行 kind：13 种日志 kind 的 UI 投影（徽章文案 + 颜色语义） */
export type RowKind = "user" | "assistant" | "tool" | "summary" | "error" | "discarded" | "aux";

export const ROW_KIND_LABELS: Record<RowKind, string> = {
  user: "USER",
  assistant: "ASSISTANT",
  tool: "TOOL",
  summary: "SUMMARY",
  error: "ERROR",
  discarded: "DISCARD",
  aux: "AUX",
};

/** turn 分割头（较粗分割线 + 摘要：轮次号 · 终止原因 · 耗时 · 用量；点击折叠/展开） */
export interface TurnHeaderRow {
  type: "turn-header";
  key: string;
  turnKey: string;
  /** null = 事件纪元前的孤儿事件桶 */
  turnId: string | null;
  turnIndex: number;
  seq: number;
  createdAt: string;
  collapsed: boolean;
  /** 搜索命中数（query 非空时用于头右侧显示） */
  matchCount: number;
  roundCount: number;
  toolCount: number;
  /** 本 turn message_error 计数（>0 时头右侧 ⚠ 徽章，扫读定位异常轮） */
  errorCount: number;
  /** 墙钟耗时（首→末事件，ms；时间不可解析为 null） */
  turnMs: number | null;
  /** 与上一 turn 跨天时显示的日期标签（本地 MM-DD） */
  dateLabel: string | null;
  ended: TurnEndedPayload | null;
  context: TurnContextPayload | null;
}

/** 单行事件（30px 固定高：kind 徽章 + 单行摘要 + 尾随 token/耗时） */
export interface EventRow {
  type: "event";
  key: string;
  turnKey: string;
  seq: number;
  createdAt: string;
  /** 时间轴用毫秒时间戳（解析失败为 null → 不上图） */
  t: number | null;
  kind: RowKind;
  label: string;
  summary: string;
  isError: boolean;
  /** assistant 无正文、摘要取自思考内容（行渲染为斜体弱化：内心活动非发言） */
  thinkingDerived: boolean;
  /** tool 行的执行耗时 / assistant 行的生成耗时；其余 null */
  durationMs: number | null;
  /** assistant 行的 token 计数；其余 null */
  tokens: number | null;
  /** 搜索命中（query 非空时：命中正常显示，未命中降透明度） */
  match: boolean;
  /** 原始事件（检查器直接读 payload，不再二次聚合） */
  event: SessionEvent;
  /** 生成中 ephemeral 行标记（流式临时观感行，不落库；落库行无此字段）。
   *  行渲染据此显示脉冲点；seq 恒 -1 不参与时间轴/选中联动 */
  streaming?: boolean;
}

/** 折叠态 = 只渲染 turn 头（头自带 统计/终止/用量，无需额外摘要行） */
export type TrajectoryRow = TurnHeaderRow | EventRow;

export interface BuildRowsOptions {
  /** 已折叠的 turn key 集合（搜索时忽略——强制展开以呈现命中） */
  collapsedTurns: Set<string>;
  /** 显示辅助事件（attachment_stored / modal_adapted / hook_injected） */
  showAux: boolean;
  /** 搜索词（大小写不敏感子串；命中 summary 或 payload 序列化文本） */
  query: string;
  /** 窗口前（seq 更早一侧）的全局轮次数（M3：尾部优先分页下首屏轮号不从 1 起） */
  turnOffset: number;
}

const NULL_TURN = "__orphan__";

/**
 * 搜索文本缓存（M2）：key = 事件对象引用。append-only 保证同一对象的 payload
 * 不可变；增量拼接/翻页时旧对象引用复用，切会话后旧数组整体替换 → WeakMap 自动
 * 回收。若无此缓存，每次 buildRows（折叠/开关/搜索词变化）都全量 JSON.stringify
 * 所有 payload——千轮会话（万级事件 × 含 thinking 全文的 payload）是可见卡顿源。
 * 非搜索态完全跳过构造（原实现即使 query="" 也 stringify）。
 */
const searchTextCache = new WeakMap<SessionEvent, string>();

/** 本地日期标签 MM-DD（解析失败返回 ""，不参与跨天比较） */
function localDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** 取首行文本（摘要用；去 Markdown 标记的最小努力：直接裁首个换行） */
function firstLine(s: string | null | undefined, max = 120): string {
  if (!s) return "";
  const line = s.split("\n", 1)[0].trim();
  return line.length > max ? `${line.slice(0, max)}…` : line;
}

function compactJson(s: string, max = 80): string {
  const t = s.replace(/\s+/g, " ").trim();
  return t.length > max ? `${t.slice(0, max)}…` : t;
}

function summarizeEvent(ev: SessionEvent): { kind: RowKind; summary: string; isError: boolean; durationMs: number | null; tokens: number | null; thinkingDerived: boolean } | null {
  switch (ev.kind) {
    case "user_message": {
      const p = ev.payload as UserMessagePayload;
      const imgs = p.blocks?.filter((b) => b.type === "image").length ?? 0;
      const text = firstLine(p.content);
      const summary = text || (imgs ? `[图片 ×${imgs}]` : "(空消息)");
      return { kind: "user", summary, isError: false, durationMs: null, tokens: null, thinkingDerived: false };
    }
    case "assistant_message": {
      const p = ev.payload as AssistantMessagePayload;
      const text = firstLine(p.content);
      const hasThinking = !!p.blocks?.some((b) => b.type === "thinking");
      const thinkBlock = p.blocks?.find((b): b is Extract<ContentBlock, { type: "thinking" }> => b.type === "thinking");
      const thinkText = thinkBlock ? firstLine(thinkBlock.thinking, 100) : "";
      const hasToolUse = !!p.blocks?.some((b) => b.type === "tool_use");
      // thinking 与正文可共存（extended thinking：[thinking..., text...]）。
      // 无正文的常见形态：思考完直接调工具 / 流式中途冻结。
      let summary: string;
      let thinkingDerived = false;
      if (text) {
        summary = hasThinking ? `💭 ${text}` : text;
      } else if (thinkText) {
        summary = `💭 ${thinkText}`; // 思考即摘要——它就是本行最有信息量的内容
        thinkingDerived = true;
      } else if (hasToolUse) {
        summary = "(仅工具调用)";
      } else {
        summary = "(无文本输出)";
      }
      if (p.continuation) summary = `↻ ${summary}`;
      return {
        kind: "assistant",
        summary,
        isError: false,
        durationMs: p.duration_ms ?? null,
        tokens: p.token_count ?? null,
        thinkingDerived,
      };
    }
    case "tool_execution": {
      const p = ev.payload as ToolExecutionPayload;
      const args = compactJson(p.arguments);
      return {
        kind: "tool",
        summary: args ? `${p.tool_name}  ${args}` : p.tool_name,
        isError: p.is_error,
        durationMs: p.duration_ms,
        tokens: null,
        thinkingDerived: false,
      };
    }
    case "summary_created":
    case "summary_updated": {
      const p = ev.payload as SummaryPayload;
      return { kind: "summary", summary: firstLine(p.content), isError: false, durationMs: null, tokens: null, thinkingDerived: false };
    }
    case "message_error": {
      const p = ev.payload as MessageErrorPayload;
      return { kind: "error", summary: `${p.kind}: ${firstLine(p.error, 160)}`, isError: true, durationMs: null, tokens: null, thinkingDerived: false };
    }
    case "message_discarded": {
      const p = ev.payload as MessageDiscardedPayload;
      return { kind: "discarded", summary: firstLine(p.reason, 160), isError: false, durationMs: null, tokens: null, thinkingDerived: false };
    }
    case "modal_adapted": {
      const p = ev.payload as ModalAdaptedPayload;
      return { kind: "aux", summary: `视觉适配 ${p.stage}/${p.mode} ×${p.items?.length ?? 0}`, isError: false, durationMs: null, tokens: null, thinkingDerived: false };
    }
    case "hook_injected": {
      const p = ev.payload as HookInjectedPayload;
      return { kind: "aux", summary: `钩子注入 ${p.point}`, isError: false, durationMs: null, tokens: null, thinkingDerived: false };
    }
    case "attachment_stored": {
      const items = (ev.payload as { items?: unknown[] }).items;
      const n = Array.isArray(items) ? items.length : 0;
      return { kind: "aux", summary: `附件落库 ×${n}`, isError: false, durationMs: null, tokens: null, thinkingDerived: false };
    }
    default:
      // turn_context / turn_ended 折进头；tool_result_message 是结果行镜像（工具行已含结果）
      return null;
  }
}

/**
 * seq 正序事件流 → 表格行。纯函数。
 *
 * 输入假定已按 seq 升序（list_session_events 的保证）。
 */
export function buildRows(events: SessionEvent[], opts: BuildRowsOptions): TrajectoryRow[] {
  const q = opts.query.trim().toLowerCase();

  // 预扫：supersede 索引（同 (turn,message) 的 assistant 取最后一条）+ turn 统计
  const lastIndexOf = new Map<string, number>();
  const turnStats = new Map<
    string,
    {
      roundIds: Set<string>;
      toolCount: number;
      errorCount: number;
      ended: TurnEndedPayload | null;
      context: TurnContextPayload | null;
      firstSeq: number;
      createdAt: string;
      firstAtMs: number;
      lastAtMs: number;
    }
  >();
  for (let i = 0; i < events.length; i++) {
    const ev = events[i];
    const tk = ev.turn_id ?? NULL_TURN;
    if (ev.kind === "assistant_message") {
      lastIndexOf.set(`${tk}|${ev.message_id ?? `__seq${ev.seq}`}`, i);
    }
    let st = turnStats.get(tk);
    if (!st) {
      st = { roundIds: new Set(), toolCount: 0, errorCount: 0, ended: null, context: null, firstSeq: ev.seq, createdAt: ev.created_at, firstAtMs: Number.NaN, lastAtMs: Number.NaN };
      turnStats.set(tk, st);
    }
    const tMs = Date.parse(ev.created_at);
    if (Number.isFinite(tMs)) {
      if (!Number.isFinite(st.firstAtMs)) st.firstAtMs = tMs;
      st.lastAtMs = tMs;
    }
    if (ev.kind === "assistant_message") st.roundIds.add(ev.message_id ?? `__seq${ev.seq}`);
    else if (ev.kind === "tool_execution") st.toolCount += 1;
    else if (ev.kind === "message_error") st.errorCount += 1;
    else if (ev.kind === "turn_ended") st.ended = ev.payload as TurnEndedPayload;
    else if (ev.kind === "turn_context") st.context = ev.payload as TurnContextPayload;
  }

  const rows: TrajectoryRow[] = [];
  let currentTurnKey: string | null = null;
  let currentHeader: TurnHeaderRow | null = null;
  // M3：窗口前还有更早分页时，窗口内首桶不是全局第 0 轮——从偏移起算
  let turnIndex = opts.turnOffset - 1;
  let prevDate = ""; // 跨天检测：仅日期变化时在头上标 MM-DD
  // 折叠时暂存本 turn 的事件行（搜索需要 matchCount，先攒后放）
  let pendingRows: EventRow[] = [];

  function flushTurn() {
    if (!currentHeader) return;
    const matchCount = pendingRows.reduce((n, r) => n + (r.match ? 1 : 0), 0);
    currentHeader.matchCount = matchCount;
    // 折叠（非搜索态）：只留头——头自带「N 条回复 · M 次工具」统计，不再补摘要行
    if (!(q === "" && currentHeader.collapsed)) rows.push(...pendingRows);
    pendingRows = [];
  }

  for (let i = 0; i < events.length; i++) {
    const ev = events[i];
    const tk = ev.turn_id ?? NULL_TURN;

    if (tk !== currentTurnKey) {
      flushTurn();
      const st = turnStats.get(tk)!;
      turnIndex += 1;
      currentTurnKey = tk;
      const d = localDate(st.createdAt);
      const dateLabel = d && d !== prevDate ? d : null;
      if (d) prevDate = d;
      currentHeader = {
        type: "turn-header",
        key: `th-${tk}`,
        turnKey: tk,
        turnId: ev.turn_id,
        turnIndex,
        seq: st.firstSeq,
        createdAt: st.createdAt,
        collapsed: opts.collapsedTurns.has(tk),
        matchCount: 0,
        roundCount: st.roundIds.size,
        toolCount: st.toolCount,
        errorCount: st.errorCount,
        turnMs: Number.isFinite(st.firstAtMs) && Number.isFinite(st.lastAtMs) ? Math.max(0, st.lastAtMs - st.firstAtMs) : null,
        dateLabel,
        ended: st.ended,
        context: st.context,
      };
      rows.push(currentHeader);
    }

    const s = summarizeEvent(ev);
    if (!s) continue; // 折进头/镜像 kind 不生成行
    if (s.kind === "aux" && !opts.showAux) continue;
    if (ev.kind === "assistant_message" && lastIndexOf.get(`${tk}|${ev.message_id ?? `__seq${ev.seq}`}`) !== i) {
      continue; // supersede：已被续写覆盖
    }

    // M2：仅搜索态构造 searchText（WeakMap 缓存，见 searchTextCache 注释）
    let match = true;
    if (q !== "") {
      let st = searchTextCache.get(ev);
      if (st === undefined) {
        st = `${s.summary}\n${JSON.stringify(ev.payload)}`.toLowerCase();
        searchTextCache.set(ev, st);
      }
      match = st.includes(q);
    }
    pendingRows.push({
      type: "event",
      key: `${tk}-${ev.kind}-${ev.seq}`,
      turnKey: tk,
      seq: ev.seq,
      createdAt: ev.created_at,
      t: Date.parse(ev.created_at) || null,
      kind: s.kind,
      label: ROW_KIND_LABELS[s.kind],
      summary: s.summary,
      isError: s.isError,
      thinkingDerived: s.thinkingDerived,
      durationMs: s.durationMs,
      tokens: s.tokens,
      match,
      event: ev,
    });
  }
  flushTurn();

  return rows;
}

// =========================================================================
// composable：尾部优先分页拉取
// =========================================================================

/** 每页事件数（大会话数据层：先载最新一页，「加载更早」向前翻） */
export const TRAJECTORY_PAGE_SIZE = 1000;

/**
 * 轨迹回放视图 composable：尾部优先分页拉事件，暴露响应式状态。
 * 行模型由视图层 computed(buildRows) 派生（折叠/搜索/辅助开关是纯视图状态）。
 */
export function useTrajectory() {
  const events = ref<SessionEvent[]>([]);
  const loading = ref(false);
  const loadingEarlier = ref(false);
  const error = ref<string | null>(null);
  /** 事件纪元前的旧会话（零事件，Phase 2A legacy 路由）→ UI 空态提示，非 bug */
  const legacy = ref(false);
  const hasMore = ref(false);
  /** 窗口前（更早分页一侧）的全局轮次数（M3：轮号全局偏移；0 = 窗口从头开始） */
  const turnOffset = ref(0);

  let currentId: string | null = null;
  let minSeq: number | null = null;

  /** 窗口还有更早内容时查一次全局轮偏移（含孤儿桶一组；与前端连续段切桶在
   *  罕见的交错孤儿场景可差 1，可接受——见 repo::count_turns_before 注释）。 */
  async function refreshTurnOffset() {
    const id = currentId;
    if (!id || minSeq == null) return;
    try {
      const n = await bridge.trajectory.turnOffset(id, minSeq);
      if (currentId === id) turnOffset.value = n;
    } catch {
      /* 偏移查询失败降级为窗口相对编号（0），不阻断主流程 */
    }
  }

  async function load(conversationId: string) {
    currentId = conversationId;
    loading.value = true;
    error.value = null;
    try {
      const page = await bridge.trajectory.listEvents(conversationId, TRAJECTORY_PAGE_SIZE);
      if (currentId !== conversationId) return; // 切换会话竞态守卫
      legacy.value = page.length === 0;
      events.value = page;
      hasMore.value = page.length === TRAJECTORY_PAGE_SIZE;
      minSeq = page.length ? page[0].seq : null;
      turnOffset.value = 0;
      if (hasMore.value) void refreshTurnOffset();
    } catch (e) {
      if (currentId !== conversationId) return;
      error.value = e instanceof Error ? e.message : String(e);
      events.value = [];
      legacy.value = false;
      hasMore.value = false;
    } finally {
      if (currentId === conversationId) loading.value = false;
    }
  }

  /**
   * live 追加：拉取 seq > 已载最大 seq 的增量并原地拼接。
   * 返回新事件数（0 = 已追平）。调用方按需滚底（跟随纪律：仅在底部时跟随）。
   * append-only 保证新事件恒在尾部——supersede 的 assistant_message 会以新 seq
   * 重复出现，由 buildRows 的 lastIndexOf 归并，无需在此去重。
   */
  async function refreshLatest(): Promise<number> {
    const id = currentId;
    if (!id || loading.value) return 0;
    const maxSeq = events.value.length ? events.value[events.value.length - 1].seq : 0;
    try {
      const inc = await bridge.trajectory.listEvents(id, TRAJECTORY_PAGE_SIZE, undefined, maxSeq);
      if (currentId !== id) return 0; // 切换会话竞态守卫
      if (inc.length) {
        events.value = [...events.value, ...inc];
        legacy.value = false;
      }
      return inc.length;
    } catch {
      return 0; // 轮询失败静默：下一轮再试（与影子日志同款宽容）
    }
  }

  /** 「加载更早」：以当前已载最小 seq 为游标向前翻一页 */
  async function loadEarlier() {
    if (!currentId || minSeq == null || loadingEarlier.value || !hasMore.value) return;
    loadingEarlier.value = true;
    try {
      const page = await bridge.trajectory.listEvents(currentId, TRAJECTORY_PAGE_SIZE, minSeq);
      if (currentId !== null && page.length) {
        minSeq = page[0].seq;
        events.value = [...page, ...events.value];
      }
      hasMore.value = page.length === TRAJECTORY_PAGE_SIZE;
      if (hasMore.value) void refreshTurnOffset(); // M3：窗口起点前移，重查全局偏移
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e);
    } finally {
      loadingEarlier.value = false;
    }
  }

  return { events, loading, loadingEarlier, error, legacy, hasMore, turnOffset, load, loadEarlier, refreshLatest };
}
