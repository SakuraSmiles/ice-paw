<script setup lang="ts">
// TaskSettings.vue — 设置·定时任务（0.9.3）：任务列表 + 新建/编辑表单 + 执行日志。
// 设计真相源 docs/scheduled-tasks-design.md——载体仅专属会话（懒建，表单不出现
// 载体选择）；「结果出现在某会话」由转发投递配置（进阶折叠区）。
// 编辑交互契约：表单草稿 + 显式保存；启用开关/立即运行是治理动作即时生效；
// 删除两步确认（武装态）。执行日志每轮一条，可跳转载体会话。
import { computed, onMounted, reactive, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { Clock, ChevronDown, ChevronRight, Pencil, Trash2, Plus } from "@lucide/vue";
import { bridge } from "../../api/bridge";
import { useAgentStore } from "../../stores/agent";
import { useChatStore } from "../../stores/chat";
import { msgOf } from "../../utils/errors";
import type { ScheduledTaskView, TaskRun, TaskScheduleKind } from "../../types";

const router = useRouter();
const agentStore = useAgentStore();
const chatStore = useChatStore();

const tasks = ref<ScheduledTaskView[]>([]);
const loadError = ref("");
const loading = ref(false);
/** 展开执行日志的任务 id 集 */
const openRuns = ref<Set<string>>(new Set());
const runsCache = ref<Map<string, TaskRun[]>>(new Map());
/** 立即运行的行内反馈：task_id → 已发起时刻 */
const justTriggered = ref<Map<string, number>>(new Map());

const WEEKDAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"];

function parseData(json: string): Record<string, unknown> {
  try { return JSON.parse(json) as Record<string, unknown>; } catch { return {}; }
}

/** 调度人话摘要（列表行 + 表单回显） */
function scheduleLabel(kind: string, dataJson: string): string {
  const d = parseData(dataJson);
  switch (kind) {
    case "once": return `一次性 ${d.at ?? "?"}`;
    case "daily": return `每天 ${d.time ?? "?"}`;
    case "weekly": {
      const days = Array.isArray(d.weekdays) ? (d.weekdays as number[]).map((w) => WEEKDAY_LABELS[w] ?? "?").join("/") : "?";
      return `每周${days} ${d.time ?? "?"}`;
    }
    case "interval": return `每 ${d.minutes ?? "?"} 分钟`;
    case "cron": return `cron：${d.expr ?? "?"}`;
    default: return kind;
  }
}

function statusLabel(s: string): string {
  return { running: "运行中", done: "已完成", error: "失败", missed: "已错过" }[s] ?? s;
}

const agentName = (id: string) => agentStore.list.find((a) => a.id === id)?.name ?? "（已删除）";

async function load() {
  loading.value = true;
  loadError.value = "";
  try {
    tasks.value = await bridge.tasks.list();
  } catch (e) {
    loadError.value = msgOf(e);
  } finally {
    loading.value = false;
  }
}

onMounted(load);

// =========================================================================
// 执行日志
// =========================================================================

async function toggleRuns(taskId: string) {
  if (openRuns.value.has(taskId)) {
    openRuns.value.delete(taskId);
  } else {
    openRuns.value.add(taskId);
    try {
      runsCache.value.set(taskId, await bridge.tasks.runs(taskId, 50));
    } catch (e) {
      loadError.value = msgOf(e);
    }
  }
}

async function refreshRuns(taskId: string) {
  try {
    runsCache.value.set(taskId, await bridge.tasks.runs(taskId, 50));
  } catch { /* 下次展开再刷 */ }
}

/** 跳转载体会话（无载体 = 尚未跑过，无跳转） */
function openConversation(convId: string | null) {
  if (!convId) return;
  chatStore.selectConversation(convId);
  router.push("/");
}

// =========================================================================
// 治理动作（即时）
// =========================================================================

async function toggleEnabled(t: ScheduledTaskView) {
  try {
    await bridge.tasks.update({ id: t.id, enabled: t.enabled ? 0 : 1 });
    await load();
  } catch (e) {
    loadError.value = msgOf(e);
  }
}

async function runNow(t: ScheduledTaskView) {
  try {
    await bridge.tasks.runNow(t.id);
    justTriggered.value.set(t.id, Date.now());
    setTimeout(() => { justTriggered.value.delete(t.id); }, 4000);
    // 执行记录异步落——稍后自动刷两次（发起 + 可能完成）
    setTimeout(load, 3000);
    setTimeout(load, 12000);
  } catch (e) {
    loadError.value = msgOf(e);
  }
}

/** 删除两步确认（武装态，外点/超时解除） */
const armedDelete = ref<string | null>(null);
let armTimer: ReturnType<typeof setTimeout> | null = null;

function armDelete(id: string) {
  armedDelete.value = id;
  if (armTimer) clearTimeout(armTimer);
  armTimer = setTimeout(() => { armedDelete.value = null; }, 4000);
}

async function confirmDelete(t: ScheduledTaskView) {
  if (armedDelete.value !== t.id) { armDelete(t.id); return; }
  armedDelete.value = null;
  if (armTimer) clearTimeout(armTimer);
  try {
    await bridge.tasks.remove(t.id);
    if (editingId.value === t.id) closeForm();
    await load();
  } catch (e) {
    loadError.value = msgOf(e);
  }
}

// =========================================================================
// 新建 / 编辑（草稿 + 显式保存）
// =========================================================================

const WEEKDAY_OPTIONS = [0, 1, 2, 3, 4, 5, 6];
const KIND_OPTIONS: { value: TaskScheduleKind; label: string }[] = [
  { value: "daily", label: "每天" },
  { value: "weekly", label: "每周" },
  { value: "interval", label: "间隔" },
  { value: "once", label: "一次性" },
  { value: "cron", label: "cron" },
];

const showForm = ref(false);
const editingId = ref<string | null>(null);
const formError = ref("");
const saving = ref(false);
const advancedOpen = ref(false);

interface Draft {
  name: string;
  agent_id: string;
  kind: TaskScheduleKind;
  at: string;        // once（datetime-local 值）
  time: string;      // daily/weekly HH:MM
  weekdays: number[];
  minutes: number;
  expr: string;
  prompt: string;
  miss_policy: "run_once" | "skip";
  deliver_to: string | null;
}

const draft = reactive<Draft>({
  name: "", agent_id: "", kind: "daily",
  at: "", time: "09:00", weekdays: [1], minutes: 30, expr: "0 0 9 * * *",
  prompt: "", miss_policy: "run_once", deliver_to: null,
});

function emptyDraft(): void {
  Object.assign(draft, {
    name: "", agent_id: agentStore.list[0]?.id ?? "",
    kind: "daily", at: "", time: "09:00", weekdays: [1], minutes: 30, expr: "0 0 9 * * *",
    prompt: "", miss_policy: "run_once", deliver_to: null,
  });
}

function openCreate() {
  editingId.value = null;
  emptyDraft();
  formError.value = "";
  advancedOpen.value = false;
  showForm.value = true;
}

function openEdit(t: ScheduledTaskView) {
  editingId.value = t.id;
  const d = parseData(t.schedule_data);
  Object.assign(draft, {
    name: t.name,
    agent_id: t.agent_id,
    kind: t.schedule_kind as TaskScheduleKind,
    at: typeof d.at === "string" ? d.at.replace(" ", "T").slice(0, 16) : "",
    time: typeof d.time === "string" ? d.time : "09:00",
    weekdays: Array.isArray(d.weekdays) ? (d.weekdays as number[]) : [1],
    minutes: typeof d.minutes === "number" ? d.minutes : 30,
    expr: typeof d.expr === "string" ? d.expr : "0 9 * * *",
    prompt: t.prompt,
    miss_policy: (t.miss_policy === "skip" ? "skip" : "run_once") as Draft["miss_policy"],
    deliver_to: t.deliver_to_conv_id,
  });
  formError.value = "";
  advancedOpen.value = false;
  showForm.value = true;
}

function closeForm() {
  showForm.value = false;
  editingId.value = null;
}

/** 草稿 → schedule_data JSON（once 的 datetime-local 转 DB 时间格式） */
function scheduleDataOf(): string {
  switch (draft.kind) {
    case "once": return JSON.stringify({ at: draft.at.replace("T", " ") + ":00" });
    case "daily": return JSON.stringify({ time: draft.time });
    case "weekly": return JSON.stringify({ weekdays: [...draft.weekdays].sort(), time: draft.time });
    case "interval": return JSON.stringify({ minutes: draft.minutes });
    case "cron": return JSON.stringify({ expr: draft.expr });
  }
}

const canSave = computed(() =>
  draft.name.trim() !== "" &&
  draft.agent_id !== "" &&
  draft.prompt.trim() !== "" &&
  (draft.kind === "cron" ? draft.expr.trim() !== "" : draft.kind !== "once" || draft.at !== "") &&
  (draft.kind !== "weekly" || draft.weekdays.length > 0),
);

async function save() {
  if (!canSave.value || saving.value) return;
  saving.value = true;
  formError.value = "";
  try {
    const payload = {
      name: draft.name.trim(),
      agent_id: draft.agent_id,
      schedule_kind: draft.kind,
      schedule_data: scheduleDataOf(),
      prompt: draft.prompt,
      miss_policy: draft.miss_policy,
      deliver_to_conv_id: draft.deliver_to,
    };
    if (editingId.value) {
      await bridge.tasks.update({ id: editingId.value, ...payload });
    } else {
      await bridge.tasks.create(payload);
    }
    closeForm();
    await load();
  } catch (e) {
    formError.value = msgOf(e);
  } finally {
    saving.value = false;
  }
}

// =========================================================================
// 档位预览（cron/档位防写错——表单实时显示未来三个运行时点）
// =========================================================================

const previewTimes = ref<string[]>([]);
const previewError = ref("");
let previewTimer: ReturnType<typeof setTimeout> | null = null;

watch(
  () => [draft.kind, draft.at, draft.time, draft.weekdays.slice(), draft.minutes, draft.expr] as const,
  () => {
    if (!showForm.value) return;
    if (previewTimer) clearTimeout(previewTimer);
    previewTimer = setTimeout(async () => {
      previewError.value = "";
      previewTimes.value = [];
      if (draft.kind === "once" && !draft.at) return;
      if (draft.kind === "cron" && !draft.expr.trim()) return;
      try {
        previewTimes.value = await bridge.tasks.preview(draft.kind, scheduleDataOf());
      } catch (e) {
        previewError.value = msgOf(e);
      }
    }, 300);
  },
  { deep: true },
);

const deliverTargets = computed(() => chatStore.conversations);
const convTitle = (id: string | null) =>
  chatStore.conversations.find((c) => c.id === id)?.title ?? "（会话已删除）";
</script>

<template>
  <div class="task-settings">
    <div class="page-head">
      <div>
        <h2 class="page-title">定时任务</h2>
        <p class="page-desc">到点自动让 Agent 执行一轮任务——结果落在任务专属会话，可转发到其他会话。</p>
      </div>
      <button v-if="!showForm" class="btn-new" @click="openCreate">
        <Plus :size="14" /> 新建任务
      </button>
    </div>

    <p v-if="loadError" class="load-error">{{ loadError }}</p>
    <p v-if="!loading && tasks.length === 0 && !showForm" class="empty-hint">
      还没有定时任务——点「新建任务」创建第一个，例如「每天 9 点汇报项目进度」。
    </p>

    <!-- 新建 / 编辑表单 -->
    <div v-if="showForm" class="task-form">
      <h3 class="form-title">{{ editingId ? "编辑任务" : "新建任务" }}</h3>
      <div class="form-grid">
        <label class="field">
          <span class="field-label">名称</span>
          <input v-model="draft.name" type="text" placeholder="如：每日项目进度汇报" />
        </label>
        <label class="field">
          <span class="field-label">Agent</span>
          <select v-model="draft.agent_id">
            <option v-for="a in agentStore.list" :key="a.id" :value="a.id">{{ a.name }}</option>
          </select>
        </label>
      </div>

      <div class="form-grid">
        <div class="field">
          <span class="field-label">调度</span>
          <div class="sched-row">
            <select v-model="draft.kind">
              <option v-for="o in KIND_OPTIONS" :key="o.value" :value="o.value">{{ o.label }}</option>
            </select>
            <input v-if="draft.kind === 'daily' || draft.kind === 'weekly'" v-model="draft.time" type="time" />
            <input v-else-if="draft.kind === 'once'" v-model="draft.at" type="datetime-local" />
            <input v-else-if="draft.kind === 'interval'" v-model.number="draft.minutes" type="number" min="10" step="5" />
            <input v-else-if="draft.kind === 'cron'" v-model="draft.expr" type="text" class="cron-input" placeholder="秒 分 时 日 月 周" />
          </div>
        </div>
        <div v-if="draft.kind === 'weekly'" class="field">
          <span class="field-label">命中日（周一=0）</span>
          <div class="weekday-row">
            <button
              v-for="w in WEEKDAY_OPTIONS" :key="w"
              type="button"
              :class="['wd', { on: draft.weekdays.includes(w) }]"
              @click="draft.weekdays.includes(w) ? draft.weekdays = draft.weekdays.filter((x) => x !== w) : draft.weekdays.push(w)"
            >{{ WEEKDAY_LABELS[w] }}</button>
          </div>
        </div>
      </div>

      <label class="field">
        <span class="field-label">提示词（每次执行发给 Agent 的内容）</span>
        <textarea v-model="draft.prompt" rows="3" placeholder="如：查看项目里各会话最近的进展，汇总成三行日报。" />
      </label>

      <p v-if="previewError" class="preview-error">{{ previewError }}</p>
      <p v-else-if="previewTimes.length" class="preview-ok">接下来：{{ previewTimes.join(" → ") }}</p>
      <p v-else-if="draft.kind === 'interval'" class="preview-hint">最小间隔 10 分钟</p>

      <!-- 进阶折叠 -->
      <button type="button" class="adv-toggle" @click="advancedOpen = !advancedOpen">
        <ChevronDown v-if="advancedOpen" :size="14" /><ChevronRight v-else :size="14" />
        进阶（错过策略 / 结果转发）
      </button>
      <div v-if="advancedOpen" class="adv-body">
        <div class="form-grid">
          <label class="field">
            <span class="field-label">应用未运行时错过</span>
            <select v-model="draft.miss_policy">
              <option value="run_once">下次启动补跑一次</option>
              <option value="skip">跳过并顺延（记录「已错过」）</option>
            </select>
          </label>
          <label class="field">
            <span class="field-label">结果转发到会话（可选）</span>
            <select v-model="draft.deliver_to">
              <option :value="null">不转发</option>
              <option v-for="c in deliverTargets" :key="c.id" :value="c.id">{{ c.title || c.id }}</option>
            </select>
          </label>
        </div>
        <p class="adv-hint">转发走跨会话投递通道——目标会话按其收件政策消费（自动接收 / 需批准）。</p>
      </div>

      <p v-if="formError" class="load-error">{{ formError }}</p>
      <div class="form-actions">
        <button class="btn-primary" :disabled="!canSave || saving" @click="save">{{ saving ? "保存中…" : "保存" }}</button>
        <button class="btn-ghost" @click="closeForm">取消</button>
      </div>
    </div>

    <!-- 任务列表 -->
    <div v-for="t in tasks" :key="t.id" class="task-card" :class="{ off: !t.enabled }">
      <div class="card-main">
        <button class="btn-runs" :title="openRuns.has(t.id) ? '收起执行记录' : '展开执行记录'" @click="toggleRuns(t.id); refreshRuns(t.id)">
          <ChevronDown v-if="openRuns.has(t.id)" :size="14" /><ChevronRight v-else :size="14" />
        </button>
        <div class="card-info">
          <div class="card-title-row">
            <Clock :size="13" class="card-clock" />
            <span class="card-name">{{ t.name }}</span>
            <span v-if="t.last_run" :class="['status-chip', t.last_run.status]">{{ statusLabel(t.last_run.status) }}</span>
          </div>
          <div class="card-sub">
            {{ agentName(t.agent_id) }} · {{ scheduleLabel(t.schedule_kind, t.schedule_data) }}
            · {{ t.enabled ? (t.next_run ? `下次 ${t.next_run}` : "已完成") : "已停用" }}
            <template v-if="t.deliver_to_conv_id"> · 转发至「{{ convTitle(t.deliver_to_conv_id) }}」</template>
          </div>
        </div>
        <div class="card-ops">
          <button
            class="op-btn run"
            :title="justTriggered.has(t.id) ? '已发起——稍候看执行记录' : '立即运行一次（不影响计划）'"
            :disabled="!!justTriggered.has(t.id)"
            @click="runNow(t)"
          >{{ justTriggered.has(t.id) ? "已发起" : "立即运行" }}</button>
          <button class="op-btn" title="编辑" @click="openEdit(t)"><Pencil :size="13" /></button>
          <button
            :class="['op-btn', 'del', { armed: armedDelete === t.id }]"
            :title="armedDelete === t.id ? '再点一次确认删除（执行记录一并清除，任务会话保留）' : '删除任务'"
            @click="confirmDelete(t)"
          ><Trash2 :size="13" />{{ armedDelete === t.id ? "确认删除" : "" }}</button>
          <button
            class="op-toggle"
            role="switch"
            :aria-checked="!!t.enabled"
            :title="t.enabled ? '停用（保留配置）' : '启用'"
            @click="toggleEnabled(t)"
          ><span class="knob" /></button>
        </div>
      </div>

      <!-- 执行日志 -->
      <div v-if="openRuns.has(t.id)" class="runs-panel">
        <p v-if="(runsCache.get(t.id) ?? []).length === 0" class="empty-hint">还没有执行记录。</p>
        <div v-for="r in runsCache.get(t.id) ?? []" :key="r.id" class="run-row">
          <span :class="['status-dot', r.status]" :title="statusLabel(r.status)" />
          <span class="run-time">{{ r.started_at }}</span>
          <span class="run-summary">
            <template v-if="r.status === 'error'">{{ r.error }}</template>
            <template v-else>{{ r.summary ?? (r.status === "running" ? "执行中…" : statusLabel(r.status)) }}</template>
          </span>
          <button v-if="r.conv_id" class="op-btn" title="打开任务会话" @click="openConversation(r.conv_id)">查看会话</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.task-settings { display: flex; flex-direction: column; gap: var(--ip-spacing-3); padding-bottom: var(--ip-spacing-8); }

.page-head { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--ip-spacing-4); }
.page-title { margin: 0; font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.page-desc { margin: 4px 0 0; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); }

