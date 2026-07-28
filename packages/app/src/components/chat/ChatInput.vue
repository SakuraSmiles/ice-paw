<script setup lang="ts">
// ChatInput.vue — 聊天输入框 + 停止按钮
import { computed, watch, nextTick, ref } from "vue";
import { useChatStore } from "../../stores/chat";

const chat = useChatStore();
const textareaRef = ref<HTMLTextAreaElement | null>(null);

// 用 store.draftText 替代本地 input ref，跨页面切换保持内容
const input = computed({
  get: () => chat.draftText,
  set: (v: string) => { chat.draftText = v; },
});

// 发送结束后（sending: true → false），自动聚焦输入框
watch(() => chat.sending, (sending) => {
  if (!sending) {
    nextTick(() => textareaRef.value?.focus());
  }
});

function send() {
  const text = input.value.trim();
  if (!text || chat.sending) return;
  chat.draftText = "";
  chat.sendMessage(text);
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    send();
  }
}
</script>

<template>
  <div class="input-area">
    <div class="input-container">
      <div class="input-wrapper" :class="{ 'is-sending': chat.sending }">
        <textarea
          ref="textareaRef"
          v-model="input"
          class="chat-textarea"
          placeholder="输入消息…"
          rows="1"
          :disabled="chat.sending"
          @keydown="handleKeydown"
        />
        <div class="btn-group">
          <button
            v-if="!chat.sending"
            class="btn-send"
            :class="{ active: input.trim() }"
            :disabled="!input.trim()"
            @click="send"
            title="发送 (Enter)"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <line x1="22" y1="2" x2="11" y2="13" />
              <polygon points="22 2 15 22 11 13 2 9 22 2" />
            </svg>
          </button>
          <button
            v-else
            class="btn-stop"
            @click="chat.stopGeneration()"
            title="停止生成"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
              <rect x="6" y="6" width="12" height="12" rx="2" />
            </svg>
          </button>
        </div>
      </div>
      <p class="input-hint">{{ chat.sending ? "正在生成…" : "Enter 发送 · Shift+Enter 换行" }}</p>
    </div>
  </div>
</template>

<style scoped>
.input-area { flex-shrink:0; padding:16px 24px 24px; border-top:1px solid var(--color-chat-header-border); }
.input-container { max-width:800px; margin:0 auto; display:flex; flex-direction:column; gap:8px; }
.input-wrapper { display:flex; align-items:flex-start; gap:8px; padding:8px 8px 8px 16px; background-color:var(--color-input-bg); border:1px solid var(--color-input-border); border-radius:12px; transition:border-color var(--ip-duration-base) var(--ip-ease-out),box-shadow var(--ip-duration-base) var(--ip-ease-out); }
.input-wrapper:focus-within { border-color:var(--color-input-focus-border); box-shadow:0 0 0 3px rgba(46,141,100,0.12); }
.input-wrapper.is-sending { border-color:var(--ip-primary-400); box-shadow:0 0 0 3px rgba(46,141,100,0.08); }
.chat-textarea { flex:1; border:none; outline:none; background:transparent; resize:none; font-size:var(--ip-text-body-size); line-height:1.5; color:var(--ip-color-text-primary); max-height:200px; min-height:22px; padding:0; }
.chat-textarea::placeholder { color:var(--ip-color-text-placeholder); }
.chat-textarea:disabled { opacity:0.35; cursor:not-allowed; }
.btn-group { position:relative; width:36px; height:36px; flex-shrink:0; }
.btn-send { position:absolute; inset:0; display:flex; align-items:center; justify-content:center; border-radius:var(--ip-radius-md); background-color:var(--ip-color-bg-tertiary); color:var(--ip-color-text-disabled); border:none; cursor:pointer; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.btn-send.active { background-color:var(--color-message-user-bg); color:white; }
.btn-send.active:hover { opacity:0.9; transform:scale(1.05); }
.btn-send.active:active { transform:scale(0.95); }
.btn-stop { position:absolute; inset:0; display:flex; align-items:center; justify-content:center; border-radius:var(--ip-radius-md); background-color:var(--ip-danger-base); color:white; border:none; cursor:pointer; transition:all var(--ip-duration-fast) var(--ip-ease-out); animation:stop-enter 0.2s ease-out; }
.btn-stop:hover { opacity:0.9; }
@keyframes stop-enter { from { opacity:0; transform:scale(0.85); } to { opacity:1; transform:scale(1); } }
.input-hint { font-size:11px; color:var(--ip-color-text-disabled); text-align:center; }
</style>
