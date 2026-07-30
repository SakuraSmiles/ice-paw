<script setup lang="ts">
// McpFormModal.vue — MCP Server 新建/编辑弹窗
// 字段：name, id, description, command, args[], env{}, trust_level, enabled
import { ref, computed } from "vue";
import type { McpServer, NewMcpServer, McpServerUpdate, McpTrustLevel } from "../../types";
import { bridge } from "../../api/bridge";
import Switch from "../common/Switch.vue";

const props = defineProps<{
  server: McpServer | null;
}>();

const emit = defineEmits<{
  close: [];
  saved: [server: McpServer];
  delete: [server: McpServer];
}>();

const isEdit = computed(() => !!props.server);

// env Record ↔ 键值对数组
const envToEntries = (env: Record<string, string> | undefined | null) =>
  Object.entries(env ?? {}).map(([key, value]) => ({ key, value }));
const entriesToEnv = (entries: { key: string; value: string }[]): Record<string, string> => {
  const env: Record<string, string> = {};
  for (const e of entries) {
    const k = e.key.trim();
    if (k) env[k] = e.value;
  }
  return env;
};

const form = ref({
  id: props.server?.id ?? "",
  name: props.server?.name ?? "",
  description: props.server?.description ?? "",
  command: props.server?.command ?? "",
  args: props.server ? [...props.server.args] : [],
  envEntries: props.server ? envToEntries(props.server.env) : [],
  trust_level: (props.server?.trust_level ?? "untrusted") as McpTrustLevel,
  enabled: props.server?.enabled ?? true,
});

const saving = ref(false);
const error = ref("");
const confirmingDelete = ref(false);

function validate(): boolean {
  if (!isEdit.value && !form.value.id.trim()) { error.value = "ID 不能为空"; return false; }
  if (!form.value.name.trim()) { error.value = "名称不能为空"; return false; }
  if (!form.value.command.trim()) { error.value = "启动命令不能为空"; return false; }
  error.value = "";
  return true;
}

async function save() {
  if (saving.value) return;
  if (!validate()) return;
  saving.value = true;
  error.value = "";
  try {
    const env = entriesToEnv(form.value.envEntries);
    if (isEdit.value && props.server) {
      const input: McpServerUpdate = {
        id: props.server.id,
        name: form.value.name,
        description: form.value.description,
        command: form.value.command,
        args: form.value.args,
        env,
        enabled: form.value.enabled,
        trust_level: form.value.trust_level,
      };
      emit("saved", await bridge.mcp.update(input));
    } else {
      const input: NewMcpServer = {
        id: form.value.id,
        name: form.value.name,
        description: form.value.description,
        command: form.value.command,
        args: form.value.args,
        env,
        enabled: form.value.enabled,
        trust_level: form.value.trust_level,
      };
      emit("saved", await bridge.mcp.create(input));
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : "保存失败";
    console.error("保存 MCP Server 失败:", e);
  } finally {
    saving.value = false;
  }
}

function addArg() { form.value.args.push(""); }
function removeArg(i: number) { form.value.args.splice(i, 1); }
function addEnv() { form.value.envEntries.push({ key: "", value: "" }); }
function removeEnv(i: number) { form.value.envEntries.splice(i, 1); }

function onDeleteClick() { confirmingDelete.value = true; }
function cancelDelete() { confirmingDelete.value = false; }
function confirmDelete() {
  if (props.server) emit("delete", props.server);
}
</script>

<template>
  <div class="modal-overlay" @click.self="emit('close')">
    <div class="modal-container">
      <!-- 头部 -->
      <div class="modal-header">
        <h2 class="modal-title">{{ isEdit ? "编辑 MCP Server" : "新建 MCP Server" }}</h2>
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
            <label class="form-label">名称 <span class="label-req">*</span></label>
            <div class="form-control">
              <input v-model="form.name" type="text" class="input" placeholder="例如：文件系统工具集" />
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
                placeholder="例如：filesystem"
                :disabled="isEdit"
                :class="{ 'input-disabled': isEdit }"
              />
            </div>
          </div>

          <!-- 描述 -->
          <div class="form-row">
            <label class="form-label">描述</label>
            <div class="form-control">
              <input v-model="form.description" type="text" class="input" placeholder="可选，说明这个 Server 的用途" />
            </div>
          </div>

          <!-- 命令 -->
          <div class="form-row">
            <label class="form-label">启动命令 <span class="label-req">*</span></label>
            <div class="form-control">
              <input v-model="form.command" type="text" class="input input-mono" placeholder="如 npx / node / uvx" />
            </div>
          </div>

          <!-- 参数 -->
          <div class="form-row">
            <label class="form-label">
              参数
              <span class="label-hint">每行一个</span>
            </label>
            <div class="form-control">
              <div v-for="(_, i) in form.args" :key="i" class="dyn-row">
                <input v-model="form.args[i]" type="text" class="input input-mono" placeholder="如 @modelcontextprotocol/server-filesystem" />
                <button type="button" class="dyn-remove" @click="removeArg(i)">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
                </button>
              </div>
              <button type="button" class="dyn-add" @click="addArg">+ 添加参数</button>
            </div>
          </div>

          <!-- 环境变量 -->
          <div class="form-row">
            <label class="form-label">环境变量</label>
            <div class="form-control">
              <div v-for="(e, i) in form.envEntries" :key="i" class="dyn-row">
                <input v-model="e.key" type="text" class="input input-mono dyn-key" placeholder="KEY" />
                <span class="dyn-eq">=</span>
                <input v-model="e.value" type="text" class="input input-mono" placeholder="value" />
                <button type="button" class="dyn-remove" @click="removeEnv(i)">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
                </button>
              </div>
              <button type="button" class="dyn-add" @click="addEnv">+ 添加环境变量</button>
            </div>
          </div>

          <!-- 信任级别 -->
          <div class="form-row">
            <label class="form-label">信任级别</label>
            <div class="form-control">
              <div class="seg-group">
                <button
                  type="button"
                  class="seg-btn"
                  :class="{ active: form.trust_level === 'untrusted' }"
                  @click="form.trust_level = 'untrusted'"
                >每次确认</button>
                <button
                  type="button"
                  class="seg-btn"
                  :class="{ active: form.trust_level === 'trusted' }"
                  @click="form.trust_level = 'trusted'"
                >信任</button>
              </div>
              <p class="form-hint">
                {{ form.trust_level === "trusted" ? "信任：工具调用免确认，更流畅" : "每次确认：工具调用前弹窗确认，更安全" }}
              </p>
            </div>
          </div>

          <!-- 启用 -->
          <div class="form-row">
            <label class="form-label">启用</label>
            <div class="form-control form-control-inline">
              <Switch v-model="form.enabled" />
              <span class="inline-label">{{ form.enabled ? "启动时自动运行" : "已停用" }}</span>
            </div>
          </div>
        </div>
      </div>

      <!-- 底部 -->
      <div class="modal-footer">
        <div class="footer-left">
          <template v-if="isEdit">
            <template v-if="confirmingDelete">
              <span class="confirm-text">确认删除？</span>
              <button class="btn btn-danger" @click="confirmDelete">确认</button>
              <button class="btn btn-secondary" @click="cancelDelete">取消</button>
            </template>
            <button v-else class="btn btn-danger" @click="onDeleteClick">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="3 6 5 6 21 6" />
                <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
              </svg>
              删除
            </button>
          </template>
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

.modal-body { flex: 1; overflow-y: auto; padding: 0 24px 4px; }

.form-error {
  padding: 10px 14px; margin-bottom: 16px;
  background-color: var(--ip-danger-bg); border: 1px solid var(--ip-danger-border);
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-body-sm-size); color: var(--ip-danger-text);
}

