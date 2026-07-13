// IcePaw 聊天状态管理 Store
//
// 职责：
//   1. 维护当前会话的消息列表（messages）
//   2. 维护流式生成状态（isStreaming + streamingContent）
//   3. 订阅 Rust 侧 4 个 chat:* 事件，驱动本地状态更新
//   4. 提供 sendMessage / stopGeneration / loadMessages 三个核心 action
//
// 设计要点：
//   - Composition API 风格（与 stores/agents.ts / stores/conversations.ts 一致）
//   - 监听器持久化：setupListeners() 在 ChatPage onMounted 时调一次，组件销毁时
//     调 teardownListeners()，避免内存泄漏（plan §3.2.2）。
//   - 事件按 conversation_id 过滤：只有当前 store 关心的会话事件才会更新本地状态，
//     避免切换会话时被旧流污染。
//   - 乐观插入：sendMessage 先在本地插入一条 user 消息（id 以 "__temp_user_" 前缀
//     标记），chat:start 到达时用真实 user_message_id 替换。
//   - 助手消息同样在 chat:start 时插入占位（content=""），chat:chunk 时累计 delta，
//     chat:done 时清空 streamingContent + 解除活跃态。
//   - 错误处理：chat:error 把 message.error 写入助手消息并清流式状态。
//
// 注意：
//   - streamingContent 仅作为响应式累加器用于内部状态；真正展示用的实时内容
//     已由 chat:chunk 写入 messages 中对应助手行的 content 字段。
//   - "currentMessages" getter 直接返回 messages.value。早期版本会附加一条
//     `id: "__streaming_virtual__"` 的虚拟助手行作为「chat:start 晚于 chat:chunk
//     抵达」的兜底，但与 messages 中已存在的真实助手行同时渲染会导致 **两条
//     AI 气泡并存** 的回归，已移除（详见该 getter 上的注释）。

import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { bridge } from "../api/bridge";
import { useConversationsStore } from "./conversations";
import type {
  ChatChunkPayload,
  ChatDonePayload,
  ChatErrorPayload,
  ChatRetryingPayload,
  ChatStartPayload,
  Message,
} from "../types";

// ============================================================================
// 内部常量
// ============================================================================

/** 乐观插入的用户消息 ID 前缀（chat:start 时替换为真实 ID） */
const TEMP_USER_PREFIX = "__temp_user_";

/** 监听器是否已注册（防止重复 setup） */
let listenersRegistered = false;

// ============================================================================
// store
// ============================================================================

/**
 * 聊天 Store
 *
 * state:
 *   - messages          当前会话的消息列表（不含流式占位行）
 *   - isStreaming       是否正在生成
 *   - streamingContent  流式增量累积（内部累加器，用于 chat:done 时的统计/调试）
 *   - activeConvId      正在生成的会话 ID（用于事件过滤）
 *   - loading           加载历史消息状态
 *   - error             最近错误描述（供 Toast 显示）
 *
 * getters:
 *   - currentMessages   当前显示的消息列表（= messages.value；真实助手行由
 *                       chat:start 插入、流式期间由 chat:chunk 实时累加）
 *   - lastMessage       最后一条消息
 *   - isEmpty           列表为空且非流中
 *
 * actions:
 *   - setupListeners()        注册 4 个 chat:* 事件监听（幂等）
 *   - teardownListeners()     注销所有监听（组件销毁时调用）
 *   - loadMessages(convId)    加载某会话的历史消息
 *   - sendMessage(content)    发送用户消息并触发流式生成
 *   - stopGeneration()        主动停止当前会话的流式生成
 *   - clearError()            清空 error 字段
 */
