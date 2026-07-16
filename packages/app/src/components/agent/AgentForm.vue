<script setup lang="ts">
// Agent 创建/编辑表单 — 侧滑面板
//
// 职责：
//   - create 模式：顶部展示模板选择器 → 点选后自动填充 name / system_prompt / temperature
//   - edit   模式：不展示模板选择器，直接编辑现有 Agent
//   - 字段顺序（按方案 v2 4.1）：
//       [模板选择器] → Name → [角色设定 System Prompt] → Provider → Model → API Key → [高级折叠：Base URL / Temperature / Max Tokens]
//   - 创建 / 编辑成功后由父组件（AgentManagerPage）根据 payload.templateId 写入 agentMeta
//
// props:
//   - mode:   "create" | "edit"
//   - agent:  编辑模式下的目标 Agent
//   - open:   面板是否显示
//
// emits:
//   - update:open  关闭面板
//   - submit       提交表单，payload 字段见 AgentFormPayload

import { ref, computed, watch } from "vue";
import { Input, Textarea, Button } from "@ice-paw/ui";
import { X, Sparkles, ChevronDown } from "lucide-vue-next";
import type { Agent } from "../../types";
import {
  AGENT_TEMPLATES,
  type AgentTemplate,
} from "../../data/agentTemplates";
import { saturatedToBgFg } from "../../utils/agentAvatar";

// 顺序即默认下拉顺序；首位 = 新建 Agent 的默认 Provider。
// GLM 保留为可选 Provider（不推荐），以兼容已有 GLM Agent 配置。
const PROVIDERS = ["MiniMax", "OpenAI", "Anthropic", "DeepSeek", "GLM"] as const;
type ProviderName = (typeof PROVIDERS)[number];

const MODEL_PRESETS: Record<ProviderName, string[]> = {
  MiniMax: ["minimax-cn/M3"],
  OpenAI: ["gpt-4o", "gpt-4o-mini"],
  Anthropic: ["claude-sonnet-4-20250514"],
  DeepSeek: ["deepseek-chat", "deepseek-reasoner"],
  // 保留 GLM 选项仅供已有 Agent 兼容；新建不再推荐。
  GLM: ["glm-5.2", "glm-4.7", "glm-4-flash"],
};

/**
 * 表单提交 payload。
 * 新增 `templateId` 字段（仅 create 模式可能存在），
 *   父组件在创建成功后根据该字段写入 agentMeta 到 localStorage。
 */
export interface AgentFormPayload {
  name: string;
  provider: string;
  model: string;
  api_key: string;
  base_url: string;
  system_prompt: string;
  temperature: number;
  max_tokens: number;
  rotateApiKey: boolean;
  /** create 模式下用户选中的模板 id；未选模板或 edit 模式下为 undefined */
  templateId?: string;
  /** P2-3: 是否启用 prompt caching（默认 true） */
  cachePrompt?: boolean;
  /**
   * A3-2: 历史消息窗口上限。
   * - `undefined`（留空）→ 使用系统默认
   * - 正整数 N → 最多加载最近 N 条历史消息注入 LLM 上下文
   */
  maxHistoryMessages?: number | null;
}

const props = defineProps<{
  mode: "create" | "edit";
  agent: Agent | null;
  open: boolean;
}>();

const emit = defineEmits<{
  "update:open": [value: boolean];
  submit: [payload: AgentFormPayload];
}>();

// ============================================================================
// 表单状态
// ============================================================================

const name = ref("");
const provider = ref<string>(PROVIDERS[0]);
const model = ref("");
const customModel = ref("");
const useCustomModel = ref(false);
const apiKey = ref("");
const rotateApiKey = ref(false);
const baseUrl = ref("");
const systemPrompt = ref("");
const temperature = ref(0.7);
const maxTokens = ref(4096);
/** P2-3: 是否启用 prompt caching（默认 true） */
const cachePrompt = ref(true);
/**
 * A3-2: 历史消息窗口上限（用户输入字符串，留空 = 系统默认）。
 * 后端 AgentRow.max_history_messages 是 Option<i32>，前端用字符串方便
 * 表达「未填写」状态（空串），提交时再转为 number | null。
 */
const maxHistoryMessagesInput = ref<string>("");

/** 当前选中的模板 id（create 模式才有意义） */
const selectedTemplateId = ref<string | null>(null);

const errors = ref<Record<string, string>>({});

/** 高级设置是否展开（默认折叠） */
const advancedOpen = ref<boolean>(false);

