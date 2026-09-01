// Sidebar.rail.test.ts — 侧栏收起/展开（rail 模式，2026-09-01）行为锁定：
// ① 预置 collapsed → 首渲染即 rail 树 + 56px 内联宽 + 无调宽把手（setup 同步
//    读 localStorage，非 onMounted——无展开树闪帧）
// ② 点收起钮 → localStorage 落 "1"、rail 树接管；**.btn-theme-toggle 恒 ≤1**：
//    收起态不放主题钮（用户拍板，rail footer 只有设置），展开态 footer 那份即
//    全应用唯一实例——主题圆形扩散起点探测的不变式，最值得锁的回归点
// ③ 会话钮开 flyout → 搜索过滤 → 点项选中、flyout 关且搜索词清空
// ④ 展开钮还原：localStorage 落 "0"、宽度回 sidebarWidth（默认 320）
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import Sidebar from "../Sidebar.vue";
import { useChatStore } from "../../../stores/chat";
import type { Conversation } from "../../../types";

const mockInvoke = vi.mocked(invoke);
const push = vi.fn();

vi.mock("vue-router", () => ({
  useRouter: () => ({
    push,
    currentRoute: { value: { name: "Home", path: "/", fullPath: "/" } },
  }),
}));

// useTheme 的 getCurrentWindow().setTheme 在 happy-dom 下同步抛（无 Tauri internals）
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTheme: () => Promise.resolve() }),
}));

const COLLAPSED_KEY = "icepaw-sidebar-collapsed";

function conv(id: string, title: string): Conversation {
  return {
    id,
    agent_id: "ag-1",
    title,
    pinned: false,
    created_at: "2026-08-20 10:00:00",
    updated_at: "2026-08-20 10:00:00",
    project_id: null,
    kind: "chat",
  };
}

/** mount + 冲刷 onMounted 的三连 load（agent/project/conversations——invoke 统一
 * 返回 []，loadConversations 无 loaded 守卫会用返回值整替列表） */
async function mountSidebar() {
  const w = mount(Sidebar);
  await flushPromises();
  return w;
}

function seedConversations(list: Conversation[]) {
  // onMounted 的 loadConversations 已把列表整替为 []，种子须在其后注入
  const chat = useChatStore();
  chat.conversations = list;
}

describe("Sidebar 收起/展开（rail 模式）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    localStorage.clear();
    mockInvoke.mockReset();
    mockInvoke.mockResolvedValue([] as never);
    push.mockReset();
  });

  it("预置 collapsed：首渲染即 rail 树 + 56px 内联宽 + 无调宽把手", async () => {
    localStorage.setItem(COLLAPSED_KEY, "1");
    const w = await mountSidebar();
    expect(w.find(".sidebar-rail").exists()).toBe(true);
    // 展开树缺席（互斥）：会话列表/搜索/展开 footer 全不渲染
    expect(w.find(".conv-list").exists()).toBe(false);
    expect(w.find(".sidebar-search").exists()).toBe(false);
    expect(w.find(".sidebar-top").exists()).toBe(false);
    expect(w.find(".sidebar").attributes("style")).toContain("width: 56px");
    // 收起态无宽度可调：把手隐藏（sidebarWidth 值不动，展开即还原）
    expect(w.find(".panel-resize-handle").exists()).toBe(false);
    // 收起态不放主题钮（用户拍板 2026-09-01）：rail footer 只有设置钮
    expect(w.findAll(".btn-theme-toggle").length).toBe(0);
    expect(w.find(".rail-footer .btn-icon").exists()).toBe(true); // 设置钮在位
  });

  it("收起钮：localStorage 落 1、rail 树接管、主题钮归零（恒 ≤1 不变式）", async () => {
    const w = await mountSidebar();
    expect(w.findAll(".btn-theme-toggle").length).toBe(1); // 展开态 footer 那份

    await w.find('.btn-icon[title="收起侧边栏"]').trigger("click");
    expect(localStorage.getItem(COLLAPSED_KEY)).toBe("1");
    expect(w.find(".sidebar").classes()).toContain("collapsed");
    expect(w.find(".sidebar-rail").exists()).toBe(true);
    // 不变式：收起树无主题钮——全应用恒 ≤1 实例（若两态树同时渲染即破）
    expect(w.findAll(".btn-theme-toggle").length).toBe(0);
  });

  it("会话 flyout：开 → 搜索过滤 → 点项选中并关闭、搜索词清空", async () => {
    localStorage.setItem(COLLAPSED_KEY, "1");
    const w = await mountSidebar();
    seedConversations([conv("c1", "报告初稿"), conv("c2", "周会纪要")]);
    await flushPromises();

    await w.find('.btn-icon[title="会话列表"]').trigger("click");
    const menu = w.find(".flyout-menu");
    expect(menu.classes()).toContain("open");
    expect(w.findAll(".flyout-list .conv-item").length).toBe(2);

    // 搜索过滤走与展开列表同一 searchQuery（标题/agent 名匹配）
    await w.find(".flyout-search input").setValue("周会");
    expect(w.findAll(".flyout-list .conv-item").length).toBe(1);
    expect(w.find(".flyout-list .conv-name").text()).toBe("周会纪要");

    await w.findAll(".flyout-list .conv-item")[0].trigger("click");
    expect(menu.classes()).not.toContain("open");
    // 关即清搜索词（下次打开不残留上次过滤）
    expect((w.find(".flyout-search input").element as HTMLInputElement).value).toBe("");
    // 点项 = 关 flyout + 走 selectConv 选中
    expect(useChatStore().activeConvId).toBe("c2");
  });

  it("展开钮：localStorage 落 0、宽度还原 sidebarWidth（默认 320）", async () => {
    localStorage.setItem(COLLAPSED_KEY, "1");
    const w = await mountSidebar();
    await w.find('.btn-icon[title="展开侧边栏"]').trigger("click");
    expect(localStorage.getItem(COLLAPSED_KEY)).toBe("0");
    expect(w.find(".sidebar-rail").exists()).toBe(false);
    expect(w.find(".conv-list").exists()).toBe(true);
    expect(w.find(".sidebar").attributes("style")).toContain("width: 320px");
    // 展开态把手回归
    expect(w.find(".panel-resize-handle").exists()).toBe(true);
  });
});
