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
import { watch, nextTick, ref, computed, onMounted, onUnmounted, onActivated, onDeactivated } from "vue";
import { useChatStore } from "../../stores/chat";
import { formatTime, formatDateLabel } from "../../utils/time";
import MarkdownRenderer from "./MarkdownRenderer.vue";
import ConfigProposalCard from "./ConfigProposalCard.vue";
import type { Message, MessageRole } from "../../types";

const chat = useChatStore();
const listRef = ref<HTMLElement | null>(null);
const showScrollBtn = ref(false);
/** 是否「跟随底部」自动滚动。用户向上滚离底部时置 false（边生成边看不被打扰），
 *  滚回底部或点「回到底部」时恢复 true。距底部 <= 阈值视为在跟随。*/
const autoFollow = ref(true);
const FOLLOW_THRESHOLD = 120;
let suppressScrollCheck = false;
let paginating = false;
let scrollPosCache = { scrollHeight: 0, scrollTop: 0 };

// 工具调用卡片展开状态
const expandedToolCalls = ref<Set<string>>(new Set());
// 思考过程展开状态（按消息 ID）
const expandedThinking = ref<Set<string>>(new Set());
// 思考计时
const thinkingNow = ref(Date.now());
let thinkingTimer: ReturnType<typeof setInterval> | null = null;

watch(() => chat.streamingThinking, (val) => {
  if (val && !thinkingTimer) {
    thinkingTimer = setInterval(() => { thinkingNow.value = Date.now(); }, 200);
  } else if (!val && thinkingTimer) {
    clearInterval(thinkingTimer);
    thinkingTimer = null;
  }
});

const thinkingElapsed = computed(() => {
  const start = chat.thinkingStartTime;
  if (!start) return '';
  const elapsed = Math.floor((thinkingNow.value - start) / 1000);
  if (elapsed < 60) return `${elapsed}s`;
  const m = Math.floor(elapsed / 60);
  const s = elapsed % 60;
  return `${m}m ${s}s`;
});

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

/** 判断一个 assistant 消息是否有非 text 的附属内容（tool/thinking） */
function hasExtras(msg: { content_blocks?: string }): boolean {
  if (!msg.content_blocks || msg.content_blocks === '[]') return false;
  try {
    const blocks = JSON.parse(msg.content_blocks);
    return Array.isArray(blocks) && blocks.some((b: Record<string, unknown>) => b.type === 'tool_use' || b.type === 'thinking');
  } catch { return false; }
}

// 检测滚动位置：非底部显示按钮，靠近顶部触发分页
function onScroll() {
  if (suppressScrollCheck || paginating) return;
  const el = listRef.value;
  if (!el) return;

  const distToBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
  showScrollBtn.value = distToBottom > 80;
  // 用户向上滚离底部 → 停止跟随（边生成边看不被滚动条打扰）；滚回底部 → 恢复跟随
  autoFollow.value = distToBottom <= FOLLOW_THRESHOLD;

  // 分页触发：距顶部 200px 且还有更多数据
  if (el.scrollTop < 200 && chat.hasMore && !chat.loadingMore && !chat.sending) {
    paginating = true;
    scrollPosCache.scrollHeight = el.scrollHeight;
    scrollPosCache.scrollTop = el.scrollTop;
    chat.loadMoreMessages().then(() => {
      nextTick(() => {
        const newEl = listRef.value;
        if (newEl) {
          const added = newEl.scrollHeight - scrollPosCache.scrollHeight;
          newEl.scrollTop = scrollPosCache.scrollTop + added;
        }
        paginating = false;
      });
    });
  }
}

function scrollToBottom(smooth?: boolean) {
  if (listRef.value) {
    suppressScrollCheck = true;
    autoFollow.value = true; // 手动滚到底部 = 恢复跟随
    showScrollBtn.value = false;
    listRef.value.scrollTo({ top: listRef.value.scrollHeight, behavior: smooth !== false ? "smooth" : "instant" });
    setTimeout(() => { suppressScrollCheck = false; }, smooth !== false ? 500 : 50);
  }
}

