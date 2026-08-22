<!--
  ChatMessages — 聊天消息列表（含分页加载、thinking/tool_call 展开、Markdown 渲染）

  行为：
  - 向下滚动到底部时触发分页加载 (loadMoreMessages)
  - thinking 块默认折叠，可展开查看推理过程
  - tool_call 卡片显示工具名+参数，点击展开查看结果
  - 流式消息自动跟随滚动（用户手动上滚后暂停跟随）

  Props: 无（直接从 chat store 读取）
  Emits: 无
-->
<script setup lang="ts">
import { watch, nextTick, ref, computed, onActivated } from "vue";
import { useChatStore } from "../../stores/chat";
import { formatTime, formatDateLabel } from "../../utils/time";
import MarkdownRenderer from "./MarkdownRenderer.vue";
import ConfigProposalCard from "./ConfigProposalCard.vue";
import DelegationCard from "./DelegationCard.vue";
import PlanCard from "./PlanCard.vue";
import ImagePreview from "./ImagePreview.vue";
import AttachmentDetail from "./AttachmentDetail.vue";
import TurnRail from "./TurnRail.vue";
import BudgetPill from "./BudgetPill.vue";
import ErrorBanner from "../common/ErrorBanner.vue";
import { useThinkingTimer } from "../../composables/useThinkingTimer";
import { useScrollFollow } from "../../composables/useScrollFollow";
import { useTurnRail } from "../../composables/useTurnRail";
import { useActiveTurn, THRESHOLD_PX } from "../../composables/useActiveTurn";
import { formatTokenCount } from "../../utils/format";
import { shortCode, parseReferenceBlocks, resolveGroupMid } from "../../utils/refs";
import type { ParsedRef } from "../../utils/refs";
import { memoized } from "../../utils/blockMemo";
import type { Message, MessageRole, PlanItem } from "../../types";

const chat = useChatStore();
const listRef = ref<HTMLElement | null>(null);

// 滚动跟随 + 分页 + 阅读位置记忆（逻辑抽到 composable：自动贴底 / 上滚暂停 /
// 顶部触发分页 / 每会话滚动锚点——切走再回来按意图贴底或原位恢复）
const { showScrollBtn, autoFollow, paginating, scrollToBottom, restoreForConversation } = useScrollFollow(listRef);

// =========================================================================
// 轮次导航条（UX #5 v2）：锚点（后端轻量行，已排除占位行）+
// 视位侦测 + 跨页跳转；定容窗口（当前轮居中）状态在 TurnRail 组件内。
// 定位是「目录」不是 minimap——未加载页的内容高度不可知，按轮次索引对
// 任意规模（几千轮）都成立。
// =========================================================================
const { anchors, loadAnchors } = useTurnRail();

// ---- 视位侦测（useActiveTurn）：底线语义——输入框上方一根判定线，线在
// 哪轮的区域里就是哪轮（P11 二轮）；IntersectionObserver 只对实际相交
// （已渲染、坐标真实）的锚点判定，治旧 topsCache 静态 offsetTop 在
// content-visibility:auto 估高布局下系统性漂移（导航条卡旧轮/错位）的根因 ----
const { activeTurn, turnOfMsg, refresh: refreshActiveTurn, pin: pinTurn, clearPin } = useActiveTurn(listRef, anchors);

// 锚点刷新：会话切换 + 尾消息变化（= 新一轮开始；向前翻页不动尾，不误触）
watch(() => chat.activeConvId, (cid) => {
  clearPin(); // 旧会话的跳转钉子不可跨会话存活（composable 内锚点重载兜底，双保险）
  activeTurn.value = null;
  void loadAnchors(cid);
}, { immediate: true });
watch(() => chat.messages[chat.messages.length - 1]?.id, () => {
  if (chat.activeConvId) void loadAnchors(chat.activeConvId);
});

// 翻页/新轮/锚点重载后重绑观察（与旧 rebuildTops 同触发面；滚动监听由
// composable 自挂在 listRef 上）
watch([() => chat.messages.length, anchors], () => { void nextTick(refreshActiveTurn); });

// ---- 跳转：窗口内直接滚；窗口外逐页补到锚点（无进展即止，无页数上限——
// 大会话深跳不静默失败）。落点 = 锚点顶停在视口顶 THRESHOLD_PX 上方的阅读位
// （服务眼睛）；轮号即时性由钉子保证（视位判定是底线语义——P11 二轮：跳转
// 落点后线落在 N+k 区域，纯线判定会显得跳错，钉住 N 直到用户亲手滚动）。
// 滚动结束后复核漂移并瞬时校正（content-visibility 估高 → 渲染后真实高度） ----
/** 落点余量（px）：锚点顶边停在阅读位基准线上方这一距离 */
const JUMP_MARGIN_PX = 24;
/** 漂移校正容差（px）：scrollend 后锚点实际位与目标位差超此值才补滚 */
const JUMP_DRIFT_PX = 32;
/** 连点/切会话时作废旧校正（旧闭包的 el 可能已不在 DOM） */
let jumpToken = 0;

async function jumpToTurn(messageId: string) {
  const root = listRef.value;
  if (!root) return;
  let el = root.querySelector<HTMLElement>(`[data-mid="${messageId}"]`);
  while (!el && chat.hasMore) {
    const before = chat.messages.length;
    await chat.loadMoreMessages();
    if (chat.messages.length === before) break; // 无进展防死循环
    await nextTick();
    el = root.querySelector<HTMLElement>(`[data-mid="${messageId}"]`);
  }
  if (!el) return;
  const turn = turnOfMsg.value.get(messageId);
  if (turn !== undefined) pinTurn(turn); // 钉号先行：号码即时反馈，不依赖滚动完成
  autoFollow.value = false; // 跳历史位 = 非跟随态（与 restoreForConversation 同语义）
  const desired = THRESHOLD_PX - JUMP_MARGIN_PX;
  root.scrollTo({ top: Math.max(0, el.offsetTop - desired), behavior: "smooth" });
  scheduleJumpCorrection(root, el, desired);
}

/** 跳到最新 = 显式滚动意图：解钉（钉住会压住底线判定）+ 贴底跟随 */
function jumpLatest() {
  clearPin();
  scrollToBottom();
}

/** smooth 滚动结束后复核：估高布局被真实渲染替换会使锚点实际位置漂移，
 *  超容差则瞬时补滚一次（scrollend 优先，环境缺失时 500ms 定时兜底）。 */
function scheduleJumpCorrection(root: HTMLElement, el: HTMLElement, desired: number) {
  const token = ++jumpToken;
  const check = () => {
    if (token !== jumpToken) return; // 已有更新的跳转/校正，本次作废
    const top = el.getBoundingClientRect().top - root.getBoundingClientRect().top;
    const drift = top - desired;
    if (Math.abs(drift) > JUMP_DRIFT_PX) {
      root.scrollTo({ top: Math.max(0, root.scrollTop + drift), behavior: "auto" });
    }
  };
  if ("onscrollend" in root) {
    root.addEventListener("scrollend", check, { once: true });
  } else {
    setTimeout(check, 500);
  }
}

// 工具调用卡片展开状态
const expandedToolCalls = ref<Set<string>>(new Set());
// 思考过程展开状态（按消息 ID）
const expandedThinking = ref<Set<string>>(new Set());
// 思考耗时实时计时（逻辑抽到 composable：streamingThinking 期间每 200ms tick + KeepAlive 协同）
const { thinkingElapsed } = useThinkingTimer();

// 图片预览 / 文档详情 弹窗状态（同时只开一个，Teleport 到 body）
const previewImages = ref<{ data: string; mediaType: string }[] | null>(null);
const previewIndex = ref(0);
const detailAttachments = ref<{ name: string; kind: string; size: number }[] | null>(null);
const detailIndex = ref(0);
const detailTexts = ref<Record<string, string>>({});

const toolCallList = computed(() => {
  return Array.from(chat.streamingToolCalls.values());
});

function toggleToolCall(id: string) {
  const set = new Set(expandedToolCalls.value);
  if (set.has(id)) set.delete(id); else set.add(id);
  expandedToolCalls.value = set;
}

function toggleThinking(msgId: string) {
  const set = new Set(expandedThinking.value);
  if (set.has(msgId)) set.delete(msgId); else set.add(msgId);
  expandedThinking.value = set;
}

import { formatJson, truncateJson } from "../../utils/format";

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/** 判断一个 assistant 消息是否有非 text 的附属内容（tool/thinking）。
 *  memo 化：模板热路径每渲染每消息调用（见 utils/blockMemo.ts）。 */
const hasExtras = memoized((contentBlocks: string): boolean => {
  if (contentBlocks === '[]') return false;
  try {
    const blocks = JSON.parse(contentBlocks);
    return Array.isArray(blocks) && blocks.some((b: Record<string, unknown>) => b.type === 'tool_use' || b.type === 'thinking');
  } catch { return false; }
});

function msgHasExtras(msg: { content_blocks?: string }): boolean {
  return hasExtras(msg.content_blocks ?? '[]');
}

onActivated(() => {
  // KeepAlive 返回（设置页↔聊天）：DOM 与滚动位置原样保留，**不强制贴底**——
  // 旧实现无条件 scrollToBottom 是「回来后阅读位置丢失」的根源之一。仅跟随态
  // 补一次精确贴底（后台流式期间 messages.length watcher 已持续贴底）。
  // 滚动监听挂载/卸载由 useScrollFollow 自管；thinking 计时器由 useThinkingTimer 自管。
  nextTick(() => { if (autoFollow.value) scrollToBottom(false); });
});

