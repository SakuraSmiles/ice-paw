<script setup lang="ts">
// WelcomePanel — IcePaw 品牌化欢迎面板（Wave 2）
//
// 职责：
//   - 有 Agent 但无会话时的品牌化欢迎页（hero 形态）
//   - 包含：品牌 pill、Hero 标题、步骤指示器、OnboardCard、paw-trail
//   - 数据全部来自 props，不直接持有 Pinia store
//
// 数据来源（由 ChatPage.vue 通过 props 注入）：
//   - agentName   来自 agentsStore.current?.name
//   - modelName   来自 agentsStore.current?.model
//   - agentMeta   来自 useAgentMeta().getFullMeta(agentsStore.current)

import { computed } from "vue";
import { Code, Search, PenTool, GraduationCap } from "lucide-vue-next";
import PawBrandMark from "../common/PawBrandMark.vue";
import PawTrail from "../common/PawTrail.vue";
import WelcomeInput from "./WelcomeInput.vue";
import type { AgentMeta } from "../../composables/useAgentMeta";
import type { ContentBlock } from "../../types";

const props = defineProps<{
  agentName: string;
  modelName: string;
  agentMeta: AgentMeta | null;
}>();

const emit = defineEmits<{
  send: [content: string, contentBlocks?: ContentBlock[]];
  stop: [];
  "use-prompt": [text: string];
  "create-agent": [];
}>();

// 是否有 Agent（控制 OnboardCard 折叠/展开）
const hasAgent = computed(() => !!props.agentName);

// Agent 摘要：头像文字 + 颜色
const agentSummary = computed(() => {
  if (!props.agentMeta) return null;
  return {
    avatarText: props.agentMeta.avatarText,
    avatarColor: props.agentMeta.avatarColor,
    avatarFg: props.agentMeta.avatarFg ?? "#FFFFFF",
  };
});

// 4 个场景卡片静态数据
const scenarios = [
  {
    id: "software",
    title: "软件开发",
    desc: "主 Agent + 审查 Agent",
    icon: Code,
    iconColor: "#4680C2",
    iconBg: "#E1EDF9",
  },
  {
    id: "research",
    title: "研究调研",
    desc: "搜索 + 总结 + 写作",
    icon: Search,
    iconColor: "#2D8B66",
    iconBg: "#DAEFEE",
  },
  {
    id: "content",
    title: "内容创作",
    desc: "大纲 + 文案 + 润色",
    icon: PenTool,
    iconColor: "#6B5BBA",
    iconBg: "#E5E0F2",
  },
  {
    id: "learning",
    title: "学习助手",
    desc: "讲解 + 测验 + 反馈",
    icon: GraduationCap,
    iconColor: "#B8862A",
    iconBg: "#FDF3DC",
  },
];

// 透传 send/stop 事件给父组件
function handleSend(content: string, contentBlocks?: ContentBlock[]): void {
  emit("send", content, contentBlocks);
}
function handleStop(): void {
  emit("stop");
}
function handleUsePrompt(text: string): void {
  emit("use-prompt", text);
}
function handleCreateAgent(): void {
  emit("create-agent");
}
</script>

