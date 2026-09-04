// buildRows 行模型单测（轨迹表格数据内核）
import { describe, expect, it, vi, beforeEach } from "vitest";
import { buildRows, useTrajectory, type EventRow, type TurnHeaderRow } from "../useTrajectory";
import type { SessionEvent } from "../../types";
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
  const rows = buildRows(events, { collapsedTurns: new Set(), showAux: false, query: "", turnOffset: 0, ...opts });
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
    const rows = buildRows(events, { collapsedTurns: new Set(["t1"]), showAux: false, query: "", turnOffset: 0 });
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
    const rows = buildRows(events, { collapsedTurns: new Set(["t1", "t2"]), showAux: false, query: "needle", turnOffset: 0 });
    const evRowsAll = rows.filter((r): r is EventRow => r.type === "event");
    expect(evRowsAll.length).toBeGreaterThan(0);
    const hit = evRowsAll.find((r) => r.summary.includes("zzz"));
    expect(hit?.match).toBe(true);
    expect(evRowsAll.find((r) => r.summary === "第一问")?.match).toBe(false);
    // 命中数上头
    const h2 = rows.find((r) => r.type === "turn-header" && r.turnKey === "t2");
    expect(h2?.type === "turn-header" && h2.matchCount === 1).toBe(true);
  });

  it("辅助事件默认隐藏，showAux 打开", () => {
    const aux = [
      ev("modal_adapted", { stage: "user_image", mode: "ocr_substitute", items: [{ index: 0, outcome: "ocr" }] }),
      ev("hook_injected", { point: "before_llm", prompt: "p" }),
      ev("attachment_stored", { kind: "page", items: [{ idx: 0, name: "a", kind: "pdf", label: "l", token_est: 1 }] }),
    ];
    expect(evRows(aux).events()).toHaveLength(0);
    expect(evRows(aux, { showAux: true }).events().map((r) => r.kind)).toEqual(["aux", "aux", "aux"]);
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

  it("M2：搜索文本跨 buildRows 调用命中一致（WeakMap 缓存不改变 match 语义）", () => {
    const events = [
      ev("user_message", { content: "第一问", blocks: [] }),
      ev("tool_execution", { tool_call_id: "c", tool_name: "t", arguments: "{\"deep\":\"cached-needle\"}", is_error: false, duration_ms: 1 }, { messageId: "m1" }),
    ];
    // 同一批事件对象多次调用（模拟折叠/开关切换重算）：payload 深处命中结果稳定
    for (let i = 0; i < 3; i++) {
      const rows = buildRows(events, { collapsedTurns: new Set(), showAux: false, query: "cached-needle", turnOffset: 0 });
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
