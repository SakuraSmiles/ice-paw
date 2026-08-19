import { describe, it, expect } from "vitest";
import { memoized } from "../blockMemo";

describe("memoized", () => {
  it("同 key 返回同一引用（命中缓存）", () => {
    const fn = memoized((s: string) => ({ len: s.length }));
    const a = fn("abc");
    const b = fn("abc");
    expect(b).toBe(a);
  });

  it("不同 key 各自结果，互不串扰", () => {
    const fn = memoized((s: string) => [s]);
    expect(fn("a")[0]).toBe("a");
    expect(fn("b")[0]).toBe("b");
    expect(fn("a")).toBe(fn("a"));
  });

  it("同 key 只计算一次（纯函数幂等前提）", () => {
    let calls = 0;
    const fn = memoized((s: string) => {
      calls++;
      return s.toUpperCase();
    });
    expect(fn("x")).toBe("X");
    expect(fn("x")).toBe("X");
    expect(calls).toBe(1);
  });

  it("超 maxEntries 全清且可重建（重建值相等、引用重建）", () => {
    const fn = memoized((s: string) => ({ v: s }), 3);
    const k1 = fn("1");
    fn("2");
    fn("3");
    expect(fn("1")).toBe(k1); // 3 个 key 在限内，命中
    fn("4"); // 第 4 个触发全清
    const k1b = fn("1");
    expect(k1b).not.toBe(k1); // 已清空 → 重算重建引用
    expect(k1b).toEqual(k1); // 纯函数保证值相等
  });

  it("null 结果也可缓存命中", () => {
    let calls = 0;
    const fn = memoized((s: string): string | null => {
      calls++;
      return s === "bad" ? null : s;
    });
    expect(fn("bad")).toBeNull();
    expect(fn("bad")).toBeNull();
    expect(calls).toBe(1);
  });
});
