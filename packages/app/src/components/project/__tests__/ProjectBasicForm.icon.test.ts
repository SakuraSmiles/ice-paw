// ProjectBasicForm.icon.test.ts — 图标与颜色行读写锁：
// 图片预览/清除归 null patch 单对象 v-model、主题色 swatch 落值/无、
// EntityAvatar 预览两级链。
import { describe, it, expect, afterEach, vi } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import ProjectBasicForm from "../ProjectBasicForm.vue";

// plugin-dialog 顶层 import 须先 mock
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const wrappers: VueWrapper[] = [];

function mountForm(overrides?: Partial<{
  name: string; avatar: string | null; themeColor: string | null;
}>) {
  const w = mount(ProjectBasicForm, {
    props: {
      modelValue: {
        name: overrides?.name ?? "测试项目",
        description: "",
        workspacePath: "",
        avatar: overrides?.avatar ?? null,
        themeColor: overrides?.themeColor ?? null,
      },
    },
  });
  wrappers.push(w);
  return w;
}

/** 最新一次 update:modelValue 载荷 */
function lastPatch(w: VueWrapper): {
  name: string; description: string; workspacePath: string;
  avatar: string | null; themeColor: string | null;
} {
  const emitted = w.emitted("update:modelValue");
  return emitted?.[emitted.length - 1]?.[0] as ReturnType<typeof lastPatch>;
}

describe("ProjectBasicForm 图标与颜色", () => {
  afterEach(() => {
    for (const w of wrappers) w.unmount();
    wrappers.length = 0;
    document.body.innerHTML = "";
  });

  it("清空×：图片归 null；预览两级链（图片档 img / 无图首字渐变）", async () => {
    const w = mountForm({ avatar: "data:image/webp;base64,x" });
    expect(w.find(".avatar-field .entity-avatar img").attributes("src")).toBe("data:image/webp;base64,x");

    await w.find(".af-clear").trigger("click"); // AvatarField 右上清空钮
    const p = lastPatch(w);
    expect(p.avatar).toBeNull();

    const w2 = mountForm();
    expect(w2.find(".avatar-field .entity-avatar img").exists()).toBe(false);
    expect(w2.find(".avatar-field .entity-avatar").text()).toBe("测");
  });

  it("主题色 swatch：点选落值 / 「无」清空；预览底色随 accent", async () => {
    const w = mountForm();
    // 初始「无」态高亮
    expect(w.find(".swatch-none").classes()).toContain("active");

    await w.findAll(".swatch")[0].trigger("click");
    expect(lastPatch(w).themeColor).toBe("#4680C2");
    // 预览头像底色 = accent（兜底档共用）
    expect(w.find(".avatar-field .entity-avatar").attributes("style")).toContain("#4680C2");

    await w.find(".swatch-none").trigger("click");
    expect(lastPatch(w).themeColor).toBeNull();
  });
});
