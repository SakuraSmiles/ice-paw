// sharedEditComponents.test.ts — MA-2 抽出的三个共享编辑组件行为锁定：
// ProjectBasicForm（单对象 v-model + 目录选择内聚）/ ProjectMembersChips
// （chips add/remove 上交）/ ProjectContextEditor（自持加载·脏检查·分文件保存）。
// 双入口（ProjectList 展开区 + 项目详情页设置 tab）共用，本文件锁组件契约。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import ProjectBasicForm from "../ProjectBasicForm.vue";
import ProjectMembersChips from "../ProjectMembersChips.vue";
import ProjectContextEditor from "../ProjectContextEditor.vue";
import { useAgentStore } from "../../../stores/agent";
import type { Agent } from "../../../types";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const mockInvoke = vi.mocked(invoke);
const mockOpen = vi.mocked(openDialog);

function agent(id: string, name: string): Agent {
  return {
    id,
    name,
    provider: "anthropic",
    model: "claude-test",
    system_prompt: "",
    base_url: null,
    temperature: 0.7,
    max_tokens: 1024,
    extra_params: {},
    sort_order: 0,
    cache_prompt: false,
    has_api_key: true,
    created_at: "2026-08-18 00:00:00",
    updated_at: "2026-08-18 00:00:00",
  };
}

function ctxOut(project_md = "", available = true) {
  return {
    available,
    dir: available ? "C:/ws/projects/p1" : null,
    project_md,
    conventions_md: "",
  };
}

beforeEach(() => {
  setActivePinia(createPinia());
  mockInvoke.mockReset().mockResolvedValue(undefined as never);
  mockOpen.mockReset();
});

describe("ProjectBasicForm", () => {
  const base = { name: "Alpha", description: "说明", workspacePath: "", avatar: null, themeColor: null };

  it("字段初值渲染 + 单对象 v-model 整体替换（未改字段保留）", async () => {
    const w = mount(ProjectBasicForm, { props: { modelValue: { ...base } } });
    expect((w.find('input[type="text"]').element as HTMLInputElement).value).toBe("Alpha");

    await w.findAll('input[type="text"]')[0].setValue("Beta");
    const emitted = w.emitted("update:modelValue");
    expect(emitted?.[0]).toEqual([{ name: "Beta", description: "说明", workspacePath: "", avatar: null, themeColor: null }]);
  });

  it("目录选择内聚：ws-btn / 只读输入框点击都开 directory 对话框并回填", async () => {
    mockOpen.mockResolvedValue("D:/code/ice-paw" as never);
    const w = mount(ProjectBasicForm, { props: { modelValue: { ...base } } });

    await w.find(".ws-btn").trigger("click");
    expect(mockOpen).toHaveBeenCalledWith(
      expect.objectContaining({ directory: true, multiple: false }),
    );
    const emitted = w.emitted("update:modelValue");
    expect(emitted?.[0]).toEqual([
      { name: "Alpha", description: "说明", workspacePath: "D:/code/ice-paw", avatar: null, themeColor: null },
    ]);

    // 只读输入框点击同路（readonly 仍触发 click）
    mockOpen.mockResolvedValue("E:/other" as never);
    await w.find(".workspace-input").trigger("click");
    expect(w.emitted("update:modelValue")?.[1]).toEqual([
      { name: "Alpha", description: "说明", workspacePath: "E:/other", avatar: null, themeColor: null },
    ]);
  });

  it("对话框取消（null）不 emit", async () => {
    mockOpen.mockResolvedValue(null as never);
    const w = mount(ProjectBasicForm, { props: { modelValue: { ...base } } });
    await w.find(".ws-btn").trigger("click");
    expect(w.emitted("update:modelValue")).toBeUndefined();
  });
});

describe("ProjectMembersChips", () => {
  function mountChips(memberIds: string[]) {
    const store = useAgentStore();
    store.list = [agent("a1", "甲"), agent("a2", "乙")];
    return mount(ProjectMembersChips, { props: { memberIds } });
  }

  it("已选 × 移除 / 候选 + 添加，emit 上交（不直接持久化）", async () => {
    const w = mountChips(["a1"]);
    const chips = w.findAll(".member-chip");
    expect(chips).toHaveLength(2); // 1 已选 + 1 候选
    expect(chips[0].text()).toContain("甲");
    expect(chips[0].classes()).toContain("selected");
    expect(chips[1].text()).toContain("乙");

    await chips[0].trigger("click");
    expect(w.emitted("remove")?.[0]).toEqual(["a1"]);
    await chips[1].trigger("click");
    expect(w.emitted("add")?.[0]).toEqual(["a2"]);
  });

  it("成员与候选都空 → 暂无可用智能体引导", () => {
    const store = useAgentStore();
    store.list = [];
    const w = mount(ProjectMembersChips, { props: { memberIds: [] } });
    expect(w.find(".members-empty").text()).toBe("暂无可用智能体");
    expect(w.findAll(".member-chip")).toHaveLength(0);
  });
});

describe("ProjectContextEditor", () => {
  it("挂载即 force 加载并渲染；干净态保存禁用 + 已同步提示", async () => {
    mockInvoke.mockResolvedValue(ctxOut("# 项目说明") as never);
    const w = mount(ProjectContextEditor, { props: { projectId: "p1" } });
    await flushPromises();

    // force 加载（绕过 store 缓存——防外部编辑器改后陈旧）
    expect(mockInvoke).toHaveBeenCalledWith("get_project_context", { projectId: "p1" });
    expect((w.find(".ctx-md").element as HTMLTextAreaElement).value).toBe("# 项目说明");
    expect(w.find(".ctx-dir-btn").exists()).toBe(true); // 有 dir 才有「打开目录」

    const saveBtn = w.findAll("button").find((b) => b.text().includes("保存项目背景"))!;
    expect(saveBtn.attributes("disabled")).toBeDefined();
    expect(w.find(".ctx-saved").text()).toBe("已与文件同步");
  });

  it("脏检查 → 保存只写变更文件 → 回到已同步", async () => {
    mockInvoke.mockResolvedValue(ctxOut("# 旧内容") as never);
    const w = mount(ProjectContextEditor, { props: { projectId: "p1" } });
    await flushPromises();

    await w.find(".ctx-md").setValue("# 新内容");
    const saveBtn = w.findAll("button").find((b) => b.text().includes("保存项目背景"))!;
    expect(saveBtn.attributes("disabled")).toBeUndefined();

    await saveBtn.trigger("click");
    await flushPromises();
    expect(mockInvoke).toHaveBeenCalledWith("set_project_context", {
      projectId: "p1",
      file: "project.md",
      content: "# 新内容",
    });
    expect(w.find(".ctx-saved").exists()).toBe(true);
  });

  it("available=false → 不可用引导替代编辑区", async () => {
    mockInvoke.mockResolvedValue(ctxOut("", false) as never);
    const w = mount(ProjectContextEditor, { props: { projectId: "p1" } });
    await flushPromises();
    expect(w.find(".ctx-guide").text()).toContain("未解析到默认工作区");
    expect(w.find(".ctx-md").exists()).toBe(false);
  });

  it("加载失败 → 错误文案不吞", async () => {
    mockInvoke.mockRejectedValue(new Error("boom") as never);
    const w = mount(ProjectContextEditor, { props: { projectId: "p1" } });
    await flushPromises();
    expect(w.find(".form-error").text()).toContain("boom");
  });
});
