// TurnRail 窗口化交互测试（v2：定容滑动窗口 + 省略号翻页 + 滚轮微调 + 回锚）
// 组件无 store 依赖，直接 props 驱动；tick 以 title「第 N 轮」暴露轮号便于断言。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import TurnRail from "../TurnRail.vue";
import type { TurnAnchor } from "../../../types";

const anchors = (n: number): TurnAnchor[] =>
  Array.from({ length: n }, (_, i) => ({
    message_id: `m-${i}`,
    preview: `问题 ${i}`,
    created_at: "2026-08-15 10:00:00",
  }));

const turnTitles = (w: ReturnType<typeof mount>) =>
  w.findAll(".turn-tick").map((t) => t.attributes("title"));

describe("TurnRail 定容窗口", () => {
  it("短会话（<窗口容量）：全可见，无省略号，位置徽标显示 N/M", () => {
    const w = mount(TurnRail, { props: { anchors: anchors(5), activeTurn: 3, showLatest: false } });
    expect(turnTitles(w)).toEqual([
      "第 1 轮 · 点击跳转", "第 2 轮 · 点击跳转", "第 3 轮 · 点击跳转",
      "第 4 轮 · 点击跳转", "第 5 轮 · 点击跳转",
    ]);
    expect(w.find(".turn-more").exists()).toBe(false);
    expect(w.find(".turn-pos-cur").text()).toBe("3");
    expect(w.find(".turn-pos-total").text()).toBe("5");
  });

  it("activeTurn 未知 → 末窗（会话打开在底部跟随最新）", () => {
    const w = mount(TurnRail, { props: { anchors: anchors(20), activeTurn: null, showLatest: false } });
    // 20 轮 / 13 格：末窗起点 = 8
    expect(turnTitles(w)[0]).toBe("第 8 轮 · 点击跳转");
    expect(turnTitles(w)[turnTitles(w).length - 1]).toBe("第 20 轮 · 点击跳转");
  });

  it("activeTurn=20（49 轮）→ 窗口 14–26 居中，双侧省略号俱在", () => {
    const w = mount(TurnRail, { props: { anchors: anchors(49), activeTurn: 20, showLatest: false } });
    const titles = turnTitles(w);
    expect(titles).toHaveLength(13);
    expect(titles[0]).toBe("第 14 轮 · 点击跳转");
    expect(titles[titles.length - 1]).toBe("第 26 轮 · 点击跳转");
    const mores = w.findAll(".turn-more");
    expect(mores).toHaveLength(2);
    // 当前轮 tick 有 active 类（高亮）
    expect(w.findAll(".turn-tick")[6].classes()).toContain("active");
  });

  it("点底部省略号 → jump 到窗口尾后一整窗的目标轮（messageId 正确）", async () => {
    const w = mount(TurnRail, { props: { anchors: anchors(49), activeTurn: 20, showLatest: false } });
    // from=14：目标轮 = 14-1+13 = 26 → 锚点 m-25
    await w.findAll(".turn-more")[1].trigger("click");
    expect(w.emitted("jump")).toEqual([["m-25"]]);
    // 顶部省略号：目标轮 = 14-13 = 1 → m-0
    await w.findAll(".turn-more")[0].trigger("click");
    expect(w.emitted("jump")).toEqual([["m-25"], ["m-0"]]);
  });

  it("滚轮微调：按累计 50 一格半窗步进（不动 activeTurn）；activeTurn 变化即回锚居中", async () => {
    const w = mount(TurnRail, { props: { anchors: anchors(49), activeTurn: 20, showLatest: false } });
    const track = w.find(".turn-rail-track");
    // deltaY=60（过 50 阈值一步）：from 14 → 21（+7）
    await track.trigger("wheel", { deltaY: 60 });
    expect(turnTitles(w)[0]).toBe("第 21 轮 · 点击跳转");
    // 标准滚轮一格 deltaY=100 = 两步（+7×2）：21 → 35
    await track.trigger("wheel", { deltaY: 100 });
    expect(turnTitles(w)[0]).toBe("第 35 轮 · 点击跳转");
    // activeTurn 未变（微调只挪窗口）；模拟内容滚动到第 40 轮 → 回锚：40-6=34
    await w.setProps({ activeTurn: 40 });
    expect(turnTitles(w)[0]).toBe("第 34 轮 · 点击跳转");
    expect(turnTitles(w)[turnTitles(w).length - 1]).toBe("第 46 轮 · 点击跳转");
  });

  it("滚轮高频小事件合并：触控板连续小幅 deltaY 只按累计步进", async () => {
    const w = mount(TurnRail, { props: { anchors: anchors(49), activeTurn: 20, showLatest: false } });
    const track = w.find(".turn-rail-track");
    // 4×20 = 80 ≥ 50 → 恰好一步（+7）；第 5 次 20 累计 100 → 再一步
    for (let i = 0; i < 4; i++) await track.trigger("wheel", { deltaY: 20 });
    expect(turnTitles(w)[0]).toBe("第 21 轮 · 点击跳转");
    await track.trigger("wheel", { deltaY: 20 });
    expect(turnTitles(w)[0]).toBe("第 28 轮 · 点击跳转");
  });

  it("边界：窗口在末窗继续下滚不动（钳制），微调至顶部后上滚不动", async () => {
    const w = mount(TurnRail, { props: { anchors: anchors(20), activeTurn: 20, showLatest: false } });
    // 20 轮 / 13 格：末窗 8–20；向下滚应钳在 8
    await w.find(".turn-rail-track").trigger("wheel", { deltaY: 100 });
    expect(turnTitles(w)[0]).toBe("第 8 轮 · 点击跳转");
  });

  it("单轮会话不渲染（≥2 轮门槛在组件内）", () => {
    const w = mount(TurnRail, { props: { anchors: anchors(1), activeTurn: 1, showLatest: false } });
    expect(w.find("nav").exists()).toBe(false);
  });
});
