<script setup lang="ts">
// Agent 创建/编辑表单 — 侧滑面板
//
// 职责：
//   - 右侧滑入面板，新建和编辑共用
//   - 字段：name（必填）、provider（下拉）、model（联动预设+手输）、api_key、base_url、
//          system_prompt（多行）、temperature（滑块 0-2）、max_tokens（数字）
//   - 校验：name 非空、api_key 长度 > 0
//   - 提交：emit('submit', payload)，由外层调 store action + Toast
//
// props:
//   - mode:  'create' | 'edit'
//   - agent: 编辑模式下当前 Agent（create 模式为 null）
//   - open:  是否显示面板
//
// emits:
//   - 'update:open': 关闭面板
//   - 'submit':     提交表单数据

import { ref, computed, watch } from "vue";
import type { Agent } from "../../types";

// ============================================================================
// 类型
// ============================================================================

/** Provider 列表 */
const PROVIDERS = ["OpenAI", "Anthropic", "GLM", "DeepSeek"] as const;
type ProviderName = (typeof PROVIDERS)[number];

/** Provider -> 预设模型映射 */
const MODEL_PRESETS: Record<ProviderName, string[]> = {
  OpenAI: ["gpt-4o", "gpt-4o-mini"],
  Anthropic: ["claude-sonnet-4-20250514"],
  GLM: ["glm-4-flash", "glm-4-plus"],
  DeepSeek: ["deepseek-chat", "deepseek-reasoner"],
};

/** 提交载荷（create 与 edit 统一为一个结构） */
export interface AgentFormPayload {
  name: string;
  provider: string;
  model: string;
  api_key: string;
  base_url: string;
  system_prompt: string;
  temperature: number;
  max_tokens: number;
  /** 编辑模式独有：需要轮换 key 时为 true */
  rotateApiKey: boolean;
}

// ============================================================================
// Props & Emits
// ============================================================================

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
const customModel = ref(""); // 用户手输的自定义模型名
const useCustomModel = ref(false);
const apiKey = ref("");
const rotateApiKey = ref(false); // 编辑模式下是否要更换 key
const baseUrl = ref("");
const systemPrompt = ref("");
const temperature = ref(0.7);
const maxTokens = ref(4096);

/** 校验错误信息 */
const errors = ref<Record<string, string>>({});

// ============================================================================
// 计算属性
// ============================================================================

/** 当前 provider 的预设模型列表 */
const presetModels = computed<string[]>(() => {
  return MODEL_PRESETS[provider.value as ProviderName] ?? [];
});

/** 实际提交的 model 值 */
const modelValue = computed(() => {
  return useCustomModel.value ? customModel.value : model.value;
});

/** 面板标题 */
const panelTitle = computed(() => {
  return props.mode === "create" ? "新建 Agent" : "编辑 Agent";
});

// ============================================================================
// 监听器
// ============================================================================

/** provider 变化时，重置 model 选择 */
watch(provider, () => {
  model.value = presetModels.value[0] ?? "";
  useCustomModel.value = false;
  customModel.value = "";
});

/** 面板打开时，根据 mode 初始化表单 */
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
// 方法
// ============================================================================

/** 重置表单到默认值 */
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

/** 从现有 Agent 填充表单 */
function populateFromAgent(a: Agent): void {
  name.value = a.name;
  // 查找匹配的 provider 显示名（大小写不敏感）
  const match = PROVIDERS.find((p) => p.toLowerCase() === a.provider.toLowerCase());
  provider.value = match ?? PROVIDERS[0];
  // 检查 model 是否在预设中
  const presets = MODEL_PRESETS[provider.value as ProviderName] ?? [];
  if (presets.includes(a.model)) {
    model.value = a.model;
    useCustomModel.value = false;
  } else {
    customModel.value = a.model;
    useCustomModel.value = true;
  }
  apiKey.value = ""; // api_key 不回填，显示占位
  rotateApiKey.value = false;
  baseUrl.value = a.base_url ?? "";
  systemPrompt.value = a.system_prompt ?? "";
  temperature.value = a.temperature;
  maxTokens.value = a.max_tokens;
}

