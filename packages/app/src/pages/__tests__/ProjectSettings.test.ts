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

/** 设置页触达的后端命令按需分发（store 未预载时 ProjectList 同款兜底语义）。
 *  行可变：set_project_agents 后回读反映新名单（成员保存的 read-after-write） */
function mockBackend() {
  const row = project();
  mockInvoke.mockImplementation((async (cmd: string, args: Record<string, unknown>) => {
    switch (cmd) {
      case "list_projects":
        return [row];
      case "set_project_agents": {
        const members = (args?.members ?? []) as [string, string][];
        row.agents = members.map(([id, role]) => ({
          project_id: row.id, agent_id: id, role, joined_at: "2026-08-18 00:00:00",
        }));
        return undefined;
      }
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
    expect(w.find(".eb-inline").text()).toContain("项目名称不能为空"); // W2：表单错误行收编 ErrorBanner（inline 形态）
  });

  it("成员 chips 只改草稿，随「保存」全量提交并收起操作行（显式契约）", async () => {
    const { w } = await mountSettings();
    // 页面上「可用（非禁用）的保存」——基础信息未脏时唯一，成员卡出现操作行后 +1
    const enabledSaves = () =>
      w.findAll("button").filter((b) => b.text() === "保存" && b.attributes("disabled") === undefined);
    expect(enabledSaves()).toHaveLength(0); // 未脏：成员卡无操作行

    // v-model 草稿更新（a1 → a1+a2）——此刻零持久化调用
    w.findComponent(ProjectMembersChips).vm.$emit("update:memberIds", ["a1", "a2"]);
    await flushPromises();
    expect(mockInvoke.mock.calls.some(([c]) => c === "set_project_agents")).toBe(false);
    expect(enabledSaves()).toHaveLength(1); // 成员卡操作行出现

    await enabledSaves()[0].trigger("click");
    await flushPromises();
    expect(mockInvoke).toHaveBeenCalledWith("set_project_agents", {
      projectId: "p1",
      members: [["a1", "member"], ["a2", "member"]],
    });
    // 保存后权威刷新（list_projects 回读新名单）+ 会话缓存刷新，操作行收起
    expect(mockInvoke.mock.calls.some(([c]) => c === "list_projects")).toBe(true);
    expect(mockInvoke.mock.calls.some(([c]) => c === "list_all_conversations")).toBe(true);
    expect(enabledSaves()).toHaveLength(0);
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
