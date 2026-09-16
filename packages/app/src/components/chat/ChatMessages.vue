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
import { useRouter } from "vue-router";
import { ArrowLeftRight, AtSign, Brain, CornerUpRight, MessageSquareText, Shield, Wrench } from "@lucide/vue";
import { useChatStore } from "../../stores/chat";
import { useAgentStore } from "../../stores/agent";
import { useChannel, loadChannelNotices, type ElectionCard } from "../../composables/useChannel";
import { formatTime, formatDateLabel, parseDbTime } from "../../utils/time";
import MarkdownRenderer from "./MarkdownRenderer.vue";
import ConfigProposalCard from "./ConfigProposalCard.vue";
import DelegationCard from "./DelegationCard.vue";
import PlanCard from "./PlanCard.vue";
import StatusGlyph from "./StatusGlyph.vue";
import ImagePreview from "./ImagePreview.vue";
import AttachmentDetail from "./AttachmentDetail.vue";
import TurnRail from "./TurnRail.vue";
import ErrorBanner from "../common/ErrorBanner.vue";
import { useThinkingTimer } from "../../composables/useThinkingTimer";
import { useScrollFollow } from "../../composables/useScrollFollow";
import { useTurnRail } from "../../composables/useTurnRail";
import { useActiveTurn, THRESHOLD_PX } from "../../composables/useActiveTurn";
import { formatTokenCount, formatThinkingMs, formatFileSize } from "../../utils/format";
import { shortCode, parseReferenceBlocks, resolveGroupMid } from "../../utils/refs";
import type { ParsedRef } from "../../utils/refs";
import { incomingInfoOf, parseIncomingText, type IncomingInfo } from "../../utils/crossSession";
import { memoized } from "../../utils/blockMemo";
import { summarizeToolCall, dirnameOf, type ToolLineSummary } from "../../utils/toolSummary";
import { toolDisplayName } from "../../utils/toolLabels";
import { revealItemInDir, openPath } from "@tauri-apps/plugin-opener";
import ToolExpandDetail from "./ToolExpandDetail.vue";
import EntityAvatar from "../common/EntityAvatar.vue";
import ChannelNotice from "./ChannelNotice.vue";
import ChannelElectionCard from "./ChannelElectionCard.vue";
import type { ChannelMentionPayload, Message, MessageRole, PlanItem, SessionEvent } from "../../types";

const chat = useChatStore();
const agent = useAgentStore();
const router = useRouter();
const listRef = ref<HTMLElement | null>(null);

// 错误横幅行动按钮：这批 kind 的「怎么办」落在 agent 设置页（Key/端点/套餐），
// slug 与后端 error_mapping 的 LlmErrorKind::slug 一一对应，前端只做集合匹配
const CONFIG_FIXABLE_ERROR_KINDS = new Set([
  "llm.auth",
  "llm.forbidden",
  "llm.insufficient_balance",
  "llm.glm_resource_pack",
]);
const errorActionable = computed(() =>
  chat.lastErrorKind ? CONFIG_FIXABLE_ERROR_KINDS.has(chat.lastErrorKind) : false,
);

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
  void loadChannelNotices(cid); // 频道事件通知（非频道会话内部清空，幂等）
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
  // 跳转目标组处于默认收纳时的 UX 缺口（2026-09-16 机制审计）：@引用跳进的
  // 常见形态是被引段已收进「过程叙述」折叠行——落点只见一行摘要、被引内容仍
  // 藏着。先展开该组过程收纳再定位（展开改变组高，须在 offsetTop 量取前生效）。
  const jumpGroup = messageGroups.value.find((g) => g.key === "grp-" + messageId);
  if (jumpGroup && processCollapseEligible(jumpGroup) && !expandedProcessGroups.value.has(jumpGroup.key)) {
    const set = new Set(expandedProcessGroups.value);
    set.add(jumpGroup.key);
    expandedProcessGroups.value = set;
    await nextTick();
  }
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

// ③ 组级工具折叠的展开状态（默认折、存「已展开」——TrajectoryView collapsedTurns
// 的拷贝式 toggle 范式，方向相反）。键 = 组键（grp-<msgId>），与 expandedToolCalls
// （tool_use id）键空间不相交；折叠时不清理 expandedToolCalls——重展开恢复各行原展开态。
const expandedToolGroups = ref<Set<string>>(new Set());

function toggleToolGroup(key: string) {
  const set = new Set(expandedToolGroups.value);
  if (set.has(key)) set.delete(key); else set.add(key);
  expandedToolGroups.value = set;
}

// 组级思考聚合的展开状态（2026-09-16 拍板：组内 ≥2 段思考聚合到气泡顶部，
// 总开关 + 展开顶部堆叠）。键 = 组键，与 expandedToolGroups 同范式。
const expandedThinkingGroups = ref<Set<string>>(new Set());

function toggleThinkingGroup(key: string) {
  const set = new Set(expandedThinkingGroups.value);
  if (set.has(key)) set.delete(key); else set.add(key);
  expandedThinkingGroups.value = set;
}

// 组级过程叙述收纳的展开状态（2026-09-16 拍板：多轮回合过程碎句折叠、正中区
// 只留末段正文；展开=过程段回 item 原位）。键 = 组键，与前两组 Set 同范式。
const expandedProcessGroups = ref<Set<string>>(new Set());

function toggleProcessGroup(key: string) {
  const set = new Set(expandedProcessGroups.value);
  if (set.has(key)) set.delete(key); else set.add(key);
  expandedProcessGroups.value = set;
}

function toggleThinking(msgId: string) {
  const set = new Set(expandedThinking.value);
  if (set.has(msgId)) set.delete(msgId); else set.add(msgId);
  expandedThinking.value = set;
}

import { truncateJson } from "../../utils/format";

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

/** 字节数格式化已上移 utils/format（附件卡与工具行摘要共用单一真相源） */

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

// ===== MA-3 跨会话来件（incoming 卡）=====
// 消费回合物化的 user 消息带双块结构（后端 compose_incoming_blocks）+ 元数据
// （messages.incoming_source，migration 53）。解析两级：元数据优先（权威数据
// 源，格式演进免疫；正文取 content_blocks 末 Text 块），无元数据回退扁平文本
// 解析（legacy 消息 / 元数据坏行降级）。头部可点跳源会话（同 openReference 会话分支）。
/** 来件解析（消息级，元数据优先；模板每渲染多次调用，来源数据小解析开销可忽略） */
function incomingMsgOf(msg: Message): IncomingInfo | null {
  if (msg.incoming_source) return incomingInfoOf(msg);
  return incomingOf(msg.content ?? '');
}

/** 扁平文本解析（memo 化兜底路径：字符串参数即缓存键） */
const incomingOf = memoized((content: string): IncomingInfo | null => parseIncomingText(content));

/** 源会话是否可达（已删则头部纯展示，不可点） */
function sourceConvExists(convId: string): boolean {
  return chat.conversations.some((c) => c.id === convId);
}

function openIncomingSource(convId: string) {
  if (sourceConvExists(convId)) chat.selectConversation(convId);
}

