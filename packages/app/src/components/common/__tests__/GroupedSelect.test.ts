// GroupedSelect.test.ts — 可选可输分组选择器（el-select filterable + optgroup 风格）行为锁：
// 控件即输入框（实时过滤，无独立搜索框）、组头纯标签不可选、条目点选 emit select、
// allowCustom 目录外入口（无精确命中才出现）、回车优先 custom、Esc/点外关闭恢复显示值、
// modelValue 高亮、unmatchedLabel 回显手输名、插槽。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import GroupedSelect from "../GroupedSelect.vue";
import type { ComboboxGroup } from "../Combobox.vue";

const GROUPS: ComboboxGroup[] = [
  {
    id: "openai",
    label: "OpenAI",
    note: "官方端点",
    items: [
      { label: "gpt-4o", value: "openai::gpt-4o", data: { provider: "openai", model: "gpt-4o" } },
      { label: "gpt-4o-mini", value: "openai::gpt-4o-mini", data: { provider: "openai", model: "gpt-4o-mini" } },
    ],
  },
  { id: "glm", label: "智谱", items: [{ label: "glm-5.2", value: "glm::glm-5.2" }] },
];

function mountSelect(modelValue = "", props: Record<string, unknown> = {}) {
  return mount(GroupedSelect, {
    props: { modelValue, groups: GROUPS, placeholder: "选择或输入模型名", ...props },
    attachTo: document.body,
  });
}

async function openDropdown(w: ReturnType<typeof mountSelect>) {
  await w.find(".gs-input").trigger("focus");
}

function inputValue(w: ReturnType<typeof mountSelect>) {
  return (w.find(".gs-input").element as HTMLInputElement).value;
}