/** 校验表单 */
function validate(): boolean {
  const errs: Record<string, string> = {};
  if (!name.value.trim()) {
    errs.name = "名称不能为空";
  }
  // create 模式：apiKey 必填；edit 模式仅在 rotateApiKey 时必填
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

/** 提交表单 */
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

/** 关闭面板 */
function close(): void {
  emit("update:open", false);
}
</script>

<template>
  <Teleport to="body">
    <Transition name="slide">
      <div v-if="open" class="panel-overlay" @click.self="close">
        <div class="panel">
          <!-- 面板头部 -->
          <div class="panel-header">
            <h3 class="panel-title">{{ panelTitle }}</h3>
            <button class="btn-close" title="关闭" @click="close">X</button>
          </div>

          <!-- 表单内容 -->
          <div class="panel-body">
            <form @submit.prevent="handleSubmit">
              <!-- 名称 -->
              <div class="form-group">
                <label class="form-label" for="agent-name">名称 *</label>
                <input
                  id="agent-name"
                  v-model="name"
                  class="form-input"
                  type="text"
                  placeholder="例如：论文润色助手"
                  autocomplete="off"
                />
                <span v-if="errors.name" class="form-error">{{ errors.name }}</span>
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
                  <select v-model="model" class="form-select form-select-model">
                    <option
                      v-for="m in presetModels"
                      :key="m"
                      :value="m"
                    >
                      {{ m }}
                    </option>
                  </select>
                  <button
                    type="button"
                    class="btn-text"
                    @click="useCustomModel = true"
                  >
                    自定义
                  </button>
                </div>
                <div v-else class="model-row">
                  <input
                    id="agent-model"
                    v-model="customModel"
                    class="form-input"
                    type="text"
                    placeholder="输入模型名称"
                    autocomplete="off"
                  />
                  <button
                    type="button"
                    class="btn-text"
                    @click="
                      useCustomModel = false;
                      customModel = '';
                    "
                  >
                    预设
                  </button>
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
                    <button
                      type="button"
                      class="btn-text"
                      @click="rotateApiKey = true"
                    >
                      更换 Key
                    </button>
                  </div>
                </template>
                <template v-else>
                  <input
                    id="agent-apikey"
                    v-model="apiKey"
                    class="form-input"
                    type="password"
                    placeholder="sk-..."
                    autocomplete="off"
                  />
                  <span v-if="errors.api_key" class="form-error">{{ errors.api_key }}</span>
                </template>
              </div>

              <!-- Base URL -->
              <div class="form-group">
                <label class="form-label" for="agent-baseurl">Base URL（可选）</label>
                <input
                  id="agent-baseurl"
                  v-model="baseUrl"
                  class="form-input"
                  type="text"
                  placeholder="留空使用默认地址"
                  autocomplete="off"
                />
              </div>

              <!-- System Prompt -->
              <div class="form-group">
                <label class="form-label" for="agent-systemprompt">System Prompt（可选）</label>
                <textarea
                  id="agent-systemprompt"
                  v-model="systemPrompt"
                  class="form-textarea"
                  rows="3"
                  placeholder="系统提示词..."
                />
              </div>

              <!-- Temperature -->
              <div class="form-group">
                <label class="form-label">
                  Temperature: {{ temperature.toFixed(1) }}
                </label>
                <input
                  v-model.number="temperature"
                  class="form-range"
                  type="range"
                  min="0"
                  max="2"
                  step="0.1"
                />
              </div>

              <!-- Max Tokens -->
              <div class="form-group">
                <label class="form-label" for="agent-maxtokens">Max Tokens</label>
                <input
                  id="agent-maxtokens"
                  v-model.number="maxTokens"
                  class="form-input"
                  type="number"
                  min="1"
                  max="128000"
                  step="1"
                />
              </div>
            </form>
          </div>

          <!-- 面板底部 -->
          <div class="panel-footer">
            <button class="btn btn-secondary" @click="close">取消</button>
            <button class="btn btn-primary" @click="handleSubmit">
              {{ mode === "create" ? "创建" : "保存" }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.panel-overlay {
  position: fixed;
  inset: 0;
  z-index: 9000;
  background: rgba(0, 0, 0, 0.25);
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
  background: var(--panel-bg, #ffffff);
  border-left: 1px solid var(--panel-border, #e0e0e0);
  box-shadow: -4px 0 16px rgba(0, 0, 0, 0.08);
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 20px;
  border-bottom: 1px solid var(--panel-border, #e0e0e0);
}

.panel-title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: var(--text-primary, #1a1a1a);
}

.btn-close {
  width: 28px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary, #888);
  font-size: 16px;
  cursor: pointer;
}
.btn-close:hover {
  background: var(--hover-bg, #f0f0f0);
}

.panel-body {
  flex: 1;
  overflow-y: auto;
  padding: 20px;
}

.panel-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 14px 20px;
  border-top: 1px solid var(--panel-border, #e0e0e0);
}

/* 表单 */
.form-group {
  margin-bottom: 16px;
}

.form-label {
  display: block;
  margin-bottom: 4px;
  font-size: 13px;
  font-weight: 500;
  color: var(--text-primary, #1a1a1a);
}

.label-hint {
  font-weight: 400;
  color: var(--text-secondary, #888);
}

.form-input,
.form-select,
.form-textarea {
  width: 100%;
  padding: 8px 10px;
  font-size: 14px;
  border: 1px solid var(--input-border, #d0d0d0);
  border-radius: 6px;
  background: var(--input-bg, #ffffff);
  color: var(--text-primary, #1a1a1a);
  outline: none;
  box-sizing: border-box;
  transition: border-color 100ms ease;
}
.form-input:focus,
.form-select:focus,
.form-textarea:focus {
  border-color: var(--focus-border, #1a73e8);
}

.form-textarea {
  resize: vertical;
  font-family: inherit;
}

.form-range {
  width: 100%;
  cursor: pointer;
}

.form-error {
  display: block;
  margin-top: 4px;
  font-size: 12px;
  color: var(--danger-fg, #d93025);
}

.model-row {
  display: flex;
  gap: 8px;
  align-items: center;
}
.form-select-model {
  flex: 1;
}

.apikey-masked {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border: 1px solid var(--input-border, #d0d0d0);
  border-radius: 6px;
  background: var(--input-bg, #f9f9f9);
}
.masked-text {
  flex: 1;
  font-size: 14px;
  color: var(--text-secondary, #888);
  letter-spacing: 1px;
}

.btn-text {
  flex-shrink: 0;
  padding: 4px 8px;
  font-size: 12px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--accent-fg, #1a73e8);
  cursor: pointer;
}
.btn-text:hover {
  background: var(--hover-bg, #f0f0f0);
}

.btn {
  padding: 8px 20px;
  font-size: 14px;
  border: 1px solid transparent;
  border-radius: 6px;
  cursor: pointer;
  transition: background 100ms ease;
}

.btn-secondary {
  background: var(--btn-secondary-bg, #f0f0f0);
  color: var(--text-secondary, #555);
  border-color: var(--btn-secondary-border, #d0d0d0);
}
.btn-secondary:hover {
  background: var(--btn-secondary-bg-hover, #e0e0e0);
}

.btn-primary {
  background: var(--accent-bg, #1a73e8);
  color: #fff;
}
.btn-primary:hover {
  background: var(--accent-bg-hover, #1557b0);
}

/* 滑入/滑出动画 */
.slide-enter-from .panel {
  transform: translateX(100%);
}
.slide-enter-active .panel {
  transition: transform 200ms ease;
}
.slide-leave-to .panel {
  transform: translateX(100%);
}
.slide-leave-active .panel {
  transition: transform 200ms ease;
}

/* 暗色模式 */
@media (prefers-color-scheme: dark) {
  .panel {
    --panel-bg: #1e1e2e;
    --panel-border: #3a3a4a;
  }
  .panel-title {
    --text-primary: #f0f0f0;
  }
  .btn-close {
    --hover-bg: #3a3a4a;
  }
  .form-label {
    --text-primary: #e0e0e0;
  }
  .form-input,
  .form-select,
  .form-textarea {
    --input-bg: #2a2a3a;
    --input-border: #4a4a5a;
    --focus-border: #4a90e2;
    --text-primary: #f0f0f0;
  }
  .apikey-masked {
    --input-border: #4a4a5a;
    --input-bg: #2a2a3a;
  }
  .masked-text {
    --text-secondary: #666;
  }
  .btn-text {
    --accent-fg: #4a90e2;
    --hover-bg: #3a3a4a;
  }
  .btn-secondary {
    --btn-secondary-bg: #3a3a4a;
    --btn-secondary-border: #4a4a5a;
    --btn-secondary-bg-hover: #4a4a5a;
    --text-secondary: #ccc;
  }
  .btn-primary {
    --accent-bg: #4a90e2;
  }
  .btn-primary:hover {
    --accent-bg-hover: #357abd;
  }
}
</style>