<template>
  <div class="welcome-panel">
    <!-- 主背景光效 -->
    <div class="welcome-panel__bg-glow" aria-hidden="true" />

    <!-- 内容区 -->
    <div class="welcome-panel__content">

      <!-- 品牌 pill -->
      <div class="brand-pill">
        <PawBrandMark :size="22" :animated="false" />
        <span class="brand-pill__text">欢迎来到 IcePaw</span>
      </div>

      <!-- Hero 标题 -->
      <h1 class="hero-headline">
        建一个 Agent，开始<em>你的第一次</em>对话。
      </h1>

      <!-- 副标题 -->
      <p class="hero-subhead">
        IcePaw 是为多 Agent 项目协作设计的桌面工作站。先设置一个
        Agent，它会作为这个项目的第一位成员。
      </p>

      <!-- 步骤指示器 -->
      <div class="steps" role="list" aria-label="上手步骤">
        <div class="step step--active" role="listitem">
          <span class="step__num">1</span>
          <span class="step__label">建 Agent</span>
        </div>
        <div class="step-connector" aria-hidden="true" />
        <div class="step step--idle" role="listitem">
          <span class="step__num">2</span>
          <span class="step__label">选场景</span>
        </div>
        <div class="step-connector" aria-hidden="true" />
        <div class="step step--idle" role="listitem">
          <span class="step__num">3</span>
          <span class="step__label">开始对话</span>
        </div>
      </div>

      <!-- OnboardCard：双栏 grid -->
      <div class="onboard-card">
        <!-- 左栏：AgentForm -->
        <div class="onboard-form">
          <div class="card-eyebrow">Step 1 · 建 Agent</div>
          <h2 class="card-headline">
            给这位伙伴<em>起个名字</em>，并接上模型。
          </h2>

          <!-- 已有 Agent：折叠为摘要行 -->
          <template v-if="hasAgent && agentSummary">
            <div class="agent-summary">
              <span
                class="agent-summary__avatar"
                :style="{ backgroundColor: agentSummary.avatarColor, color: agentSummary.avatarFg }"
              >
                {{ agentSummary.avatarText }}
              </span>
              <span class="agent-summary__name">{{ agentName }}</span>
              <span class="agent-summary__model">{{ modelName }}</span>
              <button
                class="agent-summary__switch"
                type="button"
                @click="handleCreateAgent"
              >
                切换 Agent →
              </button>
            </div>
          </template>

          <!-- 无 Agent：展示引导文案（实际创建由 create-agent emit 触发） -->
          <template v-else>
            <p class="form-hint">
              还没有 Agent？点击下方按钮开始创建。
            </p>
            <button
              class="btn-create-agent"
              type="button"
              @click="handleCreateAgent"
            >
              + 创建 Agent
            </button>
          </template>
        </div>

        <!-- 右栏：ScenarioGrid 预览 -->
        <div class="onboard-preview">
          <div class="card-eyebrow">Step 2 · 选场景</div>
          <div class="scenario-grid">
            <button
              v-for="scenario in scenarios"
              :key="scenario.id"
              class="scenario-card"
              type="button"
              @click="handleUsePrompt(scenario.desc)"
            >
              <span
                class="scenario-card__icon-wrap"
                :style="{ background: scenario.iconBg }"
              >
                <component
                  :is="scenario.icon"
                  :size="20"
                  :color="scenario.iconColor"
                  :stroke-width="2"
                  aria-hidden="true"
                />
              </span>
              <span class="scenario-card__title">{{ scenario.title }}</span>
              <span class="scenario-card__desc">{{ scenario.desc }}</span>
            </button>
          </div>
          <p class="preview-hint">选择一个场景开始，或直接输入你的需求</p>
        </div>
      </div>

      <!-- 输入区（复用 WelcomeInput） -->
      <div class="welcome-input-wrapper">
        <WelcomeInput
          :agentName="agentName"
          :modelName="modelName"
          @send="handleSend"
          @stop="handleStop"
        />
      </div>

      <!-- PawTrail 装饰条 -->
      <div class="paw-trail-wrapper">
        <PawTrail />
      </div>

    </div>
  </div>
</template>

<style scoped>
/* ---------------------------------------------------------------------------
   根容器
   --------------------------------------------------------------------------- */
.welcome-panel {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 100%;
  min-height: 100%;
  overflow: hidden;
}

/* 主背景光效 */
.welcome-panel__bg-glow {
  position: absolute;
  inset: 0;
  background: radial-gradient(
    ellipse 1200px 800px at 80% -10%,
    rgba(70, 128, 194, 0.06),
    transparent 60%
  );
  pointer-events: none;
  z-index: 0;
}

/* 内容区 */
.welcome-panel__content {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 100%;
  max-width: 800px;
  padding: var(--ip-spacing-10) var(--ip-spacing-6) var(--ip-spacing-8);
  gap: var(--ip-spacing-6);
}

/* ---------------------------------------------------------------------------
   Brand pill
   --------------------------------------------------------------------------- */
.brand-pill {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-2) var(--ip-spacing-4);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-2xl);
  box-shadow: var(--ip-shadow-sm);
  transition: box-shadow var(--ip-duration-fast) var(--ip-ease-out);
}
.brand-pill:hover {
  box-shadow: var(--ip-shadow-md);
}
.brand-pill__text {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-secondary);
  letter-spacing: 0.01em;
}

/* ---------------------------------------------------------------------------
   Hero 标题
   --------------------------------------------------------------------------- */
.hero-headline {
  font-family: var(--ip-font-display);
  font-size: clamp(2rem, 4.2vw, 3.4rem);
  font-weight: 400;
  line-height: 1.05;
  letter-spacing: -0.025em;
  color: var(--ip-color-text-primary);
  text-wrap: balance;
  text-align: center;
  margin: 0;
}
.hero-headline em {
  font-style: italic;
  color: var(--ip-primary-600);
}

.hero-subhead {
  font-family: var(--ip-font-sans);
  font-size: var(--ip-text-body-md-size);
  line-height: 1.65;
  color: var(--ip-color-text-secondary);
  text-align: center;
  max-width: 540px;
  margin: 0;
}

/* ---------------------------------------------------------------------------
   步骤指示器
   --------------------------------------------------------------------------- */
.steps {
  display: flex;
  align-items: center;
  gap: 0;
}
.step {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-1) var(--ip-spacing-3);
  border-radius: var(--ip-radius-full);
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
  cursor: default;
}
.step--active {
  background: var(--ip-primary-500);
  color: var(--ip-white);
  box-shadow: 0 4px 12px rgba(70, 128, 194, 0.25);
}
.step--idle {
  background: var(--ip-white);
  border: 1px solid var(--ip-gray-200);
  color: var(--ip-color-text-tertiary);
}
.step--done {
  background: var(--ip-white);
  color: var(--ip-color-text-secondary);
}
.step__num {
  font-family: var(--ip-font-mono);
  font-size: 12px;
  font-weight: 600;
  width: 18px;
  height: 18px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.25);
}
.step--idle .step__num {
  background: var(--ip-gray-100);
}
.step__label {
  white-space: nowrap;
}
.step-connector {
  width: 28px;
  height: 1px;
  background: var(--ip-gray-300);
  flex-shrink: 0;
}

