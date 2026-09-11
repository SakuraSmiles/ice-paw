// buildRows 行模型单测（轨迹表格数据内核）
import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  buildRows,
  useTrajectory,
  chatOnlyHidden,
  countByFilterKey,
  isChatOnly,
  loadHiddenKinds,
  saveHiddenKinds,
  specialTurnOf,
  specialOfEvent,
  DEFAULT_HIDDEN,
  FILTER_KEYS,
  type EventRow,
  type TurnHeaderRow,
} from "../useTrajectory";
import type { SessionEvent } from "../../types";
import { shortCode } from "../../utils/refs";
import { bridge } from "../../api/bridge";

// composable 用例：bridge 走 Tauri invoke，测试环境用 vi.mock 替身
vi.mock("../../api/bridge", () => ({
  bridge: { trajectory: { listEvents: vi.fn(), turnOffset: vi.fn(), exportJsonl: vi.fn() } },
}));

let seq = 0;
/** 事件构造器：seq 自增，payload 由 kind 决定默认形态 */
function ev(kind: SessionEvent["kind"], payload: unknown, opts: { turnId?: string | null; messageId?: string | null } = {}): SessionEvent {
  seq += 1;
  return {
    id: seq,
    session_id: "s1",
    seq,
    kind,
    actor: "user",
    turn_id: opts.turnId === undefined ? "t1" : opts.turnId,
    message_id: opts.messageId ?? null,
    payload: payload as never,
    created_at: `2026-08-14T10:00:${String(seq % 60).padStart(2, "0")}Z`,
  };
}

function ctx() {
  return { provider: "anthropic", effective_model: "glm-5.2", tools_enabled: true, tool_names: ["bash", "read_file"] };
}
function ended(termination = "stop") {
  return { termination, rounds: 1, usage: { prompt_tokens: 100, completion_tokens: 50, cached_tokens: 0 } };
}

function evRows(events: SessionEvent[], opts?: Partial<Parameters<typeof buildRows>[1]>) {
  const rows = buildRows(events, { collapsedTurns: new Set(), hiddenKinds: new Set(DEFAULT_HIDDEN), query: "", turnOffset: 0, ...opts });
  return {
    rows,
    events: () => rows.filter((r): r is EventRow => r.type === "event"),
    headers: () => rows.filter((r): r is TurnHeaderRow => r.type === "turn-header"),
  };
}

