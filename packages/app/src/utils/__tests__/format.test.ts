import { describe, it, expect } from "vitest";
import { formatTokenCompact, formatTokenCount, formatThinkingMs } from "../format";

describe("formatTokenCompact", () => {
  it("三档：原数 / K / M（图表语境，不走万进制）", () => {
    expect(formatTokenCompact(0)).toBe("0");
    expect(formatTokenCompact(42)).toBe("42");
    expect(formatTokenCompact(999)).toBe("999");
    expect(formatTokenCompact(1000)).toBe("1K");
    expect(formatTokenCompact(2700)).toBe("2.7K");
    expect(formatTokenCompact(11_700)).toBe("11.7K");
    // 999999 → 999.999K 四舍五入进位 1000.0K（toFixed 语义，与万进制版同款边界）
    expect(formatTokenCompact(999_999)).toBe("1000.0K");
    expect(formatTokenCompact(1_000_000)).toBe("1M");
    expect(formatTokenCompact(1_230_000)).toBe("1.2M");
  });

  it("非有限数原样返回字符串", () => {
    expect(formatTokenCompact(Number.NaN)).toBe("NaN");
  });
});

describe("formatTokenCount", () => {
  it("三档：原数 / K / 万", () => {
    expect(formatTokenCount(0)).toBe("0");
    expect(formatTokenCount(42)).toBe("42");
    expect(formatTokenCount(999)).toBe("999");
    expect(formatTokenCount(1000)).toBe("1K");
    expect(formatTokenCount(1500)).toBe("1.5K");
    expect(formatTokenCount(9900)).toBe("9.9K");
    // 9999 → 9.999K 四舍五入进位为 10.0K（toFixed 语义，可接受）
    expect(formatTokenCount(9999)).toBe("10.0K");
    expect(formatTokenCount(10_000)).toBe("1万");
    expect(formatTokenCount(80_000)).toBe("8万");
    expect(formatTokenCount(800_000)).toBe("80万");
    expect(formatTokenCount(125_000)).toBe("12.5万");
  });

  it("非有限数与负数原样返回字符串", () => {
    expect(formatTokenCount(-5)).toBe("-5");
    expect(formatTokenCount(Number.NaN)).toBe("NaN");
  });
});

describe("formatThinkingMs", () => {
  it("秒档：整数秒（floor），与流式计时显示同一形态", () => {
    expect(formatThinkingMs(0)).toBe("0s");
    expect(formatThinkingMs(999)).toBe("0s");
    expect(formatThinkingMs(30_000)).toBe("30s");
    expect(formatThinkingMs(59_999)).toBe("59s");
  });

  it("分级：分 + 余秒", () => {
    expect(formatThinkingMs(60_000)).toBe("1m 0s");
    expect(formatThinkingMs(90_000)).toBe("1m 30s");
    expect(formatThinkingMs(125_400)).toBe("2m 5s");
  });
});
