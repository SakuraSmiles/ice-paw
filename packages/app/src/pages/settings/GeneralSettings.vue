<!--
  GeneralSettings — 通用设置页（卡片分组式，2026-08-28 重设计）

  三卡：本地环境 / 视觉读取 / 语义检索。视觉与语义检索引用「设置-模型」里的
  模型配置实体（ModelProfile Phase 1：Key 一处换全局生效，实体在模型页维护，
  本页管用哪个）；旧版明文配置仍在生效时展示 legacy 提示行（读侧回落，
  boot 迁移失败下次重试）。

  约定（2026-10-31 用户拍板：平摊表格属性 = 即时生效，单组件原子性）：
  - 平铺在背景上的属性（工作空间/时区/视觉引用链）改了就存——无草稿、无保存
    按钮；工作空间/时区为「落库成功才更新回显」（非乐观），视觉链因 Combobox
    乐观回显为「先改回显再落库」（失败留新值 + 卡级 ErrorBanner retry）
  - 语义检索活引用换绑 = 选新引用即弹重建确认浮层（test → save → rebuild 三步
    流；与模型页被引用编辑共用 EmbedSwitchOverlay 组件）作为唯一提交点——确认=
    提交配置+重建、取消=回滚回显（测试失败浮层留开供重试，不自动回滚）；首次启用/
    停用直接提交（停用保留向量）
  - 图标一律 @lucide/vue（HelpCircle/FolderOpen/LocateFixed/ChevronDown...）
-->
<script setup lang="ts">
import { ref, computed, onMounted, onActivated, onUnmounted } from "vue";
import draggable from "vuedraggable";
import { open } from "@tauri-apps/plugin-dialog";
import { bridge } from "../../api/bridge";
import { setTimezone } from "../../utils/time";
import { msgOf, stripInvokePrefix } from "../../utils/errors";
import ErrorBanner from "../../components/common/ErrorBanner.vue";
import Combobox from "../../components/common/Combobox.vue";
import EmbedSwitchOverlay from "../../components/common/EmbedSwitchOverlay.vue";
import type { ComboboxItem } from "../../components/common/Combobox.vue";
import { useProviders } from "../../composables/useProviders";
import { useClickOutside } from "../../composables/useClickOutside";
import { useModelProfiles, profileById } from "../../composables/useModelProfiles";
import {
  HelpCircle, FolderOpen, LocateFixed, ChevronDown,
  Plus, Trash2, FlaskConical, Loader2, Check, X, GripVertical,
} from "@lucide/vue";
import type { UserPreferences } from "../../types";

const prefs = ref<UserPreferences>({});
const loading = ref(true);
// 页级数据源失败（UI-2 banner 形态）：prefs 不可信时全页表态，防照空表单误配置
const loadError = ref<string | null>(null);
// 卡级 inline 错误位（动作/落库失败）：key = 'workspaceOpen'|'workspaceSave'|'timezone'|'vision'|'embedding'|'datadir'
const saveErrors = ref<Record<string, { msg: string; retry?: () => void }>>({});

/** 短定时器登记（W6）：卸载即清，防迟到回调碰已卸载组件 */
const shortTimers = new Set<ReturnType<typeof setTimeout>>();
function later(fn: () => void, ms: number): void {
  const t = setTimeout(() => { shortTimers.delete(t); fn(); }, ms);
  shortTimers.add(t);
}
onUnmounted(() => { shortTimers.forEach(clearTimeout); shortTimers.clear(); });

// ---- 即时生效状态（2026-10-31 拍板）：平摊表格属性改了就存，无草稿/快照/保存按钮 ----
const workspace = ref("");
const timezone = ref("");
const visionIds = ref<string[]>([]);
const embeddingId = ref("");

/** prefs 回填即时态（无草稿可保——值即真相，刷新即覆盖为最新持久化值） */
function applyPrefs(raw: UserPreferences) {
  // 统一为 / 分隔符（后端 Windows 返回 \）
  if (raw.default_workspace_path) {
    raw.default_workspace_path = raw.default_workspace_path.replace(/\\/g, "/");
  }
  prefs.value = raw;
  workspace.value = raw.default_workspace_path ?? "";
  timezone.value = raw.timezone ?? "";
  visionIds.value = raw.vision_profile_ids ? [...raw.vision_profile_ids] : [];
  embeddingId.value = raw.embedding_profile_id ?? "";
}

async function load() {
  loading.value = true;
  loadError.value = null;
  try {
    const [raw] = await Promise.all([
      bridge.preferences.get(),
      loadProviders(),
      sharedLoad(),
    ]);
    applyPrefs(raw);
  } catch (e) {
    console.error("加载设置失败:", e);
    loadError.value = stripInvokePrefix(msgOf(e));
  } finally {
    loading.value = false;
  }
}

// =========================================================================
// 本地环境卡：工作空间 / 时区（即时落库）/ 数据目录（只读）
// =========================================================================

async function pickDirectory() {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择默认工作空间目录",
    defaultPath: workspace.value || undefined,
  });
  if (!selected) return;
  const p = selected.replace(/\\/g, "/");
  if (p === workspace.value) return; // 同值零写入
  await commitWorkspace(p);
}

/** 工作空间：落库成功才更新回显（非乐观）；失败记卡级错误 + retry */
async function commitWorkspace(p: string) {
  delete saveErrors.value.workspaceSave;
  try {
    await bridge.preferences.set("default_workspace_path", p);
    workspace.value = p;
  } catch (e) {
    console.error("保存工作空间失败:", e);
    saveErrors.value.workspaceSave = { msg: stripInvokePrefix(msgOf(e)), retry: () => void commitWorkspace(p) };
  }
}

const dataDir = ref("");

async function loadDataDir() {
  try {
    const raw = await bridge.logs.getDataDir();
    // 统一为 / 分隔符（后端 Windows 返回 \）
    dataDir.value = raw.replace(/\\/g, "/");
  } catch (e) {
    console.error("加载数据目录失败:", e);
  }
}

