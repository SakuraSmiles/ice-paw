// ProjectList.edit.test.ts — MA-2 抽共享组件后的编辑区回归锁定：
// 点卡片展开 → 三共享组件（BasicForm/MembersChips/ContextEditor）就位 →
// 改名保存走 project.update（update_project）并收起。锁的是「编排」，
// 组件内部行为见 sharedEditComponents.test.ts。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import ProjectList from "../ProjectList.vue";
import ProjectBasicForm from "../../components/project/ProjectBasicForm.vue";
import ProjectMembersChips from "../../components/project/ProjectMembersChips.vue";
import ProjectContextEditor from "../../components/project/ProjectContextEditor.vue";
import type { Project } from "../../types";

const mockInvoke = vi.mocked(invoke);

function project(id: string, name: string): Project {
  return {
    id,
    name,
    description: "",
    icon: "folder",
    sort_order: 0,
    workspace_path: null,
    theme_color: null,
    archived: false,
    created_at: "2026-08-18 00:00:00",
    updated_at: "2026-08-18 00:00:00",
    agents: [{ project_id: id, agent_id: "a1", role: "member", joined_at: "2026-08-18 00:00:00" }],
  };
}

/** onMounted 三连（projects/agents/conversations）按命令分发 mock */
function mockBackend(rows: Project[]) {
  mockInvoke.mockImplementation((async (cmd: string) => {
    switch (cmd) {
      case "list_projects":
        return rows;
      case "list_agents":
        return [];
      case "list_all_conversations":
        return [];
      case "get_project_context":
        return {
          available: true,
          dir: "C:/ws/projects/p1",
          project_md: "# P",
          conventions_md: "",
        };
      default:
        return undefined;
    }
  }) as never);
}

async function mountList() {
  const w = mount(ProjectList);
  await flushPromises();
  return w;
}

describe("ProjectList 编辑区（共享组件化后回归）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset();
  });

  it("点卡片展开 → 三共享组件就位（ContextEditor 自持加载）", async () => {
    mockBackend([project("p1", "Alpha")]);
    const w = await mountList();

    expect(w.findComponent(ProjectBasicForm).exists()).toBe(false);
    await w.find(".proj-card:not(.new-card)").trigger("click");
    await flushPromises();

    expect(w.findComponent(ProjectBasicForm).exists()).toBe(true);
    expect(w.findComponent(ProjectMembersChips).exists()).toBe(true);
    const editor = w.findComponent(ProjectContextEditor);
    expect(editor.exists()).toBe(true);
    expect(editor.props("projectId")).toBe("p1");
    // ContextEditor 自持 force 加载（原 toggleEdit 内联逻辑已搬走）
    expect(mockInvoke).toHaveBeenCalledWith("get_project_context", { projectId: "p1" });
  });

  it("改名保存走 update_project 并收起展开区", async () => {
    mockBackend([project("p1", "Alpha")]);
    const w = await mountList();
    await w.find(".proj-card:not(.new-card)").trigger("click");
    await flushPromises();

    // BasicForm 单对象 v-model：改名字段
    const nameInput = w.findComponent(ProjectBasicForm).find('input[type="text"]');
    await nameInput.setValue("新名字");

    await w.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();

    expect(mockInvoke).toHaveBeenCalledWith("update_project", {
      input: expect.objectContaining({ id: "p1", name: "新名字" }),
    });
    expect(w.findComponent(ProjectBasicForm).exists()).toBe(false); // 收起
  });

  it("成员 chips emit → add_project_agent + 列表刷新", async () => {
    mockBackend([project("p1", "Alpha")]);
    const w = await mountList();
    await w.find(".proj-card:not(.new-card)").trigger("click");
    await flushPromises();

    w.findComponent(ProjectMembersChips).vm.$emit("add", "a2");
    await flushPromises();

    expect(mockInvoke).toHaveBeenCalledWith("add_project_agent", {
      projectId: "p1",
      agentId: "a2",
      role: "member",
    });
    // addMember 内 project.load(true) 刷新（二次 list_projects）
    const listCalls = mockInvoke.mock.calls.filter(([c]) => c === "list_projects");
    expect(listCalls.length).toBeGreaterThanOrEqual(2);
  });

  it("空名保存拦截（不发 update_project）", async () => {
    mockBackend([project("p1", "Alpha")]);
    const w = await mountList();
    await w.find(".proj-card:not(.new-card)").trigger("click");
    await flushPromises();

    await w.findComponent(ProjectBasicForm).find('input[type="text"]').setValue("   ");
    await w.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();

    expect(mockInvoke.mock.calls.some(([c]) => c === "update_project")).toBe(false);
    expect(w.find(".form-error").text()).toContain("项目名称不能为空");
  });
});
