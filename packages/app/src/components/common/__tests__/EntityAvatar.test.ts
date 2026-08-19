// EntityAvatar — 两级兜底头像测试：image → 名字哈希渐变+首字
import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import EntityAvatar from "../EntityAvatar.vue";

describe("EntityAvatar", () => {
  it("一级：image 存在时渲染 img（object-fit cover）", () => {
    const w = mount(EntityAvatar, {
      props: { name: "架构师", image: "data:image/png;base64,xxx", size: "md" },
    });
    expect(w.find("img").exists()).toBe(true);
    expect(w.attributes("style") ?? "").not.toContain("linear-gradient");
  });

  it("二级：无 image 时名字哈希渐变 + 首字（CJK 安全）", () => {
    const w = mount(EntityAvatar, { props: { name: "架构师" } });
    expect(w.text()).toBe("架");
    expect(w.attributes("style")).toContain("linear-gradient");
  });

  it("哈希稳定：同名恒定同渐变、不同名大概率不同", () => {
    const a = mount(EntityAvatar, { props: { name: "架构师" } });
    const b = mount(EntityAvatar, { props: { name: "架构师" } });
    expect(a.attributes("style")).toBe(b.attributes("style"));
    const c = mount(EntityAvatar, { props: { name: "审阅者" } });
    expect(c.attributes("style")).not.toBe(a.attributes("style"));
  });

  it("accent 优先于哈希渐变作为底色（项目主题色）", () => {
    const w = mount(EntityAvatar, {
      props: { name: "x", accent: "#ff0000" },
    });
    expect(w.attributes("style")).toContain("#ff0000");
    expect(w.attributes("style")).not.toContain("linear-gradient");
  });

  it("img 加载失败降级到下一级（防脏 base64 白块）", async () => {
    const w = mount(EntityAvatar, {
      props: { name: "架构师", image: "data:image/png;base64,broken" },
    });
    expect(w.find("img").exists()).toBe(true);
    await w.find("img").trigger("error");
    expect(w.find("img").exists()).toBe(false);
    expect(w.text()).toBe("架");
  });

  it("换 image 重试（失败标记重置）", async () => {
    const w = mount(EntityAvatar, {
      props: { name: "架构师", image: "data:image/png;base64,broken" },
    });
    await w.find("img").trigger("error");
    expect(w.find("img").exists()).toBe(false);
    await w.setProps({ image: "data:image/png;base64,good" });
    expect(w.find("img").exists()).toBe(true);
  });

  it("尺寸 class 透传", () => {
    const w = mount(EntityAvatar, { props: { name: "x", size: "xs" } });
    expect(w.classes()).toContain("size-xs");
  });

  it("空名兜底不崩（首字 ?）", () => {
    const w = mount(EntityAvatar, { props: { name: "" } });
    expect(w.text()).toBe("?");
  });
});