// 切换会话等消息加载完成后，按该会话的滚动锚点恢复：离开时在底部 → 贴底；
// 在读历史 → 锚点消息原视口位（锚点在分页窗口外先翻页加载再定位）。
// 覆盖侧栏/任务胶囊/委派卡/面包屑全部进入路径。
watch(() => chat.msgLoading, async (loading) => {
  if (!loading && chat.messages.length > 0 && chat.activeConvId) {
    await nextTick();
    await restoreForConversation(chat.activeConvId, {
      loadMore: () => chat.loadMoreMessages(),
      hasMore: () => chat.hasMore,
    });
  }
});

// 自动滚到底部（分页加载时不触发；用户向上看内容时不抢滚动条）。
// 增长源必须盯全：只盯 messages.length/streamingText 时，工具卡出现/参数增长、
// 思考块流式这些「内容在长高但两个源没动」的窗口会让视口漂离底部，下一个
// 事件再来时猛拉回底——表现即工具调用期间滚动条「一跳一跳」。缩高场景
// （折叠/塌陷）无需处理：贴底时浏览器自动钳制 scrollTop 保持贴底。
watch(
  [() => chat.messages.length, () => chat.streamingText, () => chat.streamingThinking, () => chat.streamingToolCalls],
  async () => {
    if (paginating.value || !autoFollow.value) return;
    await nextTick();
    const el = listRef.value;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  },
);

const copiedId = ref<string | null>(null);

/** 复制：现代 API → execCommand 降级 → 如实失败态（宁可红 ✕ 不假绿 ✓）。*/
async function copyContent(content: string, id?: string) {
  let ok = false;
  try {
    await navigator.clipboard.writeText(content);
    ok = true;
  } catch {
    try {
      const ta = document.createElement("textarea");
      ta.value = content;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      ok = document.execCommand("copy");
      document.body.removeChild(ta);
    } catch {
      // 两级降级都失败：保持 ok=false 初值
    }
  }
  if (!id) return;
  copiedId.value = ok ? id : "fail:" + id;
  setTimeout(() => { copiedId.value = null; }, 2000);
}

/** 错误横幅「重试」：以同内容重发上一条失败发送（store.lastFailedSend 为据）。*/
function retryLastSend() {
  const last = chat.lastFailedSend;
  if (!last || chat.sending) return;
  void chat.sendMessage(last.content, last.blocks.length > 0 ? last.blocks : undefined);
}

// ===== @ 引用：气泡引用按钮（一键成 chip）+ 历史引用卡片（点击跳转） =====
const quotedId = ref<string | null>(null);

/** 引用按钮：把该消息（assistant=整组）推入输入框引用 chip，与 @ 弹层同产物。
 *  重复点击不重复加 chip（store 去重），成功态照常显示——按钮语义已满足。*/
function quoteMessage(msgId: string, role: string) {
  const kindLabel = role === "assistant" ? "回答" : "消息";
  chat.addPendingRef({
    refKind: "message",
    targetId: msgId,
    display: `${kindLabel}#${shortCode(msgId)}`,
  });
  quotedId.value = msgId;
  setTimeout(() => { if (quotedId.value === msgId) quotedId.value = null; }, 2000);
}

/** 历史引用卡片点击跳转：会话 → 打开该会话；消息 → 同会话滚动定位；
 *  agent → 无动作（title 已示身份）。目标已删（会话不在列表）则静默 no-op。*/
function openReference(r: ParsedRef) {
  if (r.refKind === "conversation") {
    if (chat.conversations.some((c) => c.id === r.targetId)) chat.selectConversation(r.targetId);
    return;
  }
  if (r.refKind === "message") {
    void jumpToTurn(resolveGroupMid(chat.messages, r.targetId));
  }
}

/** 解析 image 块。memo 化：模板热路径每渲染每消息调用。 */
const parseImageBlocks = memoized((contentBlocks: string): { data: string; mediaType: string }[] => {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { data: string; media_type: string; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'image'
    ).map((b) => ({ data: b.data, mediaType: b.media_type }));
  } catch { return []; }
});

/** 解析 attachment 块（附件元信息卡片：文件名 / 类型 / 字节数）。memo 化同上。 */
const parseAttachmentBlocks = memoized((contentBlocks: string): { name: string; kind: string; size: number }[] => {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { name: string; kind: string; size: number; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'attachment'
    ).map((b) => ({ name: b.name, kind: b.kind, size: b.size }));
  } catch { return []; }
});

/** 字节数 → 人类可读（如 "1.2 MB"） */
function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** 扩展名 → 展示标签（卡片上的类型名） */
const KIND_LABELS: Record<string, string> = {
  docx: 'Word', xlsx: 'Excel', xls: 'Excel', pdf: 'PDF',
};
function kindLabel(kind: string): string {
  return KIND_LABELS[kind.toLowerCase()] ?? kind.toUpperCase();
}

/** 用户消息是否含图片或附件块（纯附件无文本时仍需显示气泡） */
function hasUserMedia(msg: { content_blocks?: string }): boolean {
  if (!msg.content_blocks || msg.content_blocks === '[]') return false;
  return parseImageBlocks(msg.content_blocks).length > 0
    || parseAttachmentBlocks(msg.content_blocks).length > 0;
}

/** 剥离旧版遗留的 `[附件 xxx]` 文本标记（新版用 Attachment 块，不再生成；此为清理旧历史） */
function cleanUserContent(text: string | null | undefined): string {
  if (!text) return '';
  return text.replace(/\[附件[^\]]*\]/g, '').trim();
}

/**
 * 从 content_blocks 的 Text 块里提取后端 materialize 注入的附件原文。
 * 后端格式：<uploaded_file name="xxx" type="yyy">\n[系统提示…]\n{正文}\n</uploaded_file>
 * 返回 { 文件名 → 正文（已剥离系统提示行）}，供附件详情弹窗展示。
 */
function parseExtractedTexts(contentBlocks: string): Record<string, string> {
  const out: Record<string, string> = {};
  if (!contentBlocks || contentBlocks === '[]') return out;
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return out;
    const re = /<uploaded_file\s+name="([^"]+)"[^>]*>([\s\S]*?)<\/uploaded_file>/g;
    for (const b of blocks) {
      if (typeof b !== 'object' || b === null) continue;
      const bl = b as Record<string, unknown>;
      if (bl.type !== 'text' || typeof bl.text !== 'string') continue;
      let m: RegExpExecArray | null;
      re.lastIndex = 0;
      while ((m = re.exec(bl.text)) !== null) {
        const body = m[2].replace(/^\[系统提示[：:][^\]]*\]\s*/m, '').trim();
        out[m[1]] = body;
      }
    }
  } catch { /* ignore */ }
  return out;
}

/** 多图堆叠：第 i 张的 transform（第 0 张最上层完整，其余向右下错位+微旋转露边） */
function imgStackStyle(i: number): Record<string, string> {
  const offset = i * 8;
  const rot = i === 0 ? 0 : (i % 2 === 1 ? 5 : -5);
  return { transform: `translate(${offset}px, ${offset / 2}px) rotate(${rot}deg)`, zIndex: String(10 - i) };
}

/**
 * 多文档堆叠：用负 margin-top 让后一张向上叠（容器高度自动），
 * 配合 zIndex 让第 0 张完整置顶、其余向下露一条边（≈14px）。
 */
function docStackStyle(i: number): Record<string, string> {
  return { marginTop: i === 0 ? '0px' : '-30px', zIndex: String(10 - i) };
}

function openImagePreview(images: { data: string; mediaType: string }[], idx: number) {
  previewImages.value = images;
  previewIndex.value = idx;
}
function openAttachmentDetail(attachments: { name: string; kind: string; size: number }[], idx: number, contentBlocks: string) {
  detailAttachments.value = attachments;
  detailIndex.value = idx;
  detailTexts.value = parseExtractedTexts(contentBlocks);
}

/** 解析 tool_use 块。memo 化：模板热路径每渲染每消息调用。 */
const parseToolUseBlocks = memoized((contentBlocks: string): { id: string; name: string; input: string }[] => {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { id: string; name: string; input: string; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'tool_use'
    ).map((b) => ({ id: b.id, name: b.name, input: b.input }));
  } catch { return []; }
});

/** 解析 tool_result 块。memo 化同上。 */
const parseToolResultBlocks = memoized((contentBlocks: string): { toolUseId: string; content: string; isError: boolean }[] => {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { tool_use_id: string; content: string; is_error?: boolean; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'tool_result'
    ).map((b) => ({ toolUseId: b.tool_use_id, content: b.content, isError: b.is_error ?? false }));
  } catch { return []; }
});

/** 解析 thinking 块。memo 化同上。 */
const parseThinkingBlocks = memoized((contentBlocks: string): string[] => {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { thinking: string; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'thinking'
    ).map((b) => b.thinking);
  } catch { return []; }
});

/** tool_use_id → tool_result 全局索引（替代逐 tool_use 向后扫 + 逐次 JSON.parse
 *  的 O(n²)：messages 每次（末条整对象替换式）更新时 O(n) 重建，parse 走 memo，
 *  重建即查表。tool_use_id 全局唯一，同 id 复用仅见于畸形数据——取最后出现。 */
const toolResultIndex = computed(() => {
  const idx = new Map<string, { content: string; isError: boolean }>();
  for (const m of chat.messages) {
    if (m.role !== "user") continue;
    for (const r of parseToolResultBlocks(m.content_blocks)) {
      idx.set(r.toolUseId, { content: r.content, isError: r.isError });
    }
  }
  return idx;
});

/** 查 tool_use_id 对应的 tool_result（查全局索引，见 toolResultIndex）。*/
function findToolResult(
  toolUseId: string,
): { content: string; isError: boolean } | null {
  return toolResultIndex.value.get(toolUseId) ?? null;
}

/** 查询某个 tool_use_id 对应的 tool_result 是否有 isError（跨消息配对）*/
function getToolHasError(toolUseId: string): boolean {
  return findToolResult(toolUseId)?.isError ?? false;
}

