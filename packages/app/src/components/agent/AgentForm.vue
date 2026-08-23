<script setup lang="ts">
// AgentForm.vue — Agent 表单（内联组件，供卡片展开编辑 / 新建复用）
// 从原 AgentFormModal 提取表单主体与逻辑，去掉弹窗外壳；布局改为垂直（适配卡片宽度）。
// 字段：name, id, model（含隐式 provider）, api_key, base_url, workspace_path
//
// 模型选择 = 可选可输分组下拉（Provider+模型合并，用户心智「我要用哪个模型」）：
// - 选预设条目 = 厂商+模型同时确定，URL 锁定注册表地址（只读，防抄错；测试
//   连接走通的端点由系统回填固化，如智谱标准/Coding 自动匹配）
// - 手输目录外名字（无精确命中）→「使用自定义模型」落 custom：URL 必填可编辑
//   （Ollama/vLLM 等本机或自建 OpenAI 兼容端点从这里进，可免 Key）
// - 目录来自后端 PROVIDERS 注册表（list_providers 单一真相源），hidden 条目
//   （Ollama/custom/旧入口）不进下拉，仅编辑态存量兜底合成一组显示
// Key/URL 字段的规则（requires_key / requires_base_url）由推导出的 provider
// 驱动，与后端校验一致；「测试连接」一次往返两用——验证配置 + 拉取模型并入下拉。
import AvatarField from "../common/AvatarField.vue";
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
import { STYLE_PRESETS, fillPresetName, isBirthDefaultPrompt, type StylePreset } from "../../data/stylePresets";

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
  // 头像（身份出生证字段，非运行时配置）：不传图走名字渐变兜底
  avatar: (props.agent?.avatar as string | null) ?? null,
});

// ---- 头像行 ----




