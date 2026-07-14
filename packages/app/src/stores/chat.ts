// IcePaw 聊天状态管理 Store
//
// 职责：
//   1. 维护当前会话的消息列表（messages）
//   2. 维护流式生成状态（isStreaming + streamingContent）
//   3. 订阅 Rust 侧 4 个 chat:* 事件，驱动本地状态更新
//   4. 提供 sendMessage / stopGeneration / loadMessages / loadOlderMessages
//
// 设计要点：
//   - Composition API 风格（与 stores/agents.ts / stores/conversations.ts 一致）
//   - 监听器持久化：setupListeners() 在 ChatPage onMounted 时调一次，组件销毁时
//     调 teardownListeners()，避免内存泄漏（plan §3.2.2）。
//   - 事件按 conversation_id 过滤：只有当前 store 关心的会话事件才会更新本地状态，
//     避免切换会话时被旧流污染。
//   - 乐观插入：sendMessage 先在本地插入一条 user 消息（id 以 "__temp_user_" 前缀
//     标记，rowid 哨兵值 -1），chat:start 到达时用真实 user_message_id 替换。
//   - 助手消息同样在 chat:start 时插入占位（content=""，rowid=-1），
//     chat:chunk 时累计 delta，chat:done 时清空 streamingContent + 解除活跃态。
//   - 错误处理：chat:error 把 message.error 写入助手消息并清流式状态。
//   - 分页（P2）：loadMessages 只取最近 20 条；loadOlderMessages 向上翻页，
//     复合游标 (created_at, rowid) 详见 icepaw-chat-perf-design.md §3.3。
//     滚动位置补偿由 MessageList 组件负责（store 不感知 DOM）。
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
import { useToast } from "../composables/useToast";
import type {
  ChatChunkPayload,
  ChatDonePayload,
  ChatErrorPayload,
  ChatRetryingPayload,
  ChatStartPayload,
  ChatThinkingPayload,
  ChatToolCallDeltaPayload,
  ChatToolCallEndPayload,
  ChatToolCallStartPayload,
  ChatToolResultPayload,
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
// 分页常量（P2 性能优化 — 详见 icepaw-chat-perf-design.md §3.3）
// ============================================================================

/** 初始加载的消息数（首屏只取最近 20 条，体验：立即可见 + 滚动到底部） */
const INITIAL_PAGE_SIZE = 20;

/** 向上翻页每次加载的消息数（用户在顶部附近时触发） */
const OLDER_PAGE_SIZE = 20;

/**
 * 乐观插入消息 / chat:start 占位消息使用的 rowid 哨兵值。
 *
 * 含义：消息存在于本地 store 中，但尚未从 DB 拉取真实 rowid。
 * 影响：不影响渲染；不影响流式逻辑；不影响「向上翻页」游标计算
 *       （游标取自 messages[0] 即最早一条 DB 消息，sentinel 消息必在尾部）。
 * 详见设计文档 §6.3。
 */
const ROWID_SENTINEL = -1;

/** 发送时附带的模板信息（P2-4 模板） */
export interface AppliedTemplate {
  templateId: string;
  values: Record<string, string>;
}

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
 *   - loading           加载历史消息状态（loadMessages in-flight）
 *   - hasMoreOlder      是否还有更早的历史可加载（向上翻页用）
 *   - loadingOlder      向上翻页 in-flight 标志
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
 *   - loadMessages(convId)    加载某会话最近 20 条 + 重置分页状态
 *   - loadOlderMessages()     向上翻页（OLDER_PAGE_SIZE=20，prepend）
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

  /**
   * 是否还有更早的历史消息可加载（向上翻页用）。
   * - 初始 true（保守假设有更多历史）
   * - `loadMessages` 加载后，若返回条数 < INITIAL_PAGE_SIZE 置 false
   * - `loadOlderMessages` 同理：剩余条数 < OLDER_PAGE_SIZE 置 false
   */
  const hasMoreOlder = ref<boolean>(true);

  /**
   * 是否正在加载更早的历史（向上翻页 in-flight）。
   * 防止快速滚到顶部触发重复加载。
   */
  const loadingOlder = ref<boolean>(false);

  /** 最近一次错误描述（供 Toast 显示） */
  const error = ref<string | null>(null);

  /** 是否正在重试中（LLM 流式中断后自动重试） */
  const retrying = ref<boolean>(false);

  /** 当前重试进度："1/4" 格式 */
  const retryProgress = ref<string>("1/4");

  /** P2-3: 最近一次流式完成的 token usage */
  const lastUsage = ref<{
    prompt_tokens: number;
    completion_tokens: number;
    cached_tokens: number;
  } | null>(null);

  /** listen 返回的 unlisten 函数列表 */
  const unlistens: UnlistenFn[] = [];

  // ============================================================================
  // P2-1 工具调用状态
  // ============================================================================

  /** 当前活跃的工具调用（id → 状态信息） */
  interface ActiveToolCall {
    id: string;
    name: string;
    /** 累积的参数 JSON 片段 */
    argumentsBuffer: string;
    /** 是否已完成（收到 tool-call-end） */
    ended: boolean;
  }

  /** 当前轮次的工具调用列表 */
  const activeToolCalls = ref<ActiveToolCall[]>([]);

  /** 当前轮次的思考过程内容累积 */
  const thinkingContent = ref<string>("");

  /** 是否启用工具调用 */
  const toolsEnabled = ref<boolean>(false);

  /**
   * 已应用的模板（下次 sendMessage 时传给后端）。
   * - null 表示本次发送不携带模板
   * - 输入框内容被用户修改后会自动清空（由 ChatInput / WelcomeInput 主动调）
   */
  const appliedTemplate = ref<AppliedTemplate | null>(null);

  /**
   * 工具调用结果映射（tool_use_id → 结果信息）。
   * 流式中通过 chat:tool-result 事件填充，
   * 流式结束后在 chat:done / chat:error 中清空。
   *
   * 该 map 同时供实时 ToolCallBlock 展示使用：
   * 流式期间 tool-call-end 到达 → ended=true，
   * 随后 tool-result 到达 → 进入此 map，前端即可将 result / isError 透传给组件。
   */
  interface ToolResultEntry {
    content: string;
    isError: boolean;
  }

  const toolResults = ref<Record<string, ToolResultEntry>>({});

  // ============================================================================
  // 内部工具
  // ============================================================================

  /**
   * 替换乐观插入的 user 消息 ID 为真实 ID。
   * 仅替换当前 messages 数组中第一个以 TEMP_USER_PREFIX 开头的项。
   * 注：chat:start payload 不含 rowid，故替换后 rowid 仍保留 ROWID_SENTINEL。
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
          content_blocks: "[]",
          token_count: null,
          error: null,
          created_at: new Date().toISOString(),
          // 占位消息尚无 DB rowid，使用哨兵；流式结束后也不会刷新
          // （游标永远取 messages[0]，sentinel 必在尾部，不影响分页）。
          rowid: ROWID_SENTINEL,
        });

        // 置流式状态
        isStreaming.value = true;
        streamingContent.value = "";
        error.value = null;
        retrying.value = false;
        retryProgress.value = "1/4";
        // P2-1: 重置工具调用和思考状态
        activeToolCalls.value = [];
        thinkingContent.value = "";
        // P2-1: 重置工具调用结果映射
        toolResults.value = {};
        // P2-3: 清除上一轮的 usage
        lastUsage.value = null;
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
        // P2-1: 清理工具调用状态
        activeToolCalls.value = [];
        thinkingContent.value = "";
        // P2-1: 清理工具调用结果
        toolResults.value = {};
        // P2-3: 记录 token usage
        lastUsage.value = p.usage ?? null;
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
        // P2-1: 清理工具调用状态
        activeToolCalls.value = [];
        thinkingContent.value = "";
        // P2-1: 清理工具调用结果
        toolResults.value = {};

        // P2-3: 通过全局 Toast 醒目提示错误（后端已做友好化映射）
        // kind="cancelled" 是用户主动停止，不打扰；其他错误用 error 级别
        if (p.kind !== "cancelled") {
          try {
            const toast = useToast();
            toast.error(p.message, { duration: 5000 });
          } catch {
            // toast composable 在 SSR / 非浏览器环境下可能不可用，静默忽略
          }
        }
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

    // ----- P2-1: chat:tool-call-start -----
    unlistens.push(
      await listen<ChatToolCallStartPayload>("chat:tool-call-start", (e) => {
        const p = e.payload;
        if (p.conversation_id !== activeConvId.value) return;

        activeToolCalls.value = [
          ...activeToolCalls.value,
          {
            id: p.id,
            name: p.name,
            argumentsBuffer: "",
            ended: false,
          },
        ];
      }),
    );

    // ----- P2-1: chat:tool-call-delta -----
    unlistens.push(
      await listen<ChatToolCallDeltaPayload>("chat:tool-call-delta", (e) => {
        const p = e.payload;
        if (p.conversation_id !== activeConvId.value) return;

        const idx = activeToolCalls.value.findIndex((tc) => tc.id === p.id);
        if (idx >= 0) {
          const tc = activeToolCalls.value[idx];
          activeToolCalls.value.splice(idx, 1, {
            ...tc,
            argumentsBuffer: tc.argumentsBuffer + p.delta,
          });
        }
      }),
    );

    // ----- P2-1: chat:tool-call-end -----
    unlistens.push(
      await listen<ChatToolCallEndPayload>("chat:tool-call-end", (e) => {
        const p = e.payload;
        if (p.conversation_id !== activeConvId.value) return;

        const idx = activeToolCalls.value.findIndex((tc) => tc.id === p.id);
        if (idx >= 0) {
          const tc = activeToolCalls.value[idx];
          activeToolCalls.value.splice(idx, 1, { ...tc, ended: true });
        }
      }),
    );

    // ----- P2-1: chat:tool-result -----
    // 收到此事件时表示工具已执行完成。content 为字符串，is_error 标识是否出错。
    // 存入 toolResults map，前端实时 ToolCallBlock 据此展示 result / isError。
    unlistens.push(
      await listen<ChatToolResultPayload>("chat:tool-result", (e) => {
        const p = e.payload;
        if (p.conversation_id !== activeConvId.value) return;

        toolResults.value = {
          ...toolResults.value,
          [p.tool_use_id]: {
            content: p.content,
            isError: p.is_error,
          },
        };
      }),
    );

    // ----- P2-1: chat:thinking -----
    unlistens.push(
      await listen<ChatThinkingPayload>("chat:thinking", (e) => {
        const p = e.payload;
        if (p.conversation_id !== activeConvId.value) return;

        thinkingContent.value += p.content;
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
   * 加载某会话的历史消息（仅取最近 INITIAL_PAGE_SIZE 条，不全量）。
   *
   * 行为变化（P2 性能优化）：
   *   - limit 从 100 降到 INITIAL_PAGE_SIZE（默认 20），首屏快 5 倍
   *   - 重置分页状态 hasMoreOlder / loadingOlder
   *   - 拉取失败写入 error 字段，保留旧列表
   *
   * @param conversationId 目标会话 ID；传 null/"" 则清空 messages 并返回
   */
  async function loadMessages(conversationId: string | null): Promise<void> {
    if (!conversationId) {
      messages.value = [];
      hasMoreOlder.value = false;
      loadingOlder.value = false;
      error.value = null;
      return;
    }

    // 切换会话：清流式状态（避免旧会话的 chunk 持续污染新会话视图）
    if (isStreaming.value && activeConvId.value !== conversationId) {
      isStreaming.value = false;
      streamingContent.value = "";
      activeConvId.value = null;
    }

    loading.value = true;
    error.value = null;
    // 重置翻页状态：进入新会话时一律以「可向上翻」开始保守
    // 若返回不足 INITIAL_PAGE_SIZE 条，loadOlderMessages / hasMoreOlder 在下面更新
    hasMoreOlder.value = true;
    loadingOlder.value = false;
    try {
      const list = await bridge.messages.list(conversationId, {
        limit: INITIAL_PAGE_SIZE,
      });
      messages.value = list;
      // 当返回条数 < 请求条数时，认为这是该会话的全部历史，无更多可加载
      hasMoreOlder.value = list.length >= INITIAL_PAGE_SIZE;
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    } finally {
      loading.value = false;
    }
  }

  /**
   * 向上翻页：加载更早的历史消息（OLDER_PAGE_SIZE 条），prepend 到 messages 头部。
   *
   * 流程：
   *   1. 守门：非加载中、有更多历史、当前消息非空、当前会话存在
   *   2. 取 messages[0] 的 (created_at, rowid) 作为复合游标
   *      —— ROWID_SENTINEL (-1) 理论上不会出现在这里，因为 sentinel
   *      消息必在尾部（用户→助手）；messages[0] 必为 DB 加载的真实消息
   *   3. 调 bridge.messages.list(before: cursor, limit: OLDER_PAGE_SIZE)
   *   4. prepend 新消息到 messages 头部
   *   5. 更新 hasMoreOlder
   *
   * 滚动位置补偿由 MessageList 组件负责（监听 messages.length 变化 + 锚点）。
   *
   * @returns 本次新加载的消息数量（0 表示无可加载或被守门拦下）
   */
  async function loadOlderMessages(): Promise<number> {
    if (loadingOlder.value || !hasMoreOlder.value) return 0;
    if (messages.value.length === 0) return 0;

    const convStore = useConversationsStore();
    const convId = convStore.currentId;
    if (!convId) return 0;

    const oldest = messages.value[0];
    if (!oldest) return 0;
    // 防御：sentinel 不作为游标（理论上 sentinel 不会在头部，但显式校验避免隐患）
    if (oldest.rowid === ROWID_SENTINEL) return 0;

    loadingOlder.value = true;
    try {
      const older = await bridge.messages.list(convId, {
        limit: OLDER_PAGE_SIZE,
        before: [oldest.created_at, oldest.rowid],
      });
      if (older.length > 0) {
        messages.value = [...older, ...messages.value];
      }
      hasMoreOlder.value = older.length >= OLDER_PAGE_SIZE;
      return older.length;
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      return 0;
    } finally {
      loadingOlder.value = false;
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
   * P2-2：可选 `contentBlocks` 传入多模态块（文本+图片等）。
   * - 若提供 `contentBlocks`：以 `contentBlocks` 为准（Rust 侧优先使用）
   * - content 文本会写入 DB `content` 列 （纯文本预览），
   *   同时存为 `content_blocks` JSON 字符串以便后续 HistoryMessage 还原
   *
   * @param content     用户输入文本（会做 trim）
   * @param contentBlocks P2-2 多模态内容块数组（可选）
   */
  async function sendMessage(
    content: string,
    contentBlocks?: import("../types").ContentBlock[],
  ): Promise<void> {
    const trimmed = content.trim();
    // P2-2 多模态：允许只发图片（文本为空但有 contentBlocks）
    const hasBlocks = !!contentBlocks && contentBlocks.length > 0;
    if (!trimmed && !hasBlocks) return;

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
      // P2-2 序列化：本地乐观插入同步携带 blocks，否则旧消息仍为 []
      content_blocks: hasBlocks ? JSON.stringify(contentBlocks) : "[]",
      token_count: null,
      error: null,
      created_at: new Date().toISOString(),
      // 哨兵 rowid：本地乐观消息，不会作为向上翻页的游标
      rowid: ROWID_SENTINEL,
    };
    messages.value = [...messages.value, optimistic];

    try {
      await bridge.chat.sendMessage(
        convId,
        trimmed,
        appliedTemplate.value
          ? {
              template_id: appliedTemplate.value.templateId,
              values: appliedTemplate.value.values,
            }
          : undefined,
        toolsEnabled.value,
        hasBlocks ? contentBlocks : undefined,
      );
      // 成功：清空已应用模板，保持乐观插入由 chat:start 替换真实 ID
      appliedTemplate.value = null;
    } catch (err) {
      // 失败：回滚
      messages.value = messages.value.filter((m) => m.id !== tempId);
      error.value = err instanceof Error ? err.message : String(err);
      activeConvId.value = null;
      throw err;
    }
  }

  /**
   * 设置已应用的模板（由 ChatInput / WelcomeInput 调用）
   * @param payload 模板 + 变量值；传 null 清空
   */
  function setAppliedTemplate(payload: AppliedTemplate | null): void {
    appliedTemplate.value = payload;
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
    activeToolCalls.value = [];
    thinkingContent.value = "";
    toolResults.value = {};

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
    appliedTemplate,
    // P2-1
    activeToolCalls,
    thinkingContent,
    toolsEnabled,
    toolResults,
    // P2-3
    lastUsage,
    // P2 分页
    hasMoreOlder,
    loadingOlder,
    // getters
    currentMessages,
    lastMessage,
    isEmpty,
    // actions
    setupListeners,
    teardownListeners,
    loadMessages,
    loadOlderMessages,
    sendMessage,
    stopGeneration,
    clearError,
    setAppliedTemplate,
  };
});
