<script setup lang="ts">
// AgentForm.vue — Agent 表单（内联组件，供卡片展开编辑 / 新建复用）
// 从原 AgentFormModal 提取表单主体与逻辑，去掉弹窗外壳；布局改为垂直（适配卡片宽度）。
// 字段：name, id, model（含隐式 provider）, api_key, base_url, workspace_path
//
// 模型选择 = 一个分组下拉（Provider+模型合并，用户心智「我要用哪个模型」一步到位）：
// - 组头 = 厂商纯标签（不可点），组内条目 = 模型，点选同时推导 provider+model
// - 目录来自后端 PROVIDERS 注册表（list_providers 单一真相源），hidden 条目
//   （自定义/旧入口）不进下拉，仅编辑态存量兜底合成一组显示
// - 在线拉取并入所属组；多端点厂商（智谱标准/Coding）探测自动匹配并回填走通地址
// Key/URL 字段的规则（requires_key / requires_base_url / 默认地址）由推导出的
// provider 驱动，与后端校验一致；「测试连接」与「拉取模型」共用一次往返。
import { ref, computed, onMounted, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { Agent, NewAgent, AgentUpdate, ProviderConnectionResult, ProviderInfo } from "../../types";
import { bridge } from "../../api/bridge";
import { loadProviders } from "../../composables/useProviders";
import GroupedSelect from "../common/GroupedSelect.vue";
import ProviderIcon from "../common/ProviderIcon.vue";
import type { ComboboxGroup, ComboboxItem } from "../common/Combobox.vue";
import MoreMenu from "../common/MoreMenu.vue";

const props = defineProps<{
  agent: Agent | null;
}>();

const emit = defineEmits<{
  saved: [agent: Agent];
  cancel: [];
  delete: [agent: Agent];
}>();

const isEdit = computed(() => !!props.agent);

// ---- Provider 目录（单一真相源；失败降级空表 → 纯手输） ----
const providerList = ref<ProviderInfo[]>([]);

const defaultWorkspace = ref("");

// form.provider 存注册名（由模型选择隐式推导，不再作为独立表单字段）
const form = ref({
  id: props.agent?.id ?? "",
  name: props.agent?.name ?? "",
  provider: props.agent?.provider ?? "openai",
  model: props.agent?.model ?? "",
  api_key: "",
  base_url: props.agent?.base_url ?? "",
  workspace_path: props.agent?.workspace_path ?? "",
});

onMounted(async () => {
  providerList.value = await loadProviders();
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

// ---- 当前 provider 的目录元数据（目录未加载时按「最保守」降级：要 key、有默认地址空） ----
const currentProvider = computed(
  () => providerList.value.find((p) => p.name === form.value.provider) ?? null,
);
const requiresKey = computed(() => currentProvider.value?.requires_key ?? true);
const requiresBaseUrl = computed(() => currentProvider.value?.requires_base_url ?? false);
const defaultUrl = computed(() => currentProvider.value?.default_url ?? "");

// ---- 分组模型目录：组=厂商（optgroup 纯标签，hidden 条目不进下拉），条目 value=`provider::model` ----

/** 在线拉取结果（挂在拉取时的 provider 上，切走不清——列表还在原组里） */
const fetched = ref<{ provider: string; models: string[] } | null>(null);

/** matched_url 自动回填的地址（切厂商时清掉，避免旧端点跟到新厂商） */
const autoFilledUrl = ref<string | null>(null);

const modelEntry = (provider: string, model: string, note?: string): ComboboxItem => ({
  label: model,
  value: `${provider}::${model}`,
  note,
  data: { provider, model },
});

const modelGroups = computed<ComboboxGroup[]>(() => {
  const groups: ComboboxGroup[] = providerList.value
    .filter((p) => !p.hidden)
    .map((p) => {
      const models = [...p.models];
      if (fetched.value?.provider === p.name) {
        models.push(...fetched.value.models.filter((m) => !models.includes(m)));
      }
      const items = models.map((m) => modelEntry(p.name, m));
      // Ollama 免 Key 零门槛：组内常驻拉取条目（拉到真实列表后再选）
      if (p.name === "ollama" && !p.models.length) {
        items.push({ label: "↻ 拉取已安装模型", value: "ollama::__fetch__", data: { provider: "ollama", isFetch: true } });
      }
      return { id: p.name, label: p.label, note: p.note ?? undefined, items };
    });
  // 兜底：当前 (provider, model) 不在目录（编辑态存量目录外模型）→ 插进所属组；
  // 归属组是 hidden 条目（自定义/旧入口，不进下拉）时单独合成一组，显示与高亮有解
  const m = form.value.model;
  if (m && !groups.some((g) => g.items.some((it) => it.value === `${form.value.provider}::${m}`))) {
    const owner = groups.find((g) => g.id === form.value.provider);
    if (owner) {
      owner.items.push(modelEntry(owner.id ?? form.value.provider, m, "当前配置"));
    } else {
      const cp = currentProvider.value;
      groups.push({
        id: form.value.provider,
        label: cp?.label ?? form.value.provider,
        note: cp?.note ?? undefined,
        items: [modelEntry(form.value.provider, m, "当前配置")],
      });
    }
  }
  return groups;
});

/** GroupedSelect 受控值：条目存在传 key（显示 label、高亮）；否则传空（显示 placeholder） */
const modelValue = computed(() => {
  const key = `${form.value.provider}::${form.value.model}`;
  return modelGroups.value.some((g) => g.items.some((it) => it.value === key)) ? key : "";
});

// ---- 选择处理：点条目（select 事件）推导 provider+model；拉取条目就地触发探测 ----
function onModelSelect(item: ComboboxItem) {
  const data = item.data as { provider?: string; model?: string; isFetch?: boolean } | undefined;
  connResult.value = null;
  if (data?.isFetch) {
    form.value.provider = data.provider ?? form.value.provider;
    void runTest();
    return;
  }
  const next = data?.provider ?? form.value.provider;
  if (next !== form.value.provider) {
    // 切厂商：旧厂商的自动回填/注册表地址不跟过来（用户手输的代理地址不动）
    const old = currentProvider.value;
    const known = old ? [old.default_url, ...old.alt_urls.map((a) => a[1])] : [];
    if (form.value.base_url && (form.value.base_url === autoFilledUrl.value || known.includes(form.value.base_url))) {
      form.value.base_url = "";
    }
    autoFilledUrl.value = null;
  }
  form.value.provider = next;
  form.value.model = data?.model ?? item.label;
}

const saving = ref(false);

const hasFileConfig = computed(
  () => isEdit.value && !!props.agent?.workspace_path && !!props.agent?.config_from_file,
);

// ---- 测试连接 / 拉取模型（同一往返；失败是结果不是异常，行内红字展示） ----
const testing = ref(false);
const connResult = ref<ProviderConnectionResult | null>(null);

async function runTest() {
  if (testing.value) return;
  testing.value = true;
  connResult.value = null;
  try {
    // 编辑态带 agent_id：表单没填 key 时后端用存量 key 探测（密文不回显）
    const res = await bridge.providers.testConnection(
      form.value.provider,
      form.value.base_url || undefined,
      form.value.api_key || undefined,
      isEdit.value ? props.agent?.id : undefined,
    );
    connResult.value = res;
    if (res.ok) {
      // 走通地址回填固化（多端点回退时 matched 可能是备选端点，如智谱 Coding）
      if (res.matched_url && res.matched_url !== form.value.base_url) {
        form.value.base_url = res.matched_url;
        autoFilledUrl.value = res.matched_url;
      }
      if (res.models.length > 0) {
        // 拉取结果挂在当前 provider 组（与静态目录去重合并）
        fetched.value = { provider: form.value.provider, models: res.models };
      }
    }
  } catch (e) {
    // 命令本身失败（如未注册 provider / 缺地址被 Validation 拦）——同样行内展示
    connResult.value = {
      ok: false,
      model_count: 0,
      models: [],
      error: e instanceof Error ? e.message : String(e),
      matched_url: null,
    };
  } finally {
    testing.value = false;
  }
}

/** 一键填入注册表默认地址（默认值可见可恢复，治「手填抄错地址」） */
function restoreDefaultUrl() {
  form.value.base_url = defaultUrl.value;
}

const urlPlaceholder = computed(() =>
  requiresBaseUrl.value
    ? "必填，例如 http://localhost:8000/v1"
    : defaultUrl.value
      ? `默认 ${defaultUrl.value}，留空即用`
      : "留空用默认",
);

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

function openInExplorer() {
  if (form.value.workspace_path) {
    revealItemInDir(form.value.workspace_path);
  }
}

const hasWorkspacePath = computed(() => !!form.value.workspace_path?.trim());

const error = ref("");

function validate(): boolean {
  if (!form.value.id.trim()) { error.value = "ID 不能为空"; return false; }
  if (!form.value.name.trim()) { error.value = "名称不能为空"; return false; }
  if (!form.value.model.trim()) { error.value = "模型不能为空"; return false; }
  // Key 必填/格式校验只对「需要 key 的 provider」生效（ollama/custom 本地免鉴权）
  if (requiresKey.value && !isEdit.value && !form.value.api_key.trim()) {
    error.value = "API Key 不能为空"; return false;
  }
  if (requiresKey.value && isEdit.value && form.value.api_key && form.value.api_key.trim().length < 8) {
    error.value = "API Key 格式不正确"; return false;
  }
  if (requiresBaseUrl.value && !form.value.base_url.trim()) {
    error.value = "自定义 Provider 必须填写 API URL"; return false;
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
        provider: form.value.provider,
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
        provider: form.value.provider,
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
  <div class="agent-form">
    <!-- 顶部操作条（展开面板习惯：操作在顶部，始终可见） -->
    <!-- 配置区：caption 标题 + 右侧操作（无框，靠留白分区） -->
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
          <input v-model="form.name" type="text" class="input" placeholder="例如：代码助手" />
        </div>
        <div class="field">
          <label class="field-label">ID <span class="req">*</span><span class="hint">不可改</span></label>
          <input v-model="form.id" type="text" class="input" placeholder="code-assistant" :disabled="isEdit" :class="{ 'input-disabled': isEdit }" />
        </div>
      </div>

      <!-- 模型（Provider 与模型合并为分组选择器：选模型即隐式确定厂商） -->
      <div class="field">
        <label class="field-label">模型 <span class="req">*</span></label>
        <div class="model-group">
          <GroupedSelect
            :model-value="modelValue"
            :groups="modelGroups"
            placeholder="选择模型（按厂商分组）"
            @select="onModelSelect"
          >
            <!-- 关闭态控件前缀：当前归属厂商的图标 -->
            <template #control-icon>
              <ProviderIcon v-if="form.model" :name="form.provider" />
            </template>
            <!-- 组头：厂商品牌图标（未知 provider 渲染为空，不破版式） -->
            <template #group-icon="{ group }">
              <ProviderIcon :name="group.id ?? ''" :size="13" />
            </template>
          </GroupedSelect>
          <button
            type="button"
            class="ws-btn"
            :class="{ 'ws-btn-fetching': testing }"
            :disabled="testing"
            title="在线拉取完整模型列表（公开 API 需先填 API Key；下拉内置目录无需 Key 随时可选）"
            @click="runTest"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 12a9 9 0 1 1-2.64-6.36" /><polyline points="21 3 21 9 15 9" />
            </svg>
          </button>
        </div>
        <p class="field-hint">
          当前服务：{{ currentProvider?.label ?? form.provider }}{{ requiresKey ? "" : "（免 API Key）" }}
        </p>
      </div>

      <!-- API Key + API URL（两列） -->
      <div class="field-row">
        <div class="field">
          <label class="field-label">
            API Key
            <span v-if="requiresKey && !isEdit" class="req">*</span>
            <span v-if="isEdit && requiresKey" :class="props.agent?.has_api_key ? 'badge badge-ok' : 'badge badge-warn'">
              {{ props.agent?.has_api_key ? "已配置" : "未配置" }}
            </span>
          </label>
          <input
            v-model="form.api_key"
            type="password"
            class="input"
            :placeholder="requiresKey ? (isEdit ? '留空保持现有' : '输入 API Key') : '本地服务无需 API Key'"
          />
        </div>
        <div class="field">
          <label class="field-label">
            API URL
            <span v-if="requiresBaseUrl" class="req">*</span>
            <span v-else class="hint">可选</span>
          </label>
          <input v-model="form.base_url" type="text" class="input" :placeholder="urlPlaceholder" />
          <!-- 连接测试行：按钮 + 默认地址可见可一键恢复 + 行内结果 -->
          <div class="conn-row">
            <button type="button" class="conn-btn" :disabled="testing" @click="runTest">
              {{ testing ? "测试中…" : "测试连接" }}
            </button>
            <button
              v-if="defaultUrl"
              type="button"
              class="conn-btn conn-btn-ghost"
              :title="`填入默认地址 ${defaultUrl}`"
              @click="restoreDefaultUrl"
            >填入默认地址</button>
            <span v-if="connResult" :class="connResult.ok ? 'conn-ok' : 'conn-err'" :title="connResult.error ?? undefined">
              {{ connResult.ok ? `连接成功，发现 ${connResult.model_count} 个模型` : connResult.error }}
            </span>
          </div>
        </div>
      </div>

      <!-- 工作区 -->
      <div class="field">
        <label class="field-label">工作区</label>
        <div class="workspace-group">
          <input
            v-model="form.workspace_path"
            type="text"
            class="input workspace-input"
            placeholder="选择工作区目录"
            readonly
            @click="pickWorkspace"
          />
          <button type="button" class="ws-btn" title="选择目录" @click="pickWorkspace">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
          </button>
          <button v-if="hasWorkspacePath" type="button" class="ws-btn ws-btn-open" title="在文件管理器中打开" @click="openInExplorer">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M18 15v2a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2h2" />
              <polyline points="15 3 21 3 21 9" /><line x1="10" y1="14" x2="21" y2="3" />
            </svg>
          </button>
          <span v-if="hasFileConfig" class="ws-badge">agent.yaml</span>
        </div>
        <p class="field-hint">在此目录下创建 <code>agent.yaml</code> 可配置 system_prompt、temperature 等</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.agent-form {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 10px;
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

/* Provider + 模型 两列 */
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

.badge {
  padding: 0 6px;
  font-size: 10px;
  font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-full);
  line-height: 18px;
}
.badge-ok { background-color: var(--ip-success-bg); color: var(--ip-success-text); }
.badge-warn { background-color: var(--ip-warning-bg); color: var(--ip-warning-text); }

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

/* 模型 GroupedSelect + 拉取按钮（与工作区 input+btn 同语系） */
.model-group {
  display: flex;
  gap: 6px;
  align-items: center;
}
.model-group .gs {
  flex: 1;
  min-width: 0;
}
/* 拉取中旋转反馈 */
.ws-btn-fetching svg {
  animation: ws-spin 0.9s linear infinite;
}
@keyframes ws-spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

/* 连接测试行：小号文字按钮 + 行内结果（绿/红），失败原因可 hover 看全 */
.conn-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 18px;
  flex-wrap: wrap;
}
.conn-btn {
  height: 20px;
  padding: 0 8px;
  font-size: 10px;
  color: var(--ip-primary-600);
  background-color: var(--ip-color-primary-soft-bg);
  border: none;
  border-radius: var(--ip-radius-full);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.conn-btn:hover { background-color: var(--ip-primary-100); }
.conn-btn:disabled { opacity: 0.6; cursor: wait; }
.conn-btn-ghost {
  color: var(--ip-color-text-tertiary);
  background-color: var(--ip-color-bg-tertiary);
}
.conn-btn-ghost:hover { color: var(--ip-color-text-secondary); background-color: var(--ip-color-bg-secondary); }
.conn-ok {
  font-size: 10px;
  color: var(--ip-success-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 100%;
}
.conn-err {
  font-size: 10px;
  color: var(--ip-danger-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 100%;
}

/* 工作区 */
.workspace-group {
  display: flex;
  gap: 6px;
  align-items: center;
}
.workspace-input {
  flex: 1;
  cursor: pointer;
}
.ws-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  flex-shrink: 0;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.ws-btn:hover {
  background-color: var(--ip-color-bg-secondary);
  color: var(--ip-primary-600);
  border-color: var(--color-input-focus-border);
}
.ws-btn-open {
  color: var(--ip-primary-600);
  border-color: var(--ip-primary-300);
  background-color: var(--ip-color-primary-soft-bg);
}
.ws-badge {
  display: inline-flex;
  align-items: center;
  height: 22px;
  padding: 0 8px;
  font-size: 10px;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-primary-tint-text);
  background-color: var(--ip-color-primary-tint-bg);
  border-radius: var(--ip-radius-full);
  white-space: nowrap;
  font-family: var(--ip-font-mono);
}

.field-hint {
  margin: 0;
  font-size: 10px;
  color: var(--ip-color-text-tertiary);
  line-height: 1.4;
}
.field-hint code {
  font-family: var(--ip-font-mono);
  background: var(--ip-color-bg-tertiary);
  padding: 0 4px;
  border-radius: var(--ip-radius-sm);
}

/* 区段标题（caption 小标题，无框，靠留白分区） */
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
.btn-sm {
  height: 28px;
  padding: 0 14px;
}

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

.btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 30px;
  padding: 0 14px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  white-space: nowrap;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-primary {
  color: white;
  background-color: var(--ip-primary-600);
  border: none;
}
.btn-primary:hover { background-color: var(--ip-primary-700); }
.btn-primary:disabled { opacity: 0.6; cursor: not-allowed; }
.btn-secondary {
  color: var(--ip-color-text-secondary);
  background-color: transparent;
  border: 1px solid var(--ip-color-border-default);
}
.btn-secondary:hover {
  background-color: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
}
.btn-danger {
  color: var(--ip-danger-base);
  background-color: transparent;
  border: 1px solid var(--ip-danger-border);
}
.btn-danger:hover {
  background-color: var(--ip-danger-bg);
  color: var(--ip-danger-active);
}
</style>
