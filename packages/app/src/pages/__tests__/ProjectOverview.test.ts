// ProjectOverview.test.ts — 概览 tab 二轮重设计后的行为锁定：
// 统计带数字渲染（overview + 成员数）/ 任务状态条分桶 / 最近活动 /
// 成员卡直达设置 / 空态。会话入口已撤（完整列表在侧栏，不重复造）。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import ProjectOverview from "../project/ProjectOverview.vue";
import { useProjectStore } from "../../stores/project";
import type { Project } from "../../types";

const mockInvoke = vi.mocked(invoke);
const push = vi.fn();

vi.mock("vue-router", () => ({
  useRoute: () => ({ params: { id: "p1" } }),
  useRouter: () => ({ push }),
}));

function project(): Project {
  return {
    id: "p1", name: "Alpha", description: "", icon: "folder", sort_order: 0,
    workspace_path: null, theme_color: null, archived: false,
    created_at: "2026-08-18 00:00:00", updated_at: "2026-08-18 00:00:00",
    agents: [
      { project_id: "p1", agent_id: "a1", role: "member", joined_at: "2026-08-18 00:00:00" },
      { project_id: "p1", agent_id: "a2", role: "member", joined_at: "2026-08-18 00:00:00" },
    ],
  };
}

const OVERVIEW_OUT = {
  chat_conversations: 7, delegation_conversations: 3, messages: 42,
  tasks_total: 3, tasks_done: 1, tasks_failed: 1, tasks_ended_other: 0,
  last_activity_at: "2026-08-18 01:00:00",
};

function mockBackend(tasks: unknown[] = []) {
  mockInvoke.mockImplementation((async (cmd: string) => {
    switch (cmd) {
      case "get_project_overview": return OVERVIEW_OUT;
      case "list_project_tasks": return tasks;
      default: return undefined;
    }
  }) as never);
}

async function mountOverview() {
  const ps = useProjectStore();
  ps.list = [project()];
  ps.loaded = true;
  const w = mount(ProjectOverview);
  await flushPromises();
  return w;
}

describe("ProjectOverview 概览 tab（二轮重设计）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset();
    push.mockReset();
    mockBackend();
  });

  it("统计带：四卡数字（会话/委派任务/消息/成员）来自 overview 与 store", async () => {
    const w = await mountOverview();
    const values = w.findAll(".stat-value").map((n) => n.text());
    expect(values).toEqual(["7", "3", "42", "2"]);
    const labels = w.findAll(".stat-label").map((n) => n.text());
    expect(labels).toEqual(["会话", "委派任务", "消息", "成员"]);
  });

  it("任务状态卡：桶数渲染 + 最近活动；零任务走优雅空态", async () => {
    let w = await mountOverview();
    expect(w.find(".mix-empty").exists()).toBe(true);
    // 空任务也有 meta 行（共 0 个任务 / 最近活动来自 overview）
    expect(w.find(".mix-meta").text()).toContain("共 0 个任务");
    expect(w.find(".mix-meta").text()).toContain("最近活动");

    mockBackend([
      { conv_id: "t1", termination: "stop" },
      { conv_id: "t2", termination: "error" },
    ]);
    w = await mountOverview();
    const legend = w.findAll(".mix-item").map((n) => n.text());
    expect(legend).toContain("已完成 1");
    expect(legend).toContain("未成功 1");
    expect(w.find(".mix-empty").exists()).toBe(false);
    expect(w.find(".mix-meta").text()).toContain("共 2 个任务");
  });

  it("成员卡点击直达设置 tab", async () => {
    const w = await mountOverview();
    await w.find(".stat-card-click").trigger("click");
    expect(push).toHaveBeenCalledWith("/projects/p1/settings");
  });

  it("会话入口区已撤：无会话行/查看全部按钮", async () => {
    const w = await mountOverview();
    expect(w.find(".conv-section").exists()).toBe(false);
    expect(w.find(".view-all").exists()).toBe(false);
  });
});