// ===== MA-1：delegate_to_agent 委派卡片（DelegationCard 的取数层）=====
/** 解析委派工具参数（agent_id/task）。流式期间参数逐字到达，解析失败返回 null。 */
function parseDelegateInput(input: string): { agentId: string; task: string } | null {
  try {
    const o = JSON.parse(input) as { agent_id?: unknown; task?: unknown };
    if (o && typeof o.agent_id === "string") {
      return { agentId: o.agent_id, task: typeof o.task === "string" ? o.task : "" };
    }
  } catch { /* 参数未接收完/非 JSON */ }
  return null;
}

/** 解析委派 tool_result（child_conversation_id/agent_name/finish_reason/rounds）。
 *  Err 路径的 content 是错误文案非 JSON → null（状态走 isError）。 */
function parseDelegateResult(content: string): {
  childConvId: string | null; agentName: string | null;
  finishReason: string | null; rounds: number | null;
} | null {
  try {
    const o = JSON.parse(content) as Record<string, unknown>;
    if (o && (typeof o.child_conversation_id === "string" || typeof o.agent_name === "string")) {
      return {
        childConvId: typeof o.child_conversation_id === "string" ? o.child_conversation_id : null,
        agentName: typeof o.agent_name === "string" ? o.agent_name : null,
        finishReason: typeof o.finish_reason === "string" ? o.finish_reason : null,
        rounds: typeof o.rounds === "number" ? o.rounds : null,
      };
    }
  } catch { /* 非委派 JSON */ }
  return null;
}

/** 本会话当前运行中的委派子会话 id（运行中卡片跳转用）。
 *  child_conversation_id 在完成时的 tool_result 才回传，但后端在子会话创建成功
 *  即 emit chat:delegation-started（store 刷新列表）→ 运行中也能跳。
 *  v1 串行执行（execute_tool_round 顺序跑工具）保证同父同时至多一个运行中委派。 */
const runningDelegationChildId = computed(() => {
  const pid = chat.activeConvId;
  if (!pid) return null;
  const running = chat.conversations.find(
    (c) => c.kind === "delegation" && c.parent_conversation_id === pid && chat.streamingConvIds.has(c.id),
  );
  return running?.id ?? null;
});

/** 历史 tool_use 的委派卡片取数：跨消息配对 tool_result，无结果=进行中。 */
function delegateCardFor(
  tu: { id: string; name: string; input: string },
): {
  agentName: string; agentId: string | null; task: string; status: "running" | "done" | "error";
  childConvId: string | null; finishReason: string | null; rounds: number | null; hasError: boolean;
} | null {
  if (tu.name !== "delegate_to_agent") return null;
  const input = parseDelegateInput(tu.input);
  const tr = findToolResult(tu.id);
  const dr = tr && !tr.isError ? parseDelegateResult(tr.content) : null;
  const status = !tr ? "running" : tr.isError ? "error" : "done";
  return {
    agentName: dr?.agentName ?? input?.agentId ?? "…",
    agentId: input?.agentId ?? null,
    task: input?.task ?? "",
    status,
    childConvId: dr?.childConvId ?? (status === "running" ? runningDelegationChildId.value : null),
    finishReason: dr?.finishReason ?? null,
    rounds: dr?.rounds ?? null,
    hasError: tr?.isError ?? false,
  };
}

/** 流式中的委派卡片取数：call.arguments 逐字到达，call.result 到达即完成。 */
function delegateStreamCard(call: {
  name: string; arguments: string; ended: boolean;
  result?: { content: string; isError: boolean } | null;
}): {
  agentName: string; agentId: string | null; task: string; status: "running" | "done" | "error";
  childConvId: string | null; finishReason: string | null; rounds: number | null;
} | null {
  if (call.name !== "delegate_to_agent") return null;
  const input = parseDelegateInput(call.arguments || "{}");
  const dr = call.result && !call.result.isError ? parseDelegateResult(call.result.content) : null;
  const status = !call.result ? "running" : call.result.isError ? "error" : "done";
  return {
    agentName: dr?.agentName ?? input?.agentId ?? "…",
    agentId: input?.agentId ?? null,
    task: input?.task ?? (call.ended ? "" : "正在接收参数…"),
    status,
    childConvId: dr?.childConvId ?? (status === "running" ? runningDelegationChildId.value : null),
    finishReason: dr?.finishReason ?? null,
    rounds: dr?.rounds ?? null,
  };
}

/** 委派卡片「打开任务」：打开子会话并直落轨迹 tab（store 附带刷新会话列表）。 */
function openChildConv(childId: string) {
  chat.openConversationAtTrajectory(childId);
}

// ===== C5：update_plan 计划卡片（PlanCard 的取数层）=====
/** 解析计划工具参数（steps → PlanItem[]）。steps 缺失/非数组 → null（流式参数
 *  未到齐）；steps: [] 合法（agent 主动清空计划）。字段宽容降级，不为展示卡死。 */
function parsePlanInput(input: string): PlanItem[] | null {
  try {
    const o = JSON.parse(input) as { steps?: unknown };
    if (o && Array.isArray(o.steps)) {
      return o.steps
        .filter((s): s is Record<string, unknown> => typeof s === "object" && s !== null)
        .map((s) => ({
          text: typeof s.text === "string" ? s.text : "",
          status: typeof s.status === "string" ? s.status : "pending",
          task_conversation_id:
            typeof s.task_conversation_id === "string" ? s.task_conversation_id : null,
        }));
    }
  } catch { /* 参数未接收完/非 JSON */ }
  return null;
}

/** 历史 tool_use 的计划卡片取数：Err（校验失败）→ null 落回通用行承载错误。 */
function planCardFor(
  tu: { id: string; name: string; input: string },
): PlanItem[] | null {
  if (tu.name !== "update_plan") return null;
  const items = parsePlanInput(tu.input);
  if (!items) return null;
  return findToolResult(tu.id)?.isError ? null : items;
}

/** 流式中的计划卡片取数：arguments 逐字到达（解析成功即出卡）；Err → 通用行。 */
function planStreamCard(call: {
  name: string; arguments: string;
  result?: { content: string; isError: boolean } | null;
}): PlanItem[] | null {
  if (call.name !== "update_plan") return null;
  if (call.result?.isError) return null;
  return parsePlanInput(call.arguments || "{}");
}

/** blocks 是否全部为 tool_result（memo 化：messageGroups 每次重算对每消息调用）。 */
const isAllToolResultBlocks = memoized((contentBlocks: string): boolean => {
  try {
    const blocks = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks) || blocks.length === 0) return false;
    return blocks.every((b: Record<string, unknown>) => b.type === "tool_result");
  } catch {
    return false;
  }
});

/** 判断 user 消息是否仅含 tool_result（无文本/图片）。
 *  这种消息是工具调用结果，不单独成气泡，其内容并入上一条 assistant 的工具卡片。*/
function isToolResultOnlyUser(msg: { role: string; content: string; content_blocks: string }): boolean {
  if (msg.role !== "user" || msg.content) return false;
  return isAllToolResultBlocks(msg.content_blocks);
}

// ===== 消息分组（连续同 agent 的 assistant 合并成一个气泡块）=====
interface GroupedItem { msg: Message; idx: number }
interface MessageGroup {
  key: string;
  role: MessageRole;
  model: string | null;
  items: GroupedItem[];
  firstIdx: number;
  lastIdx: number;
}

/** 把 chat.messages 按「连续 assistant + 同 model」分组。tool_result-only user 被跳过
 *  且不切断 assistant 连续性（其内容并入上一条 assistant 的工具卡片）。数据层 messages 不变。*/
const messageGroups = computed<MessageGroup[]>(() => {
  const out: MessageGroup[] = [];
  for (let i = 0; i < chat.messages.length; i++) {
    const msg = chat.messages[i];
    if (isToolResultOnlyUser(msg)) continue;
    const prev = out[out.length - 1];
    const mergeable =
      msg.role === "assistant" &&
      prev !== undefined &&
      prev.role === "assistant" &&
      prev.model === (msg.model ?? null);
    if (mergeable) {
      prev.items.push({ msg, idx: i });
      prev.lastIdx = i;
    } else {
      out.push({
        key: "grp-" + msg.id,
        role: msg.role,
        model: msg.model ?? null,
        items: [{ msg, idx: i }],
        firstIdx: i,
        lastIdx: i,
      });
    }
  }
  return out;
});

/** 该 item 是否是当前正在流式的 assistant（活跃生成目标）。
 *  依据：sending 期间 messages 末条恒为流式 assistant 占位。*/
function isLiveAssistant(item: GroupedItem): boolean {
  return chat.sending && item.msg.role === "assistant" && item.idx === chat.messages.length - 1;
}

/** 该 item 是否属于当前回合（turnFirstIdx 锚点之后）——保持 MarkdownRenderer 的
 *  streaming 视图（代码块不折叠）。多轮工具回合每轮冻结时若立即折叠，代码块
 *  「展开→瞬间回到 420px 上限」会让列表高度骤缩、贴底滚动条跳一下；回合结束
 *  （chat:done 清锚点）再统一沉淀，折叠只发生一次。*/
function isTurnStreaming(item: GroupedItem): boolean {
  return chat.sending && chat.turnFirstIdx !== null && item.idx >= chat.turnFirstIdx;
}

/** 该 item 是否是全局最后一条 assistant（用于 chat:done 后驻留的「思考·已完成」块）。*/
function isLastAssistant(item: GroupedItem): boolean {
  return item.msg.role === "assistant" && item.idx === chat.messages.length - 1;
}

/** 组内所有非空文本（多轮 assistant 的 content 以空行连接，供组级复制）。*/
function groupText(g: MessageGroup): string {
  return g.items.map((it) => it.msg.content).filter(Boolean).join("\n\n");
}

/** assistant 组 footer 是否可见：组内有文本或附属内容（工具/思考）才显示；
 *  纯流式空占位（只有三个点动画、无内容）不显示，避免时间戳/model 悬在空气泡下。*/
