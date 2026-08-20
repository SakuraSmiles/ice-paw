<!--
  GeneralSettings — 通用设置页（937 行）

  逻辑分区（共享 prefs ref）：
  A. 加载/保存/重置      (~50 行)
  B. 主题 + 字体         (~60 行)
  C. 语言                (~30 行)
  D. 时区                (~200 行, 含 Intl 时区列表+搜索)
  E. 快捷键              (~100 行, 含录制模式)
  F. 自动滚动+时间戳     (~30 行)
  G. 工作区路径          (~80 行, 含 dialog 选择)
  H. 数据目录            (~40 行)

  未来迭代建议：D/E/G 区可提取为独立子组件（需处理 prefs ref 共享）。
-->
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { bridge } from "../../api/bridge";
import { setTimezone } from "../../utils/time";
import Combobox from "../../components/common/Combobox.vue";
import type { UserPreferences } from "../../types";

const prefs = ref<UserPreferences>({});
const loading = ref(true);
const saving = ref(false);
const saved = ref(false);
/** 用户主动操作（保存工作空间 / 打开数据目录）失败的可见反馈 */
const actionError = ref("");

async function load() {
  loading.value = true;
  try {
    const raw = await bridge.preferences.get();
    // 统一为 / 分隔符（后端 Windows 返回 \）
    if (raw.default_workspace_path) {
      raw.default_workspace_path = raw.default_workspace_path.replace(/\\/g, "/");
    }
    prefs.value = raw;
    oldEmbedding.value = { provider: raw.embedding_provider ?? "", model: raw.embedding_model ?? "" };
  } catch (e) {
    console.error("加载设置失败:", e);
  } finally {
    loading.value = false;
  }
}

async function pickDirectory() {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择默认工作空间目录",
    defaultPath: prefs.value.default_workspace_path || undefined,
  });
  if (selected) {
    prefs.value.default_workspace_path = selected;
  }
}

async function saveWorkspacePath() {
  saved.value = false;
  saving.value = true;
  actionError.value = "";
  try {
    await bridge.preferences.set(
      "default_workspace_path",
      prefs.value.default_workspace_path ?? "",
    );
    saved.value = true;
    setTimeout(() => { saved.value = false; }, 2000);
  } catch (e) {
    actionError.value = `保存失败：${e instanceof Error ? e.message : String(e)}`;
    console.error("保存失败:", e);
  } finally {
    saving.value = false;
  }
}

// =========================================================================
// 数据目录（数据库 / stronghold / 日志 所在目录）
// =========================================================================
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
  actionError.value = "";
  try {
    await bridge.logs.openDataDir();
  } catch (e) {
    actionError.value = `打开数据目录失败：${e instanceof Error ? e.message : String(e)}`;
    console.error("打开数据目录失败:", e);
  }
}

onMounted(load);

// ---- Embedding 配置 ----
const embeddingProviders = ["智谱 GLM", "OpenAI", "DeepSeek"];
const embeddingModelMap: Record<string, { provider: string; models: string[]; keyUrl: string }> = {
  "智谱 GLM": { provider: "glm", models: ["embedding-3"], keyUrl: "https://open.bigmodel.cn/usercenter/proj-mgmt/apikeys" },
  "OpenAI": { provider: "openai", models: ["text-embedding-3-small", "text-embedding-3-large"], keyUrl: "https://platform.openai.com/api-keys" },
  "DeepSeek": { provider: "deepseek", models: [], keyUrl: "https://platform.deepseek.com/api_keys" },
};
/** Combobox 展示用的 provider 名（智谱 GLM / OpenAI / DeepSeek），反向映射回内部 provider key */
const embeddingProviderDisplay = computed(() => {
  const p = prefs.value.embedding_provider || "";
  return Object.entries(embeddingModelMap).find(([, v]) => v.provider === p)?.[0] ?? "";
});
const embeddingModelSuggestions = computed(() => {
  return embeddingModelMap[embeddingProviderDisplay.value]?.models ?? [];
});
const embeddingKeyUrl = computed(() => embeddingModelMap[embeddingProviderDisplay.value]?.keyUrl ?? "");