async function openDataDir() {
  delete saveErrors.value.datadir;
  try {
    await bridge.logs.openDataDir();
  } catch (e) {
    console.error("打开数据目录失败:", e);
    saveErrors.value.datadir = { msg: stripInvokePrefix(msgOf(e)), retry: () => void openDataDir() };
  }
}

/** 用系统文件管理器打开默认工作空间目录（镜像数据目录 openDataDir 行为） */
async function openWorkspace() {
  delete saveErrors.value.workspaceOpen;
  const p = workspace.value.trim();
  if (!p) return;
  try {
    await bridge.appInfo.openPath(p);
  } catch (e) {
    console.error("打开工作空间目录失败:", e);
    saveErrors.value.workspaceOpen = { msg: stripInvokePrefix(msgOf(e)), retry: () => void openWorkspace() };
  }
}

onMounted(load);
onMounted(loadDataDir);

// KeepAlive 瞬态清理：测试态是「刚操作过」的即时反馈，回到本页时已过期。
// 同时刷新即时态（legacy 迁移状态/引用实体可能在其他页变化——模型页增删
// profile 等；即时生效模式下无草稿，刷新即覆盖为最新持久化值）。
onActivated(async () => {
  visionTests.value = {};
  embedTest.value = { status: "idle" };
  try {
    applyPrefs(await bridge.preferences.get());
  } catch {
    // 刷新失败保留旧值（页面已可用，不整页报错）
  }
});

// =========================================================================
// 模型引用目录（实体在「设置-模型」维护；本页管用哪个）
// =========================================================================
const { providers: providerList, loadProviders } = useProviders();
const { profiles, loadModelProfiles: sharedLoad } = useModelProfiles();

function providerLabelOf(name: string): string {
  return providerList.value.find((p) => p.name === name)?.label ?? name;
}

/** 引用选择器条目（label 含厂商/模型，别名重复时也可辨认） */
const profileItems = computed<ComboboxItem[]>(() =>
  profiles.value.map((p) => ({
    label: `${p.alias}（${providerLabelOf(p.provider)} · ${p.model}）`,
    value: p.id,
  })),
);

type TestState =
  | { status: "idle" }
  | { status: "testing" }
  | { status: "ok"; msg: string }
  | { status: "fail"; msg: string };

function okFailMsg(t: TestState): string {
  return t.status === "ok" || t.status === "fail" ? stripInvokePrefix(t.msg) : "";
}

// =========================================================================
// 卡 2：视觉读取（profile 引用链——主模型 + 降级 N；即时落库）
// =========================================================================
const visionTests = ref<Record<number, TestState>>({});

/** 旧版视觉配置仍在生效（vision_profile_ids 未落且旧字段有值）——读侧回落中 */
const legacyVisionActive = computed(() => {
  const p = prefs.value;
  return p.vision_profile_ids == null && !!(p.vision_config?.length || p.vision_provider);
});

/** 引用链即时提交：过滤空占位行后落库（Some=权威，含 [] = 显式清空）；
    乐观回显（Combobox 选中即显），失败留新值 + 卡级错误 retry，不回滚 */
async function commitVision(next: string[]) {
  delete saveErrors.value.vision;
  const ids = next.filter((id) => id !== "");
  try {
    await bridge.preferences.set("vision_profile_ids", ids);
    visionIds.value = ids; // 空占位行随提交消解
  } catch (e) {
    saveErrors.value.vision = { msg: stripInvokePrefix(msgOf(e)), retry: () => void commitVision(next) };
  }
}

function onChainPick(i: number, v: string) {
  if (!profiles.value.some((p) => p.id === v)) return; // 手输不匹配 → 忽略
  const ids = [...visionIds.value];
  ids[i] = v;
  visionIds.value = ids;
  void commitVision(ids);
}

function removeChain(i: number) {
  const ids = visionIds.value.filter((_, idx) => idx !== i);
  visionIds.value = ids;
  void commitVision(ids);
}

function addChainEntry() {
  // 本地空占位行：选值后即时落库；空引用无信息量，永不落库
  if (visionIds.value.includes("")) return; // 已有未选占位行，不再重复加
  visionIds.value = [...visionIds.value, ""];
}

/** vuedraggable item-key：profile id 唯一；空占位行（至多一个）给固定键 */
const chainItemKey = (id: string) => id || "__empty__";

/** 拖拽调序结束 → 立即落库（v-model 已把 visionIds 换成新序） */
function onChainDragEnd() {
  void commitVision(visionIds.value);
}

async function testVisionAt(i: number) {
  const id = visionIds.value[i];
  if (!id) return;
  visionTests.value[i] = { status: "testing" };
  try {
    const r = await bridge.modelProfiles.testVision(id);
    visionTests.value[i] = { status: "ok", msg: `${r.latency_ms} ms · ${r.sample || "（空回复）"}` };
  } catch (e) {
    visionTests.value[i] = { status: "fail", msg: msgOf(e) };
  }
}

function visionTestOf(i: number): TestState {
  return visionTests.value[i] ?? { status: "idle" };
}
function visionTestMsgOf(i: number): string {
  return okFailMsg(visionTestOf(i));
}

// =========================================================================
// 卡 3：语义检索（单 profile 引用 + 切换重建 overlay；即时落库）
// =========================================================================
const embedTest = ref<TestState>({ status: "idle" });

/** 旧版语义检索配置仍在生效（embedding_profile_id 未落且旧四键齐全） */
const legacyEmbeddingActive = computed(() => {
  const p = prefs.value;
  return p.embedding_profile_id == null && !!(p.embedding_provider && p.embedding_model && p.embedding_api_key);
});

/** 引用是否活着（id 有值且实体存在——被删的悬空引用视同未启用） */
function isLiveProfile(id: string): boolean {
  return !!id && !!profileById(profiles.value, id);
}

/** 已落引用是否活着（测试按钮判据） */
const embeddingActive = computed(() => isLiveProfile(embeddingId.value));

function embeddingLabelOf(id: string): string {
  const p = profileById(profiles.value, id);
  return p ? `${p.alias}（${providerLabelOf(p.provider)} · ${p.model}）` : "（配置已删除）";
}