/** 气泡正文：来件显正文块（元数据路径）/剥标注头（文本路径），普通消息走 cleanUserContent */
function userBubbleText(msg: Message): string {
  const inc = incomingMsgOf(msg);
  return inc ? inc.body : cleanUserContent(msg.content);
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

/** 解析 thinking 块。memo 化同上。durationMs = 后端落库的思考段耗时
 *  （旧消息无此字段 → null，label 只显示「思考」）。 */
const parseThinkingBlocks = memoized((contentBlocks: string): { thinking: string; durationMs: number | null }[] => {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { thinking: string; duration_ms?: number; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'thinking'
    ).map((b) => ({ thinking: b.thinking, durationMs: typeof b.duration_ms === 'number' ? b.duration_ms : null }));
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

// ===== 工具行展示（2026-09-07 拍板）：展示名 + 次级信息左置 + 文件名右锚可点 =====

/** 历史工具行摘要：memo 化（模板热路径每渲染每工具调用；findToolResult 每次
 *  computed 重建新对象，memo 键取内容值拼串——与 parseToolUseBlocks 同款模式，
 *  返回同引用勿 mutate）。流式行走直调（参数逐字变化，memo 无益）。 */
const summarizeCached = memoized((key: string): ToolLineSummary | null => {
  const [name, input, content, isError] = JSON.parse(key) as [
    string, string, string | null, boolean | null,
  ];
  return content != null
    ? summarizeToolCall(name, input, { content, isError: isError === true })
    : summarizeToolCall(name, input, null);
});

function summaryFor(tu: { id: string; name: string; input: string }): ToolLineSummary | null {
  const tr = findToolResult(tu.id);
  return summarizeCached(JSON.stringify([tu.name, tu.input, tr?.content ?? null, tr?.isError ?? null]));
}

/** 文件名点击：资源管理器中定位该文件（revealItemInDir 选中）；失败（文件已
 *  删/被移走）降级打开父目录，再失败仅 console 披露——fire-and-forget，
 *  AgentForm.openInExplorer 先例。 */
function revealFile(path: string) {
  revealItemInDir(path).catch(() => {
    const dir = dirnameOf(path);
    if (dir) {
      openPath(dir).catch((e) => console.warn("[ChatMessages] reveal 失败", path, e));
    }
  });
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

/** 本会话当前运行中的委派子会话 id（**降级兜底**，仅旧后端/映射缺失时用）。
 *  精确路径是 store 的 delegationChildByToolUse（delegation-started 事件带
 *  tool_use_id 登记）——同轮多卡并行委派时每张卡查自己的键；本 computed 只剩
 *  「事件缺 tool_use_id（旧后端）+ 会话级唯一 streaming」的兜底语义。 */
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
    childConvId: dr?.childConvId
      ?? (status === "running"
        ? (chat.delegationChildByToolUse.get(tu.id) ?? runningDelegationChildId.value)
        : null),
    finishReason: dr?.finishReason ?? null,
    rounds: dr?.rounds ?? null,
    hasError: tr?.isError ?? false,
  };
}

/** 流式中的委派卡片取数：call.arguments 逐字到达，call.result 到达即完成。
 *  running 跳转先查 store 的 tool_use 映射（delegation-started 登记），降级走
 *  会话级兜底（见 runningDelegationChildId 注释）。 */
function delegateStreamCard(call: {
  id: string; name: string; arguments: string; ended: boolean;
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
    childConvId: dr?.childConvId
      ?? (status === "running"
        ? (chat.delegationChildByToolUse.get(call.id) ?? runningDelegationChildId.value)
        : null),
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
  /** 频道成员 id（messages.sender_agent_id；null = 1v1 会话/无 enrichment）。
   *  分组维度之一：频道里不同成员的 assistant 各自成组，绝不合并。 */
  sender: string | null;
  items: GroupedItem[];
  firstIdx: number;
  lastIdx: number;
}

/** 把 chat.messages 按「连续 assistant + 同 model + 同 sender」分组。tool_result-only
 *  user 被跳过且不切断 assistant 连续性（其内容并入上一条 assistant 的工具卡片）。
 *  数据层 messages 不变。*/
const messageGroups = computed<MessageGroup[]>(() => {
  const out: MessageGroup[] = [];
  for (let i = 0; i < chat.messages.length; i++) {
    const msg = chat.messages[i];
    if (isToolResultOnlyUser(msg)) continue;
    // 频道选举投票行：票面已聚合进选举卡（下方 interstitial），气泡再显一遍
    // 即重复（生产反馈②：选举一件事零散多行）。窗口外旧选举无卡时 Set 不含
    // → 照常气泡渲染，自然回退。
    if (electionVoteIds.value.has(msg.id)) continue;
    const prev = out[out.length - 1];
    const sender = msg.sender_agent_id ?? null;
    const mergeable =
      msg.role === "assistant" &&
      prev !== undefined &&
      prev.role === "assistant" &&
      prev.model === (msg.model ?? null) &&
      prev.sender === sender;
    if (mergeable) {
      prev.items.push({ msg, idx: i });
      prev.lastIdx = i;
    } else {
      out.push({
        key: "grp-" + msg.id,
        role: msg.role,
        model: msg.model ?? null,
        sender,
        items: [{ msg, idx: i }],
        firstIdx: i,
        lastIdx: i,
      });
    }
  }
  return out;
});

// ===== 频道 v1：通知交错 + 成员身份头 + 生成中发出标注 =====
// 频道事件（选举/统筹/点名）是行为事实非消息——不进 messages，按 created_at
// 与消息组交错渲染（useChannel 尾部窗口拉取 + bus 增量）。
const { notices: channelNotices, electionCards, electionVoteIds } = useChannel();
const isChannelConv = computed(() => chat.activeConversation?.kind === "channel");

/** 交错渲染单元（频道）：通知条 | 选举聚合卡——统一按 created_at 排序与消息
 *  组交错。选举卡是 useChannel 对一届选举 started→vote×N→result 的渲染前聚合
 *  （数据层/轨迹 append-only 零改），取代零散通知行（生产反馈②）。 */
type Interstitial =
  | { kind: "notice"; key: string; time: number; event: SessionEvent }
  | { kind: "election"; key: string; time: number; card: ElectionCard };

const allInterstitials = computed<Interstitial[]>(() => {
  const ns: Interstitial[] = channelNotices.value.map((e) => ({
    kind: "notice", key: "ntc-" + e.id, time: parseDbTime(e.created_at).getTime(), event: e,
  }));
  const cs: Interstitial[] = electionCards.value.map((c) => ({
    kind: "election", key: "elc-" + c.key, time: parseDbTime(c.createdAt).getTime(), card: c,
  }));
  return [...ns, ...cs].sort((a, b) => a.time - b.time);
});

/** 渲染层分组：给每组注入「组开始前发生」的频道交错单元（preInterstitials）。
 *  每个交错单元只消费一次——挂到**首个**开始时间不早于它的组（双指针线性扫，
 *  消息组与交错单元均按时间升序）。比较符含等号（2026-09-11 实测修复⑨）：DB
 *  时间戳秒级粒度，通知事件在派发时刻落库、目标成员占位行在 Pipeline 后几毫秒
 *  创建——两者常落同一墙钟秒；严格小于会把通知滑过目标成员自己的组、呈现在其
 *  答完之后（时间感倒读，生产实案：两条接力通知均晚一组呈现、末条落尾部）。
 *  ⚠️ 勿回退成「每组 filter 全量 time < start」：
 *  历史卡/通知会随每条新消息重复出现（生产实案：每次发言前后都重复出选举卡，
 *  且 TransitionGroup 重复 key 引发错位渲染）。
 *  ⑬ 延递补充：吸入候选挂不上当前组时不就地居中——同秒平局下首跳点名会先被
 *  user 消息组吃掉（路由在物化后即刻派发，点名事件与用户行同墙钟秒）、
 *  multi-@ 的其余跳撞上同秒的他人组，就地消化则目标气泡永远拿不到图标；
 *  候选进 carry 向后续组递送直至目标组或尾部（isDeferrableMention）。 */
/** 吸入气泡身份行的接力来源标注（2026-09-11 拍板，⑫ 图标化）：被点名成员的
 *  昵称行尾挂一枚小图标标注「这条回复是被触发的」，不显示发起者名字（文字
 *  形态「@来源名」易被误读成气泡主人 @ 了谁）。图标两态直观分野发起者：
 *  AtSign = 用户直接 @ 点名；CornerUpRight = 成员间接力（统筹者分派或成员
 *  转 @）；无图标 = 广播直接应答（频道默认对话流）。发起者与完整句义进
 *  hover title——完整事实流仍在轨迹页。 */
interface MentionSource { key: string; userInitiated: boolean; title: string }

type NoticeItem = Extract<Interstitial, { kind: "notice" }>;

/** 延递判据（⑬）：未拦截且发起可归因的点名 = 吸入候选（成员接力 from≠null，
 *  或用户真 @ 点名 from=null 且非 broadcast——广播接令不候选，统筹者直接应答
 *  是默认对话流无需标注）。护栏拦截/广播/选举卡不延递——事实性内容按时间
 *  就地呈现；候选挂不上当前组时向后递送（见 renderGroups），全程无组可吸
 *  才落尾部居中条（目标不在加载窗口/已退出的落空兜底）。 */
function isDeferrableMention(it: Interstitial): it is NoticeItem {
  if (it.kind !== "notice") return false;
  const p = it.event.payload as ChannelMentionPayload;
  return p.blocked_reason == null && (!!p.from_agent_id || !p.broadcast);
}

/** 吸入判据：吸入候选 + 目标恰好是本组的发言成员——吸入只发生在「被点名者
 *  自己的气泡」上（护栏拦截类永不吸入，保留居中条显性可见）。 */
function absorbableInto(it: Interstitial, g: MessageGroup): it is NoticeItem {
  if (g.role !== "assistant" || !g.sender) return false;
  if (!isDeferrableMention(it)) return false;
  const p = it.event.payload as ChannelMentionPayload;
  return p.to_agent_id === g.sender;
}

function mentionSourceOf(it: NoticeItem, g: MessageGroup): MentionSource {
  const p = it.event.payload as ChannelMentionPayload;
  const toName = g.items[0].msg.sender_agent_name
    || (g.sender ? agent.getById(g.sender)?.name : undefined)
    || "已退出成员";
  if (p.from_agent_id) {
    const from = agent.getById(p.from_agent_id)?.name ?? "已退出成员";
    return { key: it.key, userInitiated: false, title: `${from} 点名 ${toName} 接力` };
  }
  return { key: it.key, userInitiated: true, title: `用户 点名 ${toName}` };
}

interface RenderGroup extends MessageGroup {
  preInterstitials: Interstitial[];
  mentionSource: MentionSource | null;
  /** 被本组吸入的交错单元 key 全集（含未成标注的多余条）——tail 消费集
   *  必须一并收走，否则吸入的通知掉到尾部双渲染 */
  absorbedKeys: string[];
}
const renderGroups = computed<RenderGroup[]>(() => {
  const items = allInterstitials.value;
  let cursor = 0;
  // 延递中的吸入候选：之前的组挂不上（同秒平局先撞 user 组/他人组），等
  // 后续组的目标气泡（⑬；终局 = 某组吸入 或 尾部居中条）
  let carry: NoticeItem[] = [];
  return messageGroups.value.map((g) => {
    const start = parseDbTime(g.items[0].msg.created_at).getTime();
    const pre: Interstitial[] = [];
    const absorbed: NoticeItem[] = [];
    const deferred: NoticeItem[] = [];
    while (cursor < items.length && items[cursor].time <= start) {
      const it = items[cursor];
      cursor += 1;
      if (absorbableInto(it, g)) absorbed.push(it);
      else if (isDeferrableMention(it)) deferred.push(it);
      else pre.push(it);
    }
    // 本窗口新候选 + 之前延递的一并复试：能吸则吸，挂不上继续延递
    const retry = [...carry, ...deferred];
    carry = [];
    for (const it of retry) {
      if (absorbableInto(it, g)) absorbed.push(it);
      else carry.push(it);
    }
    return {
      ...g,
      preInterstitials: pre,
      mentionSource: absorbed[0] ? mentionSourceOf(absorbed[0], g) : null,
      absorbedKeys: absorbed.map((it) => it.key),
    };
  });
});

/** 尾部交错单元：晚于最后一组开始的频道事件（在途接力的实时尾巴 / 进行中的
 *  选举卡——result 事件到达时 useChannel 补拉后卡自然翻完成态）+ 延递到底
 *  未找到目标气泡的点名（⑬ 落空兜底：目标不在加载窗口/已退出）。消费集含
 *  挂组（preInterstitials）与吸入（absorbedKeys）两路——漏收吸入 key 会让
 *  已吸入的通知在尾部再出一条居中条（双渲染）。 */
const tailInterstitials = computed(() => {
  const consumed = new Set<string>();
  for (const g of renderGroups.value) {
    for (const it of g.preInterstitials) consumed.add(it.key);
    for (const k of g.absorbedKeys) consumed.add(k);
  }
  return allInterstitials.value.filter((it) => !consumed.has(it.key));
});

/** 用户消息是否在最近前置 assistant 的生成窗口内发出（频道插话事实标注）。
 *  判据：该 assistant 的 created_at + turn_duration_ms（本轮生成窗口）晚于本条
 *  用户消息 created_at；无 duration 数据（旧消息未 enrich）诚实不标注。 */
function sentDuringGeneration(item: GroupedItem): boolean {
  if (item.msg.role !== "user") return false;
  for (let i = item.idx - 1; i >= 0; i--) {
    const m = chat.messages[i];
    if (m.role !== "assistant") continue; // 夹在中间的 tool_result-only user 跳过
    if (m.turn_duration_ms == null) return false;
    return parseDbTime(m.created_at).getTime() + m.turn_duration_ms
      > parseDbTime(item.msg.created_at).getTime();
  }
  return false;
}

/** 频道成员身份头取数（sender_agent_name 快照优先，agent store 兜底——成员
 *  可能已删；coordinator = 频道视图当前统筹位，非历史事实的当下投影）。 */
function channelSenderOf(g: MessageGroup): { name: string; image: string | null; coordinator: boolean } | null {
  const id = g.sender;
  if (!id) return null;
  const a = agent.getById(id);
  return {
    name: g.items[0].msg.sender_agent_name || a?.name || "已退出成员",
    image: a?.avatar ?? null,
    coordinator: chat.channelView?.coordinator_agent_id === id,
  };
}

/** 组级时间区间：开始（组首 created_at）→ 完成（组尾 created_at + 本轮生成
 *  耗时）。无 duration 数据或同值时只显开始。频道会话的 assistant 组 footer 用。 */
function groupTimeRange(g: MessageGroup): string {
  const first = chat.messages[g.firstIdx];
  const last = chat.messages[g.lastIdx];
  const start = formatTime(first.created_at);
  if (last.turn_duration_ms == null) return start;
  const end = formatTime(new Date(parseDbTime(last.created_at).getTime() + last.turn_duration_ms).toISOString());
  return end !== start ? `${start} → ${end}` : start;
}

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

// ===== ③ 组级工具折叠（生产实案：23% 回合 >20 次工具、峰值 150 行刷屏）=====
/** 折叠阈值：连续 assistant 组内通用工具行 ≥8 才折叠（少量工具不折，保留直观流）。 */
const TOOL_COLLAPSE_THRESHOLD = 8;

/** 豁免谓词：该 tool_use 渲染为结构化卡片（委派/计划）而非通用行。
 *  memo 化（summarizeCached 的 JSON-key 先例）；**计数与渲染必须共用同一判定**
 *  （本函数 ↔ delegateCardFor/planCardFor 的非 null 条件）——否则「摘要说 42
 *  实际显示 41」。委托 Err 的兜底通用行随折叠隐藏，错误态仍由卡片本体检出。 */
const structuredCardKind = memoized((key: string): "delegate" | "plan" | null => {
  const [name, input, isError] = JSON.parse(key) as [string, string, boolean];
  if (name === "delegate_to_agent") return "delegate";
  if (name === "update_plan" && !isError && parsePlanInput(input) != null) return "plan";
  return null;
});

function structuredCardKindOf(tu: { id: string; name: string; input: string }): "delegate" | "plan" | null {
  return structuredCardKind(JSON.stringify([tu.name, tu.input, getToolHasError(tu.id)]));
}

/** 各 assistant 组的工具行统计（跨 item 聚合——刷屏源是回合内多轮 × 每轮 3-5 条
 *  在合并组里累积，非单行爆炸）。豁免（结构化卡）不计入：摘要行数 = 被隐藏的
 *  通用行数，两口径天然一致。user 组不入表。 */
const groupToolStats = computed<Map<string, { total: number; errors: number }>>(() => {
  const stats = new Map<string, { total: number; errors: number }>();
  for (const g of messageGroups.value) {
    if (g.role !== "assistant") continue;
    let total = 0;
    let errors = 0;
    for (const it of g.items) {
      for (const tu of parseToolUseBlocks(it.msg.content_blocks)) {
        if (structuredCardKindOf(tu) !== null) continue;
        total++;
        if (getToolHasError(tu.id)) errors++;
      }
    }
    if (total > 0) stats.set(g.key, { total, errors });
  }
  return stats;
});

/** 组是否与本回合生成窗口相交（isTurnStreaming 的组级形态）：含 live item 的组
 *  恒 lastIdx ≥ turnFirstIdx → 永不折叠（已冻结轮工具卡在 TTFT/纯文本流式期必须
 *  可见，c9d2680 frozen-round 修复语义）；回合结束（clearTurnAnchors 置 null）才沉淀。 */
function groupInLiveTurn(g: MessageGroup): boolean {
  return chat.sending && chat.turnFirstIdx !== null && g.lastIdx >= chat.turnFirstIdx;
}

/** 组具备折叠资格（≥阈值且非生成中）——摘要行在折/展两态都渲染（toggle 载体）。 */
function toolCollapseEligible(g: MessageGroup): boolean {
  const s = groupToolStats.value.get(g.key);
  return !!s && s.total >= TOOL_COLLAPSE_THRESHOLD && !groupInLiveTurn(g);
}

function isToolsCollapsed(g: MessageGroup): boolean {
  return toolCollapseEligible(g) && !expandedToolGroups.value.has(g.key);
}

// ===== 组级思考聚合（2026-09-16 拍板：≥2 段聚合到气泡顶部，与工具折叠对称——
// 思考顶、正文中、工具总量底；生成中组不聚合——流式思考实时在场，回合结束沉淀）=====
/** 聚合区的一段思考。key 与 item 内 think-block 展开键同构（msgId + '-h' + 段序），
 *  聚合前后展开态互通；段耗时取块自身 duration_ms（持久口径——聚合区是历史回看
 *  视角，thinkingDurations 内存 map 是「刚结束」瞬态且 per-message 对段级无意义）。 */
interface GroupedThink { key: string; text: string; durationMs: number | null }

const groupThinkingStats = computed<Map<string, { segs: GroupedThink[]; totalMs: number | null }>>(() => {
  const stats = new Map<string, { segs: GroupedThink[]; totalMs: number | null }>();
  for (const g of messageGroups.value) {
    if (g.role !== "assistant") continue;
    const segs: GroupedThink[] = [];
    let totalMs = 0;
    let hasMs = false;
    for (const it of g.items) {
      parseThinkingBlocks(it.msg.content_blocks).forEach((b, ti) => {
        segs.push({ key: it.msg.id + "-h" + ti, text: b.thinking, durationMs: b.durationMs });
        if (b.durationMs != null) { totalMs += b.durationMs; hasMs = true; }
      });
    }
    // ≥2 段才聚合（1 段保持原位贴正文——短对话的思考-正文就近对应有价值）
    if (segs.length >= 2) stats.set(g.key, { segs, totalMs: hasMs ? totalMs : null });
  }
  return stats;
});

/** 组具备思考聚合资格（≥2 段且非生成中）。聚合生效时 item 内思考行恒隐藏
 *  （含「刚结束」驻留块——其内容已 freeze 进 content_blocks，聚合区承载）。 */
function thinkingAggregateEligible(g: MessageGroup): boolean {
  return groupThinkingStats.value.has(g.key) && !groupInLiveTurn(g);
}

/** 聚合段标签（镜像 item 内三态：块耗时 → 只显「思考」）。 */
function thinkSegLabel(seg: GroupedThink): string {
  return seg.durationMs != null ? "思考 · " + formatThinkingMs(seg.durationMs) : "思考";
}

// ===== 组级过程叙述收纳（2026-09-16 拍板；三轮：门控退役 + 截断标注 + 空壳治理保留）=====
// 三明治收纳（思考顶/正文中/工具底）对多轮长回合缺第三层收纳：中间轮次的冒号
// 碎句（「重新执行：」「修复后重跑：」失去对应工具行后成串悬空堆叠（真机实案：
// 16 次工具组 7 段正文全是过程叙述）。收纳后过程段折叠为「过程叙述 · N 段」，
// 展开回 item 原位（与工具折叠同交互——时间线与工具行交错复原）。
// 演进三轮（用户拍板「组合拳」）：
// ① 二轮末段收束门控（segConcluded 形态代理）**已退役**——dev 实测碎句组
//   （大组 38%）全段直显，「偏多偏杂」依旧；且生产库交叉表实证：异常终态
//   大组 95% 以冒号碎句收尾——**截断决定形态**，猜形态 = 猜截断，猜不准。
// ② 三轮换轴：判定从「猜末段形态」换成「读回合结尾事实」——组后首条真 user
//   消息是「继续」（撞顶续跑的形态签名，生产实案 60/62 走「继续」按钮）=
//   截断组，收纳行换「回合被截断 · N 段过程」标注（warning 色）——碎句尾段
//   被语境化，不再冒充结论；正常组维持「过程叙述 · N 段」。源头治理配套在
//   平台层提示词两行（system_prompt.rs 2026-09-16），一周后复测验证。
// ③ 空壳治理保留：收纳只藏 item 内容不收骨架，50 轮组 ≈ 30+ 个空
//   .message-item 以每层 12px 轮距堆出数百 px 空白（真机实案 380px+）——
//   空壳整个不渲染（visibleItemsOf）。
/** 收纳阈值：组内非空正文段 ≥3（过程段 ≥2）才收纳；两段以下多为实质正文
 *  （说明+结论的轻量两段），过度收纳损害阅读。 */
const PROCESS_NARRATIVE_MIN = 3;

interface GroupProcessStats { count: number; lastContentIdx: number }

/** 各 assistant 组的正文段统计：count = 非空 content 的 item 数，lastContentIdx
 *  = 末段所在 item 下标（末 item 可能是无正文纯工具轮——末段=最后有 content 者）。 */
const groupProcessStats = computed<Map<string, GroupProcessStats>>(() => {
  const stats = new Map<string, GroupProcessStats>();
  for (const g of messageGroups.value) {
    if (g.role !== "assistant") continue;
    let count = 0;
    let lastContentIdx = -1;
    for (const it of g.items) {
      if (it.msg.content) { count++; lastContentIdx = it.idx; }
    }
    if (count >= PROCESS_NARRATIVE_MIN) {
      stats.set(g.key, { count, lastContentIdx });
    }
  }
  return stats;
});

/** 组具备收纳资格（≥阈值 + 非生成中——frozen-round 语义：流式正文实时在场，
 *  回合结束沉淀）。默认收纳全部达阈值组（三轮门控退役）。 */
function processCollapseEligible(g: MessageGroup): boolean {
  return groupProcessStats.value.has(g.key) && !groupInLiveTurn(g);
}

/** 组后首条真 user 消息是否是「继续」——回合撞顶被截断后用户续跑的形态签名
 *  （生产实案 60/62 续跑全走「继续」按钮 = sendMessage('继续')）。扫描跳过
 *  tool_result-only 行；遇 assistant（换档降级/频道新成员组）= 非续跑。末组
 *  无后续 → false（截断后未续跑的末组拿不到信号，碎句尾段直显无标注——罕见
 *  口径，完备解是读 turn_ended 事件，暂不做）。 */
function groupTruncatedAfter(g: MessageGroup): boolean {
  for (let i = g.lastIdx + 1; i < chat.messages.length; i++) {
    const m = chat.messages[i];
    if (isToolResultOnlyUser(m)) continue;
    if (m.role !== "user") return false;
    return (m.content ?? "").trim() === "继续";
  }
  return false;
}

/** 收纳行标签：截断组「回合被截断 · N 段过程」（warning 色由模板 class 承载）、
 *  正常组「过程叙述 · N 段」；展开态「收起 · N 段过程叙述」。N = 过程段数
 *  （总段数 - 末段）。 */
function processRowLabel(g: MessageGroup, collapsed: boolean): string {
  const n = (groupProcessStats.value.get(g.key)?.count ?? 0) - 1;
  if (!collapsed) return `收起 · ${n} 段过程叙述`;
  return groupTruncatedAfter(g) ? `回合被截断 · ${n} 段过程` : `过程叙述 · ${n} 段`;
}

function isProcessCollapsed(g: MessageGroup): boolean {
  return processCollapseEligible(g) && !expandedProcessGroups.value.has(g.key);
}

// ===== 分页前插合并的收纳态保全（2026-09-16 机制审计修复；同日七轮拍板改版）=====
// loadMoreMessages 前插一页更早消息时，窗口原首组与新页尾续同 model/sender 则
// 并组——组键 grp-<首条id> 随组头易主而变。三个收纳展开集按键存档，不转移 =
// 用户手动展开被静默重置回收纳。锚 = 组末条消息 id（前插只动组头、组尾恒定；
// 尾部追加不变键）。**预置展开分支已按用户拍板移除（2026-09-16 七轮）：翻历史
// 分页时三收纳一律默认收起——原「原本全可见组并入达阈值 → 预置展开（阅读连续
// 优先）」被推翻，读到折叠行想看再点开，不代替用户做展开决定。**
function transferGroupExpansion(
  setRef: typeof expandedToolGroups,
  oldKey: string,
  newKey: string,
) {
  if (!setRef.value.has(oldKey)) return;
  const set = new Set(setRef.value);
  set.delete(oldKey);
  set.add(newKey);
  setRef.value = set;
}

watch(messageGroups, (groups, prev) => {
  if (!prev) return;
  const oldByTail = new Map<string, MessageGroup>();
  for (const og of prev) oldByTail.set(og.items[og.items.length - 1].msg.id, og);
  for (const g of groups) {
    const og = oldByTail.get(g.items[g.items.length - 1].msg.id);
    if (!og || og.key === g.key) continue;
    transferGroupExpansion(expandedToolGroups, og.key, g.key);
    transferGroupExpansion(expandedThinkingGroups, og.key, g.key);
    transferGroupExpansion(expandedProcessGroups, og.key, g.key);
  }
});

/** 折叠态下该 item 的正文应隐藏（非末段正文 item）；展开态恒 false（回原位）。 */
function isProcessNarrativeItem(g: MessageGroup, item: GroupedItem): boolean {
  if (!isProcessCollapsed(g)) return false;
  const s = groupProcessStats.value.get(g.key);
  return s != null && item.idx !== s.lastContentIdx;
}

/** 折叠态下该 item 是否已无任何可见内容（空壳）。可见性四路：流式通道（live
 *  item 恒在场）/ 正文（非过程收纳段）/ 思考（组未聚合且本 item 有思考块，或
 *  刚结束驻留块）/ 工具（未折叠或含豁免卡）。三收纳在生成中组全不生效，
 *  生成中组天然全可见；「刚结束」驻留思考块也保 item 在场（其载体在本 item）。 */
function itemCollapsedAway(g: MessageGroup, item: GroupedItem): boolean {
  if (isLiveAssistant(item)) return false;
  if (item.msg.content && !isProcessNarrativeItem(g, item)) return false;
  const thinkVisible =
    !thinkingAggregateEligible(g) &&
    (parseThinkingBlocks(item.msg.content_blocks).length > 0 ||
      (isLastAssistant(item) && !!chat.thinkingDuration && !!chat.lastThinkingContent));
  if (thinkVisible) return false;
  const tus = parseToolUseBlocks(item.msg.content_blocks);
  if (tus.length > 0 && (!isToolsCollapsed(g) || tus.some((tu) => structuredCardKindOf(tu) !== null))) {
    return false;
  }
  return true;
}

/** 组内当前应渲染的 item（滤掉空壳）。收纳只藏内容不收 .message-item 骨架的
 *  旧形态，会让 50 轮组 ≈ 30+ 个空壳以每层 12px 轮距堆出大片空白（真机实案
 *  380px+，2026-09-16 二轮）——直接从 v-for 数据源滤除，空壳不进 DOM。 */
function visibleItemsOf(g: MessageGroup): GroupedItem[] {
  return g.items.filter((item) => !itemCollapsedAway(g, item));
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
  // P10④ doom_loop：同一工具以同类方式反复失败（连纠正指令都无效），自动终止止损
  doom_loop: "同一工具反复失败，已自动终止",
  tool_use: "已达工具调用轮数上限",
};

/** finish_reason 文案：budget_exceeded 在有预算数据时附具体数字
 *  （已用 X / 上限 Y），让用户知道为什么这么快烧完、还剩多少跑道。
 *  tool_use 按本回合是否发生过续期分叉（①-2）：无续期=用户配置的显式上限
 *  （指路 agent.yaml）；有续期=默认额度用尽（含自动续期，指路改配置耗额度）。 */
function finishReasonLabel(reason: string): string {
  if (reason === "budget_exceeded" && chat.budget) {
    const b = chat.budget;
    return `本次 token 预算已达上限（已用 ${formatTokenCount(b.cumulative_tokens)} / 上限 ${formatTokenCount(b.effective_cap)}）`;
  }
  if (reason === "tool_use" && chat.lastTurnRounds != null) {
    return chat.turnRoundRenewals
      ? `已达最大轮数（${chat.lastTurnRounds} 轮，含自动续期）`
      : `已到你配置的 ${chat.lastTurnRounds} 轮上限（可在 agent.yaml 调整）`;
  }
  return finishReasonLabels[reason] || reason;
}
// 「发送消息即可续跑」的终止类：提示行内附「继续」按钮（abort=用户主动停，不列）
const RESUMABLE_REASONS = new Set([
  "budget_exceeded",
  "tool_use",
  "stuck",
  "doom_loop",
  "length",
  "max_tokens",
]);
</script>

<template>
  <!-- wrap：相对定位包裹层——承载「跳到最新」右侧轨道按钮（独立于滚动容器，
       绝对定位子元素若挂在滚动容器内会随内容滚动，位置漂移） -->
  <!-- fade-off：贴底跟随（autoFollow）时撤底缘渐隐——正在生成的消息尾部
       不再压在半透明带下；滚上读历史时渐隐回归「下面还有内容」示向 -->
  <div class="messages-wrap" :class="{ 'fade-off': autoFollow }">
    <div ref="listRef" class="messages-area">
    <!-- 错误提示（UI-2 批次三：ErrorBanner 化——有失败发送可重发，否则仅关闭；
         配置类错误附「去检查配置」直达 agent 设置，怎么办不再只有一句话） -->
    <ErrorBanner
      v-if="chat.lastError"
      variant="banner"
      class="chat-error-banner"
      :title="chat.lastFailedSend ? '发送失败' : '请求出错'"
      :detail="chat.lastError"
      :retry-label="chat.lastFailedSend ? '重试' : null"
      :action-label="errorActionable ? '去检查配置' : undefined"
      dismissible
      @retry="retryLastSend"
      @action="router.push('/settings/agents')"
      @dismiss="chat.clearConvError()"
    />
    <!-- 分页加载指示器 -->
    <div v-if="chat.loadingMore" class="load-more-hint">加载更早消息…</div>
    <div v-if="!chat.hasMore && chat.pagedOnce" class="load-more-hint load-more-end">已显示全部消息</div>

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
      <template v-for="group in renderGroups" :key="group.key">
        <!-- 日期分组标签（基于组首）-->
        <div v-if="isNewDay(group.firstIdx)" class="date-divider">{{ formatDateLabel(chat.messages[group.firstIdx].created_at) }}</div>
        <!-- 频道交错单元（组开始前发生：通知条/选举聚合卡，按 created_at 交错） -->
        <template v-for="it in group.preInterstitials" :key="it.key">
          <ChannelElectionCard v-if="it.kind === 'election'" :card="it.card" />
          <ChannelNotice v-else :event="it.event" />
        </template>
        <!-- data-mid=组首消息 id：useScrollFollow 锚点捕获/恢复的 DOM 定位符。
             channel-msg：频道 assistant 组——外层气泡退位（背景移 .assistant-body）-->
        <div :class="['message-group', group.role, { 'channel-msg': isChannelConv && group.role === 'assistant' }]" :data-mid="group.items[0].msg.id">
          <!-- ===== 用户消息组（单条，透明壳）===== -->
          <template v-if="group.role === 'user'">
            <div class="message-content user">
              <div v-if="cleanUserContent(group.items[0].msg.content) || hasUserMedia(group.items[0].msg) || parseReferenceBlocks(group.items[0].msg.content_blocks).length > 0" class="message-bubble">
                <!-- MA-3 跨会话来件：来源标注头（点击跳源会话；源已删纯展示）。
                     检测 = incoming_source 元数据优先（权威），无元数据回退
                     [来自会话「 前缀文本解析（legacy） -->
                <div
                  v-if="incomingMsgOf(group.items[0].msg)"
                  class="user-incoming-head"
                  :class="{ clickable: sourceConvExists(incomingMsgOf(group.items[0].msg)!.sourceConvId) }"
                  :title="sourceConvExists(incomingMsgOf(group.items[0].msg)!.sourceConvId) ? '打开源会话' : '源会话已删除'"
                  @click="openIncomingSource(incomingMsgOf(group.items[0].msg)!.sourceConvId)"
                >
                  <ArrowLeftRight :size="13" class="incoming-icon" aria-hidden="true" />
                  <span class="incoming-src">来自「{{ incomingMsgOf(group.items[0].msg)!.sourceTitle }}」的 agent {{ incomingMsgOf(group.items[0].msg)!.agentName }}</span>
                  <span class="incoming-pill">跨会话消息</span>
                </div>
                <span v-if="userBubbleText(group.items[0].msg)" class="user-text">{{ userBubbleText(group.items[0].msg) }}</span>

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
                  <!-- 频道插话事实标注：发出时上一条回答仍在生成（不打断在途回合） -->
                  <span v-if="isChannelConv && sentDuringGeneration(group.items[0])" class="gen-time-flag" title="发出时上一条回答仍在生成中——插话不打断在途回合">生成中发出</span>
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
            <!-- 频道群聊布局（2026-09-11 拍板）：参考常见聊天软件的群聊气泡——频道态
                 wrap 转 grid 两列（头像独立左列 36px + 身体列），头像气泡外、昵称在
                 气泡上方；气泡背景/圆角/padding 下放给 .assistant-body（外层退位透明）。
                 1v1 态 wrap/body 双层透明直通，视觉与旧结构等价（头部已有 agent 身份）。
                 单值包装取数（委派卡同款先例）；统筹者 Shield 是当下投影非历史事实 -->
            <div class="assistant-wrap" :class="{ channel: isChannelConv }">
              <template v-for="sender in [channelSenderOf(group)]" :key="sender ? 'ch-head' : 'ch-none'">
                <EntityAvatar v-if="isChannelConv && sender" :name="sender.name" :image="sender.image" size="lg" class="channel-avatar" />
                <div v-if="isChannelConv && sender" class="channel-sender-head">
                  <span class="channel-sender-name">{{ sender.name }}</span>
                  <Shield v-if="sender.coordinator" :size="12" class="channel-sender-shield" aria-hidden="true" />
                  <!-- 触发来源吸入标注（⑫ 图标化）：本气泡成员是被谁唤醒的——
                       AtSign=用户 @ 点名 / CornerUpRight=成员间接力；发起者进 hover title -->
                  <span v-if="group.mentionSource" class="channel-mention-src" :title="group.mentionSource.title">
                    <AtSign v-if="group.mentionSource.userInitiated" :size="12" aria-hidden="true" />
                    <CornerUpRight v-else :size="12" aria-hidden="true" />
                  </span>
                </div>
              </template>
              <div class="assistant-body">
            <!-- 组级收纳胶囊行（2026-09-16 五轮拍板：三行合一置气泡最前端）——
                 思考/工具/过程三胶囊并排（序固定 思考→工具→过程），各配语义
                 Lucide 图标（Brain/Wrench/MessageSquareText——图标表内容域，
                 原状态图标 done/error 与收纳语义不符，用户拍板换掉）。
                 置顶而非置底：底部工具行展开向上顶会把点击控件推出视野；置顶
                 展开向下流（details 语感），控件钉在位。
                 六轮（用户反馈「不够明显、区别度不够」）：胶囊 chrome 对齐房内
                 chip 语言（ChatHeader 频道 tag 徽章）——收起=实底软色胶囊（供能
                 暗示「这里有内容」）、展开=幽灵描边胶囊（is-open，控件退位）；
                 div→button 化 + aria-expanded（键盘可达基线）。
                 ⚠️ 胶囊必须在 item v-for 之外（与 message-item 同级——组级错位
                 教训：落 item 内 = 每轮一条重复摘要行）。 -->
            <div v-if="thinkingAggregateEligible(group) || toolCollapseEligible(group) || processCollapseEligible(group)" class="group-summary-pills">
              <button v-if="thinkingAggregateEligible(group)" type="button" class="think-toggle summary-pill think-group-summary" :class="{ 'is-open': expandedThinkingGroups.has(group.key) }" :aria-expanded="expandedThinkingGroups.has(group.key)" @click="toggleThinkingGroup(group.key)">
                <Brain :size="14" class="pill-glyph" aria-hidden="true" />
                <span class="think-label">思考 · {{ groupThinkingStats.get(group.key)?.segs.length }} 段</span>
                <span v-if="groupThinkingStats.get(group.key)?.totalMs != null" class="think-label think-group-total">{{ formatThinkingMs(groupThinkingStats.get(group.key)!.totalMs!) }}</span>
                <span class="think-chevron">{{ expandedThinkingGroups.has(group.key) ? '▾' : '▸' }}</span>
              </button>
              <button v-if="toolCollapseEligible(group)" type="button" class="tool-toggle summary-pill tool-group-summary" :class="{ 'is-open': !isToolsCollapsed(group) }" :aria-expanded="!isToolsCollapsed(group)" @click="toggleToolGroup(group.key)">
                <Wrench :size="14" class="pill-glyph" aria-hidden="true" />
                <template v-if="isToolsCollapsed(group)">
                  <span class="tool-name">{{ groupToolStats.get(group.key)?.total }} 次工具调用</span>
                  <span v-if="(groupToolStats.get(group.key)?.errors ?? 0) > 0" class="tool-fail-count">{{ groupToolStats.get(group.key)?.errors }} 失败</span>
                </template>
                <span v-else class="tool-name">收起 · {{ groupToolStats.get(group.key)?.total }} 次工具调用</span>
                <span class="tool-chevron">{{ isToolsCollapsed(group) ? '▸' : '▾' }}</span>
              </button>
              <button v-if="processCollapseEligible(group)" type="button" class="tool-toggle summary-pill process-group-summary" :class="{ 'is-open': !isProcessCollapsed(group) }" :aria-expanded="!isProcessCollapsed(group)" @click="toggleProcessGroup(group.key)">
                <MessageSquareText :size="14" class="pill-glyph" aria-hidden="true" />
                <span class="tool-name" :class="{ 'process-truncated': isProcessCollapsed(group) && groupTruncatedAfter(group) }">{{ processRowLabel(group, isProcessCollapsed(group)) }}</span>
                <span class="tool-chevron">{{ isProcessCollapsed(group) ? '▸' : '▾' }}</span>
              </button>
            </div>
            <!-- 组级思考聚合（≥2 段）：收起=胶囊总量，展开=胶囊行下堆叠各段
                 （段内交互照旧，展开键与 item 内同构——聚合前后互通）。 -->
            <Transition name="think-fade">
              <div v-if="thinkingAggregateEligible(group) && expandedThinkingGroups.has(group.key)" class="think-group-stack">
                <div v-for="seg in groupThinkingStats.get(group.key)?.segs" :key="seg.key" class="think-block">
                  <div class="think-toggle" @click="toggleThinking(seg.key)">
                    <StatusGlyph status="done" class="think-glyph" />
                    <span class="think-label">{{ thinkSegLabel(seg) }}</span>
                    <span class="think-chevron">{{ expandedThinking.has(seg.key) ? '▾' : '▸' }}</span>
                  </div>
                  <Transition name="think-fade">
                    <div v-if="expandedThinking.has(seg.key)" class="think-body">
                      <MarkdownRenderer :content="seg.text" />
                    </div>
                  </Transition>
                </div>
              </div>
            </Transition>
            <div v-for="item in visibleItemsOf(group)" :key="item.msg.id" class="message-item">
              <!-- 三个点动画：仅当前流式 item 且无任何返回时显示 -->
              <div v-if="isLiveAssistant(item) && item.msg.content === '' && !chat.streamingThinking && toolCallList.length === 0" class="think-dots">
                <span class="think-dot" /><span class="think-dot" /><span class="think-dot" />
              </div>

              <!-- 骨架隐藏（think-dots 之外的空占位不渲染内部模板）只属于当前流式 item：
                   用 item 级 isLiveAssistant 判定，勿用全局 chat.sending——多轮工具回合里
                   已冻结轮（如纯工具轮：无文本无思考，工具卡全在 content_blocks）在下一轮
                   TTFT/纯文本流式期间会被整块藏掉，直到下一轮首个 tool-call-start/thinking
                   才救回（生产实案 2026-09-03：上轮工具记录间歇性消失、done 后全恢复）。 -->
              <template v-if="item.msg.content || !isLiveAssistant(item) || chat.streamingThinking || toolCallList.length > 0">
                <!-- 思考过程（历史消息）；末条且 done 块显示时跳过避免重复；组级
                     思考聚合生效时恒隐藏（顶部聚合区承载，防双渲染） -->
                <template v-for="(think, ti) in parseThinkingBlocks(item.msg.content_blocks)" :key="'think-' + item.msg.id + '-' + ti">
                  <div v-if="!thinkingAggregateEligible(group) && !(isLastAssistant(item) && chat.thinkingDuration && chat.lastThinkingContent)" class="think-block">
                    <div class="think-toggle" @click="toggleThinking(item.msg.id + '-h' + ti)">
                      <StatusGlyph status="done" class="think-glyph" />
                      <!-- 耗时两级来源：内存 thinkingDurations（本轮会话，含多轮中间轮）
                           → 块内 duration_ms（后端落库，重启后兜底）→ 无则只显示「思考」 -->
                      <span class="think-label">{{ chat.thinkingDurations.has(item.msg.id) ? '思考 · ' + chat.thinkingDurations.get(item.msg.id) : think.durationMs != null ? '思考 · ' + formatThinkingMs(think.durationMs) : '思考' }}</span>
                      <span class="think-chevron">{{ expandedThinking.has(item.msg.id + '-h' + ti) ? '▾' : '▸' }}</span>
                    </div>
                    <Transition name="think-fade">
                      <div v-if="expandedThinking.has(item.msg.id + '-h' + ti)" class="think-body">
                        <MarkdownRenderer :content="think.thinking" />
                      </div>
                    </Transition>
                  </div>
                </template>

                <!-- 思考过程（流式 / 刚结束，带切换动画） -->
                <Transition name="think-swap" mode="out-in">
                  <div v-if="isLiveAssistant(item) && chat.streamingThinking" key="live" class="think-block">
                    <div class="think-toggle" @click="toggleThinking('streaming')">
                      <StatusGlyph status="running" class="think-glyph" />
                      <span class="think-label">思考</span>
                      <span class="think-status">进行中… {{ thinkingElapsed }}</span>
                      <span class="think-chevron">{{ expandedThinking.has('streaming') ? '▾' : '▸' }}</span>
                    </div>
                    <Transition name="think-fade">
                      <div v-if="expandedThinking.has('streaming')" class="think-body">
                        <MarkdownRenderer :content="chat.streamingThinking" streaming />
                      </div>
                    </Transition>
                  </div>
                  <div v-else-if="!thinkingAggregateEligible(group) && isLastAssistant(item) && chat.thinkingDuration && chat.lastThinkingContent" key="done" class="think-block">
                    <div class="think-toggle" @click="toggleThinking('done')">
                      <StatusGlyph status="done" class="think-glyph" />
                      <span class="think-label">思考 · {{ chat.thinkingDuration }}</span>
                      <span class="think-chevron">{{ expandedThinking.has('done') ? '▾' : '▸' }}</span>
                    </div>
                    <Transition name="think-fade">
                      <div v-if="expandedThinking.has('done')" class="think-body">
                        <MarkdownRenderer :content="chat.lastThinkingContent" />
                      </div>
                    </Transition>
                  </div>
                </Transition>

                <!-- 文字（按时间线顺序：thinking → 文本 → 工具，匹配 content_blocks）。
                     过程收纳折叠态：非末段正文隐藏（收进顶部过程胶囊）；展开回原位 -->
                <div v-if="item.msg.content && !isProcessNarrativeItem(group, item)" class="message-bubble">
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
                        <template v-if="d.hasError && !isToolsCollapsed(group)">
                          <div class="tool-toggle" @click="toggleToolCall(tu.id)">
                            <StatusGlyph status="error" />
                            <span class="tool-name">{{ toolDisplayName(tu.name) }}</span>
                            <span class="tool-preview">调用失败</span>
                            <span class="tool-chevron">{{ expandedToolCalls.has(tu.id) ? '▾' : '▸' }}</span>
                          </div>
                          <Transition name="tool-slide">
                            <div v-if="expandedToolCalls.has(tu.id)" class="tool-expand">
                              <ToolExpandDetail :name="tu.name" :args-json="tu.input" :result="findToolResult(tu.id)" />
                            </div>
                          </Transition>
                        </template>
                      </template>
                      <template v-else>
                        <!-- C5：update_plan 渲染为计划卡片；校验失败（Err）落回通用行承载原始错误 -->
                        <template v-for="p in [planCardFor(tu)]" :key="p ? 'plan-card' : 'plan-none'">
                          <PlanCard v-if="p" :items="p" @open-task="openChildConv" />
                          <template v-else>
                            <!-- ③ 组级折叠：折叠时通用行整体隐藏（委派/计划卡豁免仍可见，
                                 总量摘要行见组尾）；豁免判定与计数共用 structuredCardKind -->
                            <template v-if="!isToolsCollapsed(group)">
                            <!-- 工具行摘要（P1 工具）：展示名 + 次级信息左置 + 文件名右锚可点；
                                 非 P1 / 参数畸形 → 通用行（展示名词表降级英文原值） -->
                            <template v-for="s in [summaryFor(tu)]" :key="s ? 'sum' : 'sum-none'">
                              <div v-if="s" class="tool-toggle" @click="toggleToolCall(tu.id)">
                                <StatusGlyph :status="getToolHasError(tu.id) ? 'error' : 'done'" />
                                <span class="tool-name">{{ s.display }}</span>
                                <span v-if="s.diff" class="tool-diff" title="行级变更：+ 新增 - 删除 ~ 修改">
                                  <span v-if="s.diff.added" class="diff-add">+{{ s.diff.added }}</span>
                                  <span v-if="s.diff.removed" class="diff-del">-{{ s.diff.removed }}</span>
                                  <span v-if="s.diff.changed" class="diff-mod">~{{ s.diff.changed }}</span>
                                </span>
                                <span v-if="s.secondary" class="tool-secondary">{{ s.secondary }}</span>
                                <span class="tool-file" title="在资源管理器中显示" @click.stop="revealFile(s.revealPath)">{{ s.fileLabel }}</span>
                                <span class="tool-chevron">{{ expandedToolCalls.has(tu.id) ? '▾' : '▸' }}</span>
                              </div>
                              <div v-else class="tool-toggle" @click="toggleToolCall(tu.id)">
                                <StatusGlyph :status="getToolHasError(tu.id) ? 'error' : 'done'" />
                                <span class="tool-name">{{ toolDisplayName(tu.name) }}</span>
                                <span class="tool-preview">{{ truncateJson(tu.input) }}</span>
                                <span class="tool-chevron">{{ expandedToolCalls.has(tu.id) ? '▾' : '▸' }}</span>
                              </div>
                              <Transition name="tool-slide">
                                <div v-if="expandedToolCalls.has(tu.id)" class="tool-expand">
                                  <ToolExpandDetail :name="tu.name" :args-json="tu.input" :result="findToolResult(tu.id)" />
                                </div>
                              </Transition>
                            </template>
                            </template>
                          </template>
                        </template>
                      </template>
                    </template>
                  </div>
                </div>

                <!-- 工具调用（当前流式）。③ 折叠互斥结构性成立：含 live item 的组
                     恒 lastIdx ≥ turnFirstIdx → groupInLiveTurn → 永不折叠，本块零改动 -->
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
                            <!-- 流式工具行摘要：直调（参数逐字变化，memo 无益）；参数到齐瞬间
                                 从通用行切结构行（delegate 卡同款切换形态） -->
                            <template v-for="s in [summarizeToolCall(call.name, call.arguments || '{}', call.result ?? null)]" :key="s ? 'sum' : 'sum-none'">
                              <div v-if="s" class="tool-toggle" @click="toggleToolCall(call.id)">
                                <StatusGlyph
                                  v-if="call.ended && call.result"
                                  :status="call.result.isError ? 'error' : 'done'"
                                />
                                <StatusGlyph v-else-if="call.ended" status="wait" />
                                <StatusGlyph v-else status="running" variant="spinner" />
                                <span class="tool-name">{{ s.display }}</span>
                                <span v-if="call.result?.durationMs" class="tool-duration">{{ formatDuration(call.result.durationMs) }}</span>
                                <span v-if="s.diff" class="tool-diff" title="行级变更：+ 新增 - 删除 ~ 修改">
                                  <span v-if="s.diff.added" class="diff-add">+{{ s.diff.added }}</span>
                                  <span v-if="s.diff.removed" class="diff-del">-{{ s.diff.removed }}</span>
                                  <span v-if="s.diff.changed" class="diff-mod">~{{ s.diff.changed }}</span>
                                </span>
                                <span v-if="s.secondary" class="tool-secondary">{{ s.secondary }}</span>
                                <span class="tool-file" title="在资源管理器中显示" @click.stop="revealFile(s.revealPath)">{{ s.fileLabel }}</span>
                                <span class="tool-chevron">{{ expandedToolCalls.has(call.id) ? '▾' : '▸' }}</span>
                              </div>
                              <div v-else class="tool-toggle" @click="toggleToolCall(call.id)">
                                <StatusGlyph
                                  v-if="call.ended && call.result"
                                  :status="call.result.isError ? 'error' : 'done'"
                                />
                                <StatusGlyph v-else-if="call.ended" status="wait" />
                                <StatusGlyph v-else status="running" variant="spinner" />
                                <span class="tool-name">{{ toolDisplayName(call.name) }}</span>
                                <span v-if="call.result?.durationMs" class="tool-duration">{{ formatDuration(call.result.durationMs) }}</span>
                                <span class="tool-preview">{{ truncateJson(call.arguments || '') }}</span>
                                <span class="tool-chevron">{{ expandedToolCalls.has(call.id) ? '▾' : '▸' }}</span>
                              </div>
                              <Transition name="tool-slide">
                                <div v-if="expandedToolCalls.has(call.id)" class="tool-expand">
                                  <ToolExpandDetail :name="call.name" :args-json="call.arguments || '{}'" :result="call.result ?? null" />
                                </div>
                              </Transition>
                            </template>
                          </template>
                        </template>
                      </template>
                    </template>
                  </div>
                </div>
              </template>
            </div>

            <!-- 组级 footer：时间(组首) / model(一次) / token(求和) / 复制(组内文本)。
                 频道会话时间带完成点（组尾 created_at + 本轮生成耗时） -->
            <div v-if="assistantGroupFooterVisible(group)" class="message-footer">
              <div class="footer-left">
                <span class="message-time">{{ isChannelConv ? groupTimeRange(group) : formatTime(chat.messages[group.firstIdx].created_at) }}</span>
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
              </div><!-- /assistant-body：频道态=气泡体（背景/圆角/padding 承接者） -->
            </div><!-- /assistant-wrap：频道态=群聊行（头像列+身体列 grid） -->
          </template>
        </div>
      </template>
      <!-- 频道交错单元（末组之后：在途接力通知 / 进行中或刚完成的选举卡） -->
      <template v-for="it in tailInterstitials" :key="it.key">
        <ChannelElectionCard v-if="it.kind === 'election'" :card="it.card" />
        <ChannelNotice v-else :event="it.event" />
      </template>
    </TransitionGroup>

    <!-- 配置提案审批卡片（内联） -->
    <div v-if="chat.pendingProposal" class="proposal-wrapper">
      <ConfigProposalCard :proposal="chat.pendingProposal" />
    </div>

    <!-- finish_reason 提示（B3：可续跑类附「继续」按钮，一键发「继续」续跑任务） -->
    <div v-if="chat.lastFinishReason && chat.lastFinishReason !== 'stop' && chat.lastFinishReason !== 'end_turn' && chat.messages.length > 0" class="finish-reason">
      <span>{{ finishReasonLabel(chat.lastFinishReason) }}</span>
      <!-- 预算 HUD 已迁 ChatInput 输入框下方（2026-08-22）；budget_exceeded 的
           具体数字仍由 finishReasonLabel 行内联 -->
      <button
        v-if="RESUMABLE_REASONS.has(chat.lastFinishReason) && !chat.sending"
        class="continue-btn"
        title="任务状态完好，发送「继续」即可接着跑"
        @click="chat.sendMessage('继续')"
      >继续</button>
    </div>

    <div v-if="chat.sending && chat.messages.length > 0" class="cursor-bar">
      <div class="cursor-track">
        <StatusGlyph status="running" /><span class="cursor-label">正在生成…</span>
      </div>
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

    <!-- 降级换档 toast（非阻塞，5s 自动消失；预算续期 toast 同款定位与视觉） -->
    <Transition name="budget-toast">
      <div v-if="chat.modelSwitchNotice" class="budget-renewal-toast">
        <span class="budget-renewal-text">{{ chat.modelSwitchNotice }}</span>
      </div>
    </Transition>

    <!-- 工具轮数续期 toast（① 自动续跑；前两个 toast 同款） -->
    <Transition name="budget-toast">
      <div v-if="chat.roundsNotice" class="budget-renewal-toast">
        <span class="budget-renewal-text">{{ chat.roundsNotice }}</span>
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
   不随滚动条出现/消失漂移，内容列也不再横向抖 6px。
   底 padding 16px：贴底静息的呼吸位（旧值 32px 是为「常驻渐隐让尾部落带中段」
   设计的，渐隐改随 autoFollow 联动后贴底无蒙层，理由死掉回归紧凑值） */
.messages-area { flex:1; overflow-y:auto; scrollbar-gutter:stable; padding:var(--ip-spacing-6) 0 var(--ip-spacing-4); position:relative; }
.messages-container { display:flex; flex-direction:column; gap: var(--ip-spacing-4); padding:0 var(--msg-col-right) 0 48px; }

/* ===== 底缘渐隐（2026-09-01 联动改版：随贴底状态显隐 + 96px）=====
   非贴底（autoFollow=false，读历史/中途滚离）时内容贴近输入框逐渐变淡：
   96px 带内从透明到页面底色，ease 型三停（0% → 32%@48% → 74%@78% → 纯底色）
   ——起步缓收尾快；终点纯底色与输入区（透明底同 --ip-color-bg-secondary）无缝。
   - 贴底（fade-off）时整层 opacity:0：正在生成的消息尾部不再压在半透明带下
     （贴底静息/短会话不满屏也不挂空渐变带）。autoFollow 判定距底 ≤120px，
     比「绝对贴底」更稳——图片加载/流式增高的 1-2px 抖动不会引发闪烁。
   - 显隐走 class 切换 + opacity 过渡：阈值跨越是离散事件非每帧，opacity 只
     重绘本蒙层（合成器友好）；勿改成 scroll 事件按距底连续写 style（每帧
     reactive 写入，generation-lag 教训）。reduced-motion 由 global.css 全局
     归零覆盖 *::after。
   - 渐变蒙层而非 mask-image：全屏图片预览/附件详情渲染在 .messages-area
     内（fixed 定位），mask 会连它们的底缘一起淡掉；盖底色渐变无此问题。
   - color-mix 产半透明底色（WebView2 = Chromium 111+ 支持）。
   - z-index 走 base 档（0）= 盖过 z-auto 滚动内容（::after 是 wrap 末位伪元素，
     与定位内容同在 0/auto 档时按树序仍压过滚动区）；低于 TurnRail(3)/
     兜底按钮(raised)/toast/模态层，带内悬浮件保持清晰。pointer-events:none 不挡点击。 */
.messages-wrap::after {
  content: '';
  position: absolute;
  left: 0; right: 0; bottom: 0;
  height: 96px;
  background: linear-gradient(to bottom,
    transparent,
    color-mix(in srgb, var(--ip-color-bg-secondary) 32%, transparent) 48%,
    color-mix(in srgb, var(--ip-color-bg-secondary) 74%, transparent) 78%,
    var(--ip-color-bg-secondary));
  pointer-events: none;
  z-index: var(--ip-z-base);
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out);
}
.messages-wrap.fade-off::after { opacity: 0; }

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
  border-radius:12px; border-bottom-left-radius:4px; padding:var(--ip-spacing-3) var(--ip-spacing-4); /* 垂直 14→12：间距令牌无 14 档，就近收编 */
}
.message-group.user { align-self:flex-end; max-width:70%; }

/* ===== 频道群聊布局（2026-09-11 拍板：参考常见聊天软件群聊气泡）=====
   1v1 态：wrap/body 双层透明直通（flex column + gap 等价旧结构，视觉零变化）。
   频道态：wrap 转 grid 两列——头像独立左列（36px，跨两行）+ 身体列（昵称行 +
   气泡体）；气泡背景/圆角/padding 从外层组容器下放给 .assistant-body。
   ⚠️ 身体列孩子一律显式 grid-column:2 定位——匿名组（旧消息未 enrich、无头像无
   头）只有 .assistant-body 一个孩子，靠自动放置会塌进 36px 头像列。 */
.assistant-wrap { display:flex; flex-direction:column; gap:2px; min-width:0; }
.assistant-wrap.channel {
  display:grid;
  grid-template-columns:36px minmax(0, 1fr);
  column-gap:var(--ip-spacing-3, 12px);
  row-gap:2px;
}
.channel-avatar { grid-column:1; grid-row:1 / span 2; align-self:start; flex-shrink:0; }
.assistant-wrap.channel .channel-sender-head { grid-column:2; grid-row:1; }
.assistant-wrap.channel .assistant-body { grid-column:2; grid-row:2; display:flex; flex-direction:column; gap:2px; min-width:0; }
/* 频道态外层组容器退位：背景/圆角/padding 全部让给 .assistant-body（群聊行 = 头像列 + 气泡体） */
.message-group.assistant.channel-msg {
  background-color:transparent; color:inherit;
  border-radius:0; padding:0;
}
.message-group.assistant.channel-msg .assistant-body {
  background-color:var(--ip-color-bg-message-ai); color:var(--ip-color-text-message-ai);
  border-radius:12px; border-bottom-left-radius:4px;
  padding:var(--ip-spacing-3) var(--ip-spacing-4);
}
.message-content { display:flex; flex-direction:column; gap:4px; min-width:0; }
.message-group.user .message-content { align-items:flex-end; }

/* 组内多条 assistant item：item 内子项适度间距，轮次之间留白区分（呼吸感；
   2026-09-05 排版批 16→12——组内轮次是同一回答的连续段落，12px 足够分节，
   组间 gap 16px 保持承担「不同回答」的更大分隔） */
.message-group.assistant .message-item { display:flex; flex-direction:column; gap:6px; }
.message-group.assistant .message-item + .message-item { margin-top: var(--ip-spacing-3, 12px); }

/* ===== 用户消息气泡 ===== */
/* flex column + 统一 gap：正文 / 引用卡 / 附件卡（含堆叠）/ 图片（含堆叠）纵向
   排布的唯一间距来源——旧实现靠各元素零散 margin-top（6/4/0px 不一，单图单卡
   贴正文），多元素混排时参差；卡片的独立 margin 已移除，调间距只改 gap。 */
.message-group.user .message-bubble { display:flex; flex-direction:column; gap: var(--ip-spacing-2); padding:10px 16px; border-radius:12px; font-size:var(--ip-text-body-size); line-height:var(--ip-line-height-loose3, 1.6); white-space:pre-wrap; word-break:break-word; background-color:var(--ip-color-bg-user-bubble); color:var(--ip-color-text-on-user-bubble); border-bottom-right-radius:4px; }

/* ===== MA-3 跨会话来件来源标注头（气泡内首行；气泡底色深→文字用 on-bubble 色）===== */
.user-incoming-head { display:flex; align-items:center; gap:6px; min-width:0; padding-bottom:6px; border-bottom:1px solid rgba(255,255,255,0.18); }
.user-incoming-head.clickable { cursor:pointer; }
.incoming-icon { flex-shrink:0; color:var(--ip-color-text-on-user-bubble); opacity:0.8; display:inline-block; }
.incoming-src { flex:1; min-width:0; overflow:hidden; white-space:nowrap; text-overflow:ellipsis; font-size:var(--ip-text-caption-size); font-weight:var(--ip-font-weight-medium); }
.incoming-pill { flex-shrink:0; font-size:var(--ip-text-micro-size); line-height:1; padding:3px 8px; border-radius:var(--ip-radius-full, 999px); background:rgba(255,255,255,0.16); }

/* ===== 助手消息文字（无自带背景，由组容器承载气泡块）=====
   行高走 loose3（1.6）与用户气泡/ markdown-body 同值——2026-09-15 排版批三轮
   （令牌改值 1.5→1.6），勿回退字面量（同屏 MD 与纯文本行距不齐的根源）。 */
.message-group.assistant .message-bubble { padding:0; border-radius:0; font-size:var(--ip-text-body-size); line-height:var(--ip-line-height-loose3, 1.6); white-space:pre-wrap; word-break:break-word; background:transparent; }

/* ===== 用户消息内容（含图片） ===== */
.user-content { display:flex; flex-direction:column; gap:4px; }
.user-text { display:block; white-space:pre-wrap; }
.user-images { display:flex; flex-wrap:wrap; gap:4px; margin-top:2px; }
.user-image { max-width:200px; max-height:200px; border-radius:var(--ip-radius-lg); object-fit:cover; border:1px solid var(--ip-color-border-default); }

/* 用户附件卡片（office/pdf）—— 不透明白实体卡片：深绿气泡上的清晰层次，
   堆叠时不透明避免半透明叠加发灰/透字（半透明玻璃在重叠场景不可扩展）。
   表面/文字走语义令牌（暗色主题修复）：卡底选 secondary 而非 elevated——
   hover 档 tertiary 在暗色下恰等于 elevated（同为 gray-800）会吞掉 hover 反馈，
   secondary(850)→tertiary(800) 明暗两主题都有可见变化；明色下两档同白，观感不变 */
.user-attachments { display:flex; flex-direction:column; gap:4px; margin-top:6px; }
.user-attachment-card {
  display:flex; align-items:center; gap: var(--ip-spacing-2);
  padding:6px 10px; border-radius:8px;
  background:var(--ip-color-bg-secondary);
  border:1px solid rgba(0,0,0,0.08);
  box-shadow:0 1px 2px rgba(0,0,0,0.06);
  max-width:260px;
  color:var(--ip-color-text-primary);
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
.att-name { font-size:var(--ip-text-body-sm-size); font-weight:500; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.att-meta { font-size: var(--ip-text-micro-size); color:var(--ip-color-text-tertiary); }

/* 单个卡片/图片可点（hover 提示） */
.user-attachment-card.clickable { cursor:pointer; transition:background var(--ip-duration-fast) var(--ip-ease-out); }
.user-attachment-card.clickable:hover { background:var(--ip-color-bg-tertiary); }
.user-image.clickable { cursor:zoom-in; transition:transform var(--ip-duration-fast) var(--ip-ease-out); }
.user-image.clickable:hover { transform:scale(1.02); }

/* @ 引用卡片（用户气泡内）：轻量内联卡——图标按 ref_kind 着色，agent 无跳转。
   表面/文字同附件卡走语义令牌（secondary 卡底 + tertiary hover，暗色主题修复）；
   字号 12.5→caption(12px) 就近收编档位 */
.user-ref-card {
  display:flex; align-items:center; gap:6px;
  max-width:260px; padding:5px 10px; border-radius:8px;
  background:var(--ip-color-bg-secondary); border:1px solid rgba(0,0,0,0.08);
  box-shadow:0 1px 2px rgba(0,0,0,0.06);
  cursor:pointer; transition:background var(--ip-duration-fast) var(--ip-ease-out);
  color:var(--ip-color-text-primary); font-size:var(--ip-text-caption-size);
}
.user-ref-card:hover { background:var(--ip-color-bg-tertiary); }
.user-ref-card .ref-icon { flex:none; color:var(--ip-primary-600, var(--ip-primary-600)); }
.user-ref-card[data-ref-kind="agent"] .ref-icon { color:var(--ip-accent-agent); }
.user-ref-card[data-ref-kind="message"] .ref-icon { color:var(--ip-color-icon-muted); }
.user-ref-card[data-ref-kind="agent"] { cursor:default; }
.user-ref-card .ref-label { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }

/* 多文档重叠堆叠：子卡片 position:relative 才能让 zIndex 生效；负 margin 由内联 style 给；
   堆叠态加重投影，让"一摞卡片"的层次可见 */
.doc-stack { position:relative; max-width:260px; cursor:pointer; }
.doc-stack .user-attachment-card { position:relative; box-shadow:0 3px 10px rgba(0,0,0,0.18); }
/* 堆叠卡 hover 与单卡统一 tertiary 档（原 #fafafa/#f3f4f6 的两档微差收编；
   暗色下原浅灰裸值会变成亮块） */
.doc-stack .user-attachment-card:hover { background:var(--ip-color-bg-tertiary); }

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

/* ===== 频道 v1：成员身份头 + 生成中发出标注 ===== */
/* 群聊行形态（2026-09-11）：昵称行在气泡外（grid 身体列首行）；名字是身份锚点，
   caption-12 → body-sm-13 醒目化；margin 归零（grid row-gap 接管行距） */
.channel-sender-head { display:flex; align-items:center; gap:6px; }
.channel-sender-name { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium); color:var(--ip-color-text-secondary); }
/* Shield 进文本流：显式 inline-block（base.css svg display:block reset 陷阱） */
.channel-sender-shield { display:inline-block; color:var(--ip-primary-600); }
/* 触发来源吸入标注（⑫ 图标化）：昵称行尾一枚 12px 图标（AtSign/CornerUpRight，
   tertiary 灰弱于昵称本体；显式 inline-block 防 base.css svg 块级化）——文字
   形态「@来源名」易被误读成气泡主人 @ 了谁，改由 hover title 承载发起者 */
.channel-mention-src { display:inline-flex; align-items:center; margin-left:2px; color:var(--ip-color-text-tertiary); }
.channel-mention-src svg { display:inline-block; flex-shrink:0; }
/* 「生成中发出」事实标注（micro 主色调——是频道语境的插话事实，非错误态） */
.gen-time-flag { font-size: var(--ip-text-micro-size); color:var(--ip-color-primary-tint-text); }
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
.cursor-label { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); }
/* 生成指示的脉冲点已由 StatusGlyph running 像素格取代（2026-09-04 语系统一）；
   .state-dot 死样式（模板零引用）与 cursor-pulse 一并清除 */

/* ===== 状态 ===== */
.state-hint { height:100%; display:flex; align-items:center; justify-content:center; gap: var(--ip-spacing-2); color:var(--ip-color-text-tertiary); font-size:var(--ip-text-body-sm-size); }

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
.scroll-bottom-btn { position:absolute; top:calc(50% - 18px); right:calc(6px + (var(--msg-col-right) - 36px) / 2); z-index:var(--ip-z-raised); width:36px; height:36px; border-radius:var(--ip-radius-lg); border:1px solid var(--ip-color-border-default); background-color:var(--ip-color-bg-elevated); color:var(--ip-color-text-secondary); box-shadow:var(--ip-shadow-sm); cursor:pointer; display:flex; align-items:center; justify-content:center; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.scroll-bottom-btn:hover { background-color:var(--ip-color-bg-secondary); color:var(--ip-color-text-primary); border-color:var(--ip-color-border-strong); box-shadow:var(--ip-shadow-md); }

.fade-up-enter-active { animation:fade-up-in 0.2s ease-out; }
.fade-up-leave-active { animation:fade-up-in 0.15s ease-in reverse; }
@keyframes fade-up-in { from { opacity:0; transform:translateY(8px); } to { opacity:1; transform:translateY(0); } }

/* ===== 思考过程（无边框无背景，左绿线标识） ===== */
.think-block { margin:0; }
.think-toggle { display:flex; align-items:center; gap:6px; padding:2px 6px; cursor:pointer; user-select:none; border-radius:var(--ip-radius-sm); transition:all var(--ip-duration-fast) var(--ip-ease-out); width:100%; }
.think-toggle:hover { background:var(--ip-color-bg-tertiary); }
/* chevron 右移行尾（2026-09-04 拍板：状态 glyph 前置行首，展开操作在行尾） */
.think-chevron { margin-left:auto; font-size: var(--ip-text-micro-size); color:var(--ip-color-text-disabled); line-height:1; width:10px; flex-shrink:0; transition:transform var(--ip-duration-fast) var(--ip-ease-out); }
/* 勿加 text-transform:uppercase——label 是中文不受影响，但会把后缀的耗时单位
   （m/s）打成大写（2026-09-09 生产反馈：思考 · 1M 30S） */
.think-label { font-size:var(--ip-text-caption-size); font-weight:var(--ip-font-weight-medium); color:var(--ip-color-text-tertiary); letter-spacing:0.3px; }
.think-status { margin-left:8px; font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); }
.think-body { margin:4px 0 4px 22px; padding:6px 0 6px 14px; border-left:2px solid var(--ip-primary-200); font-size:var(--ip-text-body-sm-size); color:var(--ip-color-text-secondary); line-height:1.6; white-space:pre-wrap; word-break:break-word; }
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
/* chevron 行尾（glyph 前置后；preview 的 margin-left:auto 已把尾部让出） */
.tool-chevron { font-size: var(--ip-text-micro-size); color:var(--ip-color-text-disabled); line-height:1; width:10px; flex-shrink:0; }
.tool-name { font-size:var(--ip-text-caption-size); font-weight:var(--ip-font-weight-medium); color:var(--ip-color-text-tertiary); white-space:nowrap; }
/* 行级 diff 徽记（git 式 +N -M ~K，2026-09-09）：语义三色——success/danger/warning；
   mono micro；行内 flex 容器收图标字体基线，不参与两侧截断 */
.tool-diff { display:inline-flex; align-items:center; gap:4px; flex-shrink:0; font-family:var(--ip-font-mono, monospace); font-size:var(--ip-text-micro-size); line-height:1; white-space:nowrap; }
.tool-diff .diff-add { color:var(--ip-success-text); }
.tool-diff .diff-del { color:var(--ip-danger-text); }
.tool-diff .diff-mod { color:var(--ip-warning-text); }
.tool-duration { font-size: var(--ip-text-micro-size); color:var(--ip-color-text-disabled); font-family:var(--ip-font-mono, monospace); white-space:nowrap; flex-shrink:0; }
/* 次级信息（左置，紧跟展示名）与文件名位（右锚可点 reveal，2026-09-07 布局拍板） */
.tool-secondary { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); white-space:nowrap; }
.tool-file { margin-left:auto; margin-right:6px; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; text-align:right; flex-shrink:1; font-size:var(--ip-text-caption-size); color:var(--ip-color-text-secondary); cursor:pointer; }
.tool-file:hover { color:var(--ip-primary-600); text-decoration:underline; }
.tool-preview { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); margin-left:auto; margin-right:6px; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; text-align:right; flex-shrink:1; }
/* ③ 组级折叠摘要行的失败计数（warning 语义色；与 tool-diff 的加减速记同为行内强调位） */
.tool-fail-count { font-size:var(--ip-text-caption-size); color:var(--ip-warning-text); white-space:nowrap; }

