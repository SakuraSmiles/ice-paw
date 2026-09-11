// ChatHeader.channel.test.ts — 频道子标题形态（⑰：子标题即统筹位入口）：
// 新序 = 统筹者名（Shield）+ 成员头像叠层 +「共 N 名成员」，整体可点开频道成员浮层
// （原右侧统筹者图标方钮摘除）。归档频道只读——子标题纯文本无入口。
import { describe, it, expect, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import ChatHeader from "../ChatHeader.vue";
import { useChatStore } from "../../../stores/chat";
import type { ChannelView, Conversation } from "../../../types";

function channelConv(id: string, opts: { archived?: boolean } = {}): Conversation {
  return {
    id,
    agent_id: "a-coord",
    title: "频道测试项目",
    pinned: false,
    created_at: "2026-09-11 00:00:00",
    updated_at: "2026-09-11 00:00:00",
    project_id: "p1",
    kind: "channel",
    archived_at: opts.archived ? "2026-09-11 00:00:00" : null,
  };
}

/** 频道视图（6 成员默认；coordinator 指向首成员） */
function view(coordinatorId: string | null, memberCount = 6): ChannelView {
  const names = ["统筹甲", "成员乙", "成员丙", "成员丁", "成员戊", "成员己"];
  const members = Array.from({ length: memberCount }, (_, i) => ({
    agent_id: `ag-${i + 1}`,
    name: names[i],
    role: i === 0 ? "coordinator" : "member",
  }));
  return {
    channel: null,
    members,
    coordinator_agent_id: coordinatorId,
    coordinator_appointed: false,
  };
}

/** 挂载频道会话并注入 channelView——测试环境 invoke 恒 undefined，
 * selectConversation 的异步拉取微任务后会把 channelView 置空，故冲刷后手动赋值驱动渲染 */
async function mountChannel(opts: { archived?: boolean; coordinatorId?: string | null } = {}) {
  const chat = useChatStore();
  chat.conversations = [channelConv("c1", { archived: opts.archived })];
  chat.selectConversation("c1");
  const wrapper = mount(ChatHeader, { attachTo: document.body });
  await Promise.resolve();
  await Promise.resolve();
  if (!opts.archived) {
    chat.channelView = view(opts.coordinatorId === undefined ? "ag-1" : opts.coordinatorId);
    await nextTick();
  }
  return wrapper;
}

describe("ChatHeader 频道子标题（⑰ 子标题即统筹位入口）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    document.body.innerHTML = "";
  });

  it("新序渲染：统筹者名在前 + 头像叠层（前 4 + 溢出 +N）+「共 N 名成员」", async () => {
    const wrapper = await mountChannel();
    const btn = wrapper.find(".channel-subtitle-btn");
    expect(btn.exists()).toBe(true);
    const text = btn.text();
    expect(text).toContain("统筹甲");
    expect(text).toContain("共 6 名成员");
    // 顺序锁：统筹者名在成员计数之前（⑰ 调序——原序是头像+数量+统筹名）
    expect(text.indexOf("统筹甲")).toBeLessThan(text.indexOf("共 6 名成员"));
    expect(wrapper.findAll(".stack-avatar")).toHaveLength(4);
    expect(wrapper.find(".stack-more").text()).toBe("+2");
  });

  it("待选举：统筹位空缺 → 灰字「待选举」", async () => {
    const wrapper = await mountChannel({ coordinatorId: null });
    expect(wrapper.find(".channel-subtitle-btn").text()).toContain("待选举");
    expect(wrapper.find(".coord-pending").exists()).toBe(true);
  });

  it("点击子标题开浮层（aria-expanded 翻转）、再点关闭", async () => {
    const wrapper = await mountChannel();
    expect(wrapper.find(".channel-subtitle-btn").attributes("aria-expanded")).toBe("false");
    await wrapper.find(".channel-subtitle-btn").trigger("click");
    expect(wrapper.find(".channel-popover").exists()).toBe(true);
    expect(wrapper.find(".channel-subtitle-btn").attributes("aria-expanded")).toBe("true");
    await wrapper.find(".channel-subtitle-btn").trigger("click");
    expect(wrapper.find(".channel-popover").exists()).toBe(false);
  });

  it("Esc 关闭打开的成员浮层（useEscapeStack active 谓词让路）", async () => {
    const wrapper = await mountChannel();
    await wrapper.find(".channel-subtitle-btn").trigger("click");
    await Promise.resolve();
    expect(wrapper.find(".channel-popover").exists()).toBe(true);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await Promise.resolve();
    expect(wrapper.find(".channel-popover").exists()).toBe(false);
  });

  it("归档频道：子标题纯文本「已归档 · 记录只读」，无入口按钮", async () => {
    const wrapper = await mountChannel({ archived: true });
    expect(wrapper.find(".channel-archived-text").text()).toBe("已归档 · 记录只读");
    expect(wrapper.find(".channel-subtitle-btn").exists()).toBe(false);
  });

  it("右侧统筹者图标方钮已摘除（⑪ 版入口并入子标题）", async () => {
    const wrapper = await mountChannel();
    expect(wrapper.find(".coord-zone").exists()).toBe(false);
    expect(wrapper.find(".coord-btn").exists()).toBe(false);
  });
});