export const useChatStore = defineStore("chat", () => {
  // ============================================================================
  // state
  // ============================================================================

  /** 当前会话的消息列表（响应式） */
  const messages = ref<Message[]>([]);

  /** 是否处于流式生成中 */
  const isStreaming = ref<boolean>(false);

  /** 流式内容累加器（chat:chunk 时 append） */
  const streamingContent = ref<string>("");

  /** 当前正在生成的会话 ID（用于事件过滤；null 表示无活跃生成） */
  const activeConvId = ref<string | null>(null);

  /** 加载历史消息状态 */
  const loading = ref<boolean>(false);

  /** 最近一次错误描述（供 Toast 显示） */
  const error = ref<string | null>(null);

  /** 是否正在重试中（LLM 流式中断后自动重试） */
  const retrying = ref<boolean>(false);

  /** 当前重试进度："1/4" 格式 */
  const retryProgress = ref<string>("1/4");

  /** listen 返回的 unlisten 函数列表 */
  const unlistens: UnlistenFn[] = [];

  // ============================================================================
  // 内部工具
  // ============================================================================

  /**
   * 替换乐观插入的 user 消息 ID 为真实 ID。
   * 仅替换当前 messages 数组中第一个以 TEMP_USER_PREFIX 开头的项。
   */
  function adoptTempUserMessage(realId: string): void {
    const idx = messages.value.findIndex((m) => m.id.startsWith(TEMP_USER_PREFIX));
    if (idx >= 0) {
      const old = messages.value[idx];
      messages.value.splice(idx, 1, { ...old, id: realId });
    }
  }

  /**
   * 按 ID 查找消息在 messages.value 中的索引。
   * 未找到返回 -1。
   */
  function indexOfMessage(id: string): number {
    return messages.value.findIndex((m) => m.id === id);
  }

  /**
   * 在 messages 中追加一条助手消息占位（content=""）。
   * 若已存在同 ID 行则跳过（避免 chat:start 重复触发）。
   */
  function appendAssistantPlaceholder(msg: Message): void {
    if (indexOfMessage(msg.id) >= 0) return;
    messages.value = [...messages.value, msg];
  }

  /**
   * 往 messages 中指定 ID 行的 content 累加 delta。
   * 若行不存在则静默忽略（边界情况：chunk 比 start 先到，由 Rust 保证不会发生）。
   */
  function appendDeltaToMessage(id: string, delta: string): void {
    const idx = indexOfMessage(id);
    if (idx < 0) return;
    const old = messages.value[idx];
    messages.value.splice(idx, 1, { ...old, content: old.content + delta });
  }

  /**
   * 在 messages 中把指定 ID 行标记为错误。
   * 若行不存在则静默忽略。
   */
  function markMessageError(id: string, errMsg: string): void {
    const idx = indexOfMessage(id);
    if (idx < 0) return;
    const old = messages.value[idx];
    messages.value.splice(idx, 1, { ...old, error: errMsg });
  }

  // ============================================================================
  // getters
  // ============================================================================

  /**
   * 当前显示用的消息列表。
   * 直接返回 messages.value —— chat:start 已经在 messages 中插入助手占位
   * （content=""），chat:chunk 实时累加 content，无需再附加虚拟行。
   *
   * 历史注释：本 getter 早期会附加 `id: "__streaming_virtual__"` 的虚拟助手
   * 行作为「chat:start 晚于 chat:chunk 抵达」的兜底，但实测发现该虚拟行与
   * messages.value 中已存在的真实助手行同时渲染，导致 **两条 AI 气泡并存**
   * 的回归（见修复 commit）。当前实现以「chat:start 必先于 chat:chunk」为
   * 不变量；为消除理论上的竞态，chat:start 处理器会在收到时清空
   * streamingContent，确保累加器从空开始。
   */
  const currentMessages = computed<Message[]>(() => messages.value);

  /** 最后一条消息 */
  const lastMessage = computed<Message | null>(() => {
    const list = messages.value;
    return list.length > 0 ? (list[list.length - 1] ?? null) : null;
  });

  /** 是否为空（无消息且非流中） */
  const isEmpty = computed<boolean>(() => messages.value.length === 0 && !isStreaming.value);

  // ============================================================================
  // actions
  // ============================================================================

  /**
   * 注册 4 个 chat:* 事件监听。
   * 幂等：重复调用安全。
   * 由 ChatPage.vue 在 onMounted 时调用一次。
   */
  async function setupListeners(): Promise<void> {
    if (listenersRegistered) return;
    listenersRegistered = true;

    // ----- chat:start -----
    unlistens.push(
      await listen<ChatStartPayload>("chat:start", (e) => {
        const p = e.payload;
        if (p.conversation_id !== activeConvId.value) return;

        // 校正乐观插入的 user 消息 ID
        adoptTempUserMessage(p.user_message_id);

        // 追加助手消息占位
        appendAssistantPlaceholder({
          id: p.assistant_message_id,
          conversation_id: p.conversation_id,
          role: "assistant",
          content: "",
          token_count: null,
          error: null,
          created_at: new Date().toISOString(),
        });

        // 置流式状态
        isStreaming.value = true;
        streamingContent.value = "";
        error.value = null;
        retrying.value = false;
        retryProgress.value = "1/4";
      }),
    );

    // ----- chat:chunk -----
    unlistens.push(
      await listen<ChatChunkPayload>("chat:chunk", (e) => {
        const p = e.payload;
        if (p.conversation_id !== activeConvId.value) return;

        streamingContent.value += p.delta;
        appendDeltaToMessage(p.message_id, p.delta);
      }),
    );

    // ----- chat:done -----
    unlistens.push(
      await listen<ChatDonePayload>("chat:done", (e) => {
        const p = e.payload;
        if (p.conversation_id !== activeConvId.value) return;

        isStreaming.value = false;
        streamingContent.value = "";
        activeConvId.value = null;
        retrying.value = false;
      }),
    );

    // ----- chat:error -----
    unlistens.push(
      await listen<ChatErrorPayload>("chat:error", (e) => {
        const p = e.payload;
        if (p.conversation_id !== activeConvId.value) return;

        error.value = p.message;
        retrying.value = false;
        retryProgress.value = "1/4";
        markMessageError(p.message_id, p.message);
        isStreaming.value = false;
        streamingContent.value = "";
        activeConvId.value = null;
      }),
    );

    // ----- chat:retrying -----
    unlistens.push(
      await listen<ChatRetryingPayload>("chat:retrying", (e) => {
        const p = e.payload;
        if (p.conversation_id !== activeConvId.value) return;

        retrying.value = true;
        retryProgress.value = `${p.attempt}/${p.max_attempts}`;
      }),
    );
  }

  /**
   * 注销全部 chat:* 事件监听。
   * 由 ChatPage.vue 在 onUnmounted 时调用，避免内存泄漏。
   */
  function teardownListeners(): void {
    while (unlistens.length > 0) {
      const fn = unlistens.pop();
      try {
        fn?.();
      } catch {
        // 忽略单个 unlisten 失败
      }
    }
    listenersRegistered = false;
  }

  /**
   * 加载某会话的历史消息。
   * - 若传入空字符串 / null，则清空 messages 并返回
   * - 若当前正在流式生成且目标会话不同，强制清流式状态（避免旧会话的 chunk 持续污染）
   * - 拉取失败写入 error 字段，保留旧列表
   *
   * @param conversationId 目标会话 ID
   */
  async function loadMessages(conversationId: string | null): Promise<void> {
    if (!conversationId) {
      messages.value = [];
      error.value = null;
      return;
    }

    // 切换会话：清流式状态
    if (isStreaming.value && activeConvId.value !== conversationId) {
      isStreaming.value = false;
      streamingContent.value = "";
      activeConvId.value = null;
    }

    loading.value = true;
    error.value = null;
    try {
      const list = await bridge.messages.list(conversationId, { limit: 100 });
      messages.value = list;
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    } finally {
      loading.value = false;
    }
  }

  /**
   * 发送用户消息并触发 Rust 流式生成。
   * 流程：
   *   1. 校验非空 + 校验当前有会话 + 校验非流中
   *   2. 乐观插入 user 消息到 messages（id 以 TEMP_USER_PREFIX 开头）
   *   3. 调 bridge.chat.sendMessage（成功则由 chat:start 校正 ID）
   *   4. 失败：回滚乐观插入 + 写入 error
   *
   * @param content 用户输入文本（会做 trim）
   */
  async function sendMessage(content: string): Promise<void> {
    const trimmed = content.trim();
    if (!trimmed) return;

    if (isStreaming.value) {
      // 守门：流式中不允许再次发送
      return;
    }

    const convStore = useConversationsStore();
    const convId = convStore.currentId;
    if (!convId) {
      error.value = "没有当前会话，无法发送";
      return;
    }

    error.value = null;
    activeConvId.value = convId;

    // 乐观插入 user 消息
    const tempId = `${TEMP_USER_PREFIX}${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
    const optimistic: Message = {
      id: tempId,
      conversation_id: convId,
      role: "user",
      content: trimmed,
      token_count: null,
      error: null,
      created_at: new Date().toISOString(),
    };
    messages.value = [...messages.value, optimistic];

    try {
      await bridge.chat.sendMessage(convId, trimmed);
      // 成功：保持乐观插入，由 chat:start 替换真实 ID
    } catch (err) {
      // 失败：回滚
      messages.value = messages.value.filter((m) => m.id !== tempId);
      error.value = err instanceof Error ? err.message : String(err);
      activeConvId.value = null;
      throw err;
    }
  }

  /**
   * 主动停止当前会话的流式生成。
   * 乐观行为：立即清流式状态 + activeConvId；同时调 bridge.chat.stopGeneration
   * 通知 Rust 触发 CancellationToken（实际中断由后续 chat:done / chat:error 体现）。
   *
   * 无活跃生成或无当前会话时为 no-op。
   */
  async function stopGeneration(): Promise<void> {
    if (!isStreaming.value) return;

    const convStore = useConversationsStore();
    const convId = activeConvId.value ?? convStore.currentId;
    if (!convId) return;

    // 乐观清理
    isStreaming.value = false;
    streamingContent.value = "";
    activeConvId.value = null;

    try {
      await bridge.chat.stopGeneration(convId);
    } catch {
      // 静默：Rust 侧日志已记录
    }
  }

  /** 清空 error 字段（Toast 显示完调一下） */
  function clearError(): void {
    error.value = null;
  }

  // ============================================================================
  // 返回
  // ============================================================================

  return {
    // state
    messages,
    isStreaming,
    streamingContent,
    activeConvId,
    loading,
    error,
    retrying,
    retryProgress,
    // getters
    currentMessages,
    lastMessage,
    isEmpty,
    // actions
    setupListeners,
    teardownListeners,
    loadMessages,
    sendMessage,
    stopGeneration,
    clearError,
  };
});