/** 选引用：活引用换绑 → 弹重建确认（乐观回显新值，确认=唯一提交点）；
    首次启用 / 空引用被替换 → 直接提交（停用保留向量，另见 stopEmbedding） */
function onEmbeddingPick(v: string) {
  if (!profiles.value.some((p) => p.id === v)) return; // 手输不匹配 → 忽略
  const prev = embeddingId.value;
  embeddingId.value = v; // 乐观回显（Combobox 选中即显，回显恒随 modelValue）
  if (isLiveProfile(prev) && v && v !== prev) {
    // 旧引用在用 → 二次确认（防维度不匹配静默失效）
    pendingSwitch.value = { prevId: prev, toId: v };
    switchError.value = null;
    return;
  }
  void commitEmbedding(v);
}

/** 停用（显式清空引用，Some 权威语义；保留向量，无重建） */
function stopEmbedding() {
  embeddingId.value = "";
  void commitEmbedding("");
}

/** 提交引用到偏好（不含重建——重建在 confirmEmbeddingSwitch 里） */
async function commitEmbedding(to: string) {
  delete saveErrors.value.embedding;
  embedTest.value = { status: "idle" };
  try {
    await bridge.preferences.set("embedding_profile_id", to);
    embeddingId.value = to;
  } catch (e) {
    saveErrors.value.embedding = { msg: stripInvokePrefix(msgOf(e)), retry: () => void commitEmbedding(to) };
  }
}

async function testEmbedding() {
  const id = embeddingId.value;
  if (!id) return;
  embedTest.value = { status: "testing" };
  try {
    // profileId 腿：服务端取存量凭据（四参留空不参与）
    await bridge.kb.testEmbeddingConfig("", "", "", undefined, id);
    embedTest.value = { status: "ok", msg: "连接正常" };
  } catch (e) {
    embedTest.value = { status: "fail", msg: msgOf(e) };
  }
}

// ---- 切换重建 overlay（本页触发源 = 卡 3 换绑时即时弹；模型页编辑被引用实体共用组件）----
const pendingSwitch = ref<{ prevId: string; toId: string } | null>(null);
const rebuilding = ref(false);
const switchError = ref<string | null>(null);
const switchInfo = ref<string | null>(null);

function cancelPending() {
  // 取消 = 回滚乐观回显（选择未成为提交点）
  const ps = pendingSwitch.value;
  if (ps) embeddingId.value = ps.prevId;
  pendingSwitch.value = null;
  switchError.value = null;
}

/** 先测新（未清旧）→ 存引用 → 全量重建。测失败 = overlay 留开 + 错误文案明示，
    可重试（再点确认）或取消（取消才回滚乐观回显）。 */
async function confirmEmbeddingSwitch() {
  const ps = pendingSwitch.value;
  if (!ps) return;
  rebuilding.value = true;
  switchError.value = null;
  try {
    await bridge.kb.testEmbeddingConfig("", "", "", undefined, ps.toId);
  } catch (e) {
    switchError.value = `切换失败：${stripInvokePrefix(msgOf(e))}（未切换，原配置保留）`;
    rebuilding.value = false;
    return; // overlay 留开（pendingSwitch 未清），供重试/取消
  }
  try {
    await bridge.preferences.set("embedding_profile_id", ps.toId);
    embeddingId.value = ps.toId;
    const stats = await bridge.kb.rebuildAllEmbeddings();
    pendingSwitch.value = null;
    switchInfo.value = `已切换并重建 ${stats.chunks} 个向量（${stats.kbs} 个知识库）`;
    later(() => { switchInfo.value = null; }, 4000);
  } catch (e) {
    // 引用已切、重建失败：诚实区分（不是「未切换」）——overlay 关闭，错误留卡内
    pendingSwitch.value = null;
    switchError.value = `已切换到新配置，但向量重建失败：${stripInvokePrefix(msgOf(e))}——可修正配置后重新测试；索引将在下次文档变更时按新配置生成`;
  } finally {
    rebuilding.value = false;
  }
}

// =========================================================================
// 时区选择器
// =========================================================================
// 遵循项目 Combobox 设计模式，提供搜索过滤 + 自动检测 + UTC 偏移展示

// IANA 时区列表（浏览器 API 获取）
const timezoneList: string[] = (() => {
  try {
    // Intl.supportedValuesOf 是 ES2021 API
    return ((Intl) as unknown as { supportedValuesOf(k: string): string[] }).supportedValuesOf("timeZone") || [];
  } catch {
    return [];
  }
})();

// 输入框状态
const tzInputOpen = ref(false);
const tzFilterText = ref("");
const tzInputRef = ref<HTMLInputElement | null>(null);
const tzDropdownRef = ref<HTMLElement | null>(null);
const tzWrapRef = ref<HTMLElement | null>(null);
const detecting = ref(false);

/** 计算某个 IANA 时区的当前 UTC 偏移（缓存：下拉逐项调用，避免每渲染重建数百个 Intl.DateTimeFormat） */
const tzOffsetCache = new Map<string, string>();
function getTzOffset(tz: string): string {
  const hit = tzOffsetCache.get(tz);
  if (hit !== undefined) return hit;
  try {
    const now = new Date();
    const fmt = new Intl.DateTimeFormat("zh-CN", {
      timeZone: tz,
      timeZoneName: "shortOffset",
    });
    const parts = fmt.formatToParts(now);
    const offset = parts.find((p) => p.type === "timeZoneName")?.value ?? "";
    tzOffsetCache.set(tz, offset);
    return offset;
  } catch {
    return "";
  }
}

/** 英文区域名 → 中文 */
const regionLabels: Record<string, string> = {
  "America": "美洲",
  "Asia": "亚洲",
  "Europe": "欧洲",
  "Africa": "非洲",
  "Atlantic": "大西洋",
  "Australia": "澳洲",
  "Pacific": "太平洋",
  "Indian": "印度洋",
  "Antarctica": "南极洲",
  "Arctic": "北极",
  "Etc": "其他",
};