onMounted(async () => {
  providerList.value = await loadProviders();
  // URL 初值：编辑态存量原样；预设厂商为空时显示注册表默认（字段只读但值=实际生效地址）
  if (!form.value.base_url && !isCustomModel.value && currentProvider.value) {
    form.value.base_url = currentProvider.value.default_url;
  }
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

const modelEntry = (provider: string, model: string, note?: string): ComboboxItem => ({
  label: model,
  value: `${provider}::${model}`,
  note,
  data: { provider, model },
});

/** 手输模型（provider=custom）——URL 必填且可编辑的唯一路径 */
const isCustomModel = computed(() => form.value.provider === "custom");

const modelGroups = computed<ComboboxGroup[]>(() => {
  const groups: ComboboxGroup[] = providerList.value
    .filter((p) => !p.hidden)
    .map((p) => {
      const models = [...p.models];
      if (fetched.value?.provider === p.name) {
        models.push(...fetched.value.models.filter((m) => !models.includes(m)));
      }
      return { id: p.name, label: p.label, note: p.note ?? undefined, items: models.map((m) => modelEntry(p.name, m)) };
    });
  // 自定义端点拉取结果合成一组（custom 无静态目录；Ollama/vLLM 手输 URL 后可拉取再点选）
  if (fetched.value?.provider === "custom" && fetched.value.models.length) {
    groups.push({
      id: "custom",
      label: "自定义端点",
      note: "上次拉取",
      items: fetched.value.models.map((m) => modelEntry("custom", m)),
    });
  }
  // 兜底：当前 (provider, model) 不在目录（编辑态存量目录外模型）→ 插进所属组；
  // 归属是 hidden 条目（旧入口如 Ollama/glm-coding）单独合成一组显示与高亮；
  // custom 不合成——手输模型名由 unmatchedLabel 直接回显
  const m = form.value.model;
  if (m && !groups.some((g) => g.items.some((it) => it.value === `${form.value.provider}::${m}`))) {
    const owner = groups.find((g) => g.id === form.value.provider);
    if (owner) {
      owner.items.push(modelEntry(owner.id ?? form.value.provider, m, "当前配置"));
    } else if (!isCustomModel.value) {
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

/** GroupedSelect 受控值：条目存在传 key（显示 label、高亮）；否则传空（unmatchedLabel 回显手输名） */
const modelValue = computed(() => {
  const key = `${form.value.provider}::${form.value.model}`;
  return modelGroups.value.some((g) => g.items.some((it) => it.value === key)) ? key : "";
});

// ---- 选择处理：点目录条目 = 预设（URL 锁定注册表地址）；点「使用自定义模型」= custom（URL 必填可编辑） ----
function onModelSelect(item: ComboboxItem) {
  const data = item.data as { provider?: string; model?: string; custom?: boolean } | undefined;
  connResult.value = null;
  if (data?.custom) {
    form.value.provider = "custom";
    form.value.model = data.model ?? item.label;
    form.value.base_url = ""; // 自定义路径：端点交给用户填（必填）
    return;
  }
  const next = data?.provider ?? form.value.provider;
  if (next !== form.value.provider) {
    // 切厂商才重置 URL：预设厂商换注册表地址（只读显示实际生效端点）；
    // 同厂商换模型不动（可能是存量固化地址/用户填的自定义端点——拉取列表点选场景）
    form.value.base_url = providerList.value.find((p) => p.name === next)?.default_url ?? "";
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
    // 探测地址传参：custom 必须传显式（后端 requires_base_url 校验）；预设厂商
    // 值==注册表默认时传 undefined——后端走 [默认, ...备选] 回退序列（智谱
    // 标准/Coding 自动匹配），不把「显示的默认值」误当显式锁定
    const explicitUrl = isCustomModel.value
      ? form.value.base_url || undefined
      : form.value.base_url !== defaultUrl.value ? form.value.base_url : undefined;
    // 编辑态带 agent_id：表单没填 key 时后端用存量 key 探测（密文不回显）
    const res = await bridge.providers.testConnection(
      form.value.provider,
      explicitUrl,
      form.value.api_key || undefined,
      isEdit.value ? props.agent?.id : undefined,
    );
    connResult.value = res;
    if (res.ok) {
      // 走通地址回填固化（多端点回退时 matched 可能是备选端点，如智谱 Coding；
      // 只读字段系统赋值不受限——「这次测通了」固化成「以后都走它」）
      if (res.matched_url && res.matched_url !== form.value.base_url) {
        form.value.base_url = res.matched_url;
      }
      if (res.models.length > 0) {
        // 拉取结果挂在当前 provider 组（与静态目录去重合并）
        fetched.value = { provider: form.value.provider, models: res.models };
      }
    }
  } catch (e) {
    // 命令本身失败（如未注册 provider / custom 缺地址被 Validation 拦）——同样行内展示
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

/** URL 可编辑性：可见预设厂商锁定（注册表地址，防抄错）；custom/hidden 旧入口（如存量 Ollama 改端口）可编辑 */
const urlEditable = computed(() => !currentProvider.value || currentProvider.value.hidden);

const urlPlaceholder = computed(() =>
  requiresBaseUrl.value
    ? "必填，如 http://localhost:11434/v1（Ollama）"
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

// ---- 风格预设（编辑态：三档素材插入 agent.yaml system_prompt，2026-08-23） ----
// 两层设计（docs/agent-prompt-draft.md）：平台层纪律所有 agent 背（后端
// system_prompt.rs），人格风格归 yaml system_prompt——预设是素材不是档位，
// 插入即用户文本，后续自由修改。与「会话模板」（TemplateStage）是两个概念。
const presetOpen = ref(false);
const presetLoading = ref(false);
const presetError = ref("");
const presetDone = ref("");
const inserting = ref(false);
/** 现有 system_prompt：null=明确无值（免确认）/ undefined=读取失败（按需确认保守） */
const existingPrompt = ref<string | null | undefined>(null);
/** 进入覆盖确认态的档 id（MoreMenu 同款就地二次确认） */
const confirmingPreset = ref<string | null>(null);

async function togglePresets() {
  presetOpen.value = !presetOpen.value;
  if (!presetOpen.value || !props.agent) return;
  presetLoading.value = true;
  presetError.value = "";
  presetDone.value = "";
  confirmingPreset.value = null;
  try {
    const fields = await bridge.agents.yamlFields(props.agent.id);
    existingPrompt.value = fields.system_prompt ?? null;
  } catch {
    existingPrompt.value = undefined; // 未知：写前确认（保守）
  } finally {
    presetLoading.value = false;
  }
}

/** 覆盖确认判据：明确无值 → 免；出生默认句 → 免（最常见操作，拦一道是噪音）；其余（含未知）→ 确认 */
function presetNeedsConfirm(): boolean {
  const cur = existingPrompt.value;
  if (cur === null) return false;
  if (cur === undefined) return true;
  return !isBirthDefaultPrompt(cur, props.agent?.name ?? "");
}

/** 现有内容首行（覆盖确认里展示，让用户认出自己写的东西） */
const existingFirstLine = computed(
  () => (existingPrompt.value ?? "").split("\n").find((l) => l.trim()) || "（空）",
);

function presetPreview(p: StylePreset): string {
  return fillPresetName(p.text, form.value.name).split("\n").slice(0, 3).join("\n");
}

function onPickPreset(p: StylePreset) {
  if (inserting.value) return;
  if (presetNeedsConfirm() && confirmingPreset.value !== p.id) {
    confirmingPreset.value = p.id;
    return;
  }
  void applyPreset(p);
}

async function applyPreset(p: StylePreset) {
  if (!props.agent || inserting.value) return;
  inserting.value = true;
  presetError.value = "";
  try {
    const text = fillPresetName(p.text, form.value.name);
    await bridge.agents.setSystemPrompt(props.agent.id, text);
    existingPrompt.value = text;
    confirmingPreset.value = null;
    presetOpen.value = false;
    presetDone.value = "已写入 agent.yaml，可在文件中继续修改";
  } catch (e) {
    presetError.value = e instanceof Error ? e.message : "写入失败";
  } finally {
    inserting.value = false;
  }
}

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
    error.value = "手动输入的模型须填写 API URL（Ollama 等本机服务如 http://localhost:11434/v1）"; return false;
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
        // 头像双层 Option：null=清空 / string=设定（表单态即真值，无「不改」分支）
        avatar: form.value.avatar,
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
        avatar: form.value.avatar ?? undefined,
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
      <!-- 身份区（出生证字段围头像成组）：头像左侧跨三行；右侧第一行 名称+ID、第二行 模型、第三行 API Key+URL -->
      <div class="identity-row">
        <!-- 头像（AvatarField：hover 更换 + 右上×清空 + 裁剪器，点击/拖入/粘贴三通道）
             列宽固定、高度拉伸跟随右列三行（stretch 链），无宽高比例约束；
             无用户图时走默认头像（链路内置于 EntityAvatar，2026-08-22 全语境统一） -->
        <div class="field identity-avatar">
          <label class="field-label">头像</label>
          <AvatarField
            v-model="form.avatar"
            :name="form.name || form.id || '?'"
            size="lg"
          />
        </div>

        <div class="identity-fields">
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

          <!-- 模型（可选可输分组选择器：选预设即隐式确定厂商；手输目录外名字落自定义） -->
          <div class="field">
            <label class="field-label">模型 <span class="req">*</span></label>
            <GroupedSelect
              :model-value="modelValue"
              :groups="modelGroups"
              allow-custom
              :unmatched-label="form.model"
              placeholder="选择或输入模型名"
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
          </div>

          <!-- API Key + API URL（两列）。Key：状态徽标内嵌输入框右缘；
               URL：label 行内联「测试连接」+ 行内结果（2026-08-22 拍板，替代原提示文字） -->
          <div class="field-row">
            <div class="field">
              <label class="field-label">
                API Key
                <span v-if="requiresKey && !isEdit" class="req">*</span>
              </label>
              <div class="input-wrap" :class="{ 'has-badge': isEdit && requiresKey }">
                <input
                  v-model="form.api_key"
                  type="password"
                  class="input"
                  :placeholder="requiresKey ? (isEdit ? '留空保持现有' : '输入 API Key') : '本地服务无需 API Key'"
              />
                <span v-if="isEdit && requiresKey" :class="props.agent?.has_api_key ? 'badge badge-ok' : 'badge badge-warn'">
                  {{ props.agent?.has_api_key ? "已配置" : "未配置" }}
                </span>
              </div>
            </div>
            <div class="field">
              <div class="label-row">
                <label class="field-label">
                  API URL
                  <span v-if="requiresBaseUrl" class="req">*</span>
                </label>
                <button
                  type="button"
                  class="conn-btn"
                  :disabled="testing"
                  title="验证配置并拉取模型列表（在线服务需先填 API Key；本机/自建端点填好 URL 即可）"
                  @click="runTest"
                >
                  {{ testing ? "测试中…" : "测试连接" }}
                </button>
                <span v-if="connResult" :class="connResult.ok ? 'conn-ok' : 'conn-err'" :title="connResult.error ?? undefined">
                  {{ connResult.ok ? `连接成功，发现 ${connResult.model_count} 个模型` : connResult.error }}
                </span>
              </div>
              <input
                v-model="form.base_url"
                type="text"
                class="input"
                :class="{ 'input-locked': !urlEditable }"
                :readonly="!urlEditable"
                :placeholder="urlPlaceholder"
                :title="urlEditable ? undefined : '预设厂商地址由系统管理（测试连接会自动匹配端点）；如需自定义端点，请在上方模型框手动输入模型名'"
              />
            </div>
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
          <button
            v-if="hasFileConfig"
            type="button"
            class="ws-preset-btn"
            :class="{ active: presetOpen }"
            title="选择一套风格预设插入 agent.yaml 的 system_prompt——插入后就是你的文本，可继续修改"
            @click="togglePresets"
          >风格预设</button>
        </div>
        <!-- 风格预设面板（就地展开：三档素材预览 → 插入 agent.yaml） -->
        <div v-if="presetOpen" class="preset-panel">
          <div v-if="presetLoading" class="preset-status">读取现有配置…</div>
          <template v-else>
            <div v-for="p in STYLE_PRESETS" :key="p.id" class="preset-card">
              <div class="preset-head">
                <span class="preset-name">{{ p.name }}</span>
                <span class="preset-note">{{ p.note }}</span>
              </div>
              <pre class="preset-preview">{{ presetPreview(p) }}</pre>
              <!-- 覆盖确认态（就地，非空非默认句才进） -->
              <div v-if="confirmingPreset === p.id" class="preset-confirm">
                <span class="preset-confirm-text">将覆盖现有 system_prompt（首行：{{ existingFirstLine }}）</span>
                <div class="preset-confirm-actions">
                  <button type="button" class="btn btn-primary btn-sm" :disabled="inserting" @click="applyPreset(p)">
                    {{ inserting ? "写入中…" : "覆盖写入" }}
                  </button>
                  <button type="button" class="btn-link" @click="confirmingPreset = null">取消</button>
                </div>
              </div>
              <button v-else type="button" class="preset-insert" :disabled="inserting" @click="onPickPreset(p)">
                {{ inserting ? "写入中…" : "插入到 agent.yaml" }}
              </button>
            </div>
          </template>
          <div v-if="presetError" class="preset-error">{{ presetError }}</div>
        </div>
        <p v-if="presetDone" class="field-hint preset-done">{{ presetDone }}</p>
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
  gap: var(--ip-spacing-2_5);
}

/* 身份区：头像左（跨三行）、右侧 名称/ID 行 + 模型 行 + API Key/URL 行。
 * 高度拉伸跟随右列（stretch 链：列 → avatar-field → box 逐级 100%）；
 * 宽度按右列名义行高静态写死（用户拍板 2026-08-22，勿改回运行时推导——
 * aspect-ratio 循环尺寸 + ResizeObserver 实测两轮翻车后弃用）。 */
.identity-row {
  display: flex;
  gap: var(--ip-spacing-3);
  align-items: stretch;
}
.identity-avatar {
  flex-shrink: 0;
  /* 宽度 = 右列三行名义高度的推导（改行高/增删行时同步改）：
   * 行1 名称/ID   label18 + gap4 + input32 = 54
   * 行2 模型       label18 + gap4 + select30 = 52（hint 已删 2026-08-22）
   * 行3 Key/URL   label行20（含20px测试按钮） + gap4 + input32 = 56（测试行上移进 label）
   * 行间距 spacing-2_5×2 = 20 → 右列总高 182 − 头像自身 label 行 18+4 = 160 */
  width: 160px;
}
.identity-avatar :deep(.avatar-field) {
  flex: 1;
  align-self: stretch;
  align-items: stretch; /* 头像盒（lg 档 100%×100%）撑满 label 以下列高 */
}
.identity-fields {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2_5);
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
  gap: var(--ip-spacing-2_5);
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
  font-size: var(--ip-text-micro-size);
}

.badge {
  padding: 0 6px;
  font-size: var(--ip-text-micro-size);
  font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-full);
  line-height: 18px;
}
.badge-ok { background-color: var(--ip-success-bg); color: var(--ip-success-text); }
.badge-warn { background-color: var(--ip-warning-bg); color: var(--ip-warning-text); }

.input {
  width: 100%;
  height: var(--ip-input-h-sm);
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
  border-color: var(--ip-color-border-focus);
  background-color: var(--ip-color-bg-input);
  box-shadow: var(--ip-shadow-focus);
}
.input::placeholder { color: var(--ip-color-text-placeholder); }
.input-disabled { opacity: 0.6; cursor: not-allowed; }
/* 预设厂商 URL 锁定态：仍显示实际生效地址（含测试连接固化的端点），但不可编辑 */
.input-locked {
  cursor: default;
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-secondary);
}

/* 连接测试行：小号文字按钮 + 行内结果（绿/红），失败原因可 hover 看全；
   同时是模型列表的唯一拉取入口（一次往返两用） */
/* label 行内联动作（API URL：label + 测试连接按钮 + 行内结果，2026-08-22 拍板） */
.label-row {
  display: flex;
  align-items: center;
  gap: 6px;
}
.label-row .conn-ok,
.label-row .conn-err {
  flex: 1;
  min-width: 0; /* 超长错误省略不换行（ellipsis 见 conn-ok/conn-err 自身） */
}

/* Key 输入框内嵌状态徽标：右缘垂直居中；有徽标时输入文本让位 */
.input-wrap {
  position: relative;
}
.input-wrap .badge {
  position: absolute;
  right: 8px;
  top: 50%;
  transform: translateY(-50%);
  pointer-events: none;
}
.input-wrap.has-badge .input {
  padding-right: 64px;
}
.conn-btn {
  height: 20px;
  padding: 0 8px;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-primary-600);
  background-color: var(--ip-color-primary-soft-bg);
  border: none;
  border-radius: var(--ip-radius-full);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.conn-btn:hover { background-color: var(--ip-primary-100); }
.conn-btn:disabled { opacity: 0.6; cursor: wait; }
.conn-ok {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-success-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 100%;
}
.conn-err {
  font-size: var(--ip-text-micro-size);
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
  height: var(--ip-input-h-sm);
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
  border-color: var(--ip-color-border-focus);
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
  font-size: var(--ip-text-micro-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-primary-tint-text);
  background-color: var(--ip-color-primary-tint-bg);
  border-radius: var(--ip-radius-full);
  white-space: nowrap;
  font-family: var(--ip-font-mono);
}
/* 风格预设入口（与 agent.yaml 徽章同排；面板在下方就地展开） */
.ws-preset-btn {
  height: 22px;
  padding: 0 8px;
  flex-shrink: 0;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  cursor: pointer;
  white-space: nowrap;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.ws-preset-btn:hover,
.ws-preset-btn.active {
  color: var(--ip-primary-600);
  border-color: var(--ip-color-border-focus);
}

.preset-panel {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-2_5);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  background-color: var(--ip-color-bg-secondary);
}
.preset-status {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
}
.preset-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: var(--ip-spacing-2);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  background-color: var(--ip-color-bg-primary);
}
.preset-head {
  display: flex;
  align-items: baseline;
  gap: var(--ip-spacing-2);
}
.preset-name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  white-space: nowrap;
}
.preset-note {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
}
.preset-preview {
  margin: 0;
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-secondary);
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}
.preset-insert {
  align-self: flex-start;
  height: 24px;
  padding: 0 10px;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-primary-600);
  background-color: var(--ip-color-primary-soft-bg);
  border: 1px solid var(--ip-primary-300);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  white-space: nowrap;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.preset-insert:disabled {
  opacity: 0.6;
  cursor: default;
}
.preset-confirm {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}
.preset-confirm-text {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-warning-text);
  word-break: break-all;
}
.preset-confirm-actions {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
}
.preset-error {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-danger-text);
  word-break: break-all;
}
.preset-done {
  color: var(--ip-success-text);
}

/* 头像行（预览 + 小操作钮） */





.field-hint {
  margin: 0;
  font-size: var(--ip-text-micro-size);
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
  height: var(--ip-input-h-sm);
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
  background-color: var(--ip-primary-500);
  border: none;
}
.btn-primary:hover { background-color: var(--ip-primary-600); } /* 档位镜像：浅深主题 hover 方向都正确 */
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