watch(() => prefs.value.embedding_provider, (newProvider) => {
  // Provider 变化时自动推荐默认模型
  const display = Object.entries(embeddingModelMap).find(([, v]) => v.provider === newProvider)?.[0] ?? "";
  const models = embeddingModelMap[display]?.models ?? [];
  if (models.length > 0 && !prefs.value.embedding_model) {
    prefs.value.embedding_model = models[0];
    saveEmbedding();
  }
});

/** 上次成功保存的 embedding 配置（检测"切换"：provider/model 变化才需重建） */
const oldEmbedding = ref({ provider: "", model: "" });
const pendingSwitch = ref<{ provider: string; model: string } | null>(null);
const rebuilding = ref(false);
const switchError = ref<string | null>(null);
const switchInfo = ref<string | null>(null);

/** 旧配置是否正在用（provider+model+key 齐全） */
function isEmbeddingActive(): boolean {
  const o = oldEmbedding.value;
  return !!(o.provider && o.model && prefs.value.embedding_api_key);
}

/** provider 内部 key → 显示名（智谱 GLM/OpenAI/...），供 overlay 展示 */
function providerDisplayName(providerKey: string): string {
  return Object.entries(embeddingModelMap).find(([, v]) => v.provider === providerKey)?.[0] ?? providerKey;
}

function onEmbeddingProviderChange(displayName: string) {
  const mapping = embeddingModelMap[displayName];
  const newProvider = mapping?.provider ?? "";
  const newModel = mapping?.models[0] ?? "";
  // 旧配置在用 + provider 变 → 二次确认（防维度不匹配静默失效）
  if (isEmbeddingActive() && newProvider !== oldEmbedding.value.provider) {
    pendingSwitch.value = { provider: newProvider, model: newModel };
    switchError.value = null;
  } else {
    prefs.value.embedding_provider = newProvider;
    prefs.value.embedding_model = newModel;
    saveEmbedding();
    oldEmbedding.value = { provider: newProvider, model: newModel };
  }
}

function onEmbeddingModelChange(newModel: string) {
  if (isEmbeddingActive() && newModel !== oldEmbedding.value.model) {
    pendingSwitch.value = { provider: prefs.value.embedding_provider ?? "", model: newModel };
    switchError.value = null;
  } else {
    prefs.value.embedding_model = newModel;
    saveEmbedding();
    oldEmbedding.value = { provider: prefs.value.embedding_provider ?? "", model: newModel };
  }
}

/** 确认切换：健康检查新配置（先于清旧）→ 存 → 全量重建 */
async function confirmSwitch() {
  if (!pendingSwitch.value) return;
  const { provider, model } = pendingSwitch.value;
  rebuilding.value = true;
  switchError.value = null;
  try {
    // 1. 健康检查新配置（先于清旧，防切到无效 + 旧向量已清的双重失效）
    await bridge.kb.testEmbeddingConfig(
      provider, model,
      prefs.value.embedding_api_key ?? "",
      prefs.value.embedding_base_url ?? undefined,
    );
    // 2. 存新配置
    prefs.value.embedding_provider = provider;
    prefs.value.embedding_model = model;
    await saveEmbedding();
    // 3. 全量重建（清旧维度向量 + 重新生成）
    const stats = await bridge.kb.rebuildAllEmbeddings();
    oldEmbedding.value = { provider, model };
    pendingSwitch.value = null;
    switchInfo.value = `已切换并重建 ${stats.chunks} 个向量（${stats.kbs} 个知识库）`;
    setTimeout(() => { switchInfo.value = null; }, 4000);
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e);
    switchError.value = `切换失败：${msg}（未切换，原配置保留）`;
  } finally {
    rebuilding.value = false;
  }
}

function cancelSwitch() {
  pendingSwitch.value = null;
  switchError.value = null;
}