.btn-new { display: inline-flex; align-items: center; gap: 6px; padding: var(--ip-spacing-2) var(--ip-spacing-3); border: 1px solid var(--ip-primary-400); border-radius: var(--ip-radius-md); background: var(--ip-primary-500); color: #fff; font: inherit; cursor: pointer; }
.btn-new:hover { background: var(--ip-primary-600); }

.load-error { margin: 0; font-size: var(--ip-text-body-sm-size); color: var(--ip-danger-base); }
.empty-hint { margin: 0; padding: var(--ip-spacing-6); text-align: center; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-disabled); }

/* ===== 表单 ===== */
.task-form { display: flex; flex-direction: column; gap: var(--ip-spacing-3); padding: var(--ip-spacing-5); border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-lg); background: var(--ip-color-bg-secondary); }
.form-title { margin: 0; font-size: var(--ip-text-body-lg-size); color: var(--ip-color-text-primary); }
.form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: var(--ip-spacing-3); }
.field { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.field-label { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); }
.field input, .field select, .field textarea { padding: var(--ip-spacing-2) var(--ip-spacing-2_5); border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-md); background: var(--ip-color-bg-primary); color: var(--ip-color-text-primary); font: inherit; }
.field textarea { resize: vertical; }
.sched-row { display: flex; gap: var(--ip-spacing-2); min-width: 0; }
.sched-row select { flex-shrink: 0; }
.sched-row input { flex: 1; min-width: 0; }
.cron-input { font-family: var(--ip-font-mono, monospace); }
.weekday-row { display: flex; gap: 4px; flex-wrap: wrap; }
.wd { width: 30px; height: 26px; border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-sm); background: var(--ip-color-bg-primary); color: var(--ip-color-text-secondary); cursor: pointer; font: inherit; font-size: var(--ip-text-caption-size); }
.wd.on { border-color: var(--ip-primary-500); color: var(--ip-primary-600); background: rgba(var(--ip-primary-500-rgb), 0.08); }

