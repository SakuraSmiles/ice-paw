import { describe, it, expect } from "vitest";
import { transferKey, thinkSegLabel } from "../groupCollapse";

// U3-3 第一刀下沉的纯函数单测——原行为只能经 mount ChatMessages 间接验证
// （分页前插并组键易主的展开态转移 / 聚合段标签），下沉后直接断言。

describe("transferKey", () => {
  it("旧键在集合里 → 转移到新键，返回新集合且不原地 mutate", () => {
    const set = new Set(["a", "b"]);
    const next = transferKey(set, "a", "grp-new");
    expect(next.has("a")).toBe(false);
    expect(next.has("b")).toBe(true);
    expect(next.has("grp-new")).toBe(true);
    // 不可变语义：原集合未被污染
    expect(set.has("a")).toBe(true);
    expect(set.has("grp-new")).toBe(false);
    expect(set.size).toBe(2);
  });

  it("旧键不在集合里 → 原样复制返回，不凭空加新键", () => {
    const set = new Set(["b"]);
    const next = transferKey(set, "a", "grp-new");
    expect(next.has("b")).toBe(true);
    expect(next.has("grp-new")).toBe(false);
    expect(next.size).toBe(1);
  });

  it("空集合 → 空集合", () => {
    const next = transferKey(new Set<string>(), "a", "b");
    expect(next.size).toBe(0);
  });

  it("泛型支持非字符串键", () => {
    const set = new Set([1, 2]);
    const next = transferKey(set, 1, 3);
    expect(next.has(3)).toBe(true);
    expect(next.has(1)).toBe(false);
    expect(next.has(2)).toBe(true);
  });
});

describe("thinkSegLabel", () => {
  it("有耗时 → 「思考 · 30s」（秒/分换算走 formatThinkingMs 单一真相源）", () => {
    expect(thinkSegLabel({ durationMs: 30_000 })).toBe("思考 · 30s");
  });

  it("分/秒复合耗时", () => {
    expect(thinkSegLabel({ durationMs: 90_000 })).toBe("思考 · 1m 30s");
  });

  it("无耗时 → 只显「思考」", () => {
    expect(thinkSegLabel({ durationMs: null })).toBe("思考");
  });
});
