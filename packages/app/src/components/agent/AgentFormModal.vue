<script setup lang="ts">
// AgentFormModal.vue — Agent 新建/编辑弹窗
import { ref, computed } from "vue";
import type { Agent } from "../../types";

const props = defineProps<{
  agent: Agent | null;
}>();

const emit = defineEmits<{
  close: [];
  saved: [agent: Agent];
  delete: [agent: Agent];
}>();

const isEdit = computed(() => !!props.agent);

// ===== 表单数据 =====
const form = ref({
  name: props.agent?.name ?? "",
  provider: props.agent?.provider ?? "openai",
  model: props.agent?.model ?? "",
  api_key: "",
  base_url: props.agent?.base_url ?? "",
  system_prompt: props.agent?.system_prompt ?? "",
  temperature: props.agent?.temperature ?? 0.7,
  max_tokens: props.agent?.max_tokens ?? 4096,
  description: props.agent?.description ?? "",
  supports_vision: props.agent?.supports_vision ?? false,
  cache_prompt: props.agent?.cache_prompt ?? true,
  max_history_messages: props.agent?.max_history_messages ?? null,
});

const showAdvanced = ref(false);

// Provider 选项
const providers = [
  { value: "openai", label: "OpenAI" },
  { value: "anthropic", label: "Anthropic" },
  { value: "deepseek", label: "DeepSeek" },
  { value: "glm", label: "GLM" },
  { value: "minimax", label: "MiniMax" },
  { value: "minimax-cn", label: "MiniMax (中国站)" },
];

// 各 Provider 的推荐模型
const modelSuggestions: Record<string, string[]> = {
  openai: ["gpt-4o", "gpt-4o-mini", "o3-mini", "gpt-4.1", "gpt-4.1-mini"],
  anthropic: ["claude-sonnet-4-20250514", "claude-haiku-3-5-20241022", "claude-opus-4-20250514"],
  deepseek: ["deepseek-chat", "deepseek-reasoner"],
  glm: ["glm-4", "glm-4v", "glm-4-flash"],
  minimax: ["MiniMax-M1-0619"],
  "minimax-cn": ["MiniMax-M1-0619"],
};

const currentSuggestions = computed(() => modelSuggestions[form.value.provider] ?? []);

// 根据 provider 更新默认模型
function onProviderChange() {
  const suggestions = modelSuggestions[form.value.provider];
  if (suggestions && suggestions.length > 0) {
    form.value.model = suggestions[0];
  }
}