async function saveEmbedding() {
  saving.value = true;
  try {
    await Promise.all([
      bridge.preferences.set("embedding_provider", prefs.value.embedding_provider ?? ""),
      bridge.preferences.set("embedding_model", prefs.value.embedding_model ?? ""),
      bridge.preferences.set("embedding_api_key", prefs.value.embedding_api_key ?? ""),
    ]);
    saved.value = true;
    setTimeout(() => { saved.value = false; }, 2000);
  } catch (e) {
    console.error("保存 embedding 配置失败:", e);
  } finally {
    saving.value = false;
  }
}

// ---- Vision 配置（Phase B：扫描件/图片型 PDF 视觉读取）----
// 与 embedding 对称但更简：vision 是无状态的（每次 view_attachment_image 现取现用），
// 切换 provider/model 不需重建任何东西，故无 embedding 那套切换确认 overlay。
// 仅列真正提供视觉模型的 provider（DeepSeek 标准 API 无视觉模型，故不列）。
// MiniMax 仅 M3 支持图片输入（M2.x 不支持多模态），故只列 M3。
const visionProviders = ["智谱 GLM", "OpenAI", "MiniMax"];
const visionModelMap: Record<string, { provider: string; models: string[]; keyUrl: string }> = {
  "智谱 GLM": { provider: "glm", models: ["glm-4v-plus", "glm-4.5v", "glm-4v"], keyUrl: "https://open.bigmodel.cn/usercenter/proj-mgmt/apikeys" },
  "OpenAI": { provider: "openai", models: ["gpt-4o", "gpt-4o-mini"], keyUrl: "https://platform.openai.com/api-keys" },
  "MiniMax": { provider: "minimax", models: ["MiniMax-M3"], keyUrl: "https://platform.minimaxi.com/" },
};
const visionProviderDisplay = computed(() => {
  const p = prefs.value.vision_provider || "";
  return Object.entries(visionModelMap).find(([, v]) => v.provider === p)?.[0] ?? "";
});
const visionModelSuggestions = computed(() => visionModelMap[visionProviderDisplay.value]?.models ?? []);
const visionKeyUrl = computed(() => visionModelMap[visionProviderDisplay.value]?.keyUrl ?? "");

function onVisionProviderChange(displayName: string) {
  const mapping = visionModelMap[displayName];
  prefs.value.vision_provider = mapping?.provider ?? "";
  prefs.value.vision_model = mapping?.models[0] ?? "";
  saveVision();
}
function onVisionModelChange(newModel: string) {
  prefs.value.vision_model = newModel;
  saveVision();
}
async function saveVision() {
  saving.value = true;
  try {
    await Promise.all([
      bridge.preferences.set("vision_provider", prefs.value.vision_provider ?? ""),
      bridge.preferences.set("vision_model", prefs.value.vision_model ?? ""),
      bridge.preferences.set("vision_api_key", prefs.value.vision_api_key ?? ""),
    ]);
    saved.value = true;
    setTimeout(() => { saved.value = false; }, 2000);
  } catch (e) {
    console.error("保存 vision 配置失败:", e);
  } finally {
    saving.value = false;
  }
}

onMounted(loadDataDir);

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
const tzSelectedLabel = ref(""); // 选中后显示的标签文本

