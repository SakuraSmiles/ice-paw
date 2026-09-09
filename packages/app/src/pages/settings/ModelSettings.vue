<!--
  ModelSettings — 设置·模型页（ModelProfile Phase 1：模型配置实体库）

  视觉读取 / 语义检索的引用选择在「设置-通用」（管用哪个），本页管实体本身
  （管有什么）：AgentSettings 同款折叠卡片列表——收起态单行摘要（别名 + 厂商 +
  模型 + Key 徽标），点击展开内联编辑；新建 = 列表头虚线卡片。

  约定：
  - 全系统编辑交互统一契约（2026-09-08 拍板）：显式保存——展开卡为草稿态，
    点「保存」整批提交（成功收起）/「取消」回滚；换厂商 key 闸 = 保存时校验拦截
  - Key 框三态：已存 = 掩码圆点（密文永不回显，掩码仅表示「已存」）、点入 =
    打点输入新值（眼睛可切明文核对）；Key 草稿随「保存」提交，留空保持不变
  - 编辑被语义检索引用的 profile 的厂商/模型时弹重建确认（旁路堵漏——前端本地
    可算引用关系，防绕过通用页切换闸改模型导致向量静默错配）
  - 删除两步确认（全系统不可恢复删除统一形态）：第一次点击武装成红色确认键，
    第二次执行，blur/收起解除；仍被引用时后端三段式拒绝 inline 指路
  - 图标一律 @lucide/vue；瞬态清理 onActivated（KeepAlive 防测试态残留）
-->
<script setup lang="ts">
import { ref, computed, onMounted, onActivated } from "vue";
import {
  Plus, Trash2, FlaskConical, Loader2, Check, X, HelpCircle, ChevronRight, Eye, EyeOff,
} from "@lucide/vue";
import { bridge } from "../../api/bridge";
import ErrorBanner from "../../components/common/ErrorBanner.vue";
import GroupedSelect from "../../components/common/GroupedSelect.vue";
import ProviderIcon from "../../components/common/ProviderIcon.vue";
import type { ComboboxGroup, ComboboxItem } from "../../components/common/Combobox.vue";
import EmbedSwitchOverlay from "../../components/common/EmbedSwitchOverlay.vue";
import { useProviders } from "../../composables/useProviders";
import { useModelProfiles, profileById } from "../../composables/useModelProfiles";
import { timeAgo, formatDate, formatTime } from "../../utils/time";
import type { Agent, ModelProfile, ModelProfileUpdate, ProviderInfo, UserPreferences } from "../../types";

// =========================================================================
// 公共状态
// =========================================================================
type TestState =
  | { status: "idle" }
  | { status: "testing" }
  | { status: "ok"; msg: string }
  | { status: "fail"; msg: string };

const prefs = ref<UserPreferences>({});
const loading = ref(true);
const loadError = ref<string | null>(null);
/** 保存成功提示，2s 淡出（状态上屏，不做保存按钮） */
const savedTip = ref(false);

function flashSaved() {
  savedTip.value = true;
  setTimeout(() => { savedTip.value = false; }, 2000);
}

function msgOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** 错误文案剥掉 invoke 层包装前缀（[op/kind] 内部错误: ），只留正文 */
function stripInvokePrefix(msg: string): string {
  return msg.replace(/^\[[^\]]*\]\s*(?:内部错误:\s*)?/, "");
}

function okFailMsg(t: TestState): string {
  return t.status === "ok" || t.status === "fail" ? stripInvokePrefix(t.msg) : "";
}

const { providers: providerList, loadProviders } = useProviders();
const { profiles, loadModelProfiles: sharedLoad } = useModelProfiles();
/** agent 列表快照（引用计数用——只读，编辑入口在智能体页）。后端删除守卫同源
 *  （agents_referencing），前端计数与其对齐防「计数说可删、删除被拒」的矛盾 */
const agents = ref<Agent[]>([]);

// =========================================================================
// profile 行（收起摘要 + 展开编辑同对象）
// =========================================================================
interface ProfileRowUI {
  /** profile id 快照（= server.id，行生命周期内不变；行内定位/引用守卫判据用） */
  id: string;
  server: ModelProfile;
  alias: string;
  provider: string;
  model: string;
  baseUrl: string;
  /** Key 输入草稿（空 = 不改已存 Key；密文永不回显） */
  keyDraft: string;
  /** Key 框处于输入态（focus 后 true；掩码态让位给编辑）。掩码本身是
      「有 Key」的视觉表示——真实内容永不回显，选中复制只会得到圆点 */
  keyEditing: boolean;
  /** 编辑态眼睛开关：显示/隐藏自己刚输入的新 Key（明文仅在此瞬间出现） */
  keyRevealed: boolean;
  hasApiKey: boolean;
  /** 保存中（按钮 spinner + 禁用） */
  saving: boolean;
  /** 删除两步确认：true = 武装态（再点执行；blur / 收起解除） */
  deleteConfirm: boolean;
  test: TestState;
}

type ProfilePatch = Partial<Pick<ModelProfileUpdate, "alias" | "provider" | "model" | "api_key" | "base_url">>;

const rows = ref<ProfileRowUI[]>([]);
/** 展开编辑的 profile id（null = 全部收起）；isCreating = 新建卡片展开态 */
const expandedId = ref<string | null>(null);
const isCreating = ref(false);
/** 行级 inline 错误（键 = profile id）：文案 + 可选 retry（服务端拒绝可重试；本地校验错误改字段即消） */
const rowErrors = ref<Record<string, { msg: string; retry?: () => void }>>({});

function makeRow(p: ModelProfile): ProfileRowUI {
  return {
    id: p.id,
    server: p,
    alias: p.alias,
    provider: p.provider,
    model: p.model,
    baseUrl: p.base_url ?? "",
    keyDraft: "",
    keyEditing: false,
    keyRevealed: false,
    hasApiKey: p.has_api_key,
    saving: false,
    deleteConfirm: false,
    test: { status: "idle" },
  };
}

/** 服务端真相同步进行对象；fields 之外的草稿（正在编辑的 Key 等）保留 */
function applyServer(row: ProfileRowUI, p: ModelProfile, fields: ProfileRowKey[]) {
  row.server = p;
  row.hasApiKey = p.has_api_key;
  if (fields.includes("alias")) row.alias = p.alias;
  if (fields.includes("provider")) row.provider = p.provider;
  if (fields.includes("model")) row.model = p.model;
  if (fields.includes("base_url")) row.baseUrl = p.base_url ?? "";
  if (fields.includes("api_key")) {
    row.keyDraft = "";
    row.keyEditing = false;
    row.keyRevealed = false;
  }
}
type ProfileRowKey = "alias" | "provider" | "model" | "base_url" | "api_key";