function assistantGroupFooterVisible(g: MessageGroup): boolean {
  return g.items.some((it) => it.msg.content || msgHasExtras(it.msg));
}

/** assistant 组 token 求和（前向兼容：当前仅末轮有 token_count）。*/
function groupTokenSum(g: MessageGroup): number {
  return g.items.reduce((s, it) => s + (it.msg.token_count ?? 0), 0);
}

// ===== 时间分组（绝对时间格式化统一走 utils/time） =====
function isNewDay(idx: number): boolean {
  if (idx === 0) return true;
  const prev = chat.messages[idx - 1].created_at;
  const curr = chat.messages[idx].created_at;
  return formatDateLabel(prev) !== formatDateLabel(curr);
}

// ===== finish_reason 展示 =====
// B3：可续跑类终止（预算/轮数/停滞/长度）不再用「截断」错误框架——中性提示 +
// 一键继续按钮（后端 B1 自动续期额度用尽 / agent 显式硬上限 / stuck 真停时的兜底）。
const finishReasonLabels: Record<string, string> = {
  length: "已达长度上限，回答被截断",
  // Anthropic 系（Claude / MiniMax）的 stop_reason 原样透传为 "max_tokens"，
  // 与 OpenAI 系的 "length" 同义，统一映射成同一句中文。
  max_tokens: "已达长度上限，回答被截断",
  abort: "已手动停止",
  budget_exceeded: "本次 token 预算已达上限",
  stuck: "连续多轮无进展，已自动终止",
  tool_use: "已达工具调用轮数上限",
};

/** finish_reason 文案：budget_exceeded 在有预算数据时附具体数字
 *  （已用 X / 上限 Y），让用户知道为什么这么快烧完、还剩多少跑道。 */
function finishReasonLabel(reason: string): string {
  if (reason === "budget_exceeded" && chat.budget) {
    const b = chat.budget;
    return `本次 token 预算已达上限（已用 ${formatTokenCount(b.cumulative_tokens)} / 上限 ${formatTokenCount(b.effective_cap)}）`;
  }
  return finishReasonLabels[reason] || reason;
}
// 「发送消息即可续跑」的终止类：提示行内附「继续」按钮（abort=用户主动停，不列）
const RESUMABLE_REASONS = new Set(["budget_exceeded", "tool_use", "stuck", "length", "max_tokens"]);
</script>

