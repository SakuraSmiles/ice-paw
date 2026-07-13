<script setup lang="ts">
// 聊天页头部
//
// 职责：
//   - 标题左侧显示 Agent 小头像（20x20，字母缩写 / Lucide 图标）
//   - 显示当前会话标题 / Agent 名 + 模型名
//   - 右侧：流式中显示「停止」按钮（lucide Square 图标）
//   - 半透明毛玻璃背景（backdrop-filter: blur(8px) + 半透明底色）
//   - 高度 48px，颜色全部走 --ip-* Design Token
//
// props: 无（直接读 store）
//
// emits:
//   - stop  点击停止按钮时触发（外层接住后调 chatStore.stopGeneration）

import { computed } from "vue";
import { Square } from "lucide-vue-next";
import { useAgentsStore } from "../../stores/agents";
import { useConversationsStore } from "../../stores/conversations";
import { useChatStore } from "../../stores/chat";
import { useAgentMeta, type AgentMeta } from "../../composables/useAgentMeta";
import AgentAvatar from "../common/AgentAvatar.vue";

const agentsStore = useAgentsStore();
const conversationsStore = useConversationsStore();
const chatStore = useChatStore();
const agentMeta = useAgentMeta();

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

/** 标题层：优先用会话标题，无则用 Agent 名 */
const headerTitle = computed<string>(() => {
  const t = convTitle.value;
  if (t && t !== "新会话") return t;
  return agentName.value;
});

/** 当前 Agent 的完整 meta（用于头像渲染） */
const meta = computed<AgentMeta | null>(() => {
  const agent = agentsStore.current;
  if (!agent) return null;
  return agentMeta.getFullMeta(agent);
});

/** 停止按钮点击 */
function onStop(): void {
  emit("stop");
}
</script>

<template>
  <header class="chat-header">
    <div class="header-main">
      <div class="title-row">
        <!-- Agent 小头像（20x20） -->
        <AgentAvatar
          v-if="meta"
          :meta="meta"
          :size="20"
          class="header-avatar"
          aria-hidden="true"
        />
        <span class="conv-title">{{ headerTitle }}</span>
      </div>
      <div v-if="agentModel" class="meta-row">
        <span class="model-name">{{ agentModel }}</span>
      </div>
    </div>
    <div class="header-actions">
      <button
        v-if="chatStore.isStreaming"
        class="btn-stop"
        type="button"
        title="停止生成"
        aria-label="停止生成"
        @click="onStop"
      >
        <Square :size="14" aria-hidden="true" />
        <span class="btn-label">停止</span>
      </button>
    </div>
  </header>
</template>

<style scoped>
.chat-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-3);
  height: var(--ip-spacing-12);
  padding: 0 var(--ip-spacing-5);
  background-color: var(--ip-color-bg-header-backdrop);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
  border-bottom: 1px solid var(--ip-color-border-default);
  color: var(--ip-color-text-primary);
  flex-shrink: 0;
  position: relative;
  z-index: var(--ip-z-sticky);
}

.header-main {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: var(--ip-spacing-0_5);
  min-width: 0;
  flex: 1;
  overflow: hidden;
}

.title-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  min-width: 0;
}

.header-avatar {
  flex-shrink: 0;
}

.conv-title {
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  line-height: 1.2;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.meta-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size);
  line-height: 1.2;
  color: var(--ip-color-text-tertiary);
  min-width: 0;
}

.model-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 280px;
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-caption-size);
  letter-spacing: var(--ip-letter-spacing-normal);
}

.header-actions {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
}

.btn-stop {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  height: var(--ip-btn-h-sm);
  padding: 0 var(--ip-spacing-3);
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  font-family: inherit;
  border: 1px solid var(--ip-danger-border);
  border-radius: var(--ip-btn-radius);
  background: var(--ip-danger-bg);
  color: var(--ip-danger-text);
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.btn-stop:hover {
  background: var(--ip-danger-base);
  color: var(--ip-color-text-on-danger);
  border-color: var(--ip-danger-hover);
}

.btn-stop:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.btn-stop:active {
  background: var(--ip-danger-active);
  border-color: var(--ip-danger-active);
}

.btn-label {
  line-height: 1;
}
</style>
