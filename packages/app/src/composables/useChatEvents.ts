// composables/useChatEvents.ts
// Tauri 流式事件 → chat store 状态 的接线层。从 stores/chat.ts 的「G 区」抽出，
// 让 store 回归「纯状态 + 动作」，事件监听副作用集中在此 composable。
//
// 职责：注册 13 个 chat:* 事件监听器，把 payload 映射为对 chat store 的状态变更。
// 不持有业务状态——所有状态都在 store 里，这里只做「事件 → store 动作/赋值」。
// 例外：recentErrorConvs（chat:error ↔ chat:done(abort) 的本会话协调标志）是纯事件层
// 的瞬态簿记，与 store 长期状态无关，故留在本 composable 内。
//
// 生命周期：App.vue 在 onMounted 调 `const cleanup = await useChatEvents()`，
// onUnmounted 调 cleanup() 拆卸监听器（补齐此前缺失的 teardown）。
// 返回的 cleanup 可重复调用安全（幂等）。

import { listen } from "@tauri-apps/api/event";
import { useChatStore } from "../stores/chat";
import { friendlyError } from "../utils/errors";
import type {
  ChatStartPayload,
  ChatAssistantStartPayload,
  ChatChunkPayload,
  ChatDonePayload,
  ChatErrorPayload,
  ChatToolCallStartPayload,
  ChatToolCallDeltaPayload,
  ChatToolCallEndPayload,
  ChatToolResultPayload,
  ChatThinkingPayload,
  ToolAuthRequestPayload,
  ConfigProposalPayload,
  DelegationStartedPayload,
} from "../types";

