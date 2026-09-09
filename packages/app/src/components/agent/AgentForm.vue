<script setup lang="ts">
// AgentForm.vue — Agent 表单（内联组件，供卡片展开编辑 / 新建复用）
// 从原 AgentFormModal 提取表单主体与逻辑，去掉弹窗外壳；布局改为垂直（适配卡片宽度）。
// 字段：name, id, model（含隐式 provider）, api_key, base_url, workspace_path
//
// 模型来源两形态由 referenceMode 决定（ModelProfile Phase 3 + 2026-09-09 双入口拍板）：
// - 新建 = 双入口：「引用已有」（默认——有实体可复用，链选择器选档）/「手动填写」
//   （既有心智：可选可输分组下拉 + Key/URL + 测试连接，保存时后端自动物化为
//   「设置-模型」实体 + 引用，同配置复用既有实体）
// - 编辑 = 恒「引用已有」：合并 tag 选择器——主模型与降级链同框（首 tag 即主
//   模型，拖拽/键盘调序），Key 与端点在「设置-模型」统一维护
// - 旁路（提案 CreateAgent 等凭据不齐的 legacy 行）编辑时选实体即转正
import AvatarField from "../common/AvatarField.vue";
import { ExternalLink, ChevronDown, X, Plus, HelpCircle } from "@lucide/vue";
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from "vue";
import { useRouter } from "vue-router";
import draggable from "vuedraggable";
import { open } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { Agent, NewAgent, ModelChainTestResult, ProviderConnectionResult, ProviderInfo } from "../../types";
import { bridge } from "../../api/bridge";
import { loadProviders } from "../../composables/useProviders";
import { useModelProfiles, profileById } from "../../composables/useModelProfiles";
import GroupedSelect from "../common/GroupedSelect.vue";
import ProviderIcon from "../common/ProviderIcon.vue";
import type { ComboboxGroup, ComboboxItem } from "../common/Combobox.vue";
import MoreMenu from "../common/MoreMenu.vue";
import StylePresetPicker from "./StylePresetPicker.vue";
import { fillPresetName, type StylePreset } from "../../data/stylePresets";

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

// ---- 模型配置链（编辑态；ModelProfile Phase 3 合并 tag 选择器）----
// 有序 profile id 链：首 = 主模型，其余 = 降级链（额度耗尽 / 限流重试耗尽 /
// 端点不可达时按序换档，链尽走原错误终态）。主/降级同框排序（用户拍板 2026-09-08：
// 不做按钮，拖拽调序 + 键盘 ←/→ 无障碍通道）；去重在候选侧（选一少一）。
// ⚠️ 换档 × N 档最长退避 = 档数 × 7s，链长建议 ≤3（视觉读取链同款提醒见设置页）。
// 引用选择器数据源（模块级共享缓存，与设置-模型页同表）
const { profiles, loadModelProfiles } = useModelProfiles();
/** 编辑态链初值：[主档, ...降级链]；legacy 行（无引用）空链待选 */
const chainIds = ref<string[]>(
  props.agent?.model_profile_id
    ? [props.agent.model_profile_id, ...(props.agent.fallback_profile_ids ?? [])]
    : [],
);
/** 下拉候选：未入链的实体（选一少一，天然去重——已选项不会再出现） */
const chainCandidates = computed(() =>
  profiles.value.filter((p) => !chainIds.value.includes(p.id)),
);
/** vuedraggable item-key：id 唯一（候选已排除在链项），直接当 key */
const itemKeyFn = (id: string) => id;

/** 健康点词表（slug 对齐后端 ProfileHealth 契约；语义色圆点 + 文字） */
const PROFILE_HEALTH_META: Record<string, { tone: string; label: string }> = {
  ok: { tone: "success", label: "正常" },
  quota: { tone: "danger", label: "额度耗尽" },
  auth: { tone: "danger", label: "Key 无效" },
  model_not_found: { tone: "danger", label: "模型不存在" },
  unknown: { tone: "danger", label: "调用异常" },
  rate_limited: { tone: "warning", label: "限流中" },
  network: { tone: "warning", label: "端点不可达" },
};
/** 档位健康态（null = 未调用过，不冒充正常） */
function healthOf(id: string): { tone: string; label: string } | null {
  const slug = profileById(profiles.value, id)?.last_health;
  if (!slug) return null;
  return PROFILE_HEALTH_META[slug] ?? { tone: "neutral", label: slug };
}
/** tag 展示名（悬空 id = profile 已删的罕见态——删除守卫正常会拦，兜底诚实标注） */
function tagLabelOf(id: string): string {
  return profileById(profiles.value, id)?.alias ?? "（配置已删除）";
}

