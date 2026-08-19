// MarkdownRenderer — 代码块体验测试：fence 容器结构（语言标签/复制按钮）、
// 折叠阈值与展开收起、复制委托、streaming 门控 class
import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import MarkdownRenderer from "../MarkdownRenderer.vue";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

const writeText = vi.fn().mockResolvedValue(undefined);
beforeEach(() => {
  writeText.mockClear();
  // happy-dom 的 navigator.clipboard 是 getter-only，须 defineProperty 覆盖
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText },
    configurable: true,
  });
});

/** n 行代码的 markdown fence 文本 */
function fence(lang: string, lines: number): string {
  const body = Array.from({ length: lines }, (_, i) => `line ${i + 1}`).join("\n");
  return "```" + lang + "\n" + body + "\n```";
}

describe("MarkdownRenderer 代码块", () => {
  it("fence 渲染为容器结构：语言标签 + 复制按钮 + data 属性", () => {
    const w = mount(MarkdownRenderer, { props: { content: fence("rust", 3) } });
    const block = w.find(".md-code-block");
    expect(block.exists()).toBe(true);
    expect(block.attributes("data-lang")).toBe("rust");
    expect(block.attributes("data-lines")).toBe("3");
    expect(w.find(".md-code-lang").text()).toBe("Rust");
    expect(w.find(".md-code-copy").text()).toBe("复制");
    expect(block.find("pre code").classes()).toContain("hljs");
    expect(block.find("pre code").classes()).toContain("language-rust");
  });

  it("语言别名映射（js→JavaScript）；无语言显示「代码」", () => {
    const js = mount(MarkdownRenderer, { props: { content: fence("js", 2) } });
    expect(js.find(".md-code-lang").text()).toBe("JavaScript");
    const none = mount(MarkdownRenderer, { props: { content: fence("", 2) } });
    expect(none.find(".md-code-lang").text()).toBe("代码");
    expect(none.find(".md-code-block").attributes("data-lang")).toBe("");
  });

  it("info 串带修饰词（```rust ignore）只取语言记号", () => {
    const w = mount(MarkdownRenderer, {
      props: { content: "```rust ignore\nfn a() {}\n```" },
    });
    expect(w.find(".md-code-block").attributes("data-lang")).toBe("rust");
  });

  it("超过 24 行默认折叠（collapsed + 展开按钮）；点按钮展开/收起", async () => {
    const w = mount(MarkdownRenderer, { props: { content: fence("ts", 30) } });
    const block = w.find(".md-code-block");
    expect(block.classes()).toContain("collapsed");
    const toggle = w.find(".md-code-toggle");
    expect(toggle.text()).toBe("展开 30 行");

    await toggle.trigger("click");
    expect(block.classes()).not.toContain("collapsed");
    expect(toggle.text()).toBe("收起");

    await toggle.trigger("click");
    expect(block.classes()).toContain("collapsed");
    expect(toggle.text()).toBe("展开 30 行");
  });

  it("不超过 24 行不折叠、无展开按钮", () => {
    const w = mount(MarkdownRenderer, { props: { content: fence("ts", 24) } });
    expect(w.find(".md-code-block").classes()).not.toContain("collapsed");
    expect(w.find(".md-code-toggle").exists()).toBe(false);
  });

  it("复制按钮：写原文入剪贴板并反馈「已复制」", async () => {
    vi.useFakeTimers();
    const w = mount(MarkdownRenderer, { props: { content: "```python\nprint('hi')\n```" } });
    await w.find(".md-code-copy").trigger("click");
    expect(writeText).toHaveBeenCalledWith("print('hi')\n");
    expect(w.find(".md-code-copy").text()).toBe("已复制");
    vi.advanceTimersByTime(2000);
    expect(w.find(".md-code-copy").text()).toBe("复制");
    vi.useRealTimers();
  });

  it("streaming prop 透传 md-streaming class（CSS 门控折叠的开关）", () => {
    // 模板根 = eslint 注释 + div（fragment），classes() 须落在 .markdown-body 上
    const live = mount(MarkdownRenderer, {
      props: { content: fence("ts", 30), streaming: true },
    });
    expect(live.find(".markdown-body").classes()).toContain("md-streaming");
    const done = mount(MarkdownRenderer, { props: { content: fence("ts", 30) } });
    expect(done.find(".markdown-body").classes()).not.toContain("md-streaming");
  });

  it("缩进代码块（非 fence）不进容器结构——老样式路径不受影响", () => {
    const w = mount(MarkdownRenderer, { props: { content: "    indented code" } });
    expect(w.find(".md-code-block").exists()).toBe(false);
    expect(w.find("pre").exists()).toBe(true);
  });
});
