<script setup lang="ts">
// ProjectOverview.vue — 项目详情「概览」tab（2026-08-18 二轮重设计）：
//   纯统计带——4 数字卡（会话/委派任务/消息/成员）+ 任务状态宽卡（五桶
//   堆叠条 + 图例 + 最近活动）。纯排版（无图标块/无 hover 装饰，数字本体
//   即层级），Linear insights 式克制。
// 会话入口撤下（用户拍板：完整列表在侧栏，不重复造）；任务台账 v1 表格
// 已撤（重设计中，数据入口在统计分桶 + 轨迹 tab）。
// live：useProjectTasks 事件驱动（delegation-started / turn_ended 去抖）；
// keep-alive 离开期间错过的事件由 onActivated 补拉兜底。
import { computed, onActivated, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { bridge } from "../../api/bridge";
import { useProjectTasks } from "../../composables/useProjectTasks";
import { useAgentStore } from "../../stores/agent";
import { useChatStore } from "../../stores/chat";
import { useProjectStore } from "../../stores/project";
import { taskStatus, TASK_STATUS_LABELS } from "../../utils/taskStatus";
import { formatTokenCompact } from "../../utils/format";
import { timeAgo } from "../../utils/time";
import type { ProjectOverview as OverviewData } from "../../types";

const route = useRoute();
const router = useRouter();
const chat = useChatStore();
const project = useProjectStore();
const agent = useAgentStore();

const projectId = computed(() => String(route.params.id ?? ""));
const overview = ref<OverviewData | null>(null);

const { tasks, load: loadTasks, refresh } = useProjectTasks(projectId);

async function loadOverview() {
  try {
    overview.value = await bridge.projects.getOverview(projectId.value);
  } catch {
    overview.value = null; // 静默：统计带显示占位 —
  }
}

watch(projectId, () => {
  void loadOverview();
  void loadTasks();
}, { immediate: true });

onActivated(() => {
  void loadOverview();
  refresh();
});

// 相对时间每分钟刷新（Sidebar nowTick 同款机制）
const nowTick = ref(Date.now());
let tick: ReturnType<typeof setInterval> | null = null;
onMounted(() => {
  tick = setInterval(() => (nowTick.value = Date.now()), 60000);
  if (!agent.loaded) void agent.load(); // 成员分布的名字/模型解析依赖
});
onUnmounted(() => { if (tick) clearInterval(tick); });

function ago(iso: string): string {
  return timeAgo(iso, nowTick.value);
}

const lastActivityLabel = computed(() => {
  const at = overview.value?.last_activity_at;
  return at ? ago(at) : "";
});

/** 成员卡点击 → 设置 tab（成员管理所在） */
function gotoMembers() {
  router.push(`/projects/${projectId.value}/settings`);
}

const memberCount = computed(() => project.getById(projectId.value)?.agents?.length ?? 0);

// ---- 任务状态宽卡：五桶（running overlay 后的实时视角） ----
const bucketKeys = ["running", "done", "failed", "interrupted", "ended-other"] as const;
const buckets = computed(() => {
  const b = { running: 0, done: 0, failed: 0, interrupted: 0, "ended-other": 0 };
  for (const t of tasks.value) b[taskStatus(t, chat.streamingConvIds)] += 1;
  return b;
});
const taskTotal = computed(() => tasks.value.length);

// ---- 项目成员：环图（token 口径）+ 横条排行 ----
// 名字/模型由 agent store 解析（SQL 只回 id——migration 45 不设 FK，展示层职责）。
// 视觉主口径 = token 估算（环图分段 + 条宽归一），消息数做行尾小字；
// token 全零（估算未回填的旧库）回退消息口径并隐藏环图，诚实不伪造。
// >TOP_N 截断聚合为「其他 N 位」；占比进 title 不占版面。
const TOP_N = 5;
/** 环图/色点/条 共用顺序色板：primary 单色阶（克制、双主题成立、无语义误编码） */
const PALETTE = [
  "var(--ip-primary-600)",
  "var(--ip-primary-500)",
  "var(--ip-primary-400)",
  "var(--ip-primary-300)",
  "var(--ip-primary-200)",
];
const COLOR_OTHER = "var(--ip-color-text-tertiary)";

interface ShareRow {
  key: string;
  name: string;
  model: string | null;
  messages: number;
  tokens: number;
  other: boolean;
  othersCount: number;
  color: string;
}
const shareRows = computed<ShareRow[]>(() => {
  const shares = overview.value?.agent_shares ?? [];
  const named = (s: { agent_id: string; messages: number; tokens: number }, i: number): ShareRow => {
    const a = agent.getById(s.agent_id);
    return {
      key: s.agent_id,
      name: a?.name ?? "未知成员",
      model: a?.model ?? null,
      messages: s.messages,
      tokens: s.tokens,
      other: false,
      othersCount: 0,
      color: PALETTE[i] ?? COLOR_OTHER,
    };
  };
  if (shares.length <= TOP_N) return shares.map(named);
  const rest = shares.slice(TOP_N);
  return [
    ...shares.slice(0, TOP_N).map(named),
    {
      key: "__others__",
      name: `其他 ${rest.length} 位`,
      model: null,
      messages: rest.reduce((sum, s) => sum + s.messages, 0),
      tokens: rest.reduce((sum, s) => sum + s.tokens, 0),
      other: true,
      othersCount: rest.length,
      color: COLOR_OTHER,
    },
  ];
});

const tokensTotal = computed(() =>
  shareRows.value.reduce((sum, r) => sum + r.tokens, 0),
);
const hasTokens = computed(() => tokensTotal.value > 0);
/** 视觉口径数值：token 优先，全零回退消息数 */
function metric(r: ShareRow): number {
  return hasTokens.value ? r.tokens : r.messages;
}
const metricMax = computed(() =>
  shareRows.value.reduce((m, r) => Math.max(m, metric(r)), 0),
);
const metricTotal = computed(() =>
  shareRows.value.reduce((s, r) => s + metric(r), 0),
);
/** 条宽 %（相对最大行；榜首满宽对比感最强） */
function shareWidth(r: ShareRow): string {
  if (metricMax.value <= 0) return "0%";
  return `${Math.max((metric(r) / metricMax.value) * 100, 1.5)}%`;
}
/** 占比 %（相对总量；进 title，不占版面） */
function sharePercent(r: ShareRow): string {
  if (metricTotal.value <= 0) return "0%";
  return `${Math.round((metric(r) / metricTotal.value) * 100)}%`;
}
/** 行/弧段 hover title：名字 · 占比（token + 消息双口径） */
function rowTitle(r: ShareRow): string {
  const tok = hasTokens.value ? `${formatTokenCompact(r.tokens)} tokens · ` : "";
  return `${r.name} · ${sharePercent(r)}（${tok}${r.messages} 条）`;
}

// ---- 环图弧段（SVG stroke-dasharray；r=40 周长恒定） ----
const DONUT_C = 2 * Math.PI * 40;
/** hover 联动：环图段 ↔ 排行行双向聚焦（key 共享；null = 无 hover）。
 *  反馈语言 = 聚焦透镜：聚焦段变粗（stroke 11→14.5）+ 其余段/行整体淡化，
 *  中心切为该成员值——粗细对比 + 全场淡出，不止颜色差异。 */
const hoverKey = ref<string | null>(null);
const hoverRow = computed(() =>
  shareRows.value.find((r) => r.key === hoverKey.value) ?? null,
);
/** 环图无障碍标签（榜首 + 占比） */
const donutAria = computed(() => {
  const top = shareRows.value[0];
  return top ? `token 占比：${top.name} ${sharePercent(top)}` : "token 占比";
});
const donutSegs = computed(() => {
  const rows = shareRows.value;
  const total = metricTotal.value;
  if (total <= 0) return [];
  /* 几何模型（所见即所得）：
   * round cap 会让每端向外延伸 CAP（= 线宽一半），dash 路径本身不包含这段。
   * 因此 dash 长 = 目标视觉长 - 2·CAP、dash 起点右移 CAP——所见弧段长与
   * 间隙精确可控：段间视觉间隙恒为 SEG_GAP，hover 变粗（CSS 11→14.5，
   * 每端多伸 1.75）也落在 SEG_GAP 余量内，不压邻段。
   * 极小占比段：视觉长下限 = 一个胶囊直径（dash→0，退化成圆点）。 */
  const STROKE = 11;          // 与 CSS .donut-seg stroke-width 保持一致
  const CAP = STROKE / 2;     // round cap 每端延伸
  const SEG_GAP = 4;          // 段间视觉间隙（弧长单位，≈6.4px @160px 环）
  const multi = rows.length > 1;
  let offset = 0;
  return rows.map((r) => {
    const len = (metric(r) / total) * DONUT_C;
    // 单段满环：dash = C - 2·CAP，两端 cap 各补 CAP，闭合成无缝整圆
    const vis = multi ? Math.max(len - SEG_GAP, STROKE) : len;
    const dash = Math.max(vis - CAP * 2, 0.01);
    const start = offset + (multi ? SEG_GAP / 2 : 0) + CAP;
    const seg = { key: r.key, row: r, len: dash, offset: start };
    offset += len;
    return seg;
  });
});
</script>

<template>
  <div class="overview">
    <!-- ===== 统计带：4 数字卡 + 任务状态宽卡 ===== -->
    <div class="stat-band">
      <div class="stat-card">
        <span class="stat-value">{{ overview?.chat_conversations ?? "—" }}</span>
        <span class="stat-label">会话</span>
      </div>
      <div class="stat-card">
        <span class="stat-value">{{ overview?.delegation_conversations ?? "—" }}</span>
        <span class="stat-label">委派任务</span>
      </div>
      <div class="stat-card">
        <span class="stat-value">{{ overview?.messages ?? "—" }}</span>
        <span class="stat-label">消息</span>
      </div>
      <!-- 成员卡：点击直达设置 tab 成员管理 -->
      <button class="stat-card stat-card-click" title="管理项目成员" @click="gotoMembers">
        <span class="stat-value">{{ memberCount }}</span>
        <span class="stat-label">
          成员
          <svg class="label-chevron" width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6" /></svg>
        </span>
      </button>

      <!-- 任务状态宽卡：五桶堆叠条 + 图例 + 项目节奏 -->
      <div class="stat-card task-mix">
        <div v-if="taskTotal > 0" class="mix-body">
          <div class="mix-bar" role="img" :aria-label="`任务状态：进行中 ${buckets.running}，已完成 ${buckets.done}`">
            <span
              v-for="k in bucketKeys"
              v-show="buckets[k] > 0"
              :key="k"
              class="mix-seg"
              :class="k"
              :style="{ flexGrow: buckets[k] }"
            />
          </div>
          <div class="mix-legend">
            <span v-for="k in bucketKeys" v-show="buckets[k] > 0" :key="k" class="mix-item">
              <i class="dot" :class="k" />{{ TASK_STATUS_LABELS[k] }} {{ buckets[k] }}
            </span>
          </div>
        </div>
        <div v-else class="mix-empty">暂无委派任务</div>
        <div class="mix-meta">
          <span>共 {{ taskTotal }} 个任务</span>
          <span v-if="lastActivityLabel">最近活动 {{ lastActivityLabel }}</span>
        </div>
      </div>
    </div>

    <!-- ===== 项目成员：环图（token 口径）+ 横条排行（消息数小字） ===== -->
    <div v-if="shareRows.length > 0" class="stat-card share-card">
      <div class="share-head">
        <span class="share-title">项目成员</span>
        <span class="share-meta">{{ hasTokens ? "token 估算 · 消息数" : "按消息数" }}</span>
      </div>
      <div class="share-body">
        <!-- 环图：token 占比分段，中心总量；token 全零时整体隐藏（回退消息口径）。
             hover = 聚焦透镜：聚焦段变粗 + 其余段/行淡化 + 中心切该成员值 -->
        <div v-if="hasTokens" class="donut-wrap">
          <svg viewBox="0 0 100 100" class="donut" role="img" :aria-label="donutAria" @mouseleave="hoverKey = null">
            <circle class="donut-track" cx="50" cy="50" r="40" />
            <circle
              v-for="s in donutSegs"
              :key="s.key"
              class="donut-seg"
              :class="{ active: hoverKey === s.key, dim: hoverKey !== null && hoverKey !== s.key }"
              cx="50" cy="50" r="40"
              :stroke="s.row.color"
              :stroke-dasharray="`${s.len} ${DONUT_C - s.len}`"
              :stroke-dashoffset="-s.offset"
              @mouseenter="hoverKey = s.key"
            >
              <title>{{ rowTitle(s.row) }}</title>
            </circle>
          </svg>
          <div class="donut-center">
            <template v-if="hoverRow">
              <span class="donut-value">{{ formatTokenCompact(hoverRow.tokens) }}</span>
              <span class="donut-sub donut-sub-name" :title="rowTitle(hoverRow)">{{ hoverRow.name }} · {{ sharePercent(hoverRow) }}</span>
            </template>
            <template v-else>
              <span class="donut-value">{{ formatTokenCompact(tokensTotal) }}</span>
              <span class="donut-sub donut-label">TOKENS</span>
            </template>
          </div>
        </div>
        <!-- 四列共享 grid（名字/模型 | 轨道条 | tokens | 消息数）——跨行天然
             列对齐；行容器 display:contents 只留分组语义，title 挂 label 格
             （contents 元素不渲染盒子，title 无效）。数字列显式 grid-column，
             缺格不塌位（other 行无消息小字）。
             hover（反向）：cell enter 设 key（cell 无 leave，跨 cell/跨行
             移动由 enter 覆盖），容器级 leave 统一清空；聚焦行提亮、其余行淡化。 -->
        <div class="share-rows" @mouseleave="hoverKey = null">
          <div
            v-for="row in shareRows"
            :key="row.key"
            class="share-row"
            :class="{ hovered: hoverKey === row.key, dim: hoverKey !== null && hoverKey !== row.key }"
          >
            <div class="share-label" :title="rowTitle(row)" @mouseenter="hoverKey = row.key">
              <i class="share-dot" :style="{ background: row.color }" />
              <span class="share-name">{{ row.name }}</span>
              <span v-if="row.model" class="share-model">{{ row.model }}</span>
            </div>
            <div class="share-track" @mouseenter="hoverKey = row.key">
              <span class="share-bar" :style="{ width: shareWidth(row), background: row.color }" />
            </div>
            <span class="count-tokens" @mouseenter="hoverKey = row.key">{{ hasTokens ? formatTokenCompact(row.tokens) : row.messages }}</span>
            <span v-if="hasTokens && !row.other" class="count-msgs" @mouseenter="hoverKey = row.key">{{ row.messages }} 条</span>
          </div>
        </div>
      </div>
    </div>
    <div v-else class="stat-card share-card">
      <div class="share-head">
        <span class="share-title">项目成员</span>
        <span class="share-meta">token 估算 · 消息数</span>
      </div>
      <div class="mix-empty">成员暂无消息</div>
    </div>
    <!-- 会话入口（用户拍板撤下：完整列表在侧栏）与任务台账区（重设计中）
         先空着，只留统计带 -->
  </div>
</template>

<style scoped>
.overview {
  flex: 1; min-height: 0;
  display: flex; flex-direction: column; gap: 18px;
  overflow-y: auto;
  max-width: 960px;
  width: 100%;
  margin: 0 auto;
  scrollbar-width: none;
}
.overview::-webkit-scrollbar { display: none; }

/* ===== 统计带：纯排版（数字本体即层级，无图标块/无装饰） ===== */
.stat-band {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr)) minmax(0, 1.9fr);
  gap: 12px;
  align-items: stretch;
}
.stat-card {
  display: flex; flex-direction: column; justify-content: center;
  gap: 6px;
  padding: 18px 20px;
  background-color: var(--ip-color-bg-secondary);
  border-radius: var(--ip-radius-lg);
  font-family: inherit; text-align: left;
}
.stat-card-click {
  border: none; cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.stat-card-click:hover { background-color: var(--ip-color-bg-tertiary); }
.stat-card-click:hover .stat-value { color: var(--ip-primary-600); }

.stat-value {
  font-size: 28px; line-height: 1.1;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  font-variant-numeric: tabular-nums;
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.stat-label {
  display: inline-flex; align-items: center; gap: 3px;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
}
.label-chevron { opacity: 0; margin-left: -2px; transition: opacity var(--ip-duration-fast) var(--ip-ease-out); }
.stat-card-click:hover .label-chevron { opacity: 0.8; }

/* 任务状态宽卡 */
.task-mix { padding: 16px 20px; gap: 10px; }
.mix-body { display: flex; flex-direction: column; gap: 10px; flex: 1; justify-content: center; }
.mix-bar {
  display: flex; gap: 2px;
  height: 6px; border-radius: var(--ip-radius-full);
  overflow: hidden;
}
.mix-seg { height: 100%; min-width: 4px; }
.mix-seg.running { background: var(--ip-primary-500); }
.mix-seg.done { background: var(--ip-success-base); }
.mix-seg.failed { background: var(--ip-danger-base, var(--ip-danger-text)); }
.mix-seg.interrupted { background: var(--ip-warning-base); }
.mix-seg.ended-other { background: var(--ip-color-text-tertiary); }

.mix-legend { display: flex; flex-wrap: wrap; gap: 4px 14px; }
.mix-item {
  display: inline-flex; align-items: center; gap: 5px;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary);
  font-variant-numeric: tabular-nums;
}
.dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; }
.dot.running { background: var(--ip-primary-500); animation: mix-pulse 1.2s ease-in-out infinite; }
.dot.done { background: var(--ip-success-base); }
.dot.failed { background: var(--ip-danger-base, var(--ip-danger-text)); }
.dot.interrupted { background: var(--ip-warning-base); }
.dot.ended-other { background: var(--ip-color-text-tertiary); }
@keyframes mix-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }

