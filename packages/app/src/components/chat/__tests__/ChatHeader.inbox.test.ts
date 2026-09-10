// ChatHeader.inbox.test.ts — MA-3 收件箱入口的渲染条件：
// 散落会话（未挂项目）结构性收不到投递（项目边界=同项目互投），入口隐藏防死 UI；
// 但 pending>0 恒显示——项目删除转散落后已扣的 hold 来件仍须有处置出口（防死信）。
import { describe, it, expect, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import ChatHeader from "../ChatHeader.vue";
import { useChatStore } from "../../../stores/chat";
import { useInbox } from "../../../composables/useInbox";

function conv(id: string, projectId: string | null) {
  return {
    id,
    agent_id: "a1",
    title: "测试对话",
    pinned: false,
    created_at: "2026-08-15 00:00:00",
    updated_at: "2026-08-15 00:00:00",
    project_id: projectId,
  };
}

/** 挂载并选中会话（微任务冲刷 watcher） */
async function mountWith(id: string, projectId: string | null) {
  const chat = useChatStore();
  chat.conversations = [conv(id, projectId)];
  chat.selectConversation(id);
  const wrapper = mount(ChatHeader, { attachTo: document.body });
  await Promise.resolve();
  await Promise.resolve();
  return wrapper;
}

describe("ChatHeader 收件箱入口渲染条件", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    document.body.innerHTML = "";
    // pendingCounts 是模块级单例 Map——清空防跨用例串扰
    useInbox().pendingCounts.clear();
  });

  it("挂项目的普通会话 → 入口渲染", async () => {
    const wrapper = await mountWith("c1", "p1");
    expect(wrapper.find(".inbox-zone").exists()).toBe(true);
  });

  it("散落会话（未挂项目）且无 pending → 入口隐藏（结构性收不到投递）", async () => {
    const wrapper = await mountWith("c1", null);
    expect(wrapper.find(".inbox-zone").exists()).toBe(false);
  });

  it("散落会话但有 pending → 入口恒显示（项目删除转散落后的 hold 扣件处置出口）", async () => {
    useInbox().pendingCounts.set("c1", 2);
    const wrapper = await mountWith("c1", null);
    expect(wrapper.find(".inbox-zone").exists()).toBe(true);
    expect(wrapper.find(".inbox-badge").text()).toBe("2");
  });

  it("委派子会话即使挂项目也不渲染（不是跨会话通讯单位）", async () => {
    const chat = useChatStore();
    chat.conversations = [{ ...conv("c1", "p1"), kind: "delegation" }];
    chat.selectConversation("c1");
    const wrapper = mount(ChatHeader, { attachTo: document.body });
    await Promise.resolve();
    expect(wrapper.find(".inbox-zone").exists()).toBe(false);
  });
});
