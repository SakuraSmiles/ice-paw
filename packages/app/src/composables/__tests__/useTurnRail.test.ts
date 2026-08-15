// useTurnRail 窗口纯函数单测（定容滑动窗口是导航条 v2 的核心行为）
import { describe, it, expect } from "vitest";
import { autoWindowStart, buildTurnWindow, RAIL_WINDOW } from "../useTurnRail";
import type { TurnAnchor } from "../../types";

const anchor = (i: number): TurnAnchor => ({
  message_id: `m-${i}`,
  preview: `问题 ${i}`,
  created_at: "2026-08-15 10:00:00",
});

const anchors = (n: number) => Array.from({ length: n }, (_, i) => anchor(i));

describe("autoWindowStart", () => {
  it("activeTurn 居中（前偏 ⌊size/2⌋）", () => {
    // 49 轮 / 13 格：activeTurn=20 → 起点应为 20-6=14
    expect(autoWindowStart(49, 20, RAIL_WINDOW)).toBe(14);
  });

  it("头部钳制：activeTurn 靠近开头时窗口起点不低于 1", () => {
    expect(autoWindowStart(49, 1, RAIL_WINDOW)).toBe(1);
    expect(autoWindowStart(49, 6, RAIL_WINDOW)).toBe(1); // 6-6=0 → 钳到 1
  });

  it("尾部钳制：activeTurn 靠近末尾时窗口不超过末窗", () => {
    const maxStart = 49 - RAIL_WINDOW + 1; // 37
    expect(autoWindowStart(49, 49, RAIL_WINDOW)).toBe(maxStart);
    expect(autoWindowStart(49, 45, RAIL_WINDOW)).toBe(maxStart); // 45-6=39 → 钳到 37
  });

  it("activeTurn 未知（null）→ 末窗起点（会话打开在底部跟随最新）", () => {
    expect(autoWindowStart(49, null, RAIL_WINDOW)).toBe(49 - RAIL_WINDOW + 1);
  });

  it("总轮数 ≤ 窗口容量 → 恒为 1（全可见）", () => {
    expect(autoWindowStart(5, 3, RAIL_WINDOW)).toBe(1);
    expect(autoWindowStart(0, null, RAIL_WINDOW)).toBe(1);
  });
});

describe("buildTurnWindow", () => {
  it("空锚点 → 空切片，无边缘指示", () => {
    const w = buildTurnWindow([], 1, RAIL_WINDOW);
    expect(w.ticks).toEqual([]);
    expect(w.total).toBe(0);
    expect(w.hasPrev).toBe(false);
    expect(w.hasNext).toBe(false);
  });

  it("总轮数 ≤ 容量：全可见，一轮一线，轮号 1-based 连续", () => {
    const w = buildTurnWindow(anchors(5), 1, RAIL_WINDOW);
    expect(w.ticks.map((t) => t.turn)).toEqual([1, 2, 3, 4, 5]);
    expect(w.ticks[0]).toMatchObject({ messageId: "m-0", preview: "问题 0" });
    expect(w.hasPrev).toBe(false);
    expect(w.hasNext).toBe(false);
  });

  it("3000 轮：切片恒 ≤ 容量，from..from+size-1 连续无缝", () => {
    const from = 100;
    const w = buildTurnWindow(anchors(3000), from, RAIL_WINDOW);
    expect(w.ticks).toHaveLength(RAIL_WINDOW);
    expect(w.ticks[0].turn).toBe(from);
    expect(w.ticks[RAIL_WINDOW - 1].turn).toBe(from + RAIL_WINDOW - 1);
    expect(w.ticks[0].messageId).toBe("m-99"); // 轮号 100 = 锚点下标 99
    expect(w.hasPrev).toBe(true);
    expect(w.hasNext).toBe(true);
  });

  it("from 越界向内钳制（0 → 1；超过末窗 → 末窗起点）", () => {
    expect(buildTurnWindow(anchors(49), 0, RAIL_WINDOW).from).toBe(1);
    expect(buildTurnWindow(anchors(49), 999, RAIL_WINDOW).from).toBe(49 - RAIL_WINDOW + 1);
  });

  it("末窗（from=末窗起点）：hasNext=false，hasPrev=true（total>size）", () => {
    const w = buildTurnWindow(anchors(49), 37, RAIL_WINDOW);
    expect(w.hasPrev).toBe(true);
    expect(w.hasNext).toBe(false);
    expect(w.ticks[RAIL_WINDOW - 1].turn).toBe(49);
  });

  it("窗口切换 tick key 稳定：key=全局轮号（窗口滑动时元素按轮号对应）", () => {
    const a = buildTurnWindow(anchors(49), 14, RAIL_WINDOW);
    const b = buildTurnWindow(anchors(49), 21, RAIL_WINDOW);
    expect(b.ticks[0].turn).toBe(21);
    expect(b.ticks[0].messageId).toBe("m-20");
    expect(a.ticks[RAIL_WINDOW - 1].turn).toBe(26);
  });
});