/** 列表重建：已存在的行保留对象（保用户草稿），新行从服务端快照构造 */
function rebuildRows() {
  const next: ProfileRowUI[] = [];
  for (const p of profiles.value) {
    const existing = rows.value.find((r) => r.id === p.id);
    if (existing) {
      existing.server = p;
      existing.hasApiKey = p.has_api_key;
      next.push(existing);
    } else {
      next.push(makeRow(p));
    }
  }
  rows.value = next;
}

async function reloadProfiles() {
  await sharedLoad(true);
  rebuildRows();
}

/** 提交一批变更：成功回填服务端快照并返回 true，失败行级 inline 返回 false */
async function saveRow(row: ProfileRowUI, patch: ProfilePatch, fields: ProfileRowKey[]): Promise<boolean> {
  delete rowErrors.value[row.id];
  row.test = { status: "idle" };
  row.saving = true;
  try {
    const updated = await bridge.modelProfiles.update({ id: row.id, ...patch });
    applyServer(row, updated, fields);
    flashSaved();
    return true;
  } catch (e) {
    rowErrors.value[row.id] = { msg: stripInvokePrefix(msgOf(e)), retry: () => void saveRow(row, patch, fields) };
    return false;
  } finally {
    row.saving = false;
  }
}

/** 草稿相对服务端快照有无变更（无变更时保存按钮禁用） */
function rowDirty(row: ProfileRowUI): boolean {
  return row.alias.trim() !== row.server.alias
    || row.provider !== row.server.provider
    || row.model.trim() !== row.server.model
    || (row.baseUrl.trim() || null) !== row.server.base_url
    || !!row.keyDraft.trim();
}

/** 保存前校验（后端 validate 镜像）：换厂商 key 闸 / custom 端点必填 / 非空 */
function validateRow(row: ProfileRowUI): string | null {
  const info = providerInfoOf(row.provider);
  if (!row.alias.trim()) return "别名必填（如「智谱主力」——用于在引用处辨认）";
  if (!row.model.trim()) return "模型名必填";
  if (info?.requires_base_url && !row.baseUrl.trim()) {
    return "自定义端点必须填写 URL（如 http://localhost:11434/v1）";
  }
  // 换厂商 key 闸：需 key 厂商切换且未填新 Key——旧厂商 Key 与新端点不同鉴权域，不能沿用
  const providerChanged = row.provider !== row.server.provider;
  if (providerChanged && info?.requires_key && !row.keyDraft.trim()) {
    return `已切到「${info.label}」——请填写该厂商的 API Key 后保存（Key 与端点同一鉴权域）`;
  }
  return null;
}

/** 保存按钮：校验 → 整批提交（别名/厂商/模型/端点 + 可选新 Key）；成功收起卡片 */
async function saveRowFull(row: ProfileRowUI) {
  const invalid = validateRow(row);
  if (invalid) {
    rowErrors.value[row.id] = { msg: invalid };
    return;
  }
  delete rowErrors.value[row.id];
  const patch: ProfilePatch = {
    alias: row.alias.trim(),
    provider: row.provider,
    model: row.model.trim(),
    base_url: row.baseUrl.trim() || null,
  };
  const fields: ProfileRowKey[] = ["alias", "provider", "model", "base_url"];
  if (row.keyDraft.trim()) {
    patch.api_key = row.keyDraft.trim();
    fields.push("api_key");
  }
  // 被语义检索引用且身份（厂商/模型）变化 → 重建确认（向量维度可能变）
  if (isEmbeddingReferenced(row) && identityChanged(row)) {
    openPendingEdit(row, patch);
    return;
  }
  if (await saveRow(row, patch, fields)) {
    expandedId.value = null; // 保存成功收起（完成感；失败留开供修正）
  }
}

/** 取消：草稿回滚到服务端快照并收起 */
function cancelRow(row: ProfileRowUI) {
  resetRowToServer(row);
  delete rowErrors.value[row.id];
  expandedId.value = null;
}

function toggleExpand(row: ProfileRowUI) {
  isCreating.value = false; // 编辑时收起新建
  row.deleteConfirm = false; // 收起解除删除武装态（草稿保留在行对象上，再展开不丢）
  expandedId.value = expandedId.value === row.id ? null : row.id;
}

/** 新建卡片：点击展开/合上，合上即取消（AgentSettings toggleNew 同款）；展开时重置草稿 */
function toggleNew() {
  expandedId.value = null;
  if (!isCreating.value) {
    const first = providerList.value.find((p) => !p.hidden) ?? null;
    createDraft.value = {
      alias: "",
      provider: first?.name ?? "",
      model: first?.models[0] ?? "",
      key: "",
      baseUrl: "",
    };
    createError.value = null;
    createTest.value = { status: "idle" };
  }
  isCreating.value = !isCreating.value;
}

// ---- Provider 目录派生 ----
function providerInfoOf(name: string): ProviderInfo | null {
  return providerList.value.find((p) => p.name === name) ?? null;
}
function providerLabelOf(name: string): string {
  return providerInfoOf(name)?.label ?? name;
}
function keyUrlOf(row: ProfileRowUI): string {
  return providerInfoOf(row.provider)?.key_url ?? "";
}

// ---- 分组模型目录（2026-09-09 拍板：厂商+模型两字段合并回旧版下拉框——
// GroupedSelect 可选可输，手输目录外名字落「自定义」逃生口；与 AgentForm 手动
// 形态同组件同交互，行为零再学习）----
const modelEntry = (provider: string, model: string): ComboboxItem => ({
  label: model,
  value: `${provider}::${model}`,
  data: { provider, model },
});

// 组头只展示品牌名单行（2026-09-09 拍板：注册表 note 提示文字在窄屏会换行，全撤）
const modelGroups = computed<ComboboxGroup[]>(() =>
  providerList.value
    .filter((p) => !p.hidden)
    .map((p) => ({
      id: p.name,
      label: p.label,
      items: p.models.map((m) => modelEntry(p.name, m)),
    })),
);

