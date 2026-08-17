// ProjectSwitcher.capsule.test.ts — 胶囊形态行为锁定（2026-08 侧边栏重排）：
// 左 chip 当前空间名 / + 钮原地展开快速新建表单（Enter 提交）/ chip 开切换菜单。
// 多根组件（capsule + overlay + menu），props/emits 契约与重排前完全一致。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import ProjectSwitcher from "../ProjectSwitcher.vue";
import type { Project } from "../../../types";

function project(id: string, name: string, theme_color: string | null): Project {
  return {
    id,
    name,
    description: "",
    icon: "folder",
    sort_order: 0,
    workspace_path: null,
    theme_color,
    archived: false,
    created_at: "2026-08-17 00:00:00",
    updated_at: "2026-08-17 00:00:00",
  };
}

const PROJECTS: Project[] = [
  project("p1", "Alpha", "#ff5500"),
  project("p2", "Beta", null),
];

function mountSwitcher(overrides?: { scopeProjectId?: string | null }) {
  // 显式 null（散落会话）是合法值，不能用 ?? 兜底（会把 null 吃成 "p1"）
  const scope =
    overrides && "scopeProjectId" in overrides ? overrides.scopeProjectId! : "p1";
  return mount(ProjectSwitcher, {
    props: {
      currentProjectName: "Alpha",
      scopeProjectId: scope,
      projects: PROJECTS,
    },
  });
}

describe("ProjectSwitcher 胶囊形态", () => {
  it("chip 渲染当前空间名与主题色圆点", () => {
    const w = mountSwitcher();
    expect(w.find(".proj-chip .switcher-name").text()).toBe("Alpha");
    const dot = w.find(".proj-chip .item-dot");
    expect(dot.attributes("style")).toContain("#ff5500");
  });

  it("散落会话态：圆点回落灰色、无主题色内联样式", () => {
    const w = mountSwitcher({ scopeProjectId: null });
    expect(w.find(".proj-chip .item-dot").classes()).toContain("muted");
    expect(w.find(".proj-chip .item-dot").attributes("style")).toBeUndefined();
  });

  it("+ 钮原地展开快速新建表单，Enter 提交 emit create 并复位", async () => {
    const w = mountSwitcher();
    await w.find('.capsule-btn[title="快速新建项目"]').trigger("click");
    expect(w.find(".switcher-create").exists()).toBe(true);
    expect(w.find(".proj-chip").exists()).toBe(false); // 整行被表单替换

    await w.find(".create-input").setValue("新项目");
    await w.find(".create-input").trigger("keydown.enter");
    expect(w.emitted("create")?.[0]).toEqual(["新项目"]);
    expect(w.find(".proj-chip").exists()).toBe(true); // 复位回胶囊
  });

  it("chip 点击开向下切换菜单，选中项 emit select 并关闭", async () => {
    const w = mountSwitcher();
    await w.find(".proj-chip").trigger("click");
    expect(w.find(".switcher-menu").exists()).toBe(true);
    expect(w.findAll(".switcher-item").length).toBe(3); // 散落 + 2 项目

    await w.findAll(".switcher-item")[2].trigger("click"); // Beta
    expect(w.emitted("select")?.[0]).toEqual(["p2"]);
    expect(w.find(".switcher-menu").exists()).toBe(false);
  });

  it("管理钮 emit manage，不牵动菜单", async () => {
    const w = mountSwitcher();
    await w.find('.capsule-btn[title="管理项目"]').trigger("click");
    expect(w.emitted("manage")).toHaveLength(1);
    expect(w.find(".switcher-menu").exists()).toBe(false);
  });
});