/** 常见时区 → 中文城市名 */
const tzCityNames: Record<string, string> = {
  "Asia/Shanghai": "上海",
  "Asia/Chongqing": "重庆",
  "Asia/Hong_Kong": "香港",
  "Asia/Taipei": "台北",
  "Asia/Tokyo": "东京",
  "Asia/Seoul": "首尔",
  "Asia/Singapore": "新加坡",
  "Asia/Kuala_Lumpur": "吉隆坡",
  "Asia/Bangkok": "曼谷",
  "Asia/Dubai": "迪拜",
  "Asia/Kolkata": "加尔各答",
  "America/New_York": "纽约",
  "America/Los_Angeles": "洛杉矶",
  "America/Chicago": "芝加哥",
  "America/Denver": "丹佛",
  "America/Toronto": "多伦多",
  "America/Vancouver": "温哥华",
  "America/Sao_Paulo": "圣保罗",
  "Europe/London": "伦敦",
  "Europe/Paris": "巴黎",
  "Europe/Berlin": "柏林",
  "Europe/Moscow": "莫斯科",
  "Europe/Rome": "罗马",
  "Europe/Madrid": "马德里",
  "Australia/Sydney": "悉尼",
  "Australia/Melbourne": "墨尔本",
  "Pacific/Auckland": "奥克兰",
  "Pacific/Honolulu": "檀香山",
  "Pacific/Guam": "关岛",
  "Etc/UTC": "协调世界时",
  "UTC": "协调世界时",
};

/** 时区显示名：中文优先，回退为可读的英文名 */
function tzDisplayName(tz: string): string {
  return tzCityNames[tz] || tz.substring(tz.indexOf("/") + 1).replace(/_/g, " ");
}

/** 获取时区所属的中文区域名（如 America/New_York → "美洲"） */
function getTzRegion(tz: string): string {
  const idx = tz.indexOf("/");
  if (idx === -1) return "其他";
  const key = tz.substring(0, idx);
  return regionLabels[key] || key;
}

/** 对时区列表按区域分组 */
function groupedTimezones(search: string): Map<string, string[]> {
  const q = search.toLowerCase().trim();
  const groups = new Map<string, string[]>();
  for (const tz of timezoneList) {
    if (q && !tz.toLowerCase().includes(q)) continue;
    const region = getTzRegion(tz);
    if (!groups.has(region)) groups.set(region, []);
    groups.get(region)!.push(tz);
  }
  return groups;
}

/** 当前过滤后的时区列表（grouped） */
const filteredGroups = computed(() => groupedTimezones(tzFilterText.value));

/** 格式化的当前时区显示名（名称 + UTC 偏移，闭合态直接显示在输入框） */
const currentTzDisplay = computed(() => {
  const tz = timezone.value;
  if (!tz) return "";
  const offset = getTzOffset(tz);
  const name = tzDisplayName(tz);
  return offset ? `${name} (${offset})` : name;
});

/** 时区即时提交：落库成功才更新回显（非乐观）+ 同步全局；失败记卡级错误 + retry */
async function commitTimezone(tz: string) {
  delete saveErrors.value.timezone;
  try {
    await bridge.preferences.set("timezone", tz);
    timezone.value = tz;
    setTimezone(tz);
  } catch (e) {
    console.error("保存时区失败:", e);
    saveErrors.value.timezone = { msg: stripInvokePrefix(msgOf(e)), retry: () => void commitTimezone(tz) };
  }
}

/** 自动检测时区并即时落库 */
async function detectTimezone() {
  detecting.value = true;
  try {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (tz) await commitTimezone(tz);
  } catch (e) {
    console.error("detect tz failed:", e);
  } finally {
    detecting.value = false;
  }
}

/** 选中某个时区并即时落库 */
function selectTimezone(tz: string) {
  tzFilterText.value = "";
  tzInputOpen.value = false;
  if (tz === timezone.value) return; // 同值零写入
  void commitTimezone(tz);
}

/** 输入事件 */
function onInput(e: Event) {
  tzFilterText.value = (e.target as HTMLInputElement).value;
}

/** 打开下拉 */
function openDropdown() {
  tzInputOpen.value = true;
  tzFilterText.value = "";
  later(() => tzInputRef.value?.focus(), 0);
}

/** 关闭下拉 */
function closeDropdown() {
  tzInputOpen.value = false;
  tzFilterText.value = "";
}

/** 输入框获得焦点 — 切换为搜索模式 */
function onInputFocus() {
  tzInputOpen.value = true;
  tzFilterText.value = "";
  // 确保输入框可编辑并清空显示
  later(() => {
    if (tzInputRef.value) {
      tzInputRef.value.value = "";
    }
  }, 0);
}

function onInputBlur() {
  // 延迟关闭让点击选项先触发
  later(() => {
    if (!(tzDropdownRef.value?.contains(document.activeElement))) {
      closeDropdown();
    }
  }, 160);
}

function onInputKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") closeDropdown();
}

/** 点击外部关闭（下拉展开期间监听） */
useClickOutside(tzWrapRef, closeDropdown, () => tzInputOpen.value);

