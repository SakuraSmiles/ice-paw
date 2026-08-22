// EntityAvatar — 三级链头像测试（2026-08-22 全语境统一）：
// 用户图 → 内置默认头像图 → 名字哈希渐变+首字（遗留兜底，默认图也挂才出现）。
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import EntityAvatar from "../EntityAvatar.vue";

describe("EntityAvatar", () => {
  it("一级：image 存在时渲染用户图 img", () => {
    const w = mount(EntityAvatar, {
      props: { name: "架构师", image: "data:image/png;base64,xxx", size: "md" },
    });
    expect(w.find("img").attributes("src")).toBe("data:image/png;base64,xxx");
    expect(w.attributes("style") ?? "").not.toContain("linear-gradient");
  });

  it("二级：无 image 时走内置默认头像图（不再出首字）", () => {
    const w = mount(EntityAvatar, { props: { name: "架构师" } });
    expect(w.find("img").attributes("src")).toContain("default-agent-avatar");
    expect(w.text()).toBe(""); // 默认图是常规态，首字不出现
  });

  it("三级（遗留兜底）：用户图挂 → 默认图接棒；默认图也挂 → 首字+渐变", async () => {
    const w = mount(EntityAvatar, {
      props: { name: "架构师", image: "data:image/png;base64,broken" },
    });
    // 用户图加载失败 → src 切默认图（img :key 换新重试）
    await w.find("img").trigger("error");
    expect(w.find("img").attributes("src")).toContain("default-agent-avatar");
    // 默认图也失败（打包资产，理论不可达）→ 首字遗留兜底
    await w.find("img").trigger("error");
    expect(w.find("img").exists()).toBe(false);
    expect(w.text()).toBe("架");
    expect(w.attributes("style")).toContain("linear-gradient");
  });

  it("无图直达遗留兜底：默认图单次失败即出首字", async () => {
    const w = mount(EntityAvatar, { props: { name: "架构师" } });
    await w.find("img").trigger("error");
    expect(w.find("img").exists()).toBe(false);
    expect(w.text()).toBe("架");
  });

  it("哈希稳定：同名恒定同渐变、不同名大概率不同（遗留兜底档）", async () => {
    async function legacyBg(name: string) {
      const w = mount(EntityAvatar, { props: { name } });
      await w.find("img").trigger("error");
      return w.attributes("style");
    }
    const a = await legacyBg("架构师");
    const b = await legacyBg("架构师");
    expect(a).toBe(b);
    const c = await legacyBg("审阅者");
    expect(c).not.toBe(a);
  });

  it("accent 优先于哈希渐变作为兜底底色", async () => {
    const w = mount(EntityAvatar, {
      props: { name: "x", accent: "#ff0000" },
    });
    await w.find("img").trigger("error");
    expect(w.attributes("style")).toContain("#ff0000");
    expect(w.attributes("style")).not.toContain("linear-gradient");
  });

  it("换 image 重试（失败标记重置）", async () => {
    const w = mount(EntityAvatar, {
      props: { name: "架构师", image: "data:image/png;base64,broken" },
    });
    await w.find("img").trigger("error");
    expect(w.find("img").attributes("src")).toContain("default-agent-avatar");
    await w.setProps({ image: "data:image/png;base64,good" });
    expect(w.find("img").attributes("src")).toBe("data:image/png;base64,good");
  });

  it("尺寸 class 透传", () => {
    const w = mount(EntityAvatar, { props: { name: "x", size: "xs" } });
    expect(w.classes()).toContain("size-xs");
  });

  it("空名兜底不崩（首字 ?）", async () => {
    const w = mount(EntityAvatar, { props: { name: "" } });
    await w.find("img").trigger("error");
    expect(w.text()).toBe("?");
  });
});