// ============================================================================
// 派生
// ============================================================================

const presetModels = computed<string[]>(() => {
  return MODEL_PRESETS[provider.value as ProviderName] ?? [];
});

const modelValue = computed(() => {
  return useCustomModel.value ? customModel.value : model.value;
});

const panelTitle = computed(() => {
  return props.mode === "create" ? "新建 Agent" : "编辑 Agent";
});

const nameError = computed(() => errors.value.name ?? "");
const apiKeyError = computed(() => errors.value.api_key ?? "");

/** P2-3: 当前 provider 是否为 Anthropic（仅 Anthropic 支持显式 cache_control 断点） */
const isAnthropicProvider = computed<boolean>(() => {
  return provider.value.toLowerCase() === "anthropic";
});

/**
 * 高级设置是否需要「默认展开」：
 *   - edit 模式，且存在任一非默认值时展开，便于用户直接修改
 */
const advancedDefaultOpen = computed<boolean>(() => {
  if (props.mode !== "edit") return false;
  return (
    baseUrl.value.trim().length > 0 ||
    Math.abs(temperature.value - 0.7) > 0.001 ||
    maxTokens.value !== 4096 ||
    maxHistoryMessagesInput.value.trim().length > 0
  );
});

// ============================================================================
// 联动
// ============================================================================

watch(provider, () => {
  model.value = presetModels.value[0] ?? "";
  useCustomModel.value = false;
  customModel.value = "";
});

watch(
  () => props.open,
  (val) => {
    if (!val) return;
    resetForm();
    if (props.mode === "edit" && props.agent) {
      populateFromAgent(props.agent);
    }
  },
);

// ============================================================================
// 模板选择
// ============================================================================

/**
 * 处理模板选择。
 * 点选后自动填充 name / system_prompt / temperature / provider / model，
 * 并把模板 id 写入 selectedTemplateId（用于提交后写 meta）。
 */
function applyTemplate(tpl: AgentTemplate): void {
  // 再次点击同一模板 = 取消选择
  if (selectedTemplateId.value === tpl.id) {
    selectedTemplateId.value = null;
    return;
  }
  selectedTemplateId.value = tpl.id;
  name.value = tpl.name;
  systemPrompt.value = tpl.systemPrompt;
  temperature.value = tpl.temperature;

  // 若推荐 provider 在可选列表中则预填
  const matched = PROVIDERS.find(
    (p) => p.toLowerCase() === tpl.recommendedProvider.toLowerCase(),
  );
  if (matched) {
    provider.value = matched;
  }

  // 预填 model（若在 preset 列表中）
  const presets = MODEL_PRESETS[provider.value as ProviderName] ?? [];
  if (presets.includes(tpl.recommendedModel)) {
    useCustomModel.value = false;
    model.value = tpl.recommendedModel;
  } else {
    useCustomModel.value = true;
    customModel.value = tpl.recommendedModel;
  }
}

/** 取消模板选择 */
function clearTemplate(): void {
  selectedTemplateId.value = null;
}

// ============================================================================
// 表单重置 / 回填
// ============================================================================

function resetForm(): void {
  name.value = "";
  provider.value = PROVIDERS[0];
  model.value = presetModels.value[0] ?? "";
  customModel.value = "";
  useCustomModel.value = false;
  apiKey.value = "";
  rotateApiKey.value = false;
  baseUrl.value = "";
  systemPrompt.value = "";
  temperature.value = 0.7;
  maxTokens.value = 4096;
  cachePrompt.value = true;
  maxHistoryMessagesInput.value = "";
  selectedTemplateId.value = null;
  advancedOpen.value = false;
  errors.value = {};
}

function populateFromAgent(a: Agent): void {
  name.value = a.name;
  const match = PROVIDERS.find((p) => p.toLowerCase() === a.provider.toLowerCase());
  provider.value = match ?? PROVIDERS[0];
  const presets = MODEL_PRESETS[provider.value as ProviderName] ?? [];
  if (presets.includes(a.model)) {
    model.value = a.model;
    useCustomModel.value = false;
  } else {
    customModel.value = a.model;
    useCustomModel.value = true;
  }
  apiKey.value = "";
  rotateApiKey.value = false;
  baseUrl.value = a.base_url ?? "";
  systemPrompt.value = a.system_prompt ?? "";
  temperature.value = a.temperature;
  maxTokens.value = a.max_tokens;
  // P2-3: 读取缓存设置
  cachePrompt.value = a.cache_prompt;
  // A3-2: 读取历史窗口（null/undefined → 空串 = 系统默认）
  maxHistoryMessagesInput.value =
    a.max_history_messages != null ? String(a.max_history_messages) : "";
  selectedTemplateId.value = null;
  advancedOpen.value = advancedDefaultOpen.value;
}

