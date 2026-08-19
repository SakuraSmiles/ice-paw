// ProjectBasicForm.icon.test.ts — 图标与颜色行读写锁：
// emoji 弹层选择 patch 单对象 v-model（图片互斥清空）、主题色 swatch 落值/无、
// 清除双归 null、EntityAvatar 预览三级链；附 emojiFromIcon 的 icon 列语义。
import { describe, it, expect, afterEach, vi } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import ProjectBasicForm from "../ProjectBasicForm.vue";
import { emojiFromIcon } from "../../../utils/avatar";

// plugin-dialog 顶层 import 须先 mock
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

const wrappers: VueWrapper[] = [];

function mountForm(overrides?: Partial<{
  name: string; avatar: string | null; emoji: string | null; themeColor: string | null;
}>) {
  const w = mount(ProjectBasicForm, {
    props: {
      modelValue: {
        name: overrides?.name ?? "测试项目",
        description: "",
        workspacePath: "",
        avatar: overrides?.avatar ?? null,
        emoji: overrides?.emoji ?? null,
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
  avatar: string | null; emoji: string | null; themeColor: string | null;
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

  it("emoji 弹层选择：patch emoji 且图片互斥清空", async () => {
    const w = mountForm({ avatar: "data:image/webp;base64,old" });
    // 初始预览为图片档
    expect(w.find(".icon-row .entity-avatar img").attributes("src")).toBe("data:image/webp;base64,old");

    await w.findAll(".icon-actions .icon-btn")[1].trigger("click"); // 选 emoji
    await w.find(".emoji-pop .emoji-cell").trigger("click");

    const p = lastPatch(w);
    expect(p.emoji).toBe("🦊");
    expect(p.avatar).toBeNull(); // 互斥
  });

  it("「不使用 emoji」→ emoji 归 null", async () => {
    const w = mountForm({ emoji: "🚀" });
    await w.findAll(".icon-actions .icon-btn")[1].trigger("click");
    await w.find(".emoji-pop .emoji-clear").trigger("click");
    expect(lastPatch(w).emoji).toBeNull();
  });

  it("清除按钮：图片 + emoji 双归 null", async () => {
    const w = mountForm({ avatar: "data:image/webp;base64,x" });
    await w.findAll(".icon-actions .icon-btn")[2].trigger("click"); // 第三个=清除
    const p = lastPatch(w);
    expect(p.avatar).toBeNull();
    expect(p.emoji).toBeNull();
  });

  it("主题色 swatch：点选落值 / 「无」清空；预览底色随 accent", async () => {
    const w = mountForm();
    // 初始「无」态高亮
    expect(w.find(".swatch-none").classes()).toContain("active");

    await w.findAll(".swatch")[0].trigger("click");
    expect(lastPatch(w).themeColor).toBe("#4680C2");
    // 预览头像底色 = accent（emoji/兜底档共用）
    expect(w.find(".icon-row .entity-avatar").attributes("style")).toContain("#4680C2");

    await w.find(".swatch-none").trigger("click");
    expect(lastPatch(w).themeColor).toBeNull();
  });

  it("emoji 预览档：选定 emoji 直接显示字符", () => {
    const w = mountForm({ emoji: "🧊" });
    expect(w.find(".icon-row .entity-avatar").text()).toBe("🧊");
  });
});

describe("emojiFromIcon（projects.icon 列语义）", () => {
  it("emoji 字符（含非 ASCII）原样返回", () => {
    expect(emojiFromIcon("🚀")).toBe("🚀");
    expect(emojiFromIcon("🧊")).toBe("🧊");
  });
  it("历史图标名（纯 ASCII）与空值返回 null（走渐变兜底）", () => {
    expect(emojiFromIcon("folder")).toBeNull();
    expect(emojiFromIcon("star")).toBeNull();
    expect(emojiFromIcon(null)).toBeNull();
    expect(emojiFromIcon(undefined)).toBeNull();
    expect(emojiFromIcon("")).toBeNull();
  });
});