function addCandidate(id: string) {
  if (!chainCandidates.value.some((p) => p.id === id)) return;
  chainIds.value.push(id); // 追加到链尾（主模型 = 首 tag，调序靠拖拽/键盘）
}
function removeAt(i: number) {
  chainIds.value.splice(i, 1);
}
/** 键盘换位（无障碍通道）：←/→ 与相邻 tag 交换，焦点跟人走 */
function moveAt(i: number, delta: -1 | 1) {
  const j = i + delta;
  if (j < 0 || j >= chainIds.value.length) return;
  const ids = [...chainIds.value];
  [ids[i], ids[j]] = [ids[j], ids[i]];
  chainIds.value = ids;
  focusTag(j);
}
function onTagKeydown(e: KeyboardEvent, i: number) {
  if (e.key === "ArrowLeft") {
    e.preventDefault();
    moveAt(i, -1);
  } else if (e.key === "ArrowRight") {
    e.preventDefault();
    moveAt(i, 1);
  }
}
function focusTag(i: number) {
  nextTick(() => {
    chainBoxRef.value
      ?.querySelector<HTMLElement>(`.chain-tag[data-idx="${i}"]`)
      ?.focus();
  });
}

// ---- 下拉开合（外点关闭；Esc 关闭；新建入口跳设置-模型）----
const chainOpen = ref(false);
const chainWrapRef = ref<HTMLElement | null>(null);
const chainBoxRef = ref<HTMLElement | null>(null);
function toggleChain() {
  chainOpen.value = !chainOpen.value;
}
function closeChain() {
  chainOpen.value = false;
}
function onDocClick(e: MouseEvent) {
  if (!chainWrapRef.value?.contains(e.target as Node)) closeChain();
}
const router = useRouter();
/** 下拉内「＋ 新建模型配置」→ 设置-模型页（编辑想换全新配置由这里兜住） */
function goCreateProfile() {
  closeChain();
  void router.push({ name: "SettingsModels" });
}

// ---- 链路测试（2026-09-09 ④）：模拟 agent 发送路径——主模型起按序仿真换档，
// 报告链上首个可用档位。列模型是鉴权层动作（智谱 Coding 1113 假绿实案），
// 真发一条摘要请求才验得出对话权益；换档判据与运行时降级链同一张分类表 ----
const testingChain = ref(false);
const chainTest = ref<ModelChainTestResult | null>(null);

/** 结果文案：成功区分首档/降级档（降级命中本身是有效配置，但主模型该看一眼） */
const chainTestText = computed(() => {
  const r = chainTest.value;
  if (!r) return "";
  if (!r.ok) return r.error ?? "链路测试失败";
  const mainHit = r.profile_id === chainIds.value[0];
  return mainHit
    ? `连接成功 · 「${r.alias}」${r.model}`
    : `主模型不可用，降级到「${r.alias}」${r.model}`;
});

async function runChainTest() {
  if (testingChain.value || !chainIds.value.length) return;
  testingChain.value = true;
  chainTest.value = null;
  try {
    const res = await bridge.providers.testModelChain([...chainIds.value]);
    chainTest.value = res;
    // 测试是真实逐档调用——健康点即时刷新（同模型页「测试」语义）
    await loadModelProfiles(true);
  } catch (e) {
    chainTest.value = {
      ok: false, profile_id: null, alias: null, model: null,
      error: e instanceof Error ? e.message : String(e),
    };
  } finally {
    testingChain.value = false;
  }
}

// 链一变旧结论即失效（增删/移除/拖拽换序——主模型换人，结论换了世界）
watch(chainIds, () => { chainTest.value = null; }, { deep: true });

const error = ref("");