/** 过滤选项计数 — 空结果时展示无匹配 */
const hasFilterResults = computed(() => {
  for (const _ of filteredGroups.value) return true;
  return false;
});
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <div class="content-heading">
        <h2 class="content-title">通用设置</h2>
        <span class="content-en">GENERAL</span>
      </div>
    </div>

    <div v-if="loading" class="loading-state">加载中...</div>
    <template v-else>
      <ErrorBanner
        v-if="loadError"
        variant="banner"
        title="设置加载失败"
        :detail="loadError + '。下方显示的可能不是最新配置，重试成功前请勿编辑保存'"
        retry-label="重试"
        @retry="load"
      />
      <div class="settings-list" :class="{ 'list-untrusted': !!loadError }">

      <!-- ===== 卡 1：本地环境（即时生效）===== -->
      <section class="ds-section">
        <div class="section-head">
          <div class="section-head-row">
            <span class="section-num" aria-hidden="true">01</span>
            <h3 class="section-title">本地环境</h3>
            <span class="section-head-line" aria-hidden="true"></span>
          </div>
          <p class="section-hint">数据位置与区域偏好</p>
        </div>

        <div class="field">
          <div class="field-label">
            默认工作空间
            <span class="tip-icon" data-tip="新 Agent 未指定工作区时，自动在此目录下创建子文件夹">
              <HelpCircle :size="14" />
            </span>
          </div>
          <div class="input-group">
            <input
              :value="workspace"
              type="text"
              class="form-input is-pick"
              placeholder="点击选择目录"
              readonly
              @click="pickDirectory"
            />
            <button
              type="button"
              class="input-btn"
              title="在文件管理器中打开"
              :disabled="!workspace.trim()"
              @click="openWorkspace"
            >
              <FolderOpen :size="16" />
            </button>
          </div>
          <ErrorBanner v-if="saveErrors.workspaceSave" variant="inline" title="保存失败" :detail="saveErrors.workspaceSave.msg" :retry-label="saveErrors.workspaceSave.retry ? '重试' : null" @retry="saveErrors.workspaceSave?.retry?.()" />
          <ErrorBanner v-if="saveErrors.workspaceOpen" variant="inline" title="打开失败" :detail="saveErrors.workspaceOpen.msg" :retry-label="saveErrors.workspaceOpen.retry ? '重试' : null" @retry="saveErrors.workspaceOpen?.retry?.()" />
        </div>

        <div class="field">
          <div class="field-label">
            时区
            <span class="tip-icon" data-tip="设置后消息时间按当地时间显示，并作为上下文传给模型">
              <HelpCircle :size="14" />
            </span>
          </div>
          <div ref="tzWrapRef" class="tz-row">
            <button
              type="button"
              class="tz-detect-btn"
              :disabled="detecting"
              title="自动检测时区"
              @click="detectTimezone"
            >
              <LocateFixed v-if="!detecting" :size="14" />
              <span v-else class="tz-spinner" />
              <span>检测</span>
            </button>

            <div class="tz-combobox" :class="{ 'tz-open': tzInputOpen }">
              <input
                ref="tzInputRef"
                :value="tzInputOpen ? tzFilterText : currentTzDisplay"
                type="text"
                class="tz-input"
                :placeholder="tzInputOpen ? '搜索时区...' : '选择时区'"
                :readonly="!tzInputOpen"
                @input="onInput"
                @focus="onInputFocus"
                @blur="onInputBlur"
                @keydown="onInputKeydown"
              />
              <button
                type="button"
                class="tz-chevron"
                :class="{ rotated: tzInputOpen }"
                tabindex="-1"
                @mousedown.prevent
                @click="tzInputOpen ? closeDropdown() : openDropdown()"
              >
                <ChevronDown :size="14" />
              </button>

              <Transition name="tz-drop">
                <div v-if="tzInputOpen" ref="tzDropdownRef" class="tz-dropdown" @mousedown.prevent>
                  <template v-if="hasFilterResults">
                    <div v-for="[region, tzs] in filteredGroups" :key="region" class="tz-group">
                      <div class="tz-group-label">{{ region }}</div>
                      <button
                        v-for="tz in tzs" :key="tz" type="button"
                        :class="['tz-option', { active: timezone === tz }]"
                        @click="selectTimezone(tz)"
                      >
                        <span class="tz-opt-name">{{ tzDisplayName(tz) }}</span>
                        <span class="tz-opt-offset">{{ getTzOffset(tz) }}</span>
                      </button>
                    </div>
                  </template>
                  <div v-else class="tz-empty">无匹配时区</div>
                </div>
              </Transition>
            </div>
          </div>
          <ErrorBanner v-if="saveErrors.timezone" variant="inline" title="保存失败" :detail="saveErrors.timezone.msg" :retry-label="saveErrors.timezone.retry ? '重试' : null" @retry="saveErrors.timezone?.retry?.()" />
        </div>

        <div class="field">
          <div class="field-label">
            数据目录 · 只读
            <span class="tip-icon" data-tip="数据库、加密凭证、运行日志均存储在此目录">
              <HelpCircle :size="14" />
            </span>
          </div>
          <div class="input-group">
            <input
              :value="dataDir"
              type="text"
              class="form-input is-readonly"
              placeholder="加载中..."
              readonly
              tabindex="-1"
              aria-readonly="true"
            />
            <button type="button" class="input-btn" title="在文件管理器中打开" @click="openDataDir">
              <FolderOpen :size="16" />
            </button>
          </div>
          <ErrorBanner v-if="saveErrors.datadir" variant="inline" title="打开失败" :detail="saveErrors.datadir.msg" :retry-label="saveErrors.datadir.retry ? '重试' : null" @retry="saveErrors.datadir?.retry?.()" />
        </div>
      </section>

      <!-- ===== 卡 2：视觉读取（引用链；即时生效）===== -->
      <section class="ds-section">
        <div class="section-head">
          <div class="section-head-row">
            <span class="section-num" aria-hidden="true">02</span>
            <h3 class="section-title">
              视觉读取 · 图片代读
              <span class="tip-icon" data-tip="Agent 模型无视觉能力时，图片/扫描件由这里的配置链代读成文字。&#10;· Agent 自带视觉 → 直接用自己的模型读图，不经此链。&#10;· 按条目顺序尝试，首个成功即用；代读文本会标注实际读图模型。&#10;· 模型本体（含 Key）在「设置-模型」里维护。&#10;· 未配置 → 图片仅保留占位提示，无法代读。">
                <HelpCircle :size="14" />
              </span>
            </h3>
            <span class="section-head-line" aria-hidden="true"></span>
          </div>
          <p class="section-hint">无视觉能力的 Agent 发图时，按此链把图代读成文字（主模型 + 降级，逐个尝试）</p>
        </div>

        <div v-if="legacyVisionActive" class="legacy-note">
          旧版视觉配置仍在生效——下次启动将自动迁移为模型配置引用；在此处添加引用后改用新引用链。
        </div>

        <div v-if="visionIds.length === 0" class="vision-empty">
          <span v-if="legacyVisionActive">未配置引用链——旧版配置迁移前，代读继续走旧配置。</span>
          <span v-else>未配置——无视觉能力的 Agent 发图或扫描件将无法代读。先在「设置-模型」创建模型配置，再在此引用。</span>
        </div>
        <template v-else>
          <draggable
            v-model="visionIds"
            :item-key="chainItemKey"
            handle=".vision-drag"
            :animation="150"
            class="vision-chain"
            @end="onChainDragEnd"
          >
            <template #item="{ element: id, index }">
              <div class="vision-entry">
                <div class="vision-entry-row">
                  <span class="vision-drag" title="按住拖拽调整顺序" aria-hidden="true">
                    <GripVertical :size="16" />
                  </span>
                  <span class="vision-tag" :class="{ 'vision-tag--primary': index === 0 }">{{ index === 0 ? "主模型" : `降级 ${index}` }}</span>
                  <div class="vision-select">
                    <Combobox
                      :model-value="id"
                      :items="profileItems"
                      placeholder="选择模型配置"
                      @update:model-value="(v: string) => onChainPick(index, v)"
                    />
                  </div>
                  <button
                    class="btn"
                    :disabled="visionTestOf(index).status === 'testing' || !id"
                    title="用此配置代读 1×1 探针图（端点 + Key + 模型三合一验证）"
                    @click="testVisionAt(index)"
                  >
                    <Loader2 v-if="visionTestOf(index).status === 'testing'" :size="14" class="spin" />
                    <FlaskConical v-else :size="14" />
                    {{ visionTestOf(index).status === "testing" ? "测试中…" : "测试" }}
                  </button>
                  <button class="vision-icon-btn" title="移除此引用" @click="removeChain(index)">
                    <Trash2 :size="14" />
                  </button>
                </div>
                <div v-if="id && !profileById(profiles, id)" class="test-result">
                  <X :size="14" class="test-fail-icon" />
                  <span class="test-fail-text">该配置已被删除——请重新选择，或移除本条</span>
                </div>
                <div v-else-if="visionTestOf(index).status === 'ok' || visionTestOf(index).status === 'fail'" class="test-result">
                  <Check v-if="visionTestOf(index).status === 'ok'" :size="14" class="test-ok-icon" />
                  <X v-else :size="14" class="test-fail-icon" />
                  <span :class="visionTestOf(index).status === 'ok' ? 'test-ok-text' : 'test-fail-text'" :title="visionTestMsgOf(index)">{{ visionTestMsgOf(index) }}</span>
                </div>
              </div>
            </template>
          </draggable>
        </template>
        <!-- 添加入口恒在（空链时第一条 = 主模型；此前空链无入口，全新安装用户卡死） -->
        <button class="btn vision-add-fallback" @click="addChainEntry">
          <Plus :size="14" />{{ visionIds.length === 0 ? "添加视觉模型" : "添加降级模型" }}
        </button>
        <ErrorBanner
          v-if="saveErrors.vision"
          variant="inline"
          title="保存失败"
          :detail="saveErrors.vision.msg"
          :retry-label="saveErrors.vision.retry ? '重试' : null"
          @retry="saveErrors.vision?.retry?.()"
        />
      </section>

      <!-- ===== 卡 3：语义检索（单引用 + 切换重建；即时生效）===== -->
      <section class="ds-section">
        <div class="section-head">
          <div class="section-head-row">
            <span class="section-num" aria-hidden="true">03</span>
            <h3 class="section-title">语义检索 · 知识库索引</h3>
            <span class="section-head-line" aria-hidden="true"></span>
          </div>
          <p class="section-hint">配置后知识库支持语义匹配（向量检索），比关键词更精准；独立于聊天 Agent</p>
        </div>

        <div v-if="legacyEmbeddingActive" class="legacy-note">
          旧版语义检索配置仍在生效——下次启动将自动迁移为模型配置引用。
        </div>

        <div class="input-group">
          <Combobox
            :model-value="embeddingId"
            :items="profileItems"
            placeholder="未启用——选择模型配置"
            @update:model-value="onEmbeddingPick"
          />
          <button v-if="embeddingId" class="btn" title="停用语义检索（保留已生成的向量，关键词检索不受影响）" @click="stopEmbedding">停用</button>
          <button
            class="btn"
            :disabled="embedTest.status === 'testing' || !embeddingActive"
            title="用当前配置 embed 一条测试文本（端点 + Key + 模型三合一验证）"
            @click="testEmbedding"
          >
            <Loader2 v-if="embedTest.status === 'testing'" :size="14" class="spin" />
            <FlaskConical v-else :size="14" />
            {{ embedTest.status === "testing" ? "测试中…" : "测试" }}
          </button>
        </div>
        <div v-if="embedTest.status === 'ok' || embedTest.status === 'fail'" class="test-result">
          <Check v-if="embedTest.status === 'ok'" :size="14" class="test-ok-icon" />
          <X v-else :size="14" class="test-fail-icon" />
          <span :class="embedTest.status === 'ok' ? 'test-ok-text' : 'test-fail-text'" :title="okFailMsg(embedTest)">{{ okFailMsg(embedTest) }}</span>
        </div>
        <!-- 切换重建结果（overlay 关闭后留屏 4s——成功信息不随 overlay 蒸发） -->
        <div v-if="switchInfo" class="test-result">
          <Check :size="14" class="test-ok-icon" />
          <span class="test-ok-text">{{ switchInfo }}</span>
        </div>
        <div v-if="embeddingId && !profileById(profiles, embeddingId)" class="test-result">
          <X :size="14" class="test-fail-icon" />
          <span class="test-fail-text">引用的配置已被删除——语义检索处于未启用状态，请重新选择</span>
        </div>
        <ErrorBanner
          v-if="saveErrors.embedding"
          variant="inline"
          title="保存失败"
          :detail="saveErrors.embedding.msg"
          :retry-label="saveErrors.embedding.retry ? '重试' : null"
          @retry="saveErrors.embedding?.retry?.()"
        />
      </section>

      </div>
    </template>

    <!-- 切换语义检索模型：重建确认（与模型页被引用编辑共用 EmbedSwitchOverlay） -->
    <EmbedSwitchOverlay
      v-if="pendingSwitch"
      title="切换语义检索模型？"
      :rows="[
        { label: '当前', value: embeddingLabelOf(pendingSwitch.prevId) },
        { label: '切换到', value: embeddingLabelOf(pendingSwitch.toId) },
      ]"
      :error="switchError"
      :rebuilding="rebuilding"
      @cancel="cancelPending"
      @confirm="confirmEmbeddingSwitch"
    />
  </div>
