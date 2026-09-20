<script setup lang="ts">
// TaskSettings.vue — 设置·定时任务（0.9.3）：对齐设置页折叠卡族设计语言
// （ModelSettings/AgentSettings 同款）——新建入口 = 列表第一条虚线卡；实体卡
// 收起态纯摘要双行（点整卡展开编辑 + 执行记录 + 治理动作）；状态徽 = dot +
// 文案 + 相对时。设计真相源 docs/scheduled-tasks-design.md（六点拍板不变）。
import { computed, onMounted, reactive, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { ChevronRight, Clock, Plus } from "@lucide/vue";
import ErrorBanner from "../../components/common/ErrorBanner.vue";
import { bridge } from "../../api/bridge";
import { useAgentStore } from "../../stores/agent";
import { useChatStore } from "../../stores/chat";
import { msgOf } from "../../utils/errors";
import { timeAgo } from "../../utils/time";
import type { ScheduledTaskView, TaskRun, TaskScheduleKind } from "../../types";

const router = useRouter();
const agentStore = useAgentStore();
const chatStore = useChatStore();

const tasks = ref<ScheduledTaskView[]>([]);
const loadError = ref("");
const loading = ref(false);
/** 展开的卡（新建卡 = "new"；互斥单开，族内惯例） */
const expandedId = ref<string | null>(null);
const runsCache = ref<Map<string, TaskRun[]>>(new Map());
/** 立即运行的行内反馈（展开面板内按钮） */
const justTriggered = ref(false);

const WEEKDAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"];

function parseData(json: string): Record<string, unknown> {
  try { return JSON.parse(json) as Record<string, unknown>; } catch { return {}; }
}

/** 调度人话摘要（收起卡次行 + 表单回显） */
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

const statusLabel = (s: string) =>
  ({ running: "运行中", done: "已完成", error: "失败", missed: "已错过" }[s] ?? s);
const statusTone = (s: string) =>
  ({ running: "ok", done: "ok", error: "danger", missed: "warn" }[s] ?? "neutral");

const agentName = (id: string) => agentStore.list.find((a) => a.id === id)?.name ?? "（已删除）";
const convTitle = (id: string | null) =>
  chatStore.conversations.find((c) => c.id === id)?.title ?? "（会话已删除）";

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

/** 卡片展开/收起（点整卡 = 编辑 + 执行记录 + 治理动作；展开时拉执行记录） */
async function toggleExpand(id: string) {
  if (expandedId.value === id) {
    expandedId.value = null;
    return;
  }
  expandedId.value = id;
  if (id !== "new") {
    closeForm();
    fillDraftFromTask(tasks.value.find((t) => t.id === id));
    try {
      runsCache.value.set(id, await bridge.tasks.runs(id, 20));
    } catch { /* 下次展开再刷 */ }
  }
}

function toggleNew() {
  if (expandedId.value === "new") {
    expandedId.value = null;
  } else {
    expandedId.value = "new";
    emptyDraft();
  }
}

/** 跳转载体会话（无载体 = 尚未跑过） */
function openConversation(convId: string | null) {
  if (!convId) return;
  chatStore.selectConversation(convId);
  router.push("/");
}

// =========================================================================
// 治理动作（展开面板内）
// =========================================================================

async function toggleEnabled(t: ScheduledTaskView) {
  try {
    await bridge.tasks.update({ id: t.id, enabled: t.enabled ? 0 : 1 });
    await load();
    expandedId.value = null;
  } catch (e) {
    loadError.value = msgOf(e);
  }
}

async function runNow(t: ScheduledTaskView) {
  try {
    await bridge.tasks.runNow(t.id);
    justTriggered.value = true;
    setTimeout(() => { justTriggered.value = false; }, 4000);
    // 执行记录异步落——稍后自动刷两次
    setTimeout(async () => {
      await load();
      try { runsCache.value.set(t.id, await bridge.tasks.runs(t.id, 20)); } catch { /* 忽略 */ }
    }, 3000);
    setTimeout(async () => {
      try { runsCache.value.set(t.id, await bridge.tasks.runs(t.id, 20)); } catch { /* 忽略 */ }
    }, 12000);
  } catch (e) {
    loadError.value = msgOf(e);
  }
}

/** 删除两步确认（武装态，4s 超时解除——族内惯例） */
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
    if (expandedId.value === t.id) expandedId.value = null;
    await load();
  } catch (e) {
    loadError.value = msgOf(e);
  }
}