describe("buildRows 行模型", () => {
  it("完整 turn：context/ended 折进头，事件各一行", () => {
    const events = [
      ev("turn_context", ctx()),
      ev("user_message", { content: "帮我查天气", blocks: [] }),
      ev("assistant_message", { content: "好的，我来查", blocks: [], round: 0, continuation: false, token_count: 12 }, { messageId: "m1" }),
      ev("tool_execution", { tool_call_id: "c1", tool_name: "bash", arguments: "{\"cmd\":\"ls\"}", result: "ok", is_error: false, duration_ms: 120 }, { messageId: "m1" }),
      ev("turn_ended", ended()),
    ];
    const { events: evs, headers } = evRows(events);

    expect(headers()).toHaveLength(1);
    const h = headers()[0];
    expect(h.turnId).toBe("t1");
    expect(h.roundCount).toBe(1);
    expect(h.toolCount).toBe(1);
    expect(h.ended?.termination).toBe("stop");
    expect(h.context?.effective_model).toBe("glm-5.2");
    expect(h.dateLabel).toBe("08-14"); // 首个 turn 恒标日期
    expect(h.turnMs).toBe(4000); // 首→末事件墙钟（fixture 秒 1→5）
    expect(h.errorCount).toBe(0);

    expect(evs().map((r) => r.kind)).toEqual(["user", "assistant", "tool"]);
    expect(evs()[2].summary).toContain("bash");
    expect(evs()[2].durationMs).toBe(120);
  });

  it("assistant duration_ms 透传到行 metric 列（纪元早期无字段 → null）", () => {
    const withDur = evRows([
      ev("assistant_message", { content: "回答", blocks: [], round: 0, continuation: false, token_count: 8, duration_ms: 3500 }, { messageId: "m1" }),
    ]).events()[0];
    expect(withDur.durationMs).toBe(3500);

    const legacy = evRows([
      ev("assistant_message", { content: "旧事件无耗时字段", blocks: [], round: 0, continuation: false }, { messageId: "m2" }),
    ]).events()[0];
    expect(legacy.durationMs).toBeNull();
  });

  it("assistant 无正文：思考内容代摘要（斜体弱化态）；与正文共存时取正文", () => {
    const thinkingOnly = [
      ev("assistant_message", { content: "", blocks: [{ type: "thinking", thinking: "先分析用户意图，再决定查文件还是搜索" }, { type: "tool_use", id: "c", name: "bash", input: {} }], round: 0, continuation: false }, { messageId: "m1" }),
    ];
    const r1 = evRows(thinkingOnly).events()[0];
    expect(r1.summary).toContain("先分析用户意图");
    expect(r1.thinkingDerived).toBe(true); // 表格行渲染斜体弱化
    expect(r1.isThinking).toBe(true); // 表格行渲染 Brain 图标

    const both = [
      ev("assistant_message", { content: "正文回复", blocks: [{ type: "thinking", thinking: "内心活动" }], round: 0, continuation: false }, { messageId: "m2" }),
    ];
    const r2 = evRows(both).events()[0];
    expect(r2.summary).toBe("正文回复"); // 摘要不再内嵌符号前缀
    expect(r2.isThinking).toBe(true); // 思考标记走结构化字段（Brain 图标渲染位）
    expect(r2.thinkingDerived).toBe(false);

    const toolOnly = [
      ev("assistant_message", { content: "", blocks: [{ type: "tool_use", id: "c", name: "bash", input: {} }], round: 0, continuation: false }, { messageId: "m3" }),
    ];
    const r3 = evRows(toolOnly).events()[0];
    expect(r3.summary).toBe("(仅工具调用)");
    expect(r3.thinkingDerived).toBe(false);
  });

  it("supersede：同 message_id 多条 assistant 只保留最后一条", () => {
    const events = [
      ev("assistant_message", { content: "前半段", blocks: [], round: 0, continuation: false }, { messageId: "m1" }),
      ev("assistant_message", { content: "前半段+后半段", blocks: [], round: 0, continuation: true }, { messageId: "m1" }),
    ];
    const { events: evs, headers } = evRows(events);
    expect(evs()).toHaveLength(1);
    expect(evs()[0].isContinuation).toBe(true); // 续写标记走结构化字段（RotateCw 图标渲染位）
    expect(headers()[0].roundCount).toBe(1); // 轮数不虚增
  });

  it("折叠：只留 turn 头（头自带统计），事件行不出现", () => {
    const events = [
      ev("user_message", { content: "q", blocks: [] }),
      ev("assistant_message", { content: "a", blocks: [], round: 0, continuation: false }, { messageId: "m1" }),
      ev("tool_execution", { tool_call_id: "c", tool_name: "t", arguments: "{}", is_error: false, duration_ms: 1 }, { messageId: "m1" }),
    ];
    const rows = buildRows(events, { collapsedTurns: new Set(["t1"]), hiddenKinds: new Set(DEFAULT_HIDDEN), query: "", turnOffset: 0 });
    expect(rows.map((r) => r.type)).toEqual(["turn-header"]);
    const h = rows[0];
    expect(h.type === "turn-header" && h.roundCount === 1 && h.toolCount === 1).toBe(true);
  });

  it("搜索：强制展开 + 命中/未命中标记（含 payload 文本命中）", () => {
    const events = [
      ev("user_message", { content: "第一问", blocks: [] }),
      ev("assistant_message", { content: "答一", blocks: [], round: 0, continuation: false }, { messageId: "m1" }),
      ev("user_message", { content: "第二问", blocks: [] }, { turnId: "t2" }),
      ev("assistant_message", { content: "答二", blocks: [], round: 0, continuation: false }, { turnId: "t2", messageId: "m2" }),
      // payload 内字段命中（summary 里不含）
      ev("tool_execution", { tool_call_id: "c", tool_name: "zzz", arguments: "{\"hidden\":\"needle\"}", is_error: false, duration_ms: 1 }, { turnId: "t2", messageId: "m2" }),
    ];
    // 两 turn 都折叠 → 搜索应无视折叠（事件行照常出现）
    const rows = buildRows(events, { collapsedTurns: new Set(["t1", "t2"]), hiddenKinds: new Set(DEFAULT_HIDDEN), query: "needle", turnOffset: 0 });
    const evRowsAll = rows.filter((r): r is EventRow => r.type === "event");
    expect(evRowsAll.length).toBeGreaterThan(0);
    const hit = evRowsAll.find((r) => r.summary.includes("zzz"));
    expect(hit?.match).toBe(true);
    expect(evRowsAll.find((r) => r.summary === "第一问")?.match).toBe(false);
    // 命中数上头
    const h2 = rows.find((r) => r.type === "turn-header" && r.turnKey === "t2");
    expect(h2?.type === "turn-header" && h2.matchCount === 1).toBe(true);
  });

  it("类型筛选：默认藏三类辅助事件；细筛键逐类可控（旧 showAux 布尔细分）", () => {
    const aux = [
      ev("modal_adapted", { stage: "user_image", mode: "ocr_substitute", items: [{ index: 0, outcome: "ocr" }] }),
      ev("hook_injected", { point: "before_llm", prompt: "p" }),
      ev("attachment_stored", { kind: "page", items: [{ idx: 0, name: "a", kind: "pdf", label: "l", token_est: 1 }] }),
    ];
    // 默认（DEFAULT_HIDDEN）：三类全藏
    expect(evRows(aux).events()).toHaveLength(0);
    // 空集 = 全显（含三类辅助）
    expect(evRows(aux, { hiddenKinds: new Set() }).events().map((r) => r.kind)).toEqual(["aux", "aux", "aux"]);
    // 细分：只藏视觉适配 → 钩子注入/附件落库照常
    expect(evRows(aux, { hiddenKinds: new Set(["modal_adapted"]) }).events().map((r) => r.summary))
      .toEqual(["钩子注入 before_llm", "附件落库 ×1"]);
  });

  it("类型筛选：藏非辅助类（工具/错误），turn 头骨架与统计不受影响", () => {
    const events = [
      ev("turn_context", ctx()),
      ev("user_message", { content: "q", blocks: [] }),
      ev("assistant_message", { content: "a", blocks: [], round: 0, continuation: false }, { messageId: "m1" }),
      ev("tool_execution", { tool_call_id: "c1", tool_name: "bash", arguments: "{}", is_error: false, duration_ms: 1 }, { messageId: "m1" }),
      ev("message_error", { kind: "llm", error: "boom" }),
    ];
    const { events: evs, headers } = evRows(events, { hiddenKinds: new Set(["tool", "error", ...DEFAULT_HIDDEN]) });
    expect(evs().map((r) => r.kind)).toEqual(["user", "assistant"]);
    // 头骨架信息不丢（「仅对话」的心智：统计/错误计数仍在）
    expect(headers()).toHaveLength(1);
    expect(headers()[0].toolCount).toBe(1);
    expect(headers()[0].errorCount).toBe(1);
  });

  it("tool_result_message 不生成行（工具行已含结果，它是 DB 行侧镜像）", () => {
    const events = [ev("tool_result_message", { blocks: [] })];
    expect(evRows(events).events()).toHaveLength(0);
  });

  it("孤儿事件（turn_id=null）归入纪元前桶", () => {
    const events = [ev("user_message", { content: "旧世界", blocks: [] }, { turnId: null })];
    const h = evRows(events).headers()[0];
    expect(h.turnId).toBeNull();
    expect(evRows(events).events()[0].summary).toBe("旧世界");
  });

  it("user_message 块徽标：图片/文档/引用计数进摘要（正文与徽标并存）", () => {
    // 纯附件消息（content 空）：徽标即摘要——引用/文档不再从轨迹里消失
    const bare = [ev("user_message", {
      content: "",
      blocks: [
        { type: "image", data: "x", media_type: "image/png" },
        { type: "reference", ref_kind: "conversation", target_id: "c1", display: "会话#1234" },
        { type: "reference", ref_kind: "agent", target_id: "a1", display: "审查员#5678" },
        { type: "attachment", name: "spec.pdf", kind: "pdf", size: 4096 },
      ],
    })];
    expect(evRows(bare).events()[0].summary).toBe("[图片 ×1 · 文档 ×1 · 引用 ×2]");

    // 有正文：正文 + 徽标
    const withText = [ev("user_message", {
      content: "看看这些材料",
      blocks: [{ type: "attachment", name: "spec.pdf", kind: "pdf", size: 4096 }],
    })];
    expect(evRows(withText).events()[0].summary).toBe("看看这些材料 [文档 ×1]");

    // 纯文本：行为不变（无徽标尾巴）
    const plain = [ev("user_message", { content: "第一问", blocks: [] })];
    expect(evRows(plain).events()[0].summary).toBe("第一问");
  });

  it("多 turn 顺序与 turnIndex 递增", () => {
    const events = [
      ev("user_message", { content: "q1", blocks: [] }, { turnId: "t1" }),
      ev("user_message", { content: "q2", blocks: [] }, { turnId: "t2" }),
      ev("user_message", { content: "q3", blocks: [] }, { turnId: "t3" }),
    ];
    const hs = evRows(events).headers();
    expect(hs.map((h) => h.turnIndex)).toEqual([0, 1, 2]);
    expect(hs.map((h) => h.dateLabel)).toEqual(["08-14", null, null]); // 同日只在首个 turn 标日期
  });

  it("M3：turnOffset 让窗口内轮号从全局偏移起算（尾部优先分页）", () => {
    const events = [
      ev("user_message", { content: "q2", blocks: [] }, { turnId: "t2" }),
      ev("user_message", { content: "q3", blocks: [] }, { turnId: "t3" }),
    ];
    const hs = evRows(events, { turnOffset: 7 }).headers();
    expect(hs.map((h) => h.turnIndex)).toEqual([7, 8]); // 窗口前有 7 轮 → 首桶是第 8 轮
    // 偏移 0 = 窗口从头开始，行为与原实现一致
    expect(evRows(events).headers().map((h) => h.turnIndex)).toEqual([0, 1]);
  });

  it("specialTurnOf/specialOfEvent：chain:/cross:/election: 判轮外段，coordinator 看 kind", () => {
    expect(specialTurnOf("chain:m1")).toBe("channel");
    expect(specialTurnOf("cross:x1")).toBe("cross");
    expect(specialTurnOf("election:m1")).toBe("election");
    expect(specialTurnOf("s1::chain:m1")).toBe("channel"); // 项目时间线 scopeTurnKeys 形态
    expect(specialTurnOf("s1::cross:x1")).toBe("cross");
    expect(specialTurnOf("s1::election:m1")).toBe("election");
    expect(specialTurnOf("t1")).toBeNull();
    expect(specialTurnOf("s1::t1")).toBeNull();
    expect(specialTurnOf(null)).toBeNull();
    // specialOfEvent 超集：channel_coordinator 落 turn_id=NULL——单看 turn_key 判不出
    expect(specialOfEvent({ kind: "channel_coordinator", turn_id: null })).toBe("coordinator");
    expect(specialOfEvent({ kind: "channel_mention", turn_id: "chain:m1" })).toBe("channel");
    expect(specialOfEvent({ kind: "assistant_message", turn_id: null })).toBeNull(); // 真孤儿事件不误判
  });

  it("特殊段交错切桶：不占轮号、头 key 唯一、头字段取段首事件、同键多段齐折叠", () => {
    // 频道时间线交错形态：m1 回合 → chain 派发段 → m2 全回合 → chain 成员报告尾 @ 段
    const events = [
      ev("user_message", { content: "q1", blocks: [] }, { turnId: "m1", messageId: "m1" }),
      ev("channel_mention", { from_agent_id: null, to_agent_id: "ag2", hop_index: 1, chain_remaining: 2, blocked_reason: null }, { turnId: "chain:m1" }),
      ev("user_message", { content: "q2", blocks: [] }, { turnId: "m2", messageId: "m2" }),
      ev("assistant_message", { content: "a2", blocks: [], round: 0, continuation: false }, { turnId: "m2", messageId: "m2" }),
      ev("turn_ended", ended(), { turnId: "m2" }),
      ev("channel_mention", { from_agent_id: "ag3", to_agent_id: "ag2", hop_index: 2, chain_remaining: 1, blocked_reason: null }, { turnId: "chain:m1" }),
    ];
    const { rows, headers } = evRows(events);
    expect(headers()).toHaveLength(4); // m1 / chain 段1 / m2 / chain 段2
    expect(headers().map((h) => h.special)).toEqual([null, "channel", null, "channel"]);
    // 轮号：chain 段不占号 → m2 是第 2 轮（原实现被顶到第 3 轮）
    expect(headers().map((h) => h.turnIndex)).toEqual([0, 0, 1, 1]);
    // 行 key 全局唯一（原 th-${tk} 在交错段重复 → v-for 警告/行复用错位）
    expect(new Set(rows.map((r) => r.key)).size).toBe(rows.length);
    // 交错段头字段取段首事件自身（段 2 首 event = seq 6），非全局聚合首
    expect(headers()[3].createdAt).toBe(events[5].created_at);
    expect(headers()[3].turnMs).toBeNull(); // 特殊段无墙钟跨度（turnStats 跨着别人的回合）
    // 折叠 chain:m1：同 turnKey 两段齐收（既有折叠语义保持）
    const collapsed = evRows(events, { collapsedTurns: new Set(["chain:m1"]) }).rows;
    expect(collapsed.filter((r) => r.type === "turn-header" && r.turnKey === "chain:m1")).toHaveLength(2);
    expect(collapsed.filter((r) => r.type === "turn-header" && r.turnKey === "chain:m1" && r.collapsed)).toHaveLength(2);
    expect(collapsed.filter((r) => r.type === "event" && r.turnKey === "chain:m1")).toHaveLength(0);
  });

  it("同轮二段不膨胀：频道链上同 turn_id 被切段后复用首段号（DISTINCT turn_id 口径）", () => {
    // 生产证据形态（频道 19660d0f）：链上成员回合共用链头消息 id 作 turn_id，回合间
    // 穿插 chain: 派发段 → 同一 turn_id 被切成多段。段切换 ≠ 新轮——轮号按「新
    // turn_id 首见」递增，与后端 count_turns_before 的 DISTINCT 口径对齐。
    const seg = (turn: string, mid: string) => [
      ev("turn_context", ctx(), { turnId: turn }),
      ev("assistant_message", { content: `答 ${mid}`, blocks: [], round: 0, continuation: false }, { turnId: turn, messageId: mid }),
      ev("turn_ended", ended(), { turnId: turn }),
    ];
    const events = [
      ev("user_message", { content: "q1", blocks: [] }, { turnId: "m1", messageId: "m1" }),
      ...seg("m1", "a1"),
      ev("channel_mention", { from_agent_id: "ag2", to_agent_id: "ag3", hop_index: 2, chain_remaining: 1, blocked_reason: null }, { turnId: "chain:m1" }),
      ...seg("m1", "a2"), // 同 turn_id 二段（接力成员的回合）
      ev("channel_mention", { from_agent_id: "ag3", to_agent_id: "ag2", hop_index: 3, chain_remaining: 0, blocked_reason: null }, { turnId: "chain:m1" }),
      ...seg("m1", "a3"), // 同 turn_id 三段
      ev("user_message", { content: "q2", blocks: [] }, { turnId: "m2", messageId: "m2" }),
      ...seg("m2", "b1"),
    ];
    const hs = evRows(events).headers();
    // 头：m1 段1 / chain 段1 / m1 二段 / chain 段2（同键多段各自成头，key 带 seq 后缀唯一）
    // / m1 三段 / m2
    expect(hs).toHaveLength(6);
    expect(hs.map((h) => h.special)).toEqual([null, "channel", null, "channel", null, null]);
    // 原实现「段切换即新轮」把 m1 数成 3 轮（生产实案 16 真轮膨胀成 47 头号）；
    // 新口径 m1 三段恒第 1 轮、m2 第 2 轮
    expect(hs.map((h) => h.turnIndex)).toEqual([0, 0, 0, 0, 0, 1]);
    // 同轮二段头字段取段首事件自身（events[5] = 二段 turn_context）、无墙钟跨度
    expect(hs[2].seq).toBe(events[5].seq);
    expect(hs[2].createdAt).toBe(events[5].created_at);
    expect(hs[2].turnMs).toBeNull();
    // 首段头仍是全轮聚合口径（统计/终止照常）
    expect(hs[0].ended?.termination).toBe("stop");
    expect(hs[0].roundCount).toBe(3);
  });

  it("election:/channel_coordinator 轮外段：不占轮号、coordinator 免「纪元前事件」错标", () => {
    // ⑮ 补⑭ 漏网：election:{发起id} 段与 turn_id=NULL 的统筹位变更事件同为轮外事实
    const events = [
      ev("user_message", { content: "q1", blocks: [] }, { turnId: "m1", messageId: "m1" }),
      ev("channel_election", { phase: "started" }, { turnId: "election:m1" }),
      ev("channel_election", { phase: "vote", vote: { voter_agent_id: "a", candidate_agent_id: "b" } }, { turnId: "election:m1" }),
      ev("channel_coordinator", { action: "elected", agent_id: "b" }, { turnId: null }),
      ev("user_message", { content: "q2", blocks: [] }, { turnId: "m2", messageId: "m2" }),
    ];
    const hs = evRows(events).headers();
    expect(hs.map((h) => h.special)).toEqual([null, "election", "coordinator", null]);
    // 轮外不占号 → m2 仍是第 2 轮（原实现被顶到第 4 轮；coordinator 落孤儿桶占号）
    expect(hs.map((h) => h.turnIndex)).toEqual([0, 0, 0, 1]);
    // coordinator 头按 special 特殊化，不再错标「纪元前事件」（turnLabel 词表分派）
    expect(hs[2].turnId).toBeNull();
    expect(hs[2].special).toBe("coordinator");
    expect(hs[2].turnMs).toBeNull();
  });

  it("M2：搜索文本跨 buildRows 调用命中一致（WeakMap 缓存不改变 match 语义）", () => {
    const events = [
      ev("user_message", { content: "第一问", blocks: [] }),
      ev("tool_execution", { tool_call_id: "c", tool_name: "t", arguments: "{\"deep\":\"cached-needle\"}", is_error: false, duration_ms: 1 }, { messageId: "m1" }),
    ];
    // 同一批事件对象多次调用（模拟折叠/开关切换重算）：payload 深处命中结果稳定
    for (let i = 0; i < 3; i++) {
      const rows = buildRows(events, { collapsedTurns: new Set(), hiddenKinds: new Set(DEFAULT_HIDDEN), query: "cached-needle", turnOffset: 0 });
      const evs = rows.filter((r): r is EventRow => r.type === "event");
      expect(evs.find((r) => r.summary === "第一问")?.match).toBe(false);
      expect(evs.find((r) => r.kind === "tool")?.match).toBe(true);
    }
  });

  it("错误/丢弃/摘要各成行，错误行带 isError", () => {
    const events = [
      ev("message_error", { kind: "llm", error: "boom" }),
      ev("message_discarded", { reason: "用户取消" }),
      ev("summary_created", { summary_message_id: "sm", content: "摘要内容", covered_until_rowid: 42 }),
    ];
    const es = evRows(events).events();
    expect(es.map((r) => r.kind)).toEqual(["error", "discarded", "summary"]);
    expect(es[0].isError).toBe(true);
    expect(es[0].summary).toContain("boom");
    expect(es[2].summary).toBe("摘要内容");
    expect(evRows(events).headers()[0].errorCount).toBe(1);
  });

  it("未结束 turn：头 ended=null（进行中/崩溃）", () => {
    const events = [ev("user_message", { content: "q", blocks: [] })];
    expect(evRows(events).headers()[0].ended).toBeNull();
  });

  // ---- context_breakdown 折头（③ 可观测化；同 turn_context 先例不生成行） ----

  const breakdownPayload = {
    v: 1,
    segments: [
      { label: "system_persona", est: 300 },
      { label: "tool_defs", est: 2400, count: 24 },
      { label: "history", est: 900, count: 4 },
    ],
    est_total: 3600,
    actual_prompt_tokens: 4000,
    fingerprint: { tools: "aaa", system_stable: "bbb", os_stable: "ccc" },
    rounds: [{ prompt: 4000, cached: 0, tools_hash: "aaa", injected: false, model_switched: false }],
  };

  it("context_breakdown 折进头（header.breakdown），不生成事件行", () => {
    const events = [
      ev("turn_context", ctx()),
      ev("user_message", { content: "q", blocks: [] }),
      ev("assistant_message", { content: "a", blocks: [], round: 0, continuation: false }, { messageId: "m1" }),
      ev("turn_ended", ended()),
      ev("context_breakdown", breakdownPayload),
    ];
    const { events: evs, headers } = evRows(events);
    const h = headers()[0];
    expect(h.breakdown).not.toBeNull();
    expect(h.breakdown?.est_total).toBe(3600);
    expect(h.breakdown?.rounds).toHaveLength(1);
    // 不生成行：事件行仍是 user + assistant 两行
    expect(evs().map((r) => r.kind)).toEqual(["user", "assistant"]);
  });

  it("无 breakdown 的 turn 头 breakdown=null（无 usage 回合/旧回合）", () => {
    const events = [ev("user_message", { content: "q", blocks: [] })];
    expect(evRows(events).headers()[0].breakdown).toBeNull();
  });
});

