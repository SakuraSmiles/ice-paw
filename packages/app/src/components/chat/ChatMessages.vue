<script setup lang="ts">
// ChatMessages.vue — 聊天消息列表（含分页加载）
import { watch, nextTick, ref, computed, onMounted, onUnmounted } from "vue";
import { useChatStore } from "../../stores/chat";
import { bridge } from "../../api/bridge";
import MarkdownRenderer from "./MarkdownRenderer.vue";

const chat = useChatStore();
const listRef = ref<HTMLElement | null>(null);
const showScrollBtn = ref(false);
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

const userTimezone = ref("");

onMounted(async () => {
  try {
    const prefs = await bridge.preferences.get();
    userTimezone.value = prefs.timezone || "";
  } catch {}
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

function formatJson(str: string): string {
  try { return JSON.stringify(JSON.parse(str), null, 2); } catch { return str; }
}

function truncateJson(str: string, maxLen = 80): string {
  if (str.length <= maxLen) return str;
  return str.substring(0, maxLen) + '…';
}

/** 判断一个 assistant 消息是否有非 text 的附属内容（tool/thinking） */
function hasExtras(msg: any): boolean {
  if (!msg.content_blocks || msg.content_blocks === '[]') return false;
  try {
    const blocks = JSON.parse(msg.content_blocks);
    return Array.isArray(blocks) && blocks.some((b: any) => b.type === 'tool_use' || b.type === 'thinking');
  } catch { return false; }
}

// 检测滚动位置：非底部显示按钮，靠近顶部触发分页
function onScroll() {
  if (suppressScrollCheck || paginating) return;
  const el = listRef.value;
  if (!el) return;

  const distToBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
  showScrollBtn.value = distToBottom > 80;

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
    showScrollBtn.value = false;
    listRef.value.scrollTo({ top: listRef.value.scrollHeight, behavior: smooth !== false ? "smooth" : "instant" });
    setTimeout(() => { suppressScrollCheck = false; }, smooth !== false ? 500 : 50);
  }
}

onMounted(() => {
  listRef.value?.addEventListener("scroll", onScroll);
  scrollToBottom(false);
});
onUnmounted(() => { listRef.value?.removeEventListener("scroll", onScroll); });

// 切换会话后等消息加载完成再平滑滚动到底部
watch(() => chat.msgLoading, async (loading) => {
  if (!loading && chat.messages.length > 0) {
    await nextTick();
    scrollToBottom(true);
  }
});

// 自动滚到底部（分页加载时不触发，避免与位置恢复冲突）
watch(
  [() => chat.messages.length, () => chat.streamingText],
  async () => {
    if (paginating) return;
    await nextTick();
    const el = listRef.value;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  },
);

function copyContent(content: string) {
  navigator.clipboard.writeText(content);
}

function parseImageBlocks(contentBlocks: string): { data: string; mediaType: string }[] {
  try {
    const blocks = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b: any) => b?.type === "image").map((b: any) => ({ data: b.data, mediaType: b.media_type }));
  } catch { return []; }
}

function parseToolUseBlocks(contentBlocks: string): { id: string; name: string; input: string }[] {
  try {
    const blocks = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b: any) => b?.type === "tool_use").map((b: any) => ({ id: b.id, name: b.name, input: b.input }));
  } catch { return []; }
}

function parseToolResultBlocks(contentBlocks: string): { toolUseId: string; content: string; isError: boolean }[] {
  try {
    const blocks = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b: any) => b?.type === "tool_result").map((b: any) => ({ toolUseId: b.tool_use_id, content: b.content, isError: b.is_error ?? false }));
  } catch { return []; }
}

function parseThinkingBlocks(contentBlocks: string): string[] {
  try {
    const blocks = JSON.parse(contentBlocks);
    if (!Array.isArray(blocks)) return [];
    return blocks.filter((b: any) => b?.type === "thinking").map((b: any) => b.thinking);
  } catch { return []; }
}

/** 查询某个 tool_use_id 对应的 tool_result 是否有 isError */
function getToolHasError(contentBlocks: string, toolUseId: string): boolean {
  const results = parseToolResultBlocks(contentBlocks);
  const found = results.find(r => r.toolUseId === toolUseId);
  return found?.isError ?? false;
}

// ===== 时间分组 =====
function getDateLabel(dateStr: string): string | null {
  const d = new Date(dateStr);
  if (isNaN(d.getTime())) return null;

  const tz = userTimezone.value || undefined;
  const fmtDate = (dt: Date) => {
    if (tz) {
      try {
        const parts = new Intl.DateTimeFormat("zh-CN", { timeZone: tz, year: "numeric", month: "numeric", day: "numeric" }).formatToParts(dt);
        const y = parts.find(p => p.type === "year")?.value || "";
        const m = parts.find(p => p.type === "month")?.value || "";
        const day = parts.find(p => p.type === "day")?.value || "";
        return `${y}-${m}-${day}`;
      } catch {}
    }
    return `${dt.getFullYear()}-${dt.getMonth()}-${dt.getDate()}`;
  };

  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);

  const dKey = fmtDate(d);
  if (dKey === fmtDate(today)) return "今天";
  if (dKey === fmtDate(yesterday)) return "昨天";

  return `${d.getMonth() + 1}月${d.getDate()}日`;
}