.mix-empty {
  flex: 1; display: flex; align-items: center;
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary);
}

/* ===== 成员负载：环图 + 横条排行 ===== */
.share-card { gap: 14px; }
.share-head { display: flex; align-items: baseline; gap: 8px; }
.share-title {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
}
.share-meta { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

.share-body {
  display: flex;
  align-items: center;
  gap: 32px;
}

/* 环图：SVG 弧段 + 绝对定位中心文字（160px 大环做视觉锚点） */
.donut-wrap { position: relative; flex-shrink: 0; width: 160px; height: 160px; }
.donut { width: 100%; height: 100%; transform: rotate(-90deg); }
.donut-track {
  fill: none;
  stroke: var(--ip-color-bg-tertiary);
  stroke-width: 11;
}
.donut-seg {
  fill: none;
  stroke-width: 11; /* 几何常量：改这里须同步 script donutSegs 的 STROKE */
  stroke-linecap: round;
  pointer-events: stroke;
  transition: stroke-width 180ms var(--ip-ease-out, ease), opacity 160ms var(--ip-ease-out, ease);
}
/* 聚焦透镜：聚焦段变粗（径向扩张，占位不变），其余段淡出 */
.donut-seg.active { stroke-width: 14.5; }
.donut-seg.dim { opacity: 0.25; }
.donut-center {
  position: absolute; inset: 0;
  display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 4px;
  pointer-events: none;
}
.donut-value {
  font-size: 20px;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  font-variant-numeric: tabular-nums;
  line-height: 1.1;
}
.donut-sub { font-size: 11px; color: var(--ip-color-text-tertiary); }
/* 默认态单位小标：拉开字距的小型大写风格（科技感排版惯例） */
.donut-label {
  font-size: 9.5px;
  letter-spacing: 0.16em;
  margin-right: -0.16em; /* 补偿末字符字距，视觉居中 */
}
/* hover 态中心副行换成成员名+占比：聚焦态环内径 ~105px，溢出 ellipsis */
.donut-sub-name {
  max-width: 104px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  color: var(--ip-color-text-secondary);
}

/* 排行：四列共享 grid——所有行的 tokens/消息数字列天然右对齐同列，
   行容器 display:contents 只留分组语义（跨行对齐的关键） */
.share-rows {
  flex: 1; min-width: 0;
  display: grid;
  grid-template-columns: minmax(140px, 220px) 1fr auto auto;
  column-gap: 14px;
  row-gap: 13px;
  align-items: center;
}
.share-row { display: contents; }
.share-label {
  grid-column: 1;
  display: flex; align-items: center; gap: 8px;
  min-width: 0;
  transition: opacity 160ms var(--ip-ease-out, ease);
}
.share-dot {
  width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0;
  transition: transform 160ms var(--ip-ease-out, ease);
}
.share-name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  transition: color 160ms var(--ip-ease-out, ease);
}
.share-model {
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.share-track {
  grid-column: 2;
  height: 6px;
  border-radius: var(--ip-radius-full);
  background-color: var(--ip-color-bg-tertiary);
  overflow: hidden;
  transition: opacity 160ms var(--ip-ease-out, ease);
}
.share-bar {
  display: block; height: 100%;
  border-radius: inherit;
  transition: width var(--ip-duration-normal, 200ms) var(--ip-ease-out);
}
.count-tokens {
  grid-column: 3;
  justify-self: end;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  transition: color 160ms var(--ip-ease-out, ease), opacity 160ms var(--ip-ease-out, ease);
}
.count-msgs {
  grid-column: 4;
  justify-self: end;
  min-width: 3.5em;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-disabled);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  transition: opacity 160ms var(--ip-ease-out, ease);
}

/* hover 聚焦透镜（行侧）：聚焦行名字/数字提亮 + 色点放大，
   其余行整体淡出（display:contents 行无盒子，逐 cell 施加） */
.share-row.hovered .share-name,
.share-row.hovered .count-tokens { color: var(--ip-primary-600); }
.share-row.hovered .share-dot { transform: scale(1.35); }
.share-row.dim .share-label,
.share-row.dim .share-track,
.share-row.dim .count-tokens { opacity: 0.35; }
.share-row.dim .count-msgs { opacity: 0.35; }
.mix-meta {
  display: flex; justify-content: space-between; gap: 12px;
  padding-top: 8px;
  border-top: 1px solid var(--ip-color-border-default);
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
}
</style>