onMounted(() => {
  listRef.value?.addEventListener("scroll", onScroll);
  scrollToBottom(false);
});
onActivated(() => {
  // KeepAlive: 切回时恢复滚动位置。如果在生成中或刚切回来，滚到底部。
  // 若停用期间 thinking 仍在进行，重启计时器以还原实时耗时显示。
  if (chat.streamingThinking && !thinkingTimer) {
    thinkingNow.value = Date.now();
    thinkingTimer = setInterval(() => { thinkingNow.value = Date.now(); }, 200);
  }
  nextTick(() => scrollToBottom(false));
});
onDeactivated(() => {
  // KeepAlive: 切走时停止计时器，避免不可见区间浪费 CPU
  if (thinkingTimer) { clearInterval(thinkingTimer); thinkingTimer = null; }
});
onUnmounted(() => {
  listRef.value?.removeEventListener("scroll", onScroll);
  if (thinkingTimer) { clearInterval(thinkingTimer); thinkingTimer = null; }
});

// 切换会话后等消息加载完成再平滑滚动到底部
watch(() => chat.msgLoading, async (loading) => {
  if (!loading && chat.messages.length > 0) {
    await nextTick();
    scrollToBottom(true);
  }
});

// 自动滚到底部（分页加载时不触发；用户向上看内容时不抢滚动条）
watch(
  [() => chat.messages.length, () => chat.streamingText],
  async () => {
    if (paginating || !autoFollow.value) return;
    await nextTick();
    const el = listRef.value;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  },
);

const copiedId = ref<string | null>(null);

function copyContent(content: string, id?: string) {
  navigator.clipboard.writeText(content);
  if (id) {
    copiedId.value = id;
    setTimeout(() => { copiedId.value = null; }, 2000);
  }
}

function parseImageBlocks(contentBlocks: string): { data: string; mediaType: string }[] {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { data: string; media_type: string; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'image'
    ).map((b) => ({ data: b.data, mediaType: b.media_type }));
  } catch { return []; }
}

function parseToolUseBlocks(contentBlocks: string): { id: string; name: string; input: string }[] {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { id: string; name: string; input: string; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'tool_use'
    ).map((b) => ({ id: b.id, name: b.name, input: b.input }));
  } catch { return []; }
}

function parseToolResultBlocks(contentBlocks: string): { toolUseId: string; content: string; isError: boolean }[] {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { tool_use_id: string; content: string; is_error?: boolean; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'tool_result'
    ).map((b) => ({ toolUseId: b.tool_use_id, content: b.content, isError: b.is_error ?? false }));
  } catch { return []; }
}

function parseThinkingBlocks(contentBlocks: string): string[] {
  try {
    const blocks: unknown[] = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b): b is { thinking: string; type: string } =>
      typeof b === 'object' && b !== null && (b as Record<string, unknown>).type === 'thinking'
    ).map((b) => b.thinking);
  } catch { return []; }
}

/** 从 idx+1 起向后查相邻 user 消息里 tool_use_id 对应的 tool_result。
 *  彻底重构后 tool_result 独立存于相邻 user 消息（不再与 tool_use 同条）。*/
function findToolResult(
  toolUseId: string,
  msgIdx: number,
): { content: string; isError: boolean } | null {
  for (let i = msgIdx + 1; i < chat.messages.length; i++) {
    const m = chat.messages[i];
    if (m.role === "assistant") break; // tool_result 必紧跟其 tool_use 的 assistant
    if (m.role === "user") {
      const found = parseToolResultBlocks(m.content_blocks).find(
        (r) => r.toolUseId === toolUseId,
      );
      if (found) return { content: found.content, isError: found.isError };
    }
  }
  return null;
}

/** 查询某个 tool_use_id 对应的 tool_result 是否有 isError（跨消息配对）*/
function getToolHasError(toolUseId: string, msgIdx: number): boolean {
  return findToolResult(toolUseId, msgIdx)?.isError ?? false;
}

/** 判断 user 消息是否仅含 tool_result（无文本/图片）。
 *  这种消息是工具调用结果，不单独成气泡，其内容并入上一条 assistant 的工具卡片。*/
