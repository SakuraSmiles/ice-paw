// ChatWelcome.ctx-pill.test.ts — 项目背景注入状态条（L2 状态上屏）行为锁定：
// 项目空间 + 有内容 → 「已注入项目说明 · N 字」；空内容 → 「未填写 · 去填写」；
// 散落空间 / available=false / 加载失败 → 整条隐藏（零噪声）。
// 点击跳 /projects（与 useNewConversation「去添加成员」同目标）。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises } from "@vue/test-utils";
import ChatWelcome from "../ChatWelcome.vue";
import { useProjectStore } from "../../../stores/project";
import { invoke } from "@tauri-apps/api/core";
import type { Project } from "../../../types";

const mockInvoke = vi.mocked(invoke);
const push = vi.fn();

vi.mock("vue-router", () => ({
  useRouter: () => ({ push, currentRoute: { value: { name: "Home" } } }),
}));

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
    agents: [],
  };
}

function ctxOut(project_md: string, available = true) {
  return { available, dir: available ? "C:/ws/projects/p1" : null, project_md, conventions_md: "" };
}

async function mountWelcome(pid: string | null) {
  const ps = useProjectStore();
  ps.list = [project("p1", "Alpha")];
  ps.loaded = true;
  ps.setActiveProject(pid);
  const w = mount(ChatWelcome);
  await flushPromises();
  return w;
}

describe("ChatWelcome 项目背景状态条", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset();
    push.mockReset();
  });

  it("项目空间 + 有内容 → 显示已注入与字数，点击跳项目页", async () => {
    mockInvoke.mockResolvedValue(ctxOut("# Alpha\n技术栈：Tauri v2") as never);
    const w = await mountWelcome("p1");

    const pill = w.find(".ctx-pill");
    expect(pill.exists()).toBe(true);
    expect(pill.text()).toContain("已注入项目说明");
    expect(pill.text()).toContain("20 字"); // "# Alpha\n技术栈：Tauri v2".trim().length

    await pill.trigger("click");
    expect(push).toHaveBeenCalledWith("/projects");
  });

  it("项目空间 + 空内容 → 「未填写」+ 去填写", async () => {
    mockInvoke.mockResolvedValue(ctxOut("   ") as never);
    const w = await mountWelcome("p1");

    const pill = w.find(".ctx-pill");
    expect(pill.exists()).toBe(true);
    expect(pill.text()).toContain("项目说明未填写");
    expect(pill.text()).toContain("去填写");
  });

  it("散落空间 → 不渲染状态条", async () => {
    const w = await mountWelcome(null);
    expect(w.find(".ctx-pill").exists()).toBe(false);
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("available=false / 加载失败 → 静默隐藏", async () => {
    mockInvoke.mockResolvedValue(ctxOut("", false) as never);
    let w = await mountWelcome("p1");
    expect(w.find(".ctx-pill").exists()).toBe(false);

    mockInvoke.mockRejectedValue(new Error("boom") as never);
    setActivePinia(createPinia());
    w = await mountWelcome("p1");
    expect(w.find(".ctx-pill").exists()).toBe(false);
  });

  it("切换项目后状态条随 activeProjectId 重算（缓存命中不重复 invoke）", async () => {
    mockInvoke.mockResolvedValue(ctxOut("# 有内容") as never);
    const w = await mountWelcome("p1");
    expect(w.find(".ctx-pill").text()).toContain("已注入");

    const ps = useProjectStore();
    ps.setActiveProject(null); // 切散落
    await flushPromises();
    expect(w.find(".ctx-pill").exists()).toBe(false);

    ps.setActiveProject("p1"); // 切回 → 缓存命中，无新 invoke
    await flushPromises();
    expect(w.find(".ctx-pill").exists()).toBe(true);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
  });
});
