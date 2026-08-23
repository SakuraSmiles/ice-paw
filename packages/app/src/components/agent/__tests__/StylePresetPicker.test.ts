// StylePresetPicker.test.ts — 风格预设弹层行为锁（2026-08-23 交互重做后）：
// 全文可见（治「描述看不全」）/ create 选中切换 / edit 直插与覆盖确认两态
// （免确认豁免 = 明确无值或出生默认句）。teleport stub 到原地再查 DOM。
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

describe("StylePresetPicker", () => {
  it("三档卡片全文渲染（不止前 3 行——{name} 已替换）", () => {
    const w = mountPicker();
    const cards = w.findAll(".sp-card");
    expect(cards).toHaveLength(3);
    const eng = cards[0].text();
    expect(eng).toContain("工程");
    expect(eng).toContain("你是小冰，一名工程助手。");
    // 第 4 行之后的深层内容也可见（原面板只预览 3 行，本次重做的核心诉求）
    expect(eng).toContain("先确认再动手");
    expect(eng).toContain("write_file 落盘");
    w.unmount();
  });

  it("create：点卡选中 → 父回写 selectedId 后再点同卡取消（受控组件语义）", async () => {
    const w = mountPicker();
    await w.findAll(".sp-card")[0].trigger("click");
    expect(w.emitted("select")).toHaveLength(1);
    expect(w.emitted("select")![0][0]).toMatchObject({ id: "engineering" });
    // 父级把选中态回灌（AgentForm 真实流程：selectedPreset → selectedId prop）
    await w.setProps({ selectedId: "engineering" });
    await w.findAll(".sp-card")[0].trigger("click");
    expect(w.emitted("select")![1][0]).toBeNull();
    w.unmount();
  });

  it("create：已选档高亮（selectedId → .selected）", () => {
    const w = mountPicker({ selectedId: "companion" });
    expect(w.findAll(".sp-card")[2].classes()).toContain("selected");
    w.unmount();
  });

  it("edit 无值：点卡直接 emit pick（免确认）", async () => {
    const w = mountPicker({ mode: "edit", existingPrompt: null });
    await w.findAll(".sp-card")[1].trigger("click");
    expect(w.emitted("pick")).toHaveLength(1);
    expect(w.emitted("pick")![0][0]).toMatchObject({ id: "creative" });
    expect(w.find(".sp-confirm").exists()).toBe(false);
    w.unmount();
  });

  it("edit 出生默认句：免确认直插（最常见操作不拦）", async () => {
    const w = mountPicker({ mode: "edit", existingPrompt: "小冰 是一个 AI 助手。" });
    await w.findAll(".sp-card")[0].trigger("click");
    expect(w.emitted("pick")).toHaveLength(1);
    w.unmount();
  });

  it("edit 已有自定义内容：先翻覆盖确认，确认后才 emit pick", async () => {
    const w = mountPicker({ mode: "edit", existingPrompt: "你是一个品牌设计助手。\n第二行。" });
    await w.findAll(".sp-card")[0].trigger("click");
    expect(w.emitted("pick")).toBeUndefined();
    const confirm = w.find(".sp-confirm");
    expect(confirm.exists()).toBe(true);
    expect(confirm.text()).toContain("你是一个品牌设计助手。");
    await confirm.find(".sp-btn.primary").trigger("click");
    expect(w.emitted("pick")).toHaveLength(1);
    w.unmount();
  });

  it("Esc 关闭（useEscapeStack 栈顶回调）", async () => {
    const w = mountPicker();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await nextTick();
    expect(w.emitted("close")).toHaveLength(1);
    w.unmount();
  });
});