function isToolResultOnlyUser(msg: { role: string; content: string; content_blocks: string }): boolean {
  if (msg.role !== "user" || msg.content) return false;
  try {
    const blocks = JSON.parse(msg.content_blocks);
    if (!Array.isArray(blocks) || blocks.length === 0) return false;
    return blocks.every((b: Record<string, unknown>) => b.type === "tool_result");
  } catch {
    return false;
  }
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
  return g.items.some((it) => it.msg.content || hasExtras(it.msg));
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
const finishReasonLabels: Record<string, string> = {
  length: "已达长度上限，回答被截断",
  abort: "已手动停止",
  budget_exceeded: "Token 预算超限，回答被截断",
  stuck: "连续多轮无进展，已自动终止",
  tool_use: "已达最大工具调用轮数，回答可能不完整",
};
</script>

<template>
  <div ref="listRef" class="messages-area">
    <!-- 错误提示 -->
    <div v-if="chat.lastError" class="chat-error-banner">
      <span class="chat-error-icon">!</span>
      <span class="chat-error-text">{{ chat.lastError }}</span>
    </div>
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
        <div :class="['message-group', group.role]">
          <!-- ===== 用户消息组（单条，透明壳）===== -->
          <template v-if="group.role === 'user'">
            <div class="message-content user">
              <div class="message-bubble">
                <span v-if="group.items[0].msg.content" class="user-text">{{ group.items[0].msg.content }}</span>
                <div v-if="group.items[0].msg.content_blocks && group.items[0].msg.content_blocks !== '[]'" class="user-images">
                  <img v-for="(img, i) in parseImageBlocks(group.items[0].msg.content_blocks)" :key="i" :src="`data:${img.mediaType};base64,${img.data}`" class="user-image" loading="lazy" />
                </div>
              </div>
              <div v-if="group.items[0].msg.content" class="message-footer">
                <div class="footer-left">
                  <span class="message-time">{{ formatTime(group.items[0].msg.created_at) }}</span>
                </div>
                <div class="footer-actions">
                  <button class="copy-btn" :title="copiedId === group.items[0].msg.id ? '已复制' : '复制'" @click="copyContent(group.items[0].msg.content, group.items[0].msg.id)">
                    <svg v-if="copiedId !== group.items[0].msg.id" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                      <rect x="9" y="9" width="13" height="13" rx="2" ry="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                    </svg>
                    <svg v-else width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--ip-success-base, #16a34a)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
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
                        <MarkdownRenderer :content="chat.streamingThinking" />
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
                  <MarkdownRenderer :content="item.msg.content" />
                </div>

                <!-- 工具调用（历史/刚结束，从 content_blocks 解析，非流式） -->
                <div v-if="parseToolUseBlocks(item.msg.content_blocks).length > 0 && !(isLiveAssistant(item) && toolCallList.length > 0)" class="tools-strip">
                  <div v-for="tu in parseToolUseBlocks(item.msg.content_blocks)" :key="tu.id">
                    <div class="tool-toggle" @click="toggleToolCall(tu.id)">
                      <span class="tool-chevron">{{ expandedToolCalls.has(tu.id) ? '▾' : '▸' }}</span>
                      <span class="tool-name">{{ tu.name }}</span>
                      <span class="tool-preview">{{ truncateJson(tu.input) }}</span>
                      <span :class="['tool-dot', getToolHasError(tu.id, item.idx) ? 'tool-dot-err' : 'tool-dot-ok']"></span>
                    </div>
                    <Transition name="tool-slide">
                      <div v-if="expandedToolCalls.has(tu.id)" class="tool-expand">
                        <div class="tool-expand-group">
                          <div class="tool-expand-hdr">参数</div>
                          <pre class="tool-expand-code">{{ formatJson(tu.input) }}</pre>
                        </div>
                        <template v-for="tr in [findToolResult(tu.id, item.idx)]" :key="tr ? 'has-result' : 'no-result'">
                          <div v-if="tr" class="tool-expand-group">
                            <div :class="['tool-expand-hdr', tr.isError ? 'hdr-err' : '']">{{ tr.isError ? '错误' : '结果' }}</div>
                            <pre :class="['tool-expand-code', tr.isError ? 'code-err' : '']">{{ tr.content }}</pre>
                          </div>
                        </template>
                      </div>
                    </Transition>
                  </div>
                </div>

                <!-- 工具调用（当前流式） -->
                <div v-if="isLiveAssistant(item) && toolCallList.length > 0" class="tools-strip">
                  <div v-for="call in toolCallList" :key="call.id">
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
                <button v-if="groupText(group)" class="copy-btn" :title="copiedId === 'grp-' + group.firstIdx ? '已复制' : '复制'" @click="copyContent(groupText(group), 'grp-' + group.firstIdx)">
                  <svg v-if="copiedId !== 'grp-' + group.firstIdx" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="9" y="9" width="13" height="13" rx="2" ry="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                  </svg>
                  <svg v-else width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="var(--ip-success-base, #16a34a)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
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

    <!-- finish_reason 提示 -->
    <div v-if="chat.lastFinishReason && chat.lastFinishReason !== 'stop' && chat.lastFinishReason !== 'end_turn' && chat.messages.length > 0" class="finish-reason">
      <span>{{ finishReasonLabels[chat.lastFinishReason] || chat.lastFinishReason }}</span>
    </div>

    <div v-if="chat.sending && chat.messages.length > 0" class="cursor-bar">
      <div class="cursor-track">
        <div class="cursor-glow" /><span class="cursor-label">正在生成…</span>
      </div>
    </div>

    <Transition name="fade-up">
      <button v-if="showScrollBtn" class="scroll-bottom-btn" title="回到底部并跟随最新" @click="scrollToBottom()">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="12" y1="5" x2="12" y2="19" /><polyline points="19 12 12 19 5 12" />
        </svg>
      </button>
    </Transition>
  </div>
</template>

<style scoped>
.messages-area { flex:1; overflow-y:auto; padding:24px 0; position:relative; }
.messages-container { display:flex; flex-direction:column; gap:16px; padding:0 48px; }

/* ===== 分页指示 ===== */
.load-more-hint { text-align:center; font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); padding:8px 48px; }
.load-more-end { color:var(--ip-color-text-disabled); }