/** GroupedSelect 受控值：条目命中传 key（显 label/高亮）；否则空串
 *  （unmatchedLabel 回显行内已有模型名——目录外/自定义模型照常可编辑） */
function modelValueOf(provider: string, model: string): string {
  const key = `${provider}::${model}`;
  return modelGroups.value.some((g) => g.items.some((it) => it.value === key)) ? key : "";
}

/** 被引用次数（agent 主档/降级链 + 视觉链 + 语义检索；引用键在智能体页与
 *  通用页维护，此处只读——2026-09-09 补 agent 腿：删除守卫早就有它，计数漏算
 *  会出现「显示未引用、删除被拒」的矛盾） */
function referenceCountOf(row: ProfileRowUI): number {
  const inVision = (prefs.value.vision_profile_ids ?? []).filter((id) => id === row.id).length;
  const inEmbedding = prefs.value.embedding_profile_id === row.id ? 1 : 0;
  const inAgents = agents.value.filter(
    (a) => a.model_profile_id === row.id || (a.fallback_profile_ids ?? []).includes(row.id),
  ).length;
  return inVision + inEmbedding + inAgents;
}

function refTitleOf(row: ProfileRowUI): string {
  const n = referenceCountOf(row);
  if (n === 0) return "未被任何地方引用——可安全删除";
  return `被 Agent / 视觉读取 / 语义检索共引用 ${n} 处——删除前需先在「智能体」或「设置-通用」解除引用`;
}

// ---- 健康状态（按最后一次真实调用的结果；slug 契约 = 后端 ProfileHealth）----
/** tone 走语义色三层：success=正常 / warning=瞬时故障（自愈或地址问题）/ danger=确定性失败需人处理 */
interface HealthMeta { tone: "success" | "warning" | "danger" | "neutral"; label: string }
const HEALTH_META: Record<string, HealthMeta> = {
  ok: { tone: "success", label: "正常" },
  quota: { tone: "danger", label: "额度耗尽" },
  auth: { tone: "danger", label: "Key 无效" },
  model_not_found: { tone: "danger", label: "模型不存在" },
  unknown: { tone: "danger", label: "调用异常" },
  rate_limited: { tone: "warning", label: "限流中" },
  network: { tone: "warning", label: "端点不可达" },
};
/** null / 未知 slug 的中性态（未调用不冒充正常；未知 slug 前后端版本错位时原样展示） */
function healthOf(row: ProfileRowUI): HealthMeta {
  const slug = row.server.last_health;
  if (!slug) return { tone: "neutral", label: "未调用" };
  return HEALTH_META[slug] ?? { tone: "neutral", label: slug };
}
/** 状态胶囊 hover：状态 + 绝对时间 + 失败原文（时间规范 = 相对时上屏 + hover 绝对时） */
function healthTitleOf(row: ProfileRowUI): string {
  const at = row.server.last_health_at;
  if (!row.server.last_health || !at) {
    return "尚未被真实调用过——首次调用（视觉代读 / 语义检索 / 逐行测试）后此处显示状态";
  }
  const lines = [`状态：${healthOf(row).label}`, `最后调用：${formatDate(at)} ${formatTime(at)}`];
  if (row.server.last_health_detail) lines.push(`详情：${row.server.last_health_detail}`);
  return lines.join("\n");
}
/** 端点占位 = 空 base_url 实际生效的推导值（openai_url 优先，custom 必填提示）——
 *  真实展示所选厂商的注册表默认端点，不做固定文案（2026-09-09 拍板） */
function urlPlaceholder(provider: string): string {
  const info = providerInfoOf(provider);
  if (info?.requires_base_url) return "必填，如 http://localhost:11434/v1";
  return info?.openai_url || info?.default_url || "留空用厂商官方端点";
}

// ---- 字段草稿（显式保存：改动只进草稿，点「保存」整批提交）----
/** 行内选档（厂商+模型一并落）：切厂商端点清空重推导 + 复位测试结论（原厂商
 *  切换行为对齐）；同厂商换模型不动端点。自定义条目 → custom + 端点交用户填
 *  （必填，保存时校验拦截） */
function onRowModelSelect(row: ProfileRowUI, item: ComboboxItem) {
  const data = item.data as { provider?: string; model?: string; custom?: boolean } | undefined;
  if (data?.custom) {
    row.provider = "custom";
    row.model = data.model ?? item.label;
    row.baseUrl = "";
    row.test = { status: "idle" };
    return;
  }
  const next = data?.provider ?? row.provider;
  if (next !== row.provider) {
    row.baseUrl = "";
    row.test = { status: "idle" };
    row.deleteConfirm = false;
    row.provider = next;
  }
  row.model = data?.model ?? item.label;
}

// ---- Key 框三态（掩码 / 编辑草稿）----
/** 掩码 = 固定圆点串（与真实 Key 内容无关，仅表示「已存」） */
const KEY_MASK = "••••••••••••••••";
/** 掩码态：已存 Key、不在输入且无新草稿（有草稿时显示草稿打点，勿用旧掩码误导） */
function isKeyMasked(row: ProfileRowUI): boolean {
  return row.hasApiKey && !row.keyEditing && !row.keyDraft;
}
/** 显示值：掩码态给圆点串，其余给草稿（密文永不回显） */
function keyDisplayOf(row: ProfileRowUI): string {
  return isKeyMasked(row) ? KEY_MASK : row.keyDraft;
}
/** 输入类型：掩码态 text（圆点是字面字符），编辑态 password 打点（眼睛可切明文） */
function keyTypeOf(row: ProfileRowUI): string {
  return isKeyMasked(row) || row.keyRevealed ? "text" : "password";
}
/** 手动绑定（不走 v-model）：显示值在掩码/草稿间切换由状态驱动 */
function onKeyInput(row: ProfileRowUI, e: Event) {
  row.keyDraft = (e.target as HTMLInputElement).value;
}

/** 离开 Key 框（不保存——显式保存模式下仅做掩码/明文复位） */
function onKeyLeave(row: ProfileRowUI) {
  if (!row.keyDraft) row.keyEditing = false; // 空草稿 → 回掩码态
  row.keyRevealed = false; // 明文不留在屏上
}

