<!--
  ChatWelcome — 无活跃会话时的欢迎/空态界面

  行为：
  - 委托 useNewConversation() 处理「新建对话」入口
  - 按当前空间（项目/散落）做上下文化引导
  - 项目背景注入状态条（L2 状态上屏）：project.md 注入不再是黑盒
  - 取代早期误导性的空输入框

  Props: 无
  Emits: 无
-->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { useChatStore } from "../../stores/chat";
import { useProjectStore } from "../../stores/project";
import { useNewConversation } from "../../composables/useNewConversation";
import AgentPicker from "./AgentPicker.vue";

const router = useRouter();
const chat = useChatStore();
const project = useProjectStore();
const { showPicker, pickerAgentIds, ctaKind, ctaLabel, startNew, onPickAgent } = useNewConversation();

const inProject = computed(() => project.activeProjectId !== null);
const projectName = computed(() => project.activeProject?.name ?? "");
const titleText = computed(() =>
  inProject.value ? `在「${projectName.value}」中开始` : "开始一段新对话",
);
const descText = computed(() => {
  if (ctaKind.value === "no-agents") return "你还没有创建智能体，先创建一个再开始对话";
  if (ctaKind.value === "no-members") return "该项目还没有成员，先添加成员再开始对话";
  if (inProject.value) return "选择一位项目成员开始对话";
  return "选择一位助手开始对话";
});

// ----- 项目背景注入状态条（仅项目空间；失败/不可用静默隐藏，非关键路径） -----
const ctxState = ref<"loading" | "ready" | "empty" | "hidden">("hidden");
const ctxChars = ref(0);

async function refreshCtxStatus(pid: string | null) {
  if (!pid) {
    ctxState.value = "hidden";
    return;
  }
  ctxState.value = "loading";
  try {
    const c = await project.loadContext(pid); // 缓存命中不重复拉（编辑区 force 版互不干扰）
    if (!c.available) {
      ctxState.value = "hidden";
      return;
    }
    ctxChars.value = c.project_md.trim().length;
    ctxState.value = ctxChars.value > 0 ? "ready" : "empty";
  } catch {
    ctxState.value = "hidden";
  }
}

watch(
  () => project.activeProjectId,
  (pid) => {
    void refreshCtxStatus(pid);
  },
  { immediate: true },
);

/** 状态条点击 → 项目详情设置 tab（MA-2 直达编辑区；pill 渲染即有 activeProjectId） */
function openContextSettings() {
  const pid = project.activeProjectId;
  if (pid) router.push(`/projects/${pid}/settings`);
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
      <button
        v-if="ctxState === 'ready' || ctxState === 'empty'"
        class="ctx-pill"
        title="project.md 随每轮对话注入本项目会话（system prompt），修改即时生效——点击直达项目设置编辑"
        @click="openContextSettings"
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" />
        </svg>
        <span v-if="ctxState === 'ready'">已注入项目说明 · {{ ctxChars }} 字</span>
        <span v-else>项目说明未填写</span>
        <span class="ctx-pill-action">{{ ctxState === "ready" ? "编辑" : "去填写" }}</span>
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
  padding: var(--ip-spacing-6);
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

/* 项目背景注入状态条（tint 令牌，勿直接 primary 底色） */
.ctx-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 26px;
  padding: 0 10px;
  margin-top: 2px;
  border: 1px solid transparent;
  border-radius: var(--ip-radius-full);
  background-color: var(--ip-color-primary-tint-bg);
  color: var(--ip-color-primary-tint-text);
  font-family: inherit;
  font-size: var(--ip-text-caption-size);
  cursor: pointer;
  transition: border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.ctx-pill:hover { border-color: var(--ip-primary-300); }
.ctx-pill-action { font-weight: var(--ip-font-weight-medium); text-decoration: underline; text-underline-offset: 2px; }
</style>
