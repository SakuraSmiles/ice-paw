// Combobox.items.test.ts — items 形态（label+value+note）行为锁：
// 下拉渲染 label 主行/note 副行、选中 emit value（非显示文本）、
// 按 label 或 value 过滤、外部 modelValue 变化同步显示 label。
// options: string[] 旧路径不动，由各使用方现有行为覆盖。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import Combobox from "../Combobox.vue";

const ITEMS = [
  { label: "智谱 GLM", value: "glm", note: "标准端点" },
  { label: "智谱 GLM Coding", value: "glm-coding", note: "Coding Plan 专用" },
  { label: "Ollama 本地", value: "ollama" },
];

function mountItems(modelValue = "") {
  return mount(Combobox, {
    props: { modelValue, items: ITEMS, placeholder: "选择或输入" },
    attachTo: document.body,
  });
}

describe("Combobox items 形态", () => {
  it("聚焦展开下拉：渲染 label 主行与 note 副行", async () => {
    const w = mountItems("glm");
    await w.find("input").trigger("focus");
    const opts = w.findAll(".combobox-option-rich");
    expect(opts).toHaveLength(3);
    expect(opts[0].text()).toContain("智谱 GLM");
    expect(opts[0].text()).toContain("标准端点");
    expect(opts[2].text()).toContain("Ollama 本地");
    w.unmount();
  });

  it("初始与外部 modelValue 变化：输入框显示 value 对应 label，未收录回退原文", async () => {
    const w = mountItems("glm-coding");
    const input = w.find("input").element as HTMLInputElement;
    expect(input.value).toBe("智谱 GLM Coding");
    await w.setProps({ modelValue: "ollama" });
    expect((w.find("input").element as HTMLInputElement).value).toBe("Ollama 本地");
    // 手输/旧数据的未收录值：显示 value 原文（自由输入语义）
    await w.setProps({ modelValue: "my-custom" });
    expect((w.find("input").element as HTMLInputElement).value).toBe("my-custom");
    w.unmount();
  });

  it("点选 emit value 而非 label，输入框展示 label", async () => {
    const w = mountItems();
    await w.find("input").trigger("focus");
    const opts = w.findAll(".combobox-option-rich");
    await opts[2].trigger("click");
    const emitted = w.emitted("update:modelValue");
    expect(emitted).toBeTruthy();
    expect(emitted![0]).toEqual(["ollama"]);
    expect((w.find("input").element as HTMLInputElement).value).toBe("Ollama 本地");
    w.unmount();
  });

  it("输入按 label 或 value 过滤下拉", async () => {
    const w = mountItems();
    const input = w.find("input");
    await input.trigger("focus");
    await input.setValue("智谱");
    expect(w.findAll(".combobox-option-rich")).toHaveLength(2);
    await input.setValue("glm-coding");
    const hits = w.findAll(".combobox-option-rich");
    expect(hits).toHaveLength(1);
    expect(hits[0].text()).toContain("智谱 GLM Coding");
    w.unmount();
  });

  it("手输与 label 精确一致时 emit 对应 value，不匹配则原文透传", async () => {
    const w = mountItems();
    const input = w.find("input");
    await input.trigger("focus");
    await input.setValue("Ollama 本地");
    const emitted = w.emitted("update:modelValue")!;
    expect(emitted[emitted.length - 1]).toEqual(["ollama"]);
    await input.setValue("my-vllm-endpoint");
    expect(emitted[emitted.length - 1]).toEqual(["my-vllm-endpoint"]);
    w.unmount();
  });
});