/* ===== 日期分组 ===== */
.date-divider { display:flex; align-items:center; gap:12px; padding:20px 48px 8px; font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); }
.date-divider::before, .date-divider::after { content:''; flex:1; height:1px; background:var(--ip-color-border-default); }

/* ===== finish_reason 提示 ===== */
.finish-reason { text-align:center; padding:4px 48px 0; }
.finish-reason span { display:inline-block; font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); padding:2px 10px; border-radius:var(--ip-radius-full); background:var(--ip-color-bg-tertiary); }

/* ===== TransitionGroup 动画 ===== */
.msg-enter-active { animation:msg-in 0.35s cubic-bezier(0.16,1,0.3,1); }
.msg-leave-active { display:none; }
.msg-move { transition:transform 0.3s ease; }
@keyframes msg-in { from { opacity:0; transform:translateY(12px) scale(0.97); } to { opacity:1; transform:translateY(0) scale(1); } }

/* ===== 消息组（连续同 agent 的 assistant 合并成一个气泡块）===== */
.message-group { display:flex; flex-direction:column; gap:2px; min-width:0; }
.message-group.assistant {
  align-self:flex-start; max-width:85%;
  background-color:var(--color-message-ai-bg); color:var(--color-message-ai-text);
  border-radius:12px; border-bottom-left-radius:4px; padding:14px 16px;
}
.message-group.user { align-self:flex-end; max-width:70%; }
.message-content { display:flex; flex-direction:column; gap:4px; min-width:0; }
.message-group.user .message-content { align-items:flex-end; }

/* 组内多条 assistant item：item 内子项适度间距，轮次之间留白区分（呼吸感）*/
.message-group.assistant .message-item { display:flex; flex-direction:column; gap:6px; }
.message-group.assistant .message-item + .message-item { margin-top:16px; }

/* ===== 用户消息气泡 ===== */
.message-group.user .message-bubble { padding:10px 16px; border-radius:12px; font-size:var(--ip-text-body-size); line-height:1.6; white-space:pre-wrap; word-break:break-word; background-color:var(--color-message-user-bg); color:var(--color-message-user-text); border-bottom-right-radius:4px; }

