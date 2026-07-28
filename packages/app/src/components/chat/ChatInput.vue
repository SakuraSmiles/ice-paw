<script setup lang="ts">
// ChatInput.vue — 聊天输入框
import { ref } from "vue";

const input = ref("");

function send() {
  if (!input.value.trim()) return;
  // TODO: 接入后端 send_message
  input.value = "";
}

function handleKeydown(e: KeyboardEvent) {
  // Enter 发送，Shift+Enter 换行
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    send();
  }
}
</script>

<template>
  <div class="input-area">
    <div class="input-container">
      <div class="input-wrapper">
        <textarea
          v-model="input"
          class="chat-textarea"
          placeholder="输入消息…"
          rows="1"
          @keydown="handleKeydown"
        />
        <button
          class="btn-send"
          :class="{ active: input.trim() }"
          :disabled="!input.trim()"
          @click="send"
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="22" y1="2" x2="11" y2="13" />
            <polygon points="22 2 15 22 11 13 2 9 22 2" />
          </svg>
        </button>
      </div>
      <p class="input-hint">Enter 发送 · Shift+Enter 换行</p>
    </div>
  </div>
</template>

<style scoped>
.input-area {
  flex-shrink: 0;
  padding: 16px 24px 24px;
  border-top: 1px solid var(--color-chat-header-border);
}

.input-container {
  max-width: 800px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.input-wrapper {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 8px 8px 8px 16px;
  background-color: var(--color-input-bg);
  border: 1px solid var(--color-input-border);
  border-radius: 12px;
  transition: border-color var(--ip-duration-fast) var(--ip-ease-out);
}

.input-wrapper:focus-within {
  border-color: var(--color-input-focus-border);
  box-shadow: 0 0 0 3px rgba(46, 141, 100, 0.12);
}

.chat-textarea {
  flex: 1;
  border: none;
  outline: none;
  background: transparent;
  resize: none;
  font-size: var(--ip-text-body-size);
  line-height: 1.5;
  color: var(--ip-color-text-primary);
  max-height: 200px;
  min-height: 22px;
  padding: 0;
}

.chat-textarea::placeholder {
  color: var(--ip-color-text-placeholder);
}

.btn-send {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: var(--ip-radius-md);
  background-color: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-disabled);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
  flex-shrink: 0;
}

.btn-send.active {
  background-color: var(--color-message-user-bg);
  color: white;
}

.btn-send.active:hover {
  opacity: 0.9;
}

.input-hint {
  font-size: 11px;
  color: var(--ip-color-text-disabled);
  text-align: center;
}
</style>