function save() {
  // TODO: 接入后端 bridge.agents.create() / bridge.agents.update()
  if (isEdit.value && props.agent) {
    const updated: Agent = {
      ...props.agent,
      name: form.value.name,
      provider: form.value.provider,
      model: form.value.model,
      system_prompt: form.value.system_prompt,
      base_url: form.value.base_url || null,
      temperature: form.value.temperature,
      max_tokens: form.value.max_tokens,
      description: form.value.description,
      supports_vision: form.value.supports_vision,
      cache_prompt: form.value.cache_prompt,
      max_history_messages: form.value.max_history_messages,
      updated_at: new Date().toISOString(),
    };
    emit("saved", updated);
  } else {
    const created: Agent = {
      id: Date.now().toString(),
      name: form.value.name,
      provider: form.value.provider,
      model: form.value.model,
      system_prompt: form.value.system_prompt,
      base_url: form.value.base_url || null,
      temperature: form.value.temperature,
      max_tokens: form.value.max_tokens,
      extra_params: {},
      sort_order: 0,
      cache_prompt: form.value.cache_prompt,
      max_history_messages: form.value.max_history_messages,
      supports_vision: form.value.supports_vision,
      description: form.value.description,
      has_api_key: !!form.value.api_key,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    emit("saved", created);
  }
}

function confirmDelete() {
  if (props.agent) {
    emit("delete", props.agent);
  }
}
</script>

<template>
  <div class="modal-overlay" @click.self="emit('close')">
    <div class="modal-container">
      <!-- 弹窗头部 -->
      <div class="modal-header">
        <h2 class="modal-title">{{ isEdit ? "编辑 Agent" : "新建 Agent" }}</h2>
        <button class="modal-close" @click="emit('close')">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      <!-- 表单主体 -->
      <div class="modal-body">
        <div class="form-grid">
          <!-- 名称 -->
          <div class="form-group">
            <label class="form-label">名称</label>
            <input v-model="form.name" type="text" class="form-input" placeholder="例如：代码助手" />
          </div>

          <!-- Provider -->
          <div class="form-group">
            <label class="form-label">Provider</label>
            <select v-model="form.provider" class="form-input" @change="onProviderChange">
              <option v-for="p in providers" :key="p.value" :value="p.value">{{ p.label }}</option>
            </select>
          </div>

          <!-- 模型 -->
          <div class="form-group form-group-wide">
            <label class="form-label">模型</label>
            <input v-model="form.model" type="text" class="form-input" list="model-suggestions" placeholder="输入模型名称" />
            <datalist id="model-suggestions">
              <option v-for="m in currentSuggestions" :key="m" :value="m" />
            </datalist>
          </div>

          <!-- API Key -->
          <div class="form-group form-group-wide">
            <label class="form-label">
              API Key
              <span v-if="isEdit && props.agent?.has_api_key" class="key-hint">（已有密钥，留空则不修改）</span>
            </label>
            <input v-model="form.api_key" type="password" class="form-input" :placeholder="isEdit ? '留空保持现有密钥' : '输入 API Key'" />
          </div>

          <!-- Base URL -->
          <div class="form-group form-group-wide">
            <label class="form-label">Base URL <span class="label-opt">可选</span></label>
            <input v-model="form.base_url" type="text" class="form-input" placeholder="留空使用 Provider 默认地址" />
          </div>

          <!-- 描述 -->
          <div class="form-group form-group-wide">
            <label class="form-label">描述 <span class="label-opt">可选</span></label>
            <input v-model="form.description" type="text" class="form-input" placeholder="简短描述这个 Agent 的用途" />
          </div>

          <!-- Temperature -->
          <div class="form-group">
            <label class="form-label">Temperature <span class="label-val">{{ form.temperature }}</span></label>
            <input v-model.number="form.temperature" type="range" min="0" max="2" step="0.1" class="form-range" />
            <div class="range-labels">
              <span>精确</span>
              <span>创意</span>
            </div>
          </div>

          <!-- Max Tokens -->
          <div class="form-group">
            <label class="form-label">Max Tokens</label>
            <input v-model.number="form.max_tokens" type="number" class="form-input" min="1" max="128000" />
          </div>
        </div>

        <!-- 高级设置 -->
        <div class="advanced-toggle" @click="showAdvanced = !showAdvanced">
          <svg :class="{ rotated: showAdvanced }" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6" />
          </svg>
          高级设置
        </div>

        <div v-if="showAdvanced" class="advanced-section">
          <div class="form-grid">
            <!-- System Prompt -->
            <div class="form-group form-group-wide">
              <label class="form-label">System Prompt</label>
              <textarea v-model="form.system_prompt" class="form-textarea" rows="4" placeholder="Agent 的系统提示词（可选）" />
            </div>

            <!-- 开关 -->
            <div class="form-group">
              <label class="form-label">Vision</label>
              <div class="toggle-row">
                <button :class="['toggle-btn', { on: form.supports_vision }]" @click="form.supports_vision = !form.supports_vision">
                  <span class="toggle-knob" />
                </button>
                <span class="toggle-label">{{ form.supports_vision ? '启用' : '关闭' }}</span>
              </div>
            </div>

            <div class="form-group">
              <label class="form-label">Prompt Cache</label>
              <div class="toggle-row">
                <button :class="['toggle-btn', { on: form.cache_prompt }]" @click="form.cache_prompt = !form.cache_prompt">
                  <span class="toggle-knob" />
                </button>
                <span class="toggle-label">{{ form.cache_prompt ? '启用' : '关闭' }}</span>
              </div>
            </div>

            <div class="form-group">
              <label class="form-label">历史消息数</label>
              <input v-model.number="form.max_history_messages" type="number" class="form-input" placeholder="默认 20" />
            </div>
          </div>
        </div>
      </div>

      <!-- 底部 -->
      <div class="modal-footer">
        <button v-if="isEdit" class="btn-danger" @click="confirmDelete">删除</button>
        <div class="footer-right">
          <button class="btn-secondary" @click="emit('close')">取消</button>
          <button class="btn-primary" @click="save">{{ isEdit ? '保存' : '创建' }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* ===== 遮罩层 ===== */
.modal-overlay {
  position: fixed;
  inset: 0;
  z-index: var(--ip-z-modal-overlay);
  background-color: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 40px;
}

.modal-container {
  width: 100%;
  max-width: 600px;
  max-height: 85vh;
  background-color: var(--ip-color-bg-secondary);
  border-radius: var(--ip-radius-xl);
  box-shadow: var(--ip-shadow-xl);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* ===== 头部 ===== */
.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 20px 24px 16px;
  flex-shrink: 0;
}

.modal-title {
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0;
}

.modal-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.modal-close:hover {
  background-color: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
}

/* ===== 表单主体 ===== */
.modal-body {
  flex: 1;
  overflow-y: auto;
  padding: 0 24px 8px;
}

.form-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.form-group-wide {
  grid-column: 1 / -1;
}

.form-label {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
}

.label-opt {
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-caption-size);
}

.label-val {
  float: right;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-secondary);
  font-size: var(--ip-text-body-sm-size);
}