/* ===== 助手消息文字（无自带背景，由组容器承载气泡块）===== */
.message-group.assistant .message-bubble { padding:0; border-radius:0; font-size:var(--ip-text-body-size); line-height:1.6; white-space:pre-wrap; word-break:break-word; background:transparent; }

/* ===== 用户消息内容（含图片） ===== */
.user-content { display:flex; flex-direction:column; gap:4px; }
.user-text { display:block; white-space:pre-wrap; }
.user-images { display:flex; flex-wrap:wrap; gap:4px; margin-top:2px; }
.user-image { max-width:200px; max-height:200px; border-radius:var(--ip-radius-lg); object-fit:cover; border:1px solid var(--ip-color-border-default); }

/* ===== 消息底部（时间 + 复制按钮） ===== */
.message-footer { display:flex; align-items:center; justify-content:space-between; gap:8px; margin-top:2px; padding:0 4px; }
.footer-left { display:flex; align-items:center; gap:6px; }
.footer-actions { display:flex; align-items:center; gap:6px; opacity:0; transition:opacity var(--ip-duration-fast) var(--ip-ease-out); }
.message-group:hover .footer-actions { opacity:1; }
.message-group:hover .message-footer { opacity:1; }
.message-time { font-size:11px; color:var(--ip-color-text-disabled); }
.copy-btn { display:flex; align-items:center; justify-content:center; width:24px; height:24px; border-radius:var(--ip-radius-md); border:none; background:transparent; color:var(--ip-color-text-tertiary); cursor:pointer; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.copy-btn:hover { background-color:var(--ip-color-bg-tertiary); color:var(--ip-color-text-secondary); }

.badge-model { font-size:10px; color:var(--ip-color-text-tertiary); padding:1px 6px; border-radius:var(--ip-radius-sm); background:var(--ip-color-bg-tertiary); white-space:nowrap; }
.badge-tokens { font-size:10px; color:var(--ip-color-text-tertiary); white-space:nowrap; font-variant-numeric:tabular-nums; }

/* ===== 思考中动画 ===== */
.think-dots { display:flex; align-items:center; gap:4px; padding:4px 0; min-height:22px; }
.think-dot { width:6px; height:6px; border-radius:50%; background-color:var(--ip-color-text-secondary); animation:think-bounce 1.4s ease-in-out infinite; }
.think-dot:nth-child(2) { animation-delay:0.16s; }
.think-dot:nth-child(3) { animation-delay:0.32s; }
@keyframes think-bounce { 0%,80%,100% { transform:translateY(0); opacity:0.4; } 40% { transform:translateY(-6px); opacity:1; } }

/* ===== 流式光标 ===== */
.cursor-bar { display:flex; justify-content:flex-start; padding:4px 48px 0; }
.cursor-track { display:flex; align-items:center; gap:8px; padding:4px 0; }
.cursor-glow { width:8px; height:8px; border-radius:50%; background-color:var(--ip-primary-500); animation:cursor-pulse 1.2s ease-in-out infinite; }
.cursor-label { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-tertiary); }
@keyframes cursor-pulse { 0%,100% { opacity:1; transform:scale(1); } 50% { opacity:0.4; transform:scale(0.75); } }

/* ===== 状态 ===== */
.state-hint { height:100%; display:flex; align-items:center; justify-content:center; gap:8px; color:var(--ip-color-text-tertiary); font-size:var(--ip-text-body-sm-size); }
.state-dot { width:6px; height:6px; border-radius:50%; background-color:var(--ip-primary-500); animation:cursor-pulse 1.2s ease-in-out infinite; }