/** 计算某个 IANA 时区的当前 UTC 偏移 */
function getTzOffset(tz: string): string {
  try {
    const now = new Date();
    const fmt = new Intl.DateTimeFormat("zh-CN", {
      timeZone: tz,
      timeZoneName: "shortOffset",
    });
    const parts = fmt.formatToParts(now);
    const offset = parts.find((p) => p.type === "timeZoneName")?.value ?? "";
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

/** 是否在选择器中（有值或打开状态） */
const hasTimezone = computed(() => !!prefs.value.timezone);

/** 格式化的当前时区显示名 */
const currentTzDisplay = computed(() => {
  const tz = prefs.value.timezone;
  if (!tz) return "";
  const offset = getTzOffset(tz);
  const name = tzDisplayName(tz);
  return offset ? `${name} (${offset})` : name;
});

/** 自动检测时区 */
async function detectTimezone() {
  detecting.value = true;
  try {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (tz) {
      prefs.value.timezone = tz;
      await bridge.preferences.set("timezone", tz);
      setTimezone(tz); // 同步全局时区状态
      tzSelectedLabel.value = currentTzDisplay.value;
    }
  } catch (e) {
    console.error("detect tz failed:", e);
  } finally {
    detecting.value = false;
  }
}

/** 保存时区到后端 */
async function saveTimezone() {
  saving.value = true;
  try {
    await bridge.preferences.set("timezone", prefs.value.timezone ?? "");
    setTimezone(prefs.value.timezone ?? ""); // 同步全局时区状态，所有时间显示即时刷新
    saved.value = true;
    setTimeout(() => { saved.value = false; }, 2000);
  } catch (e) {
    console.error("save tz failed:", e);
  } finally {
    saving.value = false;
  }
}

/** 选中某个时区 */
function selectTimezone(tz: string) {
  prefs.value.timezone = tz;
  tzFilterText.value = "";
  tzInputOpen.value = false;
  tzSelectedLabel.value = currentTzDisplay.value;
  saveTimezone();
}

/** 输入事件 */
function onInput(e: Event) {
  tzFilterText.value = (e.target as HTMLInputElement).value;
}

/** 打开下拉 */
function openDropdown() {
  tzInputOpen.value = true;
  tzFilterText.value = "";
  setTimeout(() => tzInputRef.value?.focus(), 0);
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
  setTimeout(() => {
    if (tzInputRef.value) {
      tzInputRef.value.value = "";
    }
  }, 0);
}

function onInputBlur() {
  // 延迟关闭让点击选项先触发
  setTimeout(() => {
    if (!(tzDropdownRef.value?.contains(document.activeElement))) {
      closeDropdown();
    }
  }, 160);
}

function onInputKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") closeDropdown();
}

/** 点击外部关闭 */
function onDocClick(e: MouseEvent) {
  const el = tzWrapRef.value;
  if (el && !el.contains(e.target as Node)) {
    closeDropdown();
  }
}

onMounted(() => document.addEventListener("click", onDocClick));
onUnmounted(() => document.removeEventListener("click", onDocClick));

/** 过滤选项计数 — 空结果时展示无匹配 */
const hasFilterResults = computed(() => {
  for (const _ of filteredGroups.value) return true;
  return false;
});
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">通用设置</h2>
    </div>

    <div v-if="loading" class="loading-state">加载中...</div>
    <div v-else class="settings-list">

      <!-- 操作失败提示（保存工作空间 / 打开数据目录等用户主动操作） -->
      <div v-if="actionError" class="action-error">{{ actionError }}</div>

      <!-- ===== 工作空间 ===== -->
      <div class="setting-row">
        <div class="setting-label">
          <div class="setting-label-text">
            默认工作空间
            <span class="tip-icon" data-tip="新 Agent 未指定工作区时，自动在此目录下创建子文件夹">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>
            </span>
          </div>
        </div>
        <div class="setting-control">
          <div class="input-group">
            <input
              v-model="prefs.default_workspace_path"
              type="text"
              class="form-input"
              placeholder="选择或输入默认工作空间路径"
              readonly
              @click="pickDirectory"
            />
            <button type="button" class="input-btn" title="选择目录" @click="pickDirectory">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
              </svg>
            </button>
            <button type="button" class="btn-primary btn-sm" :disabled="saving" @click="saveWorkspacePath">
              {{ saving ? "保存中" : "保存" }}
            </button>
          </div>
          <span v-if="saved" class="save-tip">已保存</span>
        </div>
      </div>

      <!-- ===== 时区 ===== -->
      <div class="setting-row">
        <div class="setting-label">
          <div class="setting-label-text">
            时区
            <span class="tip-icon" data-tip="设置后消息时间按当地时间显示，并作为上下文传给模型">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>
            </span>
          </div>
        </div>
        <div class="setting-control">
          <div ref="tzWrapRef" class="tz-row">
            <button
              type="button"
              class="tz-detect-btn"
              :disabled="detecting"
              title="自动检测时区"
              @click="detectTimezone"
            >
              <svg
                v-if="!detecting"
                width="14" height="14" viewBox="0 0 24 24"
                fill="none" stroke="currentColor" stroke-width="2"
                stroke-linecap="round" stroke-linejoin="round"
              >
                <circle cx="12" cy="12" r="10" />
                <polyline points="12 6 12 12 16 14" />
              </svg>
              <span v-else class="tz-spinner" />
              <span>检测</span>
            </button>

            <div class="tz-combobox" :class="{ 'tz-open': tzInputOpen }">
              <input
                ref="tzInputRef"
                :value="tzInputOpen ? tzFilterText : (prefs.timezone ? tzDisplayName(prefs.timezone) : '')"
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
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <polyline points="6 9 12 15 18 9" />
                </svg>
              </button>

              <Transition name="tz-drop">
                <div v-if="tzInputOpen" ref="tzDropdownRef" class="tz-dropdown" @mousedown.prevent>
                  <template v-if="hasFilterResults">
                    <div v-for="[region, tzs] in filteredGroups" :key="region" class="tz-group">
                      <div class="tz-group-label">{{ region }}</div>
                      <button
                        v-for="tz in tzs" :key="tz" type="button"
                        :class="['tz-option', { active: prefs.timezone === tz }]"
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

          <div v-if="hasTimezone" class="tz-status on">
            <span class="tz-status-dot" />
            {{ currentTzDisplay }}
          </div>
          <div v-else class="tz-status off">
            未设置
          </div>
        </div>
      </div>

      <!-- ===== 数据目录 ===== -->
      <div class="setting-row">
        <div class="setting-label">
          <div class="setting-label-text">
            数据目录
            <span class="tip-icon" data-tip="数据库、加密凭证、运行日志均存储在此目录">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>
            </span>
          </div>
        </div>
        <div class="setting-control">
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
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M3 7v10a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-7l-2-3H5a2 2 0 0 0-2 2z" />
              </svg>
            </button>
          </div>
        </div>
      </div>

      <!-- ===== 知识库语义检索 ===== -->
      <div class="setting-row">
        <div class="setting-label">
          <div class="setting-label-text">
            语义检索
            <span class="tip-icon" data-tip="配置后知识库支持语义匹配（向量检索），比关键词更精准。独立于聊天 Agent。">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>
            </span>
          </div>
        </div>
        <div class="setting-control">
          <div class="input-group">
            <Combobox
              :model-value="embeddingProviderDisplay"
              :options="embeddingProviders"
              placeholder="未启用"
              @update:model-value="onEmbeddingProviderChange"
            />
          </div>
        </div>
      </div>
      <template v-if="prefs.embedding_provider">
        <div class="setting-row">
          <div class="setting-label">
            <div class="setting-label-text">检索模型</div>
          </div>
          <div class="setting-control">
            <div class="input-group">
              <Combobox
                v-if="embeddingModelSuggestions.length > 0"
                :model-value="prefs.embedding_model || ''"
                :options="embeddingModelSuggestions"
                placeholder="选择或输入模型名"
                @update:model-value="onEmbeddingModelChange"
              />
              <input v-else v-model="prefs.embedding_model" class="form-input" placeholder="输入模型名" @blur="saveEmbedding" />
            </div>
          </div>
        </div>
        <div class="setting-row">
          <div class="setting-label">
            <div class="setting-label-text">检索 API Key</div>
          </div>
          <div class="setting-control">
            <div class="input-group">
              <input v-model="prefs.embedding_api_key" type="password" class="form-input" placeholder="粘贴 API Key" @blur="saveEmbedding" />
            </div>
            <a v-if="embeddingKeyUrl" :href="embeddingKeyUrl" target="_blank" class="embed-key-link">申请 Key →</a>
          </div>
        </div>
      </template>

      <!-- 切换 embedding 模型确认 overlay -->
      <Transition name="overlay">
        <div v-if="pendingSwitch" class="embed-switch-overlay" @click.self="cancelSwitch">
          <div class="embed-switch-panel" @click.stop>
            <h3>切换语义检索模型？</h3>
            <p class="embed-switch-row">当前：<b>{{ providerDisplayName(oldEmbedding.provider) }} / {{ oldEmbedding.model }}</b></p>
            <p class="embed-switch-row">切换到：<b>{{ providerDisplayName(pendingSwitch.provider) }} / {{ pendingSwitch.model }}</b></p>
            <p class="embed-switch-warn">切换后知识库向量将失效并自动重建（可能需几十秒）。</p>
            <div v-if="switchError" class="embed-switch-error">{{ switchError }}</div>
            <div v-if="switchInfo" class="embed-switch-info">{{ switchInfo }}</div>
            <div class="embed-switch-actions">
              <button class="btn" :disabled="rebuilding" @click="cancelSwitch">取消</button>
              <button class="btn btn-primary" :disabled="rebuilding" @click="confirmSwitch">
                {{ rebuilding ? "重建中…" : "确认切换并重建" }}
              </button>
            </div>
          </div>
        </div>
      </Transition>

      <!-- ===== 视觉读取（Phase B：扫描件/图片型 PDF）===== -->
      <div class="setting-row">
        <div class="setting-label">
          <div class="setting-label-text">
            视觉读取
            <span class="tip-icon" data-tip="扫描件/图片型 PDF 文本提取为空时，由视觉模型把页面读成文字。&#10;· Agent 自带视觉（supports_vision）→ 优先用它自己的模型读图，无需此配置。&#10;· Agent 无视觉时，按顺序自动兜底：① 此处配置（精确控制模型/Key）→ ② Agent 自己的 GLM/OpenAI/MiniMax 凭据（glm-4v / gpt-4o / MiniMax-M3）→ ③ 已配的「GLM 视觉理解」MCP 凭据。&#10;即此处留空通常也能用——只要 Agent 是 GLM/OpenAI/MiniMax 系、或已配 GLM 视觉 MCP，扫描件即可自动代读。仅在想精确指定模型/Key 时才需填。">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>
            </span>
          </div>
        </div>
        <div class="setting-control">
          <div class="input-group">
            <Combobox
              :model-value="visionProviderDisplay"
              :options="visionProviders"
              placeholder="留空则自动兜底"
              @update:model-value="onVisionProviderChange"
            />
          </div>
        </div>
      </div>
      <template v-if="prefs.vision_provider">
        <div class="setting-row">
          <div class="setting-label">
            <div class="setting-label-text">视觉模型</div>
          </div>
          <div class="setting-control">
            <div class="input-group">
              <Combobox
                v-if="visionModelSuggestions.length > 0"
                :model-value="prefs.vision_model || ''"
                :options="visionModelSuggestions"
                placeholder="选择或输入模型名"
                @update:model-value="onVisionModelChange"
              />
              <input v-else v-model="prefs.vision_model" class="form-input" placeholder="输入模型名" @blur="saveVision" />
            </div>
          </div>
        </div>
        <div class="setting-row">
          <div class="setting-label">
            <div class="setting-label-text">视觉 API Key</div>
          </div>
          <div class="setting-control">
            <div class="input-group">
              <input v-model="prefs.vision_api_key" type="password" class="form-input" placeholder="粘贴 API Key" @blur="saveVision" />
            </div>
            <a v-if="visionKeyUrl" :href="visionKeyUrl" target="_blank" class="embed-key-link">申请 Key →</a>
          </div>
        </div>
      </template>

    </div>
  </div>
</template>

<style scoped>
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

.loading-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-body-sm-size);
}