// ---- 测试 / 删除 ----
async function testRow(row: ProfileRowUI) {
  if (row.test.status === "testing") return;
  row.test = { status: "testing" };
  try {
    // profileId 腿：服务端取存量凭据；keyDraft 已填则入参优先（测未保存的新 Key）。
    // 与存量一致的 URL 入参由后端降级为「未覆盖」——健康归因恢复存量测试
    // （2026-09-09 生产实案：存了端点的档位恒走覆盖路径，状态点恒不动）。
    // 厂商切换草稿不传 profileId：存量凭据属于旧厂商，拿它兜底只会测出误导结果。
    const res = await bridge.providers.testConnection(
      row.provider,
      row.baseUrl.trim() || undefined,
      row.keyDraft.trim() || undefined,
      undefined,
      row.provider === row.server.provider ? row.id : undefined,
    );
    row.test = res.ok
      ? { status: "ok", msg: `连接正常 · ${res.model_count} 个模型` }
      : { status: "fail", msg: res.error ?? "探测失败" };
  } catch (e) {
    row.test = { status: "fail", msg: msgOf(e) };
  } finally {
    // 探测是真实调用——后端已把结果沉淀为健康三列，刷新让状态点即时反映
    await reloadProfiles();
  }
}

/** 删除两步确认：第一次点击武装（按钮变确认态），第二次执行；blur 解除 */
function onDeleteClick(row: ProfileRowUI) {
  if (!row.deleteConfirm) {
    row.deleteConfirm = true;
    return;
  }
  row.deleteConfirm = false;
  void deleteRow(row);
}

async function deleteRow(row: ProfileRowUI) {
  delete rowErrors.value[row.id];
  try {
    await bridge.modelProfiles.delete(row.id);
    rows.value = rows.value.filter((r) => r.id !== row.id);
    if (expandedId.value === row.id) expandedId.value = null;
    await reloadProfiles();
    flashSaved();
  } catch (e) {
    // 后端删除守卫：仍被视觉链/语义检索引用 → 三段式拒绝，inline 指路先解引用
    rowErrors.value[row.id] = { msg: stripInvokePrefix(msgOf(e)), retry: () => void deleteRow(row) };
  }
}

// ---- 创建表单（虚线卡片展开；创建后转即时保存行）----
const creatingBusy = ref(false);
const createError = ref<string | null>(null);
const createDraft = ref({ alias: "", provider: "", model: "", key: "", baseUrl: "" });
/** 创建表单草稿测试态（与行内 testRow 同形态；toggleNew 重置） */
const createTest = ref<TestState>({ status: "idle" });

/** 新建草稿测试：无 profileId（实体还不存在）——后端按表单值探测，结果不沉淀
 *  健康三列（没有存量可记，不冒充）；需 Key 厂商未填 Key 由后端短路给引导文案 */
async function testCreate() {
  if (createTest.value.status === "testing") return;
  const d = createDraft.value;
  if (!d.provider || !d.model.trim()) {
    createTest.value = { status: "fail", msg: "请先选择厂商与模型再测试" };
    return;
  }
  createTest.value = { status: "testing" };
  try {
    const res = await bridge.providers.testConnection(
      d.provider,
      d.baseUrl.trim() || undefined,
      d.key.trim() || undefined,
    );
    createTest.value = res.ok
      ? { status: "ok", msg: `连接正常 · ${res.model_count} 个模型` }
      : { status: "fail", msg: res.error ?? "探测失败" };
  } catch (e) {
    createTest.value = { status: "fail", msg: msgOf(e) };
  }
}

/** 新建表单选档（同 onRowModelSelect 语义；无测试态/删除态字段） */
function onCreateModelSelect(item: ComboboxItem) {
  const data = item.data as { provider?: string; model?: string; custom?: boolean } | undefined;
  if (data?.custom) {
    createDraft.value.provider = "custom";
    createDraft.value.model = data.model ?? item.label;
    createDraft.value.baseUrl = "";
    return;
  }
  const next = data?.provider ?? createDraft.value.provider;
  if (next !== createDraft.value.provider) {
    createDraft.value.baseUrl = "";
    createDraft.value.provider = next;
  }
  createDraft.value.model = data?.model ?? item.label;
}

async function submitCreate() {
  if (creatingBusy.value) return;
  const d = createDraft.value;
  const info = providerInfoOf(d.provider);
  // 前端校验 = 后端 validate_new_profile 镜像
  if (!d.alias.trim()) { createError.value = "别名必填（如「智谱主力」「OpenAI 备用」——用于在引用处辨认）"; return; }
  if (!d.provider) { createError.value = "请选择厂商"; return; }
  if (!d.model.trim()) { createError.value = "请填写模型名"; return; }
  if (info?.requires_key && !d.key.trim()) {
    createError.value = `该厂商需要 API Key——${info.label} 的模型接口需鉴权，请粘贴 Key`; return;
  }
  if (info?.requires_base_url && !d.baseUrl.trim()) {
    createError.value = "自定义端点必须填写 URL（如 http://localhost:11434/v1）"; return;
  }
  creatingBusy.value = true;
  createError.value = null;
  try {
    await bridge.modelProfiles.create({
      alias: d.alias.trim(),
      provider: d.provider,
      model: d.model.trim(),
      api_key: d.key,
      base_url: d.baseUrl.trim() || null,
    });
    isCreating.value = false;
    await reloadProfiles();
    flashSaved();
  } catch (e) {
    createError.value = stripInvokePrefix(msgOf(e));
  } finally {
    creatingBusy.value = false;
  }
}

// =========================================================================
// 被引用 profile 编辑的重建确认（旁路堵漏：通用页切换闸的镜像场景）
// =========================================================================
const pendingEdit = ref<{
  rowId: string;
  alias: string;
  summary: string;
  payload: ProfilePatch & { id: string };
} | null>(null);
const rebuilding = ref(false);
const switchError = ref<string | null>(null);

/** 语义检索当前引用是否活着（id 有值且实体存在——悬空引用视同未启用） */
const embeddingActive = computed(() =>
  !!prefs.value.embedding_profile_id
  && !!profileById(profiles.value, prefs.value.embedding_profile_id),
);

/** 该行是否被语义检索引用着（前端本地可算——引用键在通用页维护，此处只读） */
function isEmbeddingReferenced(row: ProfileRowUI): boolean {
  return embeddingActive.value && prefs.value.embedding_profile_id === row.id;
}

/** 厂商/模型任一变化 = embedding 身份变化（向量维度可能变） */
function identityChanged(row: ProfileRowUI): boolean {
  return row.provider !== row.server.provider || row.model !== row.server.model;
}

