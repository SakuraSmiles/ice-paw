<script setup lang="ts">
// 聊天页头部
//
// 职责：
//   - 标题左侧显示 Agent 小头像（20x20，字母缩写 / Lucide 图标）
//   - 显示当前会话标题 / Agent 名 + 模型名
//   - 模型名区域为可点击下拉（P0-3），用于会话内临时切换模型
//   - 右侧：流式中显示「停止」按钮（lucide Square 图标）
//   - 半透明毛玻璃背景（backdrop-filter: blur(8px) + 半透明底色）
//   - 高度 48px，颜色全部走 --ip-* Design Token
//
// P0-3 模型切换（Phase 1）：
//   - 只覆盖当前会话的 `model` 字段，不切 Agent
//   - 下拉展示当前 Agent provider 支持的常见模型列表
//   - 选中后下次 sendMessage 即生效
//   - 切会话时 store 自动清空 override（无需手动管理）
//
// props: 无（直接读 store）
//
// emits:
//   - stop  点击停止按钮时触发（外层接住后调 chatStore.stopGeneration）

import { computed, onBeforeUnmount, ref } from "vue";
import { Square, Settings, ChevronDown } from "lucide-vue-next";
import { useAgentsStore } from "../../stores/agents";
import { useProjectsStore } from "../../stores/projects";
import { useConversationsStore } from "../../stores/conversations";
import { useChatStore } from "../../stores/chat";
import { useAgentMeta, type AgentMeta } from "../../composables/useAgentMeta";
import { useRouter } from "vue-router";
import AgentAvatar from "../common/AgentAvatar.vue";

const agentsStore = useAgentsStore();
const projectsStore = useProjectsStore();
const conversationsStore = useConversationsStore();
const chatStore = useChatStore();
const agentMeta = useAgentMeta();
const router = useRouter();

const emit = defineEmits<{
  stop: [];
}>();

/** 当前项目名（Phase 2: 面包屑导航用） */
const projectName = computed<string>(() => {
  return projectsStore.current?.name ?? "默认项目";
});

/** 当前会话标题（无则显示「新会话」） */
const convTitle = computed<string>(() => {
  return conversationsStore.current?.title?.trim() || "新会话";
});

/** 当前 Agent 名（无则显示「未选择 Agent」） */
const agentName = computed<string>(() => agentsStore.current?.name ?? "未选择 Agent");

/** 当前 Agent 模型（无则空串） */
const agentModel = computed<string>(() => agentsStore.current?.model ?? "");

/** 当前实际生效的 model（override 优先于 Agent 默认） */
const currentModel = computed<string>(
  () => chatStore.modelOverride ?? agentModel.value,
);

/** 模型下拉是否展开 */
const showModelList = ref(false);

/** 模型下拉的 ref（用于 click-outside 检测） */
const modelSelectorRef = ref<HTMLElement | null>(null);

/**
 * P0-3: 候选模型列表（Phase 1 简化策略）
 *
 * - 优先用当前 Agent 的 `provider` 字段匹配已知 provider 的常见模型
 * - 当前 Agent 类型不支持自定义 supported_models 字段，所以这里走硬编码兜底
 * - 列表第一个选项始终为「跟随 Agent 默认」（label 用当前 Agent 的 model）
 *
 * 后续 A3-N 在 Agent 表加 supported_models 字段后，此处可平滑替换为：
 *   `(agent.supported_models ?? []).map(...)`
 */
interface ModelOption {
  label: string;
  value: string | null; // null = 清除 override（用 Agent 默认）
  isDefault?: boolean;
}

