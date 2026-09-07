// ProjectDetailLayout.test.ts — 项目详情页死态可恢复性回归（真机实案 2026-09-07）：
// AppLayout 的 keep-alive 以组件类型缓存，缓存实例复活时 onMounted 不再跑——
// 项目被永久删除后重进详情页曾永久停在「加载中…」+ 内层「项目不存在或已删除。」。
// 锁三条兜底路径：① 直链/首挂死项目 → loadError 可恢复态；② 挂载后项目被删
// （watch current）；③ 缓存复活（onActivated，真实 KeepAlive 容器复刻）；
// ④ 同实例跨项目导航到死项目（watch projectId，keep-alive 类型键复用不重挂载）。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { h, KeepAlive, reactive, ref, nextTick } from "vue";
import { createPinia, setActivePinia } from "pinia";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

// vue-router mock：route 用 reactive 对象驱动（跨项目导航场景改 params 即触发 computed）
const routeMock = reactive({ params: { id: "p1" } as Record<string, string>, path: "/projects/p1/overview" });
const pushMock = vi.fn();
vi.mock("vue-router", () => ({
  useRoute: () => routeMock,
  useRouter: () => ({ push: pushMock }),
}));

// RouterView 函数式 stub：调 scoped slot 传空 Component（<component :is="null"> 渲染占位）。
// 模板里的 <router-view> 走全局组件注册（vue-router 插件 install），vi.mock 拦不到，
// 须走 VTU global.components 注入。
const RouterViewStub = (_props: unknown, { slots }: { slots: { default?: (b: { Component: unknown }) => unknown } }) =>
  slots.default?.({ Component: null });

import ProjectDetailLayout from "../ProjectDetailLayout.vue";
import { useProjectStore } from "../../../stores/project";

function mockProject(id: string, name: string) {
  return {
    id, name, description: "", icon: "folder", sort_order: 0,
    workspace_path: null, theme_color: null, archived: false,
    created_at: "2026-01-01 00:00:00", updated_at: "2026-01-01 00:00:00", agents: [],
  };
}

/** 可恢复态断言：错误头 + 「找不到该项目。」+ 返回按钮三件齐 */
function expectRecoverable(w: ReturnType<typeof mountLayout>) {
  expect(w.find(".head-name.err").exists()).toBe(true);
  expect(w.text()).toContain("找不到该项目。");
  expect(w.find(".load-error .btn-link").exists()).toBe(true);
}

function mountLayout() {
  return mount(ProjectDetailLayout, { global: { components: { RouterView: RouterViewStub } } });
}

describe("ProjectDetailLayout 死态可恢复性", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset().mockResolvedValue([]);
    routeMock.params = { id: "p1" };
    routeMock.path = "/projects/p1/overview";
    pushMock.mockClear();
  });

  it("直链进入死项目（store 空 + load 仍无）→ loadError 可恢复态，不停在「加载中…」", async () => {
    const project = useProjectStore();
    project.list = [mockProject("pOther", "别的项目")]; // 有别的项目，p1 不在
    const w = mountLayout();
    await flushPromises();
    expectRecoverable(w);
    expect(w.text()).not.toContain("加载中…");
  });

  it("挂载后项目被永久删除（store list 移除）→ watch current 兜底翻可恢复态", async () => {
    const project = useProjectStore();
    project.list = [mockProject("p1", "项目一")];
    const w = mountLayout();
    await flushPromises();
    expect(w.text()).toContain("项目一"); // 挂载时正常

    project.list = []; // 项目被删（permanentDelete 后 store 移除）
    await flushPromises();
    expectRecoverable(w);
  });

  it("keep-alive 缓存复活（onActivated）→ 兜底重跑翻可恢复态（生产实案时序）", async () => {
    const project = useProjectStore();
    project.list = [mockProject("p1", "项目一")];

    // 真实 KeepAlive 容器：显示 → 隐藏（deactivate 进缓存）→ 再显示（activate 复活）
    const show = ref(true);
    const host = mount({
      setup: () => () => h(KeepAlive, () => (show.value ? h(ProjectDetailLayout) : h("div"))),
    }, { global: { components: { RouterView: RouterViewStub } } });
    await flushPromises();
    expect(host.text()).toContain("项目一");

    show.value = false; // 离开详情页（实例进缓存，onMounted 之后不再跑）
    await nextTick();
    project.list = []; // 在别的页面永久删除项目
    mockInvoke.mockResolvedValue([]);
    await nextTick();

    show.value = true; // 重进详情页 → 缓存实例复活，只有 onActivated 能兜底
    await nextTick();
    await flushPromises();
    expect(host.text()).toContain("找不到该项目。");
    expect(host.text()).not.toContain("加载中…");
  });

  it("同实例跨项目导航到死项目（watch projectId）→ 错误态重判", async () => {
    const project = useProjectStore();
    project.list = [mockProject("p1", "项目一")];
    const w = mountLayout();
    await flushPromises();
    expect(w.text()).toContain("项目一");

    routeMock.params = { id: "pDead" }; // keep-alive 类型键复用实例，仅 params 变
    await nextTick();
    await flushPromises();
    expectRecoverable(w);
  });

  it("可恢复态「返回项目列表」按钮 → router.push('/projects')", async () => {
    const project = useProjectStore();
    project.list = [];
    const w = mountLayout();
    await flushPromises();
    await w.find(".load-error .btn-link").trigger("click");
    expect(pushMock).toHaveBeenCalledWith("/projects");
  });
});
