// GroupedSelect.test.ts — 分组选择器（el-select+optgroup 风格）行为锁：
// 纯选择语义（关闭态无输入、组头纯标签不可选）、条目点选 emit select、
// 搜索过滤（条目命中/组头命中保留整组）、点外关闭、modelValue 高亮与回显、
// group-extra 组尾插槽。
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
  { id: "glm", label: "智谱 GLM", items: [{ label: "glm-5.2", value: "glm::glm-5.2" }] },
  { id: "custom", label: "自定义", items: [] },
];

function mountSelect(modelValue = "") {
  return mount(GroupedSelect, {
    props: { modelValue, groups: GROUPS, placeholder: "选择模型" },
    attachTo: document.body,
  });
}

async function openDropdown(w: ReturnType<typeof mountSelect>) {
  await w.find(".gs-control").trigger("click");
}

describe("GroupedSelect 分组选择器", () => {
  it("关闭态是 selector：显示选中条目/placeholder，无输入框", async () => {
    const w = mountSelect("openai::gpt-4o");
    expect(w.find(".gs-value").text()).toBe("gpt-4o");
    expect(w.find(".gs-search").exists()).toBe(false); // 未展开无搜索框
    const w2 = mountSelect();
    expect(w2.find(".gs-value").text()).toBe("选择模型");
    w.unmount();
    w2.unmount();
  });

  it("展开渲染分组：组头纯标签（不可选）+ 组内条目；空组可见", async () => {
    const w = mountSelect();
    await openDropdown(w);
    const labels = w.findAll(".gs-group-label");
    expect(labels).toHaveLength(3);
    expect(labels[0].text()).toContain("OpenAI");
    expect(labels[0].text()).toContain("官方端点");
    // 组头不是可交互元素（optgroup 语义）
    expect(labels[0].element.tagName).toBe("DIV");
    expect(w.findAll(".gs-option")).toHaveLength(3);
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

  it("搜索过滤：条目命中只留命中项；组头命中保留整组", async () => {
    const w = mountSelect();
    await openDropdown(w);
    const search = w.find(".gs-search");
    await search.setValue("mini");
    expect(w.findAll(".gs-option")).toHaveLength(1);
    expect(w.findAll(".gs-option")[0].text()).toContain("gpt-4o-mini");
    await search.setValue("智谱");
    expect(w.findAll(".gs-option")).toHaveLength(1);
    expect(w.findAll(".gs-option")[0].text()).toContain("glm-5.2");
    w.unmount();
  });

  it("点组件外部关闭下拉", async () => {
    const w = mountSelect();
    await openDropdown(w);
    expect(w.find(".gs-dropdown").exists()).toBe(true);
    document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })); // 组件外（body 上）
    await new Promise((r) => setTimeout(r, 0));
    expect(w.find(".gs-dropdown").exists()).toBe(false);
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

  it("group-extra 插槽在组尾渲染（供自定义组输入框等）", async () => {
    const w = mount(GroupedSelect, {
      props: { modelValue: "", groups: GROUPS },
      slots: { "group-extra": '<div class="slot-mark">extra</div>' },
      attachTo: document.body,
    });
    await openDropdown(w);
    expect(w.findAll(".slot-mark").length).toBeGreaterThanOrEqual(3); // 每组尾都渲染
    w.unmount();
  });

  it("group-icon 插槽在组头渲染、control-icon 在关闭态控件渲染", async () => {
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
    expect(w.findAll(".gs-group-label .icon-mark")).toHaveLength(3); // 每组头一个
    w.unmount();
  });
});