const FALLBACK_MODELS_BY_PROVIDER: Record<string, { label: string; value: string }[]> = {
  // OpenAI / 兼容 OpenAI Chat Completions 的服务
  openai: [
    { label: "GPT-4o", value: "gpt-4o" },
    { label: "GPT-4o mini", value: "gpt-4o-mini" },
    { label: "GPT-4 Turbo", value: "gpt-4-turbo" },
    { label: "GPT-3.5 Turbo", value: "gpt-3.5-turbo" },
    { label: "o1", value: "o1" },
    { label: "o1-mini", value: "o1-mini" },
    { label: "o3-mini", value: "o3-mini" },
  ],
  // Anthropic / 兼容 Anthropic Messages API 的服务
  anthropic: [
    { label: "Claude Opus 4", value: "claude-opus-4-20250514" },
    { label: "Claude Sonnet 4", value: "claude-sonnet-4-20250514" },
    { label: "Claude 3.5 Sonnet", value: "claude-3-5-sonnet-20241022" },
    { label: "Claude 3.5 Haiku", value: "claude-3-5-haiku-20241022" },
  ],
  // MiniMax 自研（Anthropic 兼容协议）
  "minimax-cn": [
    { label: "MiniMax-M2.5", value: "MiniMax-M2.5" },
    { label: "MiniMax-M3", value: "MiniMax-M3" },
  ],
  // 智谱 GLM（OpenAI 兼容）
  glm: [
    { label: "GLM-4 Plus", value: "glm-4-plus" },
    { label: "GLM-4 Flash", value: "glm-4-flash" },
  ],
  // DeepSeek（OpenAI 兼容）
  deepseek: [
    { label: "DeepSeek V3", value: "deepseek-chat" },
    { label: "DeepSeek R1", value: "deepseek-reasoner" },
  ],
};

const availableModels = computed<ModelOption[]>(() => {
  const agent = agentsStore.current;
  const providerKey = agent?.provider?.toLowerCase() ?? "";
  const candidates =
    FALLBACK_MODELS_BY_PROVIDER[providerKey] ?? FALLBACK_MODELS_BY_PROVIDER["openai"];

  // 第一个固定为「跟随 Agent 默认」
  const opts: ModelOption[] = [];
  if (agent?.model) {
    opts.push({
      label: `跟随 Agent（${agent.model}）`,
      value: null,
      isDefault: true,
    });
  }
  for (const c of candidates) {
    // 跳过与 Agent 默认重复的项（避免下拉里出现两个相同的 model）
    if (c.value === agent?.model) continue;
    opts.push({ label: c.label, value: c.value });
  }
  return opts;
});

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

/** 是否存在可切换的模型（无可用候选时整个下拉入口隐藏） */
const hasModelChoices = computed<boolean>(() => availableModels.value.length > 1);

/** 当前 model 是否为 override（用于下拉项高亮判断） */
const isOverriding = computed<boolean>(() => chatStore.modelOverride !== null);

/** 停止按钮点击 */
function onStop(): void {
  emit("stop");
}

function openSettings(): void {
  void router.push("/settings/general");
}

/** P0-3: 切换 model override */
function selectModel(value: string | null): void {
  chatStore.setModelOverride(value);
  showModelList.value = false;
}

/** P0-3: 切换下拉显隐 */
function toggleModelList(): void {
  if (!hasModelChoices.value) return;
  showModelList.value = !showModelList.value;
}

/** P0-3: 全局点击关闭下拉 */
function handleDocumentClick(e: MouseEvent): void {
  if (!showModelList.value) return;
  const root = modelSelectorRef.value;
  if (root && !root.contains(e.target as Node)) {
    showModelList.value = false;
  }
}

/** 注册 / 注销全局点击监听 */
if (typeof window !== "undefined") {
  window.addEventListener("click", handleDocumentClick);
  onBeforeUnmount(() => {
    window.removeEventListener("click", handleDocumentClick);
  });
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
        <!-- Phase 2: 面包屑导航 项目名 / 会话标题 -->
        <span class="project-breadcrumb">{{ projectName }}</span>
        <span class="breadcrumb-sep">/</span>
        <span class="conv-title">{{ headerTitle }}</span>
      </div>
      <!-- P0-3: 模型下拉选择器 -->
      <div v-if="agentModel" ref="modelSelectorRef" class="model-selector">
        <button
          type="button"
          class="model-selector-trigger"
          :class="{ open: showModelList, disabled: !hasModelChoices }"
          :disabled="!hasModelChoices"
          :title="hasModelChoices ? '点击切换会话模型' : '当前 Agent 无可切换模型'"
          aria-haspopup="listbox"
          :aria-expanded="showModelList"
          @click.stop="toggleModelList"
        >
          <span class="model-name" :class="{ overriding: isOverriding }">
            {{ currentModel }}
          </span>
          <ChevronDown
            v-if="hasModelChoices"
            :size="12"
            class="chevron"
            :class="{ open: showModelList }"
            aria-hidden="true"
          />
        </button>
        <ul v-if="showModelList && hasModelChoices" class="model-dropdown" role="listbox">
          <li
            v-for="m in availableModels"
            :key="m.value ?? '__default__'"
            role="option"
            :aria-selected="currentModel === (m.value ?? agentModel)"
          >
            <button
              type="button"
              class="model-option"
              :class="{
                active: m.isDefault
                  ? !isOverriding
                  : m.value === chatStore.modelOverride,
                default: m.isDefault,
              }"
              @click.stop="selectModel(m.value)"
            >
              <span class="model-option-label">{{ m.label }}</span>
              <span v-if="m.isDefault" class="model-option-badge">默认</span>
            </button>
          </li>
        </ul>
      </div>
    </div>
    <div class="header-actions">
      <button
        class="btn-settings"
        type="button"
        title="设置"
        aria-label="设置"
        @click="openSettings"
      >
        <Settings :size="16" aria-hidden="true" />
      </button>
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
  overflow: visible;
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

