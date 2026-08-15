// 聊天状态管理（会话列表 + 当前会话 + 消息 + 流式生成状态机）
// 侧栏不再按 Agent 过滤，显示全部会话混合列表
//
// ## 职责边界
// 本 store 只管「状态 + 动作」：会话 CRUD、消息分页、流式生成状态机
// （sending/streamingText/thinking/bgStreams/timeout）+ freezeCurrentAssistant 等。
// Tauri 事件监听（chat:* → 状态变更）已抽到 composables/useChatEvents.ts，
// 由 App.vue 在挂载时注册、卸载时拆卸；本 store 暴露事件层所需的状态 Map
// （bgStreams/pendingAuthRequests/pendingProposals/lastErrors）与动作
// （resetSendTimeout/clearSendTimeout/freezeCurrentAssistant）供其调用。
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { parseDbTime } from "../utils/time";
import type { Conversation, Message } from "../types";
import type {
  AuthScope,
  PendingAuthEntry,
  ConfigProposalPayload,
  ConfigProposalResponse,
} from "../types";
import { bridge } from "../api/bridge";
import { useAgentStore } from "./agent";

/** 授权等待上限，与后端 tool_executor::wait_for_auth_response 的 TIMEOUT 对齐
 *  （120s 到点后端自动取消并发 tool-auth-request-cancel 清条目）*/
export const TOOL_AUTH_TIMEOUT_MS = 120_000;

