// ProviderIcon.test.ts — 品牌 glyph 渲染规则锁：已知 provider 出 24×24
// currentColor 单色 glyph、别名厂商共用 glyph、custom 走 stroke 拼图、
// 未知 provider 渲染为空（注册表新增无图标条目不破版式）。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import ProviderIcon from "../ProviderIcon.vue";

describe("ProviderIcon 品牌 glyph", () => {
  it("已知 provider 渲染 24×24 fill glyph（currentColor 单色，随主题）", () => {
    const w = mount(ProviderIcon, { props: { name: "openai" } });
    const svg = w.find("svg.provider-icon");
    expect(svg.exists()).toBe(true);
    expect(svg.attributes("viewBox")).toBe("0 0 24 24");
    expect(svg.attributes("fill")).toBe("currentColor");
    expect(svg.attributes("width")).toBe("14"); // 默认尺寸
    expect(svg.find("path").attributes("d")).toBeTruthy();
  });

  it("别名厂商共用 glyph：glm-coding→智谱、minimax-cn→MiniMax", () => {
    for (const name of ["glm", "glm-coding", "minimax", "minimax-cn"]) {
      const w = mount(ProviderIcon, { props: { name } });
      expect(w.find("svg.provider-icon").exists(), name).toBe(true);
      expect(w.find("svg").attributes("stroke")).toBeUndefined(); // fill 形态非 stroke
    }
  });

  it("custom 渲染 stroke 拼图；未知 provider 渲染为空", () => {
    const custom = mount(ProviderIcon, { props: { name: "custom" } });
    expect(custom.find("svg").attributes("stroke")).toBe("currentColor");
    expect(mount(ProviderIcon, { props: { name: "nope" } }).find("svg").exists()).toBe(false);
  });
});
