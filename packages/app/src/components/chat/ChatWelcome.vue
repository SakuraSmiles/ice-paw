<script setup lang="ts">
// ChatWelcome.vue — 无选中会话时的欢迎/空态
// 取代原先「看着能用、其实不能用」的空输入框：给一个明确的「新建对话」入口，
// 并按当前空间（项目 / 散落）做上下文化引导。新建逻辑与侧栏共用 useNewConversation。
import { computed } from "vue";
import { useChatStore } from "../../stores/chat";
import { useProjectStore } from "../../stores/project";
import { useNewConversation } from "../../composables/useNewConversation";
import AgentPicker from "./AgentPicker.vue";

const chat = useChatStore();
const project = useProjectStore();
const { showPicker, pickerAgentIds, ctaKind, ctaLabel, startNew, onPickAgent } = useNewConversation();

const inProject = computed(() => project.activeProjectId !== null);
const projectName = computed(() => project.activeProject?.name ?? "");
const titleText = computed(() =>
  inProject.value ? `在「${projectName.value}」中开始` : "开始一段新对话",
);
const descText = computed(() => {
  if (ctaKind.value === "no-members") return "该项目还没有成员，先添加成员再开始对话";
  if (inProject.value) return "选择一位项目成员开始对话";
  return "选择一位助手开始对话";
});
</script>

<template>
  <div class="chat-welcome">
    <div class="welcome-inner">
      <div class="welcome-icon">
        <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        </svg>
      </div>
      <h2 class="welcome-title">{{ titleText }}</h2>
      <p class="welcome-desc">{{ descText }}</p>
      <button class="welcome-cta" @click="startNew">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
        </svg>
        <span>{{ ctaLabel }}</span>
      </button>
      <p v-if="chat.conversations.length > 0" class="welcome-tip">或从左侧选择一个已有会话</p>
    </div>

    <AgentPicker v-if="showPicker" :agent-ids="pickerAgentIds" @select="onPickAgent" @close="showPicker = false" />
  </div>
</template>

<style scoped>
.chat-welcome {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
}
.welcome-inner {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
  max-width: 420px;
  text-align: center;
}

.welcome-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  border-radius: var(--ip-radius-xl);
  background-color: var(--ip-color-primary-tint-bg);
  color: var(--ip-primary-500);
  margin-bottom: 4px;
}

.welcome-title {
  margin: 0;
  font-size: var(--ip-text-h2-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  line-height: 1.3;
}
.welcome-desc {
  margin: 0;
  font-size: var(--ip-text-body-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.5;
}

.welcome-cta {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 38px;
  padding: 0 20px;
  margin-top: 6px;
  border-radius: var(--ip-radius-md);
  background-color: var(--ip-primary-500);
  color: white;
  border: none;
  cursor: pointer;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.welcome-cta:hover { background-color: var(--ip-primary-600); }

.welcome-tip {
  margin: 4px 0 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-disabled);
}
</style>
