<script setup lang="ts">
// McpForm.vue — MCP Server 表单（内联组件，供卡片展开编辑 / 新建复用）
// 从原 McpFormModal 提取，去掉弹窗外壳；caption 分区 + 两列布局，极简风格。
// 字段：name, id, description, command, args[], env{}, trust_level, enabled
import { ref, computed } from "vue";
import type { McpServer, NewMcpServer, McpServerUpdate, McpTrustLevel } from "../../types";
import { bridge } from "../../api/bridge";
import MoreMenu from "../common/MoreMenu.vue";

const props = defineProps<{
  server: McpServer | null;
}>();

const emit = defineEmits<{
  saved: [server: McpServer];
  cancel: [];
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

function confirmDelete() {
  if (props.server) emit("delete", props.server);
}
</script>

<template>
  <div class="mcp-form">
    <!-- 配置区：caption 标题 + 右侧操作 -->
    <div class="section-head">
      <span class="section-title">配置</span>
      <div class="section-actions">
        <button class="btn-link" @click="emit('cancel')">取消</button>
        <button class="btn btn-primary btn-sm" :disabled="saving" @click="save">
          {{ saving ? "保存中" : (isEdit ? "保存" : "创建") }}
        </button>
        <MoreMenu
          v-if="isEdit"
          :items="[{ label: '删除', value: 'delete', confirmText: '确认删除？' }]"
          @select="(v) => v === 'delete' && confirmDelete()"
        />
      </div>
    </div>

    <div v-if="error" class="form-error">{{ error }}</div>

    <div class="form-fields">
      <!-- 名称 + ID（两列） -->
      <div class="field-row">
        <div class="field">
          <label class="field-label">名称 <span class="req">*</span></label>
          <input v-model="form.name" type="text" class="input" placeholder="例如：文件系统工具集" />
        </div>
        <div class="field">
          <label class="field-label">ID <span class="req">*</span><span class="hint">不可改</span></label>
          <input v-model="form.id" type="text" class="input" placeholder="filesystem" :disabled="isEdit" :class="{ 'input-disabled': isEdit }" />
        </div>
      </div>

      <!-- 描述 -->
      <div class="field">
        <label class="field-label">描述 <span class="hint">可选</span></label>
        <input v-model="form.description" type="text" class="input" placeholder="说明这个 Server 的用途" />
      </div>

      <!-- 启动命令 -->
      <div class="field">
        <label class="field-label">启动命令 <span class="req">*</span></label>
        <input v-model="form.command" type="text" class="input input-mono" placeholder="如 npx / node / uvx" />
      </div>

      <!-- 参数 -->
      <div class="field">
        <label class="field-label">参数 <span class="hint">每行一个</span></label>
        <div class="dyn-list">
          <div v-for="(_, i) in form.args" :key="'a' + i" class="dyn-row">
            <input v-model="form.args[i]" type="text" class="input input-mono" placeholder="如 @modelcontextprotocol/server-filesystem" />
            <button type="button" class="dyn-remove" @click="removeArg(i)">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
            </button>
          </div>
          <button type="button" class="dyn-add" @click="addArg">+ 添加参数</button>
        </div>
      </div>

      <!-- 环境变量 -->
      <div class="field">
        <label class="field-label">环境变量</label>
        <div class="dyn-list">
          <div v-for="(e, i) in form.envEntries" :key="'e' + i" class="dyn-row">
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

      <!-- 信任级别（启用开关已移至折叠卡片，即时切换） -->
      <div class="field">
        <label class="field-label">信任级别</label>
        <div class="seg-group">
          <button type="button" class="seg-btn" :class="{ active: form.trust_level === 'untrusted' }" @click="form.trust_level = 'untrusted'">每次确认</button>
          <button type="button" class="seg-btn" :class="{ active: form.trust_level === 'trusted' }" @click="form.trust_level = 'trusted'">信任</button>
        </div>
        <p class="field-hint">{{ form.trust_level === "trusted" ? "免确认，更流畅" : "调用前确认，更安全" }}</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mcp-form {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 10px;
}

/* 区段标题（caption，无框靠留白分区） */
.section-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}
.section-title {
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-tertiary);
  letter-spacing: 0.02em;
}
.section-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.form-error {
  padding: 8px 12px;
  margin-bottom: 8px;
  background-color: var(--ip-danger-bg);
  border: 1px solid var(--ip-danger-border);
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-danger-text);
}

.form-fields {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}
.field-row {
  display: flex;
  gap: 10px;
}
.field-row .field {
  flex: 1;
}

.field-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
}
.req { color: var(--ip-danger-base); }
.hint {
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-tertiary);
  font-size: 10px;
}

.input {
  width: 100%;
  height: 30px;
  padding: 0 10px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  outline: none;
  box-sizing: border-box;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.input:focus {
  border-color: var(--color-input-focus-border);
  background-color: var(--color-input-bg);
  box-shadow: 0 0 0 3px rgba(46, 141, 100, 0.12);
}
.input::placeholder { color: var(--ip-color-text-placeholder); }
.input-disabled { opacity: 0.6; cursor: not-allowed; }
.input-mono { font-family: var(--ip-font-mono); }

/* 动态行（参数 / 环境变量） */
.dyn-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.dyn-row {
  display: flex;
  align-items: center;
  gap: 6px;
}
.dyn-key {
  flex: 0 0 120px;
}
.dyn-eq {
  color: var(--ip-color-text-tertiary);
  font-family: var(--ip-font-mono);
}
.dyn-remove {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 30px;
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  background: none;
  border: none;
  border-radius: var(--ip-radius-sm);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.dyn-remove:hover {
  color: var(--ip-danger-base);
  background-color: var(--ip-danger-bg);
}
.dyn-add {
  align-self: flex-start;
  padding: 2px 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-primary-600);
  cursor: pointer;
  background: none;
  border: none;
}
.dyn-add:hover { color: var(--ip-primary-700); }

/* 分段选择（信任级别）—— 边框容器，无背景块，active 用浅底 */
.seg-group {
  display: inline-flex;
  align-self: flex-start;
  background: none;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  overflow: hidden;
}
.seg-btn {
  padding: 5px 14px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  background: none;
  border: none;
  border-right: 1px solid var(--ip-color-border-default);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.seg-btn:last-child {
  border-right: none;
}
.seg-btn.active {
  color: var(--ip-primary-700);
  font-weight: var(--ip-font-weight-medium);
  background-color: var(--ip-primary-50);
}

/* 启用开关行 */
.switch-row {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 30px;
}
.switch-label {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
}

.field-hint {
  margin: 2px 0 0;
  font-size: 10px;
  color: var(--ip-color-text-tertiary);
}

/* 文字按钮（取消） */
.btn-link {
  height: 28px;
  padding: 0 10px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  background: none;
  border: none;
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-link:hover {
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
}

/* 小号主按钮（保存/创建） */
.btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 0 14px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  white-space: nowrap;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-sm { height: 28px; }
.btn-primary {
  color: white;
  background-color: var(--ip-primary-600);
  border: none;
}
.btn-primary:hover { background-color: var(--ip-primary-700); }
.btn-primary:disabled { opacity: 0.6; cursor: not-allowed; }

/* 删除（danger 色文字，hover 加深 + 浅红背景） */
.delete-link {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin-top: 14px;
  padding: 4px 8px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-danger-base);
  background: none;
  border: none;
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.delete-link:hover {
  color: var(--ip-danger-active);
  background-color: var(--ip-danger-bg);
}
</style>
