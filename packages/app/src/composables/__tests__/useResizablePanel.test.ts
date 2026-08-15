// useResizablePanel 单测：钳制纯函数（含视口联动）+ 持久化加载/重置。
// pointer 拖拽循环（window 事件 + body 光标）靠真机手测覆盖。
import { describe, it, expect, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import { defineComponent, h } from "vue";
import { clampPanelWidth, useResizablePanel } from "../useResizablePanel";

describe("clampPanelWidth", () => {
  const V = 1920; // 视口宽度

  it("静态界内原样过（取整）", () => {
    expect(clampPanelWidth(400, 300, 720, V)).toBe(400);
    expect(clampPanelWidth(420.6, 300, 720, V)).toBe(421);
  });

  it("越 min/max 夹回边界", () => {
    expect(clampPanelWidth(100, 300, 720, V)).toBe(300);
    expect(clampPanelWidth(9999, 300, 720, V)).toBe(720);
  });

  it("视口 50% 动态上限：小窗口时 max 被压到半屏", () => {
    // 视口 600 → 半屏 300；max 720 失效，但仍不低于 min
    expect(clampPanelWidth(700, 300, 720, 600)).toBe(300);
    // 视口 1000 → 半屏 500
    expect(clampPanelWidth(700, 300, 720, 1000)).toBe(500);
  });

  it("极端小窗口：半屏低于 min 时保底 min（宁可超出半屏也要可用）", () => {
    expect(clampPanelWidth(500, 300, 720, 400)).toBe(300);
  });
});

/** 在组件 setup 内跑 composable（onBeforeUnmount 需组件实例） */
function withPanel(key: string) {
  let api!: ReturnType<typeof useResizablePanel>;
  const Comp = defineComponent({
    setup() {
      api = useResizablePanel({ key, default: 320, min: 240, max: 480, dir: 1 });
      return () => h("div");
    },
  });
  mount(Comp);
  return api;
}

describe("useResizablePanel 持久化", () => {
  beforeEach(() => localStorage.clear());

  it("无存档 → 默认宽", () => {
    expect(withPanel("t1").width.value).toBe(320);
  });

  it("坏存档（NaN/垃圾串）→ 回默认不炸", () => {
    localStorage.setItem("icepaw-panel-t2", "abc");
    expect(withPanel("t2").width.value).toBe(320);
  });

  it("合法存档 → 恢复；reset → 回默认并清档", () => {
    localStorage.setItem("icepaw-panel-t3", "380");
    const p = withPanel("t3");
    expect(p.width.value).toBe(380);
    p.reset();
    expect(p.width.value).toBe(320);
    expect(localStorage.getItem("icepaw-panel-t3")).toBeNull();
  });
});
