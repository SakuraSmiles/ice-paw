// ProjectSettings.test.ts — 设置 tab 编排锁定：表单初值来自 store / 脏检查
// 驱动保存·取消 / 空名拦截 / 归档确认流（archive_project → push /projects）。
// 组件内部行为（目录选择/成员 chips/上下文编辑器）见 sharedEditComponents.test.ts。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import ProjectSettings from "../project/ProjectSettings.vue";
import ProjectBasicForm from "../../components/project/ProjectBasicForm.vue";
import ProjectMembersChips from "../../components/project/ProjectMembersChips.vue";
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
    id: "p1",
    name: "Alpha",
    description: "描述",
    icon: "folder",
    sort_order: 0,
    workspace_path: "D:/ws/alpha",
    theme_color: null,
    archived: false,
    created_at: "2026-08-18 00:00:00",
    updated_at: "2026-08-18 00:00:00",
    agents: [{ project_id: "p1", agent_id: "a1", role: "member", joined_at: "2026-08-18 00:00:00" }],
  };
}

/** 设置页触达的后端命令按需分发（store 未预载时 ProjectList 同款兜底语义） */
function mockBackend() {
  mockInvoke.mockImplementation((async (cmd: string) => {
    switch (cmd) {
      case "list_projects":
        return [project()];
      case "list_all_conversations":
        return [];
      case "get_project_context":
        return { available: true, dir: "C:/ws/projects/p1", project_md: "# P", conventions_md: "" };
      default:
        return undefined;
    }
  }) as never);
}

/** 预载 store（详情页常规路径：DetailLayout 已 project.load；测试直接喂缓存） */
async function mountSettings() {
  const ps = useProjectStore();
  ps.list = [project()];
  ps.loaded = true;
  const w = mount(ProjectSettings);
  await flushPromises();
  return { w, ps };
}

describe("ProjectSettings 设置 tab", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset();
    push.mockReset();
    mockBackend();
  });

  it("表单初值来自 store；未脏时保存/取消禁用", async () => {
    const { w } = await mountSettings();
    const nameInput = w.findComponent(ProjectBasicForm).find('input[type="text"]');
    expect((nameInput.element as HTMLInputElement).value).toBe("Alpha");

    const save = w.findAll("button").find((b) => b.text() === "保存")!;
    expect(save.attributes("disabled")).toBeDefined();
  });

  it("改脏 → 保存走 update_project；取消重置回初值", async () => {
    const { w } = await mountSettings();
    const form = w.findComponent(ProjectBasicForm);
    await form.find('input[type="text"]').setValue("Beta");

    await w.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(mockInvoke).toHaveBeenCalledWith("update_project", {
      input: expect.objectContaining({ id: "p1", name: "Beta", workspace_path: "D:/ws/alpha" }),
    });

    // 取消路径：再改 + 取消 → 回初值且保存重新禁用
    await form.find('input[type="text"]').setValue("Gamma");
    await w.findAll("button").find((b) => b.text() === "取消")!.trigger("click");
    expect((form.find('input[type="text"]').element as HTMLInputElement).value).toBe("Alpha");
    expect(w.findAll("button").find((b) => b.text() === "保存")!.attributes("disabled")).toBeDefined();
  });

  it("空名保存拦截（不发 update_project）", async () => {
    const { w } = await mountSettings();
    await w.findComponent(ProjectBasicForm).find('input[type="text"]').setValue("   ");
    await w.findAll("button").find((b) => b.text() === "保存")!.trigger("click");
    await flushPromises();
    expect(mockInvoke.mock.calls.some(([c]) => c === "update_project")).toBe(false);
    expect(w.find(".form-error").text()).toContain("项目名称不能为空");
  });

  it("成员 chips emit → add_project_agent + 刷新", async () => {
    const { w } = await mountSettings();
    w.findComponent(ProjectMembersChips).vm.$emit("add", "a2");
    await flushPromises();
    expect(mockInvoke).toHaveBeenCalledWith("add_project_agent", { projectId: "p1", agentId: "a2", role: "member" });
  });

  it("归档：确认弹窗 → archive_project + 回项目列表", async () => {
    const { w } = await mountSettings();
    await w.findAll("button").find((b) => b.text() === "归档项目")!.trigger("click");
    expect(w.find(".perm-panel").exists()).toBe(true);

    await w.findAll("button").find((b) => b.text() === "确认归档")!.trigger("click");
    await flushPromises();
    expect(mockInvoke).toHaveBeenCalledWith("archive_project", { id: "p1" });
    expect(push).toHaveBeenCalledWith("/projects");
  });
});