function openPendingEdit(row: ProfileRowUI, patch: ProfilePatch) {
  pendingEdit.value = {
    rowId: row.id,
    alias: row.server.alias,
    summary: `${providerLabelOf(row.server.provider)} · ${row.server.model} → ${providerLabelOf(row.provider)} · ${row.model}`,
    payload: { id: row.id, ...patch },
  };
  switchError.value = null;
}

function cancelPending() {
  if (pendingEdit.value) {
    const row = rows.value.find((r) => r.id === pendingEdit.value?.rowId);
    if (row) resetRowToServer(row);
  }
  pendingEdit.value = null;
  switchError.value = null;
}

function resetRowToServer(row: ProfileRowUI) {
  row.alias = row.server.alias;
  row.provider = row.server.provider;
  row.model = row.server.model;
  row.baseUrl = row.server.base_url ?? "";
  row.keyDraft = "";
  row.keyEditing = false;
  row.keyRevealed = false;
  row.deleteConfirm = false;
  row.test = { status: "idle" };
}

/** 先落变更（测试读存量须先存）→ 测新 → 失败不重建（旧向量保留） */
async function confirmProfileEdit() {
  const pe = pendingEdit.value;
  if (!pe) return;
  rebuilding.value = true;
  switchError.value = null;
  try {
    await bridge.modelProfiles.update(pe.payload);
    try {
      await bridge.kb.testEmbeddingConfig("", "", "", undefined, pe.rowId);
    } catch (e) {
      // 配置已更新但向量未清（cache model 维兜底防错配）——overlay 留开，用户可修正后重试或点取消收尾
      switchError.value = `新配置健康检查未通过：${stripInvokePrefix(msgOf(e))}。配置已更新、向量未重建——修正后重试；点「取消」将放弃表单里的未存修改（已存的配置变更保留）`;
      return;
    }
    const stats = await bridge.kb.rebuildAllEmbeddings();
    pendingEdit.value = null;
    await reloadProfiles();
    // 行草稿回滚到已存新值（= 刚提交的值）并收起卡片
    const row = rows.value.find((r) => r.id === pe.rowId);
    if (row) {
      delete rowErrors.value[row.id];
      resetRowToServer(row);
    }
    if (expandedId.value === pe.rowId) expandedId.value = null;
    void stats; // 重建统计在通用页语义检索卡呈现；此处实体已就绪
  } catch (e) {
    switchError.value = `变更失败：${stripInvokePrefix(msgOf(e))}（请重试）`;
    await reloadProfiles();
  } finally {
    rebuilding.value = false;
  }
}

// =========================================================================
// 加载 / 瞬态清理
// =========================================================================
async function load() {
  loading.value = true;
  loadError.value = null;
  try {
    // 解构位 = 数组位（曾被 4 项 Promise.all 两名解构错拿 loadProviders 结果——
    // agents 全空、计数静默漏算，测试 DEBUG 实锤）；要解构的放同一组
    const [raw, agentList] = await Promise.all([
      bridge.preferences.get(),
      bridge.agents.list(),
    ]);
    await Promise.all([loadProviders(), sharedLoad()]);
    prefs.value = raw;
    agents.value = agentList;
    rebuildRows();
  } catch (e) {
    console.error("加载模型配置失败:", e);
    loadError.value = msgOf(e);
  } finally {
    loading.value = false;
  }
}

onMounted(load);

