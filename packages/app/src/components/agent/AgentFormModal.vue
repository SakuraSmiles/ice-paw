<script setup lang="ts">
// AgentFormModal.vue — Agent 新建/编辑弹窗
// 核心字段：name, provider, model, api_key, base_url, workspace_path
// 行为层配置（system_prompt, temperature 等）放在 workspace/agent.yaml 中
import { ref, computed, onMounted, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import type { Agent, NewAgent, AgentUpdate } from "../../types";
import { bridge } from "../../api/bridge";
import Combobox from "../common/Combobox.vue";

const props = defineProps<{
  agent: Agent | null;
}>();

const emit = defineEmits<{
  close: [];
  saved: [agent: Agent];
  delete: [agent: Agent];
}>();

const isEdit = computed(() => !!props.agent);

// Provider 列表（定义在 form 之前，供初始化使用）
const providerLabels: Record<string, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  deepseek: "DeepSeek",
  glm: "GLM",
  minimax: "MiniMax",
  "minimax-cn": "MiniMax (中国站)",
};
const providerOptions = Object.values(providerLabels);
const providerKeyOf = (label: string): string => {
  return Object.entries(providerLabels).find(([, v]) => v === label)?.[0] ?? label;
};

const defaultWorkspace = ref("");

const initProviderLabel = props.agent?.provider ? (providerLabels[props.agent.provider] ?? props.agent.provider) : "OpenAI";
const form = ref({
  id: props.agent?.id ?? "",
  name: props.agent?.name ?? "",
  provider: initProviderLabel,
  model: props.agent?.model ?? "",
  api_key: "",
  base_url: props.agent?.base_url ?? "",
  workspace_path: props.agent?.workspace_path ?? "",
});

// 加载全局默认工作空间
onMounted(async () => {
  try {
    const prefs = await bridge.preferences.get();
    defaultWorkspace.value = (prefs.default_workspace_path ?? "").replace(/\\/g, "/");
    if (!props.agent && defaultWorkspace.value && form.value.id) {
      form.value.workspace_path = `${defaultWorkspace.value.replace(/\/$/, "")}/${form.value.id}`;
    }
  } catch {
    // 静默忽略
  }
});

// 新建模式下，id 变化时自动更新工作区路径
watch(() => form.value.id, (newId) => {
  if (!props.agent && defaultWorkspace.value && newId) {
    form.value.workspace_path = `${defaultWorkspace.value.replace(/\/$/, "")}/${newId}`;
  }
});

// Provider 变化时自动切换推荐模型
watch(() => form.value.provider, () => {
  const key = providerKeyOf(form.value.provider);
  const suggestions = modelSuggestions[key];
  if (suggestions && suggestions.length > 0) {
    form.value.model = suggestions[0];
  }
});

const saving = ref(false);

// 如果编辑中且有 workspace_path 且 config_from_file，显示提示
const hasFileConfig = computed(() =>
  isEdit.value && props.agent?.workspace_path && props.agent?.config_from_file,
);

const modelSuggestions: Record<string, string[]> = {
  openai: ["gpt-4o", "gpt-4o-mini", "o3-mini", "gpt-4.1", "gpt-4.1-mini"],
  anthropic: ["claude-sonnet-4-20250514", "claude-haiku-3-5-20241022", "claude-opus-4-20250514"],
  deepseek: ["deepseek-v4-pro", "deepseek-v4-flash", "deepseek-chat", "deepseek-reasoner"],
  glm: ["glm-5-turbo", "glm-5.2", "glm-5.1", "glm-4", "glm-4-flash"],
  minimax: ["MiniMax-M3", "MiniMax-M2.5", "MiniMax-M2.5-highspeed"],
  "minimax-cn": ["MiniMax-M3", "MiniMax-M2.5", "MiniMax-M2.5-highspeed"],
};

const currentProviderKey = computed(() => providerKeyOf(form.value.provider));
const currentSuggestions = computed(() => modelSuggestions[currentProviderKey.value] ?? []);

async function pickWorkspace() {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择工作区目录",
    defaultPath: form.value.workspace_path || undefined,
  });
  if (selected) {
    form.value.workspace_path = selected;
  }
}

