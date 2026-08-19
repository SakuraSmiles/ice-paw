// EmojiPicker.test.ts — 冒烟：网格渲染策展集、点选 emit select、清除按钮 emit clear。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import EmojiPicker from "../EmojiPicker.vue";

describe("EmojiPicker", () => {
  it("渲染策展 emoji 网格（role=listbox + option 单元）", () => {
    const w = mount(EmojiPicker);
    const cells = w.findAll(".emoji-cell");
    expect(cells.length).toBeGreaterThanOrEqual(140);
    expect(w.find('[role="listbox"]').exists()).toBe(true);
  });

  it("点选 emoji → emit select（字符串载荷）", async () => {
    const w = mount(EmojiPicker);
    await w.findAll(".emoji-cell")[0].trigger("click");
    expect(w.emitted("select")).toEqual([["🦊"]]);
  });

  it("「不使用 emoji」→ emit clear", async () => {
    const w = mount(EmojiPicker);
    await w.find(".emoji-clear").trigger("click");
    expect(w.emitted("clear")).toEqual([[]]);
  });
});
