<script setup lang="ts">
// Agent 创建/编辑表单 — 侧滑面板
import { ref, computed, watch } from "vue";
import { Input, Textarea, Button } from "@ice-paw/ui";
import { X } from "lucide-vue-next";
import type { Agent } from "../../types";

const PROVIDERS = ["OpenAI", "Anthropic", "GLM", "DeepSeek"] as const;
type ProviderName = (typeof PROVIDERS)[number];

const MODEL_PRESETS: Record<ProviderName, string[]> = {
  OpenAI: ["gpt-4o", "gpt-4o-mini"],
  Anthropic: ["claude-sonnet-4-20250514"],
  GLM: ["glm-4-flash", "glm-4-plus"],
  DeepSeek: ["deepseek-chat", "deepseek-reasoner"],
};

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

const errors = ref<Record<string, string>>({});

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
}

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
  });
}

function close(): void {
  emit("update:open", false);
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

              <div class="form-group">
                <label class="form-label" for="agent-provider">Provider *</label>
                <select id="agent-provider" v-model="provider" class="form-select">
                  <option v-for="p in PROVIDERS" :key="p" :value="p">{{ p }}</option>
                </select>
              </div>

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

              <div class="form-group">
                <label class="form-label" for="agent-baseurl">Base URL（可选）</label>
                <Input
                  id="agent-baseurl"
                  v-model="baseUrl"
                  size="md"
                  placeholder="留空使用默认地址"
                  autocomplete="off"
                />
              </div>

              <div class="form-group">
                <label class="form-label" for="agent-systemprompt">System Prompt（可选）</label>
                <Textarea
                  id="agent-systemprompt"
                  v-model="systemPrompt"
                  size="md"
                  :rows="3"
                  placeholder="系统提示词..."
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
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-tertiary);
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