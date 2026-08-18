// TrajectoryTable.sessionMeta.test.ts — 跨会话合并流的可选扩展回归：
// sessionMeta 不传零变化（单会话路径）/ 传入时 turn 头渲染会话徽章（delegation
// tint 令牌）/ selectedKey 跨会话同 seq 精确高亮（seq 在项目流里不唯一）。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import TrajectoryTable from "../TrajectoryTable.vue";
import { buildRows } from "../../../composables/useTrajectory";
import { scopeTurnKeys } from "../../../composables/useProjectTrajectory";
import type { ProjectEvent } from "../../../types";

/** 两会话各一轮，turn_id 同名、seq 编号重叠（项目流真实形态） */
function mergedEvents(): ProjectEvent[] {
  const mk = (id: number, session: string, seq: number, kind: "user_message" | "assistant_message"): ProjectEvent =>
    ({
      id,
      session_id: session,
      seq,
      kind,
      actor: "user",
      turn_id: "t1",
      message_id: kind === "assistant_message" ? "m1" : null,
      payload:
        kind === "user_message"
          ? { content: `来自 ${session} 的提问`, blocks: [] }
          : { content: `来自 ${session} 的回答`, blocks: [], round: 0, continuation: false },
      created_at: `2026-08-18T10:00:${String(seq).padStart(2, "0")}Z`,
      session_title: session === "s1" ? "主对话" : "委派任务",
      session_kind: session === "s1" ? "chat" : "delegation",
    }) as ProjectEvent;
  return [
    mk(1, "s1", 1, "user_message"),
    mk(2, "s1", 2, "assistant_message"),
    mk(3, "s2", 1, "user_message"), // 同 seq 不同会话
    mk(4, "s2", 2, "assistant_message"),
  ];
}

function mountTable(props: Record<string, unknown> = {}) {
  const events = mergedEvents();
  const rows = buildRows(scopeTurnKeys(events), {
    collapsedTurns: new Set(),
    showAux: false,
    query: "",
    turnOffset: 0,
  });
  return mount(TrajectoryTable, {
    props: {
      rows,
      selectedSeq: null,
      selectedTurnKey: null,
      searching: false,
      searchQuery: "",
      hasMore: false,
      loadingEarlier: false,
      ...props,
    },
  });
}

describe("TrajectoryTable 跨会话扩展（可选 prop 回归）", () => {
  it("sessionMeta 不传 = 单会话路径零变化（无徽章）", () => {
    const w = mountTable();
    expect(w.findAll(".th-session")).toHaveLength(0);
    // 同 turn_id 两会话经 scopeTurnKeys 后分属两桶（两个轮次头）
    expect(w.findAll(".trow-turn-header")).toHaveLength(2);
  });

  it("传入 sessionMeta：turn 头渲染会话徽章，delegation 走 tint 令牌", () => {
    const w = mountTable({
      sessionMeta: new Map([
        ["s1", { title: "主对话", kind: "chat" }],
        ["s2", { title: "委派任务", kind: "delegation" }],
      ]),
    });
    const badges = w.findAll(".th-session");
    expect(badges.map((b) => b.text())).toEqual(["主对话", "委派任务"]);
    expect(badges[1].classes()).toContain("th-session-delegation");
    expect(badges[0].classes()).not.toContain("th-session-delegation");
  });

  it("selectedKey 精确高亮：跨会话同 seq 只亮目标行（s1 头在前，事件序 s1→s2）", async () => {
    const events = mergedEvents();
    const rows = buildRows(scopeTurnKeys(events), { collapsedTurns: new Set(), showAux: false, query: "", turnOffset: 0 });
    // s2 的 assistant 行：seq=2 与 s1 的 assistant 行相同——只有 key 能区分
    const target = rows.find((r) => r.type === "event" && r.key === "s2::t1-assistant_message-2")!;
    expect(target).toBeTruthy();

    const w = mountTable({ selectedKey: target.key, selectedSeq: null });
    const selected = w.findAll(".trow-event.selected");
    expect(selected).toHaveLength(1);
    expect(selected[0].text()).toContain("来自 s2 的回答");

    // 回归：不传 selectedKey 时仍按 seq 匹配（单会话路径）
    const w2 = mountTable({ selectedSeq: 1, selectedKey: null });
    // seq=1 在两会话各有一行（user_message）——这正是项目流必须用 key 的原因
    expect(w2.findAll(".trow-event.selected").length).toBe(2);
  });
});
