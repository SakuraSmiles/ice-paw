<script setup lang="ts">
// 聊天页头部
//
// 职责：
//   - 显示当前 Agent 名 + 当前会话标题 + 模型名
//   - 右侧：流式中显示「停止」按钮，否则留空（占位）
//
// props: 无（直接读 store）
//
// emits:
//   - stop  点击停止按钮时触发（外层接住后调 chatStore.stopGeneration）

import { computed } from "vue";
import { useAgentsStore } from "../../stores/agents";
import { useConversationsStore } from "../../stores/conversations";
import { useChatStore } from "../../stores/chat";

const agentsStore = useAgentsStore();
const conversationsStore = useConversationsStore();
const chatStore = useChatStore();

const emit = defineEmits<{
  stop: [];
}>();

/** 当前会话标题（无则显示「新会话」） */
const convTitle = computed<string>(() => {
  return conversationsStore.current?.title?.trim() || "新会话";
});

/** 当前 Agent 名（无则显示「未选择 Agent」） */
const agentName = computed<string>(() => agentsStore.current?.name ?? "未选择 Agent");

/** 当前 Agent 模型（无则空串） */
const agentModel = computed<string>(() => agentsStore.current?.model ?? "");

/** 停止按钮点击 */
function onStop(): void {
  emit("stop");
}
</script>

<template>
  <header class="chat-header">
    <div class="header-main">
      <div class="title-row">
        <span class="conv-title">{{ convTitle }}</span>
      </div>
      <div class="meta-row">
        <span class="agent-name">{{ agentName }}</span>
        <span v-if="agentModel" class="dot">·</span>
        <span v-if="agentModel" class="model-name">{{ agentModel }}</span>
      </div>
    </div>
    <div class="header-actions">
      <button
        v-if="chatStore.isStreaming"
        class="btn-stop"
        type="button"
        title="停止生成"
        @click="onStop"
      >
        停止
      </button>
    </div>
  </header>
</template>

<style scoped>
.chat-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 20px;
  border-bottom: 1px solid var(--header-border, #e0e0e0);
  background: var(--header-bg, #ffffff);
  flex-shrink: 0;
  min-height: 56px;
}

.header-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.title-row {
  display: flex;
  align-items: center;
  min-width: 0;
}

.conv-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-primary, #1a1a1a);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.meta-row {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-secondary, #888);
  min-width: 0;
}

.agent-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 220px;
}

.dot {
  color: var(--text-secondary, #888);
}

.model-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 220px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}

.header-actions {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.btn-stop {
  padding: 6px 16px;
  font-size: 13px;
  font-weight: 500;
  border: 1px solid var(--danger-border, #d93025);
  border-radius: 4px;
  background: var(--danger-bg, #ffffff);
  color: var(--danger-fg, #d93025);
  cursor: pointer;
  transition: background 100ms ease;
  font-family: inherit;
}

.btn-stop:hover {
  background: var(--danger-bg-hover, #fde8e8);
}

/* 暗色模式 */
@media (prefers-color-scheme: dark) {
  .chat-header {
    --header-bg: #1e1e2e;
    --header-border: #3a3a4a;
  }
  .conv-title {
    --text-primary: #f0f0f0;
  }
  .meta-row {
    --text-secondary: #888;
  }
  .btn-stop {
    --danger-bg: #2a2a3a;
    --danger-fg: #ff6b6b;
    --danger-border: #5a2a2a;
  }
  .btn-stop:hover {
    --danger-bg-hover: #3a2020;
  }
}
</style>