/* ===== 设置列表 ===== */
.settings-list {
  flex: 1;
  padding: 4px 28px 24px;
  display: flex;
  flex-direction: column;
}

/* Embedding: Combobox 高度统一到 32px（和 form-input 一致） */
:deep(.combobox-input-wrap) {
  height: 32px;
}
.embed-key-link {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-primary-600);
  text-decoration: none;
  white-space: nowrap;
  flex-shrink: 0;
}
.embed-key-link:hover { text-decoration: underline; }

/* ===== 切换 embedding 模型确认 overlay ===== */
.embed-switch-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}
.embed-switch-panel {
  width: 380px;
  max-width: 90vw;
  padding: 20px 22px;
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: 12px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.15);
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.embed-switch-panel h3 {
  margin: 0 0 4px;
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.embed-switch-row {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
}
.embed-switch-warn {
  margin: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.5;
}
.embed-switch-error {
  font-size: var(--ip-text-caption-size);
  color: #e5484d;
  line-height: 1.5;
}
.embed-switch-info {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-success-text);
}
.embed-switch-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 6px;
}
.embed-switch-actions button {
  height: 30px;
  padding: 0 14px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-md);
  border: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.embed-switch-actions button.btn-primary {
  background: var(--ip-primary-600);
  border-color: var(--ip-primary-600);
  color: #fff;
}
.embed-switch-actions button:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.overlay-enter-active, .overlay-leave-active {
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out);
}
.overlay-enter-from, .overlay-leave-to {
  opacity: 0;
}