// ============================================================================
// 提交
// ============================================================================

function validate(): boolean {
  const errs: Record<string, string> = {};
  if (!name.value.trim()) {
    errs.name = "名称不能为空";
  }
  if (props.mode === "create") {
    if (!apiKey.value.trim()) {
      errs.api_key = "API Key 不能为空";
    }
  } else if (rotateApiKey.value) {
    if (!apiKey.value.trim()) {
      errs.api_key = "请输入新的 API Key";
    }
  }
  errors.value = errs;
  return Object.keys(errs).length === 0;
}

/**
 * 把字符串输入解析成 number | null | undefined：
 * - 空串 → undefined（让 Rust 侧保持原值 / 走默认）
 * - 合法正整数 N → number
 * - 非法值 → 不提交 maxHistoryMessages 字段（兜底）
 */
function parseMaxHistoryInput(): number | null | undefined {
  const raw = maxHistoryMessagesInput.value.trim();
  if (raw === "") return undefined;
  const n = Number(raw);
  if (!Number.isFinite(n) || n <= 0 || !Number.isInteger(n)) {
    // 非法输入 → 不发送该字段，由 Rust 侧保持原值/默认值
    return undefined;
  }
  // 安全上限：防止误输入超大值（例如 1e9）撑爆上下文
  const MAX_ALLOWED = 1000;
  return Math.min(n, MAX_ALLOWED);
}

function handleSubmit(): void {
  if (!validate()) return;
  emit("submit", {
    name: name.value.trim(),
    provider: provider.value.toLowerCase(),
    model: modelValue.value.trim(),
    api_key: apiKey.value.trim(),
    base_url: baseUrl.value.trim(),
    system_prompt: systemPrompt.value.trim(),
    temperature: temperature.value,
    max_tokens: maxTokens.value,
    rotateApiKey: rotateApiKey.value,
    templateId: selectedTemplateId.value ?? undefined,
    cachePrompt: cachePrompt.value,
    maxHistoryMessages: parseMaxHistoryInput(),
  });
}

function close(): void {
  emit("update:open", false);
}

/** 高级设置折叠/展开切换 */
function onAdvancedToggle(e: Event): void {
  const target = e.target as HTMLElement & { open?: boolean };
  advancedOpen.value = Boolean(target?.open);
}

// ============================================================================
// 模板卡片视觉辅助
// ============================================================================

/** 模板卡片的 bg/fg 配对 */
function templateBgFg(tpl: AgentTemplate): { bg: string; fg: string } {
  return saturatedToBgFg(tpl.color);
}
</script>