/* 骨架屏：消息列表加载中 */
.msg-skeleton { display:flex; flex-direction:column; gap:24px; padding:24px; }
.msg-skeleton-block { display:flex; flex-direction:column; gap:8px; }
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
.scroll-bottom-btn { position:fixed; top:80px; right:48px; z-index:50; width:32px; height:32px; border-radius:var(--ip-radius-lg); border:1px solid var(--ip-color-border-default); background-color:var(--ip-color-bg-elevated); color:var(--ip-color-text-secondary); box-shadow:var(--ip-shadow-sm); cursor:pointer; display:flex; align-items:center; justify-content:center; transition:all var(--ip-duration-fast) var(--ip-ease-out); backdrop-filter:blur(8px); }
.scroll-bottom-btn:hover { background-color:var(--ip-color-bg-secondary); color:var(--ip-color-text-primary); border-color:var(--ip-color-border-strong); box-shadow:var(--ip-shadow-md); }

.fade-up-enter-active { animation:fade-up-in 0.2s ease-out; }
.fade-up-leave-active { animation:fade-up-in 0.15s ease-in reverse; }
@keyframes fade-up-in { from { opacity:0; transform:translateY(8px); } to { opacity:1; transform:translateY(0); } }

/* ===== 思考过程（无边框无背景，左绿线标识） ===== */
.think-block { margin:0; }
.think-toggle { display:flex; align-items:center; gap:6px; padding:2px 6px; cursor:pointer; user-select:none; border-radius:var(--ip-radius-sm); transition:all var(--ip-duration-fast) var(--ip-ease-out); width:100%; }
.think-toggle:hover { background:var(--ip-color-bg-tertiary); }
.think-chevron { font-size:9px; color:var(--ip-color-text-disabled); line-height:1; width:10px; flex-shrink:0; transition:transform var(--ip-duration-fast) var(--ip-ease-out); }
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
.tool-chevron { font-size:9px; color:var(--ip-color-text-disabled); line-height:1; width:10px; flex-shrink:0; }
.tool-name { font-size:var(--ip-text-caption-size); font-weight:var(--ip-font-weight-medium); color:var(--ip-color-text-tertiary); white-space:nowrap; }
.tool-duration { font-size:10px; color:var(--ip-color-text-disabled); font-family:var(--ip-font-mono, monospace); white-space:nowrap; flex-shrink:0; }
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
.tool-expand-hdr { font-size:10px; font-weight:var(--ip-font-weight-semibold); color:var(--ip-color-text-tertiary); margin-bottom:4px; letter-spacing:0.5px; }
.tool-expand-hdr.hdr-err { color:var(--ip-danger-base); }
.tool-expand-code { font-size:var(--ip-text-caption-size); font-family:var(--ip-font-mono, monospace); white-space:pre-wrap; word-break:break-word; color:var(--ip-color-text-secondary); background:var(--ip-color-bg-tertiary); padding:6px 8px; border-radius:var(--ip-radius-sm); margin:0; line-height:1.5; max-height:200px; overflow-y:auto; }
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
.tool-detail-hdr { font-size:10px; font-weight:var(--ip-font-weight-semibold); color:var(--ip-color-text-tertiary); margin-bottom:4px; text-transform:uppercase; letter-spacing:0.5px; }
.tool-detail-hdr.hdr-err { color:var(--ip-danger-base); }
.tool-detail-code { font-size:var(--ip-text-caption-size); font-family:var(--ip-font-mono, monospace); white-space:pre-wrap; word-break:break-word; color:var(--ip-color-text-secondary); background:var(--ip-color-bg-tertiary); padding:6px 8px; border-radius:var(--ip-radius-sm); max-height:180px; overflow-y:auto; margin:0; line-height:1.5; }
.tool-detail-code.code-err { color:var(--ip-danger-base); }
.tool-detail-pending { font-size:var(--ip-text-caption-size); color:var(--ip-color-text-disabled); font-style:italic; }
.proposal-wrapper { padding: 0 48px; }
.chat-error-banner { display:flex; align-items:flex-start; gap:8px; margin:8px 16px; padding:10px 14px; background:#fef2f2; border:1px solid #fecaca; border-radius:var(--ip-radius-md); font-size:var(--ip-text-body-sm-size); }
.chat-error-icon { display:flex; align-items:center; justify-content:center; width:20px; height:20px; border-radius:50%; background:#ef4444; color:#fff; font-size:12px; font-weight:700; flex-shrink:0; }
.chat-error-text { color:#991b1b; line-height:1.5; word-break:break-word; }
</style>