</template>

<style scoped>
/* 页级数据不可信：内容降透明（配 ErrorBanner banner 形态） */
.settings-list.list-untrusted { opacity: 0.55; pointer-events: none; }

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
  padding: var(--ip-spacing-5) var(--ip-spacing-7) 0;
  flex-shrink: 0;
  height: 72px;
  border-bottom: 1px solid var(--ip-color-border-default);
}
.content-title {
  position: relative;
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0;
}

/* 标题下方分割线左端异色粗线段：宽=标题文字宽（自适应字数） */
.content-title::after {
  content: "";
  position: absolute;
  left: 0;
  bottom: -13px;
  width: 100%;
  height: 3px;
  background: var(--ip-accent-dsh);
}

.content-heading {
  display: flex;
  align-items: baseline;
  gap: var(--ip-spacing-3);
}

.content-en {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-micro-size);
  letter-spacing: 0.2em;
  color: var(--ip-accent-dsh);
}

.loading-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-body-sm-size);
}

/* ===== 卡片列表（滚动容器；限宽居中由 SettingsLayout 内容列统一控制） ===== */
.settings-list {
  flex: 1;
  width: 100%;
  padding: var(--ip-spacing-5) var(--ip-spacing-7) var(--ip-spacing-6);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-8);
  overflow-y: auto;
  /* 容器查询锚点：字段单行布局按「实际内容列宽」切换，而非 viewport——
     侧栏可拖动（280–380px），同窗口宽下侧栏收张差 100px，媒体查询会误判 */
  container-type: inline-size;
}