<template>
  <Teleport to="body">
    <Transition name="slide">
      <div v-if="open" class="panel-overlay" @click.self="close">
        <div class="panel">
          <header class="panel-header">
            <h3 class="panel-title">{{ panelTitle }}</h3>
            <Button
              variant="ghost"
              size="sm"
              icon-only
              type="button"
              title="关闭"
              aria-label="关闭"
              @click="close"
            >
              <X :size="16" aria-hidden="true" />
            </Button>
          </header>

          <div class="panel-body">
            <form @submit.prevent="handleSubmit">
              <!-- 模板选择器（仅 create 模式） -->
              <section v-if="mode === 'create'" class="template-section">
                <div class="section-header">
                  <Sparkles :size="14" class="section-icon" aria-hidden="true" />
                  <span class="section-title">从模板开始</span>
                  <button
                    v-if="selectedTemplateId"
                    type="button"
                    class="clear-link"
                    @click="clearTemplate"
                  >
                    清除选择
                  </button>
                </div>
                <div class="template-grid" role="radiogroup" aria-label="Agent 模板">
                  <button
                    v-for="tpl in AGENT_TEMPLATES"
                    :key="tpl.id"
                    type="button"
                    role="radio"
                    :aria-checked="selectedTemplateId === tpl.id"
                    :class="[
                      'template-card',
                      {
                        'template-card-active': selectedTemplateId === tpl.id,
                      },
                    ]"
                    :style="
                      selectedTemplateId === tpl.id
                        ? {
                            backgroundColor: templateBgFg(tpl).bg,
                            borderColor: templateBgFg(tpl).fg,
                          }
                        : {}
                    "
                    @click="applyTemplate(tpl)"
                  >
                    <span
                      class="template-icon"
                      :style="{
                        backgroundColor: templateBgFg(tpl).bg,
                        color: templateBgFg(tpl).fg,
                      }"
                    >
                      <component :is="tpl.icon" :size="18" aria-hidden="true" />
                    </span>
                    <span class="template-text">
                      <span class="template-name">{{ tpl.name }}</span>
                      <span class="template-desc">{{ tpl.description }}</span>
                    </span>
                  </button>
                </div>
              </section>

              <!-- 名称 -->
              <div class="form-group">
                <label class="form-label" for="agent-name">名称 *</label>
                <Input
                  id="agent-name"
                  v-model="name"
                  size="md"
                  placeholder="例如：论文润色助手"
                  autocomplete="off"
                  :error="Boolean(nameError)"
                  :error-message="nameError"
                />
              </div>

              <!-- 角色设定（system_prompt） -->
              <div class="form-group">
                <label class="form-label" for="agent-systemprompt">
                  角色设定
                  <span class="label-hint">描述这个助手的角色、能力和回答风格</span>
                </label>
                <Textarea
                  id="agent-systemprompt"
                  v-model="systemPrompt"
                  size="md"
                  :rows="5"
                  placeholder="例如：你是一个耐心的代码导师，擅长用通俗易懂的方式解释技术概念..."
                />
              </div>

              <!-- Provider -->
              <div class="form-group">
                <label class="form-label" for="agent-provider">Provider *</label>
                <select id="agent-provider" v-model="provider" class="form-select">
                  <option v-for="p in PROVIDERS" :key="p" :value="p">{{ p }}</option>
                </select>
              </div>

              <!-- Model -->
              <div class="form-group">
                <label class="form-label" for="agent-model">Model *</label>
                <div v-if="!useCustomModel" class="model-row">
                  <select
                    id="agent-model"
                    v-model="model"
                    class="form-select form-select-model"
                  >
                    <option v-for="m in presetModels" :key="m" :value="m">{{ m }}</option>
                  </select>
                  <Button
                    variant="ghost"
                    size="sm"
                    type="button"
                    @click="useCustomModel = true"
                  >
                    自定义
                  </Button>
                </div>
                <div v-else class="model-row">
                  <Input
                    id="agent-model"
                    v-model="customModel"
                    size="md"
                    placeholder="输入模型名称"
                    autocomplete="off"
                  />
                  <Button
                    variant="ghost"
                    size="sm"
                    type="button"
                    @click="
                      useCustomModel = false;
                      customModel = '';
                    "
                  >
                    预设
                  </Button>
                </div>
              </div>

              <!-- API Key -->
              <div class="form-group">
                <label class="form-label" for="agent-apikey">
                  API Key *
                  <span v-if="mode === 'edit'" class="label-hint">
                    （当前已配置，留空保持不变）
                  </span>
                </label>
                <template v-if="mode === 'edit' && !rotateApiKey">
                  <div class="apikey-masked">
                    <span class="masked-text">••••••••••••••••</span>
                    <Button
                      variant="ghost"
                      size="sm"
                      type="button"
                      @click="rotateApiKey = true"
                    >
                      更换 Key
                    </Button>
                  </div>
                </template>
                <template v-else>
                  <Input
                    id="agent-apikey"
                    v-model="apiKey"
                    size="md"
                    type="password"
                    placeholder="sk-..."
                    autocomplete="off"
                    :error="Boolean(apiKeyError)"
                    :error-message="apiKeyError"
                  />
                </template>
              </div>

              <!-- 高级设置（折叠） -->
              <details
                :open="advancedOpen || advancedDefaultOpen"
                class="advanced-section"
                @toggle="onAdvancedToggle"
              >
                <summary class="advanced-summary">
                  <ChevronDown :size="14" class="advanced-chevron" aria-hidden="true" />
                  <span>高级设置</span>
                </summary>
                <div class="advanced-body">
                  <div class="form-group">
                    <label class="form-label" for="agent-baseurl">
                      Base URL
                      <span class="label-hint">（可选，留空使用默认地址）</span>
                    </label>
                    <Input
                      id="agent-baseurl"
                      v-model="baseUrl"
                      size="md"
                      placeholder="https://api.example.com/v1"
                      autocomplete="off"
                    />
                  </div>

                  <div class="form-group">
                    <label class="form-label" for="agent-temperature">
                      Temperature: {{ temperature.toFixed(1) }}
                    </label>
                    <input
                      id="agent-temperature"
                      v-model.number="temperature"
                      class="form-range"
                      type="range"
                      min="0"
                      max="2"
                      step="0.1"
                    />
                  </div>

                  <div class="form-group">
                    <label class="form-label" for="agent-maxtokens">Max Tokens</label>
                    <Input
                      id="agent-maxtokens"
                      :model-value="String(maxTokens)"
                      size="md"
                      type="number"
                      :maxlength="6"
                      @update:model-value="(v) => (maxTokens = Math.max(1, Number(v) || 1))"
                    />
                  </div>

                  <!-- P2-3: Prompt Caching 开关 -->
                  <div class="form-group">
                    <label class="form-label form-label-row">
                      <span>Prompt Caching</span>
                      <span v-if="isAnthropicProvider" class="label-hint">Anthropic 缓存加速</span>
                      <span v-else class="label-hint">仅 Anthropic 支持显式缓存控制，OpenAI 会自动缓存</span>
                    </label>
                    <button
                      type="button"
                      class="toggle-btn"
                      :class="{ 'toggle-btn-on': cachePrompt, 'toggle-btn-disabled': !isAnthropicProvider }"
                      :disabled="!isAnthropicProvider"
                      :title="isAnthropicProvider ? undefined : '仅 Anthropic 支持显式缓存控制，OpenAI 会自动缓存'"
                      @click="isAnthropicProvider && (cachePrompt = !cachePrompt)"
                    >
                      <span class="toggle-track">
                        <span class="toggle-thumb" />
                      </span>
                      <span class="toggle-text">{{ isAnthropicProvider ? (cachePrompt ? '已启用' : '已关闭') : '自动' }}</span>
                    </button>
                  </div>

                  <!-- A3-2: 历史消息数上限 -->
                  <div class="form-group">
                    <label class="form-label" for="agent-max-history">
                      历史消息数上限
                      <span class="label-hint">
                        （留空 = 系统默认 20；按上下文长度调整）
                      </span>
                    </label>
                    <Input
                      id="agent-max-history"
                      :model-value="maxHistoryMessagesInput"
                      size="md"
                      type="number"
                      placeholder="20"
                      :min="1"
                      :max="1000"
                      autocomplete="off"
                      @update:model-value="(v) => (maxHistoryMessagesInput = String(v ?? ''))"
                    />
                  </div>
                </div>
              </details>
            </form>
          </div>

          <footer class="panel-footer">
            <Button variant="secondary" @click="close">取消</Button>
            <Button variant="primary" @click="handleSubmit">
              {{ mode === "create" ? "创建" : "保存" }}
            </Button>
          </footer>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.panel-overlay {
  position: fixed;
  inset: 0;
  z-index: var(--ip-z-modal-overlay, 9000);
  background: var(--ip-color-bg-overlay);
}