// =========================================================================
// 新建 / 编辑草稿（显式保存契约）
// =========================================================================

const WEEKDAY_OPTIONS = [0, 1, 2, 3, 4, 5, 6];
const KIND_OPTIONS: { value: TaskScheduleKind; label: string }[] = [
  { value: "daily", label: "每天" },
  { value: "weekly", label: "每周" },
  { value: "interval", label: "间隔" },
  { value: "once", label: "一次性" },
  { value: "cron", label: "cron" },
];

const formError = ref("");
const saving = ref(false);
const advancedOpen = ref(false);
/** 草稿绑定的任务 id（null = 新建草稿） */
const draftFor = ref<string | null>(null);

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

function closeForm() { draftFor.value = null; formError.value = ""; }

function emptyDraft(): void {
  draftFor.value = null;
  formError.value = "";
  advancedOpen.value = false;
  Object.assign(draft, {
    name: "", agent_id: agentStore.list[0]?.id ?? "",
    kind: "daily", at: "", time: "09:00", weekdays: [1], minutes: 30, expr: "0 0 9 * * *",
    prompt: "", miss_policy: "run_once", deliver_to: null,
  });
}

function fillDraftFromTask(t: ScheduledTaskView | undefined) {
  if (!t) return;
  const d = parseData(t.schedule_data);
  draftFor.value = t.id;
  formError.value = "";
  advancedOpen.value = false;
  Object.assign(draft, {
    name: t.name,
    agent_id: t.agent_id,
    kind: t.schedule_kind as TaskScheduleKind,
    at: typeof d.at === "string" ? d.at.replace(" ", "T").slice(0, 16) : "",
    time: typeof d.time === "string" ? d.time : "09:00",
    weekdays: Array.isArray(d.weekdays) ? [...(d.weekdays as number[])] : [1],
    minutes: typeof d.minutes === "number" ? d.minutes : 30,
    expr: typeof d.expr === "string" ? d.expr : "0 0 9 * * *",
    prompt: t.prompt,
    miss_policy: (t.miss_policy === "skip" ? "skip" : "run_once") as Draft["miss_policy"],
    deliver_to: t.deliver_to_conv_id,
  });
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
    if (draftFor.value) {
      await bridge.tasks.update({ id: draftFor.value, ...payload });
    } else {
      await bridge.tasks.create(payload);
    }
    expandedId.value = null;
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
    if (expandedId.value === null) return;
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
</script>

<template>
  <div class="task-settings">
    <h2 class="content-title">定时任务</h2>

    <div v-if="loading" class="loading-state">加载中...</div>
    <template v-else>
      <ErrorBanner
        v-if="loadError"
        variant="banner"
        title="定时任务加载失败"
        :detail="loadError"
        retry-label="重试"
        @retry="load"
      />

      <div class="card-list">
        <!-- 新建卡（列表第一条，虚线边框；点击展开创建表单） -->
        <div class="task-card new-card" :class="{ expanded: expandedId === 'new' }" @click="toggleNew">
          <div class="card-top">
            <div class="row-title">
              <Plus :size="16" class="new-plus" />
              <span class="card-name new-name">新建定时任务</span>
              <span class="new-hint">到点自动让 Agent 执行一轮任务</span>
            </div>
            <ChevronRight :size="16" class="card-chevron" :class="{ rotated: expandedId === 'new' }" />
          </div>
        </div>

        <!-- 实体卡 -->
        <div
          v-for="t in tasks"
          :key="t.id"
          class="task-card"
          :class="{ expanded: expandedId === t.id }"
          @click="toggleExpand(t.id)"
        >
          <!-- 收起态：纯摘要双行（族内惯例——点整卡展开） -->
          <div class="card-top">
            <div class="card-main">
              <div class="row-title">
                <Clock :size="13" class="card-clock" />
                <span class="card-name">{{ t.name }}</span>
                <span class="next-run" :title="t.next_run ?? undefined">
                  {{ t.enabled ? (t.next_run ? `下次 ${timeAgo(t.next_run)}` : "已完成") : "已停用" }}
                </span>
              </div>
              <div class="row-sub">
                <span class="sub-left">{{ agentName(t.agent_id) }} · {{ scheduleLabel(t.schedule_kind, t.schedule_data) }}
                  <template v-if="t.deliver_to_conv_id"> · 转发至「{{ convTitle(t.deliver_to_conv_id) }}」</template>
                </span>
                <span v-if="t.last_run" class="health-chip" :class="`health-chip--${statusTone(t.last_run.status)}`" :title="t.last_run.started_at">
                  <span class="health-dot" aria-hidden="true"></span>{{ statusLabel(t.last_run.status) }} · {{ timeAgo(t.last_run.started_at) }}
                </span>
              </div>
            </div>
            <ChevronRight :size="16" class="card-chevron" :class="{ rotated: expandedId === t.id }" />
          </div>

          <!-- 展开态：编辑表单 + 执行记录 + 治理动作 -->
          <div v-if="expandedId === t.id" class="expand-panel" @click.stop>
            <div class="form-grid">
              <label class="field">
                <span class="field-label">名称</span>
                <input v-model="draft.name" type="text" class="form-input" placeholder="如：每日项目进度汇报" />
              </label>
              <label class="field">
                <span class="field-label">Agent</span>
                <select v-model="draft.agent_id" class="form-input">
                  <option v-for="a in agentStore.list" :key="a.id" :value="a.id">{{ a.name }}</option>
                </select>
              </label>
            </div>

            <div class="form-grid">
              <div class="field">
                <span class="field-label">调度</span>
                <div class="sched-row">
                  <select v-model="draft.kind" class="form-input">
                    <option v-for="o in KIND_OPTIONS" :key="o.value" :value="o.value">{{ o.label }}</option>
                  </select>
                  <input v-if="draft.kind === 'daily' || draft.kind === 'weekly'" v-model="draft.time" type="time" class="form-input" />
                  <input v-else-if="draft.kind === 'once'" v-model="draft.at" type="datetime-local" class="form-input" />
                  <input v-else-if="draft.kind === 'interval'" v-model.number="draft.minutes" type="number" min="10" step="5" class="form-input" />
                  <input v-else-if="draft.kind === 'cron'" v-model="draft.expr" type="text" class="form-input cron-input" placeholder="秒 分 时 日 月 周" />
                </div>
              </div>
              <div v-if="draft.kind === 'weekly'" class="field">
                <span class="field-label">命中日（周一起算）</span>
                <div class="weekday-row">
                  <button
                    v-for="w in WEEKDAY_OPTIONS" :key="w" type="button"
                    :class="['wd', { on: draft.weekdays.includes(w) }]"
                    @click="draft.weekdays.includes(w) ? draft.weekdays = draft.weekdays.filter((x) => x !== w) : draft.weekdays.push(w)"
                  >{{ WEEKDAY_LABELS[w] }}</button>
                </div>
              </div>
            </div>

            <label class="field">
              <span class="field-label">提示词（每次执行发给 Agent 的内容）</span>
              <textarea v-model="draft.prompt" rows="3" class="form-input" placeholder="如：查看项目里各会话最近的进展，汇总成三行日报。" />
            </label>

            <p v-if="previewError" class="preview-error">{{ previewError }}</p>
            <p v-else-if="previewTimes.length" class="preview-ok">接下来：{{ previewTimes.join(" → ") }}</p>
            <p v-else-if="draft.kind === 'interval'" class="preview-hint">最小间隔 10 分钟</p>

            <!-- 进阶折叠 -->
            <button type="button" class="adv-toggle" @click="advancedOpen = !advancedOpen">
              <ChevronRight :size="13" class="adv-chevron" :class="{ rotated: advancedOpen }" />
              进阶（错过策略 / 结果转发）
            </button>
            <div v-if="advancedOpen" class="adv-body">
              <div class="form-grid">
                <label class="field">
                  <span class="field-label">应用未运行时错过</span>
                  <select v-model="draft.miss_policy" class="form-input">
                    <option value="run_once">下次启动补跑一次</option>
                    <option value="skip">跳过并顺延（记录「已错过」）</option>
                  </select>
                </label>
                <label class="field">
                  <span class="field-label">结果转发到会话（可选）</span>
                  <select v-model="draft.deliver_to" class="form-input">
                    <option :value="null">不转发</option>
                    <option v-for="c in deliverTargets" :key="c.id" :value="c.id">{{ c.title || c.id }}</option>
                  </select>
                </label>
              </div>
              <p class="adv-hint">转发走跨会话投递通道——目标会话按其收件政策消费（自动接收 / 需批准）。</p>
            </div>

            <p v-if="formError" class="form-error">{{ formError }}</p>

            <!-- 执行记录（展开面板内，任务自己的运行历史） -->
            <div class="runs-block">
              <div class="runs-head">
                <span class="runs-title">执行记录</span>
                <button v-if="t.target_conv_id" class="btn btn-ghost" @click="openConversation(t.target_conv_id)">打开任务会话</button>
              </div>
              <p v-if="(runsCache.get(t.id) ?? []).length === 0" class="runs-empty">还没有执行记录。</p>
              <div v-for="r in runsCache.get(t.id) ?? []" :key="r.id" class="run-row">
                <span :class="['status-dot', statusTone(r.status)]" :title="statusLabel(r.status)" />
                <span class="run-time">{{ r.started_at }}</span>
                <span class="run-summary">
                  <template v-if="r.status === 'error'">{{ r.error }}</template>
                  <template v-else>{{ r.summary ?? (r.status === "running" ? "执行中…" : statusLabel(r.status)) }}</template>
                </span>
              </div>
            </div>

            <!-- 操作行（治理动作族内惯例在展开面板内） -->
            <div class="ops-row">
              <button class="btn btn-primary" :disabled="!canSave || saving" @click="save">{{ saving ? "保存中…" : "保存" }}</button>
              <button class="btn" @click="expandedId = null">取消</button>
              <span class="ops-gap" />
              <button class="btn" :disabled="justTriggered" @click="runNow(t)">{{ justTriggered ? "已发起…" : "立即运行" }}</button>
              <button class="btn" @click="toggleEnabled(t)">{{ t.enabled ? "停用" : "启用" }}</button>
              <button
                :class="['btn', 'btn-del', { armed: armedDelete === t.id }]"
                :title="armedDelete === t.id ? '再点一次确认删除（执行记录一并清除，任务会话保留）' : '删除任务'"
                @click="confirmDelete(t)"
              >{{ armedDelete === t.id ? "确认删除" : "删除" }}</button>
            </div>
          </div>
        </div>

        <!-- 新建卡展开态（创建表单） -->
        <div v-if="expandedId === 'new'" class="task-card new-form-card" @click.stop>
          <div class="form-grid">
            <label class="field">
              <span class="field-label">名称</span>
              <input v-model="draft.name" type="text" class="form-input" placeholder="如：每日项目进度汇报" />
            </label>
            <label class="field">
              <span class="field-label">Agent</span>
              <select v-model="draft.agent_id" class="form-input">
                <option v-for="a in agentStore.list" :key="a.id" :value="a.id">{{ a.name }}</option>
              </select>
            </label>
          </div>
          <div class="form-grid">
            <div class="field">
              <span class="field-label">调度</span>
              <div class="sched-row">
                <select v-model="draft.kind" class="form-input">
                  <option v-for="o in KIND_OPTIONS" :key="o.value" :value="o.value">{{ o.label }}</option>
                </select>
                <input v-if="draft.kind === 'daily' || draft.kind === 'weekly'" v-model="draft.time" type="time" class="form-input" />
                <input v-else-if="draft.kind === 'once'" v-model="draft.at" type="datetime-local" class="form-input" />
                <input v-else-if="draft.kind === 'interval'" v-model.number="draft.minutes" type="number" min="10" step="5" class="form-input" />
                <input v-else-if="draft.kind === 'cron'" v-model="draft.expr" type="text" class="form-input cron-input" placeholder="秒 分 时 日 月 周" />
              </div>
            </div>
            <div v-if="draft.kind === 'weekly'" class="field">
              <span class="field-label">命中日（周一起算）</span>
              <div class="weekday-row">
                <button
                  v-for="w in WEEKDAY_OPTIONS" :key="w" type="button"
                  :class="['wd', { on: draft.weekdays.includes(w) }]"
                  @click="draft.weekdays.includes(w) ? draft.weekdays = draft.weekdays.filter((x) => x !== w) : draft.weekdays.push(w)"
                >{{ WEEKDAY_LABELS[w] }}</button>
              </div>
            </div>
          </div>
          <label class="field">
            <span class="field-label">提示词（每次执行发给 Agent 的内容）</span>
            <textarea v-model="draft.prompt" rows="3" class="form-input" placeholder="如：查看项目里各会话最近的进展，汇总成三行日报。" />
          </label>
          <p v-if="previewError" class="preview-error">{{ previewError }}</p>
          <p v-else-if="previewTimes.length" class="preview-ok">接下来：{{ previewTimes.join(" → ") }}</p>
          <p v-else-if="draft.kind === 'interval'" class="preview-hint">最小间隔 10 分钟</p>

          <button type="button" class="adv-toggle" @click="advancedOpen = !advancedOpen">
            <ChevronRight :size="13" class="adv-chevron" :class="{ rotated: advancedOpen }" />
            进阶（错过策略 / 结果转发）
          </button>
          <div v-if="advancedOpen" class="adv-body">
            <div class="form-grid">
              <label class="field">
                <span class="field-label">应用未运行时错过</span>
                <select v-model="draft.miss_policy" class="form-input">
                  <option value="run_once">下次启动补跑一次</option>
                  <option value="skip">跳过并顺延（记录「已错过」）</option>
                </select>
              </label>
              <label class="field">
                <span class="field-label">结果转发到会话（可选）</span>
                <select v-model="draft.deliver_to" class="form-input">
                  <option :value="null">不转发</option>
                  <option v-for="c in deliverTargets" :key="c.id" :value="c.id">{{ c.title || c.id }}</option>
                </select>
              </label>
            </div>
            <p class="adv-hint">转发走跨会话投递通道——目标会话按其收件政策消费（自动接收 / 需批准）。</p>
          </div>

          <p v-if="formError" class="form-error">{{ formError }}</p>
          <div class="ops-row">
            <button class="btn btn-primary" :disabled="!canSave || saving" @click="save">{{ saving ? "创建中…" : "创建" }}</button>
            <button class="btn" @click="expandedId = null">取消</button>
          </div>
        </div>

        <p v-if="tasks.length === 0" class="empty-hint">还没有定时任务——点上方「新建定时任务」创建第一个，例如「每天 9 点汇报项目进度」。</p>
      </div>
    </template>
  </div>
</template>

<style scoped>
.task-settings { display: flex; flex-direction: column; gap: var(--ip-spacing-4); padding-bottom: var(--ip-spacing-8); }
.content-title { margin: 0; font-size: var(--ip-text-h3-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.loading-state { padding: var(--ip-spacing-8); text-align: center; color: var(--ip-color-text-disabled); }

/* ===== 卡片族（对齐 profile-card 设计语言） ===== */
.card-list { display: flex; flex-direction: column; gap: var(--ip-spacing-2); }
.task-card { border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-lg); background: var(--ip-color-bg-secondary); cursor: pointer; transition: border-color var(--ip-duration-fast) var(--ip-ease-out); }
.task-card:hover { border-color: var(--ip-color-border-strong, var(--ip-color-border-default)); }
.task-card.expanded { border-color: var(--ip-primary-400); }

.new-card { border: 1px dashed var(--ip-color-border-default); background: transparent; }
.new-card:hover { border-color: var(--ip-primary-400); background: var(--ip-color-bg-tertiary); }
.new-card.expanded { border-style: dashed; background: var(--ip-color-bg-secondary); }
.new-plus { color: var(--ip-primary-500); }
.new-name { color: var(--ip-primary-600); }
.new-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-disabled); font-weight: var(--ip-font-weight-regular); }
.new-form-card { padding: var(--ip-spacing-4); display: flex; flex-direction: column; gap: var(--ip-spacing-3); cursor: default; }

