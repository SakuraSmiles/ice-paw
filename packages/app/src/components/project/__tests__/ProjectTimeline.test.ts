// ProjectTimeline.test.ts — 项目轨迹跨会话合并集成：两会话同 turn_id 不合桶、
// 会话徽章（tint 区分委派）、事件行点击 → 检查器按需展现、跨会话同 seq 选中
// 不串行。重检查器（MarkdownRenderer/ImagePreview 链）stub 隔离，只验编排。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import ProjectTimeline from "../ProjectTimeline.vue";
import type { ProjectEvent } from "../../../types";

const mockInvoke = vi.mocked(invoke);

/** 两会话（chat + delegation）各一轮：turn_id 同名、seq 编号重叠；块内连续
 *  （若无 scopeTurnKeys 前缀，同 turn_id 的连续段会错误合桶成一个头——本组
 *  fixture 即该回归的最小复现；事件级交错并发的「一头多段」是 v1 已注明的边缘） */
function mergedEvents(): ProjectEvent[] {
  const mk = (id: number, session: string, seq: number, kind: "user_message" | "assistant_message" | "turn_ended"): ProjectEvent =>
    ({
      id,
      session_id: session,
      seq,
      kind,
      actor: "user",
      turn_id: "t1",
      message_id: null,
      payload:
        kind === "user_message"
          ? { content: `${session === "s1" ? "主对话" : "委派"}的提问`, blocks: [] }
          : kind === "assistant_message"
            ? { content: `${session === "s1" ? "主对话" : "委派"}的回答`, blocks: [], round: 0, continuation: false }
            : { termination: "stop", rounds: 1, usage: null },
      created_at: `2026-08-18T10:00:${String(seq).padStart(2, "0")}Z`,
      session_title: session === "s1" ? "主对话" : "委派任务",
      session_kind: session === "s1" ? "chat" : "delegation",
    }) as ProjectEvent;
  return [
    mk(1, "s1", 1, "user_message"),
    mk(2, "s1", 2, "assistant_message"),
    mk(3, "s1", 3, "turn_ended"),
    mk(4, "s2", 1, "user_message"), // 同 turn_id、同 seq 编号——只差会话
    mk(5, "s2", 2, "assistant_message"),
    mk(6, "s2", 3, "turn_ended"),
  ];
}

function mountTimeline() {
  return mount(ProjectTimeline, {
    props: { projectId: "p1" },
    global: {
      stubs: { TrajectoryInspector: true, PanelResizeHandle: true },
    },
  });
}

describe("ProjectTimeline 跨会话合并", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset().mockImplementation((cmd: string) => {
      if (cmd === "list_project_events") return Promise.resolve(mergedEvents());
      return Promise.resolve([]);
    });
  });

  it("两会话同 turn_id 不合桶：两个轮次头 + 各自会话徽章", async () => {
    const w = mountTimeline();
    await flushPromises();

    const headers = w.findAll(".trow-turn-header");
    expect(headers).toHaveLength(2);
    const badges = w.findAll(".th-session");
    expect(badges.map((b) => b.text())).toEqual(["主对话", "委派任务"]);
    expect(badges[1].classes()).toContain("th-session-delegation");
    // 头统计不串：两会话各 1 条回复（若错误合桶会算成 2 条在一个头里）
    expect(headers[0].text()).toContain("1 条回复");
    expect(headers[1].text()).toContain("1 条回复");
  });

  it("事件行点击 → 检查器按需展现；跨会话同 seq 只选中目标行", async () => {
    const w = mountTimeline();
    await flushPromises();
    expect(w.find("trajectory-inspector-stub").exists()).toBe(false);

    // 点第 4 个事件行（s2 的 assistant，seq=2 与 s1 同号）
    const rows = w.findAll(".trow-event");
    expect(rows.length).toBeGreaterThanOrEqual(4);
    await rows[3].trigger("click");

    expect(w.find("trajectory-inspector-stub").exists()).toBe(true);
    const selected = w.findAll(".trow-event.selected");
    expect(selected).toHaveLength(1); // selectedKey 精确匹配（seq 会撞行）
    expect(selected[0].text()).toContain("委派的回答");
  });

  it("再点同一行 = 取消选中收起检查器", async () => {
    const w = mountTimeline();
    await flushPromises();
    const rows = w.findAll(".trow-event");
    await rows[0].trigger("click");
    expect(w.find("trajectory-inspector-stub").exists()).toBe(true);
    await w.findAll(".trow-event")[0].trigger("click");
    expect(w.find("trajectory-inspector-stub").exists()).toBe(false);
  });

  it("空项目：引导文案（无事件可回放）", async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "list_project_events" ? [] : []),
    );
    const w = mountTimeline();
    await flushPromises();
    expect(w.find(".pt-empty").text()).toContain("还没有事件");
  });
});