// KeepAlive 瞬态清理：测试态是「刚操作过」的即时反馈，回到本页时已过期。
// 同时刷新数据：状态点语义是「最后一次真实调用」、引用计数读 prefs——二者
// 都可能在其他页变化（通用页测试/增删引用），keep-alive 下 onMounted 不再跑，
// 不刷新就会拿着旧快照谎报「未调用」（2026-09-08 实案）。rebuildRows 保留行
// 对象，编辑草稿不动。
onActivated(async () => {
  rows.value.forEach((r) => { r.test = { status: "idle" }; });
  try {
    const [raw, agentList] = await Promise.all([
      bridge.preferences.get(),
      bridge.agents.list(),
    ]);
    await sharedLoad(true);
    prefs.value = raw;
    agents.value = agentList;
    rebuildRows();
  } catch {
    // 刷新失败保留旧快照（不置 loadError——页面已可用，别整页报错）
  }
});
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">模型</h2>
      <span v-if="savedTip" class="save-tip">已保存</span>
    </div>

    <div v-if="loading" class="loading-state">加载中...</div>
    <template v-else>
      <ErrorBanner
        v-if="loadError"
        variant="banner"
        title="模型配置加载失败"
        :detail="loadError + '。下方显示的可能不是最新配置，重试成功前请勿编辑保存'"
        retry-label="重试"
        @retry="load"
      />
      <div class="profile-list" :class="{ 'list-untrusted': !!loadError }">

        <!-- 新建卡片（列表第一条，虚线边框；点击展开创建表单） -->
        <div class="profile-card new-card" :class="{ expanded: isCreating }" @click="toggleNew">
          <div class="card-top">
            <div class="row-title">
              <Plus :size="16" class="new-plus" />
              <span class="card-name new-name">新建模型配置</span>
              <span class="new-hint">创建一条可被视觉读取 / 语义检索引用的配置</span>
            </div>
            <ChevronRight :size="16" class="card-chevron" :class="{ rotated: isCreating }" />
          </div>
          <div v-if="isCreating" class="expand-panel" @click.stop>
            <div class="row-grid">
              <div class="field">
                <div class="field-label">别名</div>
                <input v-model="createDraft.alias" type="text" class="form-input" placeholder="如「智谱主力」「OpenAI 备用」" />
              </div>
              <div class="field">
                <div class="field-label">厂商 / 模型</div>
                <GroupedSelect
                  :model-value="modelValueOf(createDraft.provider, createDraft.model)"
                  :groups="modelGroups"
                  allow-custom
                  :unmatched-label="createDraft.model"
                  placeholder="选择或输入模型名"
                  @select="onCreateModelSelect"
                >
                  <!-- 关闭态控件前缀：当前归属厂商的图标 -->
                  <template #control-icon>
                    <ProviderIcon v-if="createDraft.model" :name="createDraft.provider" />
                  </template>
                  <!-- 组头：厂商品牌图标（未知 provider 渲染为空，不破版式） -->
                  <template #group-icon="{ group }">
                    <ProviderIcon :name="group.id ?? ''" :size="13" />
                  </template>
                </GroupedSelect>
              </div>
            </div>
            <div class="field">
              <div class="field-label">
                API Key
                <a v-if="providerInfoOf(createDraft.provider)?.key_url" :href="providerInfoOf(createDraft.provider)!.key_url!" target="_blank" class="embed-key-link">申请 Key →</a>
              </div>
              <div class="input-group">
                <input v-model="createDraft.key" type="password" class="form-input" :placeholder="providerInfoOf(createDraft.provider)?.requires_key ? '粘贴 API Key' : '免 Key 厂商可留空'" />
                <button class="btn" :disabled="createTest.status === 'testing'" @click="testCreate">
                  <Loader2 v-if="createTest.status === 'testing'" :size="14" class="spin" />
                  <FlaskConical v-else :size="14" />
                  {{ createTest.status === "testing" ? "测试中…" : "测试" }}
                </button>
              </div>
              <div v-if="createTest.status === 'ok' || createTest.status === 'fail'" class="test-result">
                <Check v-if="createTest.status === 'ok'" :size="14" class="test-ok-icon" />
                <X v-else :size="14" class="test-fail-icon" />
                <span :class="createTest.status === 'ok' ? 'test-ok-text' : 'test-fail-text'" :title="okFailMsg(createTest)">{{ okFailMsg(createTest) }}</span>
              </div>
            </div>
            <div class="field">
              <div class="field-label">端点 URL</div>
              <input v-model="createDraft.baseUrl" type="text" class="form-input" :placeholder="urlPlaceholder(createDraft.provider)" />
            </div>
            <div v-if="createError" class="test-result">
              <X :size="14" class="test-fail-icon" />
              <span class="test-fail-text">{{ createError }}</span>
            </div>
            <div class="create-actions">
              <button class="btn" :disabled="creatingBusy" @click="isCreating = false">取消</button>
              <button class="btn-primary" :disabled="creatingBusy" @click="submitCreate">
                <Loader2 v-if="creatingBusy" :size="14" class="spin" />
                {{ creatingBusy ? "创建中…" : "创建" }}
              </button>
            </div>
          </div>
        </div>

        <div class="list-divider"></div>

        <!-- profile 实体列表（收起单行摘要 / 展开内联编辑） -->
        <div
          v-for="row in rows"
          :key="row.id"
          class="profile-card"
          :class="{ expanded: expandedId === row.id }"
          @click="toggleExpand(row)"
        >
          <div class="card-top">
            <div class="card-main">
              <!-- 首行：别名（用户命名）+ 引用次数（右锚定） -->
              <div class="row-title">
                <span class="card-name">{{ row.alias }}</span>
                <span
                  class="ref-count"
                  :class="{ 'ref-count--none': referenceCountOf(row) === 0 }"
                  :title="refTitleOf(row)"
                >{{ referenceCountOf(row) > 0 ? `${referenceCountOf(row)} 处引用` : "未引用" }}</span>
              </div>
              <!-- 次行：模型 tag（厂商 glyph + 模型名，AgentSettings 同款）+ 健康状态（右锚定） -->
              <div class="row-sub">
                <span class="card-model" :title="providerLabelOf(row.provider)">
                  <ProviderIcon :name="row.provider" :size="12" />
                  <span class="card-model-name">{{ row.model || "未设模型" }}</span>
                </span>
                <span class="health-chip" :class="`health-chip--${healthOf(row).tone}`" :title="healthTitleOf(row)">
                  <span class="health-dot" aria-hidden="true"></span>{{ healthOf(row).label }}<template v-if="row.server.last_health_at"> · {{ timeAgo(row.server.last_health_at) }}</template>
                </span>
              </div>
              <ErrorBanner
                v-if="rowErrors[row.id]"
                variant="inline"
                title="操作失败"
                :detail="rowErrors[row.id].msg"
                :retry-label="rowErrors[row.id].retry ? '重试' : null"
                @retry.stop="rowErrors[row.id]?.retry?.()"
              />
            </div>
            <ChevronRight :size="16" class="card-chevron" :class="{ rotated: expandedId === row.id }" />
          </div>

          <!-- 展开态：全字段编辑（草稿态，点「保存」整批提交 / 「取消」回滚） -->
          <div v-if="expandedId === row.id" class="expand-panel" @click.stop>
            <div class="row-grid">
              <div class="field">
                <div class="field-label">别名</div>
                <input v-model="row.alias" type="text" class="form-input" placeholder="如「智谱主力」" />
              </div>
              <div class="field">
                <div class="field-label">厂商 / 模型</div>
                <GroupedSelect
                  :model-value="modelValueOf(row.provider, row.model)"
                  :groups="modelGroups"
                  allow-custom
                  :unmatched-label="row.model"
                  placeholder="选择或输入模型名"
                  @select="(it: ComboboxItem) => onRowModelSelect(row, it)"
                >
                  <!-- 关闭态控件前缀：当前归属厂商的图标 -->
                  <template #control-icon>
                    <ProviderIcon v-if="row.model" :name="row.provider" />
                  </template>
                  <!-- 组头：厂商品牌图标（未知 provider 渲染为空，不破版式） -->
                  <template #group-icon="{ group }">
                    <ProviderIcon :name="group.id ?? ''" :size="13" />
                  </template>
                </GroupedSelect>
              </div>
              <!-- 删除两步确认：第一次点击武装成红色确认键，第二次执行；blur 解除 -->
              <button
                v-if="!row.deleteConfirm"
                class="vision-icon-btn delete-btn"
                title="删除此模型配置（仍被视觉读取/语义检索引用时会被拒绝）"
                @click.stop="onDeleteClick(row)"
              >
                <Trash2 :size="14" />
              </button>
              <button
                v-else
                class="btn delete-confirm-btn"
                title="再点一次执行删除；移开焦点取消"
                @click.stop="onDeleteClick(row)"
                @blur="row.deleteConfirm = false"
              >
                确认删除？
              </button>
            </div>

            <div class="field">
              <div class="field-label">
                API Key
                <a v-if="keyUrlOf(row)" :href="keyUrlOf(row)" target="_blank" class="embed-key-link">申请 Key →</a>
              </div>
              <div class="input-group">
                <!-- 三态：已存未点入 = 掩码圆点（密文永不回显）；点入 = 打点输入新值，
                     眼睛可切明文核对；草稿随「保存」提交（留空保持已存 Key 不变） -->
                <input
                  :value="keyDisplayOf(row)"
                  :type="keyTypeOf(row)"
                  class="form-input key-input"
                  :class="{ 'key-masked': isKeyMasked(row) }"
                  :placeholder="row.hasApiKey ? '输入新 Key 更换（留空保持不变）' : '粘贴 API Key'"
                  :title="isKeyMasked(row) ? '已加密存储——点击输入新值更换' : undefined"
                  autocomplete="off"
                  spellcheck="false"
                  @focus="row.keyEditing = true"
                  @input="onKeyInput(row, $event)"
                  @blur="onKeyLeave(row)"
                />
                <button
                  v-if="!isKeyMasked(row) && row.keyDraft"
                  class="vision-icon-btn key-eye"
                  type="button"
                  tabindex="-1"
                  :title="row.keyRevealed ? '隐藏 Key' : '显示 Key'"
                  @mousedown.prevent
                  @click="row.keyRevealed = !row.keyRevealed"
                >
                  <EyeOff v-if="row.keyRevealed" :size="14" />
                  <Eye v-else :size="14" />
                </button>
                <button class="btn" :disabled="row.test.status === 'testing'" @click="testRow(row)">
                  <Loader2 v-if="row.test.status === 'testing'" :size="14" class="spin" />
                  <FlaskConical v-else :size="14" />
                  {{ row.test.status === "testing" ? "测试中…" : "测试" }}
                </button>
              </div>
              <div v-if="row.test.status === 'ok' || row.test.status === 'fail'" class="test-result">
                <Check v-if="row.test.status === 'ok'" :size="14" class="test-ok-icon" />
                <X v-else :size="14" class="test-fail-icon" />
                <span :class="row.test.status === 'ok' ? 'test-ok-text' : 'test-fail-text'" :title="okFailMsg(row.test)">{{ okFailMsg(row.test) }}</span>
              </div>
            </div>

            <div class="field">
              <div class="field-label">
                端点 URL
                <span class="tip-icon" data-tip="留空 = 按厂商官方 OpenAI 兼容端点推导（视觉代读 / 语义检索共用）。&#10;自定义端点（Ollama / vLLM 等）填这里；智谱 Coding 套餐请直接选「智谱 Coding」厂商档。">
                  <HelpCircle :size="14" />
                </span>
              </div>
              <input v-model="row.baseUrl" type="text" class="form-input" :placeholder="urlPlaceholder(row.provider)" />
            </div>

            <!-- 操作行：显式保存 / 取消（全系统编辑交互统一契约） -->
            <div class="row-actions">
              <p class="expand-foot">视觉读取 / 语义检索的引用在「设置-通用」选择</p>
              <div class="action-btns">
                <button class="btn" :disabled="row.saving" @click="cancelRow(row)">取消</button>
                <button class="btn-primary" :disabled="row.saving || !rowDirty(row)" @click="saveRowFull(row)">
                  <Loader2 v-if="row.saving" :size="14" class="spin" />
                  {{ row.saving ? "保存中…" : "保存" }}
                </button>
              </div>
            </div>
          </div>
        </div>

        <div v-if="rows.length === 0" class="empty-hint">
          还没有模型配置。视觉读取与语义检索启用前，先点上方「新建模型配置」创建一条（厂商 + 模型 + Key）。
        </div>
      </div>
    </template>

    <!-- 编辑被语义检索引用的 profile：重建确认（共享组件，与通用页切换浮层同形） -->
    <EmbedSwitchOverlay
      v-if="pendingEdit"
      title="修改被语义检索引用的模型配置？"
      :rows="[
        { label: '配置', value: pendingEdit.alias },
        { label: '变更', value: pendingEdit.summary },
      ]"
      :error="switchError"
      :rebuilding="rebuilding"
      @cancel="cancelPending"
      @confirm="confirmProfileEdit"
    />
  </div>
