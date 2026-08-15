// ChatHeader.delete.test.ts — 复现手测 bug：删除按钮点击后不生效。
// 按真实交互时序驱动（click 事件冒泡到 document 的行为也一并还原）。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import ChatHeader from "../ChatHeader.vue";
import { useChatStore } from "../../../stores/chat";

function conv(id: string) {
  return {
    id,
    agent_id: "a1",
    title: "测试对话",
    pinned: false,
    created_at: "2026-08-15 00:00:00",
    updated_at: "2026-08-15 00:00:00",
    project_id: null,
  };
}

/** 真实事件派发（会冒泡到 document，还原浏览器时序——挂载在 body 上） */
function realClick(el: Element) {
  el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
}

/** 等微任务（Vue watcher flush 是微任务——document 监听在点击冒泡后才挂上） */
async function flushMicrotasks() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

describe("ChatHeader 删除确认条（手测 bug 复现）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    document.body.innerHTML = "";
  });

  it("点垃圾桶 → 确认条展开且不被同一次点击关闭；点「删除」→ 走 store 删除", async () => {
    const chat = useChatStore();
    chat.conversations = [conv("c1"), conv("c2")];
    chat.selectConversation("c1");
    const spy = vi.spyOn(chat, "deleteConversation").mockImplementation(async () => {});

    const wrapper = mount(ChatHeader, { attachTo: document.body });
    await flushMicrotasks();

    // 1) 点垃圾桶按钮（真实冒泡 click）
    const trash = document.querySelector<HTMLButtonElement>('[title="删除对话"]')!;
    expect(trash).toBeTruthy();
    realClick(trash);
    await flushMicrotasks();
    expect(wrapper.find(".confirm-bar").exists()).toBe(true); // 展开且未被同次点击关掉

    // 2) 点「删除」
    const danger = document.querySelector<HTMLButtonElement>(".confirm-btn-danger")!;
    realClick(danger);
    await flushMicrotasks();
    expect(spy).toHaveBeenCalledWith("c1");
  });

  it("确认条展开时点外部 → 收起", async () => {
    const chat = useChatStore();
    chat.conversations = [conv("c1")];
    chat.selectConversation("c1");
    const wrapper = mount(ChatHeader, { attachTo: document.body });
    await flushMicrotasks();

    realClick(document.querySelector('[title="删除对话"]')!);
    await flushMicrotasks();
    expect(wrapper.find(".confirm-bar").exists()).toBe(true);

    document.body.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await flushMicrotasks();
    expect(wrapper.find(".confirm-bar").exists()).toBe(false);
  });
});