export const useChatStore = defineStore("chat", () => {
  // ===== 会话列表（全部会话，不限 agent） =====
  const conversations = ref<Conversation[]>([]);
  const convLoading = ref(false);

  async function loadConversations() {
    convLoading.value = true;
    try {
      const fresh = await bridge.conversations.listAll();
      // 撤销窗口（5s）内的行不回灌：乐观删除后延迟才真删后端，期间任何后台
      // 刷新（如 delegation-started）都会把「已删」会话带回列表——用户所见
      // 即「删除按钮不生效」。撤销走 undoDeleteConversation 独立恢复。
      const hidden = new Set(pendingDelete.value.keys());
      conversations.value = hidden.size > 0
        ? fresh.filter((c) => !hidden.has(c.id))
        : fresh;
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
    if (oldId && sending.value) {
      bgStreams.value = new Map(bgStreams.value).set(oldId, {
        text: streamingText.value,
        thinking: streamingThinking.value,
      });
    }
    activeConvId.value = id;
    // 提案/授权请求已按 convId 隔离存储（pendingProposals / pendingAuthRequests），
    // 切换会话无需清空——切回原会话时 computed 自动恢复对应条目。
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
    // 立即清空旧消息：loadMessages 是 async，await 期间 messages 仍持有上一会话内容，
    // 会导致切换瞬间在新会话标题下闪烁旧消息。先清空再加载。
    messages.value = [];
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

  // ===== office/pdf 文件附件列表 =====
  // 与图片不同：文件在后端 send_message 入口 materialize 为 Text 块（doc::try_extract），
  // 不进 ContentBlock（LLM 读不了 OOXML 二进制）。前端只持有 base64 + 文件名 + 字节数（用于展示）。
  const pendingFiles = ref<{ name: string; data: string; size: number }[]>([]);

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
    result?: { content: string; isError: boolean; durationMs: number } | null;
  }
  const streamingToolCalls = ref<Map<string, ToolCallState>>(new Map());
  const streamingThinking = ref("");
  const thinkingStartTime = ref<number | null>(null);
  const thinkingDuration = ref<string | null>(null);
  /** 思考结束后保留内容，让用户仍可展开查看 */
  const lastThinkingContent = ref<string | null>(null);
  /** 按消息 ID 持久化思考耗时（切换会话不丢，刷新才丢） */
  const thinkingDurations = ref<Map<string, string>>(new Map());
  /** pendingAuthRequests: 按 convId 索引的待处理工具授权请求（#10 路由模型）。
   *  事件 handler 始终按 convId 存（同会话工具串行执行 → 每会话至多一条在等），
   *  值含 receivedAt 供 120s 倒计时渲染。渲染按用户注意力分流：
   *  - activeConvAuthRequest → 激活会话：输入框上方内联卡（AuthRequestCard）
   *  - backgroundAuthRequests → 其它会话：全局右下通知栈（AuthNoticeStack，
   *    带会话身份可跳转）——取代旧 ToolAuthDialog 的「全局单 modal」混淆源。*/
  const pendingAuthRequests = ref<Map<string, PendingAuthEntry>>(new Map());
  /** 激活会话的待处理授权（内联卡渲染；无全局兜底——后台会话走通知栈）*/
  const activeConvAuthRequest = computed<PendingAuthEntry | null>(() =>
    activeConvId.value ? pendingAuthRequests.value.get(activeConvId.value) ?? null : null,
  );
  /** 非激活会话的待处理授权（右下通知栈渲染），[convId, entry] 对*/
  const backgroundAuthRequests = computed<Array<[string, PendingAuthEntry]>>(() =>
    [...pendingAuthRequests.value.entries()].filter(([cid]) => cid !== activeConvId.value),
  );
  /** pendingProposals: 按 convId 索引的待处理配置提案。
   *  computed pendingProposal 暴露激活会话的条目（内联卡片随激活会话渲染）。*/
  const pendingProposals = ref<Map<string, ConfigProposalPayload>>(new Map());
  const pendingProposal = computed(() =>
    activeConvId.value ? pendingProposals.value.get(activeConvId.value) ?? null : null,
  );
  /** 每会话最近一次发送错误的可见提示（chat:error 携带的用户可读消息）。
   *  按 conversation_id 隔离——A 会话的错误横幅不会串到 B 会话顶部。
   *  （chat:error 仅在错误属于当前激活会话时才写入；切会话时 computed 自然取目标会话条目）*/
  const lastErrors = ref<Map<string, string>>(new Map());
  const lastError = computed(() =>
    activeConvId.value ? lastErrors.value.get(activeConvId.value) ?? null : null,
  );
  let sendTimeout: ReturnType<typeof setTimeout> | null = null;

  /** 后台会话的流式文本快照：切走「正在流式」的会话时把已累积文本存这里，
   *  切回时恢复。此前 streamingText 是全局单份 + chunk 处理器丢弃非激活会话的事件，
   *  导致「流式中途切走再切回」时已渲染内容丢失、从一半继续渲染。
   *  只追踪 text/thinking（工具调用/多轮结构后台不展示，最终态由后端 chat:done 落库）。*/
  const bgStreams = ref<Map<string, { text: string; thinking: string }>>(new Map());

  /** 重置 60s 静默超时（滑动窗口）：任何活动事件调用一次即重新计时。
   *  超时只重置 sending 状态（不清 streaming，交由后端 chat:done(abort) 走 freeze）。*/
  function resetSendTimeout() {
    if (sendTimeout) clearTimeout(sendTimeout);
    sendTimeout = setTimeout(() => {
      if (sending.value && pendingAuthRequests.value.size === 0) {
        console.warn("静默超时（60s 无活动），重置发送状态");
        sending.value = false;
      }
    }, 60000);
  }

  /** 清除静默超时定时器（完成 / 出错 / 停止 / 切会话时调用）。
   *  供 useChatEvents 与本 store 动作共用（sendTimeout 由本 store 独家持有）。*/
  function clearSendTimeout() {
    if (sendTimeout) { clearTimeout(sendTimeout); sendTimeout = null; }
  }

  async function sendMessage(content: string, contentBlocks?: import("../types").ContentBlock[]) {
    if (!activeConvId.value || sending.value) return;
    sending.value = true;
    // 清掉当前会话的错误横幅（per-conv 隔离：只清本会话，不影响其它会话）
    if (activeConvId.value) {
      const m = new Map(lastErrors.value);
      m.delete(activeConvId.value);
      lastErrors.value = m;
    }
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

    // office/pdf 附件：后端 materialize 为 attachment（UI 卡片，不进 LLM）+ text（提取正文，进 LLM）。
    // 前端无提取能力，乐观气泡只放 attachment 元信息卡片（name/kind/size）占位；
    // 后端为唯一真源——会剥离这些乐观块并从 files 重建，故不污染 query/标题（join_text 跳过 attachment）。
    let files: import("../types").AttachedFile[] | undefined;
    if (pendingFiles.value.length > 0) {
      files = pendingFiles.value.map((f) => ({ name: f.name, data: f.data }));
      for (const f of pendingFiles.value) {
        const dot = f.name.lastIndexOf(".");
        const kind = dot >= 0 ? f.name.slice(dot + 1).toLowerCase() : "";
        blocks.push({ type: "attachment", name: f.name, kind, size: f.size });
      }
      pendingFiles.value = [];
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

    // 侧栏卡片：把该会话标记为「刚交互」（更新时间 + 置顶到列表上方）
    if (activeConvId.value) touchConversation(activeConvId.value);

    // 前端超时保护（滑动窗口）：60s 无任何活动事件才触发；活动事件 handler 会 reset
    resetSendTimeout();

    try {
      await bridge.chat.sendMessage(activeConvId.value, content, blocks.length > 0 ? blocks : undefined, true, files);
    } catch (e) {
      console.error("发送消息失败:", e);
      sending.value = false;
      streamingText.value = "";
    }
  }

  async function stopGeneration() {
    if (!activeConvId.value) return;
    clearSendTimeout();
    // 乐观停止「生成中」状态（隐藏光标）。**不清空 streaming 内容**——交由后端
    // cancel → finalize_cancel emit 的 chat:done(abort) 走 freezeCurrentAssistant
    // 统一把已生成的部分冻结到末条 assistant。若这里先清空，chat:done 的 freeze
    // 会写入空内容，导致取消时输出到一半的气泡消失。
    sending.value = false;
    try {
      await bridge.chat.stopGeneration(activeConvId.value);
    } catch { /* 静默忽略 */ }
  }

  /** 发送授权响应（#11 带 scope 范围档）。按 request_id 定位——通知栈里的
   *  条目不属于激活会话，不能再走「当前条目」语义。先删后 invoke：乐观移除
   *  防重复点击双发响应。*/
  async function respondToAuth(requestId: string, allowed: boolean, scope: AuthScope = "once") {
    const m = new Map(pendingAuthRequests.value);
    for (const [cid, entry] of m) {
      if (entry.payload.request_id === requestId) { m.delete(cid); break; }
    }
    pendingAuthRequests.value = m;
    // invoke 直达后端（原 emit 通道因 Tauri v2 事件作用域不匹配而失效）
    await bridge.chat.respondAuth({ request_id: requestId, allowed, scope });
  }

  /** 发送配置提案响应回 Rust */
  async function respondToProposal(response: ConfigProposalResponse) {
    // invoke 直达后端（原 emit 通道因 Tauri v2 事件作用域不匹配而失效）
    await bridge.chat.respondProposal({
      request_id: response.request_id,
      decision: response.decision,
      reason: response.reason ?? null,
      changes: response.changes,
    });
    // 按激活会话 + request_id 双重校验后删除（避免删掉同 conv 已被新提案覆盖的旧条目）
    const aid = activeConvId.value;
    if (!aid) return;
    const cur = pendingProposals.value.get(aid);
    if (cur && cur.request_id === response.request_id) {
      const m = new Map(pendingProposals.value);
      m.delete(aid);
      pendingProposals.value = m;
    }
  }

  // ===== 删除 / 置顶会话（含撤销机制） =====
  const pendingDelete = ref<Map<string, { conv: Conversation; timer: ReturnType<typeof setTimeout> }>>(new Map());

  async function deleteConversation(id: string) {
    // 先清理该 id 的旧 pending（如果存在）
    const existing = pendingDelete.value.get(id);
    if (existing) { clearTimeout(existing.timer); pendingDelete.value.delete(id); }

    const conv = conversations.value.find((c) => c.id === id);
    const wasActive = activeConvId.value === id;

    // 乐观移除 UI
    conversations.value = conversations.value.filter((c) => c.id !== id);
    if (wasActive) {
      if (conversations.value.length > 0) {
        activeConvId.value = conversations.value[0].id;
        loadMessages(activeConvId.value);
      } else {
        activeConvId.value = null;
        messages.value = [];
      }
    }

    // 延迟 5 秒真正删除后端数据，期间可撤销
    const timer = setTimeout(async () => {
      try {
        await bridge.conversations.delete(id);
      } catch (e) {
        console.error("删除会话失败:", e);
      } finally {
        pendingDelete.value.delete(id);
      }
    }, 5000);

    if (conv) {
      pendingDelete.value.set(id, { conv, timer });
    }
  }

  /** 撤销删除：恢复会话到列表并取消后端删除 */
  function undoDeleteConversation(id: string) {
    const entry = pendingDelete.value.get(id);
    if (!entry) return false;
    clearTimeout(entry.timer);
    pendingDelete.value.delete(id);
    // 恢复会话到列表并按更新日期重排序
    conversations.value = [...conversations.value, entry.conv].sort(
      (a, b) => parseDbTime(b.updated_at).getTime() - parseDbTime(a.updated_at).getTime(),
    );
    // 如果之前是活跃会话，恢复
    if (!activeConvId.value) {
      activeConvId.value = entry.conv.id;
      loadMessages(entry.conv.id);
    }
    return true;
  }

  /** 当前是否有待撤销的删除 */
  const hasPendingDelete = computed(() => pendingDelete.value.size > 0);

  async function pinConversation(id: string, pinned: boolean) {
    try {
      await bridge.conversations.pin(id, pinned);
      // 更新本地状态并重新排序（pinned 优先，按 updated_at 降序）
      conversations.value = conversations.value
        .map((c) => (c.id === id ? { ...c, pinned } : c))
        .sort((a, b) => {
          if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
          // DB 时间串（UTC 无标记）与 touchConversation 写入的完整 ISO 混存，
          // 必须统一 parseDbTime——裸 new Date 对前者按本地解析，两种格式混比差 8h
          return parseDbTime(b.updated_at).getTime() - parseDbTime(a.updated_at).getTime();
        });
    } catch (e) {
      console.error("置顶操作失败:", e);
    }
  }

  /** 标记会话「刚交互」：把 updated_at 更新为当前时间并按时间重排（pinned 优先）。
   *  后端 create_message 会 bump DB 的 updated_at，但前端 conversations 数组不会自动刷新，
   *  故侧栏需主动 touch，否则卡片一直显示旧时间、也不置顶到列表上方。*/
  function touchConversation(id: string) {
    const now = new Date().toISOString();
    conversations.value = conversations.value
      .map((c) => (c.id === id ? { ...c, updated_at: now } : c))
      .sort((a, b) => {
        if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
        return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
      });
  }

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

  // ===== 新建会话 =====
  async function createConversation(agentId: string, projectId?: string | null) {
    const conv = await bridge.conversations.create(agentId, undefined, projectId);
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
    // 事件监听器由 useChatEvents 管理（App.vue 卸载时拆卸）；这里只清状态 + 超时
    clearSendTimeout();
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
    pendingProposals.value = new Map();
    pendingAuthRequests.value = new Map();
    draftText.value = "";
    openTrajectoryNext.value = false;
  }

  /** 清除当前选中会话（切项目空间时调用）：保留 conversations 列表与草稿，
   *  只重置激活会话相关状态，让右侧回到「欢迎/新建会话」态，不携带上个空间的会话。*/
  function clearActiveConversation() {
    clearSendTimeout();
    activeConvId.value = null;
    messages.value = [];
    sending.value = false;
    streamingText.value = "";
    streamingThinking.value = "";
    thinkingStartTime.value = null;
    streamingToolCalls.value = new Map();
    lastFinishReason.value = null;
    bgStreams.value = new Map();
    pendingProposals.value = new Map();
    pendingAuthRequests.value = new Map();
    openTrajectoryNext.value = false;
  }


  /** 当前正在生成的会话 ID 集合（激活的流式会话 + 后台流式会话 bgStreams）。
   *  侧栏据此给会话卡片显示「生成中」状态 + 动画——现在每个会话独立监控，可精确到单卡。*/
  const streamingConvIds = computed(() => {
    const ids = new Set<string>(bgStreams.value.keys());
    if (sending.value && activeConvId.value) ids.add(activeConvId.value);
    return ids;
  });

  // ===== MA-1：delegation 子会话的编程式入口 =====
  // 委派子会话（kind='delegation'）不进侧栏列表，唯一入口是父会话委派卡片 /
  // 项目页任务列表的点击。二者都需要「打开该会话并直接落到轨迹 tab」，
  // 而 ChatPage 的 tab 是组件内部状态——用一次性标志传递意图。
  const openTrajectoryNext = ref(false);

  /** 打开（可能在列表外的）会话并直接落到轨迹 tab。附带刷新会话列表：
   *  委派子会话是后台新建的，当前 conversations 缓存里还没有它——不刷新的话
   *  activeConversation 查不到、头部标题/agent 名会空。 */
  function openConversationAtTrajectory(id: string) {
    openTrajectoryNext.value = true;
    selectConversation(id);
    void loadConversations();
  }

  return {
    conversations, convLoading,
    activeConvId, activeConversation,
    messages, msgLoading, hasMore, loadingMore,
    sending, streamingText, draftText, pendingImages, pendingFiles, lastFinishReason, currentModel,
    streamingToolCalls, streamingThinking, thinkingStartTime, thinkingDuration, lastThinkingContent, thinkingDurations,
    // 事件层（useChatEvents）直接读写的内部 Map——暴露供其 mutate；对外读取走下方 computed
    bgStreams, pendingAuthRequests, pendingProposals, lastErrors,
    activeConvAuthRequest, backgroundAuthRequests, pendingProposal, lastError,
    streamingConvIds,
    loadConversations, selectConversation, loadMoreMessages,
    sendMessage, stopGeneration, respondToAuth, respondToProposal,
    deleteConversation, undoDeleteConversation, hasPendingDelete, pinConversation,
    // 事件层调用的状态动作（freezeCurrentAssistant 把流式态冻结进末条 assistant）
    resetSendTimeout, clearSendTimeout, freezeCurrentAssistant,
    createConversation, clearActiveConversation, reset,
    openTrajectoryNext, openConversationAtTrajectory,
  };
});