/* ===== 设置行 ===== */
.setting-row {
  display: flex;
  align-items: flex-start;
  padding: 14px 0;
  gap: 24px;
}

.setting-label {
  flex-shrink: 0;
  width: 120px;
}

.setting-label-text {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  line-height: 1.4;
  padding-top: 6px;
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
  padding: 5px 10px;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-on-primary);
  background: var(--ip-gray-800);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-lg);
  pointer-events: none;
  opacity: 0;
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out);
  z-index: 10;
  line-height: 1.5;
  text-align: center;
}
.tip-icon:hover::after {
  opacity: 1;
}

/* 暗色模式 tooltip 背景变亮 */
[data-theme='dark'] .tip-icon::after {
  background: var(--ip-gray-200);
  color: var(--ip-gray-900);
}

.setting-control {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

/* ===== 输入组 ===== */
.input-group {
  display: flex;
  gap: 6px;
  align-items: center;
}

.form-input {
  flex: 1;
  min-width: 0;
  height: 32px;
  padding: 0 10px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  outline: none;
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.form-input:focus {
  border-color: var(--color-input-focus-border);
  background-color: var(--ip-color-bg-secondary);
  box-shadow: 0 0 0 3px rgba(var(--ip-primary-500-rgb), 0.12);
}
.form-input::placeholder {
  color: var(--ip-color-text-placeholder);
}

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
  height: 32px;
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
  border-color: var(--color-input-focus-border);
  color: var(--ip-primary-600);
}

/* ===== 按钮 ===== */
.btn-primary {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  height: 32px;
  padding: 0 12px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: white;
  background-color: var(--ip-primary-500);
  border: none;
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  white-space: nowrap;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-primary:hover {
  background-color: var(--ip-primary-700);
}
.btn-primary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.save-tip {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-success-text);
}

/* 用户主动操作失败的可见提示 */
.action-error {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-danger-text);
  padding: 6px 10px;
  margin-bottom: 4px;
  background-color: var(--ip-danger-bg);
  border-radius: var(--ip-radius-md);
}

/* =========================================================================
   时区选择器
   ========================================================================= */

.tz-row {
  display: flex;
  gap: 6px;
  align-items: center;
}

/* 检测按钮 */
.tz-detect-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
  height: 32px;
  padding: 0 10px;
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
  border-color: var(--color-input-focus-border);
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
  height: 32px;
  padding: 0 0 0 10px;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.tz-combobox.tz-open,
.tz-combobox:focus-within {
  border-color: var(--color-input-focus-border);
  background-color: var(--color-input-bg);
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
  z-index: 100;
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
  padding: 5px 10px 2px;
  font-size: 10px;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-tertiary);
}

.tz-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  padding: 5px 10px;
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
  background-color: var(--color-sidebar-item-hover);
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
  padding: 7px 10px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}

/* 时区状态 */
.tz-status {
  font-size: var(--ip-text-caption-size);
  display: flex;
  align-items: center;
  gap: 5px;
  min-height: 20px;
}
.tz-status.on {
  color: var(--ip-color-text-secondary);
}
.tz-status.off {
  color: var(--ip-color-text-tertiary);
}
.tz-status-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background-color: var(--ip-primary-500);
  flex-shrink: 0;
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
