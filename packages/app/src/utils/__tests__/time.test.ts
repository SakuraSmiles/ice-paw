import { describe, it, expect, beforeEach } from "vitest";
import { formatTime, formatDate, formatDateLabel, setTimezone } from "../time";

// 重置时区状态，确保测试隔离
beforeEach(() => {
  setTimezone("");
});

describe("formatTime", () => {
  it("formats a valid ISO string as HH:MM", () => {
    const result = formatTime("2024-06-15T14:30:00");
    expect(result).toMatch(/^\d{2}:\d{2}$/);
  });

  it("includes seconds when seconds=true", () => {
    const result = formatTime("2024-06-15T14:30:45", true);
    expect(result).toMatch(/^\d{2}:\d{2}:\d{2}$/);
  });

  it("returns empty string for invalid date", () => {
    expect(formatTime("not-a-date")).toBe("");
    expect(formatTime("")).toBe("");
  });

  it("uses timezone when set", () => {
    // UTC+8 时区：14:30 UTC → 22:30
    setTimezone("Asia/Shanghai");
    const result = formatTime("2024-06-15T14:30:00Z");
    expect(result).toBe("22:30");
  });
});

describe("formatDate", () => {
  it("returns YYYY-M-D for valid date", () => {
    const result = formatDate("2024-06-15T10:00:00");
    expect(result).toMatch(/^\d{4}-\d{1,2}-\d{1,2}$/);
  });

  it("returns empty string for invalid date", () => {
    expect(formatDate("bad")).toBe("");
  });
});

describe("formatDateLabel", () => {
  it("returns '今天' for today's date", () => {
    const today = new Date();
    const iso = today.toISOString();
    expect(formatDateLabel(iso)).toBe("今天");
  });

  it("returns '昨天' for yesterday's date", () => {
    const yesterday = new Date();
    yesterday.setDate(yesterday.getDate() - 1);
    const iso = yesterday.toISOString();
    expect(formatDateLabel(iso)).toBe("昨天");
  });

  it("returns M月D日 for older dates", () => {
    const result = formatDateLabel("2024-01-15T10:00:00");
    expect(result).toMatch(/^\d{1,2}月\d{1,2}日$/);
  });

  it("returns null for invalid date", () => {
    expect(formatDateLabel("invalid")).toBeNull();
  });
});
