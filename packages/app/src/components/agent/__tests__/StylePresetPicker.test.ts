// StylePresetPicker.test.ts — 风格预设弹层行为锁（2026-08-23 第三轮：胶囊 tab
// + 单片全文 + 底部显式确认）。核心不变式：点胶囊只切换浏览（零 emit），确认
// 动作只走底部主按钮；edit 覆盖确认两段式（横幅 → 覆盖写入），出生默认句豁免。
import { describe, it, expect } from "vitest";
import { nextTick } from "vue";
import { mount } from "@vue/test-utils";
import StylePresetPicker from "../StylePresetPicker.vue";

function mountPicker(props: Record<string, unknown> = {}) {
  return mount(StylePresetPicker, {
    props: { mode: "create", agentName: "小冰", ...props },
    attachTo: document.body,
    global: { stubs: { teleport: true } },
  });
}

/** 底部主按钮（非确认横幅态） */
function primaryBtn(w: ReturnType<typeof mountPicker>) {
  return w.findAll(".sp-btn.primary").filter(Boolean).slice(-1)[0];
}

describe("StylePresetPicker · tab 浏览", () => {
  it("三个胶囊 tab，点=切换内容，不触发任何选择 emit", async () => {
    const w = mountPicker();
    const tabs = w.findAll(".sp-tab");
    expect(tabs).toHaveLength(3);
    expect(tabs[0].classes()).toContain("active"); // 默认第一档（工程）
    expect(w.find(".sp-text").text()).toContain("你是小冰，一名工程助手。");

    await tabs[1].trigger("click");
    // 重渲染后旧 wrapper 是陈旧 DOM，重新查询断言 active
    expect(w.findAll(".sp-tab")[1].classes()).toContain("active");
    // 深层行也可见（单片全文展示，治「看不全」）
    expect(w.find(".sp-text").text()).toContain("跨章节不漂移");
    expect(w.emitted("select")).toBeUndefined();
    expect(w.emitted("pick")).toBeUndefined();
    w.unmount();
  });

  it("方向键左右循环切档（tablist 键盘行为）", async () => {
    const w = mountPicker();
    await w.find(".sp-tabs").trigger("keydown", { key: "ArrowRight" });
    expect(w.findAll(".sp-tab")[1].classes()).toContain("active");
    await w.find(".sp-tabs").trigger("keydown", { key: "ArrowRight" });
    await w.find(".sp-tabs").trigger("keydown", { key: "ArrowRight" });
    expect(w.findAll(".sp-tab")[0].classes()).toContain("active");
    w.unmount();
  });

  it("打开时定位到已选档（selectedId → 初始 active）", () => {
    const w = mountPicker({ selectedId: "companion" });
    expect(w.findAll(".sp-tab")[2].classes()).toContain("active");
    expect(w.find(".sp-text").text()).toContain("对话伙伴");
    w.unmount();
  });
});

describe("StylePresetPicker · create 确认流", () => {
  it("主按钮「使用该风格」emit select（浏览 ≠ 选择）", async () => {
    const w = mountPicker();
    await w.findAll(".sp-tab")[2].trigger("click");
    await primaryBtn(w).trigger("click");
    expect(w.emitted("select")).toHaveLength(1);
    expect(w.emitted("select")![0][0]).toMatchObject({ id: "companion" });
    w.unmount();
  });

  it("已选档：胶囊带 ✓ + 「清除选择」emit select(null)", async () => {
    const w = mountPicker({ selectedId: "engineering" });
    expect(w.findAll(".sp-tab")[0].find(".sp-tab-check").exists()).toBe(true);
    await w.findAll(".sp-btn.ghost").filter((b) => b.text() === "清除选择")[0].trigger("click");
    expect(w.emitted("select")![0][0]).toBeNull();
    w.unmount();
  });
});

describe("StylePresetPicker · edit 确认流", () => {
  it("无值：主按钮直接 emit pick（免确认）", async () => {
    const w = mountPicker({ mode: "edit", existingPrompt: null });
    await w.findAll(".sp-tab")[1].trigger("click");
    await primaryBtn(w).trigger("click");
    expect(w.emitted("pick")).toHaveLength(1);
    expect(w.emitted("pick")![0][0]).toMatchObject({ id: "creative" });
    expect(w.find(".sp-confirm-text").exists()).toBe(false);
    w.unmount();
  });

  it("出生默认句：免确认直插（最常见操作不拦）", async () => {
    const w = mountPicker({ mode: "edit", existingPrompt: "小冰 是一个 AI 助手。" });
    await primaryBtn(w).trigger("click");
    expect(w.emitted("pick")).toHaveLength(1);
    w.unmount();
  });

  it("已有自定义内容：先出覆盖横幅，确认后才 emit pick；切档复位横幅", async () => {
    const w = mountPicker({ mode: "edit", existingPrompt: "你是一个品牌设计助手。\n第二行。" });
    await primaryBtn(w).trigger("click");
    expect(w.emitted("pick")).toBeUndefined();
    expect(w.find(".sp-confirm-text").text()).toContain("你是一个品牌设计助手。");
    // 返回：横幅收起仍不 emit
    await w.findAll(".sp-btn.ghost").filter((b) => b.text() === "返回")[0].trigger("click");
    expect(w.find(".sp-confirm-text").exists()).toBe(false);
    // 再确认 → 覆盖写入
    await primaryBtn(w).trigger("click");
    await primaryBtn(w).trigger("click");
    expect(w.emitted("pick")).toHaveLength(1);
    // 覆盖横幅出现后切档 → 横幅复位
    await w.findAll(".sp-tab")[1].trigger("click");
    expect(w.find(".sp-confirm-text").exists()).toBe(false);
    w.unmount();
  });
});

describe("StylePresetPicker · 关闭", () => {
  it("✕ 按钮与 Esc 都关闭（useEscapeStack 栈顶回调）", async () => {
    const w = mountPicker();
    await w.find(".sp-close").trigger("click");
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await nextTick();
    expect(w.emitted("close")).toHaveLength(2);
    w.unmount();
  });
});
