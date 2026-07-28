<script setup lang="ts">
// ChatMessages.vue — 聊天消息列表（含分页加载）
import { watch, nextTick, ref, onMounted, onUnmounted } from "vue";
import { useChatStore } from "../../stores/chat";
import MarkdownRenderer from "./MarkdownRenderer.vue";

const chat = useChatStore();
const listRef = ref<HTMLElement | null>(null);
const showScrollBtn = ref(false);
let suppressScrollCheck = false;
let paginating = false;
let scrollPosCache = { scrollHeight: 0, scrollTop: 0 };

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

function formatTime(createdAt: string): string {
  const d = new Date(createdAt);
  if (isNaN(d.getTime())) return "";
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${hh}:${mm}`;
}
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
      <div
        v-for="msg in chat.messages"
        :key="msg.id"
        :class="['message-row', msg.role]"
      >
        <div :class="['message-content', { thinking: msg.role === 'assistant' && msg.content === '' && chat.sending }]">
          <div class="message-bubble-wrap">
            <div class="message-bubble">
              <div v-if="msg.role === 'assistant' && msg.content === '' && chat.sending" class="thinking-indicator">
                <span class="think-dot" /><span class="think-dot" /><span class="think-dot" />
              </div>
              <MarkdownRenderer v-else-if="msg.role === 'assistant'" :content="msg.content" />
              <span v-else>{{ msg.content }}</span>
            </div>
            <button v-if="msg.content" class="copy-btn" title="复制" @click="copyContent(msg.content)">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <rect x="9" y="9" width="13" height="13" rx="2" ry="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
              </svg>
            </button>
          </div>
          <div v-if="msg.content" class="message-time">{{ formatTime(msg.created_at) }}</div>
        </div>
      </div>
    </TransitionGroup>

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
.message-row.assistant .message-content.thinking { max-width:140px; }
.message-row.user .message-content { max-width:70%; align-items:flex-end; }
.message-bubble { padding:10px 16px; border-radius:12px; font-size:var(--ip-text-body-size); line-height:1.6; white-space:pre-wrap; word-break:break-word; }
.message-row.user .message-bubble { background-color:var(--color-message-user-bg); color:var(--color-message-user-text); border-bottom-right-radius:4px; }
.message-row.assistant .message-bubble { background-color:var(--color-message-ai-bg); color:var(--color-message-ai-text); border-bottom-left-radius:4px; }

/* ===== 复制按钮 ===== */
.message-bubble-wrap { position:relative; display:flex; align-items:flex-start; gap:4px; }
.copy-btn { position:absolute; top:4px; right:-32px; display:flex; align-items:center; justify-content:center; width:28px; height:28px; border-radius:var(--ip-radius-md); border:none; background:transparent; color:var(--ip-color-text-tertiary); cursor:pointer; opacity:0; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.message-bubble-wrap:hover .copy-btn { opacity:1; }
.copy-btn:hover { background-color:var(--ip-color-bg-tertiary); color:var(--ip-color-text-secondary); }

.message-time { font-size:11px; color:var(--ip-color-text-disabled); padding:0 4px; }

/* ===== 思考中动画 ===== */
.thinking-indicator { display:flex; align-items:center; gap:4px; padding:4px 0; min-height:22px; }
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
</style>