/* 组级收纳胶囊行（2026-09-16 五轮：三行合一置气泡最前端）——思考/工具/过程
   三胶囊并排；flex-wrap 窄窗换行不截断；胶囊覆写行级 width:100% → 内容自适应宽。
   与正文的间距对齐段落节奏（七轮拍板「留顶部强化呈现」）：下距 = spacing-3
   （12px）恰为 .markdown-body p 的段底距——胶囊行是一段落级区块，不再贴着
   正文；行内 gap 同步升至 spacing-2 让三枚 chip 各自成形 */
.group-summary-pills { display:flex; flex-wrap:wrap; align-items:center; gap:var(--ip-spacing-2); margin-bottom:var(--ip-spacing-3); }
/* 胶囊 chrome（2026-09-16 六轮：用户反馈「不够明显、区别度不够」）——对齐房内
   chip 语言 ChatHeader .header-kind-badge（软底胶囊 = 可点开藏内容的供能暗示）；
   两态对比承载状态自述：收起 = 实底软色胶囊（这里有内容，点开看）/ 展开 =
   幽灵描边胶囊（is-open，内容已在场、控件退位）；hover 各自加深；边框两态恒
   1px 防开合尺寸跳动。色值照抄 header-kind-badge 的 var+rgba 兜底写法
   （--ip-primary-soft-border 全局无定义，rgba 兜底即频道 tag 徽章的实际渲染
   形态）。全部规则收在 .group-summary-pills 前缀下与基类（.tool-toggle/
   .think-toggle 行形态）无特异性争抢；按钮化后补 font/line-height 继承
   （UA 按钮字体不随父走）。 */
