import { describe, it, expect } from "vitest";
import { formatTokenCount } from "../format";

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
