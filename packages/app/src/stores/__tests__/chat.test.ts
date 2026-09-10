import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { flushPromises } from "@vue/test-utils";
import { useChatStore } from "../chat";
import { useChatEvents } from "../../composables/useChatEvents";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Message } from "../../types";

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

/** 工具函数：构造最小 Message */
function fakeMsg(id: string, convId: string): Message {
  return {
    id,
    conversation_id: convId,
    role: "user",
    content: id,
    content_blocks: "[]",
    token_count: null,
    error: null,
    created_at: "2024-01-01T00:00:00Z",
    rowid: 1,
    model: null,
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

  describe("useChatEvents", () => {
    it("registers all event listeners; cleanup enables re-register without leak", async () => {
      mockListen.mockResolvedValue(() => {});
      useChatStore();

      const cleanup = await useChatEvents();
      const callCount = mockListen.mock.calls.length;
      expect(callCount).toBeGreaterThan(0);

      // 拆卸后重新注册：listen 调用次数翻倍，不泄漏
      cleanup();
      await useChatEvents();
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

    describe("respondToAuth（委派预授权档透传，2026-09-03）", () => {
      it("delegationGrant 透传 delegation_grant + 乐观删条目", async () => {
        mockInvoke.mockResolvedValue(undefined);
        const store = useChatStore();
        const m = new Map(store.pendingAuthRequests);
        m.set("c1", {
          payload: {
            request_id: "d1", conversation_id: "c1", message_id: "m1",
            agent_name: "wenshu", agent_id: "a1", task: "整理文档",
          },
          receivedAt: Date.now(),
        });
        store.pendingAuthRequests = m;

        await store.respondToAuth("d1", true, "once", "commands");

        expect(store.pendingAuthRequests.size).toBe(0);
        expect(mockInvoke).toHaveBeenCalledWith("respond_tool_auth", {
          input: {
            request_id: "d1",
            allowed: true,
            scope: "once",
            delegation_grant: "commands",
          },
        });
      });

      it("工具授权不产生 delegation_grant 键（undefined 序列化省略）", async () => {
        mockInvoke.mockResolvedValue(undefined);
        const store = useChatStore();

        await store.respondToAuth("r1", false);

        const arg = mockInvoke.mock.calls[mockInvoke.mock.calls.length - 1];
        expect(arg[0]).toBe("respond_tool_auth");
        expect(arg[1]).toEqual({
          input: { request_id: "r1", allowed: false, scope: "once", delegation_grant: undefined },
        });
      });
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

    it("loadConversations during undo window does not resurrect pending-delete conv", async () => {
      // 复活竞态（手测「删除不生效」根因之一）：乐观删除后 5s 才真删后端，
      // 窗口内任何后台刷新（如 delegation-started）把「已删」行带回列表
      mockInvoke.mockResolvedValue(undefined);
      const store = useChatStore();
      store.conversations = [fakeConv("c1"), fakeConv("c2")];
      store.activeConvId = "c1";

      await store.deleteConversation("c1");
      expect(store.conversations).toHaveLength(1);

      // 后端还没真删——listAll 仍返回 c1；刷新不得回灌
      mockInvoke.mockResolvedValueOnce([fakeConv("c1"), fakeConv("c2")]);
      await store.loadConversations();
      expect(store.conversations.map((c) => c.id)).toEqual(["c2"]);

      // 撤销仍可恢复（独立于刷新路径）
      const restored = store.undoDeleteConversation("c1");
      expect(restored).toBe(true);
      expect(store.conversations.map((c) => c.id)).toContain("c1");
    });
  });

  describe("streaming state machine", () => {
    /** 设置活跃会话 + 注入 mock handlers + 显式注册事件监听 */
    async function setupStream() {
      const handlers = captureHandlers();
      const store = useChatStore();
      store.conversations = [fakeConv("c1")];
      store.activeConvId = "c1";
      // 显式注册事件监听：App.vue 负责在生产环境初始化，测试中手动触发
      await useChatEvents();
      return { store, handlers };
    }

    it("single-turn: chat:start → chat:chunk → chat:done accumulates text and resets", async () => {
      const { store, handlers } = await setupStream();

      // 模拟 sendMessage 开始（sendingConvId 是用户回合的判据锚——sendMessage
      // 在 invoke 前必设，缺省会被 chat:start 误判为外部回合走权威刷新）
      store.sending = true;
      store.sendingConvId = "c1";
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

    it("跨会话泄漏回归：切入后台流式会话时清空上一会话的工具调用（父会话委派卡不进子会话）", async () => {
      // 真实时序复刻（手测「子会话气泡底部挂着父会话的委派卡片」根因）：
      // 父会话流式发出 delegate_to_agent → 委派执行中子会话后台流式 →
      // 用户点「打开任务」切入子会话。修复前 bg 恢复分支不清 streamingToolCalls，
      // 父会话的委派调用渲染进子会话 live 气泡底部（DelegationCard 泄漏）。
      const { store, handlers } = await setupStream();
      // 父会话 c1 激活且正在流式：主 agent 发出 delegate_to_agent 调用
      store.sending = true;
      const tcs = handlers.get("chat:tool-call-start")!;
      tcs({ payload: { conversation_id: "c1", id: "tc-delg", name: "delegate_to_agent" } });
      const tcd = handlers.get("chat:tool-call-delta")!;
      tcd({ payload: { conversation_id: "c1", id: "tc-delg", delta: `{"task":"写文案"}` } });
      expect(store.streamingToolCalls.size).toBe(1);

      // 子会话 c2 在后台流式（委派执行中，chunk 走 bgStreams 快照）
      const chunkH = handlers.get("chat:chunk")!;
      chunkH({ payload: { conversation_id: "c2", delta: "子会话输出" } });

      // 用户切入子会话
      store.selectConversation("c2");

      expect(store.activeConvId).toBe("c2");
      expect(store.sending).toBe(true); // 子会话在流式，续渲染
      expect(store.streamingToolCalls.size).toBe(0); // ← 修复点：父会话调用不泄漏
      expect(store.streamingText).toBe("子会话输出"); // 后台快照恢复
      expect(store.streamingThinking).toBe("");
      expect(store.bgStreams.get("c1")).toBeDefined(); // 父会话文本被快照，切回可恢复
    });

    it("外部回合（A2）：ucb 缺省且非用户发起 → sending 即刻置位 + 权威刷新 + 不本地 push 占位", async () => {
      // MA-3 消费回合形态：emit_user_blocks=false（ucb=None）且 sendingConvId=null
      //（不是本前端发起的回合）。incoming user 行后端已落库（带来源标注双块），
      // 不刷新的话来件卡要等切走再切回才出现、回复看起来凭空流出。
      const { store, handlers } = await setupStream();
      store.sending = false;
      const invokeCalls = mockInvoke.mock.calls.length;

      const startH = handlers.get("chat:start")!;
      startH({ payload: { conversation_id: "c1", user_message_id: "u-ext", assistant_message_id: "asst-ext" } });

      expect(store.sending).toBe(true); // 生成中指示即刻可见（不等 assistant-start）
      expect(store.messages).toHaveLength(0); // 同步段先清列表——占位由 DB 权威行带入
      expect(mockInvoke.mock.calls.length).toBe(invokeCalls + 1);
      expect(mockInvoke.mock.calls[mockInvoke.mock.calls.length - 1]?.[0]).toBe("list_messages"); // 权威刷新在途

      await flushPromises();
      // 刷新完成后也不本地 push 占位（mock 空返——没有 asst-ext 行即证明没塞占位）
      expect(store.messages.filter((m) => m.id === "asst-ext")).toHaveLength(0);
    });

    it("纯文本用户回合（ucb 同为缺省）：sendingConvId 判据保住原路径——占位本地 push、不刷新", async () => {
      // 后端 emit_user_blocks=has_files：纯文本用户回合 ucb 也是 None——判据只能
      // 靠 sendingConvId（sendMessage 在 invoke 前已设）。误进外部分支会把乐观
      // 气泡整页刷掉、占位与 DB 行重复。
      const { store, handlers } = await setupStream();
      store.sending = true;
      store.sendingConvId = "c1";
      const invokeCalls = mockInvoke.mock.calls.length;

      const startH = handlers.get("chat:start")!;
      startH({ payload: { conversation_id: "c1", user_message_id: "u-1", assistant_message_id: "asst-1" } });

      expect(store.messages).toHaveLength(1);
      expect(store.messages[0].id).toBe("asst-1"); // 本地占位照常
      expect(store.messages[0].role).toBe("assistant");
      expect(mockInvoke.mock.calls.length).toBe(invokeCalls); // 未发起 list_messages
      expect(store.sending).toBe(true);
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

    it("在途回合早退可见化（A3）：写 send_failed 横幅 + lastFailedSend、不 invoke、附件不消费", async () => {
      // sending 置位的回合形态有二——用户自己的回合未完 / 正在看的消费回合
      //（chat:assistant-start 置位）。此前静默早退吞掉输入（用户只看到「发送没反应」）。
      mockInvoke.mockClear();
      const store = useChatStore();
      store.conversations = [fakeConv("c1")];
      store.activeConvId = "c1";
      store.sending = true;
      store.pendingImages = [{ data: "x", mediaType: "image/png", name: "a.png" }];

      await store.sendMessage("第二条");

      expect(store.lastErrors.get("c1")?.kind).toBe("send_failed");
      expect(store.lastError).toContain("仍在处理中");
      expect(store.lastFailedSend?.content).toBe("第二条"); // 横幅「重试」的数据源
      expect(mockInvoke).not.toHaveBeenCalled(); // 没打进还在跑的回合
      expect(store.pendingImages).toHaveLength(1); // chips 不消费（早退在并块组装之前）
    });
  });

  describe("加载竞态守卫（快速切换会话）", () => {
    it("loadMessages：A 的晚到响应被丢弃——不整替 B 的 messages、不污染 hasMore", async () => {
      // 旧会话 c1 的 list 调用挂起（慢网络），新会话 c2 的调用立即返回
      let resolveA!: (v: Message[]) => void;
      const pendingA = new Promise<Message[]>((res) => { resolveA = res; });
      mockInvoke.mockImplementationOnce(() => pendingA);
      mockInvoke.mockResolvedValueOnce([fakeMsg("m-b1", "c2")]);

      const store = useChatStore();
      store.conversations = [fakeConv("c1"), fakeConv("c2")];
      store.selectConversation("c1"); // 触发 loadMessages(c1)（在途）
      store.selectConversation("c2"); // 立刻切到 c2（首屏先回）

      await flushPromises();
      expect(store.messages.map((m) => m.id)).toEqual(["m-b1"]);
      expect(store.msgLoading).toBe(false);

      // c1 的响应此刻才到（满页 50 条）：过期响应必须整段丢弃
      resolveA(Array.from({ length: 50 }, (_, i) => fakeMsg(`m-a${i}`, "c1")));
      await flushPromises();
      expect(store.messages.map((m) => m.id)).toEqual(["m-b1"]); // 未被 A 污染
      expect(store.hasMore).toBe(false); // c2 不足 50 条——A 的满页没有污染 hasMore
      expect(store.msgLoading).toBe(false);
    });

    it("loadMoreMessages：切会话后返回的旧会话 older 消息不前插进新会话列表", async () => {
      const store = useChatStore();
      store.conversations = [fakeConv("c1"), fakeConv("c2")];
      store.activeConvId = "c1";
      store.messages = [fakeMsg("m-a2", "c1"), fakeMsg("m-a1", "c1")];
      store.hasMore = true;

      let resolveOlder!: (v: Message[]) => void;
      const pendingOlder = new Promise<Message[]>((res) => { resolveOlder = res; });
      mockInvoke.mockImplementationOnce(() => pendingOlder); // c1 的 loadMore（在途）
      mockInvoke.mockResolvedValueOnce([fakeMsg("m-b1", "c2")]); // 切到 c2 的首屏

      const loading = store.loadMoreMessages(); // c1 分页在途
      store.selectConversation("c2"); // 切走（首屏立即返回）
      await flushPromises();
      expect(store.messages.map((m) => m.id)).toEqual(["m-b1"]);

      resolveOlder([fakeMsg("m-a0", "c1")]); // c1 的 older 晚到
      await loading;
      await flushPromises();
      expect(store.messages.map((m) => m.id)).toEqual(["m-b1"]); // 未前插
      expect(store.loadingMore).toBe(false);
    });
  });

  describe("sendMessage 失败回滚（以发起回合的会话为键）", () => {
    it("发送在途切走后失败：横幅写回发起会话、bgStreams 快照清理（切回无幽灵「生成中」）", async () => {
      let rejectSend!: (reason?: unknown) => void;
      mockInvoke.mockImplementationOnce(() => new Promise<never>((_res, rej) => { rejectSend = rej; }));
      // 切到 c2 / 切回 c1 的首屏加载走 beforeEach 的默认空返

      const store = useChatStore();
      store.conversations = [fakeConv("c1"), fakeConv("c2")];
      store.activeConvId = "c1";

      const sent = store.sendMessage("hello");
      expect(store.sending).toBe(true);

      // 发送在途时切到 c2：c1 的流式态快照进 bgStreams、sending 复位
      store.selectConversation("c2");
      await flushPromises();
      expect(store.activeConvId).toBe("c2");
      expect(store.bgStreams.has("c1")).toBe(true);

      // c1 的 send 此刻被后端拒绝
      rejectSend(new Error("boom"));
      await sent;
      await flushPromises();

      // 横幅挂在发起会话 c1 头上（当前在 c2——lastError 取值为 null）
      expect(store.lastError).toBeNull();
      expect(store.lastErrors.has("c2")).toBe(false);
      expect(store.lastErrors.has("c1")).toBe(true);
      // bgStreams 快照已清：切回 c1 不会恢复出幽灵「生成中」
      expect(store.bgStreams.has("c1")).toBe(false);

      store.selectConversation("c1");
      await flushPromises();
      expect(store.sending).toBe(false); // ← 幽灵锁死回归点（修复前 bg 恢复 sending=true）
      expect(store.lastError).toContain("请求没有送达"); // 横幅随发起会话可见
    });
  });

  describe("预算 HUD（chat:budget → updateBudget）", () => {
    /** 最小 budget payload 工厂 */
    const budgetPayload = (overrides?: Partial<import("../../types").ChatBudgetPayload>) => ({
      conversation_id: "c1",
      cumulative_tokens: 120_000,
      cumulative_cached_tokens: 0,
      cumulative_prompt_tokens: 120_000,
      effective_cap: 600_000,
      initial_cap: 600_000,
      renewal_index: 0,
      max_renewals: 2,
      renewed: false,
      round: 3,
      ...overrides,
    });

    it("常规更新：写入 budget 状态（含缓存命中两路字段），不产生续期提示", () => {
      const store = useChatStore();
      store.updateBudget(budgetPayload({
        cumulative_cached_tokens: 90_000, cumulative_prompt_tokens: 120_000,
      }));
      expect(store.budget?.cumulative_tokens).toBe(120_000);
      expect(store.budget?.effective_cap).toBe(600_000);
      expect(store.budget?.cumulative_cached_tokens).toBe(90_000);
      expect(store.renewalNotice).toBeNull();
    });

    it("续期事件（renewed=true）：置位提示 + 5s 后自动清除（fake timers）", () => {
      vi.useFakeTimers();
      const store = useChatStore();
      store.updateBudget(budgetPayload({
        renewed: true, renewal_index: 1, effective_cap: 1_200_000,
      }));
      expect(store.renewalNotice).toContain("1/2");
      expect(store.renewalNotice).toContain("120万");
      vi.advanceTimersByTime(5000);
      expect(store.renewalNotice).toBeNull();
      // budget 本体仍在（HUD 显示续期后上限）
      expect(store.budget?.effective_cap).toBe(1_200_000);
      vi.useRealTimers();
    });

    it("切会话 / 新回合发送：预算状态重置", async () => {
      mockInvoke.mockResolvedValue(undefined);
      const store = useChatStore();
      store.conversations = [fakeConv("c1"), fakeConv("c2")];
      store.updateBudget(budgetPayload());

      store.selectConversation("c2");
      expect(store.budget).toBeNull();

      store.updateBudget(budgetPayload());
      await store.sendMessage("继续");
      expect(store.budget).toBeNull(); // 新回合新预算（后端 per-send）
    });
  });
});