.panel {
  position: absolute;
  top: 0;
  right: 0;
  width: 480px;
  max-width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: var(--ip-color-bg-elevated);
  border-left: 1px solid var(--ip-color-border-default);
  box-shadow: var(--ip-shadow-lg);
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--ip-spacing-4) var(--ip-spacing-5);
  border-bottom: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
}

.panel-title {
  margin: 0;
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-primary);
}

.panel-body {
  flex: 1;
  overflow-y: auto;
  padding: var(--ip-spacing-5);
}

.panel-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-3) var(--ip-spacing-5) var(--ip-spacing-4);
  border-top: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
}

/* ===== 模板选择器 ===== */
.template-section {
  margin-bottom: var(--ip-spacing-5);
  padding-bottom: var(--ip-spacing-4);
  border-bottom: 1px dashed var(--ip-color-border-default);
}

.section-header {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  margin-bottom: var(--ip-spacing-3);
}

.section-icon {
  color: var(--ip-color-text-tertiary);
}

.section-title {
  flex: 1;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.clear-link {
  appearance: none;
  background: none;
  border: none;
  padding: 2px 6px;
  font-family: inherit;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-link);
  cursor: pointer;
  border-radius: var(--ip-radius-sm);
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.clear-link:hover {
  background: var(--ip-color-bg-tertiary);
}

.clear-link:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.template-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: var(--ip-spacing-2);
}

/*
 * 极窄视口（< 360px）下落单列，避免 2 列把卡片文字挤成省略号。
 * 8 个模板纵向排成 8 行，依旧全部可见。
 */