function isNewDay(idx: number): boolean {
  if (idx === 0) return true;
  const prev = chat.messages[idx - 1].created_at;
  const curr = chat.messages[idx].created_at;
  return getDateLabel(prev) !== getDateLabel(curr);
}

function formatTime(createdAt: string): string {
  const d = new Date(createdAt);
  if (isNaN(d.getTime())) return "";
  if (userTimezone.value) {
    try {
      return new Intl.DateTimeFormat("zh-CN", {
        timeZone: userTimezone.value,
        hour: "2-digit",
        minute: "2-digit",
        hour12: false
      }).format(d);
    } catch {}
  }
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${hh}:${mm}`;
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
    <!-- 分页加载指示器 -->
    <div v-if="chat.loadingMore" class="load-more-hint">加载更早消息…</div>
    <div v-if="!chat.hasMore && chat.messages.length > 50" class="load-more-hint load-more-end">已显示全部消息</div>

    <div v-if="chat.msgLoading && chat.messages.length === 0" class="state-hint">
      <span class="state-dot" />加载中...
    </div>
    <div v-else-if="!chat.activeConvId" class="state-hint">选择一个对话开始</div>
    <div v-else-if="chat.messages.length === 0" class="state-hint">开始一段新的对话</div>
    <TransitionGroup v-else name="msg" tag="div" class="messages-container">
      <template v-for="(msg, idx) in chat.messages" :key="msg.id">
        <!-- 日期分组标签 -->
        <div v-if="isNewDay(idx)" class="date-divider">{{ getDateLabel(msg.created_at) }}</div>
        <div
          :class="['message-row', msg.role]"
        >
        <div :class="['message-content', msg.role, { 'has-extras': hasExtras(msg) }]">
          <!-- ===== 用户消息 ===== -->
          <template v-if="msg.role === 'user'">
            <div class="message-bubble">
              <span v-if="msg.content" class="user-text">{{ msg.content }}</span>
              <div v-if="msg.content_blocks && msg.content_blocks !== '[]'" class="user-images">
                <img v-for="(img, i) in parseImageBlocks(msg.content_blocks)" :key="i" :src="`data:${img.mediaType};base64,${img.data}`" class="user-image" loading="lazy" />
              </div>
            </div>
          </template>

          <!-- ===== 助手消息 ===== -->
          <template v-else-if="msg.role === 'assistant'">
            <!-- 三个点动画：仅当没有任何返回时显示 -->
            <div v-if="msg.content === '' && chat.sending && !chat.streamingThinking && toolCallList.length === 0" class="think-dots">
              <span class="think-dot" /><span class="think-dot" /><span class="think-dot" />
            </div>

            <template v-if="msg.content || !chat.sending || chat.streamingThinking || toolCallList.length > 0">
              <!-- 思考过程（历史消息） -->
              <div v-for="(think, ti) in parseThinkingBlocks(msg.content_blocks)" :key="'think-' + ti" class="think-block">
                <div class="think-toggle" @click="toggleThinking(msg.id + '-h' + ti)">
                  <span class="think-chevron">{{ expandedThinking.has(msg.id + '-h' + ti) ? '▾' : '▸' }}</span>
                  <span class="think-label">{{ chat.thinkingDurations.has(msg.id) ? '思考 · ' + chat.thinkingDurations.get(msg.id) : '思考' }}</span>
                </div>
                <Transition name="think-fade">
                  <div v-if="expandedThinking.has(msg.id + '-h' + ti)" class="think-body">
                    <MarkdownRenderer :content="think" />
                  </div>
                </Transition>
              </div>

              <!-- 思考过程（流式 / 刚结束，带切换动画） -->
              <Transition name="think-swap" mode="out-in">
                <div v-if="idx === chat.messages.length - 1 && chat.streamingThinking" key="live" class="think-block">
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
                <div v-else-if="idx === chat.messages.length - 1 && chat.thinkingDuration && chat.lastThinkingContent" key="done" class="think-block">
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

              <!-- 工具调用（历史/刚结束，从 content_blocks 解析，非流式） -->
              <div v-if="parseToolUseBlocks(msg.content_blocks).length > 0 && !(idx === chat.messages.length - 1 && toolCallList.length > 0)" class="tools-strip">
                <div v-for="tu in parseToolUseBlocks(msg.content_blocks)" :key="tu.id">
                  <div class="tool-toggle" @click="toggleToolCall(tu.id)">
                    <span class="tool-chevron">{{ expandedToolCalls.has(tu.id) ? '▾' : '▸' }}</span>
                    <span class="tool-name">{{ tu.name }}</span>
                    <span class="tool-preview">{{ truncateJson(tu.input) }}</span>
                    <span :class="['tool-dot', getToolHasError(msg.content_blocks, tu.id) ? 'tool-dot-err' : 'tool-dot-ok']"></span>
                  </div>
                  <Transition name="tool-slide">
                    <div v-if="expandedToolCalls.has(tu.id)" class="tool-expand">
                      <div class="tool-expand-group">
                        <div class="tool-expand-hdr">参数</div>
                        <pre class="tool-expand-code">{{ formatJson(tu.input) }}</pre>
                      </div>
                      <div v-for="tr in parseToolResultBlocks(msg.content_blocks)" :key="'r-' + tr.toolUseId">
                        <div v-if="tr.toolUseId === tu.id" class="tool-expand-group">
                          <div :class="['tool-expand-hdr', tr.isError ? 'hdr-err' : '']">{{ tr.isError ? '错误' : '结果' }}</div>
                          <pre :class="['tool-expand-code', tr.isError ? 'code-err' : '']">{{ tr.content }}</pre>
                        </div>
                      </div>
                    </div>
                  </Transition>
                </div>
              </div>

              <!-- 工具调用（当前流式） -->
              <div v-if="idx === chat.messages.length - 1 && toolCallList.length > 0" class="tools-strip">
                <div v-for="call in toolCallList" :key="call.id">
                  <div class="tool-toggle" @click="toggleToolCall(call.id)">
                    <span class="tool-chevron">{{ expandedToolCalls.has(call.id) ? '▾' : '▸' }}</span>
                    <span class="tool-name">{{ call.name }}</span>
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

              <!-- 文字气泡（仅当有内容时才显示） -->
              <div v-if="msg.content" class="message-bubble">
                <MarkdownRenderer :content="msg.content" />
              </div>
            </template>
          </template>

          <!-- ===== 底部信息 ===== -->
          <div v-if="msg.content || (msg.role === 'assistant' && (chat.sending || toolCallList.length > 0 || hasExtras(msg)))" class="message-footer">
            <div class="footer-left">
              <span class="message-time">{{ formatTime(msg.created_at) }}</span>
              <span v-if="msg.model && msg.role === 'assistant'" class="badge-model">{{ msg.model }}</span>
              <span v-if="msg.token_count" class="badge-tokens">{{ msg.token_count }} tokens</span>
            </div>
            <div class="footer-actions">
              <button v-if="msg.content" class="copy-btn" title="复制" @click="copyContent(msg.content)">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <rect x="9" y="9" width="13" height="13" rx="2" ry="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
              </svg>
            </button>
            </div>
          </div>
        </div>
      </div>
    </template>
    </TransitionGroup>

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
      <button v-if="showScrollBtn && !chat.sending" class="scroll-bottom-btn" @click="scrollToBottom()" title="滚动到底部">
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

/* ===== 消息行 ===== */
.message-row { display:flex; }
.message-row.user { justify-content:flex-end; }
.message-row.assistant { justify-content:flex-start; }
.message-content { display:flex; flex-direction:column; gap:4px; min-width:0; }
.message-row.assistant .message-content { max-width:85%; }
.message-row.user .message-content { max-width:70%; align-items:flex-end; }

/* ===== 用户消息气泡 ===== */
.message-row.user .message-bubble { padding:10px 16px; border-radius:12px; font-size:var(--ip-text-body-size); line-height:1.6; white-space:pre-wrap; word-break:break-word; background-color:var(--color-message-user-bg); color:var(--color-message-user-text); border-bottom-right-radius:4px; }

/* ===== 助手消息气泡（纯文字） ===== */
.message-row.assistant .message-bubble { padding:10px 16px; border-radius:12px; font-size:var(--ip-text-body-size); line-height:1.6; white-space:pre-wrap; word-break:break-word; background-color:var(--color-message-ai-bg); color:var(--color-message-ai-text); border-bottom-left-radius:4px; }

/* ===== 用户消息内容（含图片） ===== */
.user-content { display:flex; flex-direction:column; gap:4px; }
.user-text { display:block; white-space:pre-wrap; }
.user-images { display:flex; flex-wrap:wrap; gap:4px; margin-top:2px; }
.user-image { max-width:200px; max-height:200px; border-radius:var(--ip-radius-lg); object-fit:cover; border:1px solid var(--ip-color-border-default); }

/* ===== 消息底部（时间 + 复制按钮） ===== */
.message-bubble-wrap { display:flex; align-items:flex-start; }
.message-footer { display:flex; align-items:center; justify-content:space-between; gap:8px; margin-top:2px; padding:0 4px; }
.footer-left { display:flex; align-items:center; gap:6px; }
.footer-actions { display:flex; align-items:center; gap:6px; opacity:0; transition:opacity var(--ip-duration-fast) var(--ip-ease-out); }
.message-content:hover .footer-actions { opacity:1; }
.message-content:hover .message-footer { opacity:1; }
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
.tool-name { font-size:var(--ip-text-caption-size); font-weight:var(--ip-font-weight-medium); color:var(--ip-color-text-secondary); white-space:nowrap; }
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
</style>