// ---- 新建态双入口（2026-09-09 用户拍板：引用既有 + 手动填写都要有）----
// 编辑恒引用（只保留引用入口）；新建默认档按实体库状态定——暖缓存有实体时
// 「引用已有」起步（复用是实体库核心价值），冷缓存在加载完成后回评翻转。
// guard：未点过 pill + 表单未沾手（模型/Key 均空、链空）才翻，防止覆盖已填内容。
const modelMode = ref<"reference" | "manual">(
  isEdit.value || profiles.value.length ? "reference" : "manual",
);
const modeTouched = ref(false);
/** 引用形态渲染判据：模板 / 校验 / 载荷三处统一走它（编辑恒 true） */
const referenceMode = computed(() => isEdit.value || modelMode.value === "reference");
function setModelMode(mode: "reference" | "manual") {
  if (modelMode.value === mode) return;
  modeTouched.value = true;
  modelMode.value = mode;
  error.value = ""; // 换轨后旧校验错误不再适用
}

// ---- 头像行 ----




onMounted(async () => {
  providerList.value = await loadProviders();
  // 配置链数据源（失败降级空表——useModelProfiles 内已兜底，不卡表单）；
  // 新建态冷缓存加载完成后回评默认档：有实体且表单未沾手 → 翻「引用已有」
  void loadModelProfiles().then(() => {
    if (
      !isEdit.value && !modeTouched.value && modelMode.value === "manual"
      && profiles.value.length > 0 && !chainIds.value.length
      && !form.value.model && !form.value.api_key
    ) {
      modelMode.value = "reference";
    }
  });
  // URL 初值（新建态专属）：预设厂商为空时显示注册表默认（字段只读但值=实际生效地址）
  if (!isEdit.value && !form.value.base_url && !isCustomModel.value && currentProvider.value) {
    form.value.base_url = currentProvider.value.default_url;
  }
  // 下拉外点关闭（挂在 document 上，卸载时移除）
  document.addEventListener("click", onDocClick);
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

onBeforeUnmount(() => {
  document.removeEventListener("click", onDocClick);
});

// ---- 当前 provider 的目录元数据（目录未加载时按「最保守」降级：要 key、有默认地址空） ----
const currentProvider = computed(
  () => providerList.value.find((p) => p.name === form.value.provider) ?? null,
);
const requiresKey = computed(() => currentProvider.value?.requires_key ?? true);
/** Key 申请页直达（注册表 key_url 单一真相源；免 key 厂商无此链接） */
const keyApplyUrl = computed(() => currentProvider.value?.key_url ?? "");
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
      return { id: p.name, label: p.label, items: models.map((m) => modelEntry(p.name, m)) };
    });
  // 自定义端点拉取结果合成一组（custom 无静态目录；Ollama/vLLM 手输 URL 后可拉取再点选）
  if (fetched.value?.provider === "custom" && fetched.value.models.length) {
    groups.push({
      id: "custom",
      label: "自定义端点",
      items: fetched.value.models.map((m) => modelEntry("custom", m)),
    });
  }
  // 目录外兜底分支已随编辑态手动模型入口退役（Phase 3）：选择器只在新建态渲染，
  // 手输目录外名字落 custom 由 unmatchedLabel 回显
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
    // agent_id 恒不传：Key/URL 行只在新建态渲染（编辑态凭据在 profile 侧维护，
    // 测试连接对 profile 走设置-模型页的「测试」按钮）
    const res = await bridge.providers.testConnection(
      form.value.provider,
      explicitUrl,
      form.value.api_key || undefined,
      undefined,
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

// ---- 端点切换：带备选端点的可见预设厂商（当前仅智谱）----
// 智谱标准/Coding 两套端点 key 不通用，Coding 套餐额度只在 Coding 端点生效；
// 「测试连接」的自动回退救不了「标准端点列模型能通、对话报 1113」的假绿
// （列模型是鉴权层动作，不代表该端点认可对话权益——生产实案 2026-08-26），
// 必须给显式入口。选中的端点地址仍是系统管理的注册表值（只读显示，防抄错）。
const endpointOptions = computed(() => {
  const p = currentProvider.value;
  if (!p || p.hidden || p.alt_urls.length === 0) return [];
  return [
    { label: "标准端点", url: p.default_url },
    ...p.alt_urls.map(([label, url]) => ({ label, url })),
  ];
});

/** 当前选中端点（按 URL 匹配；存量固化地址不在已知清单时无高亮，点选即归位） */
const selectedEndpoint = computed(
  () => endpointOptions.value.find((o) => o.url === form.value.base_url)?.url ?? null,
);

function pickEndpoint(url: string) {
  if (form.value.base_url === url) return;
  connResult.value = null; // 换端点后旧探测结论失效
  form.value.base_url = url;
}

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

// ---- 风格预设（创建/编辑两用弹层，2026-08-23） ----
// 两层设计（docs/agent-prompt-draft.md）：平台层纪律所有 agent 背（后端
// system_prompt.rs），人格风格归 yaml system_prompt——预设是素材不是档位，
// 插入即用户文本，后续自由修改。与「会话模板」（TemplateStage）是两个概念。
// 创建态：选中随表单保存进 NewAgent.system_prompt（出生 yaml 即带，零后端改动）；
// 编辑态：选档即写 agent.yaml（覆盖确认在弹层内）。
const pickerOpen = ref(false);
const presetFetching = ref(false);
const presetError = ref("");
const presetDone = ref("");
const inserting = ref(false);
/** 现有 system_prompt：null=明确无值（免确认）/ undefined=读取失败（按需确认保守） */
const existingPrompt = ref<string | null | undefined>(null);
/** 创建态已选档（随保存写入；null=用默认通用句） */
const selectedPreset = ref<StylePreset | null>(null);

async function openPicker() {
  presetError.value = "";
  presetDone.value = "";
  // 编辑态先拉现有 system_prompt（覆盖确认判据），拉完再开——避免弹层闪确认态
  if (isEdit.value && props.agent) {
    presetFetching.value = true;
    try {
      const fields = await bridge.agents.yamlFields(props.agent.id);
      existingPrompt.value = fields.system_prompt ?? null;
    } catch {
      existingPrompt.value = undefined; // 未知：写前确认（保守）
    } finally {
      presetFetching.value = false;
    }
  }
  pickerOpen.value = true;
}

/** 编辑态：弹层确认已过 → 写 agent.yaml */
async function onPickPreset(p: StylePreset) {
  if (!props.agent || inserting.value) return;
  inserting.value = true;
  presetError.value = "";
  try {
    const text = fillPresetName(p.text, form.value.name);
    await bridge.agents.setSystemPrompt(props.agent.id, text);
    existingPrompt.value = text;
    pickerOpen.value = false;
    presetDone.value = "已写入 agent.yaml，可在文件中继续修改";
  } catch (e) {
    presetError.value = e instanceof Error ? e.message : "写入失败";
  } finally {
    inserting.value = false;
  }
}

/** 创建态：选中/清除（随保存写入） */
function onSelectPreset(p: StylePreset | null) {
  selectedPreset.value = p;
  pickerOpen.value = false;
}

function validate(): boolean {
  if (!form.value.id.trim()) { error.value = "ID 不能为空"; return false; }
  if (!form.value.name.trim()) { error.value = "名称不能为空"; return false; }
  // 引用形态（编辑恒引用 / 新建「引用已有」）：模型身份来自配置链（首 = 主模型）
  // ——厂商/模型/Key/URL 校验豁免（快照列族由后端解析产生，本表单不再持有；
  // Key 维护在「设置-模型」侧）
  if (referenceMode.value) {
    if (!chainIds.value.length) {
      error.value = "请至少选择一个模型配置（第一个为主模型）";
      return false;
    }
    error.value = "";
    return true;
  }
  if (!form.value.model.trim()) { error.value = "模型不能为空"; return false; }
  // Key 必填校验只对「需要 key 的 provider」生效（ollama/custom 本地免鉴权）；
  // 换厂商必填新 Key 闸随之退役——表单只在新建态渲染，Key 随厂商同批提交，
  // 不存在「旧厂商 Key 打新端点」的残留形态
  if (requiresKey.value && !form.value.api_key.trim()) {
    error.value = "API Key 不能为空"; return false;
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
      // 编辑态：模型身份 = 配置链拆装（首 = 主模型引用，其余按序 = 降级链）。
      // 只发出生证 + 引用族——provider/model/base_url 属快照列族（后端解析产生，
      // 同批手填会被冲突校验拒）；Key 在 profile 侧维护，无 rotateKey
      const updated = await bridge.agents.update({
        id: currentAgent.id,
        name: form.value.name,
        workspace_path: form.value.workspace_path || null,
        // 头像双层 Option：null=清空 / string=设定（表单态即真值，无「不改」分支）
        avatar: form.value.avatar,
        model_profile_id: chainIds.value[0] ?? null,
        // 降级链随批提交（Some 权威语义：[] = 显式清空）
        fallback_profile_ids: chainIds.value.slice(1),
      });
      const fresh = await bridge.agents.list();
      const real = fresh.find((a) => a.id === currentAgent.id);
      emit("saved", real ?? updated);
    } else {
      // 双入口载荷分支（2026-09-09）：「引用已有」走链拆装（同编辑态）——
      // provider/model/api_key 空串占位（后端引用模式跳过必填、快照列由 profile
      // 解析产生）；「手动填写」照旧四字段——后端 create 在保存时自动物化
      // （同配置复用既有实体）并落引用，UI 无需感知物化细节（Phase 3 拍板）
      const input: NewAgent = {
        id: form.value.id,
        name: form.value.name,
        ...(referenceMode.value
          ? {
              provider: "",
              model: "",
              api_key: "",
              model_profile_id: chainIds.value[0],
              fallback_profile_ids: chainIds.value.slice(1),
            }
          : {
              provider: form.value.provider,
              model: form.value.model,
              api_key: form.value.api_key,
              base_url: form.value.base_url || undefined,
            }),
        workspace_path: form.value.workspace_path || undefined,
        avatar: form.value.avatar ?? undefined,
        // 风格预设（创建态选档）：出生 yaml 即带这段文本（后端用 row.system_prompt
        // 生成默认 yaml，空则落通用句——零后端改动）
        system_prompt: selectedPreset.value
          ? fillPresetName(selectedPreset.value.text, form.value.name)
          : undefined,
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
        <div class="field identity-avatar" :class="{ 'identity-avatar--ref': referenceMode }">
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

          <!-- 模型：形态由 referenceMode 决定——编辑恒引用；新建「引用已有」/「手动填写」双入口切换 -->
          <div class="field">
            <div class="label-row">
              <label class="field-label">模型 <span class="req">*</span></label>
              <!-- 引用态链语义塞问号 tip（2026-09-09 ③：原 field-hint 精简入此，表单更清爽；
                   形态与 ModelSettings 端点 URL 问号一致——全系统同一模式） -->
              <span
                v-if="referenceMode"
                class="tip-icon"
                data-tip="第一个为主模型，其余按序降级——额度耗尽 / 限流 / 端点不可达时自动换档，建议 ≤3 档。&#10;拖拽或方向键调整顺序；Key 与端点在「设置-模型」维护。"
              >
                <HelpCircle :size="14" />
              </span>
              <!-- 新建态双入口（2026-09-09 拍板）：引用已有（默认，有实体可复用）/ 手动填写（保存时自动物化） -->
              <template v-if="!isEdit">
                <span v-if="modelMode === 'manual'" class="label-note">保存后 Key 与端点将存入「设置-模型」</span>
                <div class="mode-pills">
                  <button type="button" class="mode-pill" :class="{ active: modelMode === 'reference' }" @click="setModelMode('reference')">引用已有</button>
                  <button type="button" class="mode-pill" :class="{ active: modelMode === 'manual' }" @click="setModelMode('manual')">手动填写</button>
                </div>
              </template>
            </div>
            <!-- 引用形态：合并 tag 选择器——主模型与降级链同框（首 tag = 主），拖拽/键盘调序 -->
            <template v-if="referenceMode">
              <div ref="chainWrapRef" class="chain-wrap" @keydown.escape="closeChain">
                <div ref="chainBoxRef" class="chain-box" :class="{ open: chainOpen }" @click="toggleChain">
                  <!-- 开合点击面 = 整个 box 空白区（含 tag 容器空位）；tag 自身止冒泡——
                       它是拖拽/键盘目标，点它不切换开合 -->
                  <draggable
                    v-model="chainIds"
                    :item-key="itemKeyFn"
                    :animation="150"
                    filter=".tag-x"
                    class="chain-tags"
                  >
                    <template #item="{ element, index }">
                      <span
                        class="chain-tag"
                        :class="{ 'chain-tag--main': index === 0 }"
                        :data-idx="index"
                        tabindex="0"
                        title="拖拽或方向键调整顺序（第一个为主模型）"
                        @click.stop
                        @keydown="onTagKeydown($event, index)"
                      >
                        <span class="tag-dot" :class="`tag-dot--${healthOf(element)?.tone ?? 'neutral'}`" aria-hidden="true"></span>
                        <span class="tag-name">{{ tagLabelOf(element) }}</span>
                        <span v-if="index === 0" class="tag-main">主</span>
                        <button type="button" class="tag-x" title="移除" @click.stop="removeAt(index)">
                          <X :size="11" />
                        </button>
                      </span>
                    </template>
                  </draggable>
                  <span v-if="!chainIds.length" class="chain-placeholder">选择模型配置（第一个为主模型）</span>
                  <button
                    type="button"
                    class="chain-chevron"
                    :aria-expanded="chainOpen"
                    title="展开候选"
                    @click.stop="toggleChain"
                  >
                    <ChevronDown :size="14" />
                  </button>
                </div>
                <div v-if="chainOpen" class="chain-menu">
                  <button
                    v-for="p in chainCandidates"
                    :key="p.id"
                    type="button"
                    class="chain-option"
                    @click="addCandidate(p.id)"
                  >
                    <span class="tag-dot" :class="`tag-dot--${healthOf(p.id)?.tone ?? 'neutral'}`" aria-hidden="true"></span>
                    <span class="chain-option-name">{{ p.alias }}</span>
                    <span class="chain-option-meta">
                      <ProviderIcon :name="p.provider" :size="12" />
                      <span class="chain-option-model">{{ p.model }}</span>
                    </span>
                  </button>
                  <p v-if="!chainCandidates.length" class="chain-empty">没有可选的模型配置</p>
                  <button type="button" class="chain-option chain-option--new" @click="goCreateProfile">
                    <Plus :size="12" />
                    <span>新建模型配置</span>
                  </button>
                </div>
              </div>
              <!-- 链路测试（④ 2026-09-09）：模拟 agent 发送路径——主模型起按序仿真换档，
                   报告链上首个可对话档位（列模型是鉴权层动作，真发一条才作数） -->
              <div class="chain-test-row">
                <span
                  v-if="chainTest"
                  :class="chainTest.ok ? 'conn-ok' : 'conn-err'"
                  :title="chainTestText"
                >{{ chainTestText }}</span>
                <button
                  type="button"
                  class="conn-btn"
                  :disabled="testingChain || !chainIds.length"
                  title="模拟 agent 发送路径：从主模型起按序尝试，报告实际能用的档位"
                  @click="runChainTest"
                >
                  {{ testingChain ? "测试中…" : "测试连接" }}
                </button>
              </div>
            </template>
            <!-- 手动形态：可选可输分组选择器（选预设即隐式确定厂商；手输目录外名字落自定义） -->
            <GroupedSelect
              v-else
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

          <!-- API Key + API URL（两列；手动形态专属——引用形态 Key/端点在 profile 侧维护）。
               URL：label 行内联「测试连接」+ 行内结果（2026-08-22 拍板，替代原提示文字） -->
          <div v-if="!referenceMode" class="field-row">
            <div class="field">
              <label class="field-label">
                API Key
                <span v-if="requiresKey" class="req">*</span>
                <a
                  v-if="requiresKey && keyApplyUrl"
                  :href="keyApplyUrl"
                  target="_blank"
                  class="key-apply-link"
                  title="打开厂商 Key 管理页（外链）"
                >去申请<ExternalLink :size="10" /></a>
              </label>
              <input
                v-model="form.api_key"
                type="password"
                class="input"
                :placeholder="requiresKey ? '输入 API Key' : '本地服务无需 API Key'"
              />
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
              <!-- 测试连接副文案常显（原 hover tooltip 不可发现；「测试连接」是拉模型唯一入口） -->
              <p class="field-hint">「测试连接」验证配置并拉取模型列表；在线服务需先填 API Key，本机/自建端点填好 URL 即可</p>
              <div v-if="endpointOptions.length > 1" class="endpoint-row" title="智谱标准/Coding 端点 key 不通用：Coding 套餐额度只在 Coding 端点生效">
                <button
                  v-for="opt in endpointOptions"
                  :key="opt.url"
                  type="button"
                  class="endpoint-opt"
                  :class="{ active: selectedEndpoint === opt.url }"
                  @click="pickEndpoint(opt.url)"
                >{{ opt.label }}</button>
              </div>
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
          <!-- 风格预设入口（创建/编辑两用；创建态选中随保存写入，编辑态选档即写 yaml） -->
          <button
            v-if="!isEdit || hasWorkspacePath"
            type="button"
            class="ws-preset-btn"
            :class="{ active: pickerOpen, chosen: !!selectedPreset }"
            :disabled="presetFetching"
            :title="isEdit
              ? '选择一套风格预设插入 agent.yaml 的 system_prompt——插入后就是你的文本，可继续修改'
              : '选一套风格预设作为 system_prompt 起点——保存时写入，不选用默认'"
            @click="openPicker"
          >{{ selectedPreset ? `风格 · ${selectedPreset.name}` : "风格预设" }}</button>
        </div>
        <p v-if="presetDone" class="field-hint preset-done">{{ presetDone }}</p>
        <p class="field-hint">在此目录下创建 <code>agent.yaml</code> 可配置 system_prompt、temperature 等</p>
      </div>
    </div>

    <!-- 风格预设弹层（Teleport 到 body，创建/编辑两用） -->
    <StylePresetPicker
      v-if="pickerOpen"
      :mode="isEdit ? 'edit' : 'create'"
      :agent-name="form.name || form.id || ''"
      :selected-id="selectedPreset?.id ?? null"
      :existing-prompt="isEdit ? existingPrompt : null"
      :inserting="inserting"
      :error="presetError"
      @close="pickerOpen = false"
      @select="onSelectPreset"
      @pick="onPickPreset"
    />
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
/* 引用形态宽度（行3 不渲染、行2 换链选择器，按同法推导）：
 * 行2 = label18 + gap4 + chain-box32 + gap4 + 链路测试行20 = 78
 * 右列总高 54+10+78 = 142 − 头像 label 22 = 120（正方头像盒）
 * 新建引用态 label 行含 mode-pills（22px）→ 盒高 124，4px 差不设第二档 */
.identity-avatar--ref {
  width: 120px;
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
/* Key 申请直达（注册表 key_url；仅需要 Key 的厂商显示） */
.key-apply-link {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  font-size: var(--ip-text-micro-size);
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-tertiary);
  text-decoration: none;
}
.key-apply-link:hover { color: var(--ip-color-text-secondary); }
.hint {
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-micro-size);
}

/* label 行右侧可见性提示（新建态模型框：保存即转实体） */
.label-note {
  margin-left: auto;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
}
/* 新建态双入口切换 pills（复用端点胶囊形态；右锚定，手动档时与左侧 hint 同排） */
.mode-pills {
  margin-left: auto;
  display: flex;
  gap: 4px;
}

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

/* 端点切换（智谱标准/Coding）：URL 框下的分段小胶囊，选中态主色浅底 */
.endpoint-row {
  display: flex;
  gap: 4px;
  margin-top: 6px;
}
.endpoint-opt,
.mode-pill {
  padding: 0 10px;
  font-size: var(--ip-text-micro-size);
  line-height: 20px;
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.endpoint-opt:hover,
.mode-pill:hover {
  border-color: var(--ip-color-border-focus);
  color: var(--ip-color-text-primary);
}
.endpoint-opt.active,
.mode-pill.active {
  color: var(--ip-primary-600);
  background-color: var(--ip-color-primary-soft-bg);
  border-color: transparent;
  font-weight: var(--ip-font-weight-medium);
}

/* 配置链选择器（编辑态）：box 内 tag 有序列（首 = 主模型）+ 下方候选下拉。
 * 主/降级合并同框（用户拍板 2026-09-08）——排序 = 拖拽（vuedraggable）+ 键盘
 * ←/→（无障碍通道），不做上移/下移按钮。 */
.chain-wrap {
  position: relative;
}
.chain-box {
  display: flex;
  align-items: center;
  gap: 4px;
  min-height: var(--ip-input-h-sm);
  padding: 3px 4px 3px 8px;
  box-sizing: border-box;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.chain-box:hover,
.chain-box.open {
  border-color: var(--ip-color-border-focus);
}
/* draggable 根元素即 tag 容器（flex-wrap 支持多档换行） */
.chain-tags {
  display: flex;
  flex: 1;
  flex-wrap: wrap;
  gap: 4px;
  min-width: 0;
  /* 容器空位 = 开合点击面（点击冒泡到 box toggle）；手型只在 tag 上（拖拽目标） */
  cursor: pointer;
}
.chain-placeholder {
  align-self: center;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-placeholder);
}
.chain-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 0 2px 0 6px;
  font-size: var(--ip-text-micro-size);
  line-height: 20px;
  color: var(--ip-color-text-body);
  background-color: var(--ip-color-bg-elevated);
  border-radius: var(--ip-radius-sm);
  cursor: grab;
  user-select: none;
}
.chain-tag:focus-visible {
  outline: 2px solid var(--ip-color-border-focus);
  outline-offset: 1px;
}
.chain-tag--main {
  color: var(--ip-color-primary-tint-text);
  background-color: var(--ip-color-primary-tint-bg);
}
.tag-name {
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tag-main {
  font-weight: var(--ip-font-weight-medium);
  opacity: 0.75;
}
.tag-x {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border: none;
  border-radius: var(--ip-radius-sm);
  background: transparent;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
}
.tag-x:hover {
  color: var(--ip-danger-base);
  background-color: var(--ip-danger-bg);
}
.chain-chevron {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: var(--ip-radius-sm);
  background: transparent;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
}
.chain-chevron:hover {
  color: var(--ip-color-text-body);
  background-color: var(--ip-color-bg-elevated);
}
.chain-menu {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  z-index: var(--ip-z-dropdown);
  display: flex;
  flex-direction: column;
  gap: 2px;
  max-height: 260px;
  overflow-y: auto;
  padding: 4px;
  background-color: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-lg);
}
.chain-option {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 8px;
  border: none;
  border-radius: var(--ip-radius-sm);
  background: transparent;
  text-align: left;
  cursor: pointer;
}
.chain-option:hover {
  background-color: var(--ip-color-primary-soft-bg);
}
.chain-option-name {
  flex-shrink: 0;
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-primary);
}
/* 候选条目 meta：厂商 glyph + 模型名（2026-09-09 拍板——厂商文字名退役） */
.chain-option-meta {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 1;
  min-width: 0;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
}
.chain-option-model {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--ip-font-mono);
}
.chain-option--new {
  margin-top: 2px;
  padding-top: 6px;
  border-top: 1px solid var(--ip-color-border-default);
  border-radius: 0;
  color: var(--ip-primary-600);
  font-size: var(--ip-text-caption-size);
}
.chain-empty {
  margin: 0;
  padding: 4px 8px;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
}
/* 链路测试行（选择器下方）：结果文案左、按钮右锚定；min-height 占位稳定防跳版 */
.chain-test-row {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  min-height: 20px;
}
.chain-test-row .conn-ok,
.chain-test-row .conn-err {
  flex: 1;
  min-width: 0;
  text-align: right;
}
/* 健康点（语义色圆点，tone 对齐后端 ProfileHealth 契约；neutral = 未调用过） */
.tag-dot {
  flex-shrink: 0;
  width: 6px;
  height: 6px;
  border-radius: var(--ip-radius-full);
}
.tag-dot--success { background-color: var(--ip-success-base); }
.tag-dot--warning { background-color: var(--ip-warning-base); }
.tag-dot--danger { background-color: var(--ip-danger-base); }
.tag-dot--neutral { background-color: var(--ip-color-text-tertiary); }

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
/* 风格预设入口（与 agent.yaml 徽章同排；弹层 Teleport 到 body） */
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
/* 创建态已选档：主色 tint 化（一眼可见已带风格） */
.ws-preset-btn.chosen {
  color: var(--ip-color-primary-tint-text);
  background-color: var(--ip-color-primary-tint-bg);
  border-color: transparent;
}
.ws-preset-btn:disabled {
  opacity: 0.6;
  cursor: default;
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

/* ===== 问号提示图标（ModelSettings 同款 verbatim——全系统统一形态） ===== */
.tip-icon {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  color: var(--ip-color-text-tertiary);
  cursor: help;
  flex-shrink: 0;
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.tip-icon:hover {
  color: var(--ip-primary-600);
}
.tip-icon::after {
  content: attr(data-tip);
  position: absolute;
  left: 50%;
  bottom: calc(100% + 6px);
  transform: translateX(-50%);
  max-width: 260px;
  width: max-content;
  padding: var(--ip-spacing-1) var(--ip-spacing-2_5);
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-on-primary);
  background: var(--ip-gray-800);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-lg);
  pointer-events: none;
  opacity: 0;
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out);
  z-index: var(--ip-z-badge);
  line-height: 1.5;
  text-align: center;
  white-space: pre-line;
}
.tip-icon:hover::after {
  opacity: 1;
}

/* 暗色模式 tooltip 背景变亮 */
[data-theme='dark'] .tip-icon::after {
  background: var(--ip-gray-200);
  color: var(--ip-gray-900);
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
