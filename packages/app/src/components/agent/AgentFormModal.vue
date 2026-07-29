<script setup lang="ts">
// AgentFormModal.vue — Agent 新建/编辑弹窗
// 核心字段：name, provider, model, api_key, base_url, workspace_path
// 行为层配置（system_prompt, temperature 等）放在 workspace/agent.yaml 中
import { ref, computed, onMounted, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
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

// Provider 列表
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
      form.value.workspace_path = `${defaultWorkspace.value.replace(/\/$/, "")}/agents/${form.value.id}`;
    }
  } catch {
    // 静默忽略
  }
});

// 新建模式下，id 变化时自动更新工作区路径
watch(() => form.value.id, (newId) => {
  if (!props.agent && defaultWorkspace.value && newId) {
    form.value.workspace_path = `${defaultWorkspace.value.replace(/\/$/, "")}/agents/${newId}`;
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

/** 选择工作区目录 */
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

/** 在文件管理器中打开工作区 */
function openInExplorer() {
  if (form.value.workspace_path) {
    revealItemInDir(form.value.workspace_path);
  }
}

/** 工作区是否存在打开的路径 */
const hasWorkspacePath = computed(() => !!form.value.workspace_path?.trim());

const error = ref("");

function validate(): boolean {
  if (!form.value.id.trim()) { error.value = "ID 不能为空"; return false; }
  if (!form.value.name.trim()) { error.value = "名称不能为空"; return false; }
  if (!form.value.model.trim()) { error.value = "模型不能为空"; return false; }
  if (isEdit.value && form.value.api_key && form.value.api_key.trim().length < 8) {
    error.value = "API Key 格式不正确"; return false;
  }
  if (!isEdit.value && !form.value.api_key.trim()) {
    error.value = "API Key 不能为空"; return false;
  }
  error.value = "";
  return true;
}

async function save() {
  if (saving.value) return;
  if (!validate()) return;
  saving.value = true;
  error.value = "";

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
    error.value = e instanceof Error ? e.message : "保存失败";
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
      <!-- 头部 -->
      <div class="modal-header">
        <h2 class="modal-title">{{ isEdit ? "编辑智能体" : "新建智能体" }}</h2>
        <button class="modal-close" @click="emit('close')">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>

      <!-- 主体 -->
      <div class="modal-body">
        <div v-if="error" class="form-error">{{ error }}</div>

        <div class="form-table">
          <!-- 名称 -->
          <div class="form-row">
            <label class="form-label">
              名称 <span class="label-req">*</span>
            </label>
            <div class="form-control">
              <input v-model="form.name" type="text" class="input" placeholder="例如：代码助手" />
            </div>
          </div>

          <!-- ID -->
          <div class="form-row">
            <label class="form-label">
              ID <span class="label-req">*</span>
              <span class="label-hint">唯一，不可修改</span>
            </label>
            <div class="form-control">
              <input
                v-model="form.id"
                type="text"
                class="input"
                placeholder="例如：code-assistant"
                :disabled="isEdit"
                :class="{ 'input-disabled': isEdit }"
              />
            </div>
          </div>

          <!-- Provider -->
          <div class="form-row">
            <label class="form-label">Provider <span class="label-req">*</span></label>
            <div class="form-control">
              <Combobox v-model="form.provider" :options="providerOptions" />
            </div>
          </div>

          <!-- 模型 -->
          <div class="form-row">
            <label class="form-label">模型 <span class="label-req">*</span></label>
            <div class="form-control">
              <Combobox v-model="form.model" :options="currentSuggestions" placeholder="输入或选择模型名称" />
            </div>
          </div>

          <!-- API Key -->
          <div class="form-row">
            <label class="form-label">
              API Key
              <span v-if="!isEdit" class="label-req">*</span>
              <span v-if="isEdit" :class="props.agent?.has_api_key ? 'badge badge-ok ml-1' : 'badge badge-warn ml-1'">
                {{ props.agent?.has_api_key ? "已配置" : "未配置" }}
              </span>
            </label>
            <div class="form-control">
              <input
                v-model="form.api_key"
                type="password"
                class="input"
                :placeholder="isEdit ? '留空则保持现有密钥' : '输入 API Key'"
              />
            </div>
          </div>

          <!-- API URL -->
          <div class="form-row">
            <label class="form-label">
              API URL
              <span class="label-hint">可选</span>
            </label>
            <div class="form-control">
              <input
                v-model="form.base_url"
                type="text"
                class="input"
                placeholder="留空使用 Provider 默认地址"
              />
            </div>
          </div>

          <!-- 工作区 -->
          <div class="form-row">
            <label class="form-label">工作区</label>
            <div class="form-control">
              <div class="workspace-group">
                <input
                  v-model="form.workspace_path"
                  type="text"
                  class="input"
                  placeholder="选择工作区目录"
                  readonly
                  @click="pickWorkspace"
                />
                <button
                  type="button"
                  class="workspace-btn workspace-btn-dir"
                  @click="pickWorkspace"
                  title="选择目录"
                >
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                  </svg>
                </button>
                <button
                  v-if="hasWorkspacePath"
                  type="button"
                  class="workspace-btn workspace-btn-open"
                  @click="openInExplorer"
                  title="在文件管理器中打开"
                >
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M18 15v2a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2h2" />
                    <polyline points="15 3 21 3 21 9" />
                    <line x1="10" y1="14" x2="21" y2="3" />
                  </svg>
                </button>
                <span v-if="hasFileConfig" class="workspace-badge">agent.yaml</span>
              </div>
              <p class="form-hint">
                在此目录下创建 <code>agent.yaml</code> 可配置 system_prompt、temperature 等行为参数
              </p>
            </div>
          </div>
        </div>
      </div>

      <!-- 底部 -->
      <div class="modal-footer">
        <div class="footer-left">
          <button v-if="isEdit" class="btn btn-danger" @click="confirmDelete">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="3 6 5 6 21 6" />
              <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
            </svg>
            删除
          </button>
        </div>
        <div class="footer-right">
          <button class="btn btn-secondary" @click="emit('close')">取消</button>
          <button class="btn btn-primary" :disabled="saving" @click="save">
            {{ saving ? "保存中..." : (isEdit ? "保存" : "创建") }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* ===== 遮罩层 ===== */
.modal-overlay {
  position: fixed; inset: 0; z-index: var(--ip-z-modal-overlay);
  background-color: rgba(0, 0, 0, 0.4);
  display: flex; align-items: center; justify-content: center;
  padding: 40px;
}

/* ===== 弹窗容器 ===== */
.modal-container {
  width: 100%; max-width: 560px; max-height: 85vh;
  background-color: var(--ip-color-bg-elevated);
  border-radius: var(--ip-radius-xl);
  box-shadow: var(--ip-shadow-xl);
  display: flex; flex-direction: column; overflow: hidden;
}

/* ===== 头部 ===== */
.modal-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 16px 24px; flex-shrink: 0;
}
.modal-title {
  font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary); margin: 0;
}
.modal-close {
  display: flex; align-items: center; justify-content: center;
  width: 32px; height: 32px; border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary); cursor: pointer;
  background: none; border: none;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.modal-close:hover { background-color: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }

/* ===== 主体 ===== */
.modal-body { flex: 1; overflow-y: auto; padding: 0 24px 4px; }

/* 错误提示 */
.form-error {
  padding: 10px 14px; margin-bottom: 16px;
  background-color: var(--ip-danger-bg); border: 1px solid var(--ip-danger-border);
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-body-sm-size); color: var(--ip-danger-text);
}

/* ===== 表单表格（标签左 控件右） ===== */
.form-table {
  display: flex; flex-direction: column;
}

.form-row {
  display: flex;
  align-items: flex-start;
  padding: 12px 0;
  gap: 12px;
}

.form-label {
  width: 110px;
  flex-shrink: 0;
  padding-top: 6px; /* 与 32px 输入框文字对齐 */
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  line-height: 1.4;
}
.label-req { color: var(--ip-danger-base); }
.label-hint {
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-caption-size);
}

.form-control {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.ml-1 { margin-left: 4px; }

/* API Key 状态徽标 */
.badge {
  display: inline-block;
  padding: 0 6px;
  font-size: 10px;
  font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-full);
  vertical-align: middle;
  line-height: 18px;
}
.badge-ok { background-color: var(--ip-success-bg); color: var(--ip-success-text); }
.badge-warn { background-color: var(--ip-warning-bg); color: var(--ip-warning-text); }

/* 输入框 */
.input {
  width: 100%;
  height: 32px;
  padding: 0 10px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  outline: none;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
  box-sizing: border-box;
}
.input:focus {
  border-color: var(--color-input-focus-border);
  background-color: var(--color-input-bg);
  box-shadow: 0 0 0 3px rgba(46, 141, 100, 0.12);
}
.input::placeholder { color: var(--ip-color-text-placeholder); }
.input.input-disabled { opacity: 0.6; cursor: not-allowed; }

/* 提示文字 */
.form-hint {
  margin: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.4;
}
.form-hint code {
  font-family: var(--ip-font-mono);
  background: var(--ip-color-bg-tertiary);
  padding: 0 4px;
  border-radius: var(--ip-radius-sm);
}

/* Combobox 在弹窗表单中统一为 32px 高度，与输入框对齐 */
:deep(.combobox-input-wrap) {
  height: 32px;
}

/* ===== 工作区 ===== */
.workspace-group {
  display: flex;
  gap: 6px;
  align-items: center;
  flex-wrap: wrap;
}

.workspace-btn {
  display: flex; align-items: center; justify-content: center;
  width: 32px; height: 32px; flex-shrink: 0;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.workspace-btn:hover {
  background-color: var(--ip-color-bg-secondary);
  color: var(--ip-primary-600);
}
.workspace-btn-dir:hover {
  border-color: var(--color-input-focus-border);
}
.workspace-btn-open {
  color: var(--ip-primary-600);
  border-color: var(--ip-primary-300);
  background-color: var(--ip-primary-50);
}
.workspace-btn-open:hover {
  background-color: var(--ip-primary-100);
  border-color: var(--ip-primary-400);
}

.workspace-badge {
  display: inline-flex; align-items: center;
  height: 22px; padding: 0 8px;
  font-size: 10px; font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-primary-700);
  background-color: var(--ip-primary-100);
  border-radius: var(--ip-radius-full);
  white-space: nowrap;
  font-family: var(--ip-font-mono);
}

/* ===== 底部 ===== */
.modal-footer {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 24px 16px; flex-shrink: 0;
}
.footer-left { flex: 1; }
.footer-right { display: flex; gap: 8px; }

/* ===== 按钮 ===== */
.btn {
  display: flex; align-items: center; justify-content: center; gap: 6px;
  height: 32px; padding: 0 14px;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-md); cursor: pointer;
  white-space: nowrap;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.btn-primary {
  color: white; background-color: var(--ip-primary-600); border: none;
}
.btn-primary:hover { background-color: var(--ip-primary-700); }
.btn-primary:disabled { opacity: 0.6; cursor: not-allowed; }

.btn-secondary {
  color: var(--ip-color-text-secondary); background-color: transparent;
  border: 1px solid var(--ip-color-border-default);
}
.btn-secondary:hover { background-color: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }

.btn-danger {
  color: var(--ip-danger-base); background-color: transparent;
  border: 1px solid var(--ip-danger-border);
}
.btn-danger:hover { background-color: var(--ip-danger-bg); color: var(--ip-danger-active); }
</style>