<template>
  <!-- wrap：相对定位包裹层——承载「跳到最新」右侧轨道按钮（独立于滚动容器，
       绝对定位子元素若挂在滚动容器内会随内容滚动，位置漂移） -->
  <div class="messages-wrap">
    <div ref="listRef" class="messages-area">
    <!-- 错误提示（UI-2 批次三：ErrorBanner 化——有失败发送可重发，否则仅关闭） -->
    <ErrorBanner
      v-if="chat.lastError"
      variant="banner"
      class="chat-error-banner"
      :title="chat.lastFailedSend ? '发送失败' : '请求出错'"
      :detail="chat.lastError"
      :retry-label="chat.lastFailedSend ? '重试' : null"
      dismissible
      @retry="retryLastSend"
      @dismiss="chat.clearConvError()"
    />
    <!-- 分页加载指示器 -->
    <div v-if="chat.loadingMore" class="load-more-hint">加载更早消息…</div>
    <div v-if="!chat.hasMore && chat.messages.length > 50" class="load-more-hint load-more-end">已显示全部消息</div>

    <div v-if="chat.msgLoading && chat.messages.length === 0" class="msg-skeleton">
      <div v-for="n in 5" :key="n" class="msg-skeleton-block">
        <div class="msg-skeleton-line msg-skel-title" />
        <div class="msg-skeleton-line msg-skel-body" />
        <div class="msg-skeleton-line msg-skel-body msg-skel-short" />
      </div>
    </div>
    <div v-else-if="!chat.activeConvId" class="state-hint">选择一个对话开始</div>
    <div v-else-if="chat.messages.length === 0" class="state-hint">开始一段新的对话</div>
    <TransitionGroup v-else name="msg" tag="div" class="messages-container">
      <template v-for="group in messageGroups" :key="group.key">
        <!-- 日期分组标签（基于组首）-->
        <div v-if="isNewDay(group.firstIdx)" class="date-divider">{{ formatDateLabel(chat.messages[group.firstIdx].created_at) }}</div>
        <!-- data-mid=组首消息 id：useScrollFollow 锚点捕获/恢复的 DOM 定位符 -->
        <div :class="['message-group', group.role]" :data-mid="group.items[0].msg.id">
          <!-- ===== 用户消息组（单条，透明壳）===== -->
          <template v-if="group.role === 'user'">
            <div class="message-content user">
              <div v-if="cleanUserContent(group.items[0].msg.content) || hasUserMedia(group.items[0].msg) || parseReferenceBlocks(group.items[0].msg.content_blocks).length > 0" class="message-bubble">
                <span v-if="cleanUserContent(group.items[0].msg.content)" class="user-text">{{ cleanUserContent(group.items[0].msg.content) }}</span>

                <!-- @ 引用卡片（快照存在消息里；点击跳转：会话切换 / 消息定位） -->
                <div
                  v-for="r in parseReferenceBlocks(group.items[0].msg.content_blocks)"
                  :key="'ref-' + r.targetId"
                  class="user-ref-card"
                  :data-ref-kind="r.refKind"
                  :title="r.refKind === 'agent' ? `Agent：${r.display}` : '跳转到引用对象'"
                  @click="openReference(r)"
                >
                  <svg class="ref-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" /><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" /></svg>
                  <span class="ref-label">{{ r.display }}</span>
                </div>

                <!-- 文档附件：单个直显 / ≥2 重叠堆叠；点击看提取原文 -->
                <template v-if="parseAttachmentBlocks(group.items[0].msg.content_blocks).length === 1">
                  <div
                    v-for="(att, i) in parseAttachmentBlocks(group.items[0].msg.content_blocks)"
                    :key="'att-' + i"
                    class="user-attachment-card clickable"
                    :title="`查看 ${att.name} 提取内容`"
                    @click="openAttachmentDetail(parseAttachmentBlocks(group.items[0].msg.content_blocks), i, group.items[0].msg.content_blocks)"
                  >
                    <span class="att-icon" :data-kind="att.kind">{{ kindLabel(att.kind)[0] }}</span>
                    <span class="att-info">
                      <span class="att-name">{{ att.name }}</span>
                      <span class="att-meta">{{ kindLabel(att.kind) }} · {{ formatFileSize(att.size) }}</span>
                    </span>
                  </div>
                </template>
                <div
                  v-else-if="parseAttachmentBlocks(group.items[0].msg.content_blocks).length > 1"
                  class="doc-stack"
                  :title="`共 ${parseAttachmentBlocks(group.items[0].msg.content_blocks).length} 个附件，点击查看`"
                  @click="openAttachmentDetail(parseAttachmentBlocks(group.items[0].msg.content_blocks), 0, group.items[0].msg.content_blocks)"
                >
                  <div
                    v-for="(att, i) in parseAttachmentBlocks(group.items[0].msg.content_blocks).slice(0, 3)"
                    :key="'att-' + i"
                    class="user-attachment-card"
                    :style="docStackStyle(i)"
                  >
                    <span class="att-icon" :data-kind="att.kind">{{ kindLabel(att.kind)[0] }}</span>
                    <span class="att-info">
                      <span class="att-name">{{ att.name }}</span>
                      <span class="att-meta">{{ kindLabel(att.kind) }} · {{ formatFileSize(att.size) }}</span>
                    </span>
                  </div>
                  <span v-if="parseAttachmentBlocks(group.items[0].msg.content_blocks).length > 3" class="stack-badge">+{{ parseAttachmentBlocks(group.items[0].msg.content_blocks).length - 3 }}</span>
                </div>

                <!-- 图片：单图直显 / ≥2 重叠堆叠；点击全屏预览 -->
                <template v-if="parseImageBlocks(group.items[0].msg.content_blocks).length === 1">
                  <img
                    v-for="(img, i) in parseImageBlocks(group.items[0].msg.content_blocks)"
                    :key="'img-' + i"
                    :src="`data:${img.mediaType};base64,${img.data}`"
                    class="user-image clickable"
                    loading="lazy"
                    @click="openImagePreview(parseImageBlocks(group.items[0].msg.content_blocks), i)"
                  />
                </template>
                <div
                  v-else-if="parseImageBlocks(group.items[0].msg.content_blocks).length > 1"
                  class="image-stack"
                  :title="`共 ${parseImageBlocks(group.items[0].msg.content_blocks).length} 张图片，点击预览`"
                  @click="openImagePreview(parseImageBlocks(group.items[0].msg.content_blocks), 0)"
                >
                  <img
                    v-for="(img, i) in parseImageBlocks(group.items[0].msg.content_blocks).slice(0, 3)"
                    :key="'img-' + i"
                    :src="`data:${img.mediaType};base64,${img.data}`"
                    class="user-image stacked"
                    :style="imgStackStyle(i)"
                    loading="lazy"
                  />
                  <span v-if="parseImageBlocks(group.items[0].msg.content_blocks).length > 3" class="stack-badge">+{{ parseImageBlocks(group.items[0].msg.content_blocks).length - 3 }}</span>
                </div>
              </div>
              <div v-if="cleanUserContent(group.items[0].msg.content) || hasUserMedia(group.items[0].msg) || parseReferenceBlocks(group.items[0].msg.content_blocks).length > 0" class="message-footer">
                <div class="footer-left">
                  <span class="message-time">{{ formatTime(group.items[0].msg.created_at) }}</span>
                </div>
                <div class="footer-actions">
                  <button class="copy-btn" :title="copiedId === group.items[0].msg.id ? '已复制' : copiedId === 'fail:' + group.items[0].msg.id ? '复制失败：剪贴板不可用' : '复制'" @click="copyContent(group.items[0].msg.content, group.items[0].msg.id)">
                    <svg v-if="copiedId !== group.items[0].msg.id" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <rect x="9" y="9" width="13" height="13" rx="2" ry="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                    </svg>
                    <svg v-else-if="copiedId === 'fail:' + group.items[0].msg.id" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--ip-danger-base)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
                    <svg v-else width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--ip-success-base)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
                  </button>
                  <button class="copy-btn" :title="quotedId === group.items[0].msg.id ? '已加入输入框引用' : '引用'" @click="quoteMessage(group.items[0].msg.id, 'user')">
                    <svg v-if="quotedId !== group.items[0].msg.id" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" /><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" /></svg>
                    <svg v-else width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--ip-success-base)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
                  </button>
                </div>
              </div>
            </div>
          </template>

          <!-- ===== 助手消息组（气泡块：连续多轮合并）===== -->
          <template v-else-if="group.role === 'assistant'">
            <div v-for="item in group.items" :key="item.msg.id" class="message-item">
              <!-- 三个点动画：仅当前流式 item 且无任何返回时显示 -->
              <div v-if="isLiveAssistant(item) && item.msg.content === '' && !chat.streamingThinking && toolCallList.length === 0" class="think-dots">
                <span class="think-dot" /><span class="think-dot" /><span class="think-dot" />
              </div>

              <template v-if="item.msg.content || !chat.sending || chat.streamingThinking || toolCallList.length > 0">
                <!-- 思考过程（历史消息）；末条且 done 块显示时跳过避免重复 -->
                <template v-for="(think, ti) in parseThinkingBlocks(item.msg.content_blocks)" :key="'think-' + item.msg.id + '-' + ti">
                  <div v-if="!(isLastAssistant(item) && chat.thinkingDuration && chat.lastThinkingContent)" class="think-block">
                    <div class="think-toggle" @click="toggleThinking(item.msg.id + '-h' + ti)">
                      <span class="think-chevron">{{ expandedThinking.has(item.msg.id + '-h' + ti) ? '▾' : '▸' }}</span>
                      <span class="think-label">{{ chat.thinkingDurations.has(item.msg.id) ? '思考 · ' + chat.thinkingDurations.get(item.msg.id) : '思考' }}</span>
                    </div>
                    <Transition name="think-fade">
                      <div v-if="expandedThinking.has(item.msg.id + '-h' + ti)" class="think-body">
                        <MarkdownRenderer :content="think" />
                      </div>
                    </Transition>
                  </div>
                </template>

                <!-- 思考过程（流式 / 刚结束，带切换动画） -->
                <Transition name="think-swap" mode="out-in">
                  <div v-if="isLiveAssistant(item) && chat.streamingThinking" key="live" class="think-block">
                    <div class="think-toggle" @click="toggleThinking('streaming')">
                      <span class="think-chevron">{{ expandedThinking.has('streaming') ? '▾' : '▸' }}</span>
                      <span class="think-label">思考</span>
                      <span class="think-status">进行中… {{ thinkingElapsed }}</span>
                    </div>
                    <Transition name="think-fade">
                      <div v-if="expandedThinking.has('streaming')" class="think-body">
                        <MarkdownRenderer :content="chat.streamingThinking" streaming />
                      </div>
                    </Transition>
                  </div>
                  <div v-else-if="isLastAssistant(item) && chat.thinkingDuration && chat.lastThinkingContent" key="done" class="think-block">
                    <div class="think-toggle" @click="toggleThinking('done')">
                      <span class="think-chevron">{{ expandedThinking.has('done') ? '▾' : '▸' }}</span>
                      <span class="think-label">思考 · {{ chat.thinkingDuration }}</span>
                    </div>
                    <Transition name="think-fade">
                      <div v-if="expandedThinking.has('done')" class="think-body">
                        <MarkdownRenderer :content="chat.lastThinkingContent" />
                      </div>
                    </Transition>
                  </div>
                </Transition>

                <!-- 文字（按时间线顺序：thinking → 文本 → 工具，匹配 content_blocks）-->
                <div v-if="item.msg.content" class="message-bubble">
                  <MarkdownRenderer :content="item.msg.content" :streaming="isLiveAssistant(item) || isTurnStreaming(item)" />
                </div>

                <!-- 工具调用（历史/刚结束，从 content_blocks 解析，非流式） -->
                <div v-if="parseToolUseBlocks(item.msg.content_blocks).length > 0 && !(isLiveAssistant(item) && toolCallList.length > 0)" class="tools-strip">
                  <div v-for="tu in parseToolUseBlocks(item.msg.content_blocks)" :key="tu.id">
                    <!-- MA-1：delegate_to_agent 渲染为委派卡片（失败时补通用行承载原始错误） -->
                    <template v-for="d in [delegateCardFor(tu)]" :key="d ? 'dlg-card' : 'dlg-none'">
                      <template v-if="d">
                        <DelegationCard
                          :agent-name="d.agentName"
                          :agent-id="d.agentId"
                          :task="d.task"
                          :status="d.status"
                          :child-conv-id="d.childConvId"
                          :finish-reason="d.finishReason"
                          :rounds="d.rounds"
                          @open-child="openChildConv"
                        />
                        <template v-if="d.hasError">
                          <div class="tool-toggle" @click="toggleToolCall(tu.id)">
                            <span class="tool-chevron">{{ expandedToolCalls.has(tu.id) ? '▾' : '▸' }}</span>
                            <span class="tool-name">{{ tu.name }}</span>
                            <span class="tool-preview">调用失败</span>
                            <span class="tool-dot tool-dot-err"></span>
                          </div>
                          <Transition name="tool-slide">
                            <div v-if="expandedToolCalls.has(tu.id)" class="tool-expand">
                              <div class="tool-expand-group">
                                <div class="tool-expand-hdr">参数</div>
                                <pre class="tool-expand-code">{{ formatJson(tu.input) }}</pre>
                              </div>
                              <template v-for="tr in [findToolResult(tu.id)]" :key="tr ? 'has-result' : 'no-result'">
                                <div v-if="tr" class="tool-expand-group">
                                  <div class="tool-expand-hdr hdr-err">错误</div>
                                  <pre class="tool-expand-code code-err">{{ tr.content }}</pre>
                                </div>
                              </template>
                            </div>
                          </Transition>
                        </template>
                      </template>
                      <template v-else>
                        <!-- C5：update_plan 渲染为计划卡片；校验失败（Err）落回通用行承载原始错误 -->
                        <template v-for="p in [planCardFor(tu)]" :key="p ? 'plan-card' : 'plan-none'">
                          <PlanCard v-if="p" :items="p" @open-task="openChildConv" />
                          <template v-else>
                            <div class="tool-toggle" @click="toggleToolCall(tu.id)">
                              <span class="tool-chevron">{{ expandedToolCalls.has(tu.id) ? '▾' : '▸' }}</span>
                              <span class="tool-name">{{ tu.name }}</span>
                              <span class="tool-preview">{{ truncateJson(tu.input) }}</span>
                              <span :class="['tool-dot', getToolHasError(tu.id) ? 'tool-dot-err' : 'tool-dot-ok']"></span>
                            </div>
                            <Transition name="tool-slide">
                              <div v-if="expandedToolCalls.has(tu.id)" class="tool-expand">
                                <div class="tool-expand-group">
                                  <div class="tool-expand-hdr">参数</div>
                                  <pre class="tool-expand-code">{{ formatJson(tu.input) }}</pre>
                                </div>
                                <template v-for="tr in [findToolResult(tu.id)]" :key="tr ? 'has-result' : 'no-result'">
                                  <div v-if="tr" class="tool-expand-group">
                                    <div :class="['tool-expand-hdr', tr.isError ? 'hdr-err' : '']">{{ tr.isError ? '错误' : '结果' }}</div>
                                    <pre :class="['tool-expand-code', tr.isError ? 'code-err' : '']">{{ tr.content }}</pre>
                                  </div>
                                </template>
                              </div>
                            </Transition>
                          </template>
                        </template>
                      </template>
                    </template>
                  </div>
                </div>

                <!-- 工具调用（当前流式） -->
                <div v-if="isLiveAssistant(item) && toolCallList.length > 0" class="tools-strip">
                  <div v-for="call in toolCallList" :key="call.id">
                    <!-- MA-1：流式中的委派卡片（参数逐字到达/结果即完成） -->
                    <template v-for="d in [delegateStreamCard(call)]" :key="d ? 'dlg-live' : 'dlg-none'">
                      <DelegationCard
                        v-if="d"
                        :agent-name="d.agentName"
                        :agent-id="d.agentId"
                        :task="d.task"
                        :status="d.status"
                        :child-conv-id="d.childConvId"
                        :finish-reason="d.finishReason"
                        :rounds="d.rounds"
                        @open-child="openChildConv"
                      />
                      <template v-else>
                        <!-- C5：流式中的计划卡片（参数逐字到达，解析成功即出卡）；Err 落回通用行 -->
                        <template v-for="p in [planStreamCard(call)]" :key="p ? 'plan-live' : 'plan-none'">
                          <PlanCard v-if="p" :items="p" @open-task="openChildConv" />
                          <template v-else>
                            <div class="tool-toggle" @click="toggleToolCall(call.id)">
                              <span class="tool-chevron">{{ expandedToolCalls.has(call.id) ? '▾' : '▸' }}</span>
                              <span class="tool-name">{{ call.name }}</span>
                              <span v-if="call.result?.durationMs" class="tool-duration">{{ formatDuration(call.result.durationMs) }}</span>
                              <span class="tool-preview">{{ truncateJson(call.arguments || '') }}</span>
                              <span v-if="call.ended && call.result" :class="['tool-dot', call.result.isError ? 'tool-dot-err' : 'tool-dot-ok']"></span>
                              <span v-else-if="call.ended" class="tool-dot tool-dot-wait"></span>
                              <span v-else class="tool-dot tool-dot-busy"></span>
                            </div>
                            <Transition name="tool-slide">
                              <div v-if="expandedToolCalls.has(call.id)" class="tool-expand">
                                <div class="tool-expand-group">
                                  <div class="tool-expand-hdr">参数</div>
                                  <pre class="tool-expand-code">{{ formatJson(call.arguments) }}</pre>
                                </div>
                                <div v-if="call.result" class="tool-expand-group">
                                  <div :class="['tool-expand-hdr', call.result.isError ? 'hdr-err' : '']">{{ call.result.isError ? '错误' : '结果' }}</div>
                                  <pre :class="['tool-expand-code', call.result.isError ? 'code-err' : '']">{{ call.result.content }}</pre>
                                </div>
                                <div v-else class="tool-expand-group">
                                  <div class="tool-expand-hdr">结果</div>
                                  <div class="tool-expand-pending">{{ call.ended ? '等待执行结果…' : '正在接收参数…' }}</div>
                                </div>
                              </div>
                            </Transition>
                          </template>
                        </template>
                      </template>
                    </template>
                  </div>
                </div>
              </template>
            </div>

            <!-- 组级 footer：时间(组首) / model(一次) / token(求和) / 复制(组内文本) -->
            <div v-if="assistantGroupFooterVisible(group)" class="message-footer">
              <div class="footer-left">
                <span class="message-time">{{ formatTime(chat.messages[group.firstIdx].created_at) }}</span>
                <span v-if="group.model" class="badge-model">{{ group.model }}</span>
                <span v-if="groupTokenSum(group) > 0" class="badge-tokens">{{ groupTokenSum(group) }} tokens</span>
              </div>
              <div class="footer-actions">
                <button v-if="groupText(group)" class="copy-btn" :title="copiedId === 'grp-' + group.firstIdx ? '已复制' : copiedId === 'fail:' + 'grp-' + group.firstIdx ? '复制失败：剪贴板不可用' : '复制'" @click="copyContent(groupText(group), 'grp-' + group.firstIdx)">
                  <svg v-if="copiedId !== 'grp-' + group.firstIdx" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="9" y="9" width="13" height="13" rx="2" ry="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                  </svg>
                  <svg v-else-if="copiedId === 'fail:' + 'grp-' + group.firstIdx" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--ip-danger-base)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
                    <svg v-else width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--ip-success-base)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
                </button>
                <!-- 引用整组（组首 id；后端按连续 assistant 组展开=一次完整回答） -->
                <button class="copy-btn" :title="quotedId === group.items[0].msg.id ? '已加入输入框引用' : '引用'" @click="quoteMessage(group.items[0].msg.id, 'assistant')">
                  <svg v-if="quotedId !== group.items[0].msg.id" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" /><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" /></svg>
                  <svg v-else width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--ip-success-base)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
                </button>
              </div>
            </div>
          </template>
        </div>
      </template>
    </TransitionGroup>

    <!-- 配置提案审批卡片（内联） -->
    <div v-if="chat.pendingProposal" class="proposal-wrapper">
      <ConfigProposalCard :proposal="chat.pendingProposal" />
    </div>

    <!-- finish_reason 提示（B3：可续跑类附「继续」按钮，一键发「继续」续跑任务） -->
    <div v-if="chat.lastFinishReason && chat.lastFinishReason !== 'stop' && chat.lastFinishReason !== 'end_turn' && chat.messages.length > 0" class="finish-reason">
      <span>{{ finishReasonLabel(chat.lastFinishReason) }}</span>
      <BudgetPill v-if="chat.budget" :budget="chat.budget" />
      <button
        v-if="RESUMABLE_REASONS.has(chat.lastFinishReason) && !chat.sending"
        class="continue-btn"
        title="任务状态完好，发送「继续」即可接着跑"
        @click="chat.sendMessage('继续')"
      >继续</button>
    </div>

    <div v-if="chat.sending && chat.messages.length > 0" class="cursor-bar">
      <div class="cursor-track">
        <div class="cursor-glow" /><span class="cursor-label">正在生成…</span>
      </div>
      <BudgetPill v-if="chat.budget" :budget="chat.budget" />
    </div>

    <!-- 全屏图片预览（多图可翻页） -->
    <ImagePreview
      v-if="previewImages"
      :images="previewImages"
      :start-index="previewIndex"
      @close="previewImages = null"
    />
    <!-- 文档附件详情（手风琴 + 提取原文） -->
    <AttachmentDetail
      v-if="detailAttachments"
      :attachments="detailAttachments"
      :start-index="detailIndex"
      :extracted-texts="detailTexts"
      @close="detailAttachments = null"
    />
    </div>

    <!-- 预算续期 toast（非阻塞，5s 自动消失；仿 ChatHeader undo-toast 定位模式） -->
    <Transition name="budget-toast">
      <div v-if="chat.renewalNotice" class="budget-renewal-toast">
        <span class="budget-renewal-text">{{ chat.renewalNotice }}</span>
      </div>
    </Transition>

    <!-- 轮次导航条（UX #5 v2）：定容滑动窗口（当前轮居中）+ 视位高亮 +
         位置徽标 + 边缘省略号/滚轮调窗 + 底部「跳到最新」；
         ≥2 轮才出现，短会话由下方兜底按钮接住跳最新 -->
    <TurnRail
      :anchors="anchors"
      :active-turn="activeTurn"
      :show-latest="showScrollBtn"
      @jump="jumpToTurn"
      @latest="jumpLatest()"
    />

    <!-- 「跳到最新」兜底：无导航条（<2 轮）时保留原右侧轨道按钮 -->
    <Transition name="fade-up">
      <button v-if="showScrollBtn && anchors.length < 2" class="scroll-bottom-btn" title="回到底部并跟随最新" @click="jumpLatest()">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="12" y1="5" x2="12" y2="19" /><polyline points="19 12 12 19 5 12" />
        </svg>
      </button>
    </Transition>
  </div>
