// ProjectSwitcher.rail.test.ts — 收起态（rail）行为锁定（2026-09-01 侧栏收起）：
// collapsed 变体只渲染单图标钮（Folder），切换菜单改向右弹（.switcher-menu.collapsed
// 定位变体类）；开合/选中自关/遮罩点外关全部复用胶囊形态那套 isOpen 状态机与
// 列表 markup。菜单头部收进「快速新建 / 项目列表」两入口（用户拍板 2026-09-01
// 二轮补）——新建表单在菜单内原地展开（与胶囊表单同款 markup 二选一渲染）。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import ProjectSwitcher from "../ProjectSwitcher.vue";
import type { Project } from "../../../types";

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
    created_at: "2026-08-17 00:00:00",
    updated_at: "2026-08-17 00:00:00",
  };
}

const PROJECTS: Project[] = [project("p1", "Alpha"), project("p2", "Beta")];

function mountRail() {
  return mount(ProjectSwitcher, {
    props: {
      currentProjectName: "Alpha",
      scopeProjectId: "p1",
      projects: PROJECTS,
      collapsed: true,
    },
  });
}

describe("ProjectSwitcher 收起态（rail）", () => {
  it("collapsed 只渲染单图标钮：无名称区/无胶囊动作组，title 带当前空间名", () => {
    const w = mountRail();
    expect(w.find(".switcher-rail-btn").exists()).toBe(true);
    expect(w.find(".proj-name").exists()).toBe(false);
    expect(w.find(".capsule-actions").exists()).toBe(false);
    // 根与菜单各挂 collapsed 变体类（CSS 定位变体的开关）
    expect(w.find(".project-capsule").classes()).toContain("collapsed");
    expect(w.find(".switcher-menu").classes()).toContain("collapsed");
    expect(w.find(".switcher-rail-btn").attributes("title")).toContain("Alpha");
  });

  it("图标钮开菜单：列表原班（散落 + 2 项目），选中 emit select 并自关", async () => {
    const w = mountRail();
    await w.find(".switcher-rail-btn").trigger("click");
    const menu = w.find(".switcher-menu");
    expect(menu.classes()).toContain("open");
    expect(w.find(".switcher-overlay").classes()).toContain("open");
    expect(w.findAll(".switcher-item").length).toBe(3);

    await w.findAll(".switcher-item")[2].trigger("click"); // Beta
    expect(w.emitted("select")?.[0]).toEqual(["p2"]);
    expect(menu.classes()).not.toContain("open");
    expect(w.find(".switcher-overlay").classes()).not.toContain("open");
    // 开关态高亮随菜单关闭撤下（与 ⇄ 钮同语义）
    expect(w.find(".switcher-rail-btn").classes()).not.toContain("switcher-open");
  });

  it("遮罩点击关闭菜单（点外关）", async () => {
    const w = mountRail();
    await w.find(".switcher-rail-btn").trigger("click");
    expect(w.find(".switcher-menu").classes()).toContain("open");
    await w.find(".switcher-overlay").trigger("click");
    expect(w.find(".switcher-menu").classes()).not.toContain("open");
  });

  it("菜单头部两入口在位：快速新建（FolderPlus）+ 项目列表（List）", async () => {
    const w = mountRail();
    await w.find(".switcher-rail-btn").trigger("click");
    expect(w.find(".switcher-menu-header").exists()).toBe(true);
    expect(w.find('.menu-action-btn[title="快速新建项目"]').exists()).toBe(true);
    expect(w.find('.menu-action-btn[title="项目列表"]').exists()).toBe(true);
  });

  it("快速新建：菜单头部入口原地展开表单，Enter emit create 并收场关菜单", async () => {
    const w = mountRail();
    await w.find(".switcher-rail-btn").trigger("click");
    await w.find('.menu-action-btn[title="快速新建项目"]').trigger("click");
    // 菜单保持打开（表单在头部下方展开，列表照常可见）
    expect(w.find(".switcher-menu").classes()).toContain("open");
    expect(w.find(".menu-create-row").exists()).toBe(true);
    // 根级胶囊表单不渲染（collapsed 时表单只活在菜单内）
    expect(w.find(".switcher-create:not(.menu-create-row)").exists()).toBe(false);

    await w.find(".menu-create-row .create-input").setValue("新项目");
    await w.find(".menu-create-row .create-input").trigger("keydown.enter");
    expect(w.emitted("create")?.[0]).toEqual(["新项目"]);
    // 确认即收场：表单复位 + 菜单关（创建方会切空间回首页，菜单留着遮视野）
    expect(w.find(".menu-create-row").exists()).toBe(false);
    expect(w.find(".switcher-menu").classes()).not.toContain("open");
  });

  it("菜单内表单开着时点列表项 = 放弃输入：选中生效且表单复位（下次开无残留）", async () => {
    const w = mountRail();
    await w.find(".switcher-rail-btn").trigger("click");
    await w.find('.menu-action-btn[title="快速新建项目"]').trigger("click");
    await w.find(".menu-create-row .create-input").setValue("半截输入");

    await w.findAll(".switcher-item")[2].trigger("click"); // Beta
    expect(w.emitted("select")?.[0]).toEqual(["p2"]);
    expect(w.emitted("create")).toBeUndefined();
    expect(w.find(".switcher-menu").classes()).not.toContain("open");

    // 重开菜单：表单不复残留
    await w.find(".switcher-rail-btn").trigger("click");
    expect(w.find(".menu-create-row").exists()).toBe(false);
  });

  it("项目列表入口：emit manage 并关菜单", async () => {
    const w = mountRail();
    await w.find(".switcher-rail-btn").trigger("click");
    await w.find('.menu-action-btn[title="项目列表"]').trigger("click");
    expect(w.emitted("manage")).toHaveLength(1);
    expect(w.find(".switcher-menu").classes()).not.toContain("open");
  });
});