.form-input {
  height: 36px;
  padding: 0 12px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  outline: none;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.form-input:focus {
  border-color: var(--color-input-focus-border);
  background-color: var(--ip-color-bg-secondary);
  box-shadow: 0 0 0 3px rgba(46, 141, 100, 0.12);
}

.form-input::placeholder {
  color: var(--ip-color-text-placeholder);
}

select.form-input {
  cursor: pointer;
  appearance: auto;
}

.form-textarea {
  padding: 10px 12px;
  font-size: var(--ip-text-body-sm-size);
  font-family: var(--ip-font-sans);
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  outline: none;
  resize: vertical;
  min-height: 80px;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.form-textarea:focus {
  border-color: var(--color-input-focus-border);
  background-color: var(--ip-color-bg-secondary);
  box-shadow: 0 0 0 3px rgba(46, 141, 100, 0.12);
}

/* Range slider */
.form-range {
  width: 100%;
  height: 4px;
  appearance: none;
  background: var(--ip-color-border-default);
  border-radius: 2px;
  outline: none;
  cursor: pointer;
}

.form-range::-webkit-slider-thumb {
  appearance: none;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--ip-primary-500);
  border: 2px solid white;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
  cursor: pointer;
}

.range-labels {
  display: flex;
  justify-content: space-between;
  font-size: 11px;
  color: var(--ip-color-text-disabled);
}

/* Toggle switch */
.toggle-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.toggle-btn {
  position: relative;
  width: 36px;
  height: 22px;
  border-radius: 11px;
  background-color: var(--ip-color-border-default);
  border: none;
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
  padding: 0;
}

.toggle-btn.on {
  background-color: var(--ip-primary-500);
}

.toggle-knob {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background-color: white;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.15);
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}

.toggle-btn.on .toggle-knob {
  transform: translateX(14px);
}

.toggle-label {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
}

/* 高级设置折叠 */
.advanced-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 12px 0;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  user-select: none;
  border-top: 1px solid var(--ip-color-border-default);
  margin-top: 16px;
}

.advanced-toggle svg {
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}

.advanced-toggle svg.rotated {
  transform: rotate(90deg);
}

.advanced-toggle:hover {
  color: var(--ip-color-text-primary);
}

.advanced-section {
  padding-bottom: 8px;
}

/* ===== 底部 ===== */
.modal-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 24px 20px;
  flex-shrink: 0;
}

.footer-right {
  display: flex;
  gap: 8px;
  margin-left: auto;
}

.btn-primary {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 8px 16px;
  height: 36px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: white;
  background-color: var(--ip-primary-600);
  border: none;
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.btn-primary:hover {
  background-color: var(--ip-primary-700);
}

.btn-secondary {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 8px 16px;
  height: 36px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  background-color: transparent;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.btn-secondary:hover {
  background-color: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
}

.btn-danger {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 8px 16px;
  height: 36px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-danger-base);
  background-color: transparent;
  border: 1px solid var(--ip-danger-border);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.btn-danger:hover {
  background-color: var(--ip-danger-bg);
  color: var(--ip-danger-active);
}
</style>
