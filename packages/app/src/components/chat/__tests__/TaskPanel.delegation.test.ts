// TaskPanel.delegation.test.ts — 复现手测 bug：新会话从无到有委派，任务胶囊
// 不切会话不出现。按真实时序驱动（事件层 handler → store → 组件渲染），
// 锁住「委派启动即见胶囊」与「running 迟到仍自动展开」两条行为。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import TaskPanel from "../TaskPanel.vue";
import { useChatStore } from "../../../stores/chat";
import { useChatEvents } from "../../../composables/useChatEvents";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);

vi.mock("../../../api/bridge", () => ({
  bridge: {
    conversations: {
      listAll: async () => mockInvoke("list_all_conversations") as never,
    },
    trajectory: {
      currentPlan: async () => null,
    },
  },
}));

function captureHandlers() {
  const handlers = new Map<string, (event: { payload: unknown }) => void>();
  mockListen.mockImplementation(async (event, handler) => {
    handlers.set(event, handler as (event: { payload: unknown }) => void);
    return () => { handlers.delete(event); };
  });
  return handlers;
}

const PARENT = "parent-conv";
const CHILD = "child-conv";

function conv(id: string, overrides?: Record<string, unknown>) {
  return {
    id,
    agent_id: "a1",
    title: "t",
    pinned: false,
    created_at: "2026-08-15 00:00:00",
    updated_at: "2026-08-15 00:00:00",
    project_id: null,
    ...overrides,
  };
}

/** 与 ChatPage 一致的挂载形态：胶囊在 tabbar 内，随 activeConvId 存在而挂载 */
function mountPanel() {
  return mount(TaskPanel, { attachTo: document.body });
}

async function flushAsync() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

describe("TaskPanel 委派时序（手测 bug 复现）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset();
    mockListen.mockReset();
    mockInvoke.mockResolvedValue([]);
    document.body.innerHTML = "";
  });

  it("新会话从无到有：delegation-started 刷新列表后胶囊立即出现", async () => {
    const handlers = captureHandlers();
    const chat = useChatStore();
    await useChatEvents();

    // 新会话：store 已有父会话并激活（TaskPanel 此时挂载，快照空）
    chat.conversations = [conv(PARENT)];
    chat.selectConversation(PARENT);
    const wrapper = mountPanel();
    expect(wrapper.find(".task-panel").exists()).toBe(false); // 从无到有前零占用

    // 委派启动：事件 → loadConversations → 列表含子会话
    mockInvoke.mockResolvedValue([
      conv(CHILD, { kind: "delegation", parent_conversation_id: PARENT }),
      conv(PARENT),
    ]);
    handlers.get("chat:delegation-started")!({ payload: {} });
    await flushAsync();

    expect(wrapper.find(".task-panel").exists()).toBe(true);
    expect(wrapper.find(".task-pill-count").text()).toBe("1");
  });

  it("running 迟到：子会话首个 chunk 进 bgStreams 时 popover 自动展开一次", async () => {
    const handlers = captureHandlers();
    const chat = useChatStore();
    await useChatEvents();

    chat.conversations = [conv(PARENT)];
    chat.selectConversation(PARENT);
    const wrapper = mountPanel();

    // 委派启动（此刻子会话尚未流出任何 chunk → running=false）
    mockInvoke.mockResolvedValue([
      conv(CHILD, { kind: "delegation", parent_conversation_id: PARENT }),
      conv(PARENT),
    ]);
    handlers.get("chat:delegation-started")!({ payload: {} });
    await flushAsync();
    expect(wrapper.find(".task-panel").exists()).toBe(true);

    // 子会话开始流式：后台 chunk → bgStreams → streamingConvIds 含子会话
    handlers.get("chat:chunk")!({
      payload: { conversation_id: CHILD, delta: "x" },
    });
    await flushAsync();

    expect(chat.streamingConvIds.has(CHILD)).toBe(true);
    expect(wrapper.find(".task-popover").exists()).toBe(true); // 自动展开
  });
});