describe("类型筛选模型（FilterKey / 预设 / 计数 / 持久化）", () => {
  it("结构：FILTER_KEYS 13 键唯一，DEFAULT_HIDDEN 是其子集（结构锁范式）", () => {
    expect(FILTER_KEYS).toHaveLength(13);
    expect(new Set(FILTER_KEYS).size).toBe(13);
    for (const k of DEFAULT_HIDDEN) expect(FILTER_KEYS).toContain(k);
  });

  it("行为锁：每个生成行的事件 kind 空隐藏集下恰出一行（防映射漏键静默吞行）", () => {
    // 与 summarizeEvent 的产出 kind 集合保持同步——新增生成行的事件 kind 两边一起补
    const fixtures: [SessionEvent["kind"], unknown][] = [
      ["user_message", { content: "q", blocks: [] }],
      ["assistant_message", { content: "a", blocks: [], round: 0, continuation: false }],
      ["tool_execution", { tool_call_id: "c", tool_name: "t", arguments: "{}", is_error: false, duration_ms: 1 }],
      ["summary_created", { summary_message_id: "sm", content: "s", covered_until_rowid: 1 }],
      ["summary_updated", { summary_message_id: "sm", content: "s", covered_until_seq: 1 }],
      ["plan_updated", { items: [{ text: "x", status: "todo" }] }],
      ["message_error", { kind: "llm", error: "e" }],
      ["message_discarded", { reason: "r" }],
      ["model_switch", { from_model: "", to_profile_id: "mp-b", to_alias: "备用档", to_model: "glm-backup", reason: "quota", attempt: 1 }],
      ["cross_session_message", { message_id: "x1", source_conversation_id: "c-src", source_conversation_title: "主控", source_agent_id: "a1", source_agent_name: "甲", content: "材质定稿了吗", expect_reply: true, delivered_at_unix: 1 }],
      ["cross_session_message_settled", { message_id: "x1", action: "consumed", by: "user-approval" }],
      ["channel_mention", { from_agent_id: "ag1", to_agent_id: "ag2", hop_index: 1, chain_remaining: 2, blocked_reason: null }],
      ["channel_election", { phase: "vote", vote: { voter_agent_id: "ag1", candidate_agent_id: "ag2" } }],
      ["channel_coordinator", { action: "appointed", agent_id: "ag1" }],
      ["modal_adapted", { stage: "s", mode: "m", items: [] }],
      ["hook_injected", { point: "p", prompt: "x" }],
      ["attachment_stored", { kind: "page", items: [] }],
    ];
    for (const [kind, payload] of fixtures) {
      expect(evRows([ev(kind, payload)], { hiddenKinds: new Set() }).events()).toHaveLength(1);
    }
  });

  it("跨会话来件（MA-3）：CROSS 行 + 来源摘要；settled 中文 by 标注；默认不隐藏（待办事实）", () => {
    const rows = evRows([
      ev("cross_session_message", { message_id: "x1", source_conversation_id: "c-src", source_conversation_title: "主控", source_agent_id: "a1", source_agent_name: "甲", content: "材质定稿了吗", expect_reply: true, delivered_at_unix: 1 }),
      ev("cross_session_message_settled", { message_id: "x1", action: "consumed", by: "user-approval" }),
    ]);
    const [msg, settled] = rows.events();
    expect(msg.kind).toBe("cross");
    expect(msg.label).toBe("CROSS");
    expect(msg.summary).toContain("主控");
    expect(msg.summary).toContain("甲");
    expect(msg.summary).toContain("期待回复");
    expect(msg.summary).toContain("材质定稿了吗");
    expect(settled.summary).toBe("已消费（用户批准）");
    expect(DEFAULT_HIDDEN).not.toContain("cross_session");
  });

  it("model_switch：SWITCH 行 + 中文原因摘要（quota → 额度耗尽）；默认不隐藏（可见的过程事实）", () => {
    const rows = evRows([
      ev("model_switch", { from_model: "glm-5.3", from_profile_id: "mp-main", to_profile_id: "mp-b", to_alias: "备用档", to_model: "glm-backup", reason: "quota", attempt: 1 }),
    ]);
    const r = rows.events()[0];
    expect(r.kind).toBe("switch");
    expect(r.label).toBe("SWITCH");
    expect(r.summary).toContain("备用档");
    expect(r.summary).toContain("glm-backup");
    expect(r.summary).toContain("额度耗尽");
    expect(r.isError).toBe(false);
    // 默认隐藏集不含 model_switch（三类辅助事件才默认藏）
    expect(DEFAULT_HIDDEN).not.toContain("model_switch");
  });

  it("频道协作三 kind（v1 串行共享流）：CROSS 行 + 短码摘要；计数入 channel 键；默认不隐藏", () => {
    const tag = (id: string) => `成员#${shortCode(id)}`;
    const rows = evRows([
      ev("channel_mention", { from_agent_id: "ag1", to_agent_id: "ag2", hop_index: 1, chain_remaining: 2, blocked_reason: null }),
      ev("channel_mention", { from_agent_id: null, to_agent_id: "ag1", hop_index: 1, chain_remaining: 0, blocked_reason: "user_preempted" }),
      // broadcast 位分野（生产实案：广播接令被误读成「用户 @ 过」）
      ev("channel_mention", { from_agent_id: null, to_agent_id: "ag1", hop_index: 1, chain_remaining: 0, broadcast: true, blocked_reason: null }),
      ev("channel_mention", { from_agent_id: null, to_agent_id: "ag2", hop_index: 0, chain_remaining: 0, broadcast: true, blocked_reason: "coordinator_failed" }),
      ev("channel_election", { phase: "result", result: { tally: [{ agent_id: "ag2", votes: 2 }], winner_agent_id: "ag2" } }),
      ev("channel_coordinator", { action: "failed-over", agent_id: "ag2" }),
    ]);
    const evs = rows.events();
    expect(evs.map((r) => r.kind)).toEqual(["cross", "cross", "cross", "cross", "cross", "cross"]);
    expect(evs[0].label).toBe("CROSS");
    // 正常点名：from → to + 跳数/余链；拦截态：blocked_reason 原样 + to 短码
    expect(evs[0].summary).toBe(`${tag("ag1")} → 点名 ${tag("ag2")} 接力（第 1 跳 · 余 2）`);
    expect(evs[1].summary).toBe(`点名被拦（user_preempted）：${tag("ag1")}`);
    // 广播接令 ≠ 点名（from 空由 broadcast 分野）；广播拦截态同理
    expect(evs[2].summary).toBe(`广播 · ${tag("ag1")} 接令（第 1 跳 · 余 0）`);
    expect(evs[3].summary).toBe(`广播接令被拦（coordinator_failed）：${tag("ag2")}`);
    expect(evs[4].summary).toBe(`当选统筹者：${tag("ag2")}`);
    expect(evs[5].summary).toBe(`${tag("ag2")} 统筹故障换帅至`);
    // 计数走 channel 键（EV_KIND_TO_FILTER 三 kind 同键）
    const c = countByFilterKey([
      ev("channel_mention", { from_agent_id: "ag1", to_agent_id: "ag2", hop_index: 1, chain_remaining: 0, blocked_reason: null }),
      ev("channel_election", { phase: "started" }),
    ]);
    expect(c.channel).toBe(2);
    // 协作过程事实默认可见（三类辅助事件才默认藏）
    expect(DEFAULT_HIDDEN).not.toContain("channel");
  });

  it("「仅对话」预设：只留用户/回复；isChatOnly 派生判据（无第二状态源）", () => {
    const h = chatOnlyHidden();
    expect(isChatOnly(h)).toBe(true);
    // 默认态/全显态/半吊子态都不是「仅对话」
    expect(isChatOnly(new Set(DEFAULT_HIDDEN))).toBe(false);
    expect(isChatOnly(new Set())).toBe(false);
    const partial = new Set(h);
    partial.delete("tool");
    expect(isChatOnly(partial)).toBe(false);

    const events = [
      ev("user_message", { content: "q", blocks: [] }),
      ev("assistant_message", { content: "a", blocks: [], round: 0, continuation: false }, { messageId: "m1" }),
      ev("tool_execution", { tool_call_id: "c", tool_name: "t", arguments: "{}", is_error: false, duration_ms: 1 }, { messageId: "m1" }),
      ev("summary_created", { summary_message_id: "sm", content: "s", covered_until_rowid: 1 }),
    ];
    expect(evRows(events, { hiddenKinds: h }).events().map((r) => r.kind)).toEqual(["user", "assistant"]);
  });

  it("countByFilterKey：单遍计数，折进头/镜像 kind 不计", () => {
    const c = countByFilterKey([
      ev("user_message", { content: "q", blocks: [] }),
      ev("turn_context", ctx()),
      ev("tool_execution", { tool_call_id: "c1", tool_name: "t", arguments: "{}", is_error: false, duration_ms: 1 }),
      ev("tool_execution", { tool_call_id: "c2", tool_name: "t", arguments: "{}", is_error: false, duration_ms: 1 }),
      ev("attachment_stored", { kind: "page", items: [] }),
    ]);
    expect(c.user).toBe(1);
    expect(c.tool).toBe(2);
    expect(c.attachment_stored).toBe(1);
    expect(Object.keys(c).sort()).toEqual(["attachment_stored", "tool", "user"]);
  });

  it("持久化：save→load 往返；坏值/未知键清洗回默认；空数组是合法全显态", () => {
    localStorage.clear();
    expect([...loadHiddenKinds()].sort()).toEqual([...DEFAULT_HIDDEN].sort()); // 未存过 → 默认

    saveHiddenKinds(new Set(["tool", "modal_adapted"]));
    expect([...loadHiddenKinds()].sort()).toEqual(["modal_adapted", "tool"]);

    saveHiddenKinds(new Set()); // 显式全显
    expect(loadHiddenKinds().size).toBe(0);

    localStorage.setItem("icepaw-traj-hidden-kinds", "{oops"); // 坏 JSON
    expect([...loadHiddenKinds()].sort()).toEqual([...DEFAULT_HIDDEN].sort());

    localStorage.setItem("icepaw-traj-hidden-kinds", JSON.stringify(["tool", "legacy_kind_x"])); // 未知键剔除
    expect([...loadHiddenKinds()]).toEqual(["tool"]);
  });
});

