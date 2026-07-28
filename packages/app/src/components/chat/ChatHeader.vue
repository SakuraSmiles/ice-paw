<script setup lang="ts">
// ChatHeader.vue — 聊天区域顶部标题栏
import { ref } from "vue";

const agentName = ref("默认助手");
const modelName = ref("GPT-4o");
</script>

<template>
  <header class="chat-header">
    <div class="header-left">
      <div class="header-info">
        <h1 class="header-title">{{ agentName }}</h1>
        <div class="header-meta">
          <span class="header-model">{{ modelName }}</span>
        </div>
      </div>
    </div>
    <div class="header-right">
      <button class="header-btn" title="清空对话">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="3 6 5 6 21 6" />
          <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
        </svg>
      </button>
      <button class="header-btn" title="更多">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="1" />
          <circle cx="19" cy="12" r="1" />
          <circle cx="5" cy="12" r="1" />
        </svg>
      </button>
    </div>
  </header>
</template>

<style scoped>
.chat-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 24px;
  min-height: 68px;
  border-bottom: 1px solid var(--color-chat-header-border);
  background-color: var(--color-chat-header-bg);
  backdrop-filter: blur(8px);
  flex-shrink: 0;
}

.header-left {
  display: flex;
  align-items: center;
}

.header-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.header-title {
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0;
  line-height: 1.4;
}

.header-meta {
  display: flex;
  align-items: center;
  gap: 6px;
}

.header-model {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.4;
}

.header-right {
  display: flex;
  align-items: center;
  gap: 4px;
}

.header-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.header-btn:hover {
  background-color: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
}
</style>