.group-summary-pills .summary-pill {
  width:auto; flex-shrink:0;
  padding:2px 10px; gap:4px;
  border-radius:var(--ip-radius-full, 999px);
  border:1px solid rgba(var(--ip-primary-500-rgb), 0.25);
  background:var(--ip-color-primary-soft-bg, rgba(var(--ip-primary-500-rgb), 0.08));
  color:var(--ip-primary-600);
  font-family:inherit; font-size:inherit; line-height:inherit;
  transition:background var(--ip-duration-fast) var(--ip-ease-out),
             border-color var(--ip-duration-fast) var(--ip-ease-out),
             color var(--ip-duration-fast) var(--ip-ease-out);
}
.group-summary-pills .summary-pill:hover { background:rgba(var(--ip-primary-500-rgb), 0.14); }
/* 展开态：幽灵胶囊（透明底 + 默认描边 + tertiary 文本） */
.group-summary-pills .summary-pill.is-open { background:transparent; border-color:var(--ip-color-border-default); color:var(--ip-color-text-tertiary); }
.group-summary-pills .summary-pill.is-open:hover { background:var(--ip-color-bg-tertiary); }
/* 胶囊内子元素随态取色：图标/标签/chevron 一律 inherit 胶囊色（收起=主色、
   展开=tertiary），chevron 与总耗时再压一档透明度作次级信息。
   字号/字重上调（七轮拍板「强化呈现」）：胶囊是折叠内容的唯一入口，标签升
   body-sm-13 + semibold——比展开后的行级 tool-name（caption-12）大一档半档
   字重，读作区块控件而非行内元数据；仅胶囊域内覆写，展开区各行照旧 */