/* ===== DSH 分区（全平铺：无卡片、四角 L 标套准框、档案化头部）===== */
.ds-section {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-4);
  padding: var(--ip-spacing-6) var(--ip-spacing-7);
}

/* 档案化头部：中文主名 + hairline + 状态；描述句降为下一行 caption */
.section-head {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.section-head-row {
  position: relative;
  z-index: 0; /* 建立层叠上下文：背景数字垫在标题文本之下、仍浮于卡片背景之上 */
  display: flex;
  align-items: baseline;
  gap: var(--ip-spacing-3);
  padding-left: 21px; /* 标题右移约半个「0」字宽（72px × 0.578 / 2 ≈ 20.8px），数字左半露在标题左侧 */
}
/* 分组标题背景大字：数字为背景、以标题为锚，其余布局零改动（Oxanium 800，2026-10-31） */
.section-num {
  position: absolute;
  left: 0;
  top: calc(50% - 24px); /* 垂直偏移：相对标题行中线整体上调 24px */
  transform: translateY(-50%);
  z-index: -1;
  font-family: var(--ip-font-num);
  font-weight: 800;
  font-size: 72px;
  line-height: 1;
  letter-spacing: 0;
  color: var(--ip-accent-dsh);
  opacity: 0.08;
  pointer-events: none;
  user-select: none;
  white-space: nowrap;
}
.section-title {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-1_5);
  margin: 0;
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.section-head-line {
  flex: 1;
  align-self: center;
  height: 1px;
  background: var(--ip-color-border-default);
}
.section-hint {
  margin: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.5;
}

/* ===== 字段（AgentForm/McpForm 同款竖排惯例） ===== */
.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.field-label {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-1_5);
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
}

/* 单行布局（label 左、控件右）：阈值 500px——ds-section 左右 28px 内边距吃掉后
   字段可用 ~444px ≥ 128(label) + 12(gap) + ~300(control)。grid 免 wrapper：错误条
   天然落 row2·col2（恒在控件下方），label 只在 row1 垂直居中。 */
@container (min-width: 500px) {
  .field {
    display: grid;
    grid-template-columns: 128px 1fr;
    column-gap: var(--ip-spacing-3);
    row-gap: 4px;
    align-items: center;
  }
  .field-label {
    grid-column: 1;
    grid-row: 1;
  }
  .field > .input-group,
  .field > .tz-row {
    grid-column: 2;
    grid-row: 1;
    min-width: 0;
  }
  .field > .eb-inline {
    grid-column: 2;
    grid-row: 2;
  }
}

/* ===== 输入组 ===== */
.input-group {
  display: flex;
  gap: var(--ip-spacing-1_5);
  align-items: center;
}

/* 基础形态 = 独占一行：width 撑满、高度走令牌——勿写 flex:1，列向
   flex-basis 0% 会压过 height 把输入框压塌（模型页别名框实案同病根） */
.form-input {
  width: 100%;
  height: var(--ip-input-h-sm);
  padding: 0 var(--ip-spacing-2_5);
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
/* 行内组合（目录框 + 按钮）：input 占满剩余宽度 */
.input-group > .form-input {
  flex: 1;
  min-width: 0;
  width: auto;
}

/* 点选型只读框（工作空间=点击弹目录选择器）：手型光标暗示可交互 */
.form-input.is-pick { cursor: pointer; }
.form-input.is-pick:hover { border-color: var(--ip-color-border-focus); }

/* 只读展示态（如「数据目录」系统路径：不可编辑、不响应聚焦） */
.form-input.is-readonly {
  cursor: default;
  color: var(--ip-color-text-secondary);
}
.form-input.is-readonly:focus {
  border-color: var(--ip-color-border-default);
  background-color: var(--ip-color-bg-tertiary);
  box-shadow: none;
}

.input-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: var(--ip-input-h-sm);
  flex-shrink: 0;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.input-btn:hover {
  background-color: var(--ip-color-bg-secondary);
  border-color: var(--ip-color-border-focus);
  color: var(--ip-primary-600);
}
.input-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.input-btn:disabled:hover {
  background-color: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-default);
  color: var(--ip-color-text-secondary);
}