export async function useChatEvents(): Promise<() => void> {
  const chat = useChatStore();
  const unlisteners: Array<() => void> = [];
  /** 刚收到 chat:error、等待配套 chat:done(abort) 的会话集合。后端在 chat:error 后会
   *  再 emit 一次 chat:done(abort) 做收尾，据此跳过 freeze（不覆盖错误文案）+
   *  不设 lastFinishReason（避免误显「已手动停止」）——错误以 chat:error 文案为准。*/
  const recentErrorConvs = new Set<string>();

  /** 订阅 Tauri 事件并自动登记取消函数 */
  async function subscribe<T>(event: string, handler: (event: { payload: T }) => void) {
    const u = await listen<T>(event, handler);
    unlisteners.push(u);
  }

  await subscribe<ChatStartPayload>("chat:start", (e) => {
    if (e.payload.conversation_id !== chat.activeConvId) return;
    // 含附件时：用后端 materialize 后的 content_blocks（含提取正文 Text 块）patch 乐观
    // 用户消息——前端发送时只放了 Attachment 占位卡片、拿不到提取正文，不 patch 的话
    // 附件详情弹窗会全程显示「无提取文本」（要等切换会话重载才恢复）。
    const ucb = e.payload.user_content_blocks;
    if (ucb) {
      for (let i = chat.messages.length - 1; i >= 0; i--) {
        if (chat.messages[i].role === "user") {
          chat.messages[i] = { ...chat.messages[i], content_blocks: ucb };
          break;
        }
      }
    }
    chat.messages.push({
      id: e.payload.assistant_message_id,
      conversation_id: e.payload.conversation_id,
      role: "assistant",
      content: "",
      content_blocks: "[]",
      token_count: null,
      error: null,
      created_at: new Date().toISOString(),
      rowid: 0,
      model: chat.currentModel,
    });
  });

  // MA-1 UX：委派子会话创建成功即通知——刷新会话列表让子会话行立刻可见
  //（任务胶囊有数据、运行中委派卡片可跳）。child_conversation_id 此刻起即可达，
  // 不必等完成时的 tool_result 回传。
  await subscribe<DelegationStartedPayload>("chat:delegation-started", () => {
    void chat.loadConversations();
  });

  // 多轮工具调用：每轮工具执行完毕后，后端创建下一轮 assistant 占位并 emit。  // 前端据此冻结上一条 assistant（写入 tool_use/text/thinking）+ 插入 user(tool_result)
  // + 重置 streaming 状态 + push 新占位。
  await subscribe<ChatAssistantStartPayload>("chat:assistant-start", (e) => {
    if (e.payload.conversation_id !== chat.activeConvId) return;
    chat.resetSendTimeout();
    // 确保 sending 为 true：多轮工具执行间隙可能触发静默超时把它置 false
    chat.sending = true;
    chat.freezeCurrentAssistant();
    chat.streamingText = "";
    chat.streamingThinking = "";
    chat.thinkingStartTime = null;
    chat.streamingToolCalls = new Map();
    chat.messages.push({
      id: e.payload.message_id,
      conversation_id: e.payload.conversation_id,
      role: "assistant",
      content: "",
      content_blocks: "[]",
      token_count: null,
      error: null,
      created_at: new Date().toISOString(),
      rowid: 0,
      model: chat.currentModel,
    });
  });

  await subscribe<ChatChunkPayload>("chat:chunk", (e) => {
    const cid = e.payload.conversation_id;
    if (cid !== chat.activeConvId) {
      // 后台会话：累积文本到快照，切回时恢复（不触活跃 UI / messages）
      const m = new Map(chat.bgStreams);
      const cur = m.get(cid) ?? { text: "", thinking: "" };
      cur.text += e.payload.delta;
      m.set(cid, cur);
      chat.bgStreams = m;
      return;
    }
    chat.resetSendTimeout();
    chat.streamingText += e.payload.delta;
    const idx = chat.messages.length - 1;
    if (idx >= 0 && chat.messages[idx].role === "assistant") {
      chat.messages[idx] = { ...chat.messages[idx], content: chat.streamingText };
    }
  });

  // ---- 工具调用事件 ----
  await subscribe<ChatToolCallStartPayload>("chat:tool-call-start", (e) => {
    if (e.payload.conversation_id !== chat.activeConvId) return;
    chat.resetSendTimeout();
    const map = new Map(chat.streamingToolCalls);
    map.set(e.payload.id, {
      id: e.payload.id,
      name: e.payload.name,
      arguments: "",
      ended: false,
    });
    chat.streamingToolCalls = map;
  });

  await subscribe<ChatToolCallDeltaPayload>("chat:tool-call-delta", (e) => {
    if (e.payload.conversation_id !== chat.activeConvId) return;
    chat.resetSendTimeout();
    const map = new Map(chat.streamingToolCalls);
    const call = map.get(e.payload.id);
    if (call) {
      call.arguments += e.payload.delta;
      map.set(e.payload.id, { ...call });
      chat.streamingToolCalls = map;
    }
  });

  await subscribe<ChatToolCallEndPayload>("chat:tool-call-end", (e) => {
    if (e.payload.conversation_id !== chat.activeConvId) return;
    chat.resetSendTimeout();
    const map = new Map(chat.streamingToolCalls);
    const call = map.get(e.payload.id);
    if (call) {
      call.ended = true;
      map.set(e.payload.id, { ...call });
      chat.streamingToolCalls = map;
    }
  });

  await subscribe<ChatToolResultPayload>("chat:tool-result", (e) => {
    if (e.payload.conversation_id !== chat.activeConvId) return;
    chat.resetSendTimeout();
    const map = new Map(chat.streamingToolCalls);
    const call = map.get(e.payload.tool_use_id);
    if (call) {
      call.result = { content: e.payload.content, isError: e.payload.is_error, durationMs: e.payload.duration_ms };
      map.set(e.payload.tool_use_id, { ...call });
      chat.streamingToolCalls = map;
    }
  });

  // ---- 思考过程 ----
  await subscribe<ChatThinkingPayload>("chat:thinking", (e) => {
    const cid = e.payload.conversation_id;
    if (cid !== chat.activeConvId) {
      const m = new Map(chat.bgStreams);
      const cur = m.get(cid) ?? { text: "", thinking: "" };
      cur.thinking += e.payload.content;
      m.set(cid, cur);
      chat.bgStreams = m;
      return;
    }
    chat.resetSendTimeout();
    if (!chat.streamingThinking) {
      chat.thinkingStartTime = Date.now();
    }
    chat.streamingThinking += e.payload.content;
  });

  await subscribe<ChatDonePayload>("chat:done", (e) => {
    const cid = e.payload.conversation_id;
    if (cid !== chat.activeConvId) {
      // 后台会话完成：后端已把最终态落库，清掉快照即可（不触活跃 UI，无需前端 freeze）
      const m = new Map(chat.bgStreams);
      m.delete(cid);
      chat.bgStreams = m;
      recentErrorConvs.delete(cid);
      return;
    }
    // 错误后的配套 chat:done(abort)：chat:error 已是终态（写了错误文案 + 重置），
    // 这里只做最小收尾——不 freeze（会覆盖错误文案）、不设 lastFinishReason（会误显「已手动停止」）。
    if (recentErrorConvs.has(cid)) {
      recentErrorConvs.delete(cid);
      chat.clearSendTimeout();
      chat.sending = false;
      chat.streamingText = "";
      chat.streamingToolCalls = new Map();
      chat.streamingThinking = "";
      chat.thinkingStartTime = null;
      return;
    }
    chat.clearSendTimeout();

    // 记录思考耗时与内容
    if (chat.thinkingStartTime) {
      const elapsed = Math.floor((Date.now() - chat.thinkingStartTime) / 1000);
      const dur = elapsed < 60 ? `${elapsed}s` : `${Math.floor(elapsed / 60)}m ${elapsed % 60}s`;
      chat.thinkingDuration = dur;
      chat.lastThinkingContent = chat.streamingThinking || null;
      const asstMsgId = e.payload.message_id;
      if (asstMsgId) {
        const map = new Map(chat.thinkingDurations);
        map.set(asstMsgId, dur);
        chat.thinkingDurations = map;
      }
    }

    // 冻结最后一条 assistant（把本轮 streaming 文本/思考/工具调用写入其 content_blocks，
    // tool_result 分离为独立 user 消息）。替代旧的「打包全部 streamingToolCalls 进末条」
    // —— 那会把 tool_result 也塞进 assistant，违反 Anthropic 协议。
    chat.freezeCurrentAssistant();

    // M3：abort 时若目标 assistant 在 freeze 后仍为空（无内容无 blocks，后端已删 DB 行），
    // 前端同步移除，避免残留空气泡。
    if (e.payload.finish_reason === "abort") {
      const doneId = e.payload.message_id;
      chat.messages = chat.messages.filter(
        (m) => !(m.id === doneId && m.role === "assistant" && !m.content && (!m.content_blocks || m.content_blocks === "[]")),
      );
    }

    chat.sending = false;
    chat.streamingText = "";
    chat.streamingToolCalls = new Map();
    chat.streamingThinking = "";
    chat.thinkingStartTime = null;
    chat.lastFinishReason = e.payload.finish_reason;
    // 用 message_id 定位最终 assistant（freezeCurrentAssistant 可能已在末尾插入 user 消息，
    // 不能再假设末条索引），更新其 token_count
    if (e.payload.usage) {
      const doneId = e.payload.message_id;
      chat.messages = chat.messages.map((msg) =>
        msg.id === doneId && msg.role === "assistant"
          ? { ...msg, token_count: e.payload.usage!.completion_tokens }
          : msg,
      );
    }
  });

  await subscribe<ChatErrorPayload>("chat:error", (e) => {
    const cid = e.payload.conversation_id;
    // 标记：后端会紧跟一次 chat:done(abort) 收尾，chat:done 据此跳过 freeze + lastFinishReason
    recentErrorConvs.add(cid);
    if (cid !== chat.activeConvId) {
      // 后台会话出错：清掉快照（用户切回时走 DB 加载，看到错误态）
      const m = new Map(chat.bgStreams);
      m.delete(cid);
      chat.bgStreams = m;
      return;
    }
    chat.clearSendTimeout();
    chat.sending = false;
    chat.streamingText = "";
    chat.streamingThinking = "";
    chat.thinkingStartTime = null;
    chat.streamingToolCalls = new Map();
    chat.lastFinishReason = null;
    // 错误横幅按会话隔离：只写到出错会话（此处 cid === activeConvId，已过上方 early-return）
    {
      const m = new Map(chat.lastErrors);
      m.set(cid, friendlyError(e.payload.message));
      chat.lastErrors = m;
    }
    // 用 message_id 定位出错的 assistant（多轮工具下不能遍历改所有 assistant）
    const errId = e.payload.message_id;
    chat.messages = chat.messages.map((msg) => {
      if (msg.id === errId && msg.role === "assistant") {
        const friendly = friendlyError(e.payload.message);
        if (msg.content === "") {
          return { ...msg, content: `错误: ${friendly}`, error: friendly };
        }
        if (!msg.error) {
          return { ...msg, content: msg.content + `\n\n[生成中断: ${e.payload.message}]`, error: e.payload.message };
        }
      }
      return msg;
    });
  });

  // ---- 工具授权请求 ----（始终按 convId 存，不丢后台会话；cancel 时按 request_id 清）
  await subscribe<ToolAuthRequestPayload>("chat:tool-auth-request", (e) => {
    const m = new Map(chat.pendingAuthRequests);
    // receivedAt 驱动 120s 倒计时渲染（与后端 TIMEOUT 同步到期自动消失）
    m.set(e.payload.conversation_id, { payload: e.payload, receivedAt: Date.now() });
    chat.pendingAuthRequests = m;
  });
  await subscribe<{ request_id: string; conversation_id: string; reason: string }>(
    "chat:tool-auth-request-cancel",
    (e) => {
      const m = new Map(chat.pendingAuthRequests);
      for (const [cid, entry] of m) {
        if (entry.payload.request_id === e.payload.request_id) { m.delete(cid); break; }
      }
      chat.pendingAuthRequests = m;
    },
  );

  // ---- 配置提案请求 ----（始终按 convId 存，不丢后台会话；cancel 时按 request_id 清）
  await subscribe<ConfigProposalPayload>("chat:config-proposal", (e) => {
    const m = new Map(chat.pendingProposals);
    m.set(e.payload.conversation_id, e.payload);
    chat.pendingProposals = m;
  });
  await subscribe<{ request_id: string; conversation_id: string; reason: string }>(
    "chat:config-proposal-cancel",
    (e) => {
      const m = new Map(chat.pendingProposals);
      const cur = m.get(e.payload.conversation_id);
      // request_id 守卫：仅当当前条目仍是这个已失效请求时才删，
      // 避免误删同 conv 后到的、更新的活提案（同会话工具串行执行，但 cancel
      // 与新 proposal 事件到达顺序无保证）
      if (cur && cur.request_id === e.payload.request_id) {
        m.delete(e.payload.conversation_id);
        chat.pendingProposals = m;
      }
    },
  );

  /** 拆卸所有已注册的 Tauri 事件监听器 + 清发送超时（幂等，可重复调用） */
  function destroyEvents() {
    for (const u of unlisteners) u();
    unlisteners.length = 0;
    chat.clearSendTimeout();
  }

  return destroyEvents;
}
