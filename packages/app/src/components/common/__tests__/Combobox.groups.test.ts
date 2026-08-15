// Combobox.groups.test.ts — groups 分组形态（模型目录）行为锁：
// 组头/条目渲染、组头点选、条目点选只发 select（不双发 update）、
// 手输原文透传、组头命中保留整组、受控回写不清 filter（边输边筛）。
import { describe, it, expect } from "vitest";
import { ref } from "vue";
import { mount } from "@vue/test-utils";
import Combobox, { type ComboboxGroup } from "../Combobox.vue";

const GROUPS: ComboboxGroup[] = [
  {
    label: "OpenAI",
    note: "官方端点",
    headerValue: "openai",
    items: [
      { label: "gpt-4o", value: "openai::gpt-4o", data: { provider: "openai", model: "gpt-4o" } },
      { label: "gpt-4o-mini", value: "openai::gpt-4o-mini", data: { provider: "openai", model: "gpt-4o-mini" } },
    ],
  },
  { label: "自定义（OpenAI 兼容）", note: "必填 API URL", headerValue: "custom", items: [] },
];

function mountGroups(modelValue = "") {
  return mount(Combobox, {
    props: { modelValue, groups: GROUPS, placeholder: "选择或输入模型" },
    attachTo: document.body,
  });
}

describe("Combobox groups 形态", () => {
  it("分组渲染：可选组头（含 note）+ 组内条目；空组仅组头可见", async () => {
    const w = mountGroups();
    await w.find("input").trigger("focus");
    const heads = w.findAll(".combobox-group-head");
    expect(heads).toHaveLength(2);
    expect(heads[0].text()).toContain("OpenAI");
    expect(heads[0].text()).toContain("官方端点");
    expect(w.findAll(".combobox-group-item")).toHaveLength(2);
    w.unmount();
  });

  it("点组头：emit select（isHeader 标记 + 组名），不 emit update", async () => {
    const w = mountGroups();
    await w.find("input").trigger("focus");
    await w.findAll(".combobox-group-head")[1].trigger("click");
    const sel = w.emitted("select");
    expect(sel).toBeTruthy();
    expect(sel![0][0]).toMatchObject({ label: "自定义（OpenAI 兼容）", value: "custom", data: { isHeader: true } });
    expect(w.emitted("update:modelValue")).toBeFalsy();
    w.unmount();
  });

  it("点条目：emit select 携带 data 负载；输入框显示 label；不 emit update", async () => {
    const w = mountGroups();
    await w.find("input").trigger("focus");
    await w.findAll(".combobox-group-item")[0].trigger("click");
    const sel = w.emitted("select")!;
    expect(sel[0][0]).toMatchObject({ label: "gpt-4o", value: "openai::gpt-4o", data: { provider: "openai", model: "gpt-4o" } });
    expect(w.emitted("update:modelValue")).toBeFalsy();
    expect((w.find("input").element as HTMLInputElement).value).toBe("gpt-4o");
    w.unmount();
  });

  it("手输：原文透传 update:modelValue，不触发 select", async () => {
    const w = mountGroups();
    const input = w.find("input");
    await input.trigger("focus");
    await input.setValue("qwen3:8b");
    const emitted = w.emitted("update:modelValue")!;
    expect(emitted[emitted.length - 1]).toEqual(["qwen3:8b"]);
    expect(w.emitted("select")).toBeFalsy();
    w.unmount();
  });

  it("组头命中过滤 → 整组条目保留；条目命中 → 只显示命中项", async () => {
    const w = mountGroups();
    const input = w.find("input");
    await input.trigger("focus");
    await input.setValue("OpenAI");
    expect(w.findAll(".combobox-group-item")).toHaveLength(2);
    await input.setValue("mini");
    const hits = w.findAll(".combobox-group-item");
    expect(hits).toHaveLength(1);
    expect(hits[0].text()).toContain("gpt-4o-mini");
    w.unmount();
  });

  it("modelValue 高亮对应条目并显示其 label", async () => {
    const w = mountGroups("openai::gpt-4o");
    expect((w.find("input").element as HTMLInputElement).value).toBe("gpt-4o");
    await w.find("input").trigger("focus");
    const active = w.findAll(".combobox-group-item.active");
    expect(active).toHaveLength(1);
    expect(active[0].text()).toContain("gpt-4o");
    w.unmount();
  });

  it("受控 v-model 回写不清 filter：边输边筛保持有效", async () => {
    const bound = ref("");
    const w = mount(Combobox, {
      props: { modelValue: bound.value, groups: GROUPS, "onUpdate:modelValue": (v: string) => { bound.value = v; } },
      attachTo: document.body,
    });
    const input = w.find("input");
    await input.trigger("focus");
    await input.setValue("gp");
    await w.setProps({ modelValue: bound.value });
    // 回写后过滤条件仍在：mini 不含 "gp"，只剩 gpt-4o（label 前缀命中）
    expect(w.findAll(".combobox-group-item")).toHaveLength(2);
    await input.setValue("gpt-4o-m");
    await w.setProps({ modelValue: bound.value });
    expect(w.findAll(".combobox-group-item")).toHaveLength(1);
    w.unmount();
  });
});
