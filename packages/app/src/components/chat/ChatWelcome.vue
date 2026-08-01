<script setup lang="ts">
// ChatWelcome.vue — 无选中会话时的欢迎/空态
// 取代原先「看着能用、其实不能用」的空输入框：给一个明确的「新建对话」入口，
// 并按当前空间（项目 / 散落）做上下文化引导。
import { ref, computed } from "vue";
import { useRouter } from "vue-router";
import { useChatStore } from "../../stores/chat";
import { useAgentStore } from "../../stores/agent";
import { useProjectStore } from "../../stores/project";
import AgentPicker from "./AgentPicker.vue";

const chat = useChatStore();
const agent = useAgentStore();
const project = useProjectStore();
const router = useRouter();

const showPicker = ref(false);

const inProject = computed(() => project.activeProjectId !== null);
const projectName = computed(() => project.activeProject?.name ?? "");
const memberAgentIds = computed(() =>
  (project.activeProject?.agents ?? []).map((a) => a.agent_id),
);
const hasMembers = computed(() => memberAgentIds.value.length > 0);
const hasAgents = computed(() => agent.list.length > 0);

/** 选择器范围：项目内 → 仅成员；散落 → 全部 agent（项目无成员不会走到选择器） */
const pickerAgentIds = computed(() => (inProject.value ? memberAgentIds.value : undefined));

/** CTA 行为分三种：全局无 agent / 项目无成员 / 正常新建 */
const ctaKind = computed<"no-agents" | "no-members" | "new-chat">(() => {
  if (!hasAgents.value) return "no-agents";
  if (inProject.value && !hasMembers.value) return "no-members";
  return "new-chat";
});
const ctaLabel = computed(() => {
  if (ctaKind.value === "no-agents") return "去创建智能体";
  if (ctaKind.value === "no-members") return "去添加成员";
  return "新建对话";
});

const titleText = computed(() =>
  inProject.value ? `在「${projectName.value}」中开始` : "开始一段新对话",
);
const descText = computed(() => {
  if (ctaKind.value === "no-members") return "该项目还没有成员，先添加成员再开始对话";
  if (inProject.value) return "选择一位项目成员开始对话";
  return "选择一位助手开始对话";
});

function startNew() {
  if (ctaKind.value === "no-agents") { router.push("/settings/agents"); return; }
  if (ctaKind.value === "no-members") { router.push("/projects"); return; }
  // new-chat：单候选直接建，跳过选择；否则弹选择器（项目内已限定为成员）
  const ids = pickerAgentIds.value;
  const count = ids ? ids.length : agent.list.length;
  if (count === 1) {
    const id = ids ? ids[0] : agent.list[0].id;
    chat.createConversation(id, project.activeProjectId);
    return;
  }
  showPicker.value = true;
}

function onPickAgent(agentId: string) {
  showPicker.value = false;
  // createConversation 内部会 selectConversation → activeConvId 置位 → 本组件自动隐藏
  chat.createConversation(agentId, project.activeProjectId);
}
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