/* ---------------------------------------------------------------------------
   OnboardCard
   --------------------------------------------------------------------------- */
.onboard-card {
  display: grid;
  grid-template-columns: 1.2fr 1fr;
  width: 100%;
  min-height: 420px;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-3xl);
  box-shadow: var(--ip-shadow-xl);
  overflow: hidden;
}

.onboard-form {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-4);
  padding: var(--ip-spacing-8);
  border-right: 1px solid var(--ip-color-border-default);
}

.card-eyebrow {
  font-size: 12.5px;
  color: var(--ip-color-text-tertiary);
  font-weight: var(--ip-font-weight-medium);
  letter-spacing: 0.02em;
}

.card-headline {
  font-family: var(--ip-font-display);
  font-size: 22px;
  font-weight: 400;
  line-height: 1.3;
  color: var(--ip-color-text-primary);
  margin: 0;
  letter-spacing: -0.015em;
}
.card-headline em {
  font-style: italic;
  color: var(--ip-primary-600);
}

/* Agent 摘要行 */
.agent-summary {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-3);
  background: var(--ip-gray-50);
  border-radius: var(--ip-radius-lg);
  border: 1px solid var(--ip-color-border-default);
  flex-wrap: wrap;
}
.agent-summary__avatar {
  width: 32px;
  height: 32px;
  border-radius: var(--ip-radius-md);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 13px;
  font-weight: 700;
  flex-shrink: 0;
}
.agent-summary__name {
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  font-size: var(--ip-text-body-sm-size);
}
.agent-summary__model {
  font-family: var(--ip-font-mono);
  font-size: 12px;
  color: var(--ip-color-text-tertiary);
  background: var(--ip-gray-100);
  padding: 2px 6px;
  border-radius: var(--ip-radius-sm);
}
.agent-summary__switch {
  margin-left: auto;
  background: none;
  border: none;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-primary-600);
  cursor: pointer;
  padding: 0;
  white-space: nowrap;
  transition: color var(--ip-duration-fast);
}
.agent-summary__switch:hover {
  color: var(--ip-primary-700);
  text-decoration: underline;
}

.form-hint {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  margin: 0;
  line-height: 1.6;
}

.btn-create-agent {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: var(--ip-spacing-2) var(--ip-spacing-4);
  background: var(--ip-primary-500);
  color: var(--ip-white);
  border: none;
  border-radius: var(--ip-radius-lg);
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
  box-shadow: 0 2px 8px rgba(70, 128, 194, 0.25);
  width: fit-content;
}
.btn-create-agent:hover {
  background: var(--ip-primary-600);
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(70, 128, 194, 0.3);
}
.btn-create-agent:active {
  transform: translateY(0);
}

/* 右栏预览 */
.onboard-preview {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-4);
  padding: var(--ip-spacing-8);
  background: linear-gradient(180deg, var(--ip-gray-50) 0%, var(--ip-primary-50) 100%);
}

.scenario-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--ip-spacing-3);
  flex: 1;
}

.scenario-card {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-3);
  background: var(--ip-white);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-xl);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
  text-align: left;
  font-family: inherit;
}
.scenario-card:hover {
  border-color: var(--ip-primary-400);
  transform: translateY(-1px);
  box-shadow: var(--ip-shadow-md);
}
.scenario-card:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
  border-color: var(--ip-primary-500);
}
.scenario-card__icon-wrap {
  width: 36px;
  height: 36px;
  border-radius: var(--ip-radius-md);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.scenario-card__title {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  line-height: 1.3;
}
.scenario-card__desc {
  font-size: 11.5px;
  color: var(--ip-color-text-tertiary);
  line-height: 1.4;
}

.preview-hint {
  font-size: 12px;
  color: var(--ip-color-text-tertiary);
  margin: 0;
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
}

/* ---------------------------------------------------------------------------
   输入区
   --------------------------------------------------------------------------- */
.welcome-input-wrapper {
  width: 100%;
}

/* ---------------------------------------------------------------------------
   PawTrail 装饰
   --------------------------------------------------------------------------- */
.paw-trail-wrapper {
  display: flex;
  justify-content: center;
  padding-top: var(--ip-spacing-2);
}

/* ---------------------------------------------------------------------------
   响应式：tablet (≤1023px)
   --------------------------------------------------------------------------- */
@media (max-width: 1023px) {
  .onboard-card {
    grid-template-columns: 1fr;
    min-height: auto;
  }
  .onboard-form {
    border-right: none;
    border-bottom: 1px solid var(--ip-color-border-default);
  }
}

/* ---------------------------------------------------------------------------
   响应式：mobile (≤767px)
   --------------------------------------------------------------------------- */
@media (max-width: 767px) {
  .welcome-panel__content {
    padding: var(--ip-spacing-6) var(--ip-spacing-4) var(--ip-spacing-6);
    gap: var(--ip-spacing-4);
  }
  .steps {
    display: none;
  }
  .scenario-grid {
    grid-template-columns: 1fr 1fr;
  }
}
</style>
