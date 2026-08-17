import { describe, expect, it } from "vitest";
import { isWarnTermination, termLabel } from "../termLabels";

describe("termLabel", () => {
  it("词表内值出中文", () => {
    expect(termLabel("stop")).toBe("正常结束");
    expect(termLabel("interrupted")).toBe("应用中断");
  });

  it("backfill 出「历史补录」（旧会话 backfill 验收暴露的裸透回归）", () => {
    expect(termLabel("backfill")).toBe("历史补录");
  });

  it("词表外裸透原值（技术兜底，不猜）", () => {
    expect(termLabel("some_future_reason")).toBe("some_future_reason");
  });
});

describe("isWarnTermination", () => {
  it("stop 与 backfill 不算 warn——补录是诚实标注的历史数据，非异常", () => {
    expect(isWarnTermination("stop")).toBe(false);
    expect(isWarnTermination("backfill")).toBe(false);
  });

  it("其余终止渲染 warn 态", () => {
    expect(isWarnTermination("error")).toBe(true);
    expect(isWarnTermination("interrupted")).toBe(true);
    expect(isWarnTermination("budget_exceeded")).toBe(true);
  });
});