describe("GroupedSelect 可选可输分组选择器", () => {
  it("控件即输入框：显示选中条目 label / unmatchedLabel / 空值走 placeholder，无独立搜索框", async () => {
    const w = mountSelect("openai::gpt-4o");
    expect(inputValue(w)).toBe("gpt-4o");
    expect(w.find(".gs-dropdown").exists()).toBe(false); // 未展开
    w.unmount();

    // modelValue 无条目命中（手输模型名）→ unmatchedLabel 回显
    const w2 = mountSelect("", { unmatchedLabel: "qwen3:8b" });
    expect(inputValue(w2)).toBe("qwen3:8b");
    w2.unmount();

    const w3 = mountSelect();
    expect(inputValue(w3)).toBe(""); // 空值由 placeholder 提示（CSS 层，不在 value 里）
    expect((w3.find(".gs-input").element as HTMLInputElement).placeholder).toBe("选择或输入模型名");
    w3.unmount();
  });

  it("focus 展开：无独立搜索框；组头纯标签（不可选）+ 组内条目", async () => {
    const w = mountSelect();
    await openDropdown(w);
    expect(w.find(".gs-dropdown").exists()).toBe(true);
    expect(w.find(".gs-search").exists()).toBe(false); // 搜索即控件输入本身
    const labels = w.findAll(".gs-group-label");
    expect(labels).toHaveLength(2);
    expect(labels[0].text()).toContain("OpenAI");
    expect(labels[0].text()).toContain("官方端点");
    expect(labels[0].element.tagName).toBe("DIV"); // optgroup 语义，非可交互元素
    expect(w.findAll(".gs-option")).toHaveLength(3);
    w.unmount();
  });

  it("输入实时过滤：条目命中只留命中项；组头命中保留整组", async () => {
    const w = mountSelect();
    await openDropdown(w);
    const input = w.find(".gs-input");
    await input.setValue("mini");
    expect(w.findAll(".gs-option")).toHaveLength(1);
    expect(w.findAll(".gs-option")[0].text()).toContain("gpt-4o-mini");
    await input.setValue("智谱");
    expect(w.findAll(".gs-option")).toHaveLength(1);
    expect(w.findAll(".gs-option")[0].text()).toContain("glm-5.2");
    w.unmount();
  });

  it("点条目：emit select 携带 data + update:modelValue，关闭下拉", async () => {
    const w = mountSelect();
    await openDropdown(w);
    await w.findAll(".gs-option")[0].trigger("click");
    const sel = w.emitted("select")!;
    expect(sel[0][0]).toMatchObject({ label: "gpt-4o", data: { provider: "openai", model: "gpt-4o" } });
    expect(w.emitted("update:modelValue")![0]).toEqual(["openai::gpt-4o"]);
    expect(w.find(".gs-dropdown").exists()).toBe(false);
    w.unmount();
  });

  it("focus 展开显示全目录（当前值只是回显不过滤），键入才开始过滤", async () => {
    const w = mountSelect("openai::gpt-4o"); // 已有选中值
    await openDropdown(w);
    expect(w.findAll(".gs-option")).toHaveLength(3); // 全目录，未被当前值过滤
    await w.find(".gs-input").setValue("glm");
    expect(w.findAll(".gs-option")).toHaveLength(1); // 键入后过滤（只命中 glm-5.2）
    w.unmount();
  });

  it("allowCustom：输入无精确命中出现「使用自定义模型」条目，点击 emit data.custom；精确命中则不出现（模糊命中保留为逃生口）", async () => {
    const w = mountSelect("", { allowCustom: true });
    await openDropdown(w);
    const input = w.find(".gs-input");
    // 目录外名字（Ollama 本地模型等）
    await input.setValue("qwen3:8b");
    const custom = w.find(".gs-option-custom");
    expect(custom.exists()).toBe(true);
    expect(custom.text()).toContain("qwen3:8b");
    await custom.trigger("click");
    expect(w.emitted("select")![0][0]).toMatchObject({ label: "qwen3:8b", data: { custom: true, model: "qwen3:8b" } });
    expect(w.find(".gs-dropdown").exists()).toBe(false);
    // 精确命中已有条目 → 不出现 custom 条目（防重复入口）
    const w2 = mountSelect("", { allowCustom: true });
    await openDropdown(w2);
    await w2.find(".gs-input").setValue("gpt-4o");
    expect(w2.find(".gs-option-custom").exists()).toBe(false);
    // 模糊命中（glm-5 ≠ glm-5.2）→ custom 条目保留（写这个名字的逃生口）
    const w3 = mountSelect("", { allowCustom: true });
    await openDropdown(w3);
    await w3.find(".gs-input").setValue("glm-5");
    expect(w3.find(".gs-option-custom").exists()).toBe(true);
    w.unmount();
    w2.unmount();
    w3.unmount();
  });

  it("无 allowCustom：目录外输入只显示「无匹配」，不出现 custom 条目", async () => {
    const w = mountSelect();
    await openDropdown(w);
    await w.find(".gs-input").setValue("nope-model");
    expect(w.find(".gs-option-custom").exists()).toBe(false);
    expect(w.find(".gs-empty").exists()).toBe(true);
    w.unmount();
  });

  it("Enter：目录命中优先选中；完全无命中才落 custom 兜底", async () => {
    // 有（模糊）命中 → 选第一个命中条目（custom 条目仍在但 Enter 不优先它）
    const w = mountSelect("", { allowCustom: true });
    await openDropdown(w);
    await w.find(".gs-input").setValue("glm-5");
    await w.find(".gs-input").trigger("keydown", { key: "Enter" });
    expect(w.emitted("select")![0][0]).toMatchObject({ label: "glm-5.2" });
    w.unmount();

    // 完全无命中 → custom 兜底（目录外名字回车即落地）
    const w2 = mountSelect("", { allowCustom: true });
    await openDropdown(w2);
    await w2.find(".gs-input").setValue("qwen3:8b");
    await w2.find(".gs-input").trigger("keydown", { key: "Enter" });
    expect(w2.emitted("select")![0][0]).toMatchObject({ data: { custom: true, model: "qwen3:8b" } });
    w2.unmount();
  });

  it("Esc / 点组件外部关闭：未落地的输入草稿丢弃，显示值恢复", async () => {
    const w = mountSelect("glm::glm-5.2");
    await openDropdown(w);
    await w.find(".gs-input").setValue("qwen3");
    await w.find(".gs-input").trigger("keydown", { key: "Escape" });
    expect(w.find(".gs-dropdown").exists()).toBe(false);
    expect(inputValue(w)).toBe("glm-5.2"); // 草稿丢弃，恢复当前选中显示

    await openDropdown(w);
    document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })); // 组件外
    await new Promise((r) => setTimeout(r, 0));
    expect(w.find(".gs-dropdown").exists()).toBe(false);
    expect(inputValue(w)).toBe("glm-5.2");
    w.unmount();
  });

  it("modelValue 高亮对应条目", async () => {
    const w = mountSelect("glm::glm-5.2");
    await openDropdown(w);
    const active = w.findAll(".gs-option.active");
    expect(active).toHaveLength(1);
    expect(active[0].text()).toContain("glm-5.2");
    w.unmount();
  });

  it("group-icon 插槽在组头渲染、control-icon 在控件渲染", async () => {
    const w = mount(GroupedSelect, {
      props: { modelValue: "", groups: GROUPS },
      slots: {
        "group-icon": '<svg class="icon-mark" />',
        "control-icon": '<span class="ctrl-mark" />',
      },
      attachTo: document.body,
    });
    expect(w.find(".gs-control .ctrl-mark").exists()).toBe(true); // 关闭态即可见
    await openDropdown(w);
    expect(w.findAll(".gs-group-label .icon-mark")).toHaveLength(2); // 每组头一个
    w.unmount();
  });
});