</template>

<style scoped>
/* 包裹层：flex 主轴占满 + 相对定位（轨道按钮的锚）。
   --msg-col-right：内容列右侧内边距（= 基础 48px + 「跳到最新」轨道预留 32px），
   气泡/日期线/提示行右侧统一用它对齐；调轨道宽度只改这一个值。 */
.messages-wrap {
  flex:1; min-height:0; display:flex; position:relative;
  /* --msg-col-right 已上提 ChatPage .chat-view（tabbar 任务胶囊对齐同一令牌） */
  /* 右侧带水平居中（手测反馈：right:10px 贴窗口边）：气泡列右缘与窗口右缘
     （有滚动条时=滚动条）之间的预留带内，TurnRail 两侧均匀留白。
     6px = global.css ::-webkit-scrollbar 宽；轨道视觉宽 22px（tick 道）。
     右距 = 6 + (80 - 22)/2 = 35px，带中心 = 46px 恒定。 */
  --msg-rail-right: calc(6px + (var(--msg-col-right) - 22px) / 2);
}
/* scrollbar-gutter:stable 恒定预留滚动条位——带内悬浮件（导航条/兜底按钮）
   不随滚动条出现/消失漂移，内容列也不再横向抖 6px */
.messages-area { flex:1; overflow-y:auto; scrollbar-gutter:stable; padding:24px 0; position:relative; }
.messages-container { display:flex; flex-direction:column; gap: var(--ip-spacing-4); padding:0 var(--msg-col-right) 0 48px; }

/* ===== 分页指示 ===== */
.load-more-hint { text-align:center; font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); padding:8px var(--msg-col-right) 8px 48px; }
.load-more-end { color:var(--ip-color-text-disabled); }

/* ===== 日期分组 ===== */
.date-divider { display:flex; align-items:center; gap: var(--ip-spacing-3); padding:20px var(--msg-col-right) 8px 48px; font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); }
.date-divider::before, .date-divider::after { content:''; flex:1; height:1px; background:var(--ip-color-border-default); }

/* ===== finish_reason 提示（B3：中性提示 + 可续跑类「继续」按钮）===== */
.finish-reason { display:flex; align-items:center; justify-content:center; gap: var(--ip-spacing-2); padding:4px var(--msg-col-right) 0 48px; }
.finish-reason span { display:inline-block; font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); padding:2px 10px; border-radius:var(--ip-radius-full); background:var(--ip-color-bg-tertiary); }
.continue-btn { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-secondary); padding:2px 12px; border-radius:var(--ip-radius-full); border:1px solid var(--ip-color-border-default); background:var(--ip-color-bg-secondary); cursor:pointer; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.continue-btn:hover { color:var(--ip-color-text-primary); border-color:var(--ip-color-border-strong); }

/* ===== 预算续期 toast（非阻塞；messages-wrap 相对定位承载）===== */
.budget-renewal-toast {
  position:absolute; top:12px; left:50%; transform:translateX(-50%);
  z-index:var(--ip-z-notification, 1500);
  display:flex; align-items:center;
  padding:6px 16px; border-radius:var(--ip-radius-md);
  background:var(--ip-color-bg-elevated, var(--ip-color-bg-secondary));
  border:1px solid var(--ip-color-primary-soft-bg);
  box-shadow:var(--ip-shadow-md);
  pointer-events:none;
}
.budget-renewal-text { font-size:var(--ip-text-caption-size); color:var(--ip-color-primary-tint-text); white-space:nowrap; }
.budget-toast-enter-active { animation:budget-toast-in 0.25s var(--ip-ease-out); }
.budget-toast-leave-active { animation:budget-toast-in 0.2s ease-in reverse; }
@keyframes budget-toast-in {
  from { opacity:0; transform:translate(-50%, -8px); }
  to   { opacity:1; transform:translate(-50%, 0); }
}

