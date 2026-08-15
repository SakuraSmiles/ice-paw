// useTurnRail 分桶纯函数单测（千轮规模聚合是导航条的核心行为）
import { describe, it, expect } from "vitest";
import { buildTurnBuckets, RAIL_CAPACITY } from "../useTurnRail";
import type { TurnAnchor } from "../../types";

const anchor = (i: number): TurnAnchor => ({
  message_id: `m-${i}`,
  preview: `问题 ${i}`,
  created_at: "2026-08-15 10:00:00",
});

describe("buildTurnBuckets", () => {
  it("空锚点 → 空轨道", () => {
    expect(buildTurnBuckets([], RAIL_CAPACITY)).toEqual([]);
  });

  it("≤容量：一轮一线，轮号 1-based 连续", () => {
    const out = buildTurnBuckets([anchor(0), anchor(1), anchor(2)], 120);
    expect(out).toHaveLength(3);
    expect(out[0]).toMatchObject({ from: 1, to: 1, messageId: "m-0" });
    expect(out[2]).toMatchObject({ from: 3, to: 3, messageId: "m-2" });
  });

  it("恰好在容量边界不聚合", () => {
    const n = RAIL_CAPACITY;
    const out = buildTurnBuckets(Array.from({ length: n }, (_, i) => anchor(i)), RAIL_CAPACITY);
    expect(out).toHaveLength(n);
    expect(out.every((b) => b.from === b.to)).toBe(true);
  });

  it("超容量：等量聚合，组数 ≤ 容量，区间连续无缝无重叠", () => {
    const n = 3000; // 几千轮目标规模
    const out = buildTurnBuckets(Array.from({ length: n }, (_, i) => anchor(i)), RAIL_CAPACITY);
    expect(out.length).toBeLessThanOrEqual(RAIL_CAPACITY);
    expect(out[0].from).toBe(1);
    expect(out[out.length - 1].to).toBe(n);
    for (let i = 1; i < out.length; i++) {
      // 组间应无缝衔接
      expect(out[i].from).toBe(out[i - 1].to + 1);
    }
    // 每组以首轮为跳转锚 + 预览
    expect(out[0].messageId).toBe("m-0");
    expect(out[0].preview).toBe("问题 0");
  });

  it("末组不满（3001 轮 → 26 组×25 + 1 组×1 之类的尾差）仍连续", () => {
    const n = RAIL_CAPACITY * 2 + 1; // groupSize=3，末组 1 轮
    const out = buildTurnBuckets(Array.from({ length: n }, (_, i) => anchor(i)), RAIL_CAPACITY);
    expect(out[out.length - 1].to).toBe(n);
    expect(out[out.length - 1].from).toBe(n); // 末组恰好单轮
  });
});
