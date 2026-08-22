// ProjectSwitcher.capsule.test.ts — 胶囊形态行为锁定（2026-08 头行改版）：
// 左名称区纯展示（scoped 点击 emit open 直达详情页，散落态 disabled）/
// ⇄ 钮开切换菜单 / + 钮原地展开快速新建表单（Enter 提交）。
// 多根组件（capsule + overlay + menu），select/create/manage/open 契约上交 Sidebar。
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
  it("名称区渲染当前空间名与主色圆点（主题色功能已移除，无内联色）", () => {
    const w = mountSwitcher();
    expect(w.find(".proj-name .switcher-name").text()).toBe("Alpha");
    // 圆点接线：scoped 态 = 主色（非 muted、无内联色——theme_color 在数据中也被忽略）
    const dot = w.find(".proj-name .item-dot");
    expect(dot.exists()).toBe(true);
    expect(dot.classes()).not.toContain("muted");
    expect(dot.attributes("style")).toBeUndefined();
  });

  it("散落会话态：圆点转灰（muted）、名称区 disabled 置灰不可点", async () => {
    const w = mountSwitcher({ scopeProjectId: null });
    expect(w.find(".proj-name .item-dot").classes()).toContain("muted");
    expect(w.find(".proj-name .item-dot").attributes("style")).toBeUndefined();
    const nameBtn = w.find(".proj-name");
    expect(nameBtn.attributes("disabled")).toBeDefined();
    // disabled 按钮点击不 emit open（onOpenDetail 还有 scope 兜底双保险）
    await nameBtn.trigger("click");
    expect(w.emitted("open")).toBeUndefined();
  });

  it("scoped 名称区点击 emit open（项目 id），且不开菜单", async () => {
    const w = mountSwitcher();
    await w.find(".proj-name").trigger("click");
    expect(w.emitted("open")?.[0]).toEqual(["p1"]);
    expect(w.find(".switcher-menu").classes()).not.toContain("open");
  });

  it("+ 钮原地展开快速新建表单，Enter 提交 emit create 并复位", async () => {
    const w = mountSwitcher();
    await w.find('.capsule-btn[title="快速新建项目"]').trigger("click");
    expect(w.find(".switcher-create").exists()).toBe(true);
    expect(w.find(".proj-name").exists()).toBe(false); // 整行被表单替换

    await w.find(".create-input").setValue("新项目");
    await w.find(".create-input").trigger("keydown.enter");
    expect(w.emitted("create")?.[0]).toEqual(["新项目"]);
    expect(w.find(".proj-name").exists()).toBe(true); // 复位回胶囊
  });

  it("⇄ 钮开向下切换菜单，选中项 emit select 并关闭", async () => {
    const w = mountSwitcher();
    await w.find('.capsule-btn[title="切换项目空间"]').trigger("click");
    // 菜单常驻 DOM（class 驱动开合），断言 open 类
    expect(w.find(".switcher-menu").classes()).toContain("open");
    expect(w.find(".switcher-overlay").classes()).toContain("open");
    expect(w.findAll(".switcher-item").length).toBe(3); // 散落 + 2 项目
    // 菜单行圆点接线：项目行主色圆点、散落行 muted 灰点（均无内联色）
    expect(w.findAll(".switcher-item .item-dot").length).toBe(3); // 散落 + 2 项目
    expect(w.find(".switcher-item .item-dot.muted").exists()).toBe(true);
    expect(w.findAll(".switcher-item .item-dot")[1].attributes("style")).toBeUndefined();

    await w.findAll(".switcher-item")[2].trigger("click"); // Beta
    expect(w.emitted("select")?.[0]).toEqual(["p2"]);
    expect(w.find(".switcher-menu").classes()).not.toContain("open");
    expect(w.find(".switcher-overlay").classes()).not.toContain("open");
    // 展开态高亮随菜单关闭撤下
    expect(w.find('.capsule-btn[title="切换项目空间"]').classes()).not.toContain("switcher-open");
  });

  it("管理钮 emit manage，不牵动菜单", async () => {
    const w = mountSwitcher();
    await w.find('.capsule-btn[title="管理项目"]').trigger("click");
    expect(w.emitted("manage")).toHaveLength(1);
    expect(w.find(".switcher-menu").classes()).not.toContain("open");
  });
});