.preview-ok { margin: 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); }
.preview-error { margin: 0; font-size: var(--ip-text-caption-size); color: var(--ip-danger-base); }
.preview-hint { margin: 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-disabled); }

.adv-toggle { display: inline-flex; align-items: center; gap: 4px; align-self: flex-start; border: none; background: none; color: var(--ip-color-text-secondary); font: inherit; font-size: var(--ip-text-body-sm-size); cursor: pointer; padding: 0; }
.adv-toggle:hover { color: var(--ip-color-text-primary); }
.adv-body { display: flex; flex-direction: column; gap: var(--ip-spacing-2); padding: var(--ip-spacing-3); border-left: 2px solid var(--ip-color-border-default); }
.adv-hint { margin: 0; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-disabled); }

.form-actions { display: flex; gap: var(--ip-spacing-2); }
.btn-primary { padding: var(--ip-spacing-2) var(--ip-spacing-4); border: none; border-radius: var(--ip-radius-md); background: var(--ip-primary-500); color: #fff; font: inherit; cursor: pointer; }
.btn-primary:disabled { opacity: 0.45; cursor: not-allowed; }
.btn-ghost { padding: var(--ip-spacing-2) var(--ip-spacing-4); border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-md); background: transparent; color: var(--ip-color-text-secondary); font: inherit; cursor: pointer; }