/* Phase 2: 面包屑导航 */
.project-breadcrumb {
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  flex-shrink: 0;
}

.breadcrumb-sep {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  opacity: 0.5;
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

/* P0-3: 模型下拉选择器 */
.model-selector {
  position: relative;
  display: inline-flex;
  align-items: center;
  font-size: var(--ip-text-caption-size);
  line-height: 1.2;
  color: var(--ip-color-text-tertiary);
  min-width: 0;
}

.model-selector-trigger {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  padding: 2px var(--ip-spacing-2);
  margin: 0;
  border: 1px solid transparent;
  border-radius: var(--ip-radius-sm);
  background: transparent;
  color: inherit;
  font: inherit;
  cursor: pointer;
  transition: var(--ip-transition-colors);
  max-width: 320px;
  min-width: 0;
}

.model-selector-trigger:hover:not(.disabled):not(:disabled) {
  background-color: var(--ip-color-bg-hover);
  border-color: var(--ip-color-border-default);
}

.model-selector-trigger.open {
  background-color: var(--ip-color-bg-hover);
  border-color: var(--ip-color-border-default);
}

.model-selector-trigger:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.model-selector-trigger.disabled,
.model-selector-trigger:disabled {
  cursor: default;
  opacity: 0.85;
}

.model-name {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-caption-size);
  letter-spacing: var(--ip-letter-spacing-normal);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 280px;
  min-width: 0;
}

/* P0-3: 覆盖态强调（与 Agent 默认值不同时） */
.model-name.overriding {
  color: var(--ip-primary-600);
  font-weight: var(--ip-font-weight-medium);
}

.chevron {
  flex-shrink: 0;
  opacity: 0.6;
  transition: var(--ip-transition-transform);
}

.chevron.open {
  transform: rotate(180deg);
}

/* P0-3: 下拉列表 */
.model-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  z-index: var(--ip-z-popover);
  margin: 0;
  padding: var(--ip-spacing-1);
  list-style: none;
  background-color: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-lg);
  min-width: 240px;
  max-width: 360px;
  max-height: 320px;
  overflow-y: auto;
}

.model-dropdown li {
  margin: 0;
  padding: 0;
}

.model-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-2);
  width: 100%;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border: none;
  border-radius: var(--ip-radius-sm);
  background: transparent;
  color: var(--ip-color-text-primary);
  font: inherit;
  font-size: var(--ip-text-body-sm-size);
  text-align: left;
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.model-option:hover {
  background-color: var(--ip-color-bg-hover);
}

.model-option.active {
  background-color: var(--ip-color-selection-bg);
  color: var(--ip-primary-600);
  font-weight: var(--ip-font-weight-medium);
}

.model-option-label {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
  flex: 1;
}

.model-option-badge {
  flex-shrink: 0;
  padding: 1px var(--ip-spacing-1_5);
  border-radius: var(--ip-radius-sm);
  background-color: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  letter-spacing: 0.02em;
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

.btn-settings {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border: none;
  border-radius: var(--ip-radius-md);
  background: transparent;
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.btn-settings:hover {
  background-color: var(--ip-color-bg-hover);
  color: var(--ip-color-text-primary);
}

.btn-stop:active {
  background: var(--ip-danger-active);
  border-color: var(--ip-danger-active);
}

.btn-label {
  line-height: 1;
}
</style>