/* ===== TransitionGroup 动画 ===== */
.msg-enter-active { animation:msg-in 0.35s cubic-bezier(0.16,1,0.3,1); }
.msg-leave-active { display:none; }
.msg-move { transition:transform 0.3s ease; }
@keyframes msg-in { from { opacity:0; transform:translateY(12px) scale(0.97); } to { opacity:1; transform:translateY(0) scale(1); } }

/* ===== 消息组（连续同 agent 的 assistant 合并成一个气泡块）===== */
.message-group { display:flex; flex-direction:column; gap:2px; min-width:0; }
/* H1 渲染虚拟化（千轮会话防卡）：屏外消息组跳过 layout/paint——content-visibility
   是浏览器原生机制，DOM 与组件状态全保留（工具/思考展开态、图片、TransitionGroup
   动画），滚动跟随/分页逻辑零改动；contain-intrinsic-size 的 auto 前缀让浏览器
   记忆组实测高度（无记忆时按 300px 估算），滚动条稳定。WebView2（Chromium 85+）支持。
   取舍：治「渲染成本」（滚动卡顿主因）；DOM 常驻的内存未治——分页 50 条/页
   翻页累积是用户主动行为，实测仍有内存压力再考虑卸载式窗口化（有展开态丢失/
   高度跳动代价，此处不做）。 */
.message-group { content-visibility:auto; contain-intrinsic-size:auto 300px; }
.message-group.assistant {
  align-self:flex-start; max-width:85%;
  background-color:var(--ip-color-bg-message-ai); color:var(--ip-color-text-message-ai);
  border-radius:12px; border-bottom-left-radius:4px; padding:14px 16px;
}
.message-group.user { align-self:flex-end; max-width:70%; }
.message-content { display:flex; flex-direction:column; gap:4px; min-width:0; }
.message-group.user .message-content { align-items:flex-end; }

/* 组内多条 assistant item：item 内子项适度间距，轮次之间留白区分（呼吸感）*/
.message-group.assistant .message-item { display:flex; flex-direction:column; gap:6px; }
.message-group.assistant .message-item + .message-item { margin-top:16px; }

/* ===== 用户消息气泡 ===== */
/* flex column + 统一 gap：正文 / 引用卡 / 附件卡（含堆叠）/ 图片（含堆叠）纵向
   排布的唯一间距来源——旧实现靠各元素零散 margin-top（6/4/0px 不一，单图单卡
   贴正文），多元素混排时参差；卡片的独立 margin 已移除，调间距只改 gap。 */
.message-group.user .message-bubble { display:flex; flex-direction:column; gap: var(--ip-spacing-2); padding:10px 16px; border-radius:12px; font-size:var(--ip-text-body-size); line-height:1.6; white-space:pre-wrap; word-break:break-word; background-color:var(--ip-color-bg-user-bubble); color:var(--ip-color-text-on-user-bubble); border-bottom-right-radius:4px; }

/* ===== 助手消息文字（无自带背景，由组容器承载气泡块）===== */
.message-group.assistant .message-bubble { padding:0; border-radius:0; font-size:var(--ip-text-body-size); line-height:1.6; white-space:pre-wrap; word-break:break-word; background:transparent; }

/* ===== 用户消息内容（含图片） ===== */
.user-content { display:flex; flex-direction:column; gap:4px; }
.user-text { display:block; white-space:pre-wrap; }
.user-images { display:flex; flex-wrap:wrap; gap:4px; margin-top:2px; }
.user-image { max-width:200px; max-height:200px; border-radius:var(--ip-radius-lg); object-fit:cover; border:1px solid var(--ip-color-border-default); }

/* 用户附件卡片（office/pdf）—— 不透明白实体卡片：深绿气泡上的清晰层次，
   堆叠时不透明避免半透明叠加发灰/透字（半透明玻璃在重叠场景不可扩展） */
