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
import { bridge } from "../../../api/bridge";

const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);

vi.mock("../../../api/bridge", () => {
  // trajectory 快照可注入（计划折叠用例）；mock 工厂 hoist，state 须在工厂内自持
  const trajectoryState: { plan: unknown } = { plan: null };
  return {
    bridge: {
      conversations: {
        listAll: async () => mockInvoke("list_all_conversations") as never,
      },
      trajectory: {
        currentPlan: async () => trajectoryState.plan as never,
        __setPlanForTest: (p: unknown) => { trajectoryState.plan = p; },
      },
    },
  };
});

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
    (bridge.trajectory as unknown as { __setPlanForTest: (p: unknown) => void }).__setPlanForTest(null);
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
    expect(wrapper.find(".task-pill-label").text()).toBe("任务 1");
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

  it("任务 running 翻转：行背景轻闪 .just-changed，约 1.1s 后褪去（P12）", async () => {
    vi.useFakeTimers();
    try {
      const handlers = captureHandlers();
      const chat = useChatStore();
      await useChatEvents();

      chat.conversations = [conv(PARENT)];
      chat.selectConversation(PARENT);
      const wrapper = mountPanel();

      mockInvoke.mockResolvedValue([
        conv(CHILD, { kind: "delegation", parent_conversation_id: PARENT }),
        conv(PARENT),
      ]);
      handlers.get("chat:delegation-started")!({ payload: {} });
      await flushAsync();
      // 进入 running（false→true 翻转本身会闪一次，且触发自动展开）——
      // 先走完闪动窗口
      handlers.get("chat:chunk")!({ payload: { conversation_id: CHILD, delta: "x" } });
      await flushAsync();
      vi.advanceTimersByTime(1200);
      await flushAsync();

      // 面板行存在且无闪（running 到来时已自动展开；未展开则手动点开——
      // 注意点击是 toggle，已展开时再点会关上）
      if (!wrapper.find(".task-popover").exists()) {
        await wrapper.find(".task-pill").trigger("click");
        await flushAsync();
      }
      expect(wrapper.find(".task-row").classes()).not.toContain("just-changed");

      // running→结束翻转：bgStreams 整替清空 → 行背景轻闪
      chat.bgStreams = new Map();
      await flushAsync();
      expect(wrapper.find(".task-row").classes()).toContain("just-changed");

      // ~1.1s 后自动褪去
      vi.advanceTimersByTime(1200);
      await flushAsync();
      expect(wrapper.find(".task-row").classes()).not.toContain("just-changed");
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("TaskPanel 规模治理（任务截断 / 计划 done 折叠）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset();
    mockListen.mockReset();
    mockInvoke.mockResolvedValue([]);
    (bridge.trajectory as unknown as { __setPlanForTest: (p: unknown) => void }).__setPlanForTest(null);
    document.body.innerHTML = "";
  });

  it("任务 10 个（全非 running）：非 running 最多 6 行 + 「还有 4 个任务」计数行", async () => {
    const handlers = captureHandlers();
    const chat = useChatStore();
    await useChatEvents();

    chat.conversations = [conv(PARENT)];
    chat.selectConversation(PARENT);
    const wrapper = mountPanel();

    // 10 个已结束的委派任务
    mockInvoke.mockResolvedValue([
      ...Array.from({ length: 10 }, (_, i) =>
        conv(`child-${i}`, { kind: "delegation", parent_conversation_id: PARENT })),
      conv(PARENT),
    ]);
    handlers.get("chat:delegation-started")!({ payload: {} });
    await flushAsync();

    await wrapper.find(".task-pill").trigger("click");
    await flushAsync();
    expect(wrapper.findAll(".task-row")).toHaveLength(6);
    expect(wrapper.find(".task-more").text()).toBe("还有 4 个任务");
    // 胶囊计数仍是全量 10
    expect(wrapper.find(".task-pill-label").text()).toBe("任务 10");
  });

  it("计划 2 活跃 + 5 done：活跃展开、done 收「已完成 5」，点击展开/收起", async () => {
    const handlers = captureHandlers();
    const chat = useChatStore();
    await useChatEvents();

    chat.conversations = [conv(PARENT)];
    chat.selectConversation(PARENT);
    const wrapper = mountPanel();

    (bridge.trajectory as unknown as { __setPlanForTest: (p: unknown) => void }).__setPlanForTest({
      conversation_id: PARENT,
      items: [
        { text: "步骤一", status: "done", task_conversation_id: null },
        { text: "步骤二", status: "done", task_conversation_id: null },
        { text: "步骤三", status: "in_progress", task_conversation_id: null },
        { text: "步骤四", status: "done", task_conversation_id: null },
        { text: "步骤五", status: "pending", task_conversation_id: null },
        { text: "步骤六", status: "done", task_conversation_id: null },
        { text: "步骤七", status: "done", task_conversation_id: null },
      ],
      updated_at: "2026-08-17 00:00:00",
    });
    // 真实链路：plan_updated 事件（session:event-appended）→ loadPlan → 面板出现
    handlers.get("session:event-appended")!({ payload: { kind: "plan_updated", conversation_id: PARENT } });
    await flushAsync();

    await wrapper.find(".task-pill").trigger("click");
    await flushAsync();

    // 活跃 2 行展开；done 收进折叠行
    const rows = wrapper.findAll(".task-plan-row");
    expect(rows).toHaveLength(7); // v-show 常驻 DOM，断言可见性
    const visibleTexts = rows.filter((r) => r.isVisible()).map((r) => r.text());
    expect(visibleTexts).toContain("步骤三");
    expect(visibleTexts).toContain("步骤五");
    expect(visibleTexts.filter((t) => t.includes("步骤一"))).toHaveLength(0); // done 藏起
    expect(wrapper.find(".plan-done-toggle").text()).toContain("已完成 5");

    // 展开：done 7 行全部可见
    await wrapper.find(".plan-done-toggle").trigger("click");
    await flushAsync();
    expect(wrapper.findAll(".task-plan-row").filter((r) => r.isVisible())).toHaveLength(7 + 0); // 2 活跃 + 5 done（toggle 行非 plan-row）

    // 收起回折叠
    await wrapper.find(".plan-done-toggle").trigger("click");
    await flushAsync();
    expect(wrapper.findAll(".task-plan-row").filter((r) => r.isVisible())).toHaveLength(2);
  });
});
