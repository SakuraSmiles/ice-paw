// TrajectoryTable.earlierGate.test.ts — 「加载更早」胶囊门控回归锁
// （2026-09-16 生产反馈：胶囊在未滚到顶时也出现、且不消失）。
//
// 机制：earlierOn = loadingEarlier || scrollTop ≤ 80。scrollTop ref 平时只靠
// scroll 事件喂——jsdom 里直接赋值 el.scrollTop 不派发 scroll 事件，恰好忠实
// 模拟「真实浏览器中不触发 scroll 的滚动变化」类（隐藏 tab 挂载/内容增缩钳制/
// 程序性赋值）。锁三点：初始在顶可见 / 静默滚动后陈旧可见（bug 形态存档）→
// rows 变化经 post-flush 重读 DOM 自愈隐没 / 真实 scroll 事件路径照常 +
// loadingEarlier 强制可见不变。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import TrajectoryTable from "../TrajectoryTable.vue";
import { buildRows, DEFAULT_HIDDEN } from "../../../composables/useTrajectory";
import { scopeTurnKeys } from "../../../composables/useProjectTrajectory";
import type { ProjectEvent } from "../../../types";

/** 一轮对话事件（user 提问 + assistant 回答） */
function roundEvents(n = 1): ProjectEvent[] {
  const out: ProjectEvent[] = [];
  for (let i = 0; i < n; i++) {
    out.push({
      id: out.length + 1,
      session_id: "s1",
      seq: out.length + 1,
      kind: "user_message",
      actor: "user",
      turn_id: `t${i}`,
      message_id: null,
      payload: { content: `提问 ${i}`, blocks: [] },
      created_at: `2026-09-16T10:00:${String(i).padStart(2, "0")}Z`,
      session_title: "主对话",
      session_kind: "chat",
    } as ProjectEvent);
    out.push({
      id: out.length + 1,
      session_id: "s1",
      seq: out.length + 1,
      kind: "assistant_message",
      actor: "agent",
      turn_id: `t${i}`,
      message_id: `m${i}`,
      payload: { content: `回答 ${i}`, blocks: [], round: 0, continuation: false },
      created_at: `2026-09-16T10:00:${String(i).padStart(2, "0")}Z`,
      session_title: "主对话",
      session_kind: "chat",
    } as ProjectEvent);
  }
  return out;
}

function rowsOf(events: ProjectEvent[]) {
  return buildRows(scopeTurnKeys(events), {
    collapsedTurns: new Set(),
    hiddenKinds: new Set(DEFAULT_HIDDEN),
    query: "",
    turnOffset: 0,
  });
}

function mountTable(rows: ReturnType<typeof rowsOf>, props: Record<string, unknown> = {}) {
  return mount(TrajectoryTable, {
    props: {
      rows,
      selectedSeq: null,
      selectedTurnKey: null,
      searching: false,
      searchQuery: "",
      hasMore: true,
      loadingEarlier: false,
      ...props,
    },
  });
}

function gate(w: ReturnType<typeof mount>) {
  return w.find(".ttab-earlier");
}

describe("「加载更早」顶部门控", () => {
  it("初始在顶（scrollTop=0）：胶囊可见", () => {
    const w = mountTable(rowsOf(roundEvents()));
    expect(gate(w).classes()).toContain("on");
  });

  it("loadingEarlier 强制可见：加载中点击反馈不消失（与滚动位置无关）", async () => {
    const w = mountTable(rowsOf(roundEvents()));
    (w.find(".ttab").element as HTMLElement).scrollTop = 500; // 静默离顶
    await w.setProps({ loadingEarlier: true });
    expect(gate(w).classes()).toContain("on");
    await w.setProps({ loadingEarlier: false });
    // 加载结束：rows 已变化（前插一页），post-flush 自愈读回 500 → 隐没
    await w.setProps({ rows: rowsOf([...roundEvents(2), ...roundEvents()]) });
    expect(gate(w).classes()).not.toContain("on");
  });

  it("静默滚动陈旧自愈（生产实案回归锁）：无 scroll 事件的离顶 → 胶囊滞留可见；任一 rows 变化后重读 DOM 归真隐没", async () => {
    const w = mountTable(rowsOf(roundEvents()));
    expect(gate(w).classes()).toContain("on");

    // jsdom 直接赋值不派发 scroll 事件——模拟隐藏期挂载/钳制类「滚动变了但
    // 无人通知」路径。旧实现：scrollTop ref 滞留 0 → 胶囊常显不退（bug 形态）。
    (w.find(".ttab").element as HTMLElement).scrollTop = 500;
    await w.setProps({ rows: rowsOf(roundEvents(2)) }); // 任何 rows 变化（折叠/筛选/分页）
    expect(gate(w).classes()).not.toContain("on");

    // 滚回顶部后 rows 再变化 → 重新可见（自愈是双向归真，不是单向隐藏）
    (w.find(".ttab").element as HTMLElement).scrollTop = 30;
    await w.setProps({ rows: rowsOf(roundEvents()) });
    expect(gate(w).classes()).toContain("on");
  });

  it("真实 scroll 事件路径照常：滚动即更新（不依赖 rows 变化）", async () => {
    const w = mountTable(rowsOf(roundEvents()));
    const el = w.find(".ttab");
    (el.element as HTMLElement).scrollTop = 500;
    await el.trigger("scroll");
    expect(gate(w).classes()).not.toContain("on");

    (el.element as HTMLElement).scrollTop = 10;
    await el.trigger("scroll");
    expect(gate(w).classes()).toContain("on");
  });
});