.user-attachments { display:flex; flex-direction:column; gap:4px; margin-top:6px; }
.user-attachment-card {
  display:flex; align-items:center; gap: var(--ip-spacing-2);
  padding:6px 10px; border-radius:8px;
  background:#ffffff;
  border:1px solid rgba(0,0,0,0.08);
  box-shadow:0 1px 2px rgba(0,0,0,0.06);
  max-width:260px;
  color:#1f2937;
}
.att-icon {
  flex:none; width:26px; height:26px; border-radius:6px;
  display:flex; align-items:center; justify-content:center;
  font-size: var(--ip-text-micro-size); font-weight:700; color:#fff; letter-spacing:-0.5px;
}
.att-icon[data-kind="pdf"] { background:rgba(220,38,38,0.9); }
.att-icon[data-kind="docx"] { background:rgba(37,99,235,0.9); }
.att-icon[data-kind="xlsx"], .att-icon[data-kind="xls"] { background:rgba(22,163,74,0.9); }
.att-info { display:flex; flex-direction:column; min-width:0; line-height:1.35; }
.att-name { font-size:13px; font-weight:500; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.att-meta { font-size: var(--ip-text-micro-size); color:#6b7280; }

/* 单个卡片/图片可点（hover 提示） */
.user-attachment-card.clickable { cursor:pointer; transition:background var(--ip-duration-fast) var(--ip-ease-out); }
.user-attachment-card.clickable:hover { background:#f3f4f6; }
.user-image.clickable { cursor:zoom-in; transition:transform var(--ip-duration-fast) var(--ip-ease-out); }
.user-image.clickable:hover { transform:scale(1.02); }

/* @ 引用卡片（用户气泡内）：轻量内联卡——图标按 ref_kind 着色，agent 无跳转 */
.user-ref-card {
  display:flex; align-items:center; gap:6px;
  max-width:260px; padding:5px 10px; border-radius:8px;
  background:#ffffff; border:1px solid rgba(0,0,0,0.08);
  box-shadow:0 1px 2px rgba(0,0,0,0.06);
  cursor:pointer; transition:background var(--ip-duration-fast) var(--ip-ease-out);
  color:#1f2937; font-size:12.5px;
}
.user-ref-card:hover { background:#f3f4f6; }
.user-ref-card .ref-icon { flex:none; color:var(--ip-primary-600, var(--ip-primary-600)); }
.user-ref-card[data-ref-kind="agent"] .ref-icon { color:var(--ip-accent-agent); }
.user-ref-card[data-ref-kind="message"] .ref-icon { color:#6b7280; }
.user-ref-card[data-ref-kind="agent"] { cursor:default; }
.user-ref-card .ref-label { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }

/* 多文档重叠堆叠：子卡片 position:relative 才能让 zIndex 生效；负 margin 由内联 style 给；
   堆叠态加重投影，让"一摞卡片"的层次可见 */
.doc-stack { position:relative; max-width:260px; cursor:pointer; }
.doc-stack .user-attachment-card { position:relative; box-shadow:0 3px 10px rgba(0,0,0,0.18); }
.doc-stack .user-attachment-card:hover { background:#fafafa; }

/* 多图重叠堆叠：固定方形容器，子图绝对定位错位 */
.image-stack {
  position:relative; width:170px; height:170px; cursor:zoom-in;
}
.image-stack .user-image.stacked {
  position:absolute; top:0; left:0; width:150px; height:150px;
  max-width:none; max-height:none;
  box-shadow:0 2px 8px rgba(0,0,0,0.25);
  transition:transform var(--ip-duration-fast) var(--ip-ease-out);
}
.image-stack:hover .user-image.stacked { /* 悬停时整体微展开，强化"可点"反馈 */ }

/* 堆叠溢出角标（图/文档通用） */
.stack-badge {
  position:absolute; right:-6px; bottom:-6px; z-index: var(--ip-z-raised);
  min-width:22px; height:22px; padding:0 6px;
  border-radius:999px;
  background:rgba(0,0,0,0.6); color:#fff;
  font-size: var(--ip-text-micro-size); font-weight:600; line-height:22px; text-align:center;
  border:1.5px solid rgba(255,255,255,0.85);
}

/* ===== 消息底部（时间 + 复制按钮） ===== */
.message-footer { display:flex; align-items:center; justify-content:space-between; gap: var(--ip-spacing-2); margin-top:2px; padding:0 4px; }
.footer-left { display:flex; align-items:center; gap:6px; }
.footer-actions { display:flex; align-items:center; gap:6px; opacity:0; transition:opacity var(--ip-duration-fast) var(--ip-ease-out); }
.message-group:hover .footer-actions { opacity:1; }
.message-group:hover .message-footer { opacity:1; }
.message-time { font-size: var(--ip-text-micro-size); color:var(--ip-color-text-disabled); }
.copy-btn { display:flex; align-items:center; justify-content:center; width:24px; height:24px; border-radius:var(--ip-radius-md); border:none; background:transparent; color:var(--ip-color-text-tertiary); cursor:pointer; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.copy-btn:hover { background-color:var(--ip-color-bg-tertiary); color:var(--ip-color-text-secondary); }

.badge-model { font-size: var(--ip-text-micro-size); color:var(--ip-color-text-tertiary); padding:1px 6px; border-radius:var(--ip-radius-sm); background:var(--ip-color-bg-tertiary); white-space:nowrap; }
.badge-tokens { font-size: var(--ip-text-micro-size); color:var(--ip-color-text-tertiary); white-space:nowrap; font-variant-numeric:tabular-nums; }

/* ===== 思考中动画 ===== */
.think-dots { display:flex; align-items:center; gap:4px; padding:4px 0; min-height:22px; }
.think-dot { width:6px; height:6px; border-radius:50%; background-color:var(--ip-color-text-secondary); animation:think-bounce 1.4s ease-in-out infinite; }
.think-dot:nth-child(2) { animation-delay:0.16s; }
.think-dot:nth-child(3) { animation-delay:0.32s; }
@keyframes think-bounce { 0%,80%,100% { transform:translateY(0); opacity:0.4; } 40% { transform:translateY(-6px); opacity:1; } }

/* ===== 流式光标 ===== */
.cursor-bar { display:flex; justify-content:flex-start; align-items:center; gap: var(--ip-spacing-2_5); padding:4px 48px 0; }
.cursor-track { display:flex; align-items:center; gap: var(--ip-spacing-2); padding:4px 0; }
.cursor-glow { width:8px; height:8px; border-radius:50%; background-color:var(--ip-primary-500); animation:cursor-pulse 1.2s ease-in-out infinite; }
.cursor-label { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); }
@keyframes cursor-pulse { 0%,100% { opacity:1; transform:scale(1); } 50% { opacity:0.4; transform:scale(0.75); } }

/* ===== 状态 ===== */
.state-hint { height:100%; display:flex; align-items:center; justify-content:center; gap: var(--ip-spacing-2); color:var(--ip-color-text-tertiary); font-size:var(--ip-text-body-sm-size); }
.state-dot { width:6px; height:6px; border-radius:50%; background-color:var(--ip-primary-500); animation:cursor-pulse 1.2s ease-in-out infinite; }

/* 骨架屏：消息列表加载中 */
.msg-skeleton { display:flex; flex-direction:column; gap:24px; padding: var(--ip-spacing-6); }
.msg-skeleton-block { display:flex; flex-direction:column; gap: var(--ip-spacing-2); }
.msg-skeleton-line {
  height:14px; border-radius:var(--ip-radius-sm);
  background:linear-gradient(90deg, var(--ip-color-bg-tertiary) 25%, var(--ip-color-bg-secondary) 50%, var(--ip-color-bg-tertiary) 75%);
  background-size:200% 100%;
  animation:skeleton-shimmer 1.5s infinite;
}
.msg-skel-title { width:30%; }
.msg-skel-body { width:80%; }
.msg-skel-short { width:55%; }

@keyframes skeleton-shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}

/* ===== 滚动到底按钮 ===== */
/* 「跳到最新」右侧轨道位：垂直居中（top 用 calc 而非 transform 居中——
   fade-up 进出场动画要占用 transform，二者会互相覆盖）。与 TurnRail 同带
   中心对齐（带中心 46px，按钮 36px 宽 → 右距 = 6 + (80-36)/2 = 28px）。 */
.scroll-bottom-btn { position:absolute; top:calc(50% - 18px); right:calc(6px + (var(--msg-col-right) - 36px) / 2); z-index:5; width:36px; height:36px; border-radius:var(--ip-radius-lg); border:1px solid var(--ip-color-border-default); background-color:var(--ip-color-bg-elevated); color:var(--ip-color-text-secondary); box-shadow:var(--ip-shadow-sm); cursor:pointer; display:flex; align-items:center; justify-content:center; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.scroll-bottom-btn:hover { background-color:var(--ip-color-bg-secondary); color:var(--ip-color-text-primary); border-color:var(--ip-color-border-strong); box-shadow:var(--ip-shadow-md); }

.fade-up-enter-active { animation:fade-up-in 0.2s ease-out; }
.fade-up-leave-active { animation:fade-up-in 0.15s ease-in reverse; }
@keyframes fade-up-in { from { opacity:0; transform:translateY(8px); } to { opacity:1; transform:translateY(0); } }

/* ===== 思考过程（无边框无背景，左绿线标识） ===== */
.think-block { margin:0; }
.think-toggle { display:flex; align-items:center; gap:6px; padding:2px 6px; cursor:pointer; user-select:none; border-radius:var(--ip-radius-sm); transition:all var(--ip-duration-fast) var(--ip-ease-out); width:100%; }
.think-toggle:hover { background:var(--ip-color-bg-tertiary); }
.think-chevron { font-size: var(--ip-text-micro-size); color:var(--ip-color-text-disabled); line-height:1; width:10px; flex-shrink:0; transition:transform var(--ip-duration-fast) var(--ip-ease-out); }
.think-label { font-size:var(--ip-text-caption-size); font-weight:var(--ip-font-weight-medium); color:var(--ip-color-text-tertiary); letter-spacing:0.3px; text-transform:uppercase; }
.think-status { margin-left:8px; font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); }
.think-body { margin:4px 0 4px 22px; padding:6px 0 6px 14px; border-left:2px solid var(--ip-primary-200); font-size:var(--ip-text-body-sm-size); color:var(--ip-color-text-secondary); line-height:1.7; white-space:pre-wrap; word-break:break-word; }
/* 思考内容的 Markdown 继承 13px 字号 */
.think-body .markdown-body { font-size:inherit; color:inherit; line-height:inherit; }

/* 思考展开/收起动画 */
.think-fade-enter-active { animation:think-in 0.2s ease-out; }
.think-fade-leave-active { animation:think-in 0.12s ease-in reverse; }
@keyframes think-in {
  from { opacity:0; transform:translateY(-3px); }
  to   { opacity:1; transform:translateY(0); }
}

/* 思考状态切换动画（流式→已完成） */
.think-swap-enter-active { animation:think-swap-in 0.25s ease-out; }
.think-swap-leave-active { animation:think-swap-in 0.15s ease-in reverse; }
@keyframes think-swap-in {
  from { opacity:0; transform:translateY(-4px); }
  to   { opacity:1; transform:translateY(0); }
}

/* ===== 工具调用（无边框，block 行布局，与思考视觉对齐） ===== */
.tools-strip { display:flex; flex-direction:column; gap:1px; margin:0; }
.tool-toggle { display:flex; align-items:center; gap:6px; padding:2px 6px; cursor:pointer; user-select:none; border-radius:var(--ip-radius-sm); transition:background var(--ip-duration-fast) var(--ip-ease-out); width:100%; }
.tool-toggle:hover { background:var(--ip-color-bg-tertiary); }
.tool-chevron { font-size: var(--ip-text-micro-size); color:var(--ip-color-text-disabled); line-height:1; width:10px; flex-shrink:0; }
.tool-name { font-size:var(--ip-text-caption-size); font-weight:var(--ip-font-weight-medium); color:var(--ip-color-text-tertiary); white-space:nowrap; }
.tool-duration { font-size: var(--ip-text-micro-size); color:var(--ip-color-text-disabled); font-family:var(--ip-font-mono, monospace); white-space:nowrap; flex-shrink:0; }
.tool-preview { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); margin-left:auto; margin-right:6px; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; text-align:right; flex-shrink:1; }

/* 状态圆点 */
.tool-dot { width:6px; height:6px; border-radius:50%; flex-shrink:0; }
.tool-dot-ok { background:var(--ip-success-base); }
.tool-dot-err { background:var(--ip-danger-base); }
.tool-dot-wait { background:var(--ip-warning-base); }
.tool-dot-busy { background:var(--ip-primary-500); animation:tool-dot-pulse 1.2s ease-in-out infinite; }
@keyframes tool-dot-pulse { 0%,100% { opacity:1; } 50% { opacity:0.35; } }

/* 展开详情（左绿线 + 缩进，与思考 body 统一） */
.tool-expand { margin:2px 0 2px 22px; padding:4px 0 6px 14px; border-left:2px solid var(--ip-primary-200); max-height:400px; overflow-y:auto; }
.tool-expand-group { margin-bottom:8px; }
.tool-expand-group:last-child { margin-bottom:0; }
.tool-expand-hdr { font-size: var(--ip-text-micro-size); font-weight:var(--ip-font-weight-semibold); color:var(--ip-color-text-tertiary); margin-bottom:4px; letter-spacing:0.5px; }
.tool-expand-hdr.hdr-err { color:var(--ip-danger-base); }
.tool-expand-code { font-size:var(--ip-text-caption-size); font-family:var(--ip-font-mono, monospace); white-space:pre-wrap; word-break:break-word; color:var(--ip-code-text); background:var(--ip-code-bg); padding:6px 8px; border-radius:var(--ip-radius-sm); margin:0; line-height:1.5; max-height:200px; overflow-y:auto; }
.tool-expand-code.code-err { color:var(--ip-danger-base); }
.tool-expand-pending { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); font-style:italic; }

/* 工具展开/收起动画 */
.tool-slide-enter-active { animation:tool-slide-in 0.2s ease-out; }
.tool-slide-leave-active { animation:tool-slide-in 0.12s ease-in reverse; }
@keyframes tool-slide-in {
  from { opacity:0; transform:translateY(-3px); }
  to   { opacity:1; transform:translateY(0); }
}
.tool-detail-group { margin-bottom:8px; }
.tool-detail-group:last-child { margin-bottom:0; }
.tool-detail-hdr { font-size: var(--ip-text-micro-size); font-weight:var(--ip-font-weight-semibold); color:var(--ip-color-text-tertiary); margin-bottom:4px; text-transform:uppercase; letter-spacing:0.5px; }
.tool-detail-hdr.hdr-err { color:var(--ip-danger-base); }
.tool-detail-code { font-size:var(--ip-text-caption-size); font-family:var(--ip-font-mono, monospace); white-space:pre-wrap; word-break:break-word; color:var(--ip-color-text-secondary); background:var(--ip-color-bg-tertiary); padding:6px 8px; border-radius:var(--ip-radius-sm); max-height:180px; overflow-y:auto; margin:0; line-height:1.5; }
.tool-detail-code.code-err { color:var(--ip-danger-base); }
.tool-detail-pending { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); font-style:italic; }
.proposal-wrapper { padding: 0 48px; }
.chat-error-banner { display:flex; align-items:flex-start; gap: var(--ip-spacing-2); margin:8px 16px; padding:10px 14px; background:#fef2f2; border:1px solid #fecaca; border-radius:var(--ip-radius-md); font-size:var(--ip-text-body-sm-size); }
.chat-error-icon { display:flex; align-items:center; justify-content:center; width:20px; height:20px; border-radius:50%; background:#ef4444; color:#fff; font-size:12px; font-weight:700; flex-shrink:0; }
.chat-error-text { color:#991b1b; line-height:1.5; word-break:break-word; }
</style>
