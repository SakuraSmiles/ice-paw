// ProjectOverview.test.ts — 概览 tab 重设计后的行为锁定：
// 统计带数字渲染（overview + 成员数）/ 任务状态条分桶 / 会话入口
// （项目过滤 + 最近 5 条 + 点击回首页 scope 同步）/ 空态 / 成员卡直达设置。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import ProjectOverview from "../project/ProjectOverview.vue";
import { useChatStore } from "../../stores/chat";
import { useProjectStore } from "../../stores/project";
import type { Conversation, Project } from "../../types";

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

function conv(id: string, agentId: string, updatedAt: string, projectId: string | null = "p1"): Conversation {
  return {
    id, agent_id: agentId, title: `会话 ${id}`, pinned: false,
    created_at: updatedAt, updated_at: updatedAt,
    project_id: projectId, kind: projectId ? "chat" : undefined,
  } as Conversation;
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
      case "list_agents": return [
        { id: "a1", name: "前端专家", model: "m1" },
        { id: "a2", name: "测试专家", model: "m2" },
      ];
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

describe("ProjectOverview 概览 tab（重设计）", () => {
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
    expect(w.find(".section-meta").text()).toContain("最近活动");
  });

  it("任务状态条：桶数渲染；零任务走优雅空态", async () => {
    let w = await mountOverview();
    expect(w.find(".mix-empty").exists()).toBe(true);

    mockBackend([
      { conv_id: "t1", termination: "stop" },
      { conv_id: "t2", termination: "error" },
    ]);
    w = await mountOverview();
    const legend = w.findAll(".mix-item").map((n) => n.text());
    expect(legend).toContain("已完成 1");
    expect(legend).toContain("未成功 1");
    expect(w.find(".mix-empty").exists()).toBe(false);
  });

  it("会话入口：项目过滤（他项目/后台会话不进）+ 最近 5 条按 updated_at 倒序", async () => {
    const chat = useChatStore();
    chat.conversations = [
      conv("c1", "a1", "2026-08-18 00:10:00"),
      conv("c2", "a1", "2026-08-18 00:30:00"),
      conv("other", "a1", "2026-08-18 00:59:00", "p2"), // 他项目
      ...Array.from({ length: 6 }, (_, i) =>
        conv(`c${10 + i}`, "a2", `2026-08-17 23:${String(i).padStart(2, "0")}:00`)),
    ];
    const w = await mountOverview();

    const rows = w.findAll(".conv-row");
    expect(rows.length).toBe(5);
    expect(rows[0].text()).toContain("会话 c2"); // 最新在前
    expect(rows.map((r) => r.text()).some((t) => t.includes("会话 other"))).toBe(false);
  });

  it("点会话行：scope 切本项目 + 选中该会话 + 回首页", async () => {
    const chat = useChatStore();
    const ps = useProjectStore();
    ps.setActiveProject(null);
    chat.conversations = [conv("c1", "a1", "2026-08-18 00:10:00")];
    const w = await mountOverview();

    await w.findAll(".conv-row")[0].trigger("click");
    expect(ps.activeProjectId).toBe("p1");
    expect(chat.activeConvId).toBe("c1");
    expect(push).toHaveBeenCalledWith("/");
  });

  it("「在侧栏查看全部」：scope 切本项目 + 选中最近一条 + 回首页", async () => {
    const chat = useChatStore();
    chat.conversations = [conv("c1", "a1", "2026-08-18 00:10:00")];
    const w = await mountOverview();

    await w.find(".view-all").trigger("click");
    expect(useProjectStore().activeProjectId).toBe("p1");
    expect(chat.activeConvId).toBe("c1");
    expect(push).toHaveBeenCalledWith("/");
  });

  it("成员卡点击直达设置 tab", async () => {
    const w = await mountOverview();
    await w.find(".stat-card-click").trigger("click");
    expect(push).toHaveBeenCalledWith("/projects/p1/settings");
  });

  it("空项目：会话空态引导，无报错", async () => {
    const w = await mountOverview();
    expect(w.find(".conv-empty").exists()).toBe(true);
    expect(w.find(".conv-empty-hint").exists()).toBe(true);
  });
});
