// StatusGlyph.test.ts — 状态图标语系（2026-09-04 档1）：五态 DOM 形态与降级语义。
// 断言的是结构契约（像素格 9 格 / 对勾 / 叉 / 空心环），视觉与动画靠 dev 手测。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import StatusGlyph from "../StatusGlyph.vue";

describe("StatusGlyph 五态形态", () => {
  it("running：3×3 像素格恰 9 格，delay 按顺时针点亮序错峰", () => {
    const w = mount(StatusGlyph, { props: { status: "running" } });
    const cells = w.findAll(".px-cell");
    expect(cells).toHaveLength(9);
    // 点亮序（DOM 行优先）：1,2,3 顶行零错峰起步，6,9 右列、8,7 底行、4 左列、5 中心殿后
    const delays = cells.map((c) => c.attributes("style") ?? "");
    expect(delays[0]).toContain("0ms");
    expect(delays[2]).toContain("160ms");
    expect(delays[5]).toContain("240ms"); // 右列 6
    expect(delays[8]).toContain("320ms"); // 右下 9
    expect(delays[4]).toContain("640ms"); // 中心 5 最后
  });

  it("done：环形 + 对勾图标", () => {
    const w = mount(StatusGlyph, { props: { status: "done" } });
    expect(w.find(".glyph-done").exists()).toBe(true);
    // Lucide Check 渲染为 svg（class 透传 lucide）
    expect(w.find(".glyph-done svg").exists()).toBe(true);
  });

  it("error：环形 + 叉图标", () => {
    const w = mount(StatusGlyph, { props: { status: "error" } });
    expect(w.find(".glyph-error svg").exists()).toBe(true);
  });

  it("wait / pending：空心环无图标", () => {
    const wait = mount(StatusGlyph, { props: { status: "wait" } });
    expect(wait.find(".glyph-wait").exists()).toBe(true);
    expect(wait.find("svg").exists()).toBe(false);

    const pending = mount(StatusGlyph, { props: { status: "pending" } });
    expect(pending.find(".glyph-pending").exists()).toBe(true);
    expect(pending.find("svg").exists()).toBe(false);
    expect(pending.find(".px-cell").exists()).toBe(false);
  });

  it("aria 文案：默认按状态；label 覆盖（TaskPanel 已结束 → 已结束）", () => {
    const running = mount(StatusGlyph, { props: { status: "running" } });
    expect(running.attributes("aria-label")).toBe("进行中");

    const ended = mount(StatusGlyph, { props: { status: "pending", label: "已结束" } });
    expect(ended.attributes("aria-label")).toBe("已结束");
  });

  it("class 透传到根（TaskPanel 计划行划线兄弟选择器依赖）", () => {
    const w = mount(StatusGlyph, { props: { status: "done", class: "plan-mark-done" } });
    expect(w.find(".status-glyph.plan-mark-done").exists()).toBe(true);
  });

  it("size 驱动 CSS 变量（默认 14）", () => {
    const def = mount(StatusGlyph, { props: { status: "done" } });
    expect(def.attributes("style")).toContain("--glyph-size: 14px");

    const sm = mount(StatusGlyph, { props: { status: "done", size: 12 } });
    expect(sm.attributes("style")).toContain("--glyph-size: 12px");
  });
});