.form-table { display: flex; flex-direction: column; }
.form-row {
  display: flex; align-items: flex-start; padding: 12px 0; gap: 12px;
}
.form-label {
  width: 110px; flex-shrink: 0; padding-top: 6px;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary); line-height: 1.4;
}
.label-req { color: var(--ip-danger-base); }
.label-hint {
  display: block; font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-tertiary); font-size: var(--ip-text-caption-size);
}
.form-control {
  flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px;
}
.form-control-inline { flex-direction: row; align-items: center; gap: 10px; }
.inline-label { font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); }

.input {
  width: 100%; height: 32px; padding: 0 10px;
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md); outline: none;
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
.input-mono { font-family: var(--ip-font-mono); }

.form-hint {
  margin: 0; font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary); line-height: 1.4;
}

/* 动态行（参数 / 环境变量） */
.dyn-row { display: flex; align-items: center; gap: 6px; }
.dyn-key { flex: 0 0 130px; }
.dyn-eq { color: var(--ip-color-text-tertiary); font-family: var(--ip-font-mono); }
.dyn-remove {
  display: flex; align-items: center; justify-content: center;
  width: 28px; height: 32px; flex-shrink: 0;
  color: var(--ip-color-text-tertiary); cursor: pointer;
  background: none; border: none; border-radius: var(--ip-radius-sm);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.dyn-remove:hover { color: var(--ip-danger-base); background-color: var(--ip-danger-bg); }
.dyn-add {
  align-self: flex-start;
  padding: 4px 0; font-size: var(--ip-text-caption-size);
  color: var(--ip-primary-600); cursor: pointer;
  background: none; border: none;
}
.dyn-add:hover { color: var(--ip-primary-700); }

/* 分段选择（信任级别） */
.seg-group {
  display: inline-flex; padding: 2px; gap: 2px;
  background-color: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-md);
}
.seg-btn {
  padding: 5px 14px; font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary); cursor: pointer;
  background: none; border: none; border-radius: var(--ip-radius-sm);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.seg-btn.active {
  color: var(--ip-primary-700); font-weight: var(--ip-font-weight-medium);
  background-color: var(--ip-color-bg-secondary);
  box-shadow: var(--ip-shadow-sm);
}

/* ===== 底部 ===== */
.modal-footer {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 24px 16px; flex-shrink: 0;
}
.footer-left { flex: 1; display: flex; align-items: center; gap: 8px; }
.confirm-text { font-size: var(--ip-text-body-sm-size); color: var(--ip-danger-text); }
.footer-right { display: flex; gap: 8px; }

.btn {
  display: flex; align-items: center; justify-content: center; gap: 6px;
  height: 32px; padding: 0 14px;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-md); cursor: pointer; white-space: nowrap;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-primary { color: white; background-color: var(--ip-primary-600); border: none; }
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
