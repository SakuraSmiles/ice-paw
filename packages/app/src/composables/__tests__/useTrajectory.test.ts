// buildRows 行模型单测（轨迹表格数据内核）
import { describe, expect, it } from "vitest";
import { buildRows, type EventRow, type TurnHeaderRow } from "../useTrajectory";
import type { SessionEvent } from "../../types";

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
  const rows = buildRows(events, { collapsedTurns: new Set(), showAux: false, query: "", ...opts });
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

    const both = [
      ev("assistant_message", { content: "正文回复", blocks: [{ type: "thinking", thinking: "内心活动" }], round: 0, continuation: false }, { messageId: "m2" }),
    ];
    const r2 = evRows(both).events()[0];
    expect(r2.summary).toContain("💭 正文回复");
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
    expect(evs()[0].summary).toContain("↻"); // 续写标记
    expect(headers()[0].roundCount).toBe(1); // 轮数不虚增
  });

  it("折叠：只留 turn 头（头自带统计），事件行不出现", () => {
    const events = [
      ev("user_message", { content: "q", blocks: [] }),
      ev("assistant_message", { content: "a", blocks: [], round: 0, continuation: false }, { messageId: "m1" }),
      ev("tool_execution", { tool_call_id: "c", tool_name: "t", arguments: "{}", is_error: false, duration_ms: 1 }, { messageId: "m1" }),
    ];
    const rows = buildRows(events, { collapsedTurns: new Set(["t1"]), showAux: false, query: "" });
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
    const rows = buildRows(events, { collapsedTurns: new Set(["t1", "t2"]), showAux: false, query: "needle" });
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