describe("useTrajectory composable（live 追加）", () => {
  beforeEach(() => {
    seq = 0; // ev() 构造器的文件级计数器归零（前述 describe 已消耗）
    vi.mocked(bridge.trajectory.listEvents).mockReset();
  });

  it("并发 refreshLatest 不重复拼接（push 通知与兜底轮询竞态回归）", async () => {
    const { events, load, refreshLatest } = useTrajectory();
    // 首次载入两页内的小会话（不满页 → hasMore=false，不触发 turnOffset 查询）
    const e1 = ev("user_message", { content: "q", blocks: [] });
    const e2 = ev("assistant_message", { content: "a", blocks: [], round: 0, continuation: false }, { messageId: "m1" });
    vi.mocked(bridge.trajectory.listEvents).mockResolvedValueOnce([e1, e2]);
    await load("c1");
    expect(events.value.map((e) => e.seq)).toEqual([1, 2]);

    // 竞态模拟：两次增量拉取都在对方拼接前发起，拿到同批 [e3]
    const e3 = ev("tool_execution", { tool_call_id: "c", tool_name: "t", arguments: "{}", is_error: false, duration_ms: 1 }, { messageId: "m1" });
    vi.mocked(bridge.trajectory.listEvents).mockImplementation(async () => [e3]);
    const [a, b] = await Promise.all([refreshLatest(), refreshLatest()]);

    // 恰一次计入新事件；数组里 e3 只出现一次（修复前会被拼接两次 → 重复行）
    expect(a + b).toBe(1);
    expect(events.value.filter((e) => e.seq === e3.seq)).toHaveLength(1);
    expect(events.value.map((e) => e.seq)).toEqual([1, 2, 3]);
  });

  it("追平后增量为空：返回 0 且不动数组", async () => {
    const { events, load, refreshLatest } = useTrajectory();
    const e1 = ev("user_message", { content: "q", blocks: [] });
    vi.mocked(bridge.trajectory.listEvents).mockResolvedValueOnce([e1]);
    await load("c1");
    vi.mocked(bridge.trajectory.listEvents).mockResolvedValue([]);
    await expect(refreshLatest()).resolves.toBe(0);
    expect(events.value.map((e) => e.seq)).toEqual([1]);
  });
});
