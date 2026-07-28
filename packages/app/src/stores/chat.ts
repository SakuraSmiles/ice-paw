// 聊天状态管理（会话列表 + 当前会话 + 消息 + 流式事件）
// 侧栏不再按 Agent 过滤，显示全部会话混合列表
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { Conversation, Message } from "../types";
import type {
  ChatStartPayload,
  ChatChunkPayload,
  ChatDonePayload,
  ChatErrorPayload,
} from "../types";
import { bridge } from "../api/bridge";

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
    sending.value = false;
    streamingText.value = "";
    activeConvId.value = id;
    loadMessages(id);
  }

  // ===== 消息 =====
  const messages = ref<Message[]>([]);
  const msgLoading = ref(false);

  async function loadMessages(convId: string) {
    msgLoading.value = true;
    try {
      messages.value = await bridge.messages.list(convId);
    } catch (e) {
      console.error("加载消息列表失败:", e);
    } finally {
      msgLoading.value = false;
    }
  }

  // ===== 输入框草稿（跨页面切换保持） =====
  const draftText = ref("");

  // ===== 流式发送 =====
  const sending = ref(false);
  const streamingText = ref("");
  let sendTimeout: ReturnType<typeof setTimeout> | null = null;

  async function sendMessage(content: string) {
    if (!activeConvId.value || sending.value) return;
    sending.value = true;
    streamingText.value = "";

    const userMsg: Message = {
      id: "user-" + Date.now(),
      conversation_id: activeConvId.value,
      role: "user",
      content,
      content_blocks: "[]",
      token_count: null,
      error: null,
      created_at: new Date().toISOString(),
      rowid: 0,
      model: null,
    };
    messages.value = [...messages.value, userMsg];

    // 前端超时保护：60 秒无响应自动重置
    sendTimeout = setTimeout(() => {
      if (sending.value) {
        console.warn("发送超时（60s），自动重置发送状态");
        sending.value = false;
        streamingText.value = "";
      }
    }, 60000);

    try {
      await bridge.chat.sendMessage(activeConvId.value, content);
    } catch (e) {
      console.error("发送消息失败:", e);
      sending.value = false;
      streamingText.value = "";
    }
  }

  async function stopGeneration() {
    if (!activeConvId.value) return;
    if (sendTimeout) { clearTimeout(sendTimeout); sendTimeout = null; }
    // 乐观重置发送状态，不依赖后端事件响应
    sending.value = false;
    streamingText.value = "";
    try {
      await bridge.chat.stopGeneration(activeConvId.value);
    } catch { /* 静默忽略 */ }
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
        model: null,
      });
    });

    listen<ChatChunkPayload>("chat:chunk", (e) => {
      if (e.payload.conversation_id !== activeConvId.value) return;
      streamingText.value += e.payload.delta;
      const idx = messages.value.length - 1;
      if (idx >= 0 && messages.value[idx].role === "assistant") {
        messages.value = messages.value.map((msg, i) =>
          i === idx ? { ...msg, content: streamingText.value } : msg,
        );
      }
    });

    listen<ChatDonePayload>("chat:done", (e) => {
      if (e.payload.conversation_id !== activeConvId.value) return;
      if (sendTimeout) { clearTimeout(sendTimeout); sendTimeout = null; }
      sending.value = false;
      streamingText.value = "";
      // 不 reload 消息，避免闪烁
    });

    listen<ChatErrorPayload>("chat:error", (e) => {
      if (e.payload.conversation_id !== activeConvId.value) return;
      if (sendTimeout) { clearTimeout(sendTimeout); sendTimeout = null; }
      sending.value = false;
      streamingText.value = "";
      messages.value = messages.value.map((msg) => {
        if (msg.role === "assistant" && msg.content === "") {
          return { ...msg, content: `错误: ${e.payload.message}`, error: e.payload.message };
        }
        if (msg.role === "assistant" && msg.content !== "" && !msg.error) {
          return { ...msg, content: msg.content + `\n\n[生成中断: ${e.payload.message}]`, error: e.payload.message };
        }
        return msg;
      });
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
    draftText.value = "";
  }

  return {
    conversations, convLoading,
    activeConvId, activeConversation,
    messages, msgLoading,
    sending, streamingText, draftText,
    loadConversations, selectConversation,
    sendMessage, stopGeneration,
    deleteConversation, pinConversation,
    initEvents, createConversation, reset,
  };
});