.card-top { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--ip-spacing-2); padding: var(--ip-spacing-3) var(--ip-spacing-4); }
.card-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.row-title { display: flex; align-items: center; gap: 6px; min-width: 0; }
.card-clock { flex-shrink: 0; color: var(--ip-color-icon-muted); }
.card-name { font-size: var(--ip-text-body-15-size); font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.next-run { margin-left: auto; flex-shrink: 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); }
.row-sub { display: flex; align-items: center; justify-content: space-between; gap: var(--ip-spacing-2); min-width: 0; }
.sub-left { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.health-chip { display: inline-flex; align-items: center; gap: 5px; flex-shrink: 0; padding: 1px 8px; border-radius: 999px; font-size: var(--ip-text-micro-size); }
.health-chip .health-dot { width: 6px; height: 6px; border-radius: 50%; }
.health-chip--ok { background: rgba(16, 122, 87, 0.12); color: var(--ip-success-base, #107757); }
.health-chip--ok .health-dot { background: var(--ip-success-base, #107757); }
.health-chip--danger { background: rgba(178, 58, 38, 0.12); color: var(--ip-danger-base); }
.health-chip--danger .health-dot { background: var(--ip-danger-base); }
.health-chip--warn { background: rgba(146, 108, 18, 0.12); color: var(--ip-warning-base, #926c12); }
.health-chip--warn .health-dot { background: var(--ip-warning-base, #926c12); }

.card-chevron { flex-shrink: 0; color: var(--ip-color-icon-muted); transition: transform var(--ip-duration-fast) var(--ip-ease-out); }
.card-chevron.rotated { transform: rotate(90deg); }

/* ===== 展开面板（编辑表单 + 执行记录 + 操作） ===== */
.expand-panel { display: flex; flex-direction: column; gap: var(--ip-spacing-3); padding: var(--ip-spacing-4); border-top: 1px solid var(--ip-color-border-default); cursor: default; }

.form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: var(--ip-spacing-3); }
.field { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.field-label { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); }
.form-input { padding: var(--ip-spacing-2) var(--ip-spacing-2_5); border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-md); background: var(--ip-color-bg-primary); color: var(--ip-color-text-primary); font: inherit; width: 100%; box-sizing: border-box; }
textarea.form-input { resize: vertical; }
.sched-row { display: flex; gap: var(--ip-spacing-2); }
.sched-row select { flex-shrink: 0; width: auto; }
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
.adv-chevron { transition: transform var(--ip-duration-fast) var(--ip-ease-out); }
.adv-chevron.rotated { transform: rotate(90deg); }
.adv-body { display: flex; flex-direction: column; gap: var(--ip-spacing-2); padding: var(--ip-spacing-3); border-left: 2px solid var(--ip-color-border-default); }
.adv-hint { margin: 0; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-disabled); }

.form-error { margin: 0; font-size: var(--ip-text-body-sm-size); color: var(--ip-danger-base); }

/* ===== 执行记录 ===== */
.runs-block { display: flex; flex-direction: column; gap: var(--ip-spacing-1_5); padding: var(--ip-spacing-3); border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-md); }
.runs-head { display: flex; align-items: center; justify-content: space-between; }
.runs-title { font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-secondary); }
.runs-empty { margin: 0; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-disabled); }
.run-row { display: flex; align-items: center; gap: var(--ip-spacing-2); min-width: 0; padding: 2px 0; }
.status-dot { flex-shrink: 0; width: 7px; height: 7px; border-radius: 50%; }
.status-dot.ok { background: var(--ip-success-base, #107757); }
.status-dot.danger { background: var(--ip-danger-base); }
.status-dot.warn { background: var(--ip-warning-base, #926c12); }
.run-time { flex-shrink: 0; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-disabled); font-family: var(--ip-font-mono, monospace); }
.run-summary { flex: 1; min-width: 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

/* ===== 操作行 ===== */
.ops-row { display: flex; align-items: center; gap: var(--ip-spacing-2); flex-wrap: wrap; }
.ops-gap { flex: 1; }
.btn { padding: var(--ip-spacing-2) var(--ip-spacing-3_5); border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-md); background: transparent; color: var(--ip-color-text-secondary); font: inherit; cursor: pointer; }
.btn:hover { color: var(--ip-color-text-primary); }
.btn:disabled { opacity: 0.45; cursor: not-allowed; }
.btn-primary { border: none; background: var(--ip-primary-500); color: #fff; }
.btn-primary:hover { background: var(--ip-primary-600); color: #fff; }
.btn-del:hover, .btn-del.armed { color: var(--ip-danger-base); border-color: var(--ip-danger-base); }

.empty-hint { margin: 0; padding: var(--ip-spacing-6); text-align: center; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-disabled); }
</style>