.group-summary-pills .summary-pill .pill-glyph { color:inherit; }
.group-summary-pills .summary-pill .tool-name,
.group-summary-pills .summary-pill .think-label {
  color:inherit;
  font-size:var(--ip-text-body-sm-size);
  font-weight:var(--ip-font-weight-semibold);
}
/* 失败计数只升字号不碰颜色（warning 语义色刻意保留，见 .tool-fail-count 先例） */
.group-summary-pills .summary-pill .tool-fail-count { font-size:var(--ip-text-body-sm-size); }
.group-summary-pills .summary-pill .tool-chevron,
.group-summary-pills .summary-pill .think-chevron { color:inherit; opacity:0.65; font-size:var(--ip-text-caption-size); }
.group-summary-pills .summary-pill .think-group-total { color:inherit; opacity:0.75; }
/* 胶囊语义图标（Brain 思考 / Wrench 工具 / MessageSquareText 过程叙述）：
   图标表内容域非状态——状态图标 done/error 与收纳语义不符（2026-09-16 用户
   拍板换语义图标）；色随胶囊态（上方 inherit 规则） */
.pill-glyph { flex-shrink:0; }
/* 思考聚合展开堆叠区（胶囊行正下方；左缩进 22px 与 think-body 同意象）；
   总耗时是次级信息（常规字重；胶囊内色由上方 inherit+opacity 承载） */
