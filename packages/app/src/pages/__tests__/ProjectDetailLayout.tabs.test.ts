// ProjectDetailLayout.tabs.test.ts — 真实 vue-router 锁定 tab 导航机制：
// ① 点 tab → URL 与渲染组件同步切换（回归：KeepAlive 缓存键=vnode.key，
//    曾因三 tab 共用 :key=项目id 导致缓存命中旧 tab 实例、视图冻结）；
// ② 跨项目同 tab 不复用实例（防缓存串数据，key 隔离的另一半）；
// ③ tab 间往返 keep-alive 保实例（挂载数不涨——tab 状态保留是 keep-alive 的本意）。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { createRouter, createMemoryHistory, type Router } from "vue-router";
import { defineComponent, h } from "vue";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import ProjectDetailLayout from "../project/ProjectDetailLayout.vue";

const mockInvoke = vi.mocked(invoke);

/** 哑组件按 tab 显著标记；onMounted 计数供跨项目隔离/保实例断言 */
const mounts = { ov: 0, tl: 0 };
const dummy = (cls: string, key: "ov" | "tl") =>
  defineComponent({
    setup() {
      return { n: ++mounts[key] };
    },
    render: () => h("div", { class: `dummy-${cls}` }, `${cls.toUpperCase()}#${mounts[key]}`),
  });

function buildRouter(): Router {
  return createRouter({
    history: createMemoryHistory(),
    routes: [{
      path: "/",
      component: { template: "<div><router-view /></div>" },
      children: [{
        path: "projects/:id",
        component: ProjectDetailLayout,
        children: [
          { path: "", redirect: { name: "PD-Overview" } },
          { path: "overview", name: "PD-Overview", component: dummy("ov", "ov") },
          { path: "timeline", name: "PD-Timeline", component: dummy("tl", "tl") },
          { path: "settings", name: "PD-Settings", component: { template: "<div class='dummy-st'>ST</div>" } },
        ],
      }],
    }],
  });
}

async function mountLayout(router: Router) {
  const w = mount({ template: "<router-view />" }, { global: { plugins: [router] } });
  await flushPromises();
  return w;
}

describe("ProjectDetailLayout tab 导航", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mounts.ov = 0;
    mounts.tl = 0;
    mockInvoke.mockReset().mockImplementation((async (cmd: string) => {
      if (cmd === "list_projects") return [{
        id: "p1", name: "Alpha", description: "", icon: "folder", sort_order: 0,
        workspace_path: null, theme_color: null, archived: false,
        created_at: "2026-08-18 00:00:00", updated_at: "2026-08-18 00:00:00",
        agents: [],
      }];
      if (cmd === "list_agents") return [];
      return undefined;
    }) as never);
  });

  it("点 tab → URL 与渲染组件同步切换（keep-alive 键回归）", async () => {
    const router = buildRouter();
    await router.push("/projects/p1");
    await router.isReady();
    const w = await mountLayout(router);

    expect(router.currentRoute.value.fullPath).toBe("/projects/p1/overview");
    expect(w.find(".dummy-ov").exists()).toBe(true);

    await w.findAll(".tab-item").find((b) => b.text() === "项目轨迹")!.trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.fullPath).toBe("/projects/p1/timeline");
    expect(w.find(".dummy-tl").exists()).toBe(true);
    expect(w.find(".dummy-ov").exists()).toBe(false);

    await w.findAll(".tab-item").find((b) => b.text() === "设置")!.trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.fullPath).toBe("/projects/p1/settings");
    expect(w.find(".dummy-st").exists()).toBe(true);
  });

  it("tab 往返 keep-alive 保实例（挂载数不涨）", async () => {
    const router = buildRouter();
    await router.push("/projects/p1/overview");
    await router.isReady();
    const w = await mountLayout(router);
    expect(mounts.ov).toBe(1);

    await w.findAll(".tab-item").find((b) => b.text() === "项目轨迹")!.trigger("click");
    await flushPromises();
    await w.findAll(".tab-item").find((b) => b.text() === "概览")!.trigger("click");
    await flushPromises();

    expect(w.find(".dummy-ov").exists()).toBe(true);
    expect(mounts.ov).toBe(1); // 复活缓存实例而非重挂载
  });

  it("跨项目同 tab 不复用实例（防缓存串数据）", async () => {
    const router = buildRouter();
    await router.push("/projects/p1/timeline");
    await router.isReady();
    const w = await mountLayout(router);
    expect(w.find(".dummy-tl").text()).toBe("TL#1");

    await router.push("/projects/p2/timeline");
    await flushPromises();

    expect(w.find(".dummy-tl").text()).toBe("TL#2"); // 新项目新实例
  });
});
