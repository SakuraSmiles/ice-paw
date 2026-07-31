// 聊天状态管理（会话列表 + 当前会话 + 消息 + 流式事件）
// 侧栏不再按 Agent 过滤，显示全部会话混合列表
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { listen, emit } from "@tauri-apps/api/event";
import type { Conversation, Message } from "../types";
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
} from "../types";
import { bridge } from "../api/bridge";
import { useAgentStore } from "./agent";

export const useChatStore = defineStore("chat", () => {
  // ===== 会话列表（全部会话，不限 agent） =====
  const conversations = ref<Conversation[]>([]);
  const convLoading = ref(false);

  async function loadConversations() {
    convLoading.value = true;
    try {
      conversations.value = await bridge.conversations.listAll();
    } catch (e) {
      console.error("加载会话列表失败:", e);
    } finally {
      convLoading.value = false;
    }
  }

  // ===== 当前会话 =====
  const activeConvId = ref<string | null>(null);

  const activeConversation = computed(() =>
    conversations.value.find((c) => c.id === activeConvId.value) ?? null,
  );

  function selectConversation(id: string) {
    const oldId = activeConvId.value;
    // 离开「正在流式」的会话：把当前流式文本快照到 bgStreams，切回时可恢复
    if (oldId && oldId !== id && sending.value) {
      bgStreams.value = new Map(bgStreams.value).set(oldId, {
        text: streamingText.value,
        thinking: streamingThinking.value,
      });
    }
    activeConvId.value = id;
    // 切入的会话若在后台流式（bgStreams 有），恢复其文本并在末条 assistant 继续渲染
    const bg = bgStreams.value.get(id);
    if (bg) {
      sending.value = true;
      streamingText.value = bg.text;
      streamingThinking.value = bg.thinking;
      bgStreams.value = new Map([...bgStreams.value].filter(([k]) => k !== id));
    } else {
      sending.value = false;
      streamingText.value = "";
      streamingThinking.value = "";
      streamingToolCalls.value = new Map();
      thinkingStartTime.value = null;
      thinkingDuration.value = null;
      lastThinkingContent.value = null;
    }
    // finish_reason 是全局单份，切换会话时清空，避免上个会话的标签（如「已手动停止」）泄漏到新会话
    lastFinishReason.value = null;
    loadMessages(id);
  }

  // ===== 消息（含分页） =====
  const messages = ref<Message[]>([]);
  const msgLoading = ref(false);
  const hasMore = ref(true);
  const loadingMore = ref(false);

  async function loadMessages(convId: string) {
    msgLoading.value = true;
    hasMore.value = true;
    loadingMore.value = false;
    try {
      messages.value = await bridge.messages.list(convId, { limit: 50 });
      // 如果返回不足 50 条，说明没有更多了
      hasMore.value = messages.value.length >= 50;
      // 切回正在流式的会话时，把恢复的 streamingText 同步到末条 assistant，
      // 否则 DB 占位为空、要等下一个 chunk 才显示
      if (sending.value && streamingText.value && convId === activeConvId.value) {
        const idx = messages.value.length - 1;
        if (idx >= 0 && messages.value[idx].role === "assistant") {
          messages.value[idx] = { ...messages.value[idx], content: streamingText.value };
        }
      }
    } catch (e) {
      console.error("加载消息列表失败:", e);
    } finally {
      msgLoading.value = false;
    }
  }

  async function loadMoreMessages() {
    if (loadingMore.value || !hasMore.value || messages.value.length === 0) return;
    if (!activeConvId.value) return;
    loadingMore.value = true;
    const oldest = messages.value[0];
    try {
      const older = await bridge.messages.list(activeConvId.value, {
        limit: 50,
        before: [oldest.created_at, oldest.rowid],
      });
      if (older.length < 50) hasMore.value = false;
      messages.value = [...older, ...messages.value];
    } catch (e) {
      console.error("加载更早消息失败:", e);
    } finally {
      loadingMore.value = false;
    }
  }

  // ===== 输入框草稿（跨页面切换保持） =====
  const draftText = ref("");

  // ===== 图片附件列表 =====
  const pendingImages = ref<{ data: string; mediaType: string; name: string }[]>([]);

  // ===== 流式发送 =====
  const sending = ref(false);
  const streamingText = ref("");
  const lastFinishReason = ref<string | null>(null);
  const currentModel = ref<string | null>(null);

  // ===== 流式工具调用/思考状态 =====
  interface ToolCallState {
    id: string;
    name: string;
    arguments: string;
    ended: boolean;
    result?: { content: string; isError: boolean } | null;
  }
  const streamingToolCalls = ref<Map<string, ToolCallState>>(new Map());
  const streamingThinking = ref("");
  const thinkingStartTime = ref<number | null>(null);
  const thinkingDuration = ref<string | null>(null);
  /** 思考结束后保留内容，让用户仍可展开查看 */
  const lastThinkingContent = ref<string | null>(null);
  /** 按消息 ID 持久化思考耗时（切换会话不丢，刷新才丢） */
  const thinkingDurations = ref<Map<string, string>>(new Map());
  const pendingAuthRequest = ref<ToolAuthRequestPayload | null>(null);
  let sendTimeout: ReturnType<typeof setTimeout> | null = null;

  /** 后台会话的流式文本快照：切走「正在流式」的会话时把已累积文本存这里，
   *  切回时恢复。此前 streamingText 是全局单份 + chunk 处理器丢弃非激活会话的事件，
   *  导致「流式中途切走再切回」时已渲染内容丢失、从一半继续渲染。
   *  只追踪 text/thinking（工具调用/多轮结构后台不展示，最终态由后端 chat:done 落库）。*/
  const bgStreams = ref<Map<string, { text: string; thinking: string }>>(new Map());

  /** 刚收到 chat:error、等待配套 chat:done(abort) 的会话集合。后端在 chat:error 后会
   *  再 emit 一次 chat:done(abort) 做收尾，前端据此跳过 freeze（不覆盖错误文案）+
   *  不设 lastFinishReason（避免误显「已手动停止」）——错误应以 chat:error 的文案为准。*/
  const recentErrorConvs = new Set<string>();

  /** 重置 60s 静默超时（滑动窗口）：任何活动事件调用一次即重新计时。
   *  超时只重置 sending 状态（不清 streaming，交由后端 chat:done(abort) 走 freeze）。*/
  function resetSendTimeout() {
    if (sendTimeout) clearTimeout(sendTimeout);
    sendTimeout = setTimeout(() => {
      if (sending.value) {
        console.warn("静默超时（60s 无活动），重置发送状态");
        sending.value = false;
      }
    }, 60000);
  }

  async function sendMessage(content: string, contentBlocks?: import("../types").ContentBlock[]) {
    if (!activeConvId.value || sending.value) return;
    sending.value = true;
    streamingText.value = "";
    streamingThinking.value = "";
    streamingToolCalls.value = new Map();
    thinkingStartTime.value = null;
    thinkingDuration.value = null;
    lastThinkingContent.value = null;
    lastFinishReason.value = null;

    // 从当前 Agent 获取模型名
    const agentStore = useAgentStore();
    const conv = activeConversation.value;
    const agent = conv ? agentStore.getById(conv.agent_id) : null;
    currentModel.value = agent?.model ?? null;

    // 如果有待发送图片，合并到 content_blocks
    const blocks = contentBlocks ?? [];
    if (pendingImages.value.length > 0) {
      for (const img of pendingImages.value) {
        blocks.push({ type: "image", data: img.data, media_type: img.mediaType });
      }
      pendingImages.value = [];
    }

    const blocksJson = blocks.length > 0 ? JSON.stringify(blocks) : "[]";
    const userMsg: Message = {
      id: "user-" + Date.now(),
      conversation_id: activeConvId.value,
      role: "user",
      content,
      content_blocks: blocksJson,
      token_count: null,
      error: null,
      created_at: new Date().toISOString(),
      rowid: 0,
      model: currentModel.value,
    };
    messages.value = [...messages.value, userMsg];

    // 前端超时保护（滑动窗口）：60s 无任何活动事件才触发；活动事件 handler 会 reset
    resetSendTimeout();

    try {
      await bridge.chat.sendMessage(activeConvId.value, content, blocks.length > 0 ? blocks : undefined, true);
    } catch (e) {
      console.error("发送消息失败:", e);
      sending.value = false;
      streamingText.value = "";
    }
  }

  async function stopGeneration() {
    if (!activeConvId.value) return;
    if (sendTimeout) { clearTimeout(sendTimeout); sendTimeout = null; }
    // 乐观停止「生成中」状态（隐藏光标）。**不清空 streaming 内容**——交由后端
    // cancel → finalize_cancel emit 的 chat:done(abort) 走 freezeCurrentAssistant
    // 统一把已生成的部分冻结到末条 assistant。若这里先清空，chat:done 的 freeze
    // 会写入空内容，导致取消时输出到一半的气泡消失。
    sending.value = false;
    try {
      await bridge.chat.stopGeneration(activeConvId.value);
    } catch { /* 静默忽略 */ }
  }

  async function respondToAuth(allowed: boolean) {
    const req = pendingAuthRequest.value;
    if (!req) return;
    await emit("chat:tool-auth-response", { request_id: req.request_id, allowed });
    pendingAuthRequest.value = null;
  }

  // ===== 删除 / 置顶会话 =====
  async function deleteConversation(id: string) {
    try {
      await bridge.conversations.delete(id);
      const wasActive = activeConvId.value === id;
      conversations.value = conversations.value.filter((c) => c.id !== id);
      if (wasActive) {
        if (conversations.value.length > 0) {
          // 直接跳转到列表第一个，不触发 loadMessages（已退出该会话）
          activeConvId.value = conversations.value[0].id;
          loadMessages(activeConvId.value);
        } else {
          activeConvId.value = null;
          messages.value = [];
        }
      }
    } catch (e) {
      console.error("删除会话失败:", e);
    }
  }

  async function pinConversation(id: string, pinned: boolean) {
    try {
      await bridge.conversations.pin(id, pinned);
      // 更新本地状态并重新排序（pinned 优先，按 updated_at 降序）
      conversations.value = conversations.value
        .map((c) => (c.id === id ? { ...c, pinned } : c))
        .sort((a, b) => {
          if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
          return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
        });
    } catch (e) {
      console.error("置顶操作失败:", e);
    }
  }

  // ===== Tauri 事件监听 =====
  let inited = false;

  /** 把当前 streaming 状态冻结到最后一条 assistant 消息：
   *  - 末条 assistant 写入 content + content_blocks（thinking / text / tool_use，不含 result）
   *  - tool_result 组装成独立 user 消息插入末条之后（符合 Anthropic 协议：
   *    tool_result 必须在 user 消息里）
   *  调用方负责在之后重置 streaming 状态。*/
  function freezeCurrentAssistant() {
    const lastIdx = messages.value.length - 1;
    if (lastIdx < 0 || messages.value[lastIdx].role !== "assistant") return;

    const blocks: { type: string; [key: string]: unknown }[] = [];
    if (streamingThinking.value) {
      blocks.push({ type: "thinking", thinking: streamingThinking.value });
    }
    if (streamingText.value) {
      blocks.push({ type: "text", text: streamingText.value });
    }
    const resultBlocks: { type: string; [key: string]: unknown }[] = [];
    for (const call of streamingToolCalls.value.values()) {
      blocks.push({ type: "tool_use", id: call.id, name: call.name, input: call.arguments });
      if (call.result) {
        resultBlocks.push({
          type: "tool_result",
          tool_use_id: call.id,
          content: call.result.content,
          is_error: call.result.isError,
        });
      }
    }

    const frozenText = streamingText.value;
    const frozenBlocks = JSON.stringify(blocks);
    messages.value = messages.value.map((msg, i) =>
      i === lastIdx ? { ...msg, content: frozenText, content_blocks: frozenBlocks } : msg,
    );
    if (resultBlocks.length > 0) {
      messages.value.push({
        id: "toolresult-" + Date.now(),
        conversation_id: activeConvId.value ?? "",
        role: "user",
        content: "",
        content_blocks: JSON.stringify(resultBlocks),
        token_count: null,
        error: null,
        created_at: new Date().toISOString(),
        rowid: 0,
        model: null,
      });
    }
  }

  function initEvents() {
    if (inited) return;
    inited = true;

    listen<ChatStartPayload>("chat:start", (e) => {
      if (e.payload.conversation_id !== activeConvId.value) return;
      messages.value.push({
        id: e.payload.assistant_message_id,
        conversation_id: e.payload.conversation_id,
        role: "assistant",
        content: "",
        content_blocks: "[]",
        token_count: null,
        error: null,
        created_at: new Date().toISOString(),
        rowid: 0,
        model: currentModel.value,
      });
    });

    // 多轮工具调用：每轮工具执行完毕后，后端创建下一轮 assistant 占位并 emit。
    // 前端据此冻结上一条 assistant（写入 tool_use/text/thinking）+ 插入 user(tool_result)
    // + 重置 streaming 状态 + push 新占位。
    listen<ChatAssistantStartPayload>("chat:assistant-start", (e) => {
      if (e.payload.conversation_id !== activeConvId.value) return;
      resetSendTimeout();
      freezeCurrentAssistant();
      streamingText.value = "";
      streamingThinking.value = "";
      thinkingStartTime.value = null;
      streamingToolCalls.value = new Map();
      messages.value.push({
        id: e.payload.message_id,
        conversation_id: e.payload.conversation_id,
        role: "assistant",
        content: "",
        content_blocks: "[]",
        token_count: null,
        error: null,
        created_at: new Date().toISOString(),
        rowid: 0,
        model: currentModel.value,
      });
    });

    listen<ChatChunkPayload>("chat:chunk", (e) => {
      const cid = e.payload.conversation_id;
      if (cid !== activeConvId.value) {
        // 后台会话：累积文本到快照，切回时恢复（不触活跃 UI / messages）
        const m = new Map(bgStreams.value);
        const cur = m.get(cid) ?? { text: "", thinking: "" };
        cur.text += e.payload.delta;
        m.set(cid, cur);
        bgStreams.value = m;
        return;
      }
      resetSendTimeout();
      streamingText.value += e.payload.delta;
      const idx = messages.value.length - 1;
      if (idx >= 0 && messages.value[idx].role === "assistant") {
        messages.value[idx] = { ...messages.value[idx], content: streamingText.value };
      }
    });

    // ---- 工具调用事件 ----
    listen<ChatToolCallStartPayload>("chat:tool-call-start", (e) => {
      if (e.payload.conversation_id !== activeConvId.value) return;
      resetSendTimeout();
      const map = new Map(streamingToolCalls.value);
      map.set(e.payload.id, {
        id: e.payload.id,
        name: e.payload.name,
        arguments: "",
        ended: false,
      });
      streamingToolCalls.value = map;
    });

    listen<ChatToolCallDeltaPayload>("chat:tool-call-delta", (e) => {
      if (e.payload.conversation_id !== activeConvId.value) return;
      resetSendTimeout();
      const map = new Map(streamingToolCalls.value);
      const call = map.get(e.payload.id);
      if (call) {
        call.arguments += e.payload.delta;
        map.set(e.payload.id, { ...call });
        streamingToolCalls.value = map;
      }
    });

    listen<ChatToolCallEndPayload>("chat:tool-call-end", (e) => {
      if (e.payload.conversation_id !== activeConvId.value) return;
      resetSendTimeout();
      const map = new Map(streamingToolCalls.value);
      const call = map.get(e.payload.id);
      if (call) {
        call.ended = true;
        map.set(e.payload.id, { ...call });
        streamingToolCalls.value = map;
      }
    });

    listen<ChatToolResultPayload>("chat:tool-result", (e) => {
      if (e.payload.conversation_id !== activeConvId.value) return;
      resetSendTimeout();
      const map = new Map(streamingToolCalls.value);
      const call = map.get(e.payload.tool_use_id);
      if (call) {
        call.result = { content: e.payload.content, isError: e.payload.is_error };
        map.set(e.payload.tool_use_id, { ...call });
        streamingToolCalls.value = map;
      }
    });

    // ---- 思考过程 ----
    listen<ChatThinkingPayload>("chat:thinking", (e) => {
      const cid = e.payload.conversation_id;
      if (cid !== activeConvId.value) {
        const m = new Map(bgStreams.value);
        const cur = m.get(cid) ?? { text: "", thinking: "" };
        cur.thinking += e.payload.content;
        m.set(cid, cur);
        bgStreams.value = m;
        return;
      }
      resetSendTimeout();
      if (!streamingThinking.value) {
        thinkingStartTime.value = Date.now();
      }
      streamingThinking.value += e.payload.content;
    });

    listen<ChatDonePayload>("chat:done", (e) => {
      const cid = e.payload.conversation_id;
      if (cid !== activeConvId.value) {
        // 后台会话完成：后端已把最终态落库，清掉快照即可（不触活跃 UI，无需前端 freeze）
        const m = new Map(bgStreams.value);
        m.delete(cid);
        bgStreams.value = m;
        recentErrorConvs.delete(cid);
        return;
      }
      // 错误后的配套 chat:done(abort)：chat:error 已是终态（写了错误文案 + 重置），
      // 这里只做最小收尾——不 freeze（会覆盖错误文案）、不设 lastFinishReason（会误显「已手动停止」）。
      if (recentErrorConvs.has(cid)) {
        recentErrorConvs.delete(cid);
        if (sendTimeout) { clearTimeout(sendTimeout); sendTimeout = null; }
        sending.value = false;
        streamingText.value = "";
        streamingToolCalls.value = new Map();
        streamingThinking.value = "";
        thinkingStartTime.value = null;
        return;
      }
      if (sendTimeout) { clearTimeout(sendTimeout); sendTimeout = null; }

      // 记录思考耗时与内容
      if (thinkingStartTime.value) {
        const elapsed = Math.floor((Date.now() - thinkingStartTime.value) / 1000);
        const dur = elapsed < 60 ? `${elapsed}s` : `${Math.floor(elapsed / 60)}m ${elapsed % 60}s`;
        thinkingDuration.value = dur;
        lastThinkingContent.value = streamingThinking.value || null;
        const asstMsgId = e.payload.message_id;
        if (asstMsgId) {
          const map = new Map(thinkingDurations.value);
          map.set(asstMsgId, dur);
          thinkingDurations.value = map;
        }
      }

      // 冻结最后一条 assistant（把本轮 streaming 文本/思考/工具调用写入其 content_blocks，
      // tool_result 分离为独立 user 消息）。替代旧的「打包全部 streamingToolCalls 进末条」
      // —— 那会把 tool_result 也塞进 assistant，违反 Anthropic 协议。
      freezeCurrentAssistant();

      // M3：abort 时若目标 assistant 在 freeze 后仍为空（无内容无 blocks，后端已删 DB 行），
      // 前端同步移除，避免残留空气泡。
      if (e.payload.finish_reason === "abort") {
        const doneId = e.payload.message_id;
        messages.value = messages.value.filter(
          (m) => !(m.id === doneId && m.role === "assistant" && !m.content && (!m.content_blocks || m.content_blocks === "[]")),
        );
      }

      sending.value = false;
      streamingText.value = "";
      streamingToolCalls.value = new Map();
      streamingThinking.value = "";
      thinkingStartTime.value = null;
      lastFinishReason.value = e.payload.finish_reason;
      // 用 message_id 定位最终 assistant（freezeCurrentAssistant 可能已在末尾插入 user 消息，
      // 不能再假设末条索引），更新其 token_count
      if (e.payload.usage) {
        const doneId = e.payload.message_id;
        messages.value = messages.value.map((msg) =>
          msg.id === doneId && msg.role === "assistant"
            ? { ...msg, token_count: e.payload.usage!.completion_tokens }
            : msg,
        );
      }
    });

    listen<ChatErrorPayload>("chat:error", (e) => {
      const cid = e.payload.conversation_id;
      // 标记：后端会紧跟一次 chat:done(abort) 收尾，chat:done 据此跳过 freeze + lastFinishReason
      recentErrorConvs.add(cid);
      if (cid !== activeConvId.value) {
        // 后台会话出错：清掉快照（用户切回时走 DB 加载，看到错误态）
        const m = new Map(bgStreams.value);
        m.delete(cid);
        bgStreams.value = m;
        return;
      }
      if (sendTimeout) { clearTimeout(sendTimeout); sendTimeout = null; }
      sending.value = false;
      streamingText.value = "";
      streamingThinking.value = "";
      thinkingStartTime.value = null;
      streamingToolCalls.value = new Map();
      lastFinishReason.value = null;
      // 用 message_id 定位出错的 assistant（多轮工具下不能遍历改所有 assistant）
      const errId = e.payload.message_id;
      messages.value = messages.value.map((msg) => {
        if (msg.id === errId && msg.role === "assistant") {
          if (msg.content === "") {
            return { ...msg, content: `错误: ${e.payload.message}`, error: e.payload.message };
          }
          if (!msg.error) {
            return { ...msg, content: msg.content + `\n\n[生成中断: ${e.payload.message}]`, error: e.payload.message };
          }
        }
        return msg;
      });
    });

    // ---- 工具授权请求 ----
    listen<ToolAuthRequestPayload>("chat:tool-auth-request", (e) => {
      if (e.payload.conversation_id !== activeConvId.value) return;
      pendingAuthRequest.value = e.payload;
    });
  }

  // ===== 新建会话 =====
  async function createConversation(agentId: string) {
    const conv = await bridge.conversations.create(agentId);
    // 未置顶的新会话插入到所有置顶会话之后、第一个未置顶之前
    const firstUnpinned = conversations.value.findIndex((c) => !c.pinned);
    if (firstUnpinned === -1) {
      conversations.value.push(conv);
    } else {
      conversations.value.splice(firstUnpinned, 0, conv);
    }
    selectConversation(conv.id);
    return conv;
  }

  function reset() {
    if (sendTimeout) { clearTimeout(sendTimeout); sendTimeout = null; }
    conversations.value = [];
    activeConvId.value = null;
    messages.value = [];
    sending.value = false;
    streamingText.value = "";
    streamingThinking.value = "";
    thinkingStartTime.value = null;
    thinkingDuration.value = null;
    lastThinkingContent.value = null;
    bgStreams.value = new Map();
    draftText.value = "";
  }

  /** 当前正在生成的会话 ID 集合（激活的流式会话 + 后台流式会话 bgStreams）。
   *  侧栏据此给会话卡片显示「生成中」状态 + 动画——现在每个会话独立监控，可精确到单卡。*/
  const streamingConvIds = computed(() => {
    const ids = new Set<string>(bgStreams.value.keys());
    if (sending.value && activeConvId.value) ids.add(activeConvId.value);
    return ids;
  });

  return {
    conversations, convLoading,
    activeConvId, activeConversation,
    messages, msgLoading, hasMore, loadingMore,
    sending, streamingText, draftText, pendingImages, lastFinishReason, currentModel,
    streamingToolCalls, streamingThinking, thinkingStartTime, thinkingDuration, lastThinkingContent, thinkingDurations, pendingAuthRequest,
    streamingConvIds,
    loadConversations, selectConversation, loadMoreMessages,
    sendMessage, stopGeneration, respondToAuth,
    deleteConversation, pinConversation,
    initEvents, createConversation, reset,
  };
});