/* ===== 通用按钮（视觉/语义卡的测试/停用/添加） ===== */
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  height: var(--ip-input-h-sm);
  padding: 0 var(--ip-spacing-3);
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

/* Combobox 高度统一（和 form-input 一致） */
:deep(.combobox-input-wrap) {
  height: var(--ip-input-h-sm);
}

.spin { animation: rotate-cw 1s linear infinite; }
@keyframes rotate-cw {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}
@media (prefers-reduced-motion: reduce) {
  .spin { animation: none; }
}

/* ===== 视觉引用链条目 ===== */
.vision-empty {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.5;
}

.vision-chain {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
}
.vision-entry {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border-left: 2px solid var(--ip-color-border-default);
  background: transparent;
}
.vision-entry-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  min-height: 32px;
}
.vision-drag {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 22px;
  height: 24px;
  color: var(--ip-color-text-tertiary);
  cursor: grab;
}
.vision-drag:active { cursor: grabbing; }
.vision-tag {
  flex-shrink: 0;
  padding: 1px var(--ip-spacing-2);
  font-size: var(--ip-text-micro-size);
  font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-sm);
  color: var(--ip-color-text-secondary);
  border: 1px solid var(--ip-color-border-default);
}
.vision-tag--primary {
  color: var(--ip-color-text-on-primary);
  background: var(--ip-primary-600);
  border-color: var(--ip-primary-600);
}
.vision-select {
  flex: 1;
  min-width: 0;
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
.vision-icon-btn:hover:not(:disabled) { color: var(--ip-danger-text); background: var(--ip-color-bg-tertiary); }
.vision-icon-btn:disabled { opacity: 0.35; cursor: default; }
.vision-add-fallback { align-self: flex-end; }

/* legacy 提示行（旧配置仍在生效——读侧回落，等待 boot 迁移） */
.legacy-note {
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border-radius: var(--ip-radius-sm);
  background: var(--ip-warning-bg);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-warning-text);
  line-height: 1.5;
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
  left: 0;
  bottom: calc(100% + 6px);
  max-width: 260px;
  width: max-content;
  padding: 5px var(--ip-spacing-2_5);
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
  text-align: left;
}
.tip-icon:hover::after {
  opacity: 1;
}

/* 暗色模式 tooltip 背景变亮 */
[data-theme='dark'] .tip-icon::after {
  background: var(--ip-gray-200);
  color: var(--ip-gray-900);
}

/* =========================================================================
   时区选择器
   ========================================================================= */

.tz-row {
  display: flex;
  gap: var(--ip-spacing-1_5);
  align-items: center;
}

/* 检测按钮 */
.tz-detect-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  height: var(--ip-input-h-sm);
  padding: 0 var(--ip-spacing-2_5);
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
  white-space: nowrap;
}
.tz-detect-btn:hover {
  background-color: var(--ip-color-bg-secondary);
  border-color: var(--ip-color-border-focus);
  color: var(--ip-primary-600);
}
.tz-detect-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.tz-spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid var(--ip-color-border-default);
  border-top-color: var(--ip-primary-500);
  border-radius: 50%;
  animation: tz-spin 0.6s linear infinite;
}
@keyframes tz-spin {
  to { transform: rotate(360deg); }
}

/* Combobox */
.tz-combobox {
  position: relative;
  flex: 1;
  min-width: 200px;
  display: flex;
  align-items: center;
  height: var(--ip-input-h-sm);
  padding: 0 0 0 var(--ip-spacing-2_5);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.tz-combobox.tz-open,
.tz-combobox:focus-within {
  border-color: var(--ip-color-border-focus);
  background-color: var(--ip-color-bg-input);
  box-shadow: 0 0 0 3px rgba(var(--ip-primary-500-rgb), 0.12);
}

.tz-input {
  flex: 1;
  min-width: 0;
  height: 100%;
  border: none;
  outline: none;
  background: transparent;
  padding: 0;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  font-family: inherit;
}
.tz-input::placeholder {
  color: var(--ip-color-text-placeholder);
}
.tz-input[readonly] {
  cursor: default;
}

.tz-chevron {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 100%;
  background: transparent;
  border: none;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  flex-shrink: 0;
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}
.tz-chevron.rotated {
  transform: rotate(180deg);
}
.tz-chevron:hover {
  color: var(--ip-color-text-secondary);
}

/* 下拉列表 */
.tz-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  z-index: var(--ip-z-dropdown);
  max-height: 280px;
  overflow-y: auto;
  background-color: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-lg);
  padding: 4px;
}

.tz-group {
  margin-bottom: 2px;
}

.tz-group-label {
  padding: 5px var(--ip-spacing-2_5) 2px;
  font-size: var(--ip-text-micro-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-tertiary);
}

.tz-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-2);
  width: 100%;
  padding: 5px var(--ip-spacing-2_5);
  text-align: left;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.tz-option:hover {
  background-color: var(--ip-color-bg-sidebar-item-hover);
}
.tz-option.active {
  background-color: var(--ip-primary-500);
  color: var(--ip-color-text-on-primary);
}

.tz-opt-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tz-opt-offset {
  flex-shrink: 0;
  font-size: var(--ip-text-caption-size);
  font-variant-numeric: tabular-nums;
  opacity: 0.7;
}
.tz-option.active .tz-opt-offset {
  opacity: 1;
}

.tz-empty {
  padding: 7px var(--ip-spacing-2_5);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}

/* 下拉动画 */
.tz-drop-enter-active {
  animation: tz-drop-in 0.15s ease-out;
}
.tz-drop-leave-active {
  animation: tz-drop-in 0.1s ease-in reverse;
}
@keyframes tz-drop-in {
  from { opacity: 0; transform: translateY(-4px) scale(0.96); }
  to   { opacity: 1; transform: translateY(0) scale(1); }
}

</style>
