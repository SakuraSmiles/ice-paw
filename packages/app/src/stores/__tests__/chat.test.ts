import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useChatStore } from "../chat";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);

/** 捕获 listen() 注册的 handler，以便测试中手动触发事件 */
function captureHandlers() {
  const handlers = new Map<string, (event: { payload: unknown }) => void>();
  // handler 推断为 Tauri 真实类型 EventCallback<unknown>；测试侧按 {payload} 简化调用，
  // 故存入 map 时窄化为 map 值类型（无 any，满足 no-explicit-any）。
  mockListen.mockImplementation(async (event, handler) => {
    handlers.set(event, handler as (event: { payload: unknown }) => void);
    return () => { handlers.delete(event); };
  });
  return handlers;
}

/** 工具函数：构造最小 Conversation */
function fakeConv(id: string, overrides?: Partial<ReturnType<typeof useChatStore>["conversations"][number]>) {
  return {
    id,
    agent_id: "a1",
    title: "测试对话",
    pinned: false,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    project_id: null,
    ...overrides,
  };
}

describe("chatStore", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke.mockReset();
    mockListen.mockReset();
    mockInvoke.mockResolvedValue([]);
    vi.useRealTimers();
  });

  describe("initEvents", () => {
    it("registers all event listeners and is idempotent", async () => {
      mockListen.mockResolvedValue(() => {});
      const store = useChatStore();

      await store.initEvents();
      const callCount = mockListen.mock.calls.length;
      expect(callCount).toBeGreaterThan(0);

      // 第二次调用先 destroyEvents 再重新 subscribe，不会泄漏
      await store.initEvents();
      expect(mockListen.mock.calls.length).toBe(callCount * 2);
    });
  });

  describe("conversations", () => {
    it("loadConversations fetches and populates list", async () => {
      mockInvoke.mockResolvedValueOnce([fakeConv("c1")]);

      const store = useChatStore();
      await store.loadConversations();

      expect(store.conversations).toHaveLength(1);
      expect(store.conversations[0].title).toBe("测试对话");
    });

    it("createConversation calls bridge and adds to list", async () => {
      const newConv = fakeConv("new-c1");
      mockInvoke.mockResolvedValueOnce(newConv);

      const store = useChatStore();
      const result = await store.createConversation("a1");

      expect(result.id).toBe("new-c1");
      expect(mockInvoke).toHaveBeenCalledWith(
        "create_conversation",
        { input: { agent_id: "a1", title: undefined, project_id: null } },
      );
    });

    it("deleteConversation removes from list and jumps to next conv", async () => {
      mockInvoke.mockResolvedValue(undefined);
      const store = useChatStore();
      store.conversations = [fakeConv("c1"), fakeConv("c2", { title: "T2" })];
      store.activeConvId = "c1";

      await store.deleteConversation("c1");

      expect(store.conversations).toHaveLength(1);
      expect(store.activeConvId).toBe("c2");
    });

    it("pinConversation toggles pinned state", async () => {
      mockInvoke.mockResolvedValue(undefined);
      const store = useChatStore();
      store.conversations = [fakeConv("c1")];

      await store.pinConversation("c1", true);
      expect(store.conversations[0].pinned).toBe(true);

      await store.pinConversation("c1", false);
      expect(store.conversations[0].pinned).toBe(false);
    });
  });

  describe("undoDelete", () => {
    it("undoDeleteConversation restores removed conversation", async () => {
      mockInvoke.mockResolvedValue(undefined);
      const store = useChatStore();
      store.conversations = [fakeConv("c1"), fakeConv("c2")];
      store.activeConvId = "c1";

      await store.deleteConversation("c1");
      expect(store.conversations).toHaveLength(1);

      const restored = store.undoDeleteConversation("c1");
      expect(restored).toBe(true);
      expect(store.conversations).toHaveLength(2);
    });

    it("undoDeleteConversation returns false for unknown id", () => {
      const store = useChatStore();
      expect(store.undoDeleteConversation("nope")).toBe(false);
    });
  });

  describe("streaming state machine", () => {
    /** 设置活跃会话 + 注入 mock handlers + 显式注册事件监听 */
    async function setupStream() {
      const handlers = captureHandlers();
      const store = useChatStore();
      store.conversations = [fakeConv("c1")];
      store.activeConvId = "c1";
      // 显式调用 initEvents：App.vue 负责在生产环境初始化，测试中手动触发
      await store.initEvents();
      return { store, handlers };
    }

    it("single-turn: chat:start → chat:chunk → chat:done accumulates text and resets", async () => {
      const { store, handlers } = await setupStream();

      // 模拟 sendMessage 开始
      store.sending = true;
      store.streamingText = "";

      // chat:start — 追加空 assistant 占位
      const startH = handlers.get("chat:start")!;
      startH({ payload: { conversation_id: "c1", assistant_message_id: "asst-1" } });
      expect(store.messages).toHaveLength(1);
      expect(store.messages[0].role).toBe("assistant");

      // chat:chunk x3 — 累积文本
      const chunkH = handlers.get("chat:chunk")!;
      chunkH({ payload: { conversation_id: "c1", delta: "Hello" } });
      chunkH({ payload: { conversation_id: "c1", delta: ", " } });
      chunkH({ payload: { conversation_id: "c1", delta: "World!" } });
      expect(store.streamingText).toBe("Hello, World!");
      expect(store.messages[0].content).toBe("Hello, World!");

      // chat:done — 收尾
      const doneH = handlers.get("chat:done")!;
      doneH({ payload: { conversation_id: "c1", message_id: "asst-1", finish_reason: "stop", usage: null } });
      expect(store.sending).toBe(false);
      expect(store.streamingText).toBe("");
      expect(store.lastFinishReason).toBe("stop");
    });

    it("tool call lifecycle: start → delta → end → result", async () => {
      const { store, handlers } = await setupStream();
      store.sending = true;

      // tool-call-start
      const startH = handlers.get("chat:tool-call-start")!;
      startH({ payload: { conversation_id: "c1", id: "tc-1", name: "read_file" } });
      expect(store.streamingToolCalls.size).toBe(1);
      expect(store.streamingToolCalls.get("tc-1")!.name).toBe("read_file");

      // tool-call-delta — 累积参数
      const deltaH = handlers.get("chat:tool-call-delta")!;
      deltaH({ payload: { conversation_id: "c1", id: "tc-1", delta: `{"path":"/tmp/test` } });
      deltaH({ payload: { conversation_id: "c1", id: "tc-1", delta: `.txt"}` } });
      expect(store.streamingToolCalls.get("tc-1")!.arguments).toBe(`{"path":"/tmp/test.txt"}`);

      // tool-call-end
      const endH = handlers.get("chat:tool-call-end")!;
      endH({ payload: { conversation_id: "c1", id: "tc-1" } });
      expect(store.streamingToolCalls.get("tc-1")!.ended).toBe(true);

      // tool-result
      const resultH = handlers.get("chat:tool-result")!;
      resultH({ payload: { conversation_id: "c1", tool_use_id: "tc-1", content: "file contents here", is_error: false, duration_ms: 42 } });
      expect(store.streamingToolCalls.get("tc-1")!.result).toEqual({
        content: "file contents here",
        isError: false,
        durationMs: 42,
      });
    });

    it("multi-round: chat:assistant-start triggers freeze + reset + new placeholder", async () => {
      const { store, handlers } = await setupStream();
      store.sending = true;
      store.streamingText = "round 1 text";
      store.streamingThinking = "round 1 thinking";

      // 先有一个 assistant 消息在列表里（模拟第一轮）
      store.messages = [{
        id: "asst-r1",
        conversation_id: "c1",
        role: "assistant",
        content: "",
        content_blocks: "[]",
        token_count: null,
        error: null,
        created_at: new Date().toISOString(),
        rowid: 0,
        model: null,
      }];

      // chat:assistant-start — 第二轮开始
      const asstStartH = handlers.get("chat:assistant-start")!;
      asstStartH({ payload: { conversation_id: "c1", message_id: "asst-r2" } });

      // streaming 状态被重置
      expect(store.streamingText).toBe("");
      expect(store.streamingThinking).toBe("");
      expect(store.streamingToolCalls.size).toBe(0);
      // 消息列表增加了第二轮占位
      expect(store.messages.length).toBeGreaterThanOrEqual(2);
      const lastMsg = store.messages[store.messages.length - 1];
      expect(lastMsg.id).toBe("asst-r2");
      expect(lastMsg.role).toBe("assistant");
    });

    it("chat:error resets sending and sets lastError", async () => {
      const { store, handlers } = await setupStream();
      store.sending = true;
      store.streamingText = "partial...";

      const errorH = handlers.get("chat:error")!;
      errorH({ payload: { conversation_id: "c1", message_id: "asst-1", kind: "api", message: "Rate limit exceeded" } });

      expect(store.sending).toBe(false);
      expect(store.lastError).toBe("请求过于频繁，请稍后再试");
    });

    it("chat:done(abort) filters empty assistant message", async () => {
      const { store, handlers } = await setupStream();
      store.sending = true;

      // 追加一个空 assistant 占位
      store.messages = [{
        id: "asst-empty",
        conversation_id: "c1",
        role: "assistant",
        content: "",
        content_blocks: "[]",
        token_count: null,
        error: null,
        created_at: new Date().toISOString(),
        rowid: 0,
        model: null,
      }];

      const doneH = handlers.get("chat:done")!;
      doneH({ payload: { conversation_id: "c1", message_id: "asst-empty", finish_reason: "abort", usage: null } });

      expect(store.sending).toBe(false);
      // 空 assistant + abort → 被移除
      expect(store.messages).toHaveLength(0);
    });

    it("stopGeneration calls backend cancel and sets sending=false", async () => {
      mockInvoke.mockResolvedValue(undefined);
      const { store } = await setupStream();
      store.sending = true;

      await store.stopGeneration();

      expect(store.sending).toBe(false);
      expect(mockInvoke).toHaveBeenCalledWith("stop_generation", { conversationId: "c1" });
    });

    it("bgStreams: events for non-active conversation do NOT affect active streaming", async () => {
      const { store, handlers } = await setupStream();
      store.sending = true;
      store.streamingText = "active conv text";

      // chunk for different conv — should NOT touch active streamingText
      const chunkH = handlers.get("chat:chunk")!;
      chunkH({ payload: { conversation_id: "c2", delta: "background text" } });
      expect(store.streamingText).toBe("active conv text"); // 活跃会话不受影响

      // done for different conv — should NOT reset active sending
      const doneH = handlers.get("chat:done")!;
      doneH({ payload: { conversation_id: "c2", message_id: "x", finish_reason: "stop", usage: null } });
      expect(store.sending).toBe(true); // 活跃会话仍在发送中
    });
  });

  describe("sendMessage", () => {
    it("sets sending=true, resets streaming state, calls bridge", async () => {
      mockInvoke.mockResolvedValue(undefined);
      const store = useChatStore();
      store.conversations = [fakeConv("c1")];
      store.activeConvId = "c1";
      store.streamingText = "old text";
      store.streamingThinking = "old thinking";

      await store.sendMessage("hello");

      expect(store.sending).toBe(true);
      expect(store.streamingText).toBe("");
      expect(store.streamingThinking).toBe("");
      expect(store.streamingToolCalls.size).toBe(0);
      expect(store.lastError).toBeNull();
      expect(mockInvoke).toHaveBeenCalled();
    });
  });
});