</template>

<style scoped>
/* 页级数据不可信：内容降透明（配 ErrorBanner banner 形态） */
.profile-list.list-untrusted { opacity: 0.55; pointer-events: none; }

/* ===== 页面布局 ===== */
.settings-content-inner {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 0;
  min-height: 0;
}

.content-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 20px 28px 0;
  flex-shrink: 0;
  height: 56px;
}
.content-title {
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0;
}
.save-tip {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-success-text);
  flex-shrink: 0;
}

.loading-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-body-sm-size);
}

/* ===== 折叠卡片列表（AgentSettings 同款骨架） ===== */
.profile-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px 28px 24px;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
  min-height: 0;
}

.profile-card {
  padding: 12px 16px;
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-card-radius);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.profile-card:hover {
  border-color: var(--ip-primary-300);
  box-shadow: var(--ip-shadow-sm);
}
.profile-card.expanded {
  border-color: var(--ip-primary-400);
  box-shadow: var(--ip-shadow-sm);
}

.new-card {
  border: 1px dashed var(--ip-color-border-default);
  background-color: transparent;
}
.new-card:hover {
  border-color: var(--ip-primary-400);
  background-color: var(--ip-color-bg-tertiary);
}
.new-card.expanded {
  border-style: solid;
  border-color: var(--ip-primary-400);
  background-color: var(--ip-color-bg-secondary);
}

.list-divider {
  height: 1px;
  background-color: var(--ip-color-border-default);
  margin: 2px 4px;
}

.card-top {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  cursor: pointer;
}
.card-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
/* 首行：别名 + 供应商 tag + 引用次数（右锚定，chevron 旁） */
.row-title {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.card-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.new-plus { flex-shrink: 0; color: var(--ip-color-primary-tint-text); }
.new-name { color: var(--ip-color-primary-tint-text); flex-shrink: 0; }
.new-hint {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

/* 引用次数（首行右锚定）：被引用 = 有依赖不可随手删；未引用最淡（可安全删） */
.ref-count {
  flex-shrink: 0;
  margin-left: auto;
  padding: 0 6px;
  line-height: 18px;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-full);
  white-space: nowrap;
}
.ref-count--none {
  color: var(--ip-color-text-disabled);
  background-color: transparent;
}

/* 次行：模型 tag（厂商 glyph + 模型名 mono）+ 健康状态（窄屏 tag 让位省略） */
.row-sub {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: var(--ip-text-caption-size);
  /* 行高锚定 18px = tag 高（AgentSettings 同款：全局行高 1.6 的分数行盒与
     固定高 tag 混排会产生半像素错位） */
  line-height: 18px;
  color: var(--ip-color-text-secondary);
}
/* 模型 tag：AgentSettings .card-model 同形态（hover title = 厂商显示名） */
.card-model {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  flex-shrink: 1;
  height: 18px;
  padding: 0 7px 0 6px;
  border-radius: var(--ip-radius-full);
  background-color: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-tertiary);
}
.card-model-name {
  min-width: 0;
  /* 行高压平（文字盒=字号）：继承的 1.6 行高会把 flex 居中顶偏，压平后才是真居中 */
  line-height: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-micro-size);
}