.think-group-total { font-weight:var(--ip-font-weight-regular); }
.think-group-stack { margin:2px 0 6px 22px; display:grid; gap:2px; }
/* 截断组标注（三轮换轴）：回合被截断是警示事实——warning 语义色（同
   .tool-fail-count 先例），碎句尾段由它语境化、不再冒充结论。
   ⚠️ scoped 版必须排在上方 .tool-name 的 inherit 覆写之后——两规则同为
   (0,3,0)，同元素双类（tool-name+process-truncated）命中时后者胜 */
.group-summary-pills .summary-pill .process-truncated { color:var(--ip-warning-text); }
.process-truncated { color:var(--ip-warning-text); }

/* 状态图标（StatusGlyph：环形对勾/3×3 像素格/环形叉，2026-09-04 语系统一）。
   行内紧凑节奏保持：glyph 14px 与 caption 字号同高，flex 自然居中。 */

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
/* 定位/排版微调层：底色与边框走 danger 语义令牌（明暗自适应，与 ErrorBanner
   自身 .eb-banner 同值——此前裸 hex 覆盖组件令牌，暗色主题下整条横幅发亮）；
   文字色由 .eb-banner 的 --ip-danger-text 提供，此处不重复声明 */
.chat-error-banner { display:flex; align-items:flex-start; gap: var(--ip-spacing-2); margin:8px 16px; padding:10px 14px; background:var(--ip-danger-bg); border:1px solid var(--ip-danger-border); border-radius:var(--ip-radius-md); font-size:var(--ip-text-body-sm-size); }
</style>