@media (max-width: 360px) {
  .template-grid {
    grid-template-columns: 1fr;
  }
}

.template-card {
  appearance: none;
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  font-family: inherit;
  text-align: left;
  transition:
    border-color var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    transform var(--ip-duration-fast) var(--ip-ease-out);
}

.template-card:hover {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
}

.template-card:focus-visible {
  outline: none;
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

.template-card:active {
  transform: translateY(0.5px);
}

.template-card-active {
  border-width: 1.5px;
}

.template-icon {
  width: 32px;
  height: 32px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  border-radius: var(--ip-radius-md);
}

.template-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.template-name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.template-desc {
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* ===== 表单字段 ===== */
.form-group {
  margin-bottom: var(--ip-spacing-4);
}

.form-label {
  display: block;
  margin-bottom: var(--ip-spacing-1);
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-primary);
}

.label-hint {
  display: inline-block;
  margin-left: var(--ip-spacing-2);
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-caption-size);
}

.form-select {
  width: 100%;
  padding: var(--ip-input-py-md) var(--ip-input-px-md);
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-body);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-input-radius);
  outline: none;
  transition:
    border-color var(--ip-duration-fast) var(--ip-ease-out),
    box-shadow var(--ip-duration-fast) var(--ip-ease-out);
}
.form-select:hover {
  border-color: var(--ip-color-border-strong);
}
.form-select:focus {
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

.form-range {
  width: 100%;
  cursor: pointer;
  accent-color: var(--ip-primary-500);
}

.model-row {
  display: flex;
  gap: var(--ip-spacing-2);
  align-items: center;
}
.form-select-model {
  flex: 1;
}

.apikey-masked {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: var(--ip-input-py-md) var(--ip-input-px-md);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-input-radius);
  background: var(--ip-color-bg-primary);
}
.masked-text {
  flex: 1;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  letter-spacing: 1px;
}

/* ===== 高级设置（折叠） ===== */

/* P2-3: Toggle 按钮（Prompt Caching 开关） */
.form-label-row {
  display: flex;
  align-items: baseline;
  gap: var(--ip-spacing-2);
}

.toggle-btn {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  background: none;
  border: none;
  cursor: pointer;
  padding: 0;
  font-family: inherit;
}

.toggle-track {
  position: relative;
  width: 36px;
  height: 20px;
  border-radius: 10px;
  background: var(--ip-color-border-default);
  transition: background var(--ip-duration-fast) var(--ip-ease-out);
 flex-shrink: 0;
}

.toggle-btn-on .toggle-track {
  background: var(--ip-primary-500);
}

.toggle-thumb {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: #fff;
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
 box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
}

.toggle-btn-on .toggle-thumb {
  transform: translateX(16px);
}

.toggle-text {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
}

.toggle-btn-disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.toggle-btn-disabled .toggle-track {
  background: var(--ip-color-border-default);
}

.toggle-btn-disabled .toggle-thumb {
  transform: translateX(0);
}

.advanced-section {
  border-top: 1px dashed var(--ip-color-border-default);
  padding-top: var(--ip-spacing-3);
  margin-top: var(--ip-spacing-2);
}

.advanced-summary {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  cursor: pointer;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  list-style: none;
  padding: var(--ip-spacing-2) 0;
  user-select: none;
  border-radius: var(--ip-radius-sm);
}

.advanced-summary::-webkit-details-marker {
  display: none;
}

.advanced-summary::marker {
  display: none;
  content: "";
}

.advanced-summary:hover {
  color: var(--ip-color-text-primary);
}

.advanced-summary:focus-visible {
  outline: none;
  color: var(--ip-color-text-primary);
  box-shadow: var(--ip-shadow-focus);
}

.advanced-chevron {
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}

.advanced-section[open] > .advanced-summary .advanced-chevron {
  transform: rotate(180deg);
}

.advanced-body {
  padding-top: var(--ip-spacing-3);
}

/* 滑入/滑出动画 */
.slide-enter-from .panel,
.slide-leave-to .panel {
  transform: translateX(100%);
}
.slide-enter-active .panel,
.slide-leave-active .panel {
  transition: transform var(--ip-duration-base) var(--ip-ease-out);
}
.slide-enter-from,
.slide-leave-to {
  opacity: 0;
}
.slide-enter-active,
.slide-leave-active {
  transition: opacity var(--ip-duration-base) var(--ip-ease-out);
}
</style>