/* ===== 任务卡 ===== */
.task-card { display: flex; flex-direction: column; border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-lg); background: var(--ip-color-bg-secondary); }
.task-card.off { opacity: 0.62; }
.card-main { display: flex; align-items: center; gap: var(--ip-spacing-2); padding: var(--ip-spacing-3) var(--ip-spacing-4); }
.btn-runs { display: flex; align-items: center; justify-content: center; width: 22px; height: 22px; border: none; border-radius: var(--ip-radius-sm); background: transparent; color: var(--ip-color-text-disabled); cursor: pointer; }
.btn-runs:hover { color: var(--ip-color-text-primary); background: var(--ip-color-bg-tertiary); }
.card-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.card-title-row { display: flex; align-items: center; gap: 6px; min-width: 0; }
.card-clock { flex-shrink: 0; color: var(--ip-color-icon-muted); }
.card-name { font-size: var(--ip-text-body-15-size); font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.card-sub { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.status-chip { flex-shrink: 0; padding: 1px 8px; border-radius: 999px; font-size: var(--ip-text-micro-size); }
.status-chip.done { background: rgba(16, 122, 87, 0.12); color: var(--ip-success-base, #107757); }
.status-chip.running { background: rgba(var(--ip-primary-500-rgb), 0.12); color: var(--ip-primary-600); }
.status-chip.error { background: rgba(178, 58, 38, 0.12); color: var(--ip-danger-base); }
.status-chip.missed { background: rgba(146, 108, 18, 0.12); color: var(--ip-warning-base, #926c12); }

.card-ops { display: flex; align-items: center; gap: var(--ip-spacing-2); flex-shrink: 0; }
.op-btn { display: inline-flex; align-items: center; gap: 4px; padding: var(--ip-spacing-1_5) var(--ip-spacing-2_5); border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-md); background: transparent; color: var(--ip-color-text-secondary); font: inherit; font-size: var(--ip-text-caption-size); cursor: pointer; }
.op-btn:hover { color: var(--ip-color-text-primary); border-color: var(--ip-color-border-strong, var(--ip-color-border-default)); }
.op-btn.run { color: var(--ip-primary-600); border-color: var(--ip-primary-400); }
.op-btn.run:disabled { opacity: 0.55; cursor: default; }
.op-btn.del:hover, .op-btn.del.armed { color: var(--ip-danger-base); border-color: var(--ip-danger-base); }

.op-toggle { position: relative; width: 34px; height: 18px; border-radius: 999px; border: none; background: var(--ip-color-border-default); cursor: pointer; padding: 0; transition: background var(--ip-duration-fast) var(--ip-ease-out); }
.op-toggle .knob { position: absolute; top: 2px; left: 2px; width: 14px; height: 14px; border-radius: 50%; background: #fff; transition: transform var(--ip-duration-fast) var(--ip-ease-out); }
.op-toggle[aria-checked="true"] { background: var(--ip-primary-500); }
.op-toggle[aria-checked="true"] .knob { transform: translateX(16px); }

/* ===== 执行日志 ===== */
.runs-panel { display: flex; flex-direction: column; gap: 2px; border-top: 1px solid var(--ip-color-border-default); padding: var(--ip-spacing-3) var(--ip-spacing-4) var(--ip-spacing-3) calc(var(--ip-spacing-4) + 24px); }
.run-row { display: flex; align-items: center; gap: var(--ip-spacing-2); min-width: 0; padding: var(--ip-spacing-1) 0; }
.status-dot { flex-shrink: 0; width: 7px; height: 7px; border-radius: 50%; }
.status-dot.done { background: var(--ip-success-base, #107757); }
.status-dot.running { background: var(--ip-primary-500); }
.status-dot.error { background: var(--ip-danger-base); }
.status-dot.missed { background: var(--ip-warning-base, #926c12); }
.run-time { flex-shrink: 0; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-disabled); font-family: var(--ip-font-mono, monospace); }
.run-summary { flex: 1; min-width: 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