async function save() {
  if (saving.value) return;
  saving.value = true;

  try {
    const currentAgent = props.agent;
    if (isEdit.value && currentAgent) {
      const update: AgentUpdate = {
        id: currentAgent.id,
        name: form.value.name,
        provider: currentProviderKey.value,
        model: form.value.model,
        base_url: form.value.base_url || undefined,
        workspace_path: form.value.workspace_path || null,
      };
      const updated = await bridge.agents.update(update);

      if (form.value.api_key) {
        await bridge.agents.rotateKey(
          currentAgent.id,
          form.value.api_key,
          form.value.base_url || undefined,
        );
      }

      const fresh = await bridge.agents.list();
      const real = fresh.find((a) => a.id === currentAgent.id);
      emit("saved", real ?? updated);
    } else {
      const input: NewAgent = {
        id: form.value.id,
        name: form.value.name,
        provider: currentProviderKey.value,
        model: form.value.model,
        api_key: form.value.api_key,
        base_url: form.value.base_url || undefined,
        workspace_path: form.value.workspace_path || undefined,
      };
      const created = await bridge.agents.create(input);
      emit("saved", created);
    }
  } catch (e) {
    console.error("保存 Agent 失败:", e);
  } finally {
    saving.value = false;
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
      <div class="modal-header">
        <h2 class="modal-title">{{ isEdit ? "编辑 Agent" : "新建 Agent" }}</h2>
        <button class="modal-close" @click="emit('close')">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      <div class="modal-body">
        <!-- 文件配置提示 -->
        <div v-if="hasFileConfig" class="file-config-banner">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
            <polyline points="14 2 14 8 20 8" />
          </svg>
          <span>部分配置来自工作区 <code>agent.yaml</code></span>
        </div>

        <div class="form-grid">
          <div class="form-group">
            <label class="form-label">名称</label>
            <input v-model="form.name" type="text" class="form-input" placeholder="例如：代码助手" />
          </div>

          <div class="form-group">
            <label class="form-label">ID <span class="label-opt">唯一，不可修改</span></label>
            <input v-model="form.id" type="text" class="form-input" placeholder="例如：code-assistant" :disabled="isEdit" :class="{ 'input-disabled': isEdit }" />
          </div>

          <div class="form-group">
            <label class="form-label">Provider</label>
            <Combobox v-model="form.provider" :options="providerOptions" />
          </div>

          <div class="form-group form-group-wide">
            <label class="form-label">模型</label>
            <Combobox v-model="form.model" :options="currentSuggestions" placeholder="输入或选择模型名称" />
          </div>

          <div class="form-group form-group-wide">
            <label class="form-label">
              API Key
              <span v-if="isEdit" :class="props.agent?.has_api_key ? 'key-status ok' : 'key-status warn'">
                {{ props.agent?.has_api_key ? "已配置" : "未配置" }}
              </span>
              <span v-if="isEdit && props.agent?.has_api_key" class="key-hint">（留空则不修改）</span>
            </label>
            <input v-model="form.api_key" type="password" class="form-input" :placeholder="isEdit ? '留空保持现有密钥' : '输入 API Key'" />
          </div>

          <div class="form-group form-group-wide">
            <label class="form-label">API URL <span class="label-opt">可选</span></label>
            <input v-model="form.base_url" type="text" class="form-input" placeholder="留空使用 Provider 默认地址" />
          </div>

          <div class="form-group form-group-wide">
            <label class="form-label">工作区</label>
            <div class="path-picker-group">
              <input v-model="form.workspace_path" type="text" class="form-input path-input" placeholder="选择或输入工作区路径" readonly @click="pickWorkspace" />
              <button class="btn-browse" type="button" @click="pickWorkspace" title="选择目录">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                </svg>
              </button>
            </div>
            <p class="form-hint">在此目录下创建 <code>agent.yaml</code> 可配置 system_prompt、temperature 等行为参数</p>
          </div>
        </div>
      </div>

      <div class="modal-footer">
        <button v-if="isEdit" class="btn-danger" @click="confirmDelete">删除</button>
        <div class="footer-right">
          <button class="btn-secondary" @click="emit('close')">取消</button>
          <button class="btn-primary" :disabled="saving" @click="save">{{ saving ? '保存中...' : (isEdit ? '保存' : '创建') }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.modal-overlay {
  position: fixed; inset: 0;
  z-index: var(--ip-z-modal-overlay);
  background-color: rgba(0, 0, 0, 0.4);
  display: flex; align-items: center; justify-content: center;
  padding: 40px;
}
.modal-container {
  width: 100%; max-width: 540px; max-height: 85vh;
  background-color: var(--ip-color-bg-secondary);
  border-radius: var(--ip-radius-xl);
  box-shadow: var(--ip-shadow-xl);
  display: flex; flex-direction: column; overflow: hidden;
}
.modal-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 20px 24px 16px; flex-shrink: 0;
}
.modal-title { font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); margin: 0; }
.modal-close {
  display: flex; align-items: center; justify-content: center;
  width: 32px; height: 32px; border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.modal-close:hover { background-color: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }

.modal-body { flex: 1; overflow-y: auto; padding: 0 24px 8px; }

.file-config-banner {
  display: flex; align-items: center; gap: 8px;
  padding: 10px 14px; margin-bottom: 16px;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
}
.file-config-banner code {
  font-family: var(--ip-font-mono);
  background: var(--ip-color-bg-secondary);
  padding: 0 6px;
  border-radius: var(--ip-radius-sm);
}

.form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
.form-group { display: flex; flex-direction: column; gap: 5px; }
.form-group-wide { grid-column: 1 / -1; }
.form-label { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-primary); }
.label-opt { font-weight: var(--ip-font-weight-regular); color: var(--ip-color-text-tertiary); font-size: var(--ip-text-caption-size); }
.key-hint { font-weight: var(--ip-font-weight-regular); color: var(--ip-color-text-tertiary); font-size: var(--ip-text-caption-size); }
.form-input {
  height: 36px; padding: 0 12px;
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
.form-input::placeholder { color: var(--ip-color-text-placeholder); }
.form-input.input-disabled { opacity: 0.6; cursor: not-allowed; }
.form-hint {
  margin: 0; font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary); line-height: 1.4;
}
.form-hint code {
  font-family: var(--ip-font-mono);
  background: var(--ip-color-bg-tertiary);
  padding: 0 4px;
  border-radius: var(--ip-radius-sm);
}

/* API Key 状态 */
.key-status { font-weight: var(--ip-font-weight-regular); font-size: var(--ip-text-caption-size); padding: 1px 8px; border-radius: var(--ip-radius-full); margin-left: 6px; }
.key-status.ok { background-color: var(--ip-success-bg); color: var(--ip-success-text); }
.key-status.warn { background-color: var(--ip-warning-bg); color: var(--ip-warning-text); }

/* 路径选择器 */
.path-picker-group { display: flex; gap: 8px; }
.path-input { flex: 1; cursor: pointer; }
.btn-browse {
  display: flex; align-items: center; justify-content: center;
  width: 36px; height: 36px;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary);
  cursor: pointer; flex-shrink: 0;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-browse:hover { background-color: var(--ip-color-bg-secondary); border-color: var(--color-input-focus-border); color: var(--ip-primary-600); }

.modal-footer {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 24px 20px; flex-shrink: 0;
}
.footer-right { display: flex; gap: 8px; margin-left: auto; }
.btn-primary {
  display: flex; align-items: center; justify-content: center; gap: 6px;
  padding: 8px 16px; height: 36px;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium);
  color: white; background-color: var(--ip-primary-600); border: none;
  border-radius: var(--ip-radius-md); cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-primary:hover { background-color: var(--ip-primary-700); }
.btn-primary:disabled { opacity: 0.6; cursor: not-allowed; }
.btn-secondary {
  display: flex; align-items: center; justify-content: center; gap: 6px;
  padding: 8px 16px; height: 36px;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary); background-color: transparent;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md); cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-secondary:hover { background-color: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }
.btn-danger {
  display: flex; align-items: center; justify-content: center; gap: 6px;
  padding: 8px 16px; height: 36px;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium);
  color: var(--ip-danger-base); background-color: transparent;
  border: 1px solid var(--ip-danger-border);
  border-radius: var(--ip-radius-md); cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-danger:hover { background-color: var(--ip-danger-bg); color: var(--ip-danger-active); }
</style>