/* 健康状态胶囊（次行右锚定）：语义色圆点 + 文案 + 相对时——按最后一次真实
   调用着色（success=正常 / warning=瞬时 / danger=需人处理 / neutral=未调用） */
.health-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  margin-left: auto;
  font-size: var(--ip-text-micro-size);
  white-space: nowrap;
  color: var(--ip-color-text-secondary);
}
.health-dot {
  width: 6px;
  height: 6px;
  border-radius: var(--ip-radius-full);
  background-color: currentColor;
  flex-shrink: 0;
}
.health-chip--success { color: var(--ip-success-text); }
.health-chip--warning { color: var(--ip-warning-text); }
.health-chip--danger { color: var(--ip-danger-text); }
.health-chip--neutral { color: var(--ip-color-text-disabled); }

.card-chevron {
  flex-shrink: 0;
  color: var(--ip-color-text-disabled);
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}
.card-chevron.rotated {
  transform: rotate(90deg);
  color: var(--ip-primary-600);
}

/* ===== 展开面板（全字段编辑） ===== */
.expand-panel {
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid var(--ip-color-border-default);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
  cursor: default;
}
/* 身份行：别名 + 分组选择器（厂商+模型合并档）+ 删除按钮（end 对齐——按钮贴输入框底线） */
.row-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1.7fr) auto;
  gap: var(--ip-spacing-2);
  align-items: end;
}
/* 操作行：指路文案（左）+ 保存/取消（右） */
.row-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-2);
}
.expand-foot {
  margin: 0;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-disabled);
  line-height: 1.5;
}
.action-btns {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}
.delete-btn { flex-shrink: 0; margin-bottom: 4px; }
.delete-btn:hover:not(:disabled) { color: var(--ip-danger-text); background: var(--ip-color-bg-tertiary); }
/* 删除武装态（两步确认第二步）：danger 语义，替代原位 icon 按钮 */
.delete-confirm-btn {
  flex-shrink: 0;
  margin-bottom: 4px;
  color: var(--ip-danger-text);
  border-color: var(--ip-danger-border);
  white-space: nowrap;
}
.delete-confirm-btn:hover {
  color: var(--ip-color-text-on-primary);
  background-color: var(--ip-danger-base);
  border-color: var(--ip-danger-base);
}

/* ===== 字段 ===== */
.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.field-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
}

.input-group {
  display: flex;
  gap: 6px;
  align-items: center;
}

/* 基础形态 = 独占一行（.field 列 flex 子项）：width 撑满、高度走令牌——
   勿写 flex:1，列向 flex-basis 0% 会压过 height 把输入框压塌 */
.form-input {
  width: 100%;
  height: var(--ip-input-h-sm);
  padding: 0 10px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  outline: none;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.form-input:focus {
  border-color: var(--ip-color-border-focus);
  background-color: var(--ip-color-bg-input);
  box-shadow: 0 0 0 3px rgba(var(--ip-primary-500-rgb), 0.12);
}
.form-input::placeholder {
  color: var(--ip-color-text-placeholder);
}
/* Key 掩码态：圆点调淡一级（像内容在那儿，但一望可知非明文可复制态） */
.form-input.key-masked {
  color: var(--ip-color-text-secondary);
  letter-spacing: 1px;
}

/* 行内组合（Key + 测试按钮）：input 占满剩余宽度 */
.input-group > .form-input {
  flex: 1;
  min-width: 0;
  width: auto;
}

/* ===== 按钮 ===== */
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  height: var(--ip-input-h-sm);
  padding: 0 12px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  white-space: nowrap;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.btn:hover {
  background-color: var(--ip-color-bg-secondary);
  border-color: var(--ip-color-border-focus);
  color: var(--ip-primary-600);
}
.btn:disabled { opacity: 0.6; cursor: not-allowed; }
.btn-primary {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  height: var(--ip-input-h-sm);
  padding: 0 12px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: white;
  background-color: var(--ip-primary-600);
  border: none;
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  white-space: nowrap;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-primary:hover { background-color: var(--ip-primary-700); }
.btn-primary:disabled { opacity: 0.6; cursor: not-allowed; }

.create-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--ip-spacing-2);
}

.vision-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  flex-shrink: 0;
  border: none;
  border-radius: var(--ip-radius-sm);
  background: transparent;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  transition: color var(--ip-duration-fast) var(--ip-ease-out), background var(--ip-duration-fast) var(--ip-ease-out);
}
.vision-icon-btn:disabled { opacity: 0.35; cursor: default; }

/* ===== 测试结果块 ===== */
.test-result {
  display: flex;
  align-items: flex-start;
  gap: 4px;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border-radius: var(--ip-radius-sm);
  background: var(--ip-color-bg-tertiary);
  font-size: var(--ip-text-micro-size);
  line-height: 1.5;
}
.test-ok-icon { color: var(--ip-success-text); flex-shrink: 0; margin-top: 1px; }
.test-ok-text { color: var(--ip-success-text); word-break: break-all; }
.test-fail-icon { color: var(--ip-danger-text); flex-shrink: 0; margin-top: 1px; }
.test-fail-text { color: var(--ip-danger-text); word-break: break-all; }

/* GroupedSelect 高度统一（和 form-input 一致） */
:deep(.gs-control) {
  height: var(--ip-input-h-sm);
}
.embed-key-link {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-primary-600);
  text-decoration: none;
  white-space: nowrap;
}
.embed-key-link:hover { text-decoration: underline; }

.empty-hint {
  padding: 16px 12px;
  text-align: center;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.6;
}

.spin { animation: rotate-cw 1s linear infinite; }
@keyframes rotate-cw {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}
@media (prefers-reduced-motion: reduce) {
  .spin { animation: none; }
}

/* ===== 问号提示图标 ===== */
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
</style